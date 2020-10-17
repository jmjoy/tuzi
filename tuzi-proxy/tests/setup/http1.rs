use crate::setup::Context;
use http::Method;
use hyper::{
    service::{make_service_fn, service_fn},
    Body, Request, Response, Server,
};
use std::{convert::Infallible, future::Future, net::TcpListener as StdTcpListener};
use tokio::sync::oneshot;
use tracing::{debug, instrument};

#[instrument(name = "test-server-http1:handle", skip(req))]
async fn handle(req: Request<Body>) -> Result<Response<Body>, Infallible> {
    debug!("method: {}, path: {}", req.method(), req.uri().path());
    if req.method() == Method::GET {
        if req.uri().path() == "/" {
            return Ok(Response::new("Hello, World!".into()));
        }
    } else if req.method() == Method::POST {
        if req.uri().path() == "/echo" {
            let body = req.into_body();
            return Ok(Response::new(body));
        }
    }
    Ok(Response::builder().status(404).body("".into()).unwrap())
}

pub async fn server(listener: StdTcpListener, signal: impl Future<Output = ()>) {
    let make_svc = make_service_fn(|_conn| async { Ok::<_, Infallible>(service_fn(handle)) });
    let server = Server::from_tcp(listener).unwrap().serve(make_svc);
    server.with_graceful_shutdown(signal).await.unwrap();
}

pub async fn test_index(context: Context) {
    let client = context.build_client();
    let uri = context.build_http_uri("/").await;
    let response = client.get(uri).await.unwrap();
    let status = response.status();
    let content = hyper::body::to_bytes(response).await.unwrap();
    let content = String::from_utf8(content.to_vec()).unwrap();

    assert_eq!(status, 200);
    assert_eq!(content, "Hello, World!");
}

pub async fn test_echo(context: Context) {
    let client = context.build_client();
    let uri = context.build_http_uri("/echo").await;
    let request = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .body("TEST ECHO".into())
        .unwrap();
    let response = client.request(request).await.unwrap();

    let status = response.status();
    let content = hyper::body::to_bytes(response).await.unwrap();
    let content = String::from_utf8(content.to_vec()).unwrap();

    assert_eq!(status, 200);
    assert_eq!(content, "TEST ECHO");
}
