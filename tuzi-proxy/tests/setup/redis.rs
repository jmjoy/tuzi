use std::{
    future::Future,
    net::{SocketAddr, ToSocketAddrs},
    pin::Pin,
    sync::mpsc as std_mpsc,
};
use testcontainers::{clients, images, Docker};
use tokio::{sync::oneshot, task::spawn_blocking};
use tuzi_proxy::waitgroup::{WaitGroup, Worker};

pub async fn server(
    worker: Worker,
    signal: impl Future<Output = ()>,
    addr_fn: impl FnOnce(SocketAddr) -> Pin<Box<dyn Future<Output = ()> + Send>>,
) {
    let (addr_tx, addr_rx) = oneshot::channel();
    let (shutdown_tx, shutdown_rx) = std_mpsc::channel();

    let join = spawn_blocking(move || {
        let docker = clients::Cli::default();
        let node = docker.run(images::redis::Redis::default());
        let host_port = node.get_host_port(6379).unwrap();

        let addr = format!("localhost:{}", host_port)
            .to_socket_addrs()
            .unwrap()
            .nth(0)
            .unwrap();
        addr_tx.send(addr).unwrap();

        shutdown_rx.recv().unwrap();
    });

    let addr = addr_rx.await.unwrap();
    addr_fn(addr).await;
    drop(worker);

    signal.await;
    shutdown_tx.send(()).unwrap();
    join.await.unwrap();
}
