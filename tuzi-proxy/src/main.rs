mod app;
mod detect;
mod error;
mod inbound;
mod outbound;
mod stream;
mod tcp;
mod util;

use error::TuZiResult;

#[tokio::main]
async fn main() -> TuZiResult<()> {
    app::init_tracing();
    app::run().await?;
    Ok(())
}
