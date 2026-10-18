use crate::{Counter, ErrorCode, MAX_PTY_BYTES, RemoteError, SessionId};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PtyPayload {
    Input {
        host_generation: Counter,
        controller_generation: Counter,
        bytes: Vec<u8>,
    },
    Output(Vec<u8>),
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PtyFrame {
    pub session: SessionId,
    pub seq: Counter,
    pub payload: PtyPayload,
}

impl PtyFrame {
    pub fn encode(&self) -> Result<Vec<u8>, RemoteError> {
        if self.seq.0 == 0 {
            return Err(invalid());
        }
        let (kind, data) = match &self.payload {
            PtyPayload::Input { bytes, .. } => (1, bytes),
            PtyPayload::Output(bytes) => (2, bytes),
        };
        if data.is_empty() || data.len() > MAX_PTY_BYTES {
            return Err(invalid());
        }
        let mut bytes = Vec::with_capacity(44 + data.len());
        bytes.extend_from_slice(&[kind, 0, 0, 0]);
        bytes.extend_from_slice(self.session.as_bytes());
        bytes.extend_from_slice(&self.seq.0.to_be_bytes());
        if let PtyPayload::Input {
            host_generation,
            controller_generation,
            ..
        } = &self.payload
        {
            if host_generation.0 == 0 || controller_generation.0 == 0 {
                return Err(invalid());
            }
            bytes.extend_from_slice(&host_generation.0.to_be_bytes());
            bytes.extend_from_slice(&controller_generation.0.to_be_bytes());
        }
        bytes.extend_from_slice(data);
        Ok(bytes)
    }

    pub fn decode(bytes: &[u8]) -> Result<Self, RemoteError> {
        if bytes.len() < 29 || bytes[1..4] != [0, 0, 0] {
            return Err(invalid());
        }
        let session = SessionId::from_bytes(bytes[4..20].try_into().map_err(|_| invalid())?);
        let seq = Counter(u64::from_be_bytes(
            bytes[20..28].try_into().map_err(|_| invalid())?,
        ));
        if seq.0 == 0 {
            return Err(invalid());
        }
        let payload = match bytes[0] {
            1 if bytes.len() >= 45 && bytes.len() <= 44 + MAX_PTY_BYTES => {
                let host_generation = Counter(u64::from_be_bytes(
                    bytes[28..36].try_into().map_err(|_| invalid())?,
                ));
                let controller_generation = Counter(u64::from_be_bytes(
                    bytes[36..44].try_into().map_err(|_| invalid())?,
                ));
                if host_generation.0 == 0 || controller_generation.0 == 0 {
                    return Err(invalid());
                }
                PtyPayload::Input {
                    host_generation,
                    controller_generation,
                    bytes: bytes[44..].to_vec(),
                }
            }
            2 if bytes.len() <= 28 + MAX_PTY_BYTES => PtyPayload::Output(bytes[28..].to_vec()),
            _ => return Err(invalid()),
        };
        Ok(Self {
            session,
            seq,
            payload,
        })
    }
}
fn invalid() -> RemoteError {
    RemoteError::rejected(ErrorCode::InvalidFrame, "invalid or oversized PTY frame")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn binary_roundtrip_and_every_truncated_header_is_rejected() {
        let frame = PtyFrame {
            session: SessionId::new(),
            seq: Counter(u64::MAX),
            payload: PtyPayload::Input {
                host_generation: Counter(4),
                controller_generation: Counter(8),
                bytes: vec![1, 2, 3],
            },
        };
        let bytes = frame.encode().unwrap();
        assert_eq!(PtyFrame::decode(&bytes).unwrap(), frame);
        for length in 0..45 {
            assert!(PtyFrame::decode(&bytes[..length]).is_err());
        }
        let mut invalid = bytes.clone();
        invalid[2] = 1;
        assert!(PtyFrame::decode(&invalid).is_err());
        let frame = PtyFrame {
            payload: PtyPayload::Output(vec![0; MAX_PTY_BYTES + 1]),
            ..frame
        };
        assert!(frame.encode().is_err());
    }
}
