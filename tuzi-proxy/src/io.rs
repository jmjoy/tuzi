use crate::{
    error::{TuziError, TuziResult},
    parse::ResponseParserReader,
};
use anyhow::anyhow;
use async_trait::async_trait;
use nom::lib::std::mem::replace;
use std::io;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    prelude::AsyncWrite,
    sync::{broadcast, mpsc},
};
use tracing::debug;

#[async_trait]
pub trait Receivable<T> {
    async fn receive(&mut self) -> TuziResult<T>;
}

#[async_trait]
impl<T: Send> Receivable<T> for mpsc::Receiver<T> {
    async fn receive(&mut self) -> TuziResult<T> {
        match self.recv().await {
            Some(item) => Ok(item),
            None => Err(TuziError::ReceiveClosed),
        }
    }
}

#[async_trait]
impl<T: Send + Clone> Receivable<T> for broadcast::Receiver<T> {
    async fn receive(&mut self) -> TuziResult<T> {
        match self.recv().await {
            Ok(item) => Ok(item),
            Err(broadcast::RecvError::Closed) => Err(TuziError::ReceiveClosed),
            Err(e @ broadcast::RecvError::Lagged(_)) => Err(anyhow!(e).into()),
        }
    }
}

#[async_trait]
impl Receivable<Option<Vec<u8>>> for ResponseParserReader {
    async fn receive(&mut self) -> TuziResult<Option<Vec<u8>>> {
        debug!("reader receive");
        if self.exists_content.is_some() {
            let content = replace(&mut self.exists_content, None);
            if let Some(content) = content {
                if !content.is_empty() {
                    debug!("reader return exists content");
                    return Ok(Some(content));
                }
            }
        }
        debug!("reader read");
        let mut buf = [0; 4096];
        let n = self.server_read.read(&mut buf).await?;
        if n == 0 {
            Ok(None)
        } else {
            Ok(Some((&buf[..n]).to_owned()))
        }
    }
}

pub async fn receive_copy<R, W>(r: &mut R, w: &mut W) -> TuziResult<()>
where
    R: Receivable<Option<Vec<u8>>>,
    W: AsyncWrite + Unpin + Send,
{
    loop {
        let content = r.receive().await?;
        match content {
            Some(content) => {
                let n = w.write(&content).await?;
                if n == 0 {
                    return Err(io::Error::new(
                        io::ErrorKind::WriteZero,
                        "write zero byte into writer",
                    )
                    .into());
                }
            }
            None => break,
        }
    }
    Ok(())
}
