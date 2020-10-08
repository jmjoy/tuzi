pub mod http1;
pub mod redis;

use crate::error::{ITuziResult, TuziError, TuziResult};
use anyhow::anyhow;
use async_trait::async_trait;
use nom::{error::ErrorKind, IResult, Needed};
use tokio::sync::{
    broadcast::{self},
    mpsc, oneshot,
};

#[derive(Debug)]
pub enum Protocol {
    HTTP1,
    REDIS,
    MYSQL,
    RAW,
}

#[derive(Debug)]
pub struct RequestParsedData {
    pub protocol: Protocol,
    pub content: RequestParsedContent,
}

#[derive(Debug)]
pub enum RequestParsedContent {
    Content(Vec<u8>),
    Eof,
    ParseFailed,
}

pub struct RequestRawDelivery {
    pub request_raw_sender: broadcast::Sender<Option<Vec<u8>>>,
}

pub struct RequestParserDelivery {
    pub request_raw_receiver: broadcast::Receiver<Option<Vec<u8>>>,
    pub request_parsed_sender: mpsc::Sender<RequestParsedData>,
}

pub struct RequestTransportDelivery {
    pub request_parsed_receiver: mpsc::Receiver<RequestParsedData>,
    pub response_protocol_sender: oneshot::Sender<Protocol>,
}

pub struct ResponseProtocolDelivery {
    response_protocol_receiver: oneshot::Receiver<Protocol>,
}

pub fn delivery(
    count: usize,
) -> (
    RequestRawDelivery,
    Vec<RequestParserDelivery>,
    RequestTransportDelivery,
    ResponseProtocolDelivery,
) {
    let (request_raw_sender, _) = broadcast::channel(16);
    let (request_parsed_sender, request_parsed_receiver) = mpsc::channel(16);
    let (response_protocol_sender, response_protocol_receiver) = oneshot::channel();

    let mut request_parsed_deliveries = Vec::new();
    for _ in 0..count {
        let request_raw_receiver = request_raw_sender.subscribe();
        let request_parsed_sender = request_parsed_sender.clone();
        request_parsed_deliveries.push(RequestParserDelivery {
            request_raw_receiver,
            request_parsed_sender,
        });
    }

    let request_raw_delivery = RequestRawDelivery { request_raw_sender };

    let request_transport_delivery = RequestTransportDelivery {
        request_parsed_receiver,
        response_protocol_sender,
    };

    let response_protocol_delivery = ResponseProtocolDelivery {
        response_protocol_receiver,
    };

    (
        request_raw_delivery,
        request_parsed_deliveries,
        request_transport_delivery,
        response_protocol_delivery,
    )
}

#[async_trait]
pub trait Receivable<T> {
    async fn receive(&mut self) -> TuziResult<T>;
}

#[async_trait]
impl<T: Send> Receivable<T> for mpsc::Receiver<T> {
    async fn receive(&mut self) -> TuziResult<T> {
        match self.recv().await {
            Some(item) => Ok(item),
            None => Err(TuziError::ReceiveClosed),
        }
    }
}

#[async_trait]
impl<T: Send + Clone> Receivable<T> for broadcast::Receiver<T> {
    async fn receive(&mut self) -> TuziResult<T> {
        match self.recv().await {
            Ok(item) => Ok(item),
            Err(broadcast::RecvError::Closed) => Err(TuziError::ReceiveClosed),
            Err(e @ broadcast::RecvError::Lagged(_)) => Err(anyhow!(e).into()),
        }
    }
}

pub struct Parser<'a, R: Receivable<Option<Vec<u8>>>> {
    parse_content: Vec<u8>,
    recv_content: Vec<u8>,
    receiver: &'a mut R,
}

impl<'a, R: Receivable<Option<Vec<u8>>>> Parser<'a, R> {
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
