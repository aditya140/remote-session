use super::transaction::RecordingTransaction as Transaction;
use super::{RecordingDatabase, RecordingRole, holds::current, ingest::db};
use crate::{
    AuditAction, AuditEvent, AuditMetadata, Error, IdentityService, Principal,
    RecordingLifecycleAudit, Store,
};
use qgenie_remote_proto::{
    UserId,
    rest::{LifecycleProgress, LifecycleStage},
};
use sqlx::Row;

impl RecordingDatabase {
    /// One bounded, restartable unit. Callers own scheduling/cancellation and stop when idle.
    /// Each transaction releases the global lifecycle lock before the next unit starts.
    pub async fn retention_step(
        &self,
        identity: &IdentityService,
        actor: &Principal,
        reason: &str,
        batch_size: u16,
    ) -> Result<LifecycleProgress, Error> {
        self.maintenance_step(identity, actor, reason, batch_size, false)
            .await
    }
    pub(super) async fn maintenance_step(
        &self,
        identity: &IdentityService,
        actor: &Principal,
        reason: &str,
        batch_size: u16,
        synthetic: bool,
    ) -> Result<LifecycleProgress, Error> {
        let reason = reason.trim();
        if reason.is_empty() || reason.len() > 512 || !(1..=32).contains(&batch_size) {
            return Err(Error::Invalid);
        }
        let (mut tx, access) = self.begin_lifecycle(identity, actor, synthetic).await?;
        let progress = apply_step(&mut tx, actor.user, reason, batch_size).await?;
        current(identity, actor, &access)?;
        tx.commit().await?;
        current(identity, actor, &access)?;
        Ok(progress)
    }
    /// One scheduled maintenance unit under the isolated lifecycle service credential.
    /// This performs fixed retention/hold backfill; it never creates or changes a hold.
    pub async fn scheduled_retention_step(
        &self,
        reason: &str,
        batch_size: u16,
    ) -> Result<LifecycleProgress, Error> {
        self.scheduled_step(reason, batch_size, false).await
    }
    pub(super) async fn scheduled_step(
        &self,
        reason: &str,
        batch_size: u16,
        synthetic: bool,
    ) -> Result<LifecycleProgress, Error> {
        let reason = reason.trim();
        if self.role != RecordingRole::Lifecycle {
            return Err(Error::Role);
        }
        if reason.is_empty() || reason.len() > 512 || !(1..=32).contains(&batch_size) {
            return Err(Error::Invalid);
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
        let mut tx = self.transaction().await?;
        sqlx::raw_sql("SET LOCAL transaction_timeout='3s'; SET LOCAL statement_timeout='2s'; SET LOCAL lock_timeout='2s';").execute(&mut *tx).await.map_err(db)?;
        let scope = sqlx::query("SELECT * FROM qgr_recordings.maintenance_scope()")
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
        let actor: UserId = scope
            .get::<String, _>("actor_id")
            .parse()
            .map_err(|_| Error::Database)?;
        let progress = apply_step(&mut tx, actor, reason, batch_size).await?;
        tx.commit().await?;
        Ok(progress)
    }
}

async fn apply_step(
    tx: &mut Transaction,
    actor: UserId,
    reason: &str,
    batch_size: u16,
) -> Result<LifecycleProgress, Error> {
    let row = sqlx::query("SELECT * FROM qgr_recordings.lifecycle_step($1)")
        .bind(i32::from(batch_size))
        .fetch_one(&mut **tx)
        .await
        .map_err(db)?;
    let progress = LifecycleProgress {
        stage: serde_json::from_value(serde_json::Value::String(row.get("stage")))
            .map_err(|_| Error::Database)?,
        content_month: row
            .get::<Option<time::Date>, _>("content_month")
            .map(|v| v.to_string()),
        affected: u32::try_from(row.get::<i32, _>("affected")).map_err(|_| Error::Database)?,
    };
    if progress.stage != LifecycleStage::Idle {
        let action = match progress.stage {
            LifecycleStage::Retired
            | LifecycleStage::ExpiredGaps
            | LifecycleStage::ExpiredHeldContent
            | LifecycleStage::ExpiredHeldGaps => AuditAction::RetentionDeleted,
            _ => AuditAction::RecordingMaintenance,
        };
        Store::append_audit(
            tx,
            &AuditEvent {
                actor,
                action,
                machine: None,
                session: None,
                metadata: AuditMetadata {
                    recording_lifecycle: Some(RecordingLifecycleAudit {
                        reason: reason.into(),
                        hold: None,
                        progress: Some(progress.clone()),
                    }),
                    ..Default::default()
                },
            },
        )
        .await?;
    }
    Ok(progress)
}
