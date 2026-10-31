//! Observed agent activity is separate from native process lifetime and authorization.
use crate::LiveHandle;
use qgenie_remote_proto::{
    wire::{AgentEvent, TurnOutcome},
    *,
};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Activity {
    Unknown,
    Working { turn: Option<String> },
    Idle,
    Completed { turn: String },
    Failed,
    Interrupted,
    Exited,
}
impl Activity {
    /// Derive activity only from typed events. PTY screen text cannot establish it.
    pub fn from_agent(state: &AgentState) -> Self {
        let mut activity = Self::Unknown;
        for item in &state.events {
            match &item.event {
                AgentEvent::TurnStarted { turn } => {
                    activity = Self::Working {
                        turn: Some(turn.clone()),
                    }
                }
                AgentEvent::TurnCompleted { turn, outcome, .. } => {
                    activity = match outcome {
                        Some(TurnOutcome::Completed) => Self::Completed { turn: turn.clone() },
                        Some(TurnOutcome::Failed) => Self::Failed,
                        Some(TurnOutcome::Interrupted) => Self::Interrupted,
                        None => Self::Unknown,
                    };
                }
                AgentEvent::Idle => {
                    if !matches!(
                        activity,
                        Self::Completed { .. } | Self::Failed | Self::Interrupted | Self::Exited
                    ) {
                        activity = Self::Idle;
                    }
                }
                AgentEvent::Error { .. } => activity = Self::Failed,
                AgentEvent::Exited { .. } => activity = Self::Exited,
                AgentEvent::Gap { .. } | AgentEvent::Opaque { .. } => activity = Self::Unknown,
                AgentEvent::MessageDelta { .. }
                | AgentEvent::ToolStarted { .. }
                | AgentEvent::ToolOutput { .. }
                    if !matches!(activity, Self::Working { .. }) =>
                {
                    activity = Self::Working { turn: None };
                }
                _ => {}
            }
        }
        if state.unconfirmed_tail {
            Self::Unknown
        } else {
            activity
        }
    }
    pub fn label(&self) -> &'static str {
        match self {
            Self::Unknown => "activity unknown",
            Self::Working { .. } => "working",
            Self::Idle => "idle",
            Self::Completed { .. } => "turn complete",
            Self::Failed => "agent error",
            Self::Interrupted => "turn interrupted",
            Self::Exited => "agent exited",
        }
    }
}

/// A bounded durable inbox scan. An incomplete zero is never a confirmed empty inbox.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ApprovalSummary {
    pub pending: usize,
    pub complete: bool,
    pub sessions: BTreeMap<SessionId, usize>,
}
impl LiveHandle {
    pub async fn approval_summary(&self) -> Result<ApprovalSummary, RemoteError> {
        let scan = async {
            let mut summary = ApprovalSummary::default();
            let mut after = None;
            for _ in 0..32 {
                let page = self.approval_index(after, 200).await?;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| {
                        RemoteError::rejected(ErrorCode::Internal, "system clock precedes epoch")
                    })?
                    .as_millis();
                for state in page.states {
                    if state.winner.is_none()
                        && !state.cancelled
                        && state
                            .request
                            .expires_at
                            .as_ref()
                            .is_none_or(|expiry| u128::from(expiry.unix_millis()) > now)
                    {
                        summary.pending += 1;
                        *summary.sessions.entry(state.request.session).or_default() += 1;
                    }
                }
                if page.next.is_none() {
                    summary.complete = true;
                    return Ok(summary);
                }
                if page.next <= after {
                    return Err(RemoteError::rejected(
                        ErrorCode::InvalidFrame,
                        "approval index did not advance",
                    ));
                }
                after = page.next;
            }
            Ok(summary)
        };
        tokio::time::timeout(std::time::Duration::from_secs(10), scan)
            .await
            .map_err(|_| {
                RemoteError::rejected(ErrorCode::AccessStale, "approval summary timed out")
            })?
    }
}

#[derive(Debug)]
pub enum WaitTarget {
    Turn(String),
    Exit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitOutcome {
    Completed,
    Exited,
    ApprovalRequired,
    Failed,
    Interrupted,
    Lost,
    EndedBeforeTurn,
    HistoryUnavailable,
    Timeout,
    Cancelled,
}

impl WaitTarget {
    /// `None` means the requested condition has not been proved. It never means success.
    pub fn outcome(
        &self,
        lifetime: &SessionState,
        agent: Option<&AgentState>,
        pending_approvals: usize,
    ) -> Option<WaitOutcome> {
        if let Self::Turn(turn) = self {
            if let Some(outcome) = agent.and_then(|state| {
                state
                    .events
                    .iter()
                    .rev()
                    .find_map(|item| match &item.event {
                        AgentEvent::TurnCompleted {
                            turn: completed,
                            outcome,
                            ..
                        } if completed == turn => Some(*outcome),
                        _ => None,
                    })
            }) {
                return Some(match outcome {
                    Some(TurnOutcome::Completed) => WaitOutcome::Completed,
                    Some(TurnOutcome::Failed) => WaitOutcome::Failed,
                    Some(TurnOutcome::Interrupted) => WaitOutcome::Interrupted,
                    None => WaitOutcome::HistoryUnavailable,
                });
            }
        } else if matches!(lifetime, SessionState::Exited) {
            return Some(WaitOutcome::Exited);
        }
        match lifetime {
            SessionState::Lost | SessionState::OutcomeUnknown => return Some(WaitOutcome::Lost),
            SessionState::Failed => return Some(WaitOutcome::Failed),
            SessionState::Exited => {
                return Some(
                    if agent.is_none_or(|state| state.history_truncated || state.unconfirmed_tail) {
                        WaitOutcome::HistoryUnavailable
                    } else {
                        WaitOutcome::EndedBeforeTurn
                    },
                );
            }
            _ => {}
        }
        if pending_approvals > 0 {
            return Some(WaitOutcome::ApprovalRequired);
        }
        if let Self::Turn(turn) = self {
            let Some(state) = agent else {
                return Some(WaitOutcome::HistoryUnavailable);
            };
            let start = state.events.iter().position(|item|matches!(&item.event,AgentEvent::TurnStarted { turn: started } if started == turn));
            if state.unconfirmed_tail
                || (state.history_truncated && start.is_none())
                || start.is_some_and(|index| {
                    state.events[index..]
                        .iter()
                        .any(|item| matches!(item.event, AgentEvent::Gap { .. }))
                })
            {
                return Some(WaitOutcome::HistoryUnavailable);
            }
            if start.is_some_and(|index| state.events[index..].iter().take_while(|item|!matches!(&item.event,AgentEvent::TurnStarted { turn: started } if started != turn)).any(|item|matches!(item.event,AgentEvent::Error { .. }))) {
                return Some(WaitOutcome::Failed);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qgenie_remote_proto::wire::{SequencedEvent, Timestamp};
    fn state(events: Vec<AgentEvent>) -> AgentState {
        let mut state = AgentState::default();
        for event in events {
            state
                .push(SequencedEvent {
                    seq: Counter(state.seq.0 + 1),
                    at: Timestamp::from_unix_millis(1).unwrap(),
                    event,
                })
                .unwrap();
        }
        state
    }
    fn completed(turn: &str) -> AgentEvent {
        AgentEvent::TurnCompleted {
            turn: turn.into(),
            input_tokens: Counter(1),
            output_tokens: Counter(1),
            outcome: Some(TurnOutcome::Completed),
        }
    }
    #[test]
    fn targeted_wait_does_not_confuse_idle_another_turn_or_lifetime_with_completion() {
        let target = WaitTarget::Turn("target".into());
        let mut state = state(vec![completed("old"), AgentEvent::Idle]);
        assert_eq!(target.outcome(&SessionState::Idle, Some(&state), 0), None);
        assert_eq!(
            Activity::from_agent(&state),
            Activity::Completed { turn: "old".into() }
        );
        assert_eq!(
            target.outcome(&SessionState::Running, Some(&state), 1),
            Some(WaitOutcome::ApprovalRequired)
        );
        state.history_truncated = true;
        assert_eq!(
            target.outcome(&SessionState::Running, Some(&state), 0),
            Some(WaitOutcome::HistoryUnavailable)
        );
        assert_eq!(
            target.outcome(&SessionState::Lost, Some(&state), 0),
            Some(WaitOutcome::Lost)
        );
        assert_eq!(
            target.outcome(&SessionState::Exited, Some(&state), 0),
            Some(WaitOutcome::HistoryUnavailable)
        );
        state.history_truncated = false;
        assert_eq!(
            target.outcome(&SessionState::Exited, Some(&state), 0),
            Some(WaitOutcome::EndedBeforeTurn)
        );
        assert_eq!(
            WaitTarget::Exit.outcome(&SessionState::Exited, None, 0),
            Some(WaitOutcome::Exited)
        );
    }
    #[test]
    fn retained_completion_is_proof_and_unknown_tail_is_not_current_activity() {
        let target = WaitTarget::Turn("target".into());
        let mut state = state(vec![
            completed("target"),
            AgentEvent::TurnStarted {
                turn: "next".into(),
            },
        ]);
        state.history_truncated = true;
        state.unconfirmed_tail = true;
        assert_eq!(
            target.outcome(&SessionState::Running, Some(&state), 0),
            Some(WaitOutcome::Completed)
        );
        assert_eq!(Activity::from_agent(&state), Activity::Unknown);
    }
    #[test]
    fn failed_interrupted_and_legacy_completions_are_not_success() {
        for (outcome, expected) in [
            (Some(TurnOutcome::Failed), WaitOutcome::Failed),
            (Some(TurnOutcome::Interrupted), WaitOutcome::Interrupted),
            (None, WaitOutcome::HistoryUnavailable),
        ] {
            let mut event = completed("target");
            if let AgentEvent::TurnCompleted {
                outcome: actual, ..
            } = &mut event
            {
                *actual = outcome;
            }
            let state = state(vec![event]);
            assert_eq!(
                WaitTarget::Turn("target".into()).outcome(&SessionState::Idle, Some(&state), 0),
                Some(expected)
            );
        }
        let old:AgentEvent=serde_json::from_str(r#"{"kind":"turn_completed","data":{"turn":"target","input_tokens":"0","output_tokens":"0"}}"#).unwrap();
        assert!(matches!(
            old,
            AgentEvent::TurnCompleted { outcome: None, .. }
        ));
    }
}
