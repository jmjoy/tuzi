mod app;
mod inbound;
mod outbound;
mod tcp;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    app::init_tracing();
    app::run().await?;
    Ok(())
}
