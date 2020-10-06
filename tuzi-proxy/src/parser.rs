use crate::error::{ITuziResult, TuziError, TuziResult};
use anyhow::anyhow;
use async_trait::async_trait;
use nom::{error::ErrorKind, IResult, Needed};

use tokio::sync::{
    broadcast::{self},
    mpsc,
};

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

pub struct Parser<'a, R: Receiveable<Option<Vec<u8>>>> {
    parse_content: Vec<u8>,
    recv_content: Vec<u8>,
    receiver: &'a mut R,
}

impl<'a, R: Receiveable<Option<Vec<u8>>>> Parser<'a, R> {
    pub fn new(init_parse_content: Vec<u8>, receiver: &'a mut R) -> Self {
        Self {
            parse_content: init_parse_content,
            recv_content: Vec::new(),
            receiver,
        }
    }

    pub fn recv_content_ref(&self) -> &[u8] {
        &self.recv_content
    }

    pub fn clear_recv_content(&mut self) {
        self.recv_content.clear()
    }

    pub async fn parse_and_recv<T>(
        &mut self,
        f: impl Fn(&[u8]) -> ITuziResult<&[u8], T>,
    ) -> TuziResult<T> {
        loop {
            match f(&self.parse_content) {
                Ok((b, t)) => {
                    self.parse_content = b.to_owned();
                    return Ok(t);
                }
                Err(e) => match e {
                    nom::Err::Incomplete(_) => {}
                    nom::Err::Error(e) => return Err(e),
                    nom::Err::Failure(e) => return Err(e),
                },
            }
            let b = self.receiver.receive().await?;
            let b = match b {
                Some(b) => b,
                None => return Err(TuziError::ParseIncomplete),
            };

            self.parse_content.extend_from_slice(&b);
            self.recv_content.extend_from_slice(&b);
        }
    }
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
        let b = receiver.receive().await.unwrap();
        let b = match b {
            Some(b) => b,
            None => return Err(nom::Err::Incomplete(Needed::Unknown)),
        };
        content.extend_from_slice(&b);
        recv_content.extend_from_slice(&b);
    }
}
