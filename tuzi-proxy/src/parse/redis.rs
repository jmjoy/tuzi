use crate::{
    error::{ITuziResult, TuziResult},
    parse::{
        ProtocolParsable, ReceiveParser, RequestParsedContent, RequestParsedData,
        RequestParserDelivery, ResponseParserDelivery,
    },
};
use async_trait::async_trait;
use nom::{
    bytes::streaming::tag,
    character::streaming::{crlf, digit1},
    combinator::map,
    sequence::delimited,
};
use std::str;
use tracing::info;

pub struct Parser;

#[async_trait]
impl ProtocolParsable for Parser {
    fn protocol(&self) -> &'static str {
        "redis"
    }

    async fn parse_request(&self, mut delivery: RequestParserDelivery) -> TuziResult<()> {
        let mut parser = ReceiveParser::new(Vec::new(), &mut delivery.request_raw_receiver);
        let count = parser.parse_and_recv(args_count).await?;
        info!(count, "redis args count");
        delivery
            .request_parsed_sender
            .send(RequestParsedData {
                protocol: self.protocol(),
                content: RequestParsedContent::Failed,
            })
            .await
            .unwrap();
        Ok(())
    }

    async fn parse_response(&self, mut delivery: ResponseParserDelivery) -> TuziResult<()> {
        unimplemented!()
    }
}

fn args_count(input: &[u8]) -> ITuziResult<&[u8], usize> {
    delimited(
        tag("*"),
        map(digit1, |s| str::from_utf8(s).unwrap().parse().unwrap()),
        crlf,
    )(input)
}
