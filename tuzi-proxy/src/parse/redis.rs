use crate::{
    error::TuziResult,
    parse::{
        ProtocolParsable, ReceiveParser, RequestParsedContent, RequestParsedData,
        RequestParserDelivery, ResponseParserDelivery,
    },
};
use async_trait::async_trait;
use nom::{
    branch::alt,
    bytes::streaming::{is_not, tag, take},
    character::streaming::{crlf, digit1},
    combinator::map,
    sequence::{delimited, terminated},
    IResult,
};
use std::{
    str,
    sync::{
        atomic::{AtomicBool, Ordering},
        Once,
    },
};
use tokio::io::{copy, AsyncWriteExt};
use tracing::info;
use std::sync::Arc;
use crate::collect::{Collectable, Record};
use tokio::sync::mpsc;

enum ReqOrResp {
    Req(Request),
    Resp(Response),
}

struct Request {
    command: String,
}

struct Response {
    success: bool,
    result: String,
}

pub struct Collector {
    collectable: Arc<dyn Collectable>,
    receiver: mpsc::Receiver<ReqOrResp>,
}

pub struct Parser {
    collect_sender: mpsc::Sender<ReqOrResp>,
}

impl Parser {
    pub async fn new(collectable: Arc<dyn Collectable>) -> Self {
        let (sender, receiver) = mpsc::channel(16);
        let collector = Collector {
            collectable,
            receiver,
        };
        // tokio::spawn()
        Self {
            collect_sender: sender,
        }
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

        loop {
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
        }

        Ok(())
    }

    async fn parse_response(&self, mut delivery: ResponseParserDelivery) -> TuziResult<()> {
        let mut parser = ReceiveParser::new(
            delivery.reader.exists_content.clone().unwrap_or_default(),
            &mut delivery.reader,
        );
        let (success, text) = parser.parse_and_recv(resp).await?;

        info!(?success, ?text, "redis response");

        if !parser.recv_content_ref().is_empty() {
            delivery
                .client_write
                .write(parser.recv_content_ref())
                .await
                .unwrap();
        }

        copy(&mut delivery.reader.server_read, &mut delivery.client_write)
            .await
            .unwrap();
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
