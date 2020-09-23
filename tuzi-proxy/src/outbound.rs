use crate::tcp::{orig_dst_addr, Addrs};
use std::time::SystemTime;
use tokio::{
    io::AsyncWriteExt,
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

            handle(addrs, socket, context).await;
        });
    }
}

#[instrument(name = "outbound::handle", skip(socket, context))]
async fn handle(addrs: Addrs, mut socket: TcpStream, context: Context) -> anyhow::Result<()> {
    let mut original = TcpStream::connect(addrs.orig_dst).await.unwrap();

    let (mut ri, mut wi) = socket.split();
    let (mut ro, mut wo) = original.split();

    let client_to_server = async {
        let copied = tokio::io::copy(&mut ri, &mut wo).await?;
        info!(copied);
        wo.shutdown().await
    };

    let server_to_client = async {
        let copied = tokio::io::copy(&mut ro, &mut wi).await?;
        info!(copied);
        wi.shutdown().await
    };

    try_join!(client_to_server, server_to_client).unwrap();

    let delay = SystemTime::now()
        .duration_since(context.start_time)
        .unwrap();

    info!(delay = ?delay);

    Ok(())
}
