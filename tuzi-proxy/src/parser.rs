use nom::{error::ErrorKind, IResult, Needed};
use std::future::Future;
use tokio::{sync::{broadcast::{self}, mpsc}};
use tracing::debug;
use async_trait::async_trait;
use anyhow::anyhow;
use crate::error::{TuziError, TuziResult};

#[async_trait]
pub trait Receiveable<T> {
    async fn receive(&mut self) -> TuziResult<T>;
}

#[async_trait]
impl<T: Send> Receiveable<T> for mpsc::Receiver<T> {
    async fn receive(&mut self) -> TuziResult<T> {
        match self.recv().await {
            Some(item) => Ok(item),
            None => Err(TuziError::ReceiveClosed),
        }
    }
}

#[async_trait]
impl<T: Send + Clone> Receiveable<T> for broadcast::Receiver<T> {
    async fn receive(&mut self) -> TuziResult<T> {
        match self.recv().await {
            Ok(item) => Ok(item),
            Err(broadcast::RecvError::Closed) => Err(TuziError::ReceiveClosed),
            Err(e @ broadcast::RecvError::Lagged(_)) => Err(anyhow!(e).into()),
        }
    }
}

pub struct Parser {
    content: Vec<u8>,
}

pub async fn parse_and_recv<T>(
    mut content: Vec<u8>,
    receiver: &mut impl Receiveable<Option<Vec<u8>>>,
    f: impl Fn(&[u8]) -> IResult<&[u8], T>,
) -> Result<(Vec<u8>, Vec<u8>, T), nom::Err<(Vec<u8>, ErrorKind)>> {
    let mut recv_content = Vec::new();
    loop {
        match f(&content) {
            Ok((b, k)) => return Ok((b.to_owned(), recv_content, k)),
            Err(e) => match e {
                nom::Err::Incomplete(_) => {}
                nom::Err::Error((b, k)) => return Err(nom::Err::Error((b.to_owned(), k))),
                nom::Err::Failure((b, k)) => return Err(nom::Err::Failure((b.to_owned(), k))),
            },
        }
        let b = receiver.recv().await.unwrap();
        let b = match b {
            Some(b) => b,
            None => return Err(nom::Err::Incomplete(Needed::Unknown)),
        };
        content.extend_from_slice(&b);
        recv_content.extend_from_slice(&b);
    }
}
