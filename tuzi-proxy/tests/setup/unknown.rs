use crate::setup::Context;
use futures::{
    future::{select, Either},
    pin_mut,
};
use std::{future::Future, net::SocketAddr, pin::Pin};
use tokio::{
    io::{copy, AsyncReadExt, AsyncWriteExt},
    net::{TcpListener, TcpStream},
    sync::mpsc,
};

pub async fn server(
    signal: impl Future<Output = ()>,
    addr_fn: impl FnOnce(SocketAddr) -> Pin<Box<dyn Future<Output = ()> + Send>>,
) {
    let mut listener = TcpListener::bind("localhost:0").await.unwrap();
    addr_fn(listener.local_addr().unwrap()).await;

    let (mut shutdown_tx, mut shutdown_rx) = mpsc::channel(1);

    let join = tokio::spawn(async move {
        loop {
            let shutdown_fut = shutdown_rx.recv();
            let accept_fut = listener.accept();
            pin_mut!(shutdown_fut, accept_fut);

            match select(shutdown_fut, accept_fut).await {
                Either::Left((r, _)) => {
                    r.unwrap();
                    break;
                }
                Either::Right((r, _)) => {
                    let (socket, _) = r.unwrap();
                    tokio::spawn(async move {
                        let (mut read, mut write) = socket.into_split();
                        copy(&mut read, &mut write).await.unwrap();
                        write.shutdown().await.unwrap();
                    });
                }
            }
        }
    });

    signal.await;
    shutdown_tx.send(()).await.unwrap();
    join.await.unwrap();
}

pub async fn test_1(context: Context) {
    let addr = context.protocol_mapping.get("proxy").await;
    let mut sock = TcpStream::connect(addr).await.unwrap();
    context.insert_protocol_client_port("unknown", &sock).await;

    sock.write(b"hello world").await.unwrap();

    let mut buf = [0; 1024];
    let n = sock.read(&mut buf).await.unwrap();

    assert_eq!(&buf[..n], b"hello world");
}

pub async fn test_2(context: Context) {
    let addr = context.protocol_mapping.get("proxy").await;
    let mut sock = TcpStream::connect(addr).await.unwrap();
    context.insert_protocol_client_port("unknown", &sock).await;

    sock.write(b"GET THE WORLD").await.unwrap();

    let mut buf = [0; 1024];
    let n = sock.read(&mut buf).await.unwrap();

    assert_eq!(&buf[..n], b"GET THE WORLD");
}
