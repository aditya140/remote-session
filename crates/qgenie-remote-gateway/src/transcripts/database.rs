use crate::{Error, SecretDsn, Store};
use sqlx::{PgPool, Row};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingRole {
    Ingest,
    OwnerRead,
    AdminRead,
    Lifecycle,
}
impl RecordingRole {
    fn name(self) -> &'static str {
        match self {
            Self::Ingest => "qgr_record_ingest",
            Self::OwnerRead => "qgr_history_owner",
            Self::AdminRead => "qgr_history_admin",
            Self::Lifecycle => "qgr_record_lifecycle",
        }
    }
}
/// Isolated recording credential/pool. It never shares the authority/audit service identity.
#[derive(Clone)]
pub struct RecordingDatabase {
    pub(super) pool: PgPool,
    pub(super) role: RecordingRole,
    pub(super) commits: std::sync::Arc<super::transaction::Commits>,
}
impl RecordingDatabase {
    pub async fn connect(
        secret: &SecretDsn,
        role: RecordingRole,
        size: u32,
    ) -> Result<Self, Error> {
        if size == 0 || size > 32 {
            return Err(Error::Configuration);
        }
        let pool = Store::pool(secret, size).await?;
        let check=async {
            let version:Option<i32>=sqlx::query_scalar("SELECT max(version) FROM qgr.schema_versions").fetch_one(&pool).await.map_err(|_|Error::Schema)?;
            if !version.is_some_and(|v|(9..=crate::SCHEMA_MAX).contains(&v)) {return Err(Error::Schema);}
            let safe:bool=sqlx::query_scalar("SELECT NOT r.rolsuper AND NOT r.rolcreatedb AND NOT r.rolcreaterole AND NOT r.rolreplication AND NOT r.rolbypassrls AND pg_has_role(current_user,$1,'MEMBER') AND NOT EXISTS(SELECT 1 FROM pg_roles p WHERE (p.rolsuper OR p.rolcreatedb OR p.rolcreaterole OR p.rolreplication OR p.rolbypassrls) AND pg_has_role(current_user,p.oid,'MEMBER')) AND NOT EXISTS(SELECT 1 FROM pg_class c JOIN pg_namespace n ON n.oid=c.relnamespace WHERE n.nspname IN ('qgr','qgr_recordings') AND pg_has_role(current_user,c.relowner,'MEMBER')) FROM pg_roles r WHERE r.rolname=current_user").bind(role.name()).fetch_one(&pool).await.map_err(|_|Error::Role)?;
            let mixed:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_roles r WHERE r.rolname=ANY($1) AND r.rolname<>$2 AND pg_has_role(current_user,r.oid,'MEMBER'))").bind(vec!["qgr_app","qgr_audit","qgr_record_ingest","qgr_history_owner","qgr_history_admin","qgr_record_lifecycle"]).bind(role.name()).fetch_one(&pool).await.map_err(|_|Error::Role)?;
            let rewrites:bool=sqlx::query_scalar("SELECT has_schema_privilege(current_user,'qgr_recordings','CREATE') OR has_schema_privilege(current_user,'qgr','CREATE') OR has_table_privilege(current_user,'qgr_recordings.receipts','UPDATE,DELETE,TRUNCATE') OR has_table_privilege(current_user,'qgr_recordings.content','UPDATE,DELETE,TRUNCATE') OR has_table_privilege(current_user,'qgr_recordings.held_content','INSERT,UPDATE,DELETE,TRUNCATE')").fetch_one(&pool).await.map_err(|_|Error::Role)?;
            if !safe || mixed || rewrites {return Err(Error::Role);}
            let grants=sqlx::query("SELECT has_table_privilege(current_user,'qgr_recordings.content','SELECT') AS reads,has_table_privilege(current_user,'qgr_recordings.content','INSERT') AS inserts,has_table_privilege(current_user,'qgr_recordings.held_content','SELECT') AS held,has_table_privilege(current_user,'qgr_recordings.receipts','SELECT') AS receipts,has_table_privilege(current_user,'qgr_recordings.receipts','INSERT') AS receipt_inserts").fetch_one(&pool).await.map_err(|_|Error::Role)?;
            if grants.get::<bool,_>("reads")!=matches!(role,RecordingRole::OwnerRead|RecordingRole::AdminRead) || grants.get::<bool,_>("inserts")!=(role==RecordingRole::Ingest) || grants.get::<bool,_>("held")!=(role==RecordingRole::AdminRead) || grants.get::<bool,_>("receipts")!=(role!=RecordingRole::Lifecycle) || grants.get::<bool,_>("receipt_inserts")!=(role==RecordingRole::Ingest) {return Err(Error::Role);}
            if version.is_some_and(|v|v>=11) {
                let unsafe_grants:bool=sqlx::query_scalar("SELECT has_table_privilege(current_user,'qgr_recordings.held_gaps','SELECT')<>$1 OR has_table_privilege(current_user,'qgr_recordings.held_gaps','INSERT,UPDATE,DELETE,TRUNCATE') OR has_table_privilege(current_user,'qgr_recordings.held_gap_cases','SELECT,INSERT,UPDATE,DELETE,TRUNCATE') OR has_table_privilege(current_user,'qgr_recordings.held_cases','SELECT,INSERT,UPDATE,DELETE,TRUNCATE')").bind(role==RecordingRole::AdminRead).fetch_one(&pool).await.map_err(|_|Error::Role)?;
                if unsafe_grants {return Err(Error::Role);}
            }
            if version.is_some_and(|v|v>=12) {
                let direct:bool=sqlx::query_scalar("SELECT has_table_privilege(current_user,'qgr_recordings.holds','SELECT,INSERT,UPDATE,DELETE,TRUNCATE')").fetch_one(&pool).await.map_err(|_|Error::Role)?;
                if direct {return Err(Error::Role);}
            }
            Ok(())
        }.await;
        if let Err(error) = check {
            pool.close().await;
            return Err(error);
        }
        Ok(Self {
            pool,
            role,
            commits: std::sync::Arc::new(super::transaction::Commits::default()),
        })
    }
    pub fn role(&self) -> RecordingRole {
        self.role
    }
    pub(crate) async fn matches_authority(&self, authority: &Store) -> Result<bool, Error> {
        let primary: String = sqlx::query_scalar("SELECT qgr_recordings.database_identity()")
            .fetch_one(&authority.pool)
            .await
            .map_err(|_| Error::Schema)?;
        let recordings: String = sqlx::query_scalar("SELECT qgr_recordings.database_identity()")
            .fetch_one(&self.pool)
            .await
            .map_err(|_| Error::Schema)?;
        Ok(primary == recordings)
    }
    pub async fn ready(&self) -> Result<(), Error> {
        let version: Option<i32> =
            sqlx::query_scalar("SELECT max(version) FROM qgr.schema_versions")
                .fetch_one(&self.pool)
                .await
                .map_err(|_| Error::Database)?;
        if !version.is_some_and(|v| (9..=crate::SCHEMA_MAX).contains(&v)) {
            return Err(Error::Schema);
        }
        Ok(())
    }
    pub async fn close(&self) {
        self.commits.close().await;
        self.pool.close().await;
    }
}

#[cfg(test)]
pub(crate) mod tests;
