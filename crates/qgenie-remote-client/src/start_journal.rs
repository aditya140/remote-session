//! Durable start identities use indexed storage, not a lifetime file-count limit.
use crate::RemoteStartIdentity;
use fs2::FileExt;
use qgenie_remote_proto::*;
use qgenie_remote_transport::ipc::{private_directory, private_file};
use rusqlite::{Connection, OptionalExtension, params};
use std::{io::Read, path::Path};

pub(crate) struct Journal {
    db: Connection,
    _local_lock: std::fs::File,
    _remote_lock: std::fs::File,
}
impl Journal {
    pub fn open(state: &Path) -> Result<Self, RemoteError> {
        Self::open_inner(state).map_err(|_| unavailable())
    }
    fn open_inner(state: &Path) -> anyhow::Result<Self> {
        private_directory(state)?;
        let local = private_file(&state.join("starts.lock"))?;
        local.lock_exclusive()?;
        let remote = private_file(&state.join("remote-starts.lock"))?;
        remote.lock_exclusive()?;
        let path = state.join("start-identities.sqlite");
        private_file(&path)?;
        let db = Connection::open(path)?;
        db.busy_timeout(std::time::Duration::from_secs(2))?;
        // 64 MiB of identity metadata. Keep uncertain identities on exhaustion.
        db.execute_batch(
            "PRAGMA journal_mode=DELETE;
            PRAGMA synchronous=FULL;
            PRAGMA max_page_count=16384;
            CREATE TABLE IF NOT EXISTS identities(
                scope TEXT NOT NULL,key TEXT NOT NULL,digest TEXT NOT NULL,
                body TEXT,PRIMARY KEY(scope,key));
            CREATE TABLE IF NOT EXISTS imported(name TEXT PRIMARY KEY);",
        )?;
        let mut journal = Self {
            db,
            _local_lock: local,
            _remote_lock: remote,
        };
        journal.import(state, "starts")?;
        journal.import(state, "remote-starts")?;
        std::fs::File::open(state)?.sync_all()?;
        Ok(journal)
    }
    fn import(&mut self, state: &Path, name: &str) -> anyhow::Result<()> {
        if self
            .db
            .query_row("SELECT 1 FROM imported WHERE name=?1", [name], |row| {
                row.get::<_, i32>(0)
            })
            .optional()?
            .is_some()
        {
            return Ok(());
        }
        let directory = state.join(name);
        let tx = self.db.transaction()?;
        if directory.try_exists()? {
            private_directory(&directory)?;
            for entry in std::fs::read_dir(&directory)? {
                let entry = entry?;
                if entry
                    .path()
                    .extension()
                    .is_none_or(|extension| extension != "json")
                {
                    continue;
                }
                let mut bytes = Vec::new();
                private_file(&entry.path())?
                    .take(MAX_JSON_BYTES as u64 + 1)
                    .read_to_end(&mut bytes)?;
                anyhow::ensure!(
                    !bytes.is_empty() && bytes.len() <= MAX_JSON_BYTES,
                    "invalid legacy start identity"
                );
                let (scope, key, digest, body) = if name == "starts" {
                    let spec: StartSpec = decode_json(&bytes)?;
                    anyhow::ensure!(
                        entry.file_name().to_str() == Some(&format!("{}.json", spec.key)),
                        "invalid legacy identity filename"
                    );
                    ("local".to_owned(), spec.key, fingerprint(&spec)?, None)
                } else {
                    let saved: RemoteStartIdentity = decode_json(&bytes)?;
                    anyhow::ensure!(
                        saved.fingerprint.len() == 64
                            && saved
                                .fingerprint
                                .bytes()
                                .all(|byte| byte.is_ascii_hexdigit()),
                        "invalid legacy fingerprint"
                    );
                    let scope = remote_scope(&saved.origin, saved.owner, saved.machine)?;
                    anyhow::ensure!(
                        entry.file_name().to_str()
                            == Some(&format!("{}-{}.json", scope, saved.key)),
                        "invalid legacy identity filename"
                    );
                    let body = serde_json::to_string(&saved)?;
                    (scope, saved.key, saved.fingerprint, Some(body))
                };
                let previous: Option<String> = tx
                    .query_row(
                        "SELECT digest FROM identities WHERE scope=?1 AND key=?2",
                        params![scope, key.to_string()],
                        |row| row.get(0),
                    )
                    .optional()?;
                anyhow::ensure!(
                    previous.as_ref().is_none_or(|prior| prior == &digest),
                    "legacy identity conflict"
                );
                tx.execute(
                    "INSERT OR IGNORE INTO identities VALUES(?1,?2,?3,?4)",
                    params![scope, key.to_string(), digest, body],
                )?;
            }
        }
        tx.execute("INSERT INTO imported VALUES(?1)", [name])?;
        tx.commit()?;
        // Legacy files remain private migration backups. Old writers must not
        // share a migrated directory; the owning daemon still deduplicates keys.
        Ok(())
    }
    fn record(
        &self,
        scope: &str,
        key: IdempotencyKey,
        digest: &str,
        body: Option<&str>,
    ) -> Result<(), RemoteError> {
        let previous: Option<String> = self
            .db
            .query_row(
                "SELECT digest FROM identities WHERE scope=?1 AND key=?2",
                params![scope, key.to_string()],
                |row| row.get(0),
            )
            .optional()
            .map_err(|_| unavailable())?;
        if let Some(previous) = previous {
            if previous != digest {
                return Err(RemoteError {
                    code: ErrorCode::IdempotencyConflict,
                    outcome: OperationOutcome::NotSent,
                    message: "start key has different parameters".into(),
                    retryable: false,
                });
            }
            return Ok(());
        }
        self.db
            .execute(
                "INSERT INTO identities VALUES(?1,?2,?3,?4)",
                params![scope, key.to_string(), digest, body],
            )
            .map_err(|_| unavailable())?;
        Ok(())
    }
    pub fn local(&self, spec: &StartSpec) -> Result<(), RemoteError> {
        self.record("local", spec.key, &fingerprint(spec)?, None)
    }
    pub fn remote(&self, saved: &RemoteStartIdentity) -> Result<(), RemoteError> {
        let scope = remote_scope(&saved.origin, saved.owner, saved.machine)?;
        self.record(
            &scope,
            saved.key,
            &saved.fingerprint,
            Some(&serde_json::to_string(saved).map_err(|_| unavailable())?),
        )
    }
    pub fn remote_page(
        &self,
        origin: &str,
        owner: UserId,
        machine: MachineId,
        after: Option<IdempotencyKey>,
        limit: u16,
    ) -> Result<(Vec<RemoteStartIdentity>, Option<IdempotencyKey>), RemoteError> {
        if !(1..=256).contains(&limit) {
            return Err(RemoteError::rejected(
                ErrorCode::InvalidFrame,
                "journal page limit must be 1..=256",
            ));
        }
        let scope = remote_scope(origin, owner, machine)?;
        let mut query=self.db.prepare("SELECT body FROM identities WHERE scope=?1 AND (?2 IS NULL OR key>?2) ORDER BY key LIMIT ?3").map_err(|_|unavailable())?;
        let rows = query
            .query_map(
                params![
                    scope,
                    after.map(|key| key.to_string()),
                    u32::from(limit) + 1
                ],
                |row| row.get::<_, String>(0),
            )
            .map_err(|_| unavailable())?;
        let mut records = Vec::new();
        let mut bytes = 0;
        for row in rows {
            let body = row.map_err(|_| unavailable())?;
            bytes += body.len();
            if records.len() == usize::from(limit) || bytes > 512 * 1024 {
                let next = records
                    .last()
                    .map(|saved: &RemoteStartIdentity| saved.key)
                    .ok_or_else(unavailable)?;
                return Ok((records, Some(next)));
            }
            let saved: RemoteStartIdentity =
                serde_json::from_str(&body).map_err(|_| unavailable())?;
            if saved.origin != origin || saved.owner != owner || saved.machine != machine {
                return Err(unavailable());
            }
            records.push(saved);
        }
        Ok((records, None))
    }
}
fn remote_scope(origin: &str, owner: UserId, machine: MachineId) -> Result<String, RemoteError> {
    validate_gateway_origin(origin)?;
    fingerprint(&(origin, owner, machine))
}
fn unavailable() -> RemoteError {
    RemoteError {
        code: ErrorCode::Internal,
        outcome: OperationOutcome::NotSent,
        message: "start identity journal is unavailable, invalid, or full; no request was sent"
            .into(),
        retryable: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn legacy_identities_survive_migration_and_history_does_not_fill_pending_work() {
        let dir = tempfile::Builder::new()
            .permissions(
                <std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700),
            )
            .tempdir()
            .unwrap();
        let spec = StartSpec {
            key: IdempotencyKey::new(),
            command: vec!["/bin/sh".into(), "private-argv".into()],
            cwd: "/tmp".into(),
            rows: 24,
            cols: 80,
        };
        let saved = RemoteStartIdentity {
            origin: "https://example.test".into(),
            owner: UserId::new(),
            machine: MachineId::new(),
            key: IdempotencyKey::new(),
            fingerprint: "a".repeat(64),
        };
        for name in ["starts", "remote-starts"] {
            private_directory(&dir.path().join(name)).unwrap();
        }
        let local = dir.path().join("starts").join(format!("{}.json", spec.key));
        private_file(&local)
            .unwrap()
            .write_all(&serde_json::to_vec(&spec).unwrap())
            .unwrap();
        let scope = remote_scope(&saved.origin, saved.owner, saved.machine).unwrap();
        let remote = dir
            .path()
            .join("remote-starts")
            .join(format!("{scope}-{}.json", saved.key));
        private_file(&remote)
            .unwrap()
            .write_all(&serde_json::to_vec(&saved).unwrap())
            .unwrap();
        let journal = Journal::open(dir.path()).unwrap();
        journal.local(&spec).unwrap();
        journal.remote(&saved).unwrap();
        // Use the production write path, including the durable commit, past the old cap.
        for _ in 0..4100 {
            journal
                .remote(&RemoteStartIdentity {
                    key: IdempotencyKey::new(),
                    ..saved.clone()
                })
                .unwrap();
        }
        assert!(local.exists() && remote.exists());
        drop(journal);
        let journal = Journal::open(dir.path()).unwrap();
        let mut after = None;
        let mut keys = std::collections::BTreeSet::new();
        loop {
            let (page, next) = journal
                .remote_page(&saved.origin, saved.owner, saved.machine, after, 127)
                .unwrap();
            assert!(page.len() <= 127);
            assert!(page.windows(2).all(|pair| pair[0].key < pair[1].key));
            for item in &page {
                assert!(keys.insert(item.key));
            }
            if next.is_none() {
                break;
            }
            assert_eq!(next, page.last().map(|item| item.key));
            after = next;
        }
        assert_eq!(keys.len(), 4101);
        assert!(keys.contains(&saved.key));
        assert!(
            journal
                .remote_page(&saved.origin, UserId::new(), saved.machine, None, 32)
                .unwrap()
                .0
                .is_empty()
        );
        let changed = RemoteStartIdentity {
            fingerprint: "b".repeat(64),
            ..saved
        };
        assert_eq!(
            journal.remote(&changed).unwrap_err().code,
            ErrorCode::IdempotencyConflict
        );
        let mut changed = spec;
        changed.cols += 1;
        assert_eq!(
            journal.local(&changed).unwrap_err().code,
            ErrorCode::IdempotencyConflict
        );
        assert!(
            !std::fs::read(dir.path().join("start-identities.sqlite"))
                .unwrap()
                .windows(b"private-argv".len())
                .any(|bytes| bytes == b"private-argv")
        );
    }
}
