use nom::error::ErrorKind;
use std::io;

pub type TuziResult<T> = Result<T, TuziError>;

#[derive(thiserror::Error, Debug)]
pub enum TuziError {
    #[error("receiver closed")]
    ReceiveClosed,

    #[error(transparent)]
    Io(#[from] io::Error),

    #[error(transparent)]
    Nom(nom::Err<(Vec<u8>, ErrorKind)>),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl From<nom::Err<(&[u8], ErrorKind)>> for TuziError {
    fn from(e: nom::Err<(&[u8], ErrorKind)>) -> Self {
        TuziError::Nom(e.map(|(b, k)| (b.to_owned(), k)))
    }
}
