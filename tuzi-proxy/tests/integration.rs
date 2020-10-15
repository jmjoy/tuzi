mod setup;

use crate::setup::{setup, Context, http1};
use http::{Method, Request};
use hyper::body;
use nom::lib::std::alloc::handle_alloc_error;
use std::thread;
use tokio::time::delay_for;

#[tokio::test]
async fn test_all() {
    let (context, mut handles) = setup().await;
    handles.add_test(http1::test_index(context.clone())).await;
    handles.add_test(http1::test_echo(context.clone())).await;
    handles.wait().await;
}

