//! Bounded owner work pages. Task outcomes never change native process lifetime.
use crate::{Activity, ApprovalSummary, LiveHandle, RemoteAttachment};
use qgenie_remote_proto::{
    wire::{LaunchKind, Machine, Session},
    *,
};
use std::{collections::BTreeSet, time::Duration};
use tokio::time::Instant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WorkCursor {
    pub machine: MachineId,
    pub after: Option<SessionId>,
}

#[derive(Clone, Debug)]
pub struct WorkItem {
    pub session: Session,
    pub machine: Machine,
    pub activity: Activity,
    pub activity_observed: bool,
    pub pending_approvals: usize,
    pub approvals_complete: bool,
}
impl WorkItem {
    pub fn needs_attention(&self, acknowledged: bool) -> bool {
        self.pending_approvals > 0
            || !self.approvals_complete
            || self.session.recording_warning.is_some()
            || matches!(
                self.session.state,
                SessionState::Failed | SessionState::Lost | SessionState::OutcomeUnknown
            )
            || matches!(self.activity, Activity::Failed | Activity::Interrupted)
            || (matches!(self.session.kind, LaunchKind::Agent { .. })
                && (!self.activity_observed || matches!(self.activity, Activity::Unknown)))
            || (matches!(self.activity, Activity::Completed { .. }) && !acknowledged)
    }
    fn completion_key(&self) -> Option<(UserId, MachineId, SessionId, String)> {
        let Activity::Completed { turn } = &self.activity else {
            return None;
        };
        Some((
            self.session.owner,
            self.session.machine,
            self.session.id,
            turn.clone(),
        ))
    }
}

/// Local to one gateway TUI instance. Reconnects and reopening the view retain acknowledgments;
/// quitting the application clears them. This state never suppresses approvals or failures.
#[derive(Default)]
pub struct WorkAcknowledgements {
    seen: BTreeSet<(UserId, MachineId, SessionId, String)>,
}
impl WorkAcknowledgements {
    pub fn contains(&self, item: &WorkItem) -> bool {
        item.completion_key()
            .is_some_and(|key| self.seen.contains(&key))
    }
    /// False means no exact successful completion, or the 4096-entry bound was reached.
    pub fn acknowledge(&mut self, item: &WorkItem) -> bool {
        let Some(key) = item.completion_key() else {
            return false;
        };
        if self.seen.contains(&key) {
            return true;
        }
        self.seen.len() < 4096 && self.seen.insert(key)
    }
}

#[derive(Debug)]
pub struct WorkPage {
    pub items: Vec<WorkItem>,
    pub next: Option<WorkCursor>,
    pub scanned_machines: usize,
    pub unavailable_machines: Vec<MachineId>,
    pub approvals_complete: bool,
}
impl LiveHandle {
    /// Scan at most 16 sessions or 16 machines, with one native read at a time.
    /// Supply the current authenticated machine inventory. Use a dedicated connection:
    /// cancellation closes that connection and releases any unfinished attachment.
    /// Pages are not a transactional snapshot; restart from the first page for fresh coverage.
    pub async fn work_page(
        &self,
        machines: &[Machine],
        after: Option<WorkCursor>,
        approvals: &ApprovalSummary,
    ) -> Result<WorkPage, RemoteError> {
        if !self.pages {
            return Err(RemoteError::rejected(
                ErrorCode::UnsupportedCapability,
                "work pages require session-pages-v1",
            ));
        }
        if machines.len() > 1024 {
            return Err(RemoteError::rejected(
                ErrorCode::Overloaded,
                "machine inventory exceeds work scan bound",
            ));
        }
        let mut machines: Vec<_> = machines.iter().collect();
        machines.sort_by_key(|machine| machine.id);
        if machines.windows(2).any(|pair| pair[0].id == pair[1].id) {
            return Err(RemoteError::rejected(
                ErrorCode::InvalidFrame,
                "duplicate machine inventory",
            ));
        }
        let mut page = WorkPage {
            items: Vec::new(),
            next: None,
            scanned_machines: 0,
            unavailable_machines: Vec::new(),
            approvals_complete: approvals.complete,
        };
        let deadline = Instant::now() + Duration::from_secs(10);
        let machines: Vec<_> = machines
            .into_iter()
            .filter(|machine| after.is_none_or(|cursor| machine.id >= cursor.machine))
            .collect();
        for (index, machine) in machines.iter().enumerate() {
            let mut cursor = after
                .filter(|cursor| cursor.machine == machine.id)
                .and_then(|cursor| cursor.after);
            if page.scanned_machines == 16 || Instant::now() >= deadline {
                page.next = Some(WorkCursor {
                    machine: machine.id,
                    after: cursor,
                });
                return Ok(page);
            }
            page.scanned_machines += 1;
            loop {
                let result = tokio::time::timeout_at(
                    deadline,
                    self.session_page(machine.id, cursor, (16 - page.items.len()) as u16, false),
                )
                .await;
                let (sessions, next) = match result {
                    Ok(Ok(page)) => page,
                    Ok(Err(error)) if stops_scan(&error) => return Err(error),
                    _ => {
                        page.unavailable_machines.push(machine.id);
                        break;
                    }
                };
                for session in sessions {
                    let mut activity = Activity::Unknown;
                    let mut observed = false;
                    if matches!(session.kind, LaunchKind::Agent { .. })
                        && machine.online
                        && Instant::now() < deadline
                    {
                        let read = async {
                            let snapshot =
                                match self.attach(machine.id, session.id, Counter(0)).await? {
                                    RemoteAttachment::Snapshot(snapshot) => snapshot,
                                    RemoteAttachment::Replay(_) => {
                                        match self.capture(machine.id, session.id).await? {
                                            RemoteAttachment::Snapshot(snapshot) => snapshot,
                                            _ => {
                                                return Err(RemoteError::rejected(
                                                    ErrorCode::InvalidFrame,
                                                    "agent checkpoint missing",
                                                ));
                                            }
                                        }
                                    }
                                };
                            let mut state = snapshot.agent_state()?;
                            if snapshot.seq < snapshot.head {
                                state.unconfirmed_tail = true;
                            }
                            Ok(state)
                        };
                        let result = tokio::time::timeout_at(
                            deadline.min(Instant::now() + Duration::from_secs(2)),
                            read,
                        )
                        .await;
                        // Detach even when the checkpoint failed. The dedicated connection owns cancellation cleanup.
                        let _ =
                            tokio::time::timeout_at(deadline, self.detach(machine.id, session.id))
                                .await;
                        match result {
                            Ok(Ok(state)) => {
                                activity = Activity::from_agent(&state);
                                observed = !state.unconfirmed_tail;
                            }
                            Ok(Err(error)) if stops_scan(&error) => return Err(error),
                            _ => {}
                        }
                    }
                    cursor = Some(session.id);
                    page.items.push(WorkItem {
                        pending_approvals: approvals
                            .sessions
                            .get(&session.id)
                            .copied()
                            .unwrap_or(0),
                        approvals_complete: approvals.complete,
                        session,
                        machine: (*machine).clone(),
                        activity,
                        activity_observed: observed,
                    });
                    if Instant::now() >= deadline {
                        page.next = Some(WorkCursor {
                            machine: machine.id,
                            after: cursor,
                        });
                        return Ok(page);
                    }
                }
                if page.items.len() == 16 {
                    page.next = next
                        .map(|after| WorkCursor {
                            machine: machine.id,
                            after: Some(after),
                        })
                        .or_else(|| {
                            machines.get(index + 1).map(|machine| WorkCursor {
                                machine: machine.id,
                                after: None,
                            })
                        });
                    return Ok(page);
                }
                if let Some(next) = next {
                    cursor = Some(next);
                } else {
                    break;
                }
            }
        }
        Ok(page)
    }
}
fn stops_scan(error: &RemoteError) -> bool {
    matches!(
        error.code,
        ErrorCode::Unauthenticated
            | ErrorCode::Forbidden
            | ErrorCode::AccessStale
            | ErrorCode::InvalidFrame
    )
}
