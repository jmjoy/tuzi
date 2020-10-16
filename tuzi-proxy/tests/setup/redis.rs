use crate::setup::Context;
use redis::{
    aio::{ActualConnection, Connection},
    AsyncCommands, RedisError, RedisResult,
};
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

pub async fn build_aio_connection(context: Context) -> Connection {
    let addr = context.protocol_mapping.get("proxy").await;
    let client = redis::Client::open((format!("{}", addr.ip()), addr.port())).unwrap();
    let conn = client.get_async_connection().await.unwrap();
    let sock = match &conn.con {
        ActualConnection::TcpTokio(sock) => sock,
        _ => panic!("Redis connect is not tcp tokio"),
    };
    context.add_protocol_server_port("redis", sock).await;
    conn
}

pub async fn test_get_set(context: Context) {
    let mut conn = build_aio_connection(context.clone()).await;

    let result = async {
        conn.set("key1", b"foo").await?;

        redis::cmd("SET")
            .arg(&["key2", "bar"])
            .query_async(&mut conn)
            .await?;

        let result = redis::cmd("MGET")
            .arg(&["key1", "key2"])
            .query_async(&mut conn)
            .await?;

        Ok::<_, RedisError>(result)
    }
    .await;

    assert_eq!(result, Ok(("foo".to_string(), b"bar".to_vec())));
}
