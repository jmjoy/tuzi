use http::Method;
use nom::{IResult, branch::alt, bytes::complete::{is_not, tag, take_till, take_while}, character::complete::{char, crlf}, combinator::{map, not, value}, sequence::{delimited, separated_pair, terminated}};
use std::str;

#[derive(Debug, PartialEq)]
pub struct HttpRequestInfo {
    method: Method,
    uri: String,
    version: String,
}

pub fn detect_protocol(input: &[u8]) -> Option<HttpRequestInfo> {
    match http(input) {
        Ok((_input, info)) => Some(info),
        Err(_) => None,
    }
}

fn http(input: &[u8]) -> IResult<&[u8], HttpRequestInfo> {
    let (input, method) = http_method(input)?;
    let (input, uri) = delimited(char(' '), is_not(" \r\n"), char(' '))(input)?;
    let (input, version) = take_till(|c| c == b'\r')(input)?;

    Ok((
        input,
        HttpRequestInfo {
            method,
            uri: String::from_utf8(uri.to_owned()).unwrap(),
            version: str::from_utf8(version).unwrap().to_owned(),
        },
    ))
}

fn http_method(input: &[u8]) -> IResult<&[u8], Method> {
    alt((
        value(Method::GET, tag("GET")),
        value(Method::POST, tag("POST")),
        value(Method::PUT, tag("PUT")),
        value(Method::DELETE, tag("DELETE")),
    ))(input)
}

fn http_get(input: &[u8]) -> IResult<&[u8], Method> {
    let (input, _) = tag(b"GET")(input)?;
    Ok((input, Method::GET))
}

fn http_post(input: &[u8]) -> IResult<&[u8], Method> {
    let (input, _) = tag(b"POST")(input)?;
    Ok((input, Method::POST))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_detect_protocol() {
        let input = b"GET /list.php?id=43705977 HTTP/1.1\r\nHost: www.google.com\r\nConnection: keep-alive\r\nPragma: no-cache\r\nCache-Control: no-cache\r\nUser-Agent: Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/85.0.4183.102 Safari/537.36\r\nAccept: text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,image/apng,*/*;q=0.8,application/signed-exchange;v=b3;q=0.9\r\nAccept-Encoding: gzip, deflate\r\nAccept-Language: en-US,en;q=0.9,zh-CN;q=0.8,zh;q=0.7\r\n\r\n";

        assert_eq!(
            detect_protocol(input),
            Some(HttpRequestInfo {
                method: Method::GET,
                uri: "/list.php?id=43705977".to_owned(),
                version: "HTTP/1.1".to_owned(),
            })
        );
    }
}
