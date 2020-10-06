use std::io;
use nom::error::{ErrorKind, ParseError};

pub type TuziResult<T> = Result<T, TuziError>;

pub type ITuziResult<I, O> = Result<(I, O), nom::Err<TuziError>>;

#[derive(thiserror::Error, Debug)]
pub enum TuziError {
    #[error("receiver closed")]
    ReceiveClosed,

    #[error("parse incomplete")]
    ParseIncomplete,

    #[error(transparent)]
    Io(#[from] io::Error),

    #[error("nom error kind ({1:?})")]
    Nom(Vec<u8>, ErrorKind),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

impl ParseError<&[u8]> for TuziError {
    fn from_error_kind(input: &[u8], kind: ErrorKind) -> Self {
        TuziError::Nom(input.to_owned(), kind)
    }

    fn append(_input: &[u8], _kind: ErrorKind, other: Self) -> Self {
        other
    }
}
