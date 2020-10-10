use crate::{
    error::TuziResult,
    parse::{
        delivery, http1, http1::response_begin, receive_copy, redis, ClientToProxyDelivery,
        Protocol, ProtocolParsable, ProxyToServerDelivery, ReceiveParser, RequestParsedContent,
        RequestParsedData, RequestParserDelivery, ResponseParserDelivery, ResponseParserReader,
        ServerToProxyDelivery,
    },
    tcp::orig_dst_addr,
    Configuration,
};
use futures::{
    future::{select, Either},
    pin_mut,
};
use mpsc::error::TryRecvError;
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    io::Cursor,
    net::SocketAddr,
    ops::Deref,
    rc::Rc,
    sync::{Arc, Once},
    time::SystemTime,
};
use tokio::{
    io::{copy, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader, Seek},
    join,
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpListener, TcpStream,
    },
    sync::{broadcast, mpsc, oneshot},
    try_join,
};
use tracing::{debug, error, info, instrument, warn};

fn new_protocol_parsers() -> Arc<HashMap<Protocol, Arc<dyn ProtocolParsable>>> {
    let protocol_parsers = vec![http1::Parser];
    let protocol_parsers = protocol_parsers
        .into_iter()
        .map(|parser| (parser.protocol(), Arc::new(parser) as _))
        .collect();
    Arc::new(protocol_parsers)
}

struct Context {
    start_time: SystemTime,
    protocol_parsers: Arc<HashMap<Protocol, Arc<dyn ProtocolParsable>>>,
}

impl Context {
    fn new(protocol_parsers: Arc<HashMap<Protocol, Arc<dyn ProtocolParsable>>>) -> Self {
        Self {
            protocol_parsers,
            start_time: SystemTime::now(),
        }
    }
}

#[derive(Copy, Clone)]
pub enum ServerAddr {
    OriginalDst,
    Manual(SocketAddr),
}

pub async fn run_with_listener(mut listener: TcpListener, server_addr: ServerAddr) {
    debug!("after bind");

    let protocol_parsers = new_protocol_parsers();

    loop {
        let (socket, _) = listener.accept().await.unwrap();

        debug!("after accept");

        let protocol_parsers = protocol_parsers.clone();
        let context = Context::new(protocol_parsers);

        tokio::spawn(async move {
            let peer = socket.peer_addr().unwrap();
            let orig_dst = match server_addr {
                ServerAddr::OriginalDst => orig_dst_addr(&socket).unwrap(),
                ServerAddr::Manual(addr) => addr,
            };
            handle(peer, orig_dst, socket, context).await.unwrap();
        });
    }
}

#[instrument(name = "bound:handle", skip(socket, context))]
async fn handle(
    peer: SocketAddr,
    orig_dst: SocketAddr,
    mut socket: TcpStream,
    context: Context,
) -> anyhow::Result<()> {
    let mut orig = TcpStream::connect(orig_dst).await.unwrap();
    debug!("after connect to orig_dst");

    let (mut socket_read, mut socket_write) = socket.into_split();
    let (mut orig_read, mut orig_write) = orig.into_split();

    let (
        client_to_proxy_delivery,
        mut parser_deliveries,
        proxy_to_server_delivery,
        server_to_proxy_delivery,
    ) = delivery(context.protocol_parsers.len());

    let mut handles = Vec::new();

    handles.push(tokio::spawn(client_to_proxy(
        socket_read,
        client_to_proxy_delivery,
    )));

    for (_, protocol_parser) in context.protocol_parsers.iter() {
        let delivery = parser_deliveries.pop().unwrap();
        let protocol_parser = protocol_parser.clone();
        handles.push(tokio::spawn(async move {
            protocol_parser.parse_request(delivery).await.unwrap();
        }));
    }

    handles.push(tokio::spawn(proxy_to_server(
        context.protocol_parsers.keys().map(|p| *p).collect(),
        orig_write,
        proxy_to_server_delivery,
    )));

    handles.push(tokio::spawn(server_to_proxy(
        context.protocol_parsers.clone(),
        orig_read,
        socket_write,
        server_to_proxy_delivery,
    )));

    for handle in handles {
        handle.await.unwrap();
    }

    let delay = SystemTime::now()
        .duration_since(context.start_time)
        .unwrap();

    info!(delay = ?delay);

    Ok(())
}

async fn client_to_proxy(mut socket_read: OwnedReadHalf, delivery: ClientToProxyDelivery) {
    let mut buf = [0; 4096];
    let mut copied = 0;

    loop {
        let n = socket_read.read(&mut buf).await.unwrap();
        if n == 0 {
            debug!("request_sender None");
            delivery.request_raw_sender.send(None).unwrap();
            break;
        }
        copied += n;

        let content = (&buf[..n]).to_owned();
        debug!("request_sender {:?}", Some(content.len()));
        delivery.request_raw_sender.send(Some(content)).unwrap();
    }
}

async fn proxy_to_server(
    mut protocols: HashSet<Protocol>,
    mut orig_write: OwnedWriteHalf,
    mut delivery: ProxyToServerDelivery,
) {
    let mut content = Vec::new();
    let mut protocol = None;

    loop {
        if protocols.is_empty() {
            break;
        }

        let parser_fut = delivery.request_parsed_receiver.recv();
        let raw_fut = delivery.request_raw_receiver.recv();
        pin_mut!(parser_fut, raw_fut);

        match select(parser_fut, raw_fut).await {
            Either::Left((data, _)) => {
                let data = data.unwrap();
                match data.content {
                    RequestParsedContent::Content(content) => {
                        detect_protocol(
                            &mut protocol,
                            data.protocol,
                            &mut delivery.response_protocol_sender,
                        )
                        .await
                        .unwrap();
                    }
                    RequestParsedContent::Raw => {
                        detect_protocol(
                            &mut protocol,
                            data.protocol,
                            &mut delivery.response_protocol_sender,
                        )
                        .await
                        .unwrap();
                        break;
                    }
                    RequestParsedContent::Eof => {
                        return;
                    }
                    RequestParsedContent::Failed => {
                        debug!("protocol is not {}", data.protocol);
                        if !protocols.remove(data.protocol) {
                            todo!("?????");
                        }
                    }
                }
            }
            Either::Right((buf, _)) => {
                let buf = buf.unwrap();
                match buf {
                    Some(buf) => {
                        content.extend_from_slice(&buf);
                    }
                    None => {}
                }
            }
        }
    }

    receive_copy(&mut delivery.request_raw_receiver, &mut orig_write)
        .await
        .unwrap();

    orig_write.shutdown().await.unwrap();
}

async fn detect_protocol<'a>(
    protocol: &'a mut Option<Protocol>,
    detect_protocol: Protocol,
    response_protocol_sender: &'a mut mpsc::Sender<Option<Protocol>>,
) -> TuziResult<()> {
    if protocol.is_some() {
        if *protocol != Some(detect_protocol) {
            panic!("multi protocol");
        }
    } else {
        *protocol = Some(detect_protocol);
        response_protocol_sender
            .send(Some(detect_protocol))
            .await
            .unwrap();
    }
    Ok(())
}

async fn server_to_proxy(
    protocol_parsers: Arc<HashMap<Protocol, Arc<dyn ProtocolParsable>>>,
    mut server_read: OwnedReadHalf,
    mut client_write: OwnedWriteHalf,
    mut delivery: ServerToProxyDelivery,
) {
    let mut content = Vec::new();
    let mut buf = [0; 4096];
    let mut protocol = None;

    loop {
        let read_fut = server_read.read(&mut buf);
        let protocol_fut = delivery.response_protocol_receiver.recv();
        pin_mut!(read_fut, protocol_fut);

        match select(read_fut, protocol_fut).await {
            Either::Left((n, _)) => {
                let n = n.unwrap();
                if n == 0 {
                    break;
                }
                content.extend_from_slice(&buf[..n]);
            }
            Either::Right((p, _)) => {
                let p = p.unwrap();
                protocol = p;
                break;
            }
        }
    }

    debug!(?protocol, "server_to_proxy, receive protocol");

    match protocol {
        Some(protocol) => {
            let parser = protocol_parsers.get(protocol).unwrap();
            let delivery = ResponseParserDelivery {
                reader: ResponseParserReader {
                    exists_content: Some(content),
                    server_read,
                },
                client_write,
            };
            parser.parse_response(delivery).await.unwrap();
        }
        None => {
            if !content.is_empty() {
                let n = client_write.write(&content).await.unwrap();
                if n == 0 {
                    panic!("Write zero");
                }
            }
            copy(&mut server_read, &mut client_write).await.unwrap();
            debug!("raw copy success");
        }
    }
}
