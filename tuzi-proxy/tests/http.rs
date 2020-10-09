use hyper::{
    service::{make_service_fn, service_fn},
    Body, Request, Response, Server,
};
use std::{
    convert::Infallible,
    net::{SocketAddr, TcpListener as StdTcpListener},
};
use tokio::sync::oneshot;

async fn handle(_: Request<Body>) -> Result<Response<Body>, Infallible> {
    Ok(Response::new("Hello, World!".into()))
}

pub async fn server(listener: StdTcpListener, signal: oneshot::Receiver<()>) {
    let make_svc = make_service_fn(|_conn| async { Ok::<_, Infallible>(service_fn(handle)) });

    let server = Server::from_tcp(listener)
        .unwrap()
        .serve(make_svc)
        .with_graceful_shutdown(async move {
            signal.await.unwrap();
        });

    server.await.unwrap();
}
