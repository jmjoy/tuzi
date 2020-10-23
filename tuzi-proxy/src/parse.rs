pub mod http1;
pub mod redis;

use crate::{
    error::{TuziError, TuziResult},
    io::Receivable,
};
use anyhow::anyhow;
use async_trait::async_trait;
use nom::{IResult, Needed};
use std::{io, mem::replace, net::SocketAddr};
use tokio::{
    io::{AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::tcp::{OwnedReadHalf, OwnedWriteHalf},
    sync::{
        broadcast::{self},
        mpsc,
    },
};
use tracing::debug;
use tokio::task::JoinHandle;

pub type Protocol = &'static str;

#[derive(Debug)]
pub struct RequestParsedData {
    pub protocol: Protocol,
    pub content: RequestParsedContent,
}

#[derive(Debug)]
pub enum RequestParsedContent {
    Content(Vec<u8>),
    Eof,
    Raw,
    Failed,
}

pub struct ClientToProxyDelivery {
    pub request_raw_sender: broadcast::Sender<Option<Vec<u8>>>,
}

pub struct RequestParserDelivery {
    pub request_raw_receiver: broadcast::Receiver<Option<Vec<u8>>>,
    pub request_parsed_sender: mpsc::Sender<RequestParsedData>,
    pub client_addr: SocketAddr,
    pub server_addr: SocketAddr,
}

pub struct ProxyToServerDelivery {
    pub request_raw_receiver: broadcast::Receiver<Option<Vec<u8>>>,
    pub request_parsed_receiver: mpsc::Receiver<RequestParsedData>,
    pub response_protocol_sender: mpsc::Sender<Option<Protocol>>,
}

pub struct ServerToProxyDelivery {
    pub response_protocol_receiver: mpsc::Receiver<Option<Protocol>>,
}

pub fn delivery(
    count: usize,
    client_addr: SocketAddr,
    server_addr: SocketAddr,
) -> (
    ClientToProxyDelivery,
    Vec<RequestParserDelivery>,
    ProxyToServerDelivery,
    ServerToProxyDelivery,
) {
    assert!(count > 0);

    let (request_raw_sender, request_raw_receiver) = broadcast::channel(16);
    let (request_parsed_sender, request_parsed_receiver) = mpsc::channel(16);
    let (response_protocol_sender, response_protocol_receiver) = mpsc::channel(16);

    let mut parser_deliveries = Vec::new();

    for _ in 0..count {
        parser_deliveries.push(RequestParserDelivery {
            request_raw_receiver: request_raw_sender.subscribe(),
            request_parsed_sender: request_parsed_sender.clone(),
            client_addr,
            server_addr,
        });
    }

    (
        ClientToProxyDelivery { request_raw_sender },
        parser_deliveries,
        ProxyToServerDelivery {
            request_raw_receiver,
            request_parsed_receiver,
            response_protocol_sender,
        },
        ServerToProxyDelivery {
            response_protocol_receiver,
        },
    )
}

pub struct ResponseParserReader {
    pub exists_content: Option<Vec<u8>>,
    pub server_read: OwnedReadHalf,
}

pub struct ResponseParserDelivery {
    pub reader: ResponseParserReader,
    pub client_write: OwnedWriteHalf,
}

#[async_trait]
pub trait ProtocolParsable: Send + Sync {
    fn protocol(&self) -> Protocol;
    fn daemon(&self) -> Option<JoinHandle<()>>;
    async fn parse_request(&self, mut delivery: RequestParserDelivery) -> TuziResult<()>;
    async fn parse_response(&self, mut delivery: ResponseParserDelivery) -> TuziResult<()>;
}

pub struct ReceiveParser<'a, R: Receivable<Option<Vec<u8>>>> {
    parse_content: Vec<u8>,
    recv_content: Vec<u8>,
    receiver: &'a mut R,
}

impl<'a, R: Receivable<Option<Vec<u8>>>> ReceiveParser<'a, R> {
    pub fn new(init_parse_content: Vec<u8>, receiver: &'a mut R) -> Self {
        Self {
            parse_content: init_parse_content,
            recv_content: Vec::new(),
            receiver,
        }
    }

    #[inline]
    pub fn recv_content_ref(&self) -> &[u8] {
        &self.recv_content
    }

    pub fn clear_recv_content(&mut self) {
        self.recv_content.clear()
    }

    pub async fn parse_and_recv<T>(
        &mut self,
        f: impl Fn(&[u8]) -> IResult<&[u8], T>,
    ) -> TuziResult<T> {
        loop {
            match f(&self.parse_content) {
                Ok((b, t)) => {
                    self.parse_content = b.to_owned();
                    return Ok(t);
                }
                Err(e) => match e {
                    nom::Err::Incomplete(_) => {}
                    e => return Err(e.into()),
                },
            }
            let b = self.receiver.receive().await?;
            let b = match b {
                Some(b) => b,
                None => return Err(TuziError::Nom(nom::Err::Incomplete(Needed::Unknown))),
            };

            self.parse_content.extend_from_slice(&b);
            self.recv_content.extend_from_slice(&b);
        }
    }
}
