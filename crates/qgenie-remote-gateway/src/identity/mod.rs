pub(crate) mod api;
mod challenges;
mod directory;
#[cfg(test)]
pub(crate) use directory::tests::Fixture as DirectoryFixture;
pub(crate) mod freshness;
pub use directory::{DirectoryConfig, DirectoryObservation, GraphDirectory};
pub(crate) mod jwt;
mod machines;
mod tokens;
use crate::{Error, Store};
pub use jwt::{JwtConfig, JwtVerifier};
use std::{
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};
pub use tokens::MachinePrincipal;
pub struct IdentityService {
    store: Store,
    tokens: tokens::MachineTokens,
    pub(crate) directory: Option<std::sync::Arc<freshness::DirectoryFreshness>>,
}
impl IdentityService {
    pub(crate) fn origin(&self) -> &str {
        &self.tokens.origin
    }
    pub async fn from_file(store: Store, origin: String, key_file: &Path) -> Result<Self, Error> {
        let version: Option<i32> =
            sqlx::query_scalar("SELECT max(version) FROM qgr.schema_versions")
                .fetch_one(&store.pool)
                .await
                .map_err(db)?;
        if !version.is_some_and(|version| version >= 3) {
            return Err(Error::Schema);
        }
        Ok(Self {
            store,
            tokens: tokens::MachineTokens::from_file(origin, key_file)?,
            directory: None,
        })
    }
    pub async fn authenticate_machine(&self, token: &str) -> Result<MachinePrincipal, Error> {
        let actor = self.tokens.verify(token, &self.store).await?;
        self.assert_directory(actor.owner).await?;
        // Directory verification may wait on HTTPS. Recheck primary flags and
        // exact token expiry after that wait, before admitting the connection.
        self.tokens.verify(token, &self.store).await
    }
    pub(crate) async fn assert_machine(&self, actor: &MachinePrincipal) -> Result<(), Error> {
        self.assert_directory(actor.owner).await?;
        let enabled: bool = sqlx::query_scalar("SELECT NOT(m.disabled OR u.disabled OR k.revoked OR a.disabled) FROM qgr.machines m JOIN qgr.users u ON u.id=m.owner JOIN qgr.machine_keys k ON k.id=m.active_key AND k.machine=m.id CROSS JOIN qgr.access_state a WHERE m.id=$1 AND m.owner=$2 AND m.active_key=$3 AND u.directory_object IS NOT NULL")
            .bind(actor.machine.to_string()).bind(actor.owner.to_string()).bind(actor.key.to_string()).fetch_optional(&self.store.pool).await.map_err(db)?.unwrap_or(false);
        if enabled {
            Ok(())
        } else {
            Err(Error::Forbidden)
        }
    }
    pub(crate) async fn assert_directory(
        &self,
        user: qgenie_remote_proto::UserId,
    ) -> Result<(), Error> {
        if let Some(directory) = &self.directory {
            directory.assert(user).await?;
        }
        Ok(())
    }
    pub(crate) async fn assert_admin(
        &self,
        user: qgenie_remote_proto::UserId,
    ) -> Result<(), Error> {
        self.assert_directory(user).await?;
        let enabled: bool = sqlx::query_scalar("SELECT NOT disabled FROM qgr.users WHERE id=$1")
            .bind(user.to_string())
            .fetch_optional(&self.store.pool)
            .await
            .map_err(db)?
            .unwrap_or(false);
        if enabled {
            Ok(())
        } else {
            Err(Error::Forbidden)
        }
    }
}
fn db(error: sqlx::Error) -> Error {
    match error
        .as_database_error()
        .and_then(|error| error.code())
        .as_deref()
    {
        Some("23505") => Error::Conflict,
        _ => Error::Database,
    }
}
pub(crate) fn now_ms() -> Result<u64, Error> {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Database)?
        .as_millis()
        .try_into()
        .map_err(|_| Error::Database)
}

#[cfg(test)]
pub(crate) mod tests;
