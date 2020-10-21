pub mod bound;
pub mod collect;
pub mod error;
pub mod io;
pub mod parse;
pub mod tcp;
pub mod wait;

pub use crate::parse::Protocol;

use crate::bound::outbound;
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
    outbound(configuration).await;
    Ok(())
}
