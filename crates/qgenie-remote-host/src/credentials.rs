use crate::{Error, http};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey};
use fs2::FileExt;
use qgenie_remote_proto::{InstallationId, KeyId, MachineId, UserId};
use qgenie_remote_transport::ipc::{current_uid, private_directory, private_file};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    sync::{Arc, OnceLock},
    time::Duration,
};
use tokio::sync::Semaphore;
use zeroize::Zeroizing;

#[derive(Debug, Clone, Copy)]
pub struct CredentialOptions {
    pub prefer_keyring: bool,
    pub allow_private_file: bool,
}
impl Default for CredentialOptions {
    fn default() -> Self {
        Self {
            prefer_keyring: true,
            allow_private_file: true,
        }
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protection {
    OsCredentialStoreExportable,
    PrivateFile,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingStatus {
    Enrolled,
    RecoveryPending,
    RevocationPending,
    Revoked,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Binding {
    pub installation: InstallationId,
    pub key: KeyId,
    pub machine: MachineId,
    pub owner: UserId,
    pub status: BindingStatus,
}
#[derive(Clone, Serialize, Deserialize)]
struct Descriptor {
    version: u16,
    origin: String,
    uid: u32,
    os_user: String,
    installation: InstallationId,
    key: KeyId,
    public_key: String,
    protection: Protection,
}
#[derive(Clone, Serialize, Deserialize)]
struct RecoveryJournal {
    previous: Descriptor,
    descriptor: Descriptor,
    binding: Binding,
}
/// Holds a private nonblocking enrollment lock. Do not retain while running the
/// long-lived connector; use `machine_credentials` to take only the signer/binding.
pub struct Identity {
    directory: PathBuf,
    descriptor: Descriptor,
    signer: SigningKey,
    binding: Option<Binding>,
    recovery: Option<RecoveryJournal>,
    _lock: File,
}
impl std::fmt::Debug for Identity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Identity")
            .field("installation", &self.descriptor.installation)
            .field("protection", &self.descriptor.protection)
            .field("binding", &self.binding)
            .finish_non_exhaustive()
    }
}
impl Identity {
    pub async fn open(
        directory: &Path,
        origin: &str,
        os_user: &str,
        options: CredentialOptions,
    ) -> Result<Self, Error> {
        let url = http::https(origin)?;
        let canonical = url.origin().ascii_serialization();
        if canonical != origin
            || os_user.is_empty()
            || os_user.len() > 128
            || os_user.chars().any(char::is_control)
        {
            return Err(Error::Configuration);
        }
        private_directory(directory).map_err(|_| Error::State)?;
        let lock = private_file(&directory.join("identity.lock")).map_err(|_| Error::State)?;
        lock.try_lock_exclusive().map_err(|_| Error::Busy)?;
        let descriptor = load::<Descriptor>(&directory.join("identity.json"))?;
        let binding = load::<Binding>(&directory.join("binding.json"))?;
        if binding
            .as_ref()
            .is_some_and(|binding| binding.status == BindingStatus::Revoked)
        {
            return Err(Error::Revoked);
        }
        let recovery = load::<RecoveryJournal>(&directory.join("recovery.json"))?;
        let (descriptor, binding) = if let Some(journal) = &recovery {
            if journal.previous.installation != journal.descriptor.installation
                || journal.previous.origin != journal.descriptor.origin
                || journal.previous.uid != journal.descriptor.uid
                || journal.previous.os_user != journal.descriptor.os_user
                || journal.previous.key == journal.descriptor.key
                || journal.binding.key != journal.descriptor.key
                || journal.binding.installation != journal.descriptor.installation
                || journal.binding.status != BindingStatus::RecoveryPending
                || binding.as_ref().is_none_or(|binding| {
                    binding.machine != journal.binding.machine
                        || binding.owner != journal.binding.owner
                        || binding.installation != journal.binding.installation
                })
            {
                return Err(Error::State);
            }
            (
                Some(journal.descriptor.clone()),
                Some(journal.binding.clone()),
            )
        } else {
            (descriptor, binding)
        };
        let (descriptor, signer) = if let Some(descriptor) = descriptor {
            if descriptor.version != 1
                || descriptor.origin != canonical
                || descriptor.uid != current_uid()
                || descriptor.os_user != os_user
            {
                return Err(Error::State);
            }
            let seed = match descriptor.protection {
                Protection::OsCredentialStoreExportable => {
                    keyring_get(keyring_account(&descriptor)).await?
                }
                Protection::PrivateFile => {
                    let pending = directory.join("recovery.key");
                    file_seed(&if recovery.is_some()
                        && pending.try_exists().map_err(|_| Error::State)?
                    {
                        pending
                    } else {
                        directory.join("machine.key")
                    })?
                }
            };
            let signer = SigningKey::from_bytes(&seed);
            if STANDARD.encode(signer.verifying_key().as_bytes()) != descriptor.public_key {
                return Err(Error::KeyLost);
            }
            (descriptor, signer)
        } else {
            if binding.is_some()
                || directory
                    .join("machine.key")
                    .try_exists()
                    .map_err(|_| Error::State)?
            {
                return Err(Error::KeyLost);
            }
            let seed = Zeroizing::new(rand::random::<[u8; 32]>());
            let signer = SigningKey::from_bytes(&seed);
            let mut descriptor = Descriptor {
                version: 1,
                origin: canonical,
                uid: current_uid(),
                os_user: os_user.into(),
                installation: InstallationId::new(),
                key: KeyId::new(),
                public_key: STANDARD.encode(signer.verifying_key().as_bytes()),
                protection: Protection::OsCredentialStoreExportable,
            };
            let stored = options.prefer_keyring
                && keyring_set(
                    keyring_account(&descriptor),
                    STANDARD.encode(seed.as_slice()),
                )
                .await
                .is_ok();
            if !stored {
                if !options.allow_private_file {
                    return Err(Error::CredentialStore);
                }
                descriptor.protection = Protection::PrivateFile;
                // Persist a descriptor before the seed: interruption is explicit key loss,
                // and no network request can precede both durable writes.
                persist(directory, "identity.json", &descriptor)?;
                let mut file =
                    private_file(&directory.join("machine.key")).map_err(|_| Error::State)?;
                file.write_all(seed.as_slice()).map_err(|_| Error::State)?;
                file.sync_all().map_err(|_| Error::State)?;
                File::open(directory)
                    .and_then(|file| file.sync_all())
                    .map_err(|_| Error::State)?;
            }
            persist(directory, "identity.json", &descriptor)?;
            (descriptor, signer)
        };
        if binding.as_ref().is_some_and(|binding| {
            binding.installation != descriptor.installation || binding.key != descriptor.key
        }) {
            return Err(Error::State);
        }
        Ok(Self {
            directory: directory.to_owned(),
            descriptor,
            signer,
            binding,
            recovery,
            _lock: lock,
        })
    }
    pub fn installation(&self) -> InstallationId {
        self.descriptor.installation
    }
    pub fn key(&self) -> KeyId {
        self.descriptor.key
    }
    pub fn public_key(&self) -> &str {
        &self.descriptor.public_key
    }
    pub fn origin(&self) -> &str {
        &self.descriptor.origin
    }
    pub fn os_user(&self) -> &str {
        &self.descriptor.os_user
    }
    pub fn protection(&self) -> Protection {
        self.descriptor.protection
    }
    pub fn binding(&self) -> Option<&Binding> {
        self.binding.as_ref()
    }
    pub fn machine_credentials(&self) -> Result<MachineCredentials, Error> {
        if self.recovery.is_some() {
            return Err(Error::State);
        }
        let binding = self.binding.clone().ok_or(Error::State)?;
        if binding.status != BindingStatus::Enrolled {
            return Err(Error::Revoked);
        }
        Ok(MachineCredentials {
            origin: self.origin().into(),
            binding,
            signer: self.signer.clone(),
            protection: self.protection(),
        })
    }
    /// Explicit, journaled replacement of a missing or rotated key. The old
    /// installation/machine/owner stay fixed; a lost gateway reply reuses this
    /// pending replacement key on the next `open`/`enroll` call.
    pub async fn recover_key(
        directory: &Path,
        origin: &str,
        os_user: &str,
        options: CredentialOptions,
        confirmed: bool,
    ) -> Result<Self, Error> {
        if !confirmed {
            return Err(Error::Denied);
        }
        private_directory(directory).map_err(|_| Error::State)?;
        let lock = private_file(&directory.join("identity.lock")).map_err(|_| Error::State)?;
        lock.try_lock_exclusive().map_err(|_| Error::Busy)?;
        if load::<RecoveryJournal>(&directory.join("recovery.json"))?.is_some() {
            drop(lock);
            return Self::open(directory, origin, os_user, options).await;
        }
        let previous =
            load::<Descriptor>(&directory.join("identity.json"))?.ok_or(Error::KeyLost)?;
        let mut binding = load::<Binding>(&directory.join("binding.json"))?.ok_or(Error::State)?;
        if previous.version != 1
            || previous.origin != origin
            || previous.os_user != os_user
            || previous.uid != current_uid()
            || binding.installation != previous.installation
            || binding.key != previous.key
        {
            return Err(Error::State);
        }
        if binding.status != BindingStatus::Enrolled {
            return Err(Error::Revoked);
        }
        let seed = Zeroizing::new(rand::random::<[u8; 32]>());
        let signer = SigningKey::from_bytes(&seed);
        let mut descriptor = previous.clone();
        descriptor.key = KeyId::new();
        descriptor.public_key = STANDARD.encode(signer.verifying_key().as_bytes());
        descriptor.protection = Protection::OsCredentialStoreExportable;
        if !options.prefer_keyring
            || keyring_set(
                keyring_account(&descriptor),
                STANDARD.encode(seed.as_slice()),
            )
            .await
            .is_err()
        {
            if !options.allow_private_file {
                return Err(Error::CredentialStore);
            }
            descriptor.protection = Protection::PrivateFile;
            let mut file =
                private_file(&directory.join("recovery.key")).map_err(|_| Error::State)?;
            file.set_len(0).map_err(|_| Error::State)?;
            file.write_all(seed.as_slice()).map_err(|_| Error::State)?;
            file.sync_all().map_err(|_| Error::State)?;
            File::open(directory)
                .and_then(|file| file.sync_all())
                .map_err(|_| Error::State)?;
        }
        binding.key = descriptor.key;
        binding.status = BindingStatus::RecoveryPending;
        persist(
            directory,
            "recovery.json",
            &RecoveryJournal {
                previous,
                descriptor,
                binding,
            },
        )?;
        drop(lock);
        Self::open(directory, origin, os_user, options).await
    }
    pub(crate) async fn finish_recovery(&mut self) -> Result<(), Error> {
        let Some(recovery) = self.recovery.clone() else {
            return Ok(());
        };
        if self
            .binding
            .as_ref()
            .is_none_or(|binding| binding.status != BindingStatus::Enrolled)
        {
            return Err(Error::State);
        }
        if self.protection() == Protection::PrivateFile {
            let pending = self.directory.join("recovery.key");
            if pending.try_exists().map_err(|_| Error::State)? {
                std::fs::rename(pending, self.directory.join("machine.key"))
                    .map_err(|_| Error::State)?;
            }
            let seed = file_seed(&self.directory.join("machine.key"))?;
            if STANDARD.encode(SigningKey::from_bytes(&seed).verifying_key().as_bytes())
                != self.public_key()
            {
                return Err(Error::KeyLost);
            }
        } else if recovery.previous.protection == Protection::PrivateFile {
            match std::fs::remove_file(self.directory.join("machine.key")) {
                Ok(()) => {}
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(_) => return Err(Error::State),
            }
        }
        persist(&self.directory, "identity.json", &self.descriptor)?;
        if recovery.previous.protection == Protection::OsCredentialStoreExportable {
            keyring_delete(keyring_account(&recovery.previous)).await?;
        }
        std::fs::remove_file(self.directory.join("recovery.json")).map_err(|_| Error::State)?;
        File::open(&self.directory)
            .and_then(|file| file.sync_all())
            .map_err(|_| Error::State)?;
        self.recovery = None;
        Ok(())
    }
    pub(crate) fn sign(&self, bytes: &[u8]) -> [u8; 64] {
        self.signer.sign(bytes).to_bytes()
    }
    pub(crate) fn set_binding(&mut self, binding: Binding) -> Result<(), Error> {
        if binding.installation != self.installation()
            || binding.key != self.key()
            || self.binding.as_ref().is_some_and(|previous| {
                previous.machine != binding.machine || previous.owner != binding.owner
            })
        {
            return Err(Error::State);
        }
        persist(&self.directory, "binding.json", &binding)?;
        self.binding = Some(binding);
        Ok(())
    }
    pub(crate) async fn remove_key(&self) -> Result<(), Error> {
        match self.protection() {
            Protection::PrivateFile => {
                match std::fs::remove_file(self.directory.join("machine.key")) {
                    Ok(()) => File::open(&self.directory)
                        .and_then(|file| file.sync_all())
                        .map_err(|_| Error::State),
                    Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
                    Err(_) => Err(Error::State),
                }
            }
            Protection::OsCredentialStoreExportable => {
                keyring_delete(keyring_account(&self.descriptor)).await
            }
        }
    }
}
/// The long-lived connector receives this type, never `oauth::Tokens` or a user
/// refresh token. The software Ed25519 signer remains exportable in process.
pub struct MachineCredentials {
    pub(crate) origin: String,
    pub(crate) binding: Binding,
    signer: SigningKey,
    pub protection: Protection,
}
impl std::fmt::Debug for MachineCredentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MachineCredentials")
            .field("binding", &self.binding)
            .field("protection", &self.protection)
            .finish_non_exhaustive()
    }
}
impl MachineCredentials {
    pub fn binding(&self) -> &Binding {
        &self.binding
    }
    pub(crate) fn sign(&self, bytes: &[u8]) -> [u8; 64] {
        self.signer.sign(bytes).to_bytes()
    }
}
fn file_seed(path: &Path) -> Result<Zeroizing<[u8; 32]>, Error> {
    let mut bytes = Zeroizing::new(Vec::new());
    read(path, 33)
        .map_err(|_| Error::KeyLost)?
        .ok_or(Error::KeyLost)
        .map(|value| *bytes = value)?;
    Ok(Zeroizing::new(
        bytes.as_slice().try_into().map_err(|_| Error::KeyLost)?,
    ))
}
fn load<T: serde::de::DeserializeOwned>(path: &Path) -> Result<Option<T>, Error> {
    read(path, 16384)?
        .map(|bytes| qgenie_remote_proto::decode_json(&bytes).map_err(|_| Error::State))
        .transpose()
}
fn read(path: &Path, limit: u64) -> Result<Option<Vec<u8>>, Error> {
    match std::fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(Error::State),
        Ok(metadata) if !metadata.is_file() || metadata.len() > limit => return Err(Error::State),
        _ => {}
    }
    let file = private_file(path).map_err(|_| Error::State)?;
    let mut bytes = Vec::new();
    file.take(limit + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::State)?;
    if bytes.len() as u64 > limit {
        return Err(Error::State);
    }
    Ok(Some(bytes))
}
fn persist<T: Serialize>(directory: &Path, name: &str, value: &T) -> Result<(), Error> {
    let bytes = serde_json::to_vec(value).map_err(|_| Error::State)?;
    if bytes.len() > 16384 {
        return Err(Error::State);
    }
    let pending = directory.join(format!("{name}.pending"));
    let mut file = private_file(&pending).map_err(|_| Error::State)?;
    file.set_len(0).map_err(|_| Error::State)?;
    file.seek(SeekFrom::Start(0)).map_err(|_| Error::State)?;
    file.write_all(&bytes).map_err(|_| Error::State)?;
    file.sync_all().map_err(|_| Error::State)?;
    std::fs::rename(pending, directory.join(name)).map_err(|_| Error::State)?;
    File::open(directory)
        .and_then(|file| file.sync_all())
        .map_err(|_| Error::State)
}
fn keyring_account(descriptor: &Descriptor) -> String {
    format!(
        "{}:{}:{}:{}",
        descriptor.uid,
        descriptor.installation,
        descriptor.key,
        hex(&Sha256::digest(descriptor.origin.as_bytes()))
    )
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
async fn keyring_work<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, Error> + Send + 'static,
) -> Result<T, Error> {
    static WORKERS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    let permit = WORKERS
        .get_or_init(|| Arc::new(Semaphore::new(4)))
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::CredentialStore)?;
    let task = tokio::task::spawn_blocking(move || {
        let _permit = permit;
        work()
    });
    tokio::time::timeout(Duration::from_secs(3), task)
        .await
        .map_err(|_| Error::CredentialStore)?
        .map_err(|_| Error::CredentialStore)?
}
async fn keyring_set(account: String, value: String) -> Result<(), Error> {
    let value = Zeroizing::new(value);
    keyring_work(move || {
        #[cfg(feature = "os-keyring")]
        {
            keyring::Entry::new("qgenie-remote-machine-v1", &account)
                .and_then(|entry| entry.set_password(&value))
                .map_err(|_| Error::CredentialStore)
        }
        #[cfg(not(feature = "os-keyring"))]
        {
            let _ = (account, value);
            Err(Error::CredentialStore)
        }
    })
    .await
}
async fn keyring_get(account: String) -> Result<Zeroizing<[u8; 32]>, Error> {
    keyring_work(move || {
        #[cfg(feature = "os-keyring")]
        {
            let value = Zeroizing::new(
                keyring::Entry::new("qgenie-remote-machine-v1", &account)
                    .and_then(|entry| entry.get_password())
                    .map_err(|error| match error {
                        keyring::Error::NoEntry => Error::KeyLost,
                        _ => Error::CredentialStore,
                    })?,
            );
            let bytes = Zeroizing::new(
                STANDARD
                    .decode(value.as_str())
                    .map_err(|_| Error::KeyLost)?,
            );
            Ok(Zeroizing::new(
                bytes.as_slice().try_into().map_err(|_| Error::KeyLost)?,
            ))
        }
        #[cfg(not(feature = "os-keyring"))]
        {
            let _ = account;
            Err(Error::KeyLost)
        }
    })
    .await
}
async fn keyring_delete(account: String) -> Result<(), Error> {
    keyring_work(move || {
        #[cfg(feature = "os-keyring")]
        {
            match keyring::Entry::new("qgenie-remote-machine-v1", &account)
                .and_then(|entry| entry.delete_credential())
            {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(_) => Err(Error::CredentialStore),
            }
        }
        #[cfg(not(feature = "os-keyring"))]
        {
            let _ = account;
            Err(Error::CredentialStore)
        }
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    #[ignore = "requires QGR_EXPECT_MISSING_KEYRING=yes and an unavailable Linux session bus"]
    async fn missing_platform_keyring_is_disclosed_and_cannot_silently_change_an_existing_backend()
    {
        assert_eq!(
            std::env::var("QGR_EXPECT_MISSING_KEYRING").as_deref(),
            Ok("yes")
        );
        let directory = tempfile::Builder::new()
            .permissions(
                <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
            )
            .tempdir()
            .unwrap();
        let identity = Identity::open(
            directory.path(),
            "https://gateway.example.invalid",
            "fixture",
            CredentialOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(identity.protection(), Protection::PrivateFile);
        let public = identity.public_key().to_string();
        drop(identity);
        let identity = Identity::open(
            directory.path(),
            "https://gateway.example.invalid",
            "fixture",
            CredentialOptions::default(),
        )
        .await
        .unwrap();
        assert_eq!(identity.public_key(), public);
        drop(identity);
        // A claimed existing OS-store key cannot fall back to the valid seed
        // sitting in the file backend when that OS store becomes unavailable.
        let path = directory.path().join("identity.json");
        let mut descriptor: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        descriptor["protection"] = serde_json::json!("os_credential_store_exportable");
        std::fs::write(&path, serde_json::to_vec(&descriptor).unwrap()).unwrap();
        assert!(matches!(
            Identity::open(
                directory.path(),
                "https://gateway.example.invalid",
                "fixture",
                CredentialOptions::default()
            )
            .await,
            Err(Error::CredentialStore) | Err(Error::KeyLost)
        ));
        let directory = tempfile::Builder::new()
            .permissions(
                <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
            )
            .tempdir()
            .unwrap();
        assert!(matches!(
            Identity::open(
                directory.path(),
                "https://gateway.example.invalid",
                "fixture",
                CredentialOptions {
                    prefer_keyring: true,
                    allow_private_file: false
                }
            )
            .await,
            Err(Error::CredentialStore)
        ));
    }
}
