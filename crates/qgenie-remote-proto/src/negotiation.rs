use crate::{ChallengeId, ErrorCode, KeyId, MachineId, RemoteError};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Version {
    pub major: u16,
    pub minor: u16,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Hello {
    pub versions: Vec<Version>,
    pub required: BTreeSet<String>,
    pub optional: BTreeSet<String>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Welcome {
    pub version: Version,
    pub capabilities: BTreeSet<String>,
}

pub fn negotiate(
    peer: &Hello,
    local_versions: &[Version],
    local_caps: &BTreeSet<String>,
) -> Result<Welcome, RemoteError> {
    if peer.versions.is_empty()
        || peer.versions.len() > 16
        || peer.required.len() + peer.optional.len() > 64
        || peer
            .required
            .iter()
            .chain(&peer.optional)
            .any(|c| c.is_empty() || c.len() > 64)
    {
        return Err(RemoteError::rejected(
            ErrorCode::InvalidFrame,
            "invalid handshake limits",
        ));
    }
    let version = peer
        .versions
        .iter()
        .filter(|v| local_versions.contains(v))
        .max()
        .copied()
        .ok_or_else(|| {
            RemoteError::rejected(ErrorCode::UnsupportedVersion, "no common protocol version")
        })?;
    if !peer.required.is_subset(local_caps) {
        return Err(RemoteError::rejected(
            ErrorCode::UnsupportedCapability,
            "required capability is unavailable",
        ));
    }
    let capabilities = peer
        .required
        .union(&peer.optional)
        .filter(|cap| local_caps.contains(*cap))
        .cloned()
        .collect();
    Ok(Welcome {
        version,
        capabilities,
    })
}

/// Canonical signing input only. Key storage/signing belong to identity providers.
pub struct MachineChallenge {
    pub origin: String,
    pub machine: MachineId,
    pub key: KeyId,
    pub challenge: ChallengeId,
    pub nonce: [u8; 32],
    pub issued_at_ms: u64,
    pub expires_at_ms: u64,
}
impl MachineChallenge {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, RemoteError> {
        let invalid = || {
            RemoteError::rejected(
                ErrorCode::InvalidFrame,
                "invalid canonical machine challenge",
            )
        };
        validate_gateway_origin(&self.origin)?;
        if self.expires_at_ms <= self.issued_at_ms
            || self.expires_at_ms - self.issued_at_ms > 60_000
        {
            return Err(invalid());
        }
        let mut bytes = b"qgenie-remote:machine-auth:v1\0".to_vec();
        bytes.extend_from_slice(&(self.origin.len() as u16).to_be_bytes());
        bytes.extend_from_slice(self.origin.as_bytes());
        bytes.extend_from_slice(self.machine.as_bytes());
        bytes.extend_from_slice(self.key.as_bytes());
        bytes.extend_from_slice(self.challenge.as_bytes());
        bytes.extend_from_slice(&self.nonce);
        bytes.extend_from_slice(&self.issued_at_ms.to_be_bytes());
        bytes.extend_from_slice(&self.expires_at_ms.to_be_bytes());
        Ok(bytes)
    }
}

/// Validate the canonical HTTPS origin used by durable native owner bindings.
pub fn validate_gateway_origin(origin: &str) -> Result<(), RemoteError> {
    let invalid =
        || RemoteError::rejected(ErrorCode::InvalidFrame, "invalid canonical gateway origin");
    let rest = origin.strip_prefix("https://").ok_or_else(invalid)?;
    if origin.len() > 2048
        || rest.is_empty()
        || rest.contains(['/', '?', '#', '@'])
        || !rest.is_ascii()
    {
        return Err(invalid());
    }
    let (host, port) = if let Some(ip) = rest.strip_prefix('[') {
        let (ip, suffix) = ip.split_once(']').ok_or_else(invalid)?;
        let parsed: std::net::Ipv6Addr = ip.parse().map_err(|_| invalid())?;
        if ip != parsed.to_string() {
            return Err(invalid());
        }
        (
            format!("[{ip}]"),
            suffix.strip_prefix(':').map(str::to_owned),
        )
    } else {
        let (host, port) = rest
            .split_once(':')
            .map_or((rest, None), |(h, p)| (h, Some(p.to_owned())));
        if host.len() > 253
            || host.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
            })
        {
            return Err(invalid());
        }
        (host.to_owned(), port)
    };
    let port = if let Some(port) = port {
        let number: u16 = port.parse().map_err(|_| invalid())?;
        if number == 0 || number == 443 || port != number.to_string() {
            return Err(invalid());
        }
        format!(":{port}")
    } else {
        String::new()
    };
    if origin != format!("https://{host}{port}") {
        return Err(invalid());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn synthetic_versions_negotiate_without_inventing_historical_cli_support() {
        let versions = [
            Version { major: 1, minor: 0 },
            Version { major: 1, minor: 1 },
            Version { major: 2, minor: 0 },
        ];
        let caps = BTreeSet::from(["pty".into()]);
        let mut peer = Hello {
            versions: versions.to_vec(),
            required: caps.clone(),
            optional: BTreeSet::from(["codex".into()]),
        };
        let welcome = negotiate(&peer, &versions[..2], &caps).unwrap();
        assert_eq!(welcome.version, versions[1]);
        assert_eq!(welcome.capabilities, caps);
        peer.required.insert("codex".into());
        assert!(matches!(
            negotiate(&peer, &versions, &caps).unwrap_err().code,
            ErrorCode::UnsupportedCapability
        ));
        peer.required = caps.clone();
        peer.versions = vec![Version { major: 9, minor: 0 }];
        assert!(matches!(
            negotiate(&peer, &versions, &caps).unwrap_err().code,
            ErrorCode::UnsupportedVersion
        ));
    }
    #[test]
    fn challenge_is_domain_bound_and_rejects_noncanonical_origins() {
        let mut challenge = MachineChallenge {
            origin: "https://example.test".into(),
            machine: MachineId::from_bytes([1; 16]),
            key: KeyId::from_bytes([2; 16]),
            challenge: ChallengeId::from_bytes([3; 16]),
            nonce: [4; 32],
            issued_at_ms: 1,
            expires_at_ms: 60_001,
        };
        let expected = challenge.signing_bytes().unwrap();
        assert!(expected.starts_with(b"qgenie-remote:machine-auth:v1\0\0\x14https://example.test"));
        for origin in [
            "https://EXAMPLE.test",
            "https://example.test:443",
            "https://example.test/",
            "http://example.test",
            "https://x@example.test",
            "https://example.test:0444",
        ] {
            challenge.origin = origin.into();
            assert!(challenge.signing_bytes().is_err(), "{origin}");
        }
        challenge.origin = "https://other.test".into();
        assert_ne!(challenge.signing_bytes().unwrap(), expected);
    }
}
