mod setup;

use crate::setup::{setup, Context};
use http::{Method, Request};
use hyper::body;
use nom::lib::std::alloc::handle_alloc_error;
use std::thread;
use tokio::time::delay_for;

#[tokio::test]
async fn test_all() {
    let (context, mut handles) = setup().await;
    handles.add_test(test_http1_index(context.clone())).await;
    handles.add_test(test_http1_echo(context.clone())).await;
    handles.wait().await;
}

async fn test_http1_index(context: Context) {
    let client = context.build_client();
    let uri = context.build_http_uri("/").await;
    let response = client.get(uri).await.unwrap();
    let status = response.status();
    let content = body::to_bytes(response).await.unwrap();
    let content = String::from_utf8(content.to_vec()).unwrap();

    assert_eq!(status, 200);
    assert_eq!(content, "Hello, World!");
}

async fn test_http1_echo(context: Context) {
    let client = context.build_client();
    let uri = context.build_http_uri("/echo").await;
    let request = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .body("TEST ECHO".into())
        .unwrap();
    let response = client.request(request).await.unwrap();

    let status = response.status();
    let content = body::to_bytes(response).await.unwrap();
    let content = String::from_utf8(content.to_vec()).unwrap();

    assert_eq!(status, 200);
    assert_eq!(content, "TEST ECHO");
}
