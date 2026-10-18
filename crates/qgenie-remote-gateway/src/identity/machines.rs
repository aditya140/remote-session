use super::{IdentityService, db, now_ms};
use crate::{AuditAction, AuditEvent, AuditMetadata, Error, Store};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::VerifyingKey;
use qgenie_remote_proto::{
    MachineId, UserId,
    rest::{EnrollmentRequest, MachinePatch},
    wire::{Machine, Timestamp},
};
use sqlx::{Row, postgres::PgRow};
use std::collections::BTreeMap;
pub(super) fn public_key(request: &EnrollmentRequest) -> Result<Vec<u8>, Error> {
    if request.public_key.len() != 44 {
        return Err(Error::Invalid);
    }
    let bytes = STANDARD
        .decode(&request.public_key)
        .map_err(|_| Error::Invalid)?;
    if bytes.len() != 32
        || STANDARD.encode(&bytes) != request.public_key
        || VerifyingKey::from_bytes(bytes.as_slice().try_into().map_err(|_| Error::Invalid)?)
            .map_err(|_| Error::Invalid)?
            .is_weak()
    {
        return Err(Error::Invalid);
    }
    Ok(bytes)
}
fn label(value: &str, min: usize, max: usize) -> bool {
    value.len() >= min && value.len() <= max && !value.chars().any(char::is_control)
}
pub(super) fn labels(labels: &BTreeMap<String, String>) -> bool {
    labels.len() <= 16
        && labels
            .iter()
            .all(|(key, value)| label(key, 1, 64) && label(value, 0, 128))
}
pub(super) fn validate(request: &EnrollmentRequest) -> Result<(), Error> {
    public_key(request)?;
    if !label(&request.hostname, 1, 253)
        || !label(&request.name, 1, 128)
        || !label(&request.os_user, 1, 128)
        || !label(&request.os, 1, 32)
        || !label(&request.arch, 1, 32)
        || !label(&request.cli_version, 1, 64)
        || !labels(&request.labels)
        || request
            .recovery
            .as_ref()
            .is_some_and(|recovery| !recovery.confirmed)
    {
        return Err(Error::Invalid);
    }
    Ok(())
}
impl IdentityService {
    pub async fn machine(&self, owner: UserId, id: MachineId) -> Result<Machine, Error> {
        self.assert_user(owner).await?;
        let row =
            sqlx::query("SELECT * FROM qgr.machines WHERE id=$1 AND owner=$2 AND NOT disabled")
                .bind(id.to_string())
                .bind(owner.to_string())
                .fetch_optional(&self.store.pool)
                .await
                .map_err(db)?
                .ok_or(Error::NotFound)?;
        machine(&row)
    }
    pub async fn revoke(&self, owner: UserId, id: MachineId) -> Result<(), Error> {
        self.assert_directory(owner).await?;
        let mut transaction = self.store.transaction().await?;
        assert_user(&mut transaction, owner).await?;
        let disabled: bool = sqlx::query_scalar(
            "SELECT disabled FROM qgr.machines WHERE id=$1 AND owner=$2 FOR UPDATE",
        )
        .bind(id.to_string())
        .bind(owner.to_string())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(db)?
        .ok_or(Error::NotFound)?;
        if !disabled {
            sqlx::query("UPDATE qgr.machines SET disabled=true WHERE id=$1")
                .bind(id.to_string())
                .execute(&mut *transaction)
                .await
                .map_err(db)?;
            sqlx::query("UPDATE qgr.machine_keys SET revoked=true WHERE machine=$1")
                .bind(id.to_string())
                .execute(&mut *transaction)
                .await
                .map_err(db)?;
            Store::append_audit(
                &mut transaction,
                &AuditEvent {
                    actor: owner,
                    action: AuditAction::MachineRevoked,
                    machine: Some(id),
                    session: None,
                    metadata: AuditMetadata::default(),
                },
            )
            .await?;
        }
        transaction.commit().await?;
        Ok(())
    }
    pub async fn assert_user(&self, owner: UserId) -> Result<(), Error> {
        self.assert_directory(owner).await?;
        let allowed:bool=sqlx::query_scalar("SELECT NOT (u.disabled OR a.disabled) FROM qgr.users u CROSS JOIN qgr.access_state a WHERE u.id=$1").bind(owner.to_string()).fetch_optional(&self.store.pool).await.map_err(db)?.ok_or(Error::Forbidden)?;
        if allowed {
            Ok(())
        } else {
            Err(Error::Forbidden)
        }
    }
    pub async fn machines(&self, owner: UserId) -> Result<Vec<Machine>, Error> {
        self.assert_user(owner).await?;
        let rows = sqlx::query(
            "SELECT * FROM qgr.machines WHERE owner=$1 AND NOT disabled ORDER BY id LIMIT 1025",
        )
        .bind(owner.to_string())
        .fetch_all(&self.store.pool)
        .await
        .map_err(db)?;
        if rows.len() > 1024 {
            return Err(Error::Overloaded);
        }
        rows.into_iter().map(|row| machine(&row)).collect()
    }
    pub async fn patch(
        &self,
        owner: UserId,
        id: MachineId,
        patch: MachinePatch,
    ) -> Result<Machine, Error> {
        self.assert_directory(owner).await?;
        if patch.name.as_ref().is_some_and(|name| !label(name, 1, 128))
            || patch.labels.as_ref().is_some_and(|value| !labels(value))
        {
            return Err(Error::Invalid);
        }
        let mut transaction = self.store.transaction().await?;
        assert_user(&mut transaction, owner).await?;
        let row = sqlx::query(
            "SELECT * FROM qgr.machines WHERE id=$1 AND owner=$2 AND NOT disabled FOR UPDATE",
        )
        .bind(id.to_string())
        .bind(owner.to_string())
        .fetch_optional(&mut *transaction)
        .await
        .map_err(db)?
        .ok_or(Error::NotFound)?;
        let previous = machine(&row)?;
        let row = sqlx::query(
            "UPDATE qgr.machines SET name=$3,labels=$4 WHERE id=$1 AND owner=$2 RETURNING *",
        )
        .bind(id.to_string())
        .bind(owner.to_string())
        .bind(patch.name.unwrap_or(previous.name))
        .bind(
            serde_json::to_value(patch.labels.unwrap_or(previous.labels))
                .map_err(|_| Error::Invalid)?,
        )
        .fetch_one(&mut *transaction)
        .await
        .map_err(db)?;
        Store::append_audit(
            &mut transaction,
            &AuditEvent {
                actor: owner,
                action: AuditAction::MachineMetadataChanged,
                machine: Some(id),
                session: None,
                metadata: AuditMetadata::default(),
            },
        )
        .await?;
        transaction.commit().await?;
        machine(&row)
    }
}
pub(super) async fn assert_user(
    transaction: &mut sqlx::PgConnection,
    owner: UserId,
) -> Result<(), Error> {
    Store::lock_access(transaction).await?;
    let allowed:bool=sqlx::query_scalar("SELECT NOT(u.disabled OR a.disabled) FROM qgr.users u CROSS JOIN qgr.access_state a WHERE u.id=$1 FOR SHARE OF u,a").bind(owner.to_string()).fetch_optional(&mut *transaction).await.map_err(db)?.ok_or(Error::Forbidden)?;
    if allowed {
        Ok(())
    } else {
        Err(Error::Forbidden)
    }
}
fn machine(row: &PgRow) -> Result<Machine, Error> {
    let last_seen: time::OffsetDateTime = row.get("last_seen");
    Ok(Machine {
        id: row
            .get::<String, _>("id")
            .parse()
            .map_err(|_| Error::Database)?,
        owner: row
            .get::<String, _>("owner")
            .parse()
            .map_err(|_| Error::Database)?,
        name: row.get("name"),
        hostname: row.get("hostname"),
        os: row.get("os"),
        arch: row.get("arch"),
        cli_version: row.get("cli_version"),
        labels: serde_json::from_value(row.get("labels")).map_err(|_| Error::Database)?,
        agents: BTreeMap::new(),
        online: false,
        last_seen: Timestamp::from_unix_millis(
            (last_seen.unix_timestamp_nanos() / 1_000_000)
                .try_into()
                .map_err(|_| Error::Database)?,
        )
        .map_err(|_| Error::Database)?,
        sessions: 0,
    })
}
pub(super) async fn cleanup(transaction: &mut sqlx::PgConnection) -> Result<(), Error> {
    // Bounded lazy expiry cleanup. Full periodic housekeeping is registered by jobs later.
    sqlx::query("DELETE FROM qgr.machine_challenges WHERE id IN (SELECT id FROM qgr.machine_challenges WHERE expires_at_ms<$1 ORDER BY expires_at_ms LIMIT 128)").bind(now_ms()? as i64).execute(&mut *transaction).await.map_err(db)?;
    Ok(())
}
