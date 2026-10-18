use super::jwt::{unique_token, unix_seconds};
use crate::{Error, Store};
use base64::{Engine, engine::general_purpose::STANDARD};
use jsonwebtoken::{
    Algorithm, DecodingKey, EncodingKey, Header, Validation, decode, decode_header, encode,
};
use qgenie_remote_proto::{
    ChallengeId, KeyId, MachineChallenge, MachineId, UserId, rest::MachineToken, wire::Timestamp,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::path::Path;
#[derive(Debug, Clone)]
pub struct MachinePrincipal {
    pub machine: MachineId,
    pub owner: UserId,
    pub key: KeyId,
    pub expires_at: u64,
}
#[derive(Clone, Serialize, Deserialize)]
struct Claims {
    iss: String,
    aud: String,
    sub: MachineId,
    owner: UserId,
    key: KeyId,
    iat: u64,
    nbf: u64,
    exp: u64,
}
pub(super) struct MachineTokens {
    pub origin: String,
    encoding: EncodingKey,
    decoding: DecodingKey,
}
impl MachineTokens {
    pub fn from_file(origin: String, path: &Path) -> Result<Self, Error> {
        let metadata = std::fs::symlink_metadata(path).map_err(|_| Error::Configuration)?;
        if !metadata.is_file() || metadata.len() > 256 {
            return Err(Error::Configuration);
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if metadata.mode() & 0o007 != 0 {
                return Err(Error::Configuration);
            }
        }
        use std::io::Read;
        let mut encoded = String::new();
        std::fs::File::open(path)
            .map_err(|_| Error::Configuration)?
            .take(257)
            .read_to_string(&mut encoded)
            .map_err(|_| Error::Configuration)?;
        if encoded.len() > 256 {
            return Err(Error::Configuration);
        }
        let key = STANDARD
            .decode(encoded.trim())
            .map_err(|_| Error::Configuration)?;
        Self::new(origin, &key)
    }
    pub(super) fn new(origin: String, key: &[u8]) -> Result<Self, Error> {
        if key.len() != 32 {
            return Err(Error::Configuration);
        }
        MachineChallenge {
            origin: origin.clone(),
            machine: MachineId::new(),
            key: KeyId::new(),
            challenge: ChallengeId::new(),
            nonce: [0; 32],
            issued_at_ms: 1,
            expires_at_ms: 2,
        }
        .signing_bytes()
        .map_err(|_| Error::Configuration)?;
        Ok(Self {
            origin,
            encoding: EncodingKey::from_secret(key),
            decoding: DecodingKey::from_secret(key),
        })
    }
    pub(super) fn issue(
        &self,
        machine: MachineId,
        owner: UserId,
        key: KeyId,
    ) -> Result<MachineToken, Error> {
        let now = unix_seconds()?;
        let expires = now.checked_add(900).ok_or(Error::Database)?;
        let claims = Claims {
            iss: format!("{}/machine", self.origin),
            aud: "qgenie-remote-machine".into(),
            sub: machine,
            owner,
            key,
            iat: now,
            nbf: now,
            exp: expires,
        };
        let mut header = Header::new(Algorithm::HS256);
        header.typ = Some("qgr-machine+jwt".into());
        header.kid = Some("machine-v1".into());
        Ok(MachineToken {
            token: encode(&header, &claims, &self.encoding).map_err(|_| Error::Configuration)?,
            expires_at: Timestamp::from_unix_millis(expires * 1000).map_err(|_| Error::Database)?,
        })
    }
    pub(super) async fn verify(
        &self,
        token: &str,
        store: &Store,
    ) -> Result<MachinePrincipal, Error> {
        unique_token(token)?;
        let header = decode_header(token).map_err(|_| Error::Unauthenticated)?;
        if header.alg != Algorithm::HS256
            || header.typ.as_deref() != Some("qgr-machine+jwt")
            || header.kid.as_deref() != Some("machine-v1")
        {
            return Err(Error::Unauthenticated);
        }
        let mut validation = Validation::new(Algorithm::HS256);
        validation.set_issuer(&[format!("{}/machine", self.origin)]);
        validation.set_audience(&["qgenie-remote-machine"]);
        validation.set_required_spec_claims(&["iss", "aud", "sub", "exp", "iat", "nbf"]);
        validation.validate_nbf = true;
        validation.leeway = 0;
        let claims = decode::<Claims>(token, &self.decoding, &validation)
            .map_err(|_| Error::Unauthenticated)?
            .claims;
        if claims.exp <= claims.iat || claims.exp - claims.iat > 900 || claims.iat > unix_seconds()?
        {
            return Err(Error::Unauthenticated);
        }
        let row=sqlx::query("SELECT m.owner,m.active_key,m.disabled OR u.disabled OR a.disabled OR k.revoked AS denied FROM qgr.machines m JOIN qgr.users u ON u.id=m.owner JOIN qgr.machine_keys k ON k.id=m.active_key AND k.machine=m.id CROSS JOIN qgr.access_state a WHERE u.directory_object IS NOT NULL AND m.id=$1").bind(claims.sub.to_string()).fetch_optional(&store.pool).await.map_err(|_| Error::Database)?.ok_or(Error::Unauthenticated)?;
        if row.get::<bool, _>("denied")
            || row.get::<String, _>("owner") != claims.owner.to_string()
            || row.get::<String, _>("active_key") != claims.key.to_string()
        {
            return Err(Error::Forbidden);
        }
        Ok(MachinePrincipal {
            machine: claims.sub,
            owner: claims.owner,
            key: claims.key,
            expires_at: claims.exp,
        })
    }
}
