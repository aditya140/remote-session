use crate::{Error, SecretDsn};
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use std::time::Duration;
pub const SCHEMA_MIN: i32 = 1;
pub const SCHEMA_MAX: i32 = 15;
const MIGRATIONS: &[(i32, &str)] = &[
    (1, include_str!("../migrations/0001_foundation.sql")),
    (2, include_str!("../migrations/0002_identity.sql")),
    (3, include_str!("../migrations/0003_directory_identity.sql")),
    (4, include_str!("../migrations/0004_routing.sql")),
    (
        5,
        include_str!("../migrations/0005_directory_freshness.sql"),
    ),
    (6, include_str!("../migrations/0006_revocations.sql")),
    (7, include_str!("../migrations/0007_native_access.sql")),
    (8, include_str!("../migrations/0008_controllers.sql")),
    (
        9,
        include_str!("../migrations/0009_recording_protection.sql"),
    ),
    (10, include_str!("../migrations/0010_recording_ingest.sql")),
    (11, include_str!("../migrations/0011_history_reads.sql")),
    (
        12,
        include_str!("../migrations/0012_recording_lifecycle.sql"),
    ),
    (
        13,
        include_str!("../migrations/0013_native_recording_end.sql"),
    ),
    (14, include_str!("../migrations/0014_protected_start.sql")),
    (15, include_str!("../migrations/0015_approvals.sql")),
];
#[derive(Clone)]
pub struct Store {
    pub(crate) pool: PgPool,
    commits: std::sync::Arc<crate::transaction::Commits>,
}
impl Store {
    /// Domain writers take the global lock before user/machine/key locks.
    pub(crate) async fn lock_access(transaction: &mut sqlx::PgConnection) -> Result<(), Error> {
        let disabled: bool = sqlx::query_scalar("SELECT disabled FROM qgr.access_state FOR SHARE")
            .fetch_one(&mut *transaction)
            .await
            .map_err(|_| Error::Database)?;
        if disabled {
            Err(Error::Forbidden)
        } else {
            Ok(())
        }
    }
    pub(crate) async fn pool(secret: &SecretDsn, size: u32) -> Result<PgPool, Error> {
        PgPoolOptions::new()
            .max_connections(size)
            .acquire_timeout(Duration::from_secs(3))
            .after_release(|connection,_| Box::pin(async move {
                if connection.cancellation_requested() {
                    // Consume the old response/ReadyForQuery before closing a PgBouncer
                    // frontend. Never let an unbounded cleanup wait retain this pool slot.
                    let _=tokio::time::timeout(Duration::from_secs(1),sqlx::Connection::ping(connection)).await;
                    return Err(sqlx::Error::Protocol("retired canceled authority connection".into()));
                }
                Ok(true)
            }))
            .after_connect(|connection,_| Box::pin(async move { sqlx::raw_sql("SET statement_timeout='5s'; SET lock_timeout='3s'; SET idle_in_transaction_session_timeout='10s'; SET client_connection_check_interval='100ms';").execute(connection).await?;Ok(()) }))
            .connect(&secret.0)
            .await
            .map_err(|_| Error::Database)
    }
    pub async fn connect(secret: &SecretDsn, size: u32, audit: bool) -> Result<Self, Error> {
        if size == 0 || size > 128 {
            return Err(Error::Configuration);
        }
        let pool = Self::pool(secret, size).await?;
        let role = if audit { "qgr_audit" } else { "qgr_app" };
        let safe:bool=sqlx::query_scalar("SELECT NOT r.rolsuper AND NOT r.rolcreatedb AND NOT r.rolcreaterole AND NOT r.rolbypassrls AND pg_has_role(current_user,$1,'MEMBER') AND NOT EXISTS (SELECT 1 FROM pg_roles privileged WHERE (privileged.rolsuper OR privileged.rolcreaterole OR privileged.rolbypassrls) AND pg_has_role(current_user,privileged.oid,'MEMBER')) AND NOT EXISTS (SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname='qgr' AND pg_has_role(current_user,c.relowner,'MEMBER')) FROM pg_roles r WHERE r.rolname=current_user").bind(role).fetch_one(&pool).await.map_err(|_| Error::Role)?;
        let mixed: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_roles r WHERE r.rolname=ANY($1) AND r.rolname<>$2 AND pg_has_role(current_user,r.oid,'MEMBER'))")
            .bind(vec!["qgr_app","qgr_audit","qgr_record_ingest","qgr_history_owner","qgr_history_admin","qgr_record_lifecycle"])
            .bind(role)
            .fetch_one(&pool)
            .await
            .map_err(|_| Error::Role)?;
        let privilege=sqlx::query("SELECT has_table_privilege(current_user,'qgr.audit_events','SELECT') AS reads,has_table_privilege(current_user,'qgr.audit_events','UPDATE,DELETE,TRUNCATE') OR has_column_privilege(current_user,'qgr.audit_events','event_id','INSERT') OR has_schema_privilege(current_user,'qgr','CREATE') AS rewrites,has_column_privilege(current_user,'qgr.audit_events','actor','INSERT') AS appends").fetch_one(&pool).await.map_err(|_| Error::Role)?;
        if !safe
            || mixed
            || privilege.get::<bool, _>("reads") != audit
            || privilege.get::<bool, _>("rewrites")
            || !privilege.get::<bool, _>("appends")
        {
            pool.close().await;
            return Err(Error::Role);
        }
        Ok(Self {
            pool,
            commits: std::sync::Arc::default(),
        })
    }
    pub async fn migrate(secret: &SecretDsn) -> Result<(), Error> {
        let pool = Self::pool(secret, 2).await?;
        let mut transaction = pool.begin().await.map_err(|_| Error::Database)?;
        sqlx::query("SELECT pg_advisory_xact_lock(5856751401703146816)")
            .execute(&mut *transaction)
            .await
            .map_err(|_| Error::Database)?;
        sqlx::raw_sql("CREATE SCHEMA IF NOT EXISTS qgr; CREATE TABLE IF NOT EXISTS qgr.schema_versions(version integer PRIMARY KEY,checksum text NOT NULL,applied_at timestamptz NOT NULL DEFAULT clock_timestamp());").execute(&mut *transaction).await.map_err(|_| Error::Database)?;
        let rows = sqlx::query("SELECT version,checksum FROM qgr.schema_versions ORDER BY version")
            .fetch_all(&mut *transaction)
            .await
            .map_err(|_| Error::Database)?;
        for row in &rows {
            let version: i32 = row.get("version");
            let checksum: String = row.get("checksum");
            let Some((_, sql)) = MIGRATIONS.iter().find(|(known, _)| *known == version) else {
                return Err(Error::Schema);
            };
            if checksum != format!("{:x}", Sha256::digest(sql.as_bytes())) {
                return Err(Error::Schema);
            }
        }
        for (version, sql) in MIGRATIONS {
            if rows
                .iter()
                .any(|row| row.get::<i32, _>("version") == *version)
            {
                continue;
            }
            sqlx::raw_sql(*sql)
                .execute(&mut *transaction)
                .await
                .map_err(|_| Error::Database)?;
            sqlx::query("INSERT INTO qgr.schema_versions(version,checksum) VALUES($1,$2)")
                .bind(version)
                .bind(format!("{:x}", Sha256::digest(sql.as_bytes())))
                .execute(&mut *transaction)
                .await
                .map_err(|_| Error::Database)?;
        }
        transaction.commit().await.map_err(|_| Error::Database)?;
        pool.close().await;
        Ok(())
    }
    pub async fn compatible(&self) -> Result<(), Error> {
        let version: Option<i32> =
            sqlx::query_scalar("SELECT max(version) FROM qgr.schema_versions")
                .fetch_one(&self.pool)
                .await
                .map_err(|_| Error::Database)?;
        if !version.is_some_and(|version| (SCHEMA_MIN..=SCHEMA_MAX).contains(&version)) {
            return Err(Error::Schema);
        }
        Ok(())
    }
    pub(crate) async fn transaction(
        &self,
    ) -> Result<crate::transaction::AuthorityTransaction, Error> {
        crate::transaction::begin(&self.pool, self.commits.clone()).await
    }
    pub async fn close(&self) {
        self.commits.close().await;
        self.pool.close().await;
    }
}
