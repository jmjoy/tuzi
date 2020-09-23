use crate::outbound;

pub fn init_tracing() {
    tracing_subscriber::fmt::init();
}

pub async fn run() -> anyhow::Result<()> {
    // tokio::try_join!(inbound(), outbound())?;
    outbound::run().await.unwrap();
    Ok(())
}
