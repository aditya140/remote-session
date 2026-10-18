//! A native checkpoint is released only after all matching pages and its final head.
use crate::LiveHandle;
use base64::{Engine, engine::general_purpose::STANDARD};
use qgenie_remote_proto::{
    wire::{Envelope, Message, SequencedEvent},
    *,
};

pub struct RemoteReplay {
    pub requested_after: Counter,
    pub floor: Counter,
    pub head: Counter,
    pub events: Vec<SequencedEvent>,
    pub more: bool,
    pub history_truncated: bool,
}
pub struct RemoteSnapshot {
    pub requested_after: Option<Counter>,
    pub epoch: String,
    pub seq: Counter,
    pub rows: u16,
    pub cols: u16,
    pub format: SnapshotFormat,
    pub data: Vec<u8>,
    pub floor: Counter,
    pub head: Counter,
    pub last_input: Counter,
}
pub enum RemoteAttachment {
    Replay(RemoteReplay),
    Snapshot(RemoteSnapshot),
}
impl RemoteSnapshot {
    pub fn agent_state(&self) -> Result<AgentState, RemoteError> {
        if self.format != SnapshotFormat::AgentStateV1 {
            return Err(invalid());
        }
        AgentState::decode(&self.data, self.seq)
    }
}
fn invalid() -> RemoteError {
    RemoteError::rejected(
        ErrorCode::InvalidFrame,
        "invalid native replay or checkpoint response",
    )
}
impl LiveHandle {
    pub fn session_changes_available(&self) -> bool {
        self.session_changes
    }
    pub async fn attach(
        &self,
        machine: MachineId,
        session: SessionId,
        after: Counter,
    ) -> Result<RemoteAttachment, RemoteError> {
        self.read_stream(machine, session, Message::Attach { after })
            .await
    }
    /// Continuations require an existing explicit attachment and do not renew control.
    pub async fn replay(
        &self,
        machine: MachineId,
        session: SessionId,
        after: Counter,
    ) -> Result<RemoteAttachment, RemoteError> {
        self.read_stream(machine, session, Message::ReplayRequest { after })
            .await
    }
    pub async fn capture(
        &self,
        machine: MachineId,
        session: SessionId,
    ) -> Result<RemoteAttachment, RemoteError> {
        self.read_stream(machine, session, Message::Capture).await
    }
    pub async fn detach(&self, machine: MachineId, session: SessionId) -> Result<(), RemoteError> {
        // Detach has no acknowledgment in v1; use the same owned outbound queue.
        let mut request = Envelope::new(Message::Detach);
        request.machine = Some(machine);
        request.session = Some(session);
        self.detach_stream(request).await
    }
    async fn read_stream(
        &self,
        machine: MachineId,
        session: SessionId,
        message: Message,
    ) -> Result<RemoteAttachment, RemoteError> {
        let requested_after = match message {
            Message::Attach { after } | Message::ReplayRequest { after } => Some(after),
            _ => None,
        };
        if !self.replays {
            return Err(RemoteError {
                code: ErrorCode::UnsupportedCapability,
                outcome: OperationOutcome::NotSent,
                message: "gateway did not negotiate session-replay-v1".into(),
                retryable: false,
            });
        }
        let mut request = Envelope::new(message);
        request.machine = Some(machine);
        request.session = Some(session);
        let mut frames = self.request(request).await?;
        if frames.len() == 1
            && let Message::Replay {
                floor,
                head,
                events,
                more,
                history_truncated,
            } = frames.remove(0).message
        {
            return Ok(RemoteAttachment::Replay(RemoteReplay {
                requested_after: requested_after.ok_or_else(invalid)?,
                floor,
                head,
                events,
                more,
                history_truncated,
            }));
        }
        let Some(last) = frames.pop() else {
            return Err(invalid());
        };
        let Message::Head {
            floor,
            head,
            last_input,
        } = last.message
        else {
            return Err(invalid());
        };
        let Some(first) = frames.first() else {
            return Err(invalid());
        };
        let Message::Snapshot { snapshot } = &first.message else {
            return Err(invalid());
        };
        let (seq, rows, cols, format, epoch, total) = (
            snapshot.seq,
            snapshot.rows,
            snapshot.cols,
            snapshot.format,
            snapshot.epoch.clone(),
            snapshot.total_pages,
        );
        if total == 0
            || epoch.is_empty()
            || epoch.len() > 128
            || usize::from(total) != frames.len()
            || seq > head
            || floor.0 > head.0.saturating_add(1)
        {
            return Err(invalid());
        }
        let mut data = Vec::new();
        for (index, frame) in frames.into_iter().enumerate() {
            let Message::Snapshot { snapshot } = frame.message else {
                return Err(invalid());
            };
            if snapshot.page != index as u16
                || (
                    snapshot.seq,
                    snapshot.rows,
                    snapshot.cols,
                    snapshot.format,
                    snapshot.total_pages,
                ) != (seq, rows, cols, format, total)
                || snapshot.epoch != epoch
            {
                return Err(invalid());
            }
            let chunk = STANDARD.decode(snapshot.data).map_err(|_| invalid())?;
            if chunk.len() > 256 * 1024 || data.len() + chunk.len() > 8 * 1024 * 1024 {
                return Err(invalid());
            }
            data.extend(chunk);
        }
        if format == SnapshotFormat::AgentStateV1 {
            if !self.agents {
                return Err(RemoteError::rejected(
                    ErrorCode::UnsupportedCapability,
                    "structured checkpoint was not negotiated",
                ));
            }
            AgentState::decode(&data, seq)?;
        }
        Ok(RemoteAttachment::Snapshot(RemoteSnapshot {
            requested_after,
            epoch,
            seq,
            rows,
            cols,
            format,
            data,
            floor,
            head,
            last_input,
        }))
    }
}
