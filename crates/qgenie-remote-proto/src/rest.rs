//! REST identities and proof-of-possession DTOs, separate from WSS operations.
use crate::{
    ChallengeId, Counter, InstallationId, KeyId, MachineChallenge, MachineId, RemoteError,
    RequestId, UserId, wire::Timestamp,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
macro_rules! types { ($($item:item)*)=>{$(#[derive(Debug,Clone,Serialize,Deserialize)] #[cfg_attr(feature="schema",derive(schemars::JsonSchema))] $item)*}; }
types! {
    pub struct EnrollmentRequest {
        pub installation:InstallationId,pub key:KeyId,pub public_key:String,pub os_user:String,
        pub hostname:String,pub os:String,pub arch:String,pub cli_version:String,pub name:String,
        #[serde(default)] pub labels:BTreeMap<String,String>,
        #[serde(default)] pub recovery:Option<Recovery>,
    }
    pub struct Recovery { pub machine:MachineId,pub confirmed:bool }
    pub struct Proof { pub challenge:ChallengeId,pub signature:String }
    pub struct ChallengeRequest { pub key:KeyId }
    pub struct Challenge {
        pub origin:String,pub machine:MachineId,pub key:KeyId,pub challenge:ChallengeId,
        pub nonce:String,pub issued_at_ms:Counter,pub expires_at_ms:Counter,
    }
    pub struct Enrolled { pub machine:MachineId,pub key:KeyId }
    pub struct MachinePatch { pub name:Option<String>,pub labels:Option<BTreeMap<String,String>> }
    pub struct MachineRevoke {pub confirmed:bool}
    pub struct Revoked {pub machine:MachineId}
    #[serde(deny_unknown_fields)]
    pub struct RevokeAccess { pub request:RequestId,pub target:RevocationTarget,pub confirmed:bool }
    #[derive(PartialEq,Eq)]
    #[serde(tag="kind",rename_all="snake_case",deny_unknown_fields)]
    pub enum RevocationTarget { Global,User {user:UserId},Machine {machine:MachineId} }
    pub struct AccessRevocation {
        pub request:RequestId,pub target:RevocationTarget,pub revision:Counter,
        pub committed_at:Timestamp,pub propagation:RevocationPropagation,
    }
    /// Completed requires explicit cross-node/native propagation evidence.
    #[serde(rename_all="snake_case")]
    pub enum RevocationPropagation {Pending,Completed}
    pub struct HistoryQuery {
        pub stream_epoch:crate::RecordingEpochId,
        #[serde(default)] pub after:Option<String>,
        #[serde(default="history_page_limit")] pub limit:u16,
    }
    #[serde(deny_unknown_fields)]
    pub struct HistoryStreamsQuery {
        #[serde(default)] pub after:Option<String>,
        #[serde(default="history_stream_limit")] pub limit:u16,
    }
    pub struct HistoryStream {pub stream_epoch:crate::RecordingEpochId,pub stream:crate::wire::RecordingStream}
    /// Only currently authorized visible content/gaps; never a capture-completeness claim.
    pub struct HistoryStreamsPage {pub session:crate::SessionId,pub machine:MachineId,pub streams:Vec<HistoryStream>,pub next:Option<String>}
    #[serde(deny_unknown_fields)]
    pub struct AdminHistoryStreamsQuery {pub history:HistoryStreamsQuery,pub reason:String,#[serde(default)] pub case_ref:Option<String>,#[serde(default)] pub include_held:bool}
    /// Independently request recovery authority before accepting any upload receipt.
    #[serde(deny_unknown_fields)]
    pub struct RecordingAuthorityRequest { pub challenge:ChallengeId }
    #[serde(deny_unknown_fields)]
    pub struct RecordingAuthority {
        pub challenge:ChallengeId,pub origin:String,pub owner:UserId,
        pub machine:MachineId,pub key:KeyId,pub backend_epoch:String,
    }
    /// Stored redacted bytes have a separate digest from the immutable upload/receipt digest.
    pub struct HistoricalChunk {
        pub id:crate::ChunkId,pub session:crate::SessionId,pub machine:MachineId,
        pub stream:crate::wire::RecordingStream,pub stream_epoch:crate::RecordingEpochId,
        pub from:Counter,pub to:Counter,pub captured_from:Timestamp,pub captured_to:Timestamp,
        pub codec:crate::wire::RecordingCodec,pub uncompressed_bytes:u32,
        pub stored_digest:String,pub data:String,pub redaction_saturated:bool,
    }
    #[serde(tag="kind",content="body",rename_all="snake_case")]
    pub enum HistoryItem { Content(HistoricalChunk),Gap(crate::wire::LossRange) }
    /// A bounded view of committed history. This does not certify native capture completeness.
    pub struct HistoryPage {
        pub session:crate::SessionId,pub machine:MachineId,pub stream_epoch:crate::RecordingEpochId,
        pub items:Vec<HistoryItem>,pub next:Option<String>,
    }
    #[serde(deny_unknown_fields)]
    pub struct AdminHistoryQuery {
        pub history:HistoryQuery,pub reason:String,
        #[serde(default)] pub case_ref:Option<String>,
        #[serde(default)] pub include_held:bool,
    }
    /// Revision 1 places a hold; each subsequent change/release increments it by one.
    /// Retrying the same revision is safe. A hold's affected user is immutable.
    #[serde(deny_unknown_fields)]
    pub struct HoldChange {
        pub id:crate::HoldId,pub user:UserId,pub revision:Counter,pub active:bool,
        #[serde(default)] pub captured_from:Option<Timestamp>,
        #[serde(default)] pub captured_to:Option<Timestamp>,
        #[serde(default)] pub case_ref:Option<String>,pub reason:String,
    }
    /// Success fences deletion immediately; bounded materialization may still be pending.
    pub struct HoldCommit { pub id:crate::HoldId,pub revision:Counter,pub active:bool }
    #[derive(PartialEq,Eq)]
    #[serde(rename_all="snake_case")]
    pub enum LifecycleStage {
        PreservedContent,PreservedGaps,ReleasedContentCoverage,ReleasedGapCoverage,
        Retiring,Retired,ExpiredGaps,ExpiredHeldContent,ExpiredHeldGaps,Idle,
    }
    pub struct LifecycleProgress { pub stage:LifecycleStage,pub content_month:Option<String>,pub affected:u32 }
}
impl HoldChange {
    pub fn validate(&self) -> Result<(), RemoteError> {
        if self.revision.0 == 0
            || self.reason.trim().is_empty()
            || self.reason.len() > 512
            || self
                .case_ref
                .as_ref()
                .is_some_and(|v| v.trim().is_empty() || v.len() > 256)
            || self
                .captured_from
                .as_ref()
                .zip(self.captured_to.as_ref())
                .is_some_and(|(a, b)| a.unix_millis() > b.unix_millis())
        {
            return Err(RemoteError::rejected(
                crate::ErrorCode::InvalidFrame,
                "invalid legal hold",
            ));
        }
        Ok(())
    }
}
fn history_page_limit() -> u16 {
    4
}
fn history_stream_limit() -> u16 {
    32
}
impl HistoryStreamsQuery {
    pub fn validate(&self) -> Result<(), RemoteError> {
        if !(1..=64).contains(&self.limit)
            || self.after.as_ref().is_some_and(|v| {
                v.is_empty()
                    || v.len() > 2048
                    || !v
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            })
        {
            return Err(RemoteError::rejected(
                crate::ErrorCode::InvalidFrame,
                "invalid history stream query",
            ));
        }
        Ok(())
    }
}
impl HistoryQuery {
    pub fn validate(&self) -> Result<(), RemoteError> {
        if !(1..=4).contains(&self.limit)
            || self.after.as_ref().is_some_and(|cursor| {
                cursor.is_empty()
                    || cursor.len() > 2048
                    || !cursor
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            })
        {
            return Err(RemoteError::rejected(
                crate::ErrorCode::InvalidFrame,
                "invalid history page query",
            ));
        }
        Ok(())
    }
}
impl Challenge {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, RemoteError> {
        let invalid = || {
            crate::RemoteError::rejected(crate::ErrorCode::InvalidFrame, "invalid challenge nonce")
        };
        if self.nonce.len() != 44 {
            return Err(invalid());
        }
        let nonce = STANDARD.decode(&self.nonce).map_err(|_| invalid())?;
        if STANDARD.encode(&nonce) != self.nonce {
            return Err(invalid());
        }
        MachineChallenge {
            origin: self.origin.clone(),
            machine: self.machine,
            key: self.key,
            challenge: self.challenge,
            nonce: nonce.try_into().map_err(|_| invalid())?,
            issued_at_ms: self.issued_at_ms.0,
            expires_at_ms: self.expires_at_ms.0,
        }
        .signing_bytes()
    }
}
/// A bearer credential intentionally returned to the host; Debug always redacts it.
#[derive(Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct MachineToken {
    pub token: String,
    pub expires_at: Timestamp,
}
impl std::fmt::Debug for MachineToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MachineToken")
            .field("token", &"[redacted]")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// A bearer credential for authenticated connection renewal. Debug cannot expose it.
#[derive(Clone, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct AccessToken(String);
impl TryFrom<String> for AccessToken {
    type Error = String;
    fn try_from(token: String) -> Result<Self, Self::Error> {
        if token.is_empty()
            || token.len() > 16384
            || token
                .bytes()
                .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
        {
            Err("invalid credential format".into())
        } else {
            Ok(Self(token))
        }
    }
}
impl From<AccessToken> for String {
    fn from(token: AccessToken) -> Self {
        token.0
    }
}
impl AccessToken {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl std::fmt::Debug for AccessToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AccessToken([redacted])")
    }
}
