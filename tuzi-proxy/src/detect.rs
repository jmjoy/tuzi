use crate::stream::ReaderBuffer;
use http::{Method, Version};
use nom::{
    branch::alt,
    bytes::streaming::{is_not, tag, take_till, take_while},
    character::is_digit,
    character::streaming::{char, crlf, digit1, one_of},
    combinator::{map, not, value},
    preceded,
    sequence::{delimited, separated_pair, terminated},
    IResult,
};
use std::str;
use tokio::io::AsyncRead;
use tracing::info;

pub struct Detection {}

// impl<R: AsyncRead + Unpin> Detection<R> {
//     pub fn new(reader_buffer: ReaderBuffer<R>) -> Self {
//         Self { reader_buffer }
//     }

//     pub fn reader_buffer(&mut self) -> &mut ReaderBuffer<R> {
//         &mut self.reader_buffer
//     }
// }

#[derive(Debug)]
pub enum Protocol {
    HTTP_1,
    REDIS,
    MYSQL,
}

#[derive(Debug, PartialEq)]
pub struct HttpRequestInfo {
    method: Method,
    uri: String,
    version: Version,
}

pub fn detect_protocol_with_term(input: &[u8]) -> Option<Protocol> {
    match alt((
        map(http_method, |_| Protocol::HTTP_1),
        map(redis, |_| Protocol::REDIS),
        map(mysql, |_| Protocol::MYSQL),
    ))(input)
    {
        Ok((_, protocol)) => Some(protocol),
        Err(_) => None,
    }
}

pub fn detect_http(input: &[u8], new_input: &mut Vec<u8>) -> Option<HttpRequestInfo> {
    match http(input) {
        Ok((_input, info)) => {
            if let Some(index) = input.iter().position(|x| *x == b'\r') {
                new_input.extend_from_slice(&input[..index]);
                new_input.extend_from_slice(b"\r\nX-Test: Fuck");
                new_input.extend_from_slice(&input[index..]);
                info!("http: new input");
            }
            Some(info)
        }
        Err(_) => None,
    }
}

pub fn http(input: &[u8]) -> IResult<&[u8], HttpRequestInfo> {
    let (input, method) = http_method(input)?;
    let (input, uri) = delimited(char(' '), is_not(" \r\n"), char(' '))(input)?;
    let (input, version) = delimited(tag("HTTP/1."), one_of("01"), crlf)(input)?;

    Ok((
        input,
        HttpRequestInfo {
            method,
            uri: String::from_utf8(uri.to_owned()).unwrap(),
            version: match version {
                '0' => Version::HTTP_10,
                '1' => Version::HTTP_11,
                _ => unreachable!(),
            },
        },
    ))
}

fn http_method(input: &[u8]) -> IResult<&[u8], Method> {
    alt((
        value(Method::GET, tag("GET")),
        value(Method::POST, tag("POST")),
        value(Method::PUT, tag("PUT")),
        value(Method::DELETE, tag("DELETE")),
        value(Method::HEAD, tag("HEAD")),
    ))(input)
}

pub fn redis_args_count(input: &[u8]) -> IResult<&[u8], usize> {
    delimited(
        tag("*"),
        map(digit1, |s| str::from_utf8(s).unwrap().parse().unwrap()),
        crlf,
    )(input)
}

fn mysql(input: &[u8]) -> IResult<&[u8], &[u8]> {
    tag([10])(input)
}

fn detect_http_method(input: ReaderBuffer) -> IResult<ReaderBuffer, Method> {
    alt((
        value(Method::GET, tag("GET")),
        value(Method::POST, tag("POST")),
        value(Method::PUT, tag("PUT")),
        value(Method::DELETE, tag("DELETE")),
        value(Method::HEAD, tag("HEAD")),
    ))(input)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_protocol() {
        let input = b"GET /list.php?id=43705977 HTTP/1.1\r\nHost: www.google.com\r\nConnection: keep-alive\r\nPragma: no-cache\r\nCache-Control: no-cache\r\nUser-Agent: Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/85.0.4183.102 Safari/537.36\r\nAccept: text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9\r\nAccept-Encoding: gzip, deflate\r\nAccept-Language: en-US,en;q=0.9,zh-CN;q=0.8,zh;q=0.7\r\n\r\n";

        assert_eq!(
            detect_http(input, &mut vec![]),
            Some(HttpRequestInfo {
                method: Method::GET,
                uri: "/list.php?id=43705977".to_owned(),
                version: Version::HTTP_11,
            })
        );
    }

    #[test]
    fn test_nom() {
        let input = b"hel";

        fn parser(s: &[u8]) -> IResult<&[u8], &[u8]> {
            tag("hello")(s)
        }

        let output = parser(&input[..]);
        if let Err(err) = output {
            dbg!(err.is_incomplete());
        }
    }
}
