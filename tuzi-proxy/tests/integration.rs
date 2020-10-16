mod setup;

use crate::setup::{http1, redis, setup, Context};
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
    handles.add_test(redis::test_get_set(context.clone())).await;
    handles.wait().await;
}
