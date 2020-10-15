pub mod bound;
pub mod error;
mod inbound;
mod outbound;
mod parse;
mod tcp;
pub mod waitgroup;

pub use crate::parse::Protocol;

use clap::Clap;
use std::{net::SocketAddr, rc::Rc};

#[derive(Debug, Clap)]
pub struct Configuration {
    // #[clap(long)]
    // inbound: SocketAddr,
    #[clap(long)]
    outbound: SocketAddr,
}

pub fn init_tracing() {
    tracing_subscriber::fmt::init();
}

pub async fn run(configuration: Configuration) -> anyhow::Result<()> {
    let configuration = Rc::new(configuration);

    // tokio::try_join!(inbound(), outbound())?;
    outbound::run(configuration).await;
    Ok(())
}
