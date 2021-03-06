use crate::{
    collect::StdoutCollector,
    error::{TuziError, TuziResult},
    io::receive_copy,
    parse::{
        delivery, http1, redis, ClientToProxyDelivery, ParseProtocol, Protocol,
        ProxyToServerDelivery, RequestParsedContent, RequestParsedData, RequestParserDelivery,
        ResponseParserDelivery, ResponseParserReader, ServerToProxyDelivery,
    },
    tcp::orig_dst_addr,
    wait::WaitGroup,
    Configuration,
};
use async_trait::async_trait;
use futures::{
    future::{select, Either},
    pin_mut,
};
use std::{
    collections::{HashMap, HashSet},
    future::Future,
    net::SocketAddr,
    rc::Rc,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::{Duration, SystemTime},
};
use tokio::{
    io::{copy, AsyncReadExt, AsyncWriteExt},
    net::{
        tcp::{OwnedReadHalf, OwnedWriteHalf},
        TcpListener, TcpStream,
    },
    signal::ctrl_c,
    sync::mpsc,
    task::spawn_blocking,
    time::delay_for,
};
use tracing::{debug, info, instrument};

async fn new_protocol_parsers(wg: WaitGroup) -> Arc<HashMap<Protocol, Arc<dyn ParseProtocol>>> {
    let collector = Arc::new(StdoutCollector::new("==> "));

    let parsers: &[Arc<dyn ParseProtocol>] = &[
        Arc::new(http1::Http1Parser),
        Arc::new(redis::RedisParser::new(collector, wg)),
    ];
    let parsers = parsers
        .iter()
        .map(|parser| (parser.protocol(), parser.clone()))
        .collect();
    Arc::new(parsers)
}

pub enum Bound {
    inbound,
    outbound,
}

#[instrument]
pub async fn outbound(configuration: Rc<Configuration>) {
    let listener = TcpListener::bind(configuration.outbound).await.unwrap();
    run_with_listener(listener, OriginalDst, async {
        ctrl_c().await.unwrap();
        Ok(())
    })
    .await;
}

struct Context {
    start_time: SystemTime,
    parsers: Arc<HashMap<Protocol, Arc<dyn ParseProtocol>>>,
}

impl Context {
    fn new(protocol_parsers: Arc<HashMap<Protocol, Arc<dyn ParseProtocol>>>) -> Self {
        Self {
            parsers: protocol_parsers,
            start_time: SystemTime::now(),
        }
    }
}

#[async_trait]
pub trait ServerAddrProvidable {
    async fn provide_server_addr(&self, socket: &TcpStream) -> TuziResult<SocketAddr>;
}

#[derive(Clone)]
pub struct OriginalDst;

#[async_trait]
impl ServerAddrProvidable for OriginalDst {
    async fn provide_server_addr(&self, socket: &TcpStream) -> TuziResult<SocketAddr> {
        Ok(orig_dst_addr(&socket).unwrap())
    }
}

pub async fn run_with_listener(
    mut listener: TcpListener,
    providable: impl ServerAddrProvidable + Send + Sync + Clone + 'static,
    signal: impl Future<Output = TuziResult<()>>,
) {
    let wg = WaitGroup::new();

    let protocol_parsers = new_protocol_parsers(wg.clone()).await;

    let (mut shutdown_tx, mut shutdown_rx) = mpsc::channel(1);

    let join = {
        let wg = wg.clone();
        tokio::spawn(async move {
            loop {
                let providable = providable.clone();

                let shutdown_fut = shutdown_rx.recv();
                let listener_fut = listener.accept();
                pin_mut!(shutdown_fut, listener_fut);

                match select(shutdown_fut, listener_fut).await {
                    Either::Left((r, _)) => {
                        debug!("shutdown_rx recv");
                        break;
                    }
                    Either::Right((r, _)) => {
                        let (socket, _) = r.unwrap();
                        let wg = wg.clone();

                        let protocol_parsers = protocol_parsers.clone();
                        let context = Context::new(protocol_parsers);

                        tokio::spawn(async move {
                            let peer = socket.peer_addr().unwrap();
                            let orig_dst = providable.provide_server_addr(&socket).await.unwrap();
                            handle(peer, orig_dst, socket, context).await.unwrap();
                            drop(wg);
                        });
                    }
                }
            }
        })
    };

    signal.await.unwrap();
    debug!("signal received, starting graceful shutdown");

    shutdown_tx.send(()).await.unwrap();
    join.await.unwrap();
    wg.wait().await;
}

#[instrument(skip(socket, context))]
async fn handle(
    client_addr: SocketAddr,
    server_addr: SocketAddr,
    socket: TcpStream,
    context: Context,
) -> anyhow::Result<()> {
    let orig = TcpStream::connect(server_addr).await.unwrap();
    debug!("after connect to orig_dst");

    let (socket_read, socket_write) = socket.into_split();
    let (orig_read, orig_write) = orig.into_split();

    let (
        client_to_proxy_delivery,
        mut parser_deliveries,
        proxy_to_server_delivery,
        server_to_proxy_delivery,
    ) = delivery(context.parsers.len(), client_addr, server_addr);

    let mut handles = Vec::new();

    handles.push(tokio::spawn(client_to_proxy(
        client_addr.clone(),
        server_addr.clone(),
        socket_read,
        client_to_proxy_delivery,
    )));

    for (_, parser) in context.parsers.iter() {
        let delivery = parser_deliveries.pop().unwrap();
        let parser = parser.clone();
        handles.push(tokio::spawn(protocol_parse(
            client_addr.clone(),
            server_addr.clone(),
            delivery,
            parser,
        )));
    }

    handles.push(tokio::spawn(proxy_to_server(
        client_addr.clone(),
        server_addr.clone(),
        context.parsers.keys().map(|p| *p).collect(),
        orig_write,
        proxy_to_server_delivery,
    )));

    handles.push(tokio::spawn(server_to_proxy(
        client_addr.clone(),
        server_addr.clone(),
        context.parsers.clone(),
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

#[instrument(skip(socket_read, delivery))]
async fn client_to_proxy(
    client: SocketAddr,
    server: SocketAddr,
    mut socket_read: OwnedReadHalf,
    delivery: ClientToProxyDelivery,
) {
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

#[instrument(skip(delivery, parser))]
pub async fn protocol_parse(
    client: SocketAddr,
    server: SocketAddr,
    delivery: RequestParserDelivery,
    mut parser: Arc<dyn ParseProtocol>,
) {
    let mut request_parsed_sender = delivery.request_parsed_sender.clone();
    if let Err(e) = parser.parse_request(delivery).await {
        match e {
            TuziError::Nom(_) => {
                request_parsed_sender
                    .send(RequestParsedData {
                        protocol: parser.protocol(),
                        content: RequestParsedContent::Failed,
                    })
                    .await
                    .unwrap();
            }
            e => panic!(e),
        }
    }
}

#[instrument(skip(protocols, orig_write, delivery))]
async fn proxy_to_server(
    client: SocketAddr,
    server: SocketAddr,
    mut protocols: HashSet<Protocol>,
    mut orig_write: OwnedWriteHalf,
    mut delivery: ProxyToServerDelivery,
) {
    let mut content = Vec::new();
    let mut protocol = None;

    loop {
        if protocols.is_empty() {
            delivery.response_protocol_sender.send(None).await.unwrap();
            break;
        }

        let parser_fut = delivery.request_parsed_receiver.recv();
        let raw_fut = delivery.request_raw_receiver.recv();
        pin_mut!(parser_fut, raw_fut);

        // TODO Adjust must wait all protocol parser sent.

        match select(parser_fut, raw_fut).await {
            Either::Left((data, _)) => {
                let data = data.unwrap();
                debug!(data.protocol, "request_parsed_receiver, recv");
                match data.content {
                    RequestParsedContent::Content(_content) => {
                        todo!();

                        detect_protocol(
                            &mut protocol,
                            data.protocol,
                            &mut delivery.response_protocol_sender,
                        )
                        .await
                        .unwrap();
                    }
                    RequestParsedContent::Raw => {
                        debug!("protocol is raw {}", data.protocol);
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

    debug!("raw or failed, pre-read content size: {}", content.len());

    if !content.is_empty() {
        let n = orig_write.write(&content).await.unwrap();
        if n == 0 {
            todo!("Write zero");
        }
    }

    debug!("pre-read content written");

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

#[instrument(skip(protocol_parsers, server_read, client_write, delivery))]
async fn server_to_proxy(
    client: SocketAddr,
    server: SocketAddr,
    protocol_parsers: Arc<HashMap<Protocol, Arc<dyn ParseProtocol>>>,
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
