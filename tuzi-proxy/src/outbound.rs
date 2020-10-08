use crate::parse::{delivery, Protocol, RequestParsedContent, http1};
use crate::parse::http1::respnose_begin;
use crate::parse::{
    RequestParsedData, RequestParserDelivery, RequestRawDelivery, RequestTransportDelivery,
};
use crate::{parse::Parser, tcp::orig_dst_addr};
use mpsc::error::TryRecvError;
use std::{
    cell::RefCell, collections::HashMap, io::Cursor, net::SocketAddr, sync::Once, time::SystemTime,
};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader, Seek},
    join,
    net::{TcpListener, TcpStream},
    sync::{broadcast, mpsc, oneshot},
    try_join,
};
use tracing::{debug, error, info, instrument, warn};
use tokio::net::tcp::{OwnedReadHalf, OwnedWriteHalf};

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

#[instrument(name = "outbound:run")]
pub async fn run() -> anyhow::Result<()> {
    let mut listener = TcpListener::bind("127.0.0.1:4140").await?;

    debug!("after bind");

    loop {
        let (socket, _) = listener.accept().await?;

        debug!("after accept");

        let context = Context::new();

        tokio::spawn(async move {
            let peer = socket.peer_addr().unwrap();
            let orig_dst = orig_dst_addr(&socket).unwrap();
            handle(peer, orig_dst, socket, context).await.unwrap();
        });
    }
}

#[instrument(name = "outbound:handle", skip(socket, context))]
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
        request_raw_delivery,
        mut request_parsed_deliveries,
        request_transport_delivery,
        response_protocol_delivery,
    ) = delivery(2);

    let mut handles = Vec::new();
    handles.push(tokio::spawn(client_to_proxy(socket_read, request_raw_delivery)));
    handles.push(tokio::spawn(request_raw_transport(request_parsed_deliveries.pop().unwrap())));
    handles.push(tokio::spawn(async move {
        http1::parse(request_parsed_deliveries.pop().unwrap()).unwrap();
    }));
    handles.push(tokio::spawn(proxy_to_server(orig_write, request_transport_delivery)));
    handles.push(tokio::spawn(async move {
        tokio::io::copy(&mut orig_read, &mut socket_write).await.unwrap();
    }));

    for handle in handles {
        handle.await.unwrap();
    }

    // let (mut response_sender, mut response_receiver) = mpsc::channel(16);
    // let server_to_client = async move {
    //     let mut buf = [0; 4096];
    //     loop {
    //         let n = orig_read.read(&mut buf).await.unwrap();
    //         if n == 0 {
    //             let _ = response_sender.send(None).await;
    //             break;
    //         } else {
    //             let n = socket_write.write(&buf[..n]).await.unwrap();
    //             if n == 0 {
    //                 panic!("Write zero");
    //             }
    //             let _ = response_sender.send(Some(Vec::from(&buf[..n]))).await;
    //         }
    //     }
    //     socket_write.shutdown().await.unwrap();
    // };
    //
    // let server_to_client_analyzer = async move {
    //     let mut content = Vec::new();
    //
    //     let protocol = loop {
    //         let buf = response_receiver.recv().await.unwrap();
    //         let buf = match buf {
    //             Some(buf) => buf,
    //             None => break None,
    //         };
    //         content.extend_from_slice(&buf);
    //
    //         match response_protocol_receiver.try_recv() {
    //             Ok(protocol) => break Some(protocol),
    //             Err(TryRecvError::Empty) => {}
    //             Err(e @ TryRecvError::Closed) => panic!(e),
    //         }
    //     };
    //
    //     if let Some(protocol) = protocol {
    //         match protocol {
    //             "http_1" => {
    //                 let mut parser = Parser::new(content, &mut response_receiver);
    //                 match parser.parse_and_recv(response_begin).await {
    //                     Ok(begin) => info!(?begin, "http response"),
    //                     Err(_) => error!("http_1 parse failed"),
    //                 }
    //             }
    //             _ => {}
    //         }
    //     }
    // };

    let delay = SystemTime::now()
        .duration_since(context.start_time)
        .unwrap();

    info!(delay = ?delay);

    Ok(())
}

async fn client_to_proxy(mut socket_read: OwnedReadHalf, request_raw_delivery: RequestRawDelivery) {
    let mut buf = [0; 4096];
    let mut copied = 0;

    loop {
        let n = socket_read.read(&mut buf).await.unwrap();
        if n == 0 {
            debug!("request_sender None");
            request_raw_delivery.request_raw_sender.send(None).unwrap();
            break;
        }
        copied += n;

        let content = (&buf[..n]).to_owned();
        debug!("request_sender {:?}", Some(content.len()));
        request_raw_delivery.request_raw_sender.send(Some(content)).unwrap();
    }
}

async fn request_raw_transport(mut request_parser_delivery: RequestParserDelivery) {
    loop {
        let content = request_parser_delivery.request_raw_receiver.recv().await.unwrap();
        match content {
            Some(content) => {
                request_parser_delivery.request_parsed_sender.send(RequestParsedData {
                    protocol: Protocol::RAW,
                    content:  RequestParsedContent::Content(content),
                }).await.unwrap();
            }
            None => {
                request_parser_delivery.request_parsed_sender.send(RequestParsedData {
                    protocol: Protocol::RAW,
                    content:  RequestParsedContent::Eof,
                }).await.unwrap();
                break;
            }
        }
    }
}

async fn proxy_to_server(mut orig_write: OwnedWriteHalf, mut request_transport_delivery: RequestTransportDelivery) {
    request_transport_delivery.response_protocol_sender.send(Protocol::RAW).unwrap();
        loop {
            let data = request_transport_delivery.request_parsed_receiver.recv().await.unwrap();
            match data.protocol {
                Protocol::RAW => match data.content {
                    RequestParsedContent::Content(content) => {
                        let n = orig_write.write(&content).await.unwrap();
                        if n == 0 {
                            panic!("Write zero");
                        }
                    }
                    RequestParsedContent::Eof => break,
                    RequestParsedContent::ParseFailed => debug!("protocol is not raw"),
                }
                _ => {}
            }
        }
        orig_write.shutdown().await.unwrap();
}
