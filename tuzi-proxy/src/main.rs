mod app;
mod detect;
mod error;
mod inbound;
mod outbound;
mod parse;
mod tcp;
mod util;

use error::TuziResult;

#[tokio::main]
async fn main() -> TuziResult<()> {
    app::init_tracing();
    app::run().await?;
    Ok(())
}
