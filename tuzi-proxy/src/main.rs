use clap::Clap;
use tuzi_proxy::{error::TuziResult, Configuration};

#[tokio::main]
async fn main() -> TuziResult<()> {
    tuzi_proxy::init_tracing();
    let configuration = Configuration::parse();
    tuzi_proxy::run(configuration).await?;
    Ok(())
}
