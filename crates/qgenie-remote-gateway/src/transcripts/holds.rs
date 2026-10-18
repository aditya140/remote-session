use super::transaction::RecordingTransaction as Transaction;
use super::{
    RecordingDatabase, RecordingRole,
    ingest::{AccessWindow, db},
};
use crate::{
    AuditAction, AuditEvent, AuditMetadata, Error, IdentityService, Principal,
    RecordingLifecycleAudit, Store,
};
use qgenie_remote_proto::{
    rest::{HoldChange, HoldCommit},
    wire::Timestamp,
};
use sqlx::Row;

pub(super) fn datetime(at: &Timestamp) -> Result<time::OffsetDateTime, Error> {
    time::OffsetDateTime::from_unix_timestamp_nanos(i128::from(at.unix_millis()) * 1_000_000)
        .map_err(|_| Error::Invalid)
}
pub(super) fn current(
    identity: &IdentityService,
    actor: &Principal,
    access: &AccessWindow,
) -> Result<(), Error> {
    access.check()?;
    if let Some(directory) = &identity.directory {
        directory.cached(actor.user)?;
    }
    if crate::identity::jwt::unix_seconds()? >= actor.expires_at {
        return Err(Error::Unauthenticated);
    }
    Ok(())
}
impl RecordingDatabase {
    /// Only the isolated lifecycle credential and a verified platform administrator can
    /// place/change/release holds. Success fences retirement before bounded copy jobs finish.
    pub async fn change_hold(
        &self,
        identity: &IdentityService,
        actor: &Principal,
        change: HoldChange,
    ) -> Result<HoldCommit, Error> {
        self.hold_change(identity, actor, change, false).await
    }
    pub(super) async fn begin_lifecycle(
        &self,
        identity: &IdentityService,
        actor: &Principal,
        synthetic: bool,
    ) -> Result<(Transaction, AccessWindow), Error> {
        if self.role != RecordingRole::Lifecycle || !actor.platform_admin {
            return Err(Error::Forbidden);
        }
        if !synthetic && identity.directory.is_none() {
            return Err(Error::Configuration);
        }
        let version: Option<i32> =
            sqlx::query_scalar("SELECT max(version) FROM qgr.schema_versions")
                .fetch_one(&self.pool)
                .await
                .map_err(db)?;
        if !version
            .is_some_and(|v| (super::RECORDING_RELEASE_SCHEMA..=crate::SCHEMA_MAX).contains(&v))
        {
            return Err(Error::Schema);
        }
        let access = AccessWindow::user(actor.expires_at)?;
        identity.assert_admin(actor.user).await?;
        current(identity, actor, &access)?;
        let mut tx = self.transaction().await?;
        sqlx::raw_sql("SET LOCAL transaction_timeout='3s'; SET LOCAL statement_timeout='2s'; SET LOCAL lock_timeout='2s';").execute(&mut *tx).await.map_err(db)?;
        let scope = sqlx::query("SELECT * FROM qgr_recordings.lifecycle_scope($1,$2)")
            .bind(actor.user.to_string())
            .bind(i64::try_from(actor.expires_at).map_err(|_| Error::Unauthenticated)?)
            .fetch_optional(&mut *tx)
            .await
            .map_err(db)?
            .ok_or(Error::Unavailable)?;
        let mode: String = scope.get("durability");
        if !(if synthetic {
            mode == "synthetic_single_server"
        } else {
            mode == "synchronous_standby" && scope.get::<bool, _>("synchronous_ready")
        }) {
            return Err(Error::Unavailable);
        }
        sqlx::query("SELECT set_config('synchronous_commit',$1,true)")
            .bind(if synthetic { "on" } else { "remote_apply" })
            .execute(&mut *tx)
            .await
            .map_err(db)?;
        current(identity, actor, &access)?;
        Ok((tx, access))
    }
    pub(super) async fn hold_change(
        &self,
        identity: &IdentityService,
        actor: &Principal,
        mut change: HoldChange,
        synthetic: bool,
    ) -> Result<HoldCommit, Error> {
        change.validate().map_err(|_| Error::Invalid)?;
        change.reason = change.reason.trim().to_owned();
        change.case_ref = change.case_ref.map(|v| v.trim().to_owned());
        let (mut tx, access) = self.begin_lifecycle(identity, actor, synthetic).await?;
        let changed: bool = sqlx::query_scalar(
            "SELECT qgr_recordings.change_hold($1,$2,$3::numeric,$4,$5,$6,$7,$8)",
        )
        .bind(change.id.to_string())
        .bind(change.user.to_string())
        .bind(change.revision.0.to_string())
        .bind(change.active)
        .bind(change.captured_from.as_ref().map(datetime).transpose()?)
        .bind(change.captured_to.as_ref().map(datetime).transpose()?)
        .bind(&change.case_ref)
        .bind(&change.reason)
        .fetch_one(&mut *tx)
        .await
        .map_err(db)?;
        if changed {
            Store::append_audit(
                &mut tx,
                &AuditEvent {
                    actor: actor.user,
                    action: AuditAction::HoldChanged,
                    machine: None,
                    session: None,
                    metadata: AuditMetadata {
                        subject: Some(change.user),
                        recording_lifecycle: Some(RecordingLifecycleAudit {
                            reason: change.reason.clone(),
                            hold: Some(change.clone()),
                            progress: None,
                        }),
                        ..Default::default()
                    },
                },
            )
            .await?;
        }
        current(identity, actor, &access)?;
        tx.commit().await?;
        current(identity, actor, &access)?;
        Ok(HoldCommit {
            id: change.id,
            revision: change.revision,
            active: change.active,
        })
    }
}

#[cfg(test)]
pub(crate) mod tests;
