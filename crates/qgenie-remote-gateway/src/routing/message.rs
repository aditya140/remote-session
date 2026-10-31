use crate::Error;
pub(super) const MAX_ROUTED_BYTES: usize = qgenie_remote_proto::MAX_JSON_BYTES + 65536;
use base64::{Engine, engine::general_purpose::STANDARD};
use hmac::{Hmac, Mac};
use qgenie_remote_proto::{
    ClientId, Counter, MachineId, MessageId, PtyFrame, SessionId, UserId,
    wire::{Envelope, Message},
};
use qgenie_remote_transport::ws::Frame;
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::{
    collections::{HashSet, VecDeque},
    time::{Duration, Instant},
};
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub(super) enum Data {
    Json(Box<Envelope>),
    Binary(String),
}
impl Data {
    pub fn from_frame(frame: Frame) -> Result<Self, Error> {
        match frame {
            Frame::Text(frame) => {
                // Authentication credentials never enter Redis or a native dispatch frame.
                if matches!(frame.message, Message::Reauthenticate { .. }) {
                    return Err(Error::Invalid);
                }
                frame.encode().map_err(|_| Error::Invalid)?;
                Ok(Self::Json(frame))
            }
            Frame::Binary(frame) => Ok(Self::Binary(
                STANDARD.encode(frame.encode().map_err(|_| Error::Invalid)?),
            )),
        }
    }
    pub fn frame(&self) -> Result<Frame, Error> {
        match self {
            Self::Json(frame) => {
                frame.encode().map_err(|_| Error::Invalid)?;
                Ok(Frame::Text(frame.clone()))
            }
            Self::Binary(data) => {
                if data.len() > 65536 {
                    return Err(Error::Invalid);
                }
                let bytes = STANDARD.decode(data).map_err(|_| Error::Invalid)?;
                Ok(Frame::Binary(
                    PtyFrame::decode(&bytes).map_err(|_| Error::Invalid)?,
                ))
            }
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
pub(super) struct Packet {
    pub id: MessageId,
    pub source: String,
    pub destination: String,
    pub at_ms: Counter,
    pub body: Body,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(super) enum Body {
    Request {
        owner: UserId,
        client: ClientId,
        expires: Counter,
        machine: MachineId,
        generation: Counter,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        access: Option<qgenie_remote_proto::wire::HostAccess>,
        frame: Data,
    },
    Response {
        owner: UserId,
        client: ClientId,
        machine: MachineId,
        generation: Counter,
        frame: Data,
    },
    Event {
        owner: UserId,
        machine: MachineId,
        generation: Counter,
        session: Option<SessionId>,
        frame: Data,
    },
    Changed {
        owner: UserId,
        machine: MachineId,
    },
    SessionMetadataChanged {
        owner: UserId,
        machine: MachineId,
        session: SessionId,
    },
    ApprovalChanged {
        owner: UserId,
        machine: MachineId,
        session: SessionId,
        approval: qgenie_remote_proto::ApprovalId,
    },
    Resync {
        owner: UserId,
        machine: MachineId,
        generation: Counter,
    },
    Hint {
        owner: UserId,
        machine: MachineId,
        connection: ClientId,
        generation: Counter,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        access: Option<qgenie_remote_proto::wire::HostAccess>,
    },
}
/// Sign the exact payload bytes with a domain independent of machine tokens.
pub(super) struct Signer([u8; 32]);
impl Signer {
    pub fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
    pub fn encode(&self, packet: &Packet) -> Result<Vec<u8>, Error> {
        let payload = serde_json::to_vec(packet).map_err(|_| Error::Invalid)?;
        if payload.len() > MAX_ROUTED_BYTES {
            return Err(Error::Invalid);
        }
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.0).map_err(|_| Error::Configuration)?;
        mac.update(b"qgenie-remote:routing:v1\0");
        mac.update(&payload);
        let mut bytes = mac.finalize().into_bytes().to_vec();
        bytes.extend_from_slice(&payload);
        Ok(bytes)
    }
    pub fn decode(&self, bytes: &[u8], destination: &str, now: u64) -> Result<Packet, Error> {
        if bytes.len() < 33 || bytes.len() > MAX_ROUTED_BYTES + 32 {
            return Err(Error::Invalid);
        }
        let mut mac = Hmac::<Sha256>::new_from_slice(&self.0).map_err(|_| Error::Configuration)?;
        mac.update(b"qgenie-remote:routing:v1\0");
        mac.update(&bytes[32..]);
        mac.verify_slice(&bytes[..32])
            .map_err(|_| Error::Unauthenticated)?;
        let packet: Packet =
            qgenie_remote_proto::decode_json_bounded(&bytes[32..], MAX_ROUTED_BYTES)
                .map_err(|_| Error::Invalid)?;
        if packet.destination != destination
            || packet.at_ms.0 > now.saturating_add(5000)
            || now.saturating_sub(packet.at_ms.0) > 10000
            || packet.source.len() > 128
            || !packet
                .source
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(Error::Unauthenticated);
        }
        Ok(packet)
    }
}
/// Do not evict unexpired request identities: overload cannot permit replayed mutations.
pub(super) struct ReplayWindow {
    ids: HashSet<MessageId>,
    expiry: VecDeque<(Instant, MessageId)>,
}
impl ReplayWindow {
    pub fn new() -> Self {
        Self {
            ids: HashSet::new(),
            expiry: VecDeque::new(),
        }
    }
    pub fn accept(&mut self, id: MessageId) -> Result<(), Error> {
        let now = Instant::now();
        while self
            .expiry
            .front()
            .is_some_and(|(deadline, _)| *deadline <= now)
        {
            let (_, id) = self.expiry.pop_front().unwrap();
            self.ids.remove(&id);
        }
        if self.ids.contains(&id) {
            return Err(Error::Conflict);
        }
        if self.ids.len() >= 65536 {
            return Err(Error::Overloaded);
        }
        self.ids.insert(id);
        self.expiry.push_back((now + Duration::from_secs(15), id));
        Ok(())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maximum_public_nesting_survives_the_internal_routing_container() {
        let mut payload = serde_json::json!(0);
        let mut accepted = None;
        for _ in 0..70 {
            payload = serde_json::json!([payload]);
            let mut envelope =
                Envelope::new(Message::Event(qgenie_remote_proto::wire::SequencedEvent {
                    seq: Counter(1),
                    at: qgenie_remote_proto::wire::Timestamp::from_unix_millis(1).unwrap(),
                    event: qgenie_remote_proto::wire::AgentEvent::Opaque {
                        native_kind: "fixture".into(),
                        payload: payload.clone(),
                    },
                }));
            envelope.seq = Some(Counter(1));
            envelope.session = Some(qgenie_remote_proto::SessionId::new());
            let bytes = envelope.encode().unwrap();
            if Envelope::decode(&bytes).is_ok() {
                accepted = Some(envelope);
            } else {
                break;
            }
        }
        let packet = Packet {
            id: MessageId::new(),
            source: "node-a".into(),
            destination: "qgr:user:fixture".into(),
            at_ms: Counter(1000),
            body: Body::Event {
                owner: UserId::new(),
                machine: MachineId::new(),
                generation: Counter(1),
                session: None,
                frame: Data::Json(Box::new(accepted.unwrap())),
            },
        };
        let signer = Signer::new([9; 32]);
        let signed = signer.encode(&packet).unwrap();
        signer.decode(&signed, &packet.destination, 1000).unwrap();
    }
    #[test]
    fn redis_payloads_require_authenticity_destination_freshness_and_unique_requests() {
        let signer = Signer::new([7; 32]);
        let packet = Packet {
            id: MessageId::new(),
            source: "node-a".into(),
            destination: "qgr:node:node-b".into(),
            at_ms: Counter(10000),
            body: Body::Changed {
                owner: UserId::new(),
                machine: MachineId::new(),
            },
        };
        let bytes = signer.encode(&packet).unwrap();
        assert!(signer.decode(&bytes, &packet.destination, 10001).is_ok());
        assert!(signer.decode(&bytes, "qgr:node:other", 10001).is_err());
        assert!(signer.decode(&bytes, &packet.destination, 20001).is_err());
        let mut forged = bytes.clone();
        *forged.last_mut().unwrap() ^= 1;
        assert!(signer.decode(&forged, &packet.destination, 10001).is_err());
        assert!(
            Signer::new([8; 32])
                .decode(&bytes, &packet.destination, 10001)
                .is_err()
        );
        let mut replay = ReplayWindow::new();
        replay.accept(packet.id).unwrap();
        assert!(replay.accept(packet.id).is_err());
    }
}
