use nom::{error::ErrorKind, IResult, Needed};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    sync::broadcast::Receiver,
};

pub async fn parse<T>(
    mut content: Vec<u8>,
    reader: &mut (impl AsyncRead + Unpin),
    f: impl Fn(&[u8]) -> IResult<&[u8], T>,
) -> IResult<Vec<u8>, T, (Vec<u8>, ErrorKind)> {
    let mut buf = [0; 4096];

    loop {
        match f(&content) {
            Ok((b, k)) => return Ok((b.to_owned(), k)),
            Err(e) => match e {
                nom::Err::Incomplete(_) => {}
                nom::Err::Error((b, k)) => return Err(nom::Err::Error((b.to_owned(), k))),
                nom::Err::Failure((b, k)) => return Err(nom::Err::Failure((b.to_owned(), k))),
            },
        }

        let n = reader.read(&mut buf).await.unwrap();
        if n == 0 {
            return Err(nom::Err::Incomplete(Needed::Unknown));
        }
        content.extend_from_slice(&buf[..n]);
    }
}

pub async fn parse_with_receiver<T>(
    mut content: Vec<u8>,
    receiver: &mut Receiver<Option<Vec<u8>>>,
    f: impl Fn(&[u8]) -> IResult<&[u8], T>,
) -> IResult<Vec<u8>, T, (Vec<u8>, ErrorKind)> {
    loop {
        match f(&content) {
            Ok((b, k)) => return Ok((b.to_owned(), k)),
            Err(e) => match e {
                nom::Err::Incomplete(_) => {}
                nom::Err::Error((b, k)) => return Err(nom::Err::Error((b.to_owned(), k))),
                nom::Err::Failure((b, k)) => return Err(nom::Err::Failure((b.to_owned(), k))),
            },
        }

        let b = receiver.recv().await.unwrap();
        let b = match b {
            Some(b) => b,
            None => return Err(nom::Err::Incomplete(Needed::Unknown)),
        };
        content.extend_from_slice(&b);
    }
}
