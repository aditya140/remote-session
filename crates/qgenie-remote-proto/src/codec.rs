use crate::{ErrorCode, MAX_JSON_BYTES, RemoteError};
use serde::{
    Deserialize, Deserializer,
    de::{DeserializeOwned, Error, MapAccess, SeqAccess, Visitor},
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::fmt;

struct Unique(Value);
impl<'de> Deserialize<'de> for Unique {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct UniqueVisitor;
        impl<'de> Visitor<'de> for UniqueVisitor {
            type Value = Unique;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("JSON without duplicate object keys")
            }
            fn visit_bool<E: Error>(self, v: bool) -> Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_i64<E: Error>(self, v: i64) -> Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_u64<E: Error>(self, v: u64) -> Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_f64<E: Error>(self, v: f64) -> Result<Unique, E> {
                serde_json::Number::from_f64(v)
                    .map(|n| Unique(n.into()))
                    .ok_or_else(|| E::custom("invalid number"))
            }
            fn visit_str<E: Error>(self, v: &str) -> Result<Unique, E> {
                Ok(Unique(v.into()))
            }
            fn visit_none<E: Error>(self) -> Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_unit<E: Error>(self) -> Result<Unique, E> {
                Ok(Unique(Value::Null))
            }
            fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Unique, A::Error> {
                let mut values = Vec::new();
                while let Some(Unique(value)) = seq.next_element()? {
                    values.push(value);
                }
                Ok(Unique(values.into()))
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Unique, A::Error> {
                let mut values = serde_json::Map::new();
                while let Some(key) = map.next_key::<String>()? {
                    if values.contains_key(&key) {
                        return Err(A::Error::custom("duplicate JSON key"));
                    }
                    let Unique(value) = map.next_value()?;
                    values.insert(key, value);
                }
                Ok(Unique(values.into()))
            }
        }
        deserializer.deserialize_any(UniqueVisitor)
    }
}

pub fn decode_json<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, RemoteError> {
    decode_contract(bytes, MAX_JSON_BYTES, 64)
}
/// Internal containers may add bounded metadata around a public 1 MiB envelope.
/// Public network and IPC entry points must continue using `decode_json`.
pub fn decode_json_bounded<T: DeserializeOwned>(
    bytes: &[u8],
    limit: usize,
) -> Result<T, RemoteError> {
    decode_contract(bytes, limit, 72)
}
fn decode_contract<T: DeserializeOwned>(
    bytes: &[u8],
    limit: usize,
    max_depth: usize,
) -> Result<T, RemoteError> {
    if limit > 2 * MAX_JSON_BYTES || bytes.len() > limit {
        return Err(RemoteError::rejected(
            ErrorCode::FrameTooLarge,
            "JSON exceeds decoder byte limit",
        ));
    }
    let value = serde_json::from_slice::<Unique>(bytes)
        .map_err(|_| invalid())?
        .0;
    fn depth(value: &Value) -> usize {
        match value {
            Value::Array(values) => 1 + values.iter().map(depth).max().unwrap_or(0),
            Value::Object(values) => 1 + values.values().map(depth).max().unwrap_or(0),
            _ => 0,
        }
    }
    if depth(&value) > max_depth {
        return Err(invalid());
    }
    serde_json::from_value(value).map_err(|_| invalid())
}
fn invalid() -> RemoteError {
    RemoteError::rejected(ErrorCode::InvalidFrame, "invalid JSON contract")
}

pub fn fingerprint<T: serde::Serialize>(value: &T) -> Result<String, RemoteError> {
    // All current start fields are strings, arrays, and small integers. Sorted
    // serde_json maps give canonical key order for this restricted schema.
    let value = serde_json::to_value(value).map_err(|_| invalid())?;
    let bytes = serde_json::to_vec(&value).map_err(|_| invalid())?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_nested_duplicate_fields_and_deep_or_oversized_frames() {
        assert!(decode_json::<Value>(br#"{"a":{"x":1,"x":2}}"#).is_err());
        assert!(decode_json::<Value>(&vec![b' '; MAX_JSON_BYTES + 1]).is_err());
        let nested = format!("{}0{}", "[".repeat(65), "]".repeat(65));
        assert!(decode_json::<Value>(nested.as_bytes()).is_err());
    }
}
