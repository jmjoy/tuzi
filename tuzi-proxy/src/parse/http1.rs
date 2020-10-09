use crate::{
    error::{ITuziResult, TuziResult},
    parse::{Parser, ParserDelivery, Protocol, RequestParsedContent, RequestParsedData},
};
use async_trait::async_trait;
use http::{Method, Version};
use indexmap::IndexMap;
use nom::{
    branch::alt,
    bytes::streaming::{is_not, tag, take_till, take_while},
    character::{
        is_digit,
        streaming::{char, crlf, digit1, one_of},
    },
    combinator::{map, value},
    sequence::{delimited, preceded, terminated, tuple},
    IResult,
};
use std::{future::Future, pin::Pin, sync::Arc};
use tokio::sync::{
    broadcast::{self},
    mpsc,
};
use tracing::info;

#[derive(Debug, PartialEq)]
pub struct RequestBegin {
    method: Method,
    uri: String,
    version: Version,
}

pub async fn parse(mut delivery: ParserDelivery) -> TuziResult<()> {
    let mut parser = Parser::new(Vec::new(), &mut delivery.request_raw_receiver);

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

    if !parser.recv_content_ref().is_empty() {
        delivery
            .request_parsed_sender
            .send(RequestParsedData {
                protocol: Protocol::HTTP1,
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
                        protocol: Protocol::HTTP1,
                        content: RequestParsedContent::Content(recv_content),
                    })
                    .await
                    .unwrap();
            }
            None => {
                delivery
                    .request_parsed_sender
                    .send(RequestParsedData {
                        protocol: Protocol::HTTP1,
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

fn begin(input: &[u8]) -> ITuziResult<&[u8], RequestBegin> {
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

fn method(input: &[u8]) -> ITuziResult<&[u8], Method> {
    alt((
        value(Method::GET, tag("GET")),
        value(Method::POST, tag("POST")),
        value(Method::PUT, tag("PUT")),
        value(Method::DELETE, tag("DELETE")),
        value(Method::HEAD, tag("HEAD")),
    ))(input)
}

fn version(input: &[u8]) -> ITuziResult<&[u8], Version> {
    preceded(
        tag("HTTP/1."),
        map(one_of("01"), |v| match v {
            '0' => Version::HTTP_10,
            '1' => Version::HTTP_11,
            _ => unreachable!(),
        }),
    )(input)
}

fn header(input: &[u8]) -> ITuziResult<&[u8], Option<(Vec<u8>, Vec<u8>)>> {
    let (input, kv) = alt((
        map(crlf, |_| None),
        map(
            tuple((is_not(" \r\n:"), tag(": "), is_not("\r\n"), crlf)),
            |(key, _, value, _): (&[u8], _, &[u8], _)| Some((key.to_vec(), value.to_vec())),
        ),
    ))(input)?;

    Ok((input, kv))
}

pub fn respnose_begin(input: &[u8]) -> ITuziResult<&[u8], String> {
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

// #[cfg(test)]
// mod tests {
//     use super::*;

//     #[test]
//     fn test_detect_protocol() {
//         let input = b"GET /list.php?id=43705977 HTTP/1.1\r\nHost: www.google.com\r\nConnection: keep-alive\r\nPragma: no-cache\r\nCache-Control: no-cache\r\nUser-Agent: Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/85.0.4183.102 Safari/537.36\r\nAccept: text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9\r\nAccept-Encoding: gzip, deflate\r\nAccept-Language: en-US,en;q=0.9,zh-CN;q=0.8,zh;q=0.7\r\n\r\n";

//         assert_eq!(
//             detect_http(input, &mut vec![]),
//             Some(HttpRequestInfo {
//                 method: Method::GET,
//                 uri: "/list.php?id=43705977".to_owned(),
//                 version: Version::HTTP_11,
//             })
//         );
//     }

//     #[test]
//     fn test_nom() {
//         let input = b"hel";

//         fn parser(s: &[u8]) -> IResult<&[u8], &[u8]> {
//             tag("hello")(s)
//         }

//         let output = parser(&input[..]);
//         if let Err(err) = output {
//             dbg!(err.is_incomplete());
//         }
//     }
// }
