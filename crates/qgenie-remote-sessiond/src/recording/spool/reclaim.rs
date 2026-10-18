use super::*;
use qgenie_remote_proto::ipc::RecordingReceipt;
impl RecordingSpool {
    /// The execution guard runs after obtaining SQLite's write lock and validating exact
    /// immutable bytes, immediately before logical reclamation. No native lock spans I/O.
    pub(crate) fn reclaim(
        &mut self,
        scope: &RemoteScope,
        receipt: &RecordingReceipt,
        epoch: &str,
        guard: impl FnOnce() -> Result<()>,
    ) -> Result<bool> {
        scope.validate()?;
        let (id, content) = match receipt {
            RecordingReceipt::Content(r) => (r.chunk.to_string(), true),
            RecordingReceipt::Gap(r) => (r.gap.to_string(), false),
        };
        let path = match receipt {
            RecordingReceipt::Content(r) => Some(self.content_path(r.chunk)),
            _ => None,
        };
        let tx = self
            .db
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let row: Option<(String,String)> = tx.query_row(
            if content { "SELECT s.identity,c.metadata FROM chunks c JOIN streams s ON s.epoch=c.epoch WHERE c.id=?1" }
            else { "SELECT s.identity,g.metadata FROM gaps g JOIN streams s ON s.epoch=g.epoch WHERE g.id=?1" },
            [&id], |r|Ok((r.get(0)?,r.get(1)?)),
        ).optional()?;
        let Some((identity, metadata)) = row else {
            return Ok(false);
        };
        let identity: RecordingIdentity = serde_json::from_str(&identity)?;
        if identity.origin != scope.origin
            || identity.owner != scope.owner
            || identity.machine != scope.machine
        {
            return Ok(false);
        }
        match receipt {
            RecordingReceipt::Content(r) => {
                let batch: RecordingBatch = serde_json::from_str(&metadata)?;
                ensure!(
                    !epoch.is_empty()
                        && r.recovery_epoch == epoch
                        && r.machine == identity.machine
                        && r.machine == batch.machine
                        && r.session == batch.session
                        && r.stream_epoch == batch.stream_epoch
                        && r.digest == batch.digest
                        && r.from == batch.from
                        && r.to == batch.to
                        && r.captured_from == batch.captured_from
                        && r.captured_to == batch.captured_to,
                    "recording receipt mismatch"
                );
            }
            RecordingReceipt::Gap(r) => {
                let gap: LossRange = serde_json::from_str(&metadata)?;
                ensure!(
                    !epoch.is_empty()
                        && r.recovery_epoch == epoch
                        && r.machine == identity.machine
                        && r.session == gap.session
                        && r.stream_epoch == gap.stream_epoch
                        && r.digest == gap.receipt_digest()?
                        && r.from == gap.from
                        && r.to == gap.to
                        && r.captured_from == gap.captured_from
                        && r.captured_to == gap.captured_to,
                    "gap receipt mismatch"
                );
            }
        }
        guard()?;
        tx.execute(
            if content {
                "DELETE FROM chunks WHERE id=?1"
            } else {
                "DELETE FROM gaps WHERE id=?1"
            },
            [&id],
        )?;
        tx.commit()?;
        if let Some(path) = path {
            match fs::remove_file(path) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.into()),
            }
        }
        Ok(true)
    }
}
