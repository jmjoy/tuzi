use crate::{
    bound::{run_with_listener, OriginalDst},
    Configuration,
};
use futures::TryFutureExt;
use std::rc::Rc;
use tokio::{net::TcpListener, signal::ctrl_c};
use tracing::instrument;

#[instrument(name = "outbound:run")]
pub async fn run(configuration: Rc<Configuration>) {
    let listener = TcpListener::bind(configuration.outbound).await.unwrap();
    run_with_listener(listener, OriginalDst, async {
        ctrl_c().await.unwrap();
        Ok(())
    })
    .await;
}
