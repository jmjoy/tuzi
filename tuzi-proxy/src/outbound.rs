use crate::{
    detect::detect_http,
    detect::Detection,
    detect::{self, detect_protocol_with_term},
    stream::ReaderBuffer,
    tcp::{orig_dst_addr, Addrs},
};
use std::{cell::RefCell, io::Cursor, sync::Once, time::SystemTime};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader, Seek},
    join,
    net::{TcpListener, TcpStream},
    sync::broadcast,
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
            let orig_dst = orig_dst_addr(&socket).unwrap();
            let addrs = Addrs {
                local: socket.local_addr().unwrap(),
                peer: socket.peer_addr().unwrap(),
                orig_dst,
            };

            handle(addrs, socket, context).await.unwrap();
        });
    }
}

#[instrument(name = "outbound::handle", skip(socket, context))]
async fn handle(addrs: Addrs, mut socket: TcpStream, context: Context) -> anyhow::Result<()> {
    info!("before connect to orig_dst");
    let mut orig = TcpStream::connect(addrs.orig_dst).await.unwrap();

    let (mut socket_read, mut socket_write) = socket.split();
    let (mut orig_read, mut orig_write) = orig.split();

    let (request_sender, _) = broadcast::channel(16);

    let client_to_server = async {
        let mut buf = [0; 4096];
        let mut copied = 0;

        // let once = Once::new();

        // let rb = ReaderBuffer::new();

        // loop {
        //     let reader_buffer = ReaderBuffer::new(&buffer);
        //     buffer.extend_from_slice(b"fuck");
        //     reader_buffer.take(1);
        // }

        // let protocol = detection.detect_protocol();
        // info!(?protocol, "detect protocol");

        // detection.reader_buffer().write_to(&mut orig_write).await.unwrap();

        loop {
            let n = socket_read.read(&mut buf).await.unwrap();
            if n == 0 {
                debug!("request_sender None");
                request_sender.send(None).unwrap();
                break;
            }
            copied += n;

            let buf = (&buf[..n]).to_owned();
            debug!("request_sender {:?}", Some(buf.len()));
            request_sender.send(Some(buf)).unwrap();

            // once.call_once(|| match detect_protocol(&buf[..n], &mut new_buf) {
            //     Some(info) => info!(?info, "protocol: http"),
            //     None => info!("protocol: unknown"),
            // });

            // if term == 0 {
            //     let protocol = detect_protocol_with_term(&buf);
            //     info!(?protocol, "detect protocol");
            // }

            // let write_buf = if new_buf.is_empty() {
            //     &buf[..n]
            // } else {
            //     &new_buf[..]
            // };
            // let n = orig_write.write(write_buf).await.unwrap();
            // if n == 0 {
            //     panic!("Write zero");
            // }

            // new_buf.clear();

            // term += 1;
        }
    };

    let mut request_receiver = request_sender.subscribe();
    let http_detect = async move {
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
    let redis_detect = async move {
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

    let mut request_receiver = request_sender.subscribe();
    let client_to_server_output = async move {
        loop {
            let request = request_receiver.recv().await.unwrap();
            debug!("receiver output size {:?}", request.as_ref().map(Vec::len));
            let request = match request {
                Some(input) => input,
                None => break,
            };
            let n = orig_write.write(&request).await.unwrap();
            if n == 0 {
                panic!("Write zero");
            }
        }

        // info!(copied);
        orig_write.shutdown().await.unwrap();
    };

    let server_to_client = async {
        let copied = tokio::io::copy(&mut orig_read, &mut socket_write)
            .await
            .unwrap();
        info!(copied);
        socket_write.shutdown().await.unwrap();
    };

    join!(
        client_to_server,
        server_to_client,
        http_detect,
        redis_detect,
        client_to_server_output,
    );

    let delay = SystemTime::now()
        .duration_since(context.start_time)
        .unwrap();

    info!(delay = ?delay);

    Ok(())
}
