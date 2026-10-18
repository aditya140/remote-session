use qgenie_remote_gateway::{Error, RecordingDatabase};
use qgenie_remote_proto::rest::LifecycleStage;

/// A single scheduled invocation. The caller owns the process/signal lifetime. Database
/// state makes interrupted/failed units repeatable; there is no in-memory leader authority.
pub(crate) async fn retention(
    database: &RecordingDatabase,
    reason: &str,
    batch_size: u16,
    max_steps: u16,
) -> Result<(), Error> {
    if !(1..=4096).contains(&max_steps) {
        return Err(Error::Invalid);
    }
    for step in 1..=max_steps {
        let progress = database
            .scheduled_retention_step(reason, batch_size)
            .await?;
        tracing::info!(step,stage=?progress.stage,content_month=?progress.content_month,affected=progress.affected,"recording lifecycle unit committed");
        if progress.stage == LifecycleStage::Idle {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    tracing::warn!("recording lifecycle work budget exhausted; schedule another invocation");
    Err(Error::Overloaded)
}
