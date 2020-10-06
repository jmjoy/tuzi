use crate::{
    detect::{self, detect_http, detect_protocol_with_term, http_1, Detection},
    parser::Parser,
    tcp::{orig_dst_addr, Addrs},
};
use http_1::respnose_begin;
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

#[instrument(name = "outbound::run")]
pub async fn run() -> anyhow::Result<()> {
    let mut listener = TcpListener::bind("127.0.0.1:4140").await?;

    info!("after bind");

    loop {
        let (socket, _) = listener.accept().await?;

        let context = Context::new();

        info!("after accept");

        tokio::spawn(async move {
            let peer = socket.peer_addr().unwrap();
            let orig_dst = orig_dst_addr(&socket).unwrap();
            handle(peer, orig_dst, socket, context).await.unwrap();
        });
    }
}

#[instrument(name = "outbound::handle", skip(socket, context))]
async fn handle(
    peer: SocketAddr,
    orig_dst: SocketAddr,
    mut socket: TcpStream,
    context: Context,
) -> anyhow::Result<()> {
    info!("before connect to orig_dst");
    let mut orig = TcpStream::connect(orig_dst).await.unwrap();

    let (mut socket_read, mut socket_write) = socket.split();
    let (mut orig_read, mut orig_write) = orig.split();

    let (request_sender, _) = broadcast::channel(16);
    let (request_protocol_sender, mut request_protocol_receiver): (mpsc::Sender<Detection>, _) =
        mpsc::channel(16);
    let (mut response_protocol_sender, mut response_protocol_receiver) = mpsc::channel(16);

    let client_to_server = async {
        let mut buf = [0; 4096];
        let mut copied = 0;

        loop {
            let n = socket_read.read(&mut buf).await.unwrap();
            if n == 0 {
                debug!("request_sender None");
                request_sender.send(None).unwrap();
                break;
            }
            copied += n;

            let buf0 = (&buf[..n]).to_owned();
            debug!("request_sender {:?}", Some(buf.len()));
            request_sender.send(Some(buf0)).unwrap();
        }
    };

    let client_to_server_output = async move {
        let mut protocol = None;
        loop {
            let detection = request_protocol_receiver.recv().await.unwrap();

            match protocol {
                Some(protocol) => {
                    if protocol != detection.protocol {
                        panic!("multi protocol detection!");
                    }
                }
                None => {
                    protocol = Some(detection.protocol);
                }
            }

            match detection.data {
                Some(recv_content) => {
                    debug!("recv_content size: {}", recv_content.len());
                    if recv_content.len() != 0 {
                        let n = orig_write.write(&recv_content).await.unwrap();
                        if n == 0 {
                            panic!("Write zero");
                        }
                    }
                }
                None => {
                    break;
                }
            }
        }

        // info!(copied);
        orig_write.shutdown().await.unwrap();
    };

    let mut request_receiver = request_sender.subscribe();
    let _http_detect = async move {
        let mut header = Vec::new();

        let info = loop {
            let input = request_receiver.recv().await.unwrap();
            let input = match input {
                Some(input) => input,
                None => break None,
            };
            header.extend_from_slice(&input);

            match detect::http(&header) {
                Ok((_, info)) => break Some(info),
                Err(e) => match e {
                    nom::Err::Incomplete(..) => {}
                    nom::Err::Error((_, kind)) => {
                        warn!(?kind, "detect http");
                    }
                    nom::Err::Failure((_, kind)) => {
                        error!(?kind, "detect http");
                    }
                },
            }
        };

        info!(?info, "detct http");
    };

    let mut request_receiver = request_sender.subscribe();
    let _redis_detect = async move {
        let input = request_receiver.recv().await.unwrap();
        let input = match input {
            Some(input) => input,
            None => return,
        };
        match detect::redis_args_count(&input) {
            Ok((remain, _)) => {
                info!("detect redis: {}", std::str::from_utf8(remain).unwrap());
            }
            Err(e) => match e {
                nom::Err::Incomplete(..) => {}
                nom::Err::Error((_, kind)) => {
                    warn!(?kind, "detect redis");
                }
                nom::Err::Failure((_, kind)) => {
                    error!(?kind, "detect redis");
                }
            },
        }
    };

    let request_receiver = request_sender.subscribe();
    let request_protocol_sender = request_protocol_sender.clone();
    let http_1_detect = async move {
        if let Err(_e) = http_1::detect(request_receiver, request_protocol_sender).await {
            warn!("is not http protocol");
        } else {
            response_protocol_sender.send("http_1").await.unwrap();
        }
    };

    let (mut response_sender, mut response_receiver) = mpsc::channel(16);
    let server_to_client = async move {
        let mut buf = [0; 4096];
        loop {
            let n = orig_read.read(&mut buf).await.unwrap();
            if n == 0 {
                let _ = response_sender.send(None).await;
                break;
            } else {
                let n = socket_write.write(&buf[..n]).await.unwrap();
                if n == 0 {
                    panic!("Write zero");
                }
                let _ = response_sender.send(Some(Vec::from(&buf[..n]))).await;
            }
        }
        socket_write.shutdown().await.unwrap();
    };

    let server_to_client_analyzer = async move {
        let mut content = Vec::new();

        let protocol = loop {
            let buf = response_receiver.recv().await.unwrap();
            let buf = match buf {
                Some(buf) => buf,
                None => break None,
            };
            content.extend_from_slice(&buf);

            match response_protocol_receiver.try_recv() {
                Ok(protocol) => break Some(protocol),
                Err(TryRecvError::Empty) => {}
                Err(e @ TryRecvError::Closed) => panic!(e),
            }
        };

        if let Some(protocol) = protocol {
            match protocol {
                "http_1" => {
                    let mut parser = Parser::new(content, &mut response_receiver);
                    match parser.parse_and_recv(respnose_begin).await {
                        Ok(begin) => info!(?begin, "http response"),
                        Err(_) => error!("http_1 parse failed"),
                    }
                }
                _ => {}
            }
        }
    };

    join!(
        client_to_server,
        client_to_server_output,
        http_1_detect,
        server_to_client,
        server_to_client_analyzer,
    );

    let delay = SystemTime::now()
        .duration_since(context.start_time)
        .unwrap();

    info!(delay = ?delay);

    Ok(())
}
