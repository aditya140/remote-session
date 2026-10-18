//! Bounded read-only owner inbox summary. Counts never authorize a choice.
use qgenie_remote_client::LiveHandle;
use qgenie_remote_proto::wire::Message;
use std::time::Duration;
use tokio::{task::JoinSet, time::Instant};

pub(crate) struct Badge {
    jobs: JoinSet<Option<(usize, bool)>>,
    next: Instant,
    dirty: bool,
    label: String,
}
impl Default for Badge {
    fn default() -> Self {
        Self {
            jobs: JoinSet::new(),
            next: Instant::now(),
            dirty: true,
            label: "Approvals: checking · a opens inbox".into(),
        }
    }
}
impl Badge {
    pub(crate) fn label(&self) -> &str {
        &self.label
    }
    pub(crate) fn changed(&mut self) {
        self.dirty = true;
        self.label = "Approvals: updates · a opens inbox".into();
    }
    pub(crate) fn event(&mut self, message: &Message) {
        if matches!(
            message,
            Message::ApprovalRequested(_)
                | Message::ApprovalCommitted(_)
                | Message::ApprovalApplication { .. }
                | Message::ApprovalCancelled { .. }
        ) {
            self.changed();
        }
    }
    pub(crate) fn tick(&mut self, handle: Option<LiveHandle>) {
        let Some(handle) = handle else {
            self.jobs.abort_all();
            while self.jobs.try_join_next().is_some() {}
            self.label = "Approvals: offline · reconnect to review".into();
            self.dirty = true;
            return;
        };
        if !handle.approvals_available() {
            self.label = "Approvals unavailable".into();
            return;
        }
        if let Some(result) = self.jobs.try_join_next() {
            self.label = match result {
                Ok(Some((count, false))) => format!("Approvals: {count} pending · a opens inbox"),
                Ok(Some((count, true))) if count > 0 => {
                    format!("Approvals: {count}+ pending · a opens inbox")
                }
                Ok(Some(_)) => "Approvals: partial index · a opens inbox".into(),
                _ => "Approvals: status unavailable · a reviews".into(),
            };
            self.next = Instant::now() + Duration::from_secs(5);
        }
        if self.jobs.is_empty() && (self.dirty || Instant::now() >= self.next) {
            self.dirty = false;
            self.jobs.spawn(async move {
                tokio::time::timeout(Duration::from_secs(10), count(handle))
                    .await
                    .ok()
                    .flatten()
            });
        }
    }
}
async fn count(handle: LiveHandle) -> Option<(usize, bool)> {
    let mut after = None;
    let mut count = 0;
    // Historical decisions can outnumber pending approvals. Never scan an
    // unbounded history or label an incomplete scan as a complete zero.
    for _ in 0..32 {
        let page = handle.approval_index(after, 200).await.ok()?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_millis();
        count += page
            .states
            .iter()
            .filter(|state| {
                state.winner.is_none()
                    && !state.cancelled
                    && state
                        .request
                        .expires_at
                        .as_ref()
                        .is_none_or(|expiry| u128::from(expiry.unix_millis()) > now)
            })
            .count();
        after = page.next;
        if after.is_none() {
            return Some((count, false));
        }
    }
    Some((count, true))
}
