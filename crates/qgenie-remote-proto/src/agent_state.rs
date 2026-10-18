//! Versioned bounded structured checkpoint. Truncation never implies a complete transcript.
use crate::{
    Counter, ErrorCode, RemoteError, decode_json,
    wire::{AgentEvent, SequencedEvent},
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentState {
    pub v: u16,
    pub seq: Counter,
    pub history_truncated: bool,
    pub unconfirmed_tail: bool,
    pub events: Vec<SequencedEvent>,
}
const BYTES: usize = 512 * 1024;
const EVENTS: usize = 256;
fn invalid() -> RemoteError {
    RemoteError::rejected(
        ErrorCode::HistoryTruncated,
        "invalid structured agent checkpoint",
    )
}
impl Default for AgentState {
    fn default() -> Self {
        Self {
            v: 1,
            seq: Counter(0),
            history_truncated: false,
            unconfirmed_tail: false,
            events: Vec::new(),
        }
    }
}
impl AgentState {
    pub fn decode(data: &[u8], expected: Counter) -> Result<Self, RemoteError> {
        if data.len() > BYTES {
            return Err(invalid());
        }
        let state: Self = decode_json(data)?;
        if state.v != 1 || state.seq != expected || state.events.len() > EVENTS {
            return Err(invalid());
        }
        let mut cursor = None;
        for event in &state.events {
            if event.seq.0 == 0
                || event.seq > expected
                || cursor
                    .is_some_and(|previous: Counter| previous.0.checked_add(1) != Some(event.seq.0))
                || matches!(
                    event.event,
                    AgentEvent::PtyOutput { .. } | AgentEvent::Resize { .. }
                )
            {
                return Err(invalid());
            }
            cursor = Some(event.seq);
        }
        if cursor.is_some_and(|cursor| cursor != expected)
            || (state
                .events
                .first()
                .is_some_and(|event| event.seq != Counter(1))
                && !state.history_truncated)
            || (state.events.is_empty() && expected != Counter(0) && !state.history_truncated)
        {
            return Err(invalid());
        }
        Ok(state)
    }
    pub fn push(&mut self, event: SequencedEvent) -> Result<(), RemoteError> {
        if self.seq.0.checked_add(1) != Some(event.seq.0)
            || matches!(
                event.event,
                AgentEvent::PtyOutput { .. } | AgentEvent::Resize { .. }
            )
        {
            return Err(invalid());
        }
        self.seq = event.seq;
        self.history_truncated |= matches!(event.event, AgentEvent::Gap { .. });
        self.events.push(event);
        while self.events.len() > EVENTS
            || serde_json::to_vec(self).map_err(|_| invalid())?.len() > BYTES
        {
            self.events.remove(0);
            self.history_truncated = true;
        }
        Ok(())
    }
    pub fn encode(&self) -> Result<Vec<u8>, RemoteError> {
        let data = serde_json::to_vec(self).map_err(|_| invalid())?;
        Self::decode(&data, self.seq)?;
        Ok(data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wire::Timestamp;
    #[test]
    fn bounded_cutover_is_explicit_and_rejects_cursor_regression_or_terminal_data() {
        let mut state = AgentState::default();
        for seq in 1..=300 {
            state
                .push(SequencedEvent {
                    seq: Counter(seq),
                    at: Timestamp::from_unix_millis(1).unwrap(),
                    event: AgentEvent::MessageDelta {
                        message: "m".into(),
                        text: "x".repeat(4096),
                    },
                })
                .unwrap();
        }
        assert!(state.history_truncated);
        assert!(state.events.len() < 256);
        let data = state.encode().unwrap();
        assert!(data.len() <= BYTES);
        assert_eq!(
            AgentState::decode(&data, Counter(300)).unwrap().seq,
            Counter(300)
        );
        assert!(AgentState::decode(&data, Counter(299)).is_err());
        assert!(
            state
                .push(SequencedEvent {
                    seq: Counter(302),
                    at: Timestamp::from_unix_millis(1).unwrap(),
                    event: AgentEvent::Idle
                })
                .is_err()
        );
        assert!(
            state
                .push(SequencedEvent {
                    seq: Counter(301),
                    at: Timestamp::from_unix_millis(1).unwrap(),
                    event: AgentEvent::PtyOutput {
                        data: "eA==".into()
                    }
                })
                .is_err()
        );
    }
}
