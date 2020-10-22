use crate::{
    collect::{Collectable, Record},
    error::{TuziError, TuziResult},
    parse::{
        ProtocolParsable, ReceiveParser, RequestParsedContent, RequestParsedData,
        RequestParserDelivery, ResponseParserDelivery,
    },
    wait::WaitGroup,
};
use async_trait::async_trait;
use futures::StreamExt;
use nom::{
    branch::alt,
    bytes::streaming::{is_not, tag, take},
    character::streaming::{crlf, digit1},
    combinator::map,
    lib::std::collections::HashMap,
    multi::many_till,
    sequence::{delimited, terminated},
    IResult, Needed,
};
use std::{
    net::SocketAddr,
    str,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Once,
    },
    time::{Instant, SystemTime},
};
use tokio::{
    io::{copy, AsyncWriteExt},
    sync::{mpsc, Mutex},
};
use tracing::info;

#[derive(Debug)]
enum CollectMeta {
    Addrs(Addrs),
    Request(Request),
    Response(Response),
    End,
}

#[derive(Debug)]
struct Addrs {
    client_addr: SocketAddr,
    server_addr: SocketAddr,
}

#[derive(Debug)]
struct Request {
    now: SystemTime,
    command: String,
}

#[derive(Debug)]
struct Response {
    now: SystemTime,
    success: bool,
    result: String,
}

pub struct Collector {
    protocol: &'static str,
    collectable: Arc<dyn Collectable>,
    receiver: mpsc::Receiver<CollectMeta>,
    wg: WaitGroup,
}

impl Collector {
    async fn listening(mut self) {
        let mut sess_addrs = None;
        let mut sess_request = None;

        loop {
            match self.receiver.recv().await.unwrap() {
                CollectMeta::Addrs(addrs) => {
                    sess_addrs = Some(addrs);
                }
                CollectMeta::Request(request) => match sess_request {
                    Some(_) => panic!("Already has request"),
                    None => sess_request = Some(request),
                },
                CollectMeta::Response(response) => match sess_request {
                    Some(ref sess_request) => {
                        let mut request_tags = HashMap::new();
                        request_tags.insert("params".to_string(), sess_request.command.clone());

                        let mut response_tags = HashMap::new();
                        response_tags.insert("result".to_string(), response.result.clone());

                        let record = Record {
                            protocol: self.protocol,
                            success: response.success,
                            start_time: sess_request.now,
                            end_time: response.now,
                            client_addr: sess_addrs.as_ref().unwrap().client_addr.clone(),
                            server_addr: sess_addrs.as_ref().unwrap().server_addr.clone(),
                            endpoint: "".to_string(),
                            request: Some(request_tags),
                            response: Some(response_tags),
                        };

                        self.collectable.collect(record).await;
                    }
                    None => panic!("No pre request"),
                },
                CollectMeta::End => {
                    break;
                }
            }
        }

        drop(self.wg);
    }
}

pub struct Parser {
    collect_sender: Mutex<mpsc::Sender<CollectMeta>>,
}

impl Parser {
    pub async fn new(collectable: Arc<dyn Collectable>, wg: WaitGroup) -> Self {
        let (sender, receiver) = mpsc::channel(16);
        let parser = Self {
            collect_sender: Mutex::new(sender),
        };
        let collector = Collector {
            protocol: parser.protocol(),
            collectable,
            receiver,
            wg,
        };
        tokio::spawn(collector.listening());
        parser
    }
}

#[async_trait]
impl ProtocolParsable for Parser {
    fn protocol(&self) -> &'static str {
        "redis"
    }

    async fn parse_request(&self, mut delivery: RequestParserDelivery) -> TuziResult<()> {
        let mut parser = ReceiveParser::new(Vec::new(), &mut delivery.request_raw_receiver);
        let sent = AtomicBool::new(false);

        self.collect_sender
            .lock()
            .await
            .send(CollectMeta::Addrs(Addrs {
                client_addr: delivery.client_addr,
                server_addr: delivery.server_addr,
            }))
            .await
            .unwrap();

        loop {
            let now = SystemTime::now();

            let count = parser.parse_and_recv(args_count).await?;

            let mut args = Vec::new();
            for _ in 0..count {
                let arg = parser.parse_and_recv(req_arg).await?;
                args.push(arg);
            }

            info!(?count, ?args, "redis request");

            if !sent.load(Ordering::SeqCst) {
                sent.store(true, Ordering::SeqCst);

                delivery
                    .request_parsed_sender
                    .send(RequestParsedData {
                        protocol: self.protocol(),
                        content: RequestParsedContent::Raw,
                    })
                    .await
                    .unwrap();
            }

            self.collect_sender
                .lock()
                .await
                .send(CollectMeta::Request(Request {
                    now,
                    command: args.join(" "),
                }))
                .await
                .unwrap();
        }

        Ok(())
    }

    async fn parse_response(&self, mut delivery: ResponseParserDelivery) -> TuziResult<()> {
        let mut parser = ReceiveParser::new(
            delivery.reader.exists_content.clone().unwrap_or_default(),
            &mut delivery.reader,
        );

        loop {
            let now = SystemTime::now();

            let (success, text) = match parser.parse_and_recv(resp).await {
                Ok(x) => x,
                Err(TuziError::Nom(nom::Err::Incomplete(Needed::Unknown))) => break,
                Err(e) => return Err(e),
            };

            info!(?success, ?text, "redis response");

            if !parser.recv_content_ref().is_empty() {
                delivery
                    .client_write
                    .write(parser.recv_content_ref())
                    .await
                    .unwrap();

                parser.clear_recv_content();
            }

            self.collect_sender
                .lock()
                .await
                .send(CollectMeta::Response(Response {
                    now,
                    success,
                    result: text,
                }))
                .await
                .unwrap();
        }
        Ok(())
    }
}

fn args_count(input: &[u8]) -> IResult<&[u8], usize> {
    delimited(
        tag("*"),
        map(digit1, |s| str::from_utf8(s).unwrap().parse().unwrap()),
        crlf,
    )(input)
}

fn req_arg(input: &[u8]) -> IResult<&[u8], String> {
    let (input, len) = delimited(
        tag("$"),
        map(digit1, |s| {
            str::from_utf8(s).unwrap().parse::<usize>().unwrap()
        }),
        crlf,
    )(input)?;

    let (input, arg) = terminated(take(len), crlf)(input)?;
    let arg = str::from_utf8(arg).unwrap().to_owned();

    Ok((input, arg))
}

fn resp(input: &[u8]) -> IResult<&[u8], (bool, String)> {
    alt((resp_simple, resp_error))(input)
}

fn resp_simple(input: &[u8]) -> IResult<&[u8], (bool, String)> {
    map(delimited(tag("+"), is_not("\r\n"), crlf), |s: &[u8]| {
        (true, String::from_utf8(s.to_vec()).unwrap())
    })(input)
}

fn resp_error(input: &[u8]) -> IResult<&[u8], (bool, String)> {
    map(delimited(tag("-"), is_not("\r\n"), crlf), |s: &[u8]| {
        (false, String::from_utf8(s.to_vec()).unwrap())
    })(input)
}
