use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error};
use std::{fmt, str::FromStr};
use uuid::Uuid;

macro_rules! id {
    ($name:ident, $prefix:literal) => {
        #[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        #[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
        pub struct $name(#[cfg_attr(feature = "schema", schemars(with = "String"))] Uuid);

        impl $name {
            pub fn new() -> Self {
                Self(Uuid::new_v4())
            }
            pub fn as_bytes(&self) -> &[u8; 16] {
                self.0.as_bytes()
            }
            pub fn from_bytes(bytes: [u8; 16]) -> Self {
                Self(Uuid::from_bytes(bytes))
            }
        }
        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}{}", $prefix, self.0.hyphenated())
            }
        }
        impl FromStr for $name {
            type Err = String;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                let raw = s
                    .strip_prefix($prefix)
                    .ok_or_else(|| format!("expected {} UUID", $prefix))?;
                let id = Uuid::parse_str(raw).map_err(|_| "invalid UUID".to_owned())?;
                if raw != id.hyphenated().to_string() {
                    return Err("UUID must be canonical lowercase".into());
                }
                Ok(Self(id))
            }
        }
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                String::deserialize(deserializer)?
                    .parse()
                    .map_err(D::Error::custom)
            }
        }
    };
}
id!(MachineId, "m_");
id!(InstallationId, "install_");
id!(SessionId, "s_");
id!(RequestId, "req_");
id!(ClientId, "client_");
id!(IdempotencyKey, "idem_");
id!(ApprovalId, "a_");
id!(DecisionId, "d_");
id!(ChunkId, "chunk_");
id!(ReceiptId, "receipt_");
id!(KeyId, "key_");
id!(ChallengeId, "challenge_");
id!(MessageId, "msg_");
id!(UserId, "u_");
id!(GapId, "gap_");
id!(RecordingEpochId, "recording_");
id!(HoldId, "hold_");

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
pub struct Counter(#[cfg_attr(feature = "schema", schemars(with = "String"))] pub u64);

impl Serialize for Counter {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0)
    }
}
impl<'de> Deserialize<'de> for Counter {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        if text.is_empty()
            || (text.len() > 1 && text.starts_with('0'))
            || !text.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(D::Error::custom(
                "expected canonical unsigned decimal string",
            ));
        }
        text.parse().map(Self).map_err(D::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn counters_preserve_full_u64_and_reject_ambiguous_values() {
        for value in [0, 9_007_199_254_740_993, u64::MAX] {
            let json = serde_json::to_string(&Counter(value)).unwrap();
            assert_eq!(serde_json::from_str::<Counter>(&json).unwrap().0, value);
        }
        for invalid in [
            "1",
            "\"01\"",
            "\"-1\"",
            "\"+1\"",
            "\"1e2\"",
            "\"18446744073709551616\"",
        ] {
            assert!(
                serde_json::from_str::<Counter>(invalid).is_err(),
                "{invalid}"
            );
        }
    }
    #[test]
    fn identities_are_canonical_and_not_interchangeable() {
        let id = SessionId::new();
        assert_eq!(id.to_string().parse::<SessionId>().unwrap(), id);
        assert!(id.to_string().parse::<MachineId>().is_err());
        assert!(
            "s_550E8400-E29B-41D4-A716-446655440000"
                .parse::<SessionId>()
                .is_err()
        );
    }
}
