//! Bounded read-only owner inbox summary. Counts never authorize a choice.
use qgenie_remote_client::{ApprovalSummary, LiveHandle};
use qgenie_remote_proto::{SessionId, wire::Message};
use std::time::Duration;
use tokio::{task::JoinSet, time::Instant};

pub(crate) struct Badge {
    jobs: JoinSet<Option<ApprovalSummary>>,
    next: Instant,
    dirty: bool,
    label: String,
    summary: Option<ApprovalSummary>,
}
impl Default for Badge {
    fn default() -> Self {
        Self {
            jobs: JoinSet::new(),
            next: Instant::now(),
            dirty: true,
            summary: None,
            label: "Approvals: checking · a opens inbox".into(),
        }
    }
}
impl Badge {
    pub(crate) fn pending(&self, session: SessionId) -> usize {
        self.summary
            .as_ref()
            .and_then(|summary| summary.sessions.get(&session))
            .copied()
            .unwrap_or(0)
    }
    pub(crate) fn label(&self) -> &str {
        &self.label
    }
    pub(crate) fn changed(&mut self) {
        self.dirty = true;
        self.summary = None;
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
            self.summary = None;
            self.label = "Approvals: offline · reconnect to review".into();
            self.dirty = true;
            return;
        };
        if !handle.approvals_available() {
            self.label = "Approvals unavailable".into();
            return;
        }
        if let Some(result) = self.jobs.try_join_next() {
            self.summary = result.ok().flatten();
            self.label = match &self.summary {
                Some(summary) if summary.complete => {
                    format!("Approvals: {} pending · a opens inbox", summary.pending)
                }
                Some(summary) if summary.pending > 0 => {
                    format!("Approvals: {}+ pending · a opens inbox", summary.pending)
                }
                Some(_) => "Approvals: partial index · a opens inbox".into(),
                None => "Approvals: status unavailable · a reviews".into(),
            };
            self.next = Instant::now() + Duration::from_secs(5);
        }
        if self.jobs.is_empty() && (self.dirty || Instant::now() >= self.next) {
            self.dirty = false;
            self.jobs
                .spawn(async move { handle.approval_summary().await.ok() });
        }
    }
}
