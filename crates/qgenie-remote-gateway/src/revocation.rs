//! Durable manual revocation. Independent socket watches enforce denial without
//! Redis hints. Native activation will bind every dispatch to bounded grants.
use crate::{AuditAction, AuditEvent, AuditMetadata, Error, Gateway, Store};
use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::HeaderMap,
};
use qgenie_remote_proto::{
    Counter, RequestId, UserId, decode_json,
    rest::{AccessRevocation, RevocationPropagation, RevocationTarget, RevokeAccess},
    wire::Timestamp,
};
use sqlx::{Row, postgres::PgRow};
mod propagation;
pub(crate) use propagation::Propagation;

pub(crate) async fn commit(
    State(gateway): State<Gateway>,
    headers: HeaderMap,
    bytes: Bytes,
) -> Result<Json<AccessRevocation>, Error> {
    let actor = gateway.principal(&headers).await?;
    if !actor.platform_admin {
        return Err(Error::Forbidden);
    }
    gateway.ready().await?;
    let request: RevokeAccess = decode_json(&bytes).map_err(|_| Error::Invalid)?;
    if !request.confirmed {
        return Err(Error::Invalid);
    }
    let mut result = gateway.store.revoke_access(actor.user, request).await?;
    if let Some(routing) = &gateway.routing {
        routing.fence(&result.target);
    }
    gateway
        .revocation_observations
        .observe(actor.user, &mut result)?;
    Ok(Json(result))
}
pub(crate) async fn status(
    State(gateway): State<Gateway>,
    headers: HeaderMap,
    Path(request): Path<String>,
) -> Result<Json<AccessRevocation>, Error> {
    let actor = gateway.principal(&headers).await?;
    if !actor.platform_admin {
        return Err(Error::Forbidden);
    }
    let request: RequestId = request.parse().map_err(|_| Error::Invalid)?;
    let row = sqlx::query("SELECT target,revision::text,committed_at FROM qgr.access_revocations WHERE actor=$1 AND request=$2")
        .bind(actor.user.to_string()).bind(request.to_string()).fetch_optional(&gateway.store.pool).await.map_err(|_| Error::Database)?.ok_or(Error::NotFound)?;
    let mut result = result(request, row)?;
    gateway
        .revocation_observations
        .observe(actor.user, &mut result)?;
    Ok(Json(result))
}
impl Store {
    async fn revoke_access(
        &self,
        actor: UserId,
        request: RevokeAccess,
    ) -> Result<AccessRevocation, Error> {
        let mut tx = self.transaction().await?;
        // Global lock serializes idempotency/revisions before lower-order locks.
        let revision: String =
            sqlx::query_scalar("SELECT revision::text FROM qgr.access_state FOR UPDATE")
                .fetch_one(&mut *tx)
                .await
                .map_err(|_| Error::Database)?;
        let enabled: bool =
            sqlx::query_scalar("SELECT NOT disabled FROM qgr.users WHERE id=$1 FOR SHARE")
                .bind(actor.to_string())
                .fetch_optional(&mut *tx)
                .await
                .map_err(|_| Error::Database)?
                .unwrap_or(false);
        if !enabled {
            return Err(Error::Forbidden);
        }
        if let Some(row) = sqlx::query("SELECT target,revision::text,committed_at FROM qgr.access_revocations WHERE actor=$1 AND request=$2")
            .bind(actor.to_string()).bind(request.request.to_string()).fetch_optional(&mut *tx).await.map_err(|_| Error::Database)? {
            let result = result(request.request, row)?;
            if result.target != request.target { return Err(Error::Conflict); }
            return Ok(result);
        }
        let revision = Counter(
            revision
                .parse::<u64>()
                .map_err(|_| Error::Database)?
                .checked_add(1)
                .ok_or(Error::Overloaded)?,
        );
        let (action, user, machine) = match &request.target {
            RevocationTarget::Global => {
                sqlx::query("UPDATE qgr.access_state SET disabled=true")
                    .execute(&mut *tx)
                    .await
                    .map_err(|_| Error::Database)?;
                (AuditAction::GlobalRevoked, None, None)
            }
            RevocationTarget::User { user } => {
                let changed = sqlx::query("UPDATE qgr.users SET disabled=true WHERE id=$1")
                    .bind(user.to_string())
                    .execute(&mut *tx)
                    .await
                    .map_err(|_| Error::Database)?;
                if changed.rows_affected() != 1 {
                    return Err(Error::NotFound);
                }
                (AuditAction::UserRevoked, Some(*user), None)
            }
            RevocationTarget::Machine { machine } => {
                let changed = sqlx::query("UPDATE qgr.machines SET disabled=true WHERE id=$1")
                    .bind(machine.to_string())
                    .execute(&mut *tx)
                    .await
                    .map_err(|_| Error::Database)?;
                if changed.rows_affected() != 1 {
                    return Err(Error::NotFound);
                }
                sqlx::query("UPDATE qgr.machine_keys SET revoked=true WHERE machine=$1")
                    .bind(machine.to_string())
                    .execute(&mut *tx)
                    .await
                    .map_err(|_| Error::Database)?;
                (AuditAction::MachineRevoked, None, Some(*machine))
            }
        };
        sqlx::query("UPDATE qgr.access_state SET revision=$1::numeric")
            .bind(revision.0.to_string())
            .execute(&mut *tx)
            .await
            .map_err(|_| Error::Database)?;
        let target = serde_json::to_value(&request.target).map_err(|_| Error::Invalid)?;
        let row = sqlx::query("INSERT INTO qgr.access_revocations(actor,request,target,revision) VALUES($1,$2,$3,$4::numeric) RETURNING target,revision::text,committed_at")
            .bind(actor.to_string()).bind(request.request.to_string()).bind(target).bind(revision.0.to_string()).fetch_one(&mut *tx).await.map_err(|_| Error::Database)?;
        Self::append_audit(
            &mut tx,
            &AuditEvent {
                actor,
                action,
                machine,
                session: None,
                metadata: AuditMetadata {
                    request: Some(request.request),
                    subject: user,
                    generation: Some(revision),
                    ..AuditMetadata::default()
                },
            },
        )
        .await?;
        let result = result(request.request, row)?;
        tx.commit().await?;
        Ok(result)
    }
}
fn result(request: RequestId, row: PgRow) -> Result<AccessRevocation, Error> {
    let at: time::OffsetDateTime = row.get("committed_at");
    Ok(AccessRevocation {
        request,
        target: serde_json::from_value(row.get("target")).map_err(|_| Error::Database)?,
        revision: Counter(
            row.get::<String, _>("revision")
                .parse()
                .map_err(|_| Error::Database)?,
        ),
        committed_at: Timestamp::from_unix_millis(
            (at.unix_timestamp_nanos() / 1_000_000)
                .try_into()
                .map_err(|_| Error::Database)?,
        )
        .map_err(|_| Error::Database)?,
        // The HTTP plane may advance this only after a post-commit observation's
        // full original-authority horizon. SQL/UTC age alone never proves it.
        propagation: RevocationPropagation::Pending,
    })
}

#[cfg(test)]
pub(crate) mod tests;
