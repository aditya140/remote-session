//! Admission uses the isolated ingest identity, before any native start is sent.
use super::{
    RecordingDatabase, RecordingRole,
    ingest::{AccessWindow, db},
};
use crate::{Error, IdentityService};
use qgenie_remote_proto::{MachineId, UserId};
use sqlx::Row;

impl RecordingDatabase {
    pub(crate) async fn preflight_start(
        &self,
        identity: &IdentityService,
        owner: UserId,
        machine: MachineId,
        expires: u64,
    ) -> Result<(), Error> {
        if self.role != RecordingRole::Ingest {
            return Err(Error::Role);
        }
        let directory = identity.directory.as_ref().ok_or(Error::Configuration)?;
        let access = AccessWindow::user(expires)?;
        identity.assert_user(owner).await?;
        access.check()?;
        let mut tx = self.transaction().await?;
        sqlx::raw_sql("SET LOCAL transaction_timeout='3s'; SET LOCAL statement_timeout='2s'; SET LOCAL lock_timeout='2s'; SET LOCAL synchronous_commit='remote_apply';")
            .execute(&mut *tx).await.map_err(db)?;
        let version: Option<i32> =
            sqlx::query_scalar("SELECT max(version) FROM qgr.schema_versions")
                .fetch_one(&mut *tx)
                .await
                .map_err(db)?;
        if !version.is_some_and(|v| (14..=crate::SCHEMA_MAX).contains(&v)) {
            return Err(Error::Schema);
        }
        let row = sqlx::query("SELECT * FROM qgr_recordings.preflight_start($1,$2,$3)")
            .bind(machine.to_string())
            .bind(owner.to_string())
            .bind(i64::try_from(expires).map_err(|_| Error::Unauthenticated)?)
            .fetch_one(&mut *tx)
            .await
            .map_err(db)?;
        if row.get::<String, _>("durability") != "synchronous_standby"
            || !row.get::<bool, _>("synchronous_ready")
            || !row.get::<bool, _>("lifecycle_ready")
            || row.get::<bool, _>("recovery_pending")
        {
            return Err(Error::Unavailable);
        }
        access.check()?;
        directory.cached(owner)?;
        tx.commit().await?;
        access.check()?;
        directory.cached(owner)?;
        Ok(())
    }
}
