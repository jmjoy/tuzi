use crate::{
    detect::detect_protocol,
    tcp::{orig_dst_addr, Addrs},
};
use std::{sync::Once, time::SystemTime};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    try_join,
};
use tracing::{info, instrument};

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
    let mut orig = TcpStream::connect(addrs.orig_dst).await.unwrap();

    let (mut socket_read, mut socket_write) = socket.split();
    let (mut orig_read, mut orig_write) = orig.split();

    let client_to_server = async {
        let mut buf = [0; 2048];
        let mut copied = 0;

        let once = Once::new();

        loop {
            let n = socket_read.read(&mut buf).await.unwrap();
            if n == 0 {
                break;
            }
            copied += n;

            once.call_once(|| match detect_protocol(&buf[..n]) {
                Some(info) => info!(?info, "protocol: http"),
                None => info!("protocol: unknown"),
            });

            let n = orig_write.write(&buf[..n]).await.unwrap();
            if n == 0 {
                panic!("Write zero");
            }
        }

        info!(copied);
        orig_write.shutdown().await
    };

    let server_to_client = async {
        let copied = tokio::io::copy(&mut orig_read, &mut socket_write).await?;
        info!(copied);
        socket_write.shutdown().await
    };

    try_join!(client_to_server, server_to_client).unwrap();

    let delay = SystemTime::now()
        .duration_since(context.start_time)
        .unwrap();

    info!(delay = ?delay);

    Ok(())
}
