use super::*;

impl RecordingSpool {
    /// Network-facing callers must supply the immutable original binding. Filtering happens
    /// before selecting/offering a gap, so another enrollment cannot freeze or upload it.
    pub fn fetch_scoped(&mut self, scope: &RemoteScope) -> Result<Option<SpoolItem>> {
        self.fetch_filtered(Some(scope))
    }
    pub fn acknowledge_scoped(
        &mut self,
        scope: &RemoteScope,
        receipt: &CommitReceipt,
        authenticated_epoch: &str,
    ) -> Result<bool> {
        scope.validate()?;
        let identity: Option<String> = self
            .db
            .query_row(
                "SELECT s.identity FROM chunks c JOIN streams s ON s.epoch=c.epoch WHERE c.id=?1",
                [receipt.chunk.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if !matches_identity(identity, scope)? {
            return Ok(false);
        }
        self.acknowledge(receipt, authenticated_epoch)
    }
    pub fn acknowledge_gap_scoped(
        &mut self,
        scope: &RemoteScope,
        receipt: &GapCommitReceipt,
        authenticated_epoch: &str,
    ) -> Result<bool> {
        scope.validate()?;
        let identity: Option<String> = self
            .db
            .query_row(
                "SELECT s.identity FROM gaps g JOIN streams s ON s.epoch=g.epoch WHERE g.id=?1",
                [receipt.gap.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if !matches_identity(identity, scope)? {
            return Ok(false);
        }
        self.acknowledge_gap(receipt, authenticated_epoch)
    }
    pub fn status_scoped(&self, scope: &RemoteScope) -> Result<SpoolStatus> {
        scope.validate()?;
        let mut status = self.status()?;
        (status.chunks,status.pending_bytes)=self.db.query_row("SELECT count(*),coalesce(sum(c.bytes),0) FROM chunks c JOIN streams s ON s.epoch=c.epoch WHERE json_extract(s.identity,'$.origin')=?1 AND json_extract(s.identity,'$.machine')=?2 AND json_extract(s.identity,'$.owner')=?3",params![scope.origin,scope.machine.to_string(),scope.owner.to_string()],|r|Ok((r.get(0)?,r.get(1)?)))?;
        status.gaps=self.db.query_row("SELECT count(*) FROM gaps g JOIN streams s ON s.epoch=g.epoch WHERE json_extract(s.identity,'$.origin')=?1 AND json_extract(s.identity,'$.machine')=?2 AND json_extract(s.identity,'$.owner')=?3",params![scope.origin,scope.machine.to_string(),scope.owner.to_string()],|r|r.get(0))?;
        // The generic disk/capture warning applies to the whole shared device; it contains
        // no session identifiers, owner content or other enrollments' queue counts.
        Ok(status)
    }
}
fn matches_identity(value: Option<String>, scope: &RemoteScope) -> Result<bool> {
    let Some(value) = value else {
        return Ok(false);
    };
    let identity: RecordingIdentity = serde_json::from_str(&value)?;
    Ok(identity.origin == scope.origin
        && identity.machine == scope.machine
        && identity.owner == scope.owner)
}
