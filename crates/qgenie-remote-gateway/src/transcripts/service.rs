//! HTTP composition owns separate credentials for each recording purpose.
use super::{RecordingDatabase, RecordingRedaction, RecordingRole, RecordingValidation};
use crate::{Error, SecretDsn, Store};

#[derive(Clone)]
pub(crate) struct Recordings {
    pub ingest: RecordingDatabase,
    pub owner: RecordingDatabase,
    pub admin: RecordingDatabase,
    pub validation: RecordingValidation,
    pub redaction: RecordingRedaction,
}
impl Recordings {
    pub async fn connect(
        authority: &Store,
        ingest: &SecretDsn,
        owner: &SecretDsn,
        admin: &SecretDsn,
        redaction: RecordingRedaction,
    ) -> Result<Self, Error> {
        let mut opened = Vec::new();
        for (secret, role, size) in [
            (ingest, RecordingRole::Ingest, 8),
            (owner, RecordingRole::OwnerRead, 4),
            (admin, RecordingRole::AdminRead, 4),
        ] {
            let result = async {
                let database = RecordingDatabase::connect(secret, role, size).await?;
                match database.matches_authority(authority).await {
                    Ok(true) => Ok(database),
                    other => {
                        database.close().await;
                        Err(other.err().unwrap_or(Error::Configuration))
                    }
                }
            }
            .await;
            match result {
                Ok(database) => opened.push(database),
                Err(error) => {
                    for database in opened {
                        database.close().await;
                    }
                    return Err(error);
                }
            }
        }
        let mut databases = opened.into_iter();
        Ok(Self {
            ingest: databases.next().ok_or(Error::Configuration)?,
            owner: databases.next().ok_or(Error::Configuration)?,
            admin: databases.next().ok_or(Error::Configuration)?,
            validation: RecordingValidation::default(),
            redaction,
        })
    }
    pub async fn close(&self) {
        self.ingest.close().await;
        self.owner.close().await;
        self.admin.close().await;
    }
}
