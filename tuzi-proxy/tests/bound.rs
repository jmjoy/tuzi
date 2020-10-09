mod http;

use hyper::{body, Client, Uri};
use std::{convert::TryFrom, net::TcpListener as StdTcpListener, str};
use tokio::{
    net::TcpListener,
    sync::oneshot,
    time::{delay_for, Duration},
};
use tuzi_proxy::{
    bound::{run_with_listener, ServerAddr},
    init_tracing,
};

#[tokio::test]
async fn test_run_with_listener() {
    init_tracing();

    let proxy_listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let proxy_addr = proxy_listener.local_addr().unwrap();

    let server_listener = StdTcpListener::bind("127.0.0.1:0").unwrap();
    let server_addr = server_listener.local_addr().unwrap();

    let (server_signal_tx, server_signal_rx) = oneshot::channel();
    let server_join = tokio::spawn(http::server(server_listener, server_signal_rx));

    tokio::spawn(run_with_listener(
        proxy_listener,
        ServerAddr::Manual(server_addr),
    ));

    let client = Client::new();
    let uri = format!("http://127.0.0.1:{}/", proxy_addr.port());
    let uri = Uri::try_from(uri).unwrap();
    let response = client.get(uri).await.unwrap();

    let status = response.status();
    let content = body::to_bytes(response).await.unwrap();
    let content = String::from_utf8(content.to_vec()).unwrap();

    assert_eq!(status, 200);
    assert_eq!(content, "Hello, World!");

    server_signal_tx.send(()).unwrap();
    server_join.await.unwrap();
}
