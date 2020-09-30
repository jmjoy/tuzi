use std::io;

pub type TuZiResult<T> = Result<T, TuZiError>;

#[derive(thiserror::Error, Debug)]
pub enum TuZiError {
    #[error(transparent)]
    IO(#[from] io::Error),

    #[error(transparent)]
    Other(#[from] anyhow::Error),
}
