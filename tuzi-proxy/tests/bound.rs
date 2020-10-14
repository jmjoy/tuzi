mod setup;

use hyper::body;

use crate::setup::setup_servers;
use http::{Method, Request};

#[tokio::test]
async fn test_http1_index() {
    let context = setup_servers().await;

    let client = context.build_client();
    let uri = context.build_http_uri("/").await;
    let response = client.get(uri).await.unwrap();
    let status = response.status();
    let content = body::to_bytes(response).await.unwrap();
    let content = String::from_utf8(content.to_vec()).unwrap();

    assert_eq!(status, 200);
    assert_eq!(content, "Hello, World!");
}

#[tokio::test]
async fn test_http1_echo() {
    let context = setup_servers().await;

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
