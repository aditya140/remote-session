//! Bounded contiguous output delivery. A checkpoint is the only way to cross a known gap.
use crate::{LiveFrame, RemoteAttachment, RemoteSnapshot};
use base64::{Engine, engine::general_purpose::STANDARD};
use qgenie_remote_proto::{
    wire::{AgentEvent, Message, Timestamp},
    *,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

pub struct OutputItem {
    pub seq: Counter,
    pub at: Option<Timestamp>,
    pub event: AgentEvent,
}
pub struct OutputUpdate {
    pub snapshot: Option<RemoteSnapshot>,
    pub events: Vec<OutputItem>,
}
impl OutputUpdate {
    fn empty() -> Self {
        Self {
            snapshot: None,
            events: Vec::new(),
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadPlan {
    Attach { after: Counter },
    Replay { after: Counter },
    Capture,
    Idle,
    UnconfirmedTail,
}
pub struct OutputCursor {
    machine: MachineId,
    session: SessionId,
    cursor: Counter,
    floor: Counter,
    head: Counter,
    attached: bool,
    baseline: bool,
    capture: bool,
    lost: bool,
    pending: BTreeMap<Counter, (OutputItem, [u8; 32], usize)>,
    bytes: usize,
    recent: BTreeMap<Counter, [u8; 32]>,
}
fn invalid(message: &str) -> RemoteError {
    RemoteError::rejected(ErrorCode::HistoryTruncated, message)
}
impl OutputCursor {
    /// A terminal viewer waits for an explicit checkpoint before releasing events.
    pub fn terminal(machine: MachineId, session: SessionId) -> Self {
        Self::checkpointed(machine, session)
    }
    /// Full terminal/agent renderers install a checkpoint before releasing a tail.
    pub fn checkpointed(machine: MachineId, session: SessionId) -> Self {
        Self::new(machine, session, false, Counter(0))
    }
    /// Structured event consumers begin with an empty event timeline.
    pub fn events(machine: MachineId, session: SessionId) -> Self {
        Self::new(machine, session, true, Counter(0))
    }
    /// The caller retains a renderer at this exact cursor; no credential or input is restored.
    pub fn resume(machine: MachineId, session: SessionId, after: Counter) -> Self {
        Self::new(machine, session, true, after)
    }
    fn new(machine: MachineId, session: SessionId, baseline: bool, cursor: Counter) -> Self {
        Self {
            machine,
            session,
            cursor,
            floor: Counter(0),
            head: cursor,
            attached: false,
            baseline,
            capture: false,
            lost: false,
            pending: BTreeMap::new(),
            bytes: 0,
            recent: BTreeMap::new(),
        }
    }
    pub fn cursor(&self) -> Counter {
        self.cursor
    }
    pub fn head(&self) -> Counter {
        self.head
    }
    pub fn read_plan(&self) -> ReadPlan {
        if self.lost {
            ReadPlan::UnconfirmedTail
        } else if !self.attached {
            ReadPlan::Attach { after: self.cursor }
        } else if self.capture || !self.baseline {
            ReadPlan::Capture
        } else if self.cursor < self.head || !self.pending.is_empty() {
            ReadPlan::Replay { after: self.cursor }
        } else {
            ReadPlan::Idle
        }
    }
    /// Reconnect/reattachment is explicit. Controller state and input are never restored here.
    pub fn disconnected(&mut self) {
        self.attached = false;
    }
    pub fn attachment(
        &mut self,
        attachment: RemoteAttachment,
    ) -> Result<OutputUpdate, RemoteError> {
        self.attached = true;
        match attachment {
            RemoteAttachment::Snapshot(snapshot) => {
                if snapshot
                    .requested_after
                    .is_some_and(|after| snapshot.seq < after)
                {
                    self.lost = true;
                    return Err(invalid(
                        "native checkpoint regressed behind rendered output; tail is unconfirmed",
                    ));
                }
                if snapshot.seq < self.cursor {
                    self.head = self.head.max(snapshot.head);
                    if self.capture {
                        return Err(invalid(
                            "checkpoint predates the observed gap; capture again",
                        ));
                    }
                    return Ok(OutputUpdate::empty());
                }
                self.cursor = snapshot.seq;
                self.floor = self.floor.max(snapshot.floor);
                self.head = self.head.max(snapshot.head);
                self.baseline = true;
                self.capture = false;
                self.recent.clear();
                let old: Vec<_> = self
                    .pending
                    .range(..=self.cursor)
                    .map(|(seq, _)| *seq)
                    .collect();
                for seq in old {
                    self.remove(seq);
                }
                let events = self.drain()?;
                if self.floor.0 > self.cursor.0.saturating_add(1) {
                    self.capture = true;
                }
                Ok(OutputUpdate {
                    snapshot: Some(snapshot),
                    events,
                })
            }
            RemoteAttachment::Replay(page) => {
                if page.head < page.requested_after {
                    self.lost = true;
                    return Err(invalid(
                        "native replay regressed behind its requested cursor; tail is unconfirmed",
                    ));
                }
                self.observe_head(page.floor, page.head)?;
                if page.history_truncated || !self.baseline {
                    self.capture = true;
                    return Ok(OutputUpdate::empty());
                }
                let mut previous = None;
                for event in page.events {
                    if previous.is_some_and(|seq| event.seq <= seq) || event.seq > page.head {
                        self.capture = true;
                        return Err(invalid("replay event order is invalid"));
                    }
                    previous = Some(event.seq);
                    self.stage(OutputItem {
                        seq: event.seq,
                        at: Some(event.at),
                        event: event.event,
                    })?;
                }
                Ok(OutputUpdate {
                    snapshot: None,
                    events: self.drain()?,
                })
            }
        }
    }
    /// Frames for another target are ignored, so one owned connection can serve several cursors.
    pub fn frame(&mut self, frame: &LiveFrame) -> Result<OutputUpdate, RemoteError> {
        match frame {
            LiveFrame::Text(envelope) if envelope.machine == Some(self.machine) => {
                if let Message::Sessions { sessions } | Message::SessionUpdates { sessions } =
                    &envelope.message
                    && let Some(session) = sessions.iter().find(|session| {
                        session.id == self.session && session.machine == self.machine
                    })
                {
                    self.observe_head(self.floor, self.head.max(session.head))?;
                }
                if envelope.session != Some(self.session) {
                    return Ok(OutputUpdate::empty());
                }
                match &envelope.message {
                    Message::Event(event) => self.stage(OutputItem {
                        seq: event.seq,
                        at: Some(event.at.clone()),
                        event: event.event.clone(),
                    })?,
                    Message::Head { floor, head, .. } => self.observe_head(*floor, *head)?,
                    Message::Error(error) if error.code == ErrorCode::HistoryTruncated => {
                        self.capture = true
                    }
                    _ => {}
                }
            }
            LiveFrame::Binary(frame) if frame.session == self.session => {
                if let PtyPayload::Output(bytes) = &frame.payload {
                    self.stage(OutputItem {
                        seq: frame.seq,
                        at: None,
                        event: AgentEvent::PtyOutput {
                            data: STANDARD.encode(bytes),
                        },
                    })?;
                }
            }
            _ => {}
        }
        Ok(OutputUpdate {
            snapshot: None,
            events: self.drain()?,
        })
    }
    fn observe_head(&mut self, floor: Counter, head: Counter) -> Result<(), RemoteError> {
        if floor.0 > head.0.saturating_add(1) {
            self.lost = true;
            return Err(invalid("native output head regressed; tail is unconfirmed"));
        }
        self.floor = self.floor.max(floor);
        self.head = self.head.max(head);
        if self.floor.0 > self.cursor.0.saturating_add(1) {
            self.capture = true;
        }
        Ok(())
    }
    fn stage(&mut self, item: OutputItem) -> Result<(), RemoteError> {
        if item.seq.0 == 0 {
            return Err(invalid("output sequence must be positive"));
        }
        let body = serde_json::to_vec(&item.event).map_err(|_| invalid("invalid output event"))?;
        if body.len() > MAX_JSON_BYTES {
            self.capture = true;
            return Err(invalid("output event exceeded its byte budget"));
        }
        let hash: [u8; 32] = Sha256::digest(&body).into();
        if let Some(previous) = self
            .recent
            .get(&item.seq)
            .or_else(|| self.pending.get(&item.seq).map(|(_, hash, _)| hash))
        {
            if *previous != hash {
                self.lost = true;
                return Err(invalid("duplicate output identity has different content"));
            }
            return Ok(());
        }
        if item.seq <= self.cursor {
            return Ok(());
        }
        self.head = self.head.max(item.seq);
        if self.pending.len() >= 256 || self.bytes + body.len() > 2 * 1024 * 1024 {
            self.pending.clear();
            self.bytes = 0;
            self.capture = true;
            return Err(RemoteError::rejected(
                ErrorCode::Overloaded,
                "output reorder budget exceeded; request a fresh checkpoint",
            ));
        }
        self.bytes += body.len();
        self.pending.insert(item.seq, (item, hash, body.len()));
        Ok(())
    }
    fn remove(&mut self, seq: Counter) -> Option<(OutputItem, [u8; 32])> {
        self.pending.remove(&seq).map(|(item, hash, size)| {
            self.bytes -= size;
            (item, hash)
        })
    }
    fn drain(&mut self) -> Result<Vec<OutputItem>, RemoteError> {
        let mut ready = Vec::new();
        if !self.attached || !self.baseline || self.capture || self.lost {
            return Ok(ready);
        }
        while let Some(next) = self.cursor.0.checked_add(1).map(Counter) {
            let Some((item, hash)) = self.remove(next) else {
                break;
            };
            self.cursor = next;
            let gap = matches!(item.event, AgentEvent::Gap { .. });
            self.recent.insert(next, hash);
            while self.recent.len() > 128 {
                self.recent.pop_first();
            }
            ready.push(item);
            if gap {
                self.capture = true;
                break;
            }
        }
        Ok(ready)
    }
}
#[cfg(test)]
mod tests;
