use crate::{
    error::TuziResult,
    parse::{
        ProtocolParsable, ReceiveParser, RequestParsedContent, RequestParsedData,
        RequestParserDelivery, ResponseParserDelivery,
    },
};
use async_trait::async_trait;
use nom::{
    bytes::streaming::{tag, take},
    character::streaming::{crlf, digit1},
    combinator::map,
    sequence::{delimited, terminated},
    IResult,
};
use std::str;
use tokio::io::{copy, AsyncWriteExt};
use tracing::info;
use std::sync::Once;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::Ordering;

pub struct Parser;

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
            info!(count, "redis args count");

            let mut args = Vec::new();
            for _ in 0..count {
                let arg = parser.parse_and_recv(req_arg).await?;
                args.push(arg);
            }
            info!(?args, "redis arg: ");

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
        if let Some(content) = delivery.reader.exists_content {
            delivery.client_write.write(&content).await.unwrap();
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
