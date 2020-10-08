mod app;
mod error;
mod inbound;
mod outbound;
mod parse;
mod tcp;

use crate::error::TuziResult;

#[tokio::main]
async fn main() -> TuziResult<()> {
    app::init_tracing();
    app::run().await?;
    Ok(())
}
