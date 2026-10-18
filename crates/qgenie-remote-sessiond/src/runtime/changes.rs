//! Scope-only invalidations coalesce without retaining output or waiting for a viewer.
use super::*;
use qgenie_remote_proto::ipc::{DispatchFrame, RemoteContext};
use tokio::net::UnixStream;
#[cfg(test)]
mod tests;

async fn write_update(
    stream: &UnixStream,
    fence: &authority::Fence,
    frame: DispatchFrame,
) -> anyhow::Result<()> {
    let encoded = serde_json::to_vec(&frame)?;
    anyhow::ensure!(encoded.len() < 256, "invalid native notification size");
    let mut bytes = (encoded.len() as u32).to_be_bytes().to_vec();
    bytes.extend(encoded);
    let mut remaining = bytes.as_slice();
    while !remaining.is_empty() {
        let deadline = fence.check()?;
        tokio::time::timeout_at(deadline.into(), stream.writable()).await??;
        match fence.try_io(&mut || stream.try_write(remaining)) {
            Ok(0) => anyhow::bail!("native notification stream closed"),
            Ok(count) => remaining = &remaining[count..],
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                tokio::time::sleep(Duration::from_millis(2)).await;
            }
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

impl Runtime {
    pub(super) fn publish_change(&self, scope: Option<&RemoteScope>) {
        if let Some(scope) = scope
            && let Ok(changes) = self.changes.lock()
            && let Some(sender) = changes.get(scope)
        {
            sender.send_modify(|revision| *revision = revision.wrapping_add(1));
        }
    }
    pub(crate) async fn serve_changes(
        &self,
        stream: &mut UnixStream,
        context: RemoteContext,
    ) -> anyhow::Result<()> {
        anyhow::ensure!(
            context.controller_generation.is_none(),
            "read-only changes require owner access"
        );
        let fence = lock(&self.authority)?.fence(&context)?;
        let mut receiver = {
            let mut changes = lock(&self.changes)?;
            anyhow::ensure!(
                changes.contains_key(&context.scope) || changes.len() < 64,
                "native scope subscription cap reached"
            );
            changes
                .entry(context.scope)
                .or_insert_with(|| tokio::sync::watch::channel(0).0)
                .subscribe()
        };
        write_update(stream, &fence, DispatchFrame::ChangesReady).await?;
        // A suspend-aware fence is checked even when this scope is idle. The
        // original grant, never a renewal, covers every notification's write.
        let mut check = tokio::time::interval(Duration::from_millis(20));
        check.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = stream.readable() => {
                    // This stream accepts no further commands. EOF is observed
                    // promptly even if no native output occurs.
                    let mut byte=[0];
                    match stream.try_read(&mut byte) {
                        Err(error) if error.kind()==std::io::ErrorKind::WouldBlock=>{},
                        _=>return Ok(()),
                    }
                },
                _ = check.tick() => {fence.check()?;},
                changed = receiver.changed() => {
                    changed?;
                    write_update(stream,&fence,DispatchFrame::NativeChanged).await?;
                    // One latest revision per scope is retained while writes
                    // coalesce. Replay and recording remain separate owners.
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            }
        }
    }
}
