mod app;
mod detect;
mod inbound;
mod outbound;
mod tcp;
mod util;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::init_tracing();
    app::run().await?;
    Ok(())
}
