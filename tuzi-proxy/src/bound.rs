use crate::{
    parse::{
        delivery, http1, http1::respnose_begin, redis, ClientToProxyDelivery, ParseMeta, Parser,
        ParserDelivery, Protocol, ProxyToServerDelivery, RequestParsedContent, RequestParsedData,
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
    collections::HashMap,
    io::Cursor,
    net::SocketAddr,
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

struct Context {
    start_time: SystemTime,
}

impl Context {
    fn new() -> Self {
        Self {
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

    loop {
        let (socket, _) = listener.accept().await.unwrap();

        debug!("after accept");

        let context = Context::new();

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

    let count = 1;

    let (mut socket_read, mut socket_write) = socket.into_split();
    let (mut orig_read, mut orig_write) = orig.into_split();

    let (
        client_to_proxy_delivery,
        mut parser_deliveries,
        proxy_to_server_delivery,
        server_to_proxy_delivery,
    ) = delivery(count);

    let mut handles = Vec::new();

    handles.push(tokio::spawn(client_to_proxy(
        socket_read,
        client_to_proxy_delivery,
    )));

    handles.push(tokio::spawn(async move {
        http1::parse(parser_deliveries.pop().unwrap())
            .await
            .unwrap();
    }));

    handles.push(tokio::spawn(proxy_to_server(
        orig_write,
        proxy_to_server_delivery,
    )));

    handles.push(tokio::spawn(server_to_proxy(
        count,
        orig_read,
        socket_write,
        server_to_proxy_delivery,
    )));

    // handles.push(tokio::spawn(async move {
    //     copy(&mut orig_read, &mut socket_write).await.unwrap();
    // }));

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

async fn proxy_to_server(mut orig_write: OwnedWriteHalf, mut delivery: ProxyToServerDelivery) {
    delivery.response_protocol_sender.send(None).await.unwrap();

    let mut content = Vec::new();

    loop {
        let raw_fut = delivery.request_raw_receiver.recv();
        let parser_fut = delivery.request_parsed_receiver.recv();
        pin_mut!(parser_fut, raw_fut);

        match select(parser_fut, raw_fut).await {
            Either::Left((data, _)) => {
                let data = data.unwrap();

                // TODO detect more than one protocol.

                break;

                match data.protocol {
                    _ => match data.content {
                        RequestParsedContent::Content(content) => {
                            debug!("request parsed content {} {}", data.protocol, content.len());

                            // let n = orig_write.write(&content).await.unwrap();
                            // if n == 0 {
                            //     panic!("Write zero");
                            // }
                        }
                        RequestParsedContent::Eof => break,
                        RequestParsedContent::ParseFailed => debug!("protocol is not raw"),
                    },
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

    loop {
        let content = delivery.request_raw_receiver.recv().await.unwrap();
        match content {
            Some(content) => {
                let n = orig_write.write(&content).await.unwrap();
                if n == 0 {
                    panic!("Write zero");
                }
            }
            None => break,
        }
    }

    orig_write.shutdown().await.unwrap();
}

async fn server_to_proxy(
    count: usize,
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
        Some(_) => {
            todo!();
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
