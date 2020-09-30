use crate::error::TuZiResult;
use io::AsyncReadExt;
use nom::{Compare, CompareResult, InputTake};
use std::{cell::RefCell, io::Write, rc::Rc, sync::Arc, sync::RwLock};
use tokio::io::{self, AsyncRead, AsyncWrite, AsyncWriteExt};

#[derive(Clone)]
pub enum TakeDirection {
    None,
    Left,
    Right,
}

#[derive(Clone)]
pub struct ReaderBuffer {
    buffer: Arc<RwLock<Vec<u8>>>,
    position: usize,
    take_direction: TakeDirection,
}

impl InputTake for ReaderBuffer {
    fn take(&self, count: usize) -> Self {
        ReaderBuffer {
            buffer: self.buffer.clone(),
            position: count,
            take_direction: TakeDirection::Left,
        }
    }

    fn take_split(&self, count: usize) -> (Self, Self) {
        (
            ReaderBuffer {
                buffer: self.buffer.clone(),
                position: count,
                take_direction: TakeDirection::Right,
            },
            ReaderBuffer {
                buffer: self.buffer.clone(),
                position: count,
                take_direction: TakeDirection::Left,
            },
        )
    }
}

impl Compare<&[u8]> for ReaderBuffer {
    fn compare(&self, t: &[u8]) -> nom::CompareResult {
        let buffer = &*self.buffer.read().unwrap();
        let buffer = match self.take_direction {
            TakeDirection::Left => &buffer[..self.position],
            TakeDirection::Right => &buffer[self.position..],
            TakeDirection::None => unreachable!(),
        };
        let pos = buffer.iter().zip(t.iter()).position(|(a, b)| a != b);

        match pos {
            Some(_) => CompareResult::Error,
            None => {
                if buffer.len() >= t.len() {
                    CompareResult::Ok
                } else {
                    CompareResult::Incomplete
                }
            }
        }
    }

    fn compare_no_case(&self, t: &[u8]) -> nom::CompareResult {
        self.compare(t)
    }
}

impl Compare<&str> for ReaderBuffer {
    fn compare(&self, t: &str) -> CompareResult {
        self.compare(t.as_bytes())
    }

    fn compare_no_case(&self, t: &str) -> CompareResult {
        self.compare_no_case(t.as_bytes())
    }
}

impl ReaderBuffer {
    pub fn new() -> Self {
        Self {
            buffer: Arc::new(RwLock::new(Vec::new())),
            position: 0,
            take_direction: TakeDirection::None,
        }
    }

    // pub async fn extend_buffer(&mut self) -> Option<()> {
    //     let mut buf = [0; 4096];
    //     let n = self.reader.write().unwrap().read(&mut buf).await.unwrap();
    //     if n == 0 {
    //         return None;
    //     }

    //     self.buffer.write().unwrap().extend_from_slice(&buf[..n]);

    //     Some(())
    // }

    // pub async fn write_to<W: AsyncWrite + Unpin>(&mut self, w: &mut W) -> TuZiResult<()> {
    //     let n = w.write(&self.buffer.read().unwrap()).await?;
    //     if n == 0 {
    //         return Err(
    //             io::Error::new(io::ErrorKind::WriteZero, "write zero byte into writer").into(),
    //         );
    //     }

    //     io::copy(&mut *self.reader.write().unwrap(), w).await?;

    //     Ok(())
    // }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nom::{bytes::streaming::tag, IResult};

    // #[test]
    // fn test_reader_buffer() {
    //     let rb = ReaderBuffer::new();
    //     rb.buffer.write().unwrap().extend_from_slice(b"hel");

    //     fn parser(rb: ReaderBuffer) -> IResult<ReaderBuffer, ReaderBuffer> {
    //         tag("hello")(rb)
    //     }

    //     loop {
    //         match parser(rb.clone()) {
    //             Ok(_) => {
    //                 dbg!("ok");
    //                 break;
    //             }
    //             Err(err) => {
    //                 if err.is_incomplete() {
    //                     rb.buffer.write().unwrap().extend_from_slice(b"hel");
    //                     buffer.extend_from_slice(b"loooo");
    //                 }
    //             }
    //         }
    //     }
    // }

    #[test]
    fn test_nom_streaming() {

    }
}
