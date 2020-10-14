use crate::{
    error::TuziResult,
    parse::{
        Protocol, ProtocolParsable, ReceiveParser, RequestParsedContent, RequestParsedData,
        RequestParserDelivery, ResponseParserDelivery,
    },
};
use async_trait::async_trait;
use http::{Method, Version};
use indexmap::IndexMap;
use nom::{
    branch::alt,
    bytes::streaming::{is_not, tag, take_while},
    character::{
        is_digit,
        streaming::{char, crlf, one_of},
    },
    combinator::{map, value},
    sequence::{delimited, preceded, terminated, tuple},
    IResult,
};

use tokio::io::{copy, AsyncWriteExt};
use tracing::{debug, info};

#[derive(Debug, PartialEq)]
pub struct RequestBegin {
    method: Method,
    uri: String,
    version: Version,
}

pub struct Parser;

#[async_trait]
impl ProtocolParsable for Parser {
    #[inline]
    fn protocol(&self) -> Protocol {
        "http/1"
    }

    async fn parse_request(&self, mut delivery: RequestParserDelivery) -> TuziResult<()> {
        let mut parser = ReceiveParser::new(Vec::new(), &mut delivery.request_raw_receiver);

        let info = parser.parse_and_recv(begin).await?;

        let mut headers = IndexMap::new();
        loop {
            let item = parser.parse_and_recv(header).await?;
            match item {
                Some((key, value)) => {
                    headers
                        .entry(String::from_utf8(key).unwrap())
                        .or_insert(Vec::new())
                        .push(String::from_utf8(value).unwrap());
                }
                None => break,
            }
        }

        info!(?info, ?headers, "detect http");

        // TODO check the headers handler.
        delivery
            .request_parsed_sender
            .send(RequestParsedData {
                protocol: self.protocol(),
                content: RequestParsedContent::Raw,
            })
            .await
            .unwrap();

        return Ok(());

        if !parser.recv_content_ref().is_empty() {
            delivery
                .request_parsed_sender
                .send(RequestParsedData {
                    protocol: self.protocol(),
                    content: RequestParsedContent::Content(parser.recv_content_ref().to_owned()),
                })
                .await
                .unwrap();
        }

        loop {
            let recv_content = delivery.request_raw_receiver.recv().await.unwrap();
            match recv_content {
                Some(recv_content) => {
                    delivery
                        .request_parsed_sender
                        .send(RequestParsedData {
                            protocol: self.protocol(),
                            content: RequestParsedContent::Content(recv_content),
                        })
                        .await
                        .unwrap();
                }
                None => {
                    delivery
                        .request_parsed_sender
                        .send(RequestParsedData {
                            protocol: self.protocol(),
                            content: RequestParsedContent::Eof,
                        })
                        .await
                        .unwrap();
                    break;
                }
            }
        }

        Ok(())
    }

    async fn parse_response(&self, mut delivery: ResponseParserDelivery) -> TuziResult<()> {
        debug!("parse_response {}", self.protocol());

        let mut parser = ReceiveParser::new(Vec::new(), &mut delivery.reader);
        let status = parser.parse_and_recv(response_begin).await.unwrap();
        info!(?status, "Receive status");
        delivery
            .client_write
            .write(parser.recv_content_ref())
            .await
            .unwrap();
        copy(&mut delivery.reader.server_read, &mut delivery.client_write)
            .await
            .unwrap();
        Ok(())
    }
}

fn begin(input: &[u8]) -> IResult<&[u8], RequestBegin> {
    let (input, method) = method(input)?;
    let (input, uri) = delimited(char(' '), is_not(" \r\n"), char(' '))(input)?;
    let (input, version) = terminated(version, crlf)(input)?;

    Ok((
        input,
        RequestBegin {
            method,
            uri: String::from_utf8(uri.to_owned()).unwrap(),
            version,
        },
    ))
}

fn method(input: &[u8]) -> IResult<&[u8], Method> {
    alt((
        value(Method::GET, tag("GET")),
        value(Method::POST, tag("POST")),
        value(Method::PUT, tag("PUT")),
        value(Method::DELETE, tag("DELETE")),
        value(Method::HEAD, tag("HEAD")),
    ))(input)
}

fn version(input: &[u8]) -> IResult<&[u8], Version> {
    preceded(
        tag("HTTP/1."),
        map(one_of("01"), |v| match v {
            '0' => Version::HTTP_10,
            '1' => Version::HTTP_11,
            _ => unreachable!(),
        }),
    )(input)
}

fn header(input: &[u8]) -> IResult<&[u8], Option<(Vec<u8>, Vec<u8>)>> {
    let (input, kv) = alt((
        map(crlf, |_| None),
        map(
            tuple((is_not(" \r\n:"), tag(": "), is_not("\r\n"), crlf)),
            |(key, _, value, _): (&[u8], _, &[u8], _)| Some((key.to_vec(), value.to_vec())),
        ),
    ))(input)?;

    Ok((input, kv))
}

pub fn response_begin(input: &[u8]) -> IResult<&[u8], String> {
    let (input, version) = terminated(version, char(' '))(input)?;
    let (input, code) = terminated(
        map(take_while(is_digit), |s: &[u8]| {
            String::from_utf8(s.to_owned()).unwrap()
        }),
        char(' '),
    )(input)?;
    let (input, _) = terminated(is_not("\r\n"), crlf)(input)?;
    Ok((input, format!("version: {:?}, code: {}", version, code)))
}
