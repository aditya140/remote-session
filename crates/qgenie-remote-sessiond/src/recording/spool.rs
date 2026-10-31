use super::RecordingIdentity;
mod reclaim;
mod scope;
use anyhow::{Context, Result, ensure};
use base64::{Engine, engine::general_purpose::STANDARD};
use fs2::FileExt;
use qgenie_remote_proto::ipc::RemoteScope;
use qgenie_remote_proto::{
    Counter, GapId, RecordedContent as Content, RecordingRecord as Record, wire::*,
};
use qgenie_remote_transport::ipc::{private_directory, private_file};
use rusqlite::{Connection, OptionalExtension, Transaction, params};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    os::unix::fs::OpenOptionsExt,
    path::PathBuf,
    time::Duration,
};

pub struct SpoolConfig {
    pub directory: PathBuf,
    pub content_bytes: u64,
    pub metadata_bytes: u64,
}
impl SpoolConfig {
    pub fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            content_bytes: 256 * 1024 * 1024,
            metadata_bytes: 16 * 1024 * 1024,
        }
    }
}

pub enum SpoolItem {
    Content(RecordingBatch),
    Gap(LossRange),
}
pub struct SpoolStatus {
    pub pending_bytes: u64,
    pub chunks: u64,
    pub gaps: u64,
    pub warning: Option<String>,
    pub durability_unknown: bool,
}

/// Blocking worker-owned state, independent of replay SQLite and client/network lifetimes.
/// Opening reconciles outstanding reservations into conservative loss ranges before upload.
pub struct RecordingSpool {
    config: SpoolConfig,
    db: Connection,
    _lock: OwnerLock,
    unknown: bool,
}
// Fields drop in declaration order: SQLite closes before this lock releases.
// Explicit unlock also releases a descriptor duplicated during concurrent fork.
struct OwnerLock(File);
impl Drop for OwnerLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}
impl RecordingSpool {
    pub fn open(config: SpoolConfig, now: Timestamp) -> Result<Self> {
        ensure!(
            config.content_bytes > 0
                && (64 * 1024..=64 * 1024 * 1024).contains(&config.metadata_bytes),
            "invalid recording spool budgets"
        );
        private_directory(&config.directory)?;
        let lock = private_file(&config.directory.join("owner.lock"))?;
        lock.try_lock_exclusive()
            .context("recording spool already owned")?;
        let lock = OwnerLock(lock);
        let path = config.directory.join("metadata.sqlite");
        private_file(&path)?;
        let db = Connection::open(path)?;
        db.busy_timeout(Duration::from_millis(100))?;
        db.execute_batch("PRAGMA journal_mode=DELETE; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA auto_vacuum=NONE;")?;
        let version: u32 = db.pragma_query_value(None, "user_version", |r| r.get(0))?;
        ensure!(version <= 1, "unsupported recording schema");
        if version == 0 {
            db.execute_batch(include_str!("migrations/0001_spool.sql"))?;
        }
        let allocated: Option<u64> = db
            .query_row(
                "SELECT value FROM settings WHERE key='reserved_bytes'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(allocated) = allocated {
            ensure!(
                allocated == config.metadata_bytes,
                "recording metadata reserve configuration changed"
            );
        } else {
            // Allocate physical SQLite pages, then retain them on its free list for loss metadata.
            // Journals/filesystem failure can still defeat writes; this is not a disk-failure guarantee.
            db.execute("DELETE FROM allocation", [])?;
            db.execute(
                "INSERT INTO allocation VALUES(zeroblob(?1))",
                [config.metadata_bytes],
            )?;
            db.execute("DELETE FROM allocation", [])?;
            db.execute(
                "INSERT INTO settings VALUES('reserved_bytes',?1)",
                [config.metadata_bytes],
            )?;
        }
        let pages = config.metadata_bytes.div_ceil(4096) + 64;
        db.pragma_update(None, "max_page_count", pages)?;
        let mut spool = Self {
            config,
            db,
            _lock: lock,
            unknown: false,
        };
        spool.recover(now)?;
        spool.remove_orphans()?;
        Ok(spool)
    }

    pub fn register(&mut self, identity: &RecordingIdentity, at: Timestamp) -> Result<()> {
        qgenie_remote_proto::ipc::RemoteScope {
            origin: identity.origin.clone(),
            owner: identity.owner,
            machine: identity.machine,
        }
        .validate()?;
        let result = (|| {
            let tx = self.db.transaction()?;
            let text = serde_json::to_string(identity)?;
            let prior: Option<String> = tx
                .query_row(
                    "SELECT identity FROM streams WHERE epoch=?1",
                    [identity.stream_epoch.to_string()],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(prior) = prior {
                ensure!(prior == text, "recording epoch identity is immutable");
                return Ok(());
            }
            let (total, active): (u32, u32) = tx.query_row(
                "SELECT count(*),coalesce(sum(sealed=0),0) FROM streams s WHERE sealed=0 OR EXISTS(SELECT 1 FROM reservations r WHERE r.epoch=s.epoch) OR EXISTS(SELECT 1 FROM chunks c WHERE c.epoch=s.epoch) OR EXISTS(SELECT 1 FROM gaps g WHERE g.epoch=s.epoch)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            ensure!(
                total < 4096 && active < 64,
                "recording stream capacity reached"
            );
            tx.execute(
                "INSERT INTO streams VALUES(?1,?2,?3,?4,0)",
                params![
                    identity.stream_epoch.to_string(),
                    text,
                    0u64.to_be_bytes().as_slice(),
                    at.as_str()
                ],
            )?;
            tx.commit()?;
            Ok(())
        })();
        self.observe_error(result)
    }

    pub(super) fn is_sealed(&self, identity: &RecordingIdentity) -> Result<bool> {
        Ok(self.db.query_row(
            "SELECT sealed FROM streams WHERE epoch=?1",
            [identity.stream_epoch.to_string()],
            |row| row.get(0),
        )?)
    }

    /// Commits the recording sequence before the worker may expose/encode that record.
    pub fn reserve(&mut self, identity: &RecordingIdentity, at: Timestamp) -> Result<Counter> {
        let result = (|| {
            let tx = self.db.transaction()?;
            Self::identity(&tx, identity)?;
            let count: u32 = tx.query_row("SELECT count(*) FROM reservations", [], |r| r.get(0))?;
            ensure!(count < 4096, "recording reservation backlog full");
            let (frontier, sealed): (Vec<u8>, bool) = tx.query_row(
                "SELECT frontier,sealed FROM streams WHERE epoch=?1",
                [identity.stream_epoch.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            ensure!(!sealed, "recording stream sealed");
            let next = counter(&frontier)?
                .checked_add(1)
                .context("recording sequence exhausted")?;
            tx.execute(
                "INSERT INTO reservations VALUES(?1,?2,?3)",
                params![
                    identity.stream_epoch.to_string(),
                    next.to_be_bytes().as_slice(),
                    at.as_str()
                ],
            )?;
            tx.execute(
                "UPDATE streams SET frontier=?1,captured_at=?2 WHERE epoch=?3",
                params![
                    next.to_be_bytes().as_slice(),
                    at.as_str(),
                    identity.stream_epoch.to_string()
                ],
            )?;
            tx.commit()?;
            Ok(Counter(next))
        })();
        self.observe_error(result)
    }

    /// Persists a group ending on a complete record. A missing ACK never changes these batches.
    pub fn commit(
        &mut self,
        identity: &RecordingIdentity,
        batches: &[RecordingBatch],
    ) -> Result<()> {
        ensure!(
            !batches.is_empty() && batches.len() <= 256,
            "invalid recording commit group"
        );
        let reservations = self.reservations(identity)?;
        let completed = validate_group(identity, batches, &reservations)?;
        let mut prepared = Vec::new();
        for batch in batches {
            let bytes = STANDARD.decode(&batch.data)?;
            ensure!(
                bytes.len() as u64 <= self.config.content_bytes,
                "chunk exceeds configured content budget"
            );
            prepared.push((batch, bytes));
        }
        let result = (|| {
            for (batch, bytes) in &prepared {
                self.make_space(bytes.len() as u64)?;
                let path = self.content_path(batch.id);
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .mode(0o600)
                    .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
                    .open(path)?;
                file.write_all(bytes)?;
                file.sync_all()?;
                File::open(&self.config.directory)?.sync_all()?;
                let mut metadata = (*batch).clone();
                metadata.data.clear();
                self.db.execute(
                    "INSERT INTO chunks(id,epoch,metadata,bytes) VALUES(?1,?2,?3,?4)",
                    params![
                        batch.id.to_string(),
                        identity.stream_epoch.to_string(),
                        serde_json::to_string(&metadata)?,
                        bytes.len()
                    ],
                )?;
            }
            // Each persisted chunk is safe to offer. Until the whole group commits, recovery
            // conservatively marks reservations as uncertain, even if some content survived.
            let tx = self.db.transaction()?;
            for seq in completed {
                tx.execute(
                    "DELETE FROM reservations WHERE epoch=?1 AND seq=?2",
                    params![
                        identity.stream_epoch.to_string(),
                        seq.to_be_bytes().as_slice()
                    ],
                )?;
            }
            tx.commit()?;
            Ok(())
        })();
        self.observe_error(result)
    }

    /// Returns one bounded immutable item. Gaps have priority and become immutable when offered.
    pub fn fetch(&mut self) -> Result<Option<SpoolItem>> {
        self.fetch_filtered(None)
    }
    fn fetch_filtered(&mut self, scope: Option<&RemoteScope>) -> Result<Option<SpoolItem>> {
        if let Some(scope) = scope {
            scope.validate()?;
        }
        let machine = scope.map(|s| s.machine.to_string());
        let owner = scope.map(|s| s.owner.to_string());
        let gap: Option<(String, String)> = self
            .db
            .query_row(
                "SELECT g.id,g.metadata FROM gaps g JOIN streams s ON s.epoch=g.epoch WHERE (?1 IS NULL OR (json_extract(s.identity,'$.origin')=?1 AND json_extract(s.identity,'$.machine')=?2 AND json_extract(s.identity,'$.owner')=?3)) ORDER BY g.position LIMIT 1",
                params![scope.map(|s|s.origin.as_str()),machine,owner],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((id, text)) = gap {
            self.db
                .execute("UPDATE gaps SET offered=1 WHERE id=?1", [id])?;
            return Ok(Some(SpoolItem::Gap(serde_json::from_str(&text)?)));
        }
        let metadata: Option<String> = self
            .db
            .query_row(
                "SELECT c.metadata FROM chunks c JOIN streams s ON s.epoch=c.epoch WHERE (?1 IS NULL OR (json_extract(s.identity,'$.origin')=?1 AND json_extract(s.identity,'$.machine')=?2 AND json_extract(s.identity,'$.owner')=?3)) ORDER BY c.position LIMIT 1",
                params![scope.map(|s|s.origin.as_str()),machine,owner],
                |r| r.get(0),
            )
            .optional()?;
        let Some(metadata) = metadata else {
            return Ok(None);
        };
        let mut batch: RecordingBatch = serde_json::from_str(&metadata)?;
        let bytes = match self.read_content(&batch) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.unknown = true;
                self.lose_chunk(&batch, LossReason::DiskFailure)?;
                return Err(error);
            }
        };
        batch.data = STANDARD.encode(bytes);
        batch.validate()?;
        Ok(Some(SpoolItem::Content(batch)))
    }

    fn read_content(&self, batch: &RecordingBatch) -> Result<Vec<u8>> {
        let file = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
            .open(self.content_path(batch.id))?;
        ensure!(
            file.metadata()?.is_file() && file.metadata()?.len() <= 64 * 1024,
            "invalid spool content file"
        );
        let mut bytes = Vec::new();
        file.take(64 * 1024 + 1).read_to_end(&mut bytes)?;
        ensure!(
            format!("{:x}", Sha256::digest(&bytes)) == batch.digest,
            "corrupt recording content"
        );
        Ok(bytes)
    }

    /// Epoch must come from the authenticated backend recovery handshake, never from this receipt.
    pub fn acknowledge(
        &mut self,
        receipt: &CommitReceipt,
        authenticated_epoch: &str,
    ) -> Result<bool> {
        ensure!(
            receipt.recovery_epoch == authenticated_epoch && !authenticated_epoch.is_empty(),
            "recording recovery epoch mismatch"
        );
        let text: Option<String> = self
            .db
            .query_row(
                "SELECT metadata FROM chunks WHERE id=?1",
                [receipt.chunk.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let Some(text) = text else {
            return Ok(false);
        };
        let batch: RecordingBatch = serde_json::from_str(&text)?;
        ensure!(
            receipt.machine == batch.machine
                && receipt.session == batch.session
                && receipt.stream_epoch == batch.stream_epoch
                && receipt.digest == batch.digest
                && receipt.from == batch.from
                && receipt.to == batch.to
                && receipt.captured_from == batch.captured_from
                && receipt.captured_to == batch.captured_to,
            "recording receipt mismatch"
        );
        self.db.execute(
            "DELETE FROM chunks WHERE id=?1",
            [receipt.chunk.to_string()],
        )?;
        fs::remove_file(self.content_path(batch.id))?;
        Ok(true)
    }

    pub fn acknowledge_gap(
        &mut self,
        receipt: &GapCommitReceipt,
        authenticated_epoch: &str,
    ) -> Result<bool> {
        ensure!(
            receipt.recovery_epoch == authenticated_epoch && !authenticated_epoch.is_empty(),
            "recording recovery epoch mismatch"
        );
        let text: Option<String> = self
            .db
            .query_row(
                "SELECT metadata FROM gaps WHERE id=?1",
                [receipt.gap.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let Some(text) = text else {
            return Ok(false);
        };
        let gap: LossRange = serde_json::from_str(&text)?;
        let identity: RecordingIdentity =
            serde_json::from_str(&self.db.query_row::<String, _, _>(
                "SELECT identity FROM streams WHERE epoch=?1",
                [gap.stream_epoch.to_string()],
                |r| r.get(0),
            )?)?;
        ensure!(
            receipt.machine == identity.machine
                && receipt.session == gap.session
                && receipt.stream_epoch == gap.stream_epoch
                && receipt.digest == gap.receipt_digest()?
                && receipt.from == gap.from
                && receipt.to == gap.to
                && receipt.captured_from == gap.captured_from
                && receipt.captured_to == gap.captured_to,
            "gap receipt mismatch"
        );
        self.db
            .execute("DELETE FROM gaps WHERE id=?1", [receipt.gap.to_string()])?;
        Ok(true)
    }

    pub fn status(&self) -> Result<SpoolStatus> {
        let (chunks, pending_bytes) = self.db.query_row(
            "SELECT count(*),coalesce(sum(bytes),0) FROM chunks",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        let gaps = self
            .db
            .query_row("SELECT count(*) FROM gaps", [], |r| r.get(0))?;
        let warning = self
            .db
            .query_row("SELECT reason FROM warnings", [], |r| r.get(0))
            .optional()?;
        Ok(SpoolStatus {
            pending_bytes,
            chunks,
            gaps,
            warning,
            durability_unknown: self.unknown,
        })
    }

    pub fn seal(&mut self, identity: &RecordingIdentity) -> Result<()> {
        ensure!(
            self.reservations(identity)?.is_empty(),
            "uncommitted recording reservations"
        );
        self.db.execute(
            "UPDATE streams SET sealed=1 WHERE epoch=?1",
            [identity.stream_epoch.to_string()],
        )?;
        Ok(())
    }

    /// Locally observable even when the metadata disk cannot answer a query.
    pub fn durability_unknown(&self) -> bool {
        self.unknown
    }

    pub(super) fn mark_unavailable(&mut self) {
        self.unknown = true;
        let _ = self.db.execute(
            "INSERT OR REPLACE INTO warnings VALUES(1,'recording_durability_unknown')",
            [],
        );
    }

    /// Worker reports lost admissions after draining earlier queued records.
    pub(super) fn capture_loss(
        &mut self,
        identity: &RecordingIdentity,
        count: u64,
        start: Timestamp,
        end: Timestamp,
    ) -> Result<()> {
        self.capture_loss_kind(identity, count, start, end, LossReason::CaptureBackpressure)
    }
    pub(super) fn capture_loss_kind(
        &mut self,
        identity: &RecordingIdentity,
        count: u64,
        start: Timestamp,
        end: Timestamp,
        reason: LossReason,
    ) -> Result<()> {
        ensure!(count > 0, "empty recording capture loss");
        let warning = match &reason {
            LossReason::RecoveryLoss => "recording_recovery_loss",
            _ => "recording_capture_backpressure",
        };
        let result = (|| {
            let tx = self.db.transaction()?;
            Self::identity(&tx, identity)?;
            let pending: u32 = tx.query_row(
                "SELECT count(*) FROM reservations WHERE epoch=?1",
                [identity.stream_epoch.to_string()],
                |r| r.get(0),
            )?;
            ensure!(pending == 0, "capture loss precedes unfinished records");
            let frontier: Vec<u8> = tx.query_row(
                "SELECT frontier FROM streams WHERE epoch=?1 AND sealed=0",
                [identity.stream_epoch.to_string()],
                |r| r.get(0),
            )?;
            let frontier = counter(&frontier)?;
            let from = frontier
                .checked_add(1)
                .context("recording sequence exhausted")?;
            let to = frontier
                .checked_add(count)
                .context("recording sequence exhausted")?;
            let gap = LossRange {
                id: GapId::new(),
                session: identity.session,
                stream: identity.stream.clone(),
                stream_epoch: identity.stream_epoch,
                from: Counter(from),
                to: Counter(to),
                captured_from: start.clone().min(end.clone()),
                captured_to: start.max(end.clone()),
                reason,
            };
            put_gap(&tx, gap)?;
            tx.execute(
                "UPDATE streams SET frontier=?1,captured_at=?2 WHERE epoch=?3",
                params![
                    to.to_be_bytes().as_slice(),
                    end.as_str(),
                    identity.stream_epoch.to_string()
                ],
            )?;
            tx.execute("INSERT OR REPLACE INTO warnings VALUES(1,?1)", [warning])?;
            tx.commit()?;
            Ok(())
        })();
        self.observe_error(result)
    }

    /// After a storage outage, commits conservative loss intervals before normal capture resumes.
    pub fn reconcile(&mut self, now: Timestamp) -> Result<()> {
        let result = self.recover(now).and_then(|()| self.remove_orphans());
        if result.is_ok() {
            self.unknown = false;
        }
        self.observe_error(result)
    }

    fn make_space(&mut self, incoming: u64) -> Result<()> {
        loop {
            let (used, count): (u64, u64) = self.db.query_row(
                "SELECT coalesce(sum(bytes),0),count(*) FROM chunks",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            if used
                .checked_add(incoming)
                .is_some_and(|total| total <= self.config.content_bytes)
                && count < 8192
            {
                return Ok(());
            }
            let text: String = self.db.query_row(
                "SELECT metadata FROM chunks ORDER BY position LIMIT 1",
                [],
                |r| r.get(0),
            )?;
            let batch: RecordingBatch = serde_json::from_str(&text)?;
            let gap = LossRange {
                id: GapId::new(),
                session: batch.session,
                stream: batch.stream.clone(),
                stream_epoch: batch.stream_epoch,
                from: batch.from,
                to: batch.to,
                captured_from: batch.captured_from,
                captured_to: batch.captured_to,
                reason: LossReason::SpoolCap,
            };
            let tx = self.db.transaction()?;
            put_gap(&tx, gap)?;
            tx.execute("DELETE FROM chunks WHERE id=?1", [batch.id.to_string()])?;
            tx.execute(
                "INSERT OR REPLACE INTO warnings VALUES(1,'recording_spool_cap')",
                [],
            )?;
            tx.commit()?; // Loss metadata is durable before deleting any unacknowledged bytes.
            fs::remove_file(self.content_path(batch.id))?;
        }
    }

    fn recover(&mut self, now: Timestamp) -> Result<()> {
        let streams: Vec<_> = {
            let mut query = self
                .db
                .prepare("SELECT identity,frontier,captured_at,sealed FROM streams")?;
            query
                .query_map([], |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, Vec<u8>>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, bool>(3)?,
                    ))
                })?
                .collect::<std::result::Result<_, _>>()?
        };
        for (text, frontier, at, sealed) in streams {
            let identity: RecordingIdentity = serde_json::from_str(&text)?;
            let reservations = self.reservations(&identity)?;
            let frontier = counter(&frontier)?;
            if sealed && reservations.is_empty() {
                continue;
            }
            let first = reservations
                .keys()
                .next()
                .copied()
                .unwrap_or(frontier.saturating_add(1));
            let last = if sealed {
                frontier
            } else {
                frontier.saturating_add(1)
            };
            ensure!(
                first > 0 && last >= first,
                "invalid recording recovery interval"
            );
            let at: Timestamp = at.try_into().map_err(anyhow::Error::msg)?;
            let start = reservations
                .values()
                .chain(std::iter::once(&at))
                .chain(std::iter::once(&now))
                .min()
                .unwrap()
                .clone();
            let end = reservations
                .values()
                .chain(std::iter::once(&at))
                .chain(std::iter::once(&now))
                .max()
                .unwrap()
                .clone();
            let gap = LossRange {
                id: GapId::new(),
                session: identity.session,
                stream: identity.stream,
                stream_epoch: identity.stream_epoch,
                from: Counter(first),
                to: Counter(last),
                captured_from: start,
                captured_to: end,
                reason: LossReason::RecoveryLoss,
            };
            let tx = self.db.transaction()?;
            put_gap(&tx, gap)?;
            tx.execute(
                "DELETE FROM reservations WHERE epoch=?1",
                [identity.stream_epoch.to_string()],
            )?;
            tx.execute(
                "UPDATE streams SET frontier=?1,captured_at=?2 WHERE epoch=?3",
                params![
                    last.to_be_bytes().as_slice(),
                    now.as_str(),
                    identity.stream_epoch.to_string()
                ],
            )?;
            tx.execute(
                "INSERT OR REPLACE INTO warnings VALUES(1,'recording_recovery_loss')",
                [],
            )?;
            tx.commit()?;
        }
        let chunks: Vec<String> = {
            let mut query = self
                .db
                .prepare("SELECT metadata FROM chunks ORDER BY position")?;
            query
                .query_map([], |r| r.get(0))?
                .collect::<std::result::Result<_, _>>()?
        };
        for text in chunks {
            let batch: RecordingBatch = serde_json::from_str(&text)?;
            if self.read_content(&batch).is_err() {
                self.lose_chunk(&batch, LossReason::DiskFailure)?;
            }
        }
        Ok(())
    }

    fn lose_chunk(&mut self, batch: &RecordingBatch, reason: LossReason) -> Result<()> {
        let gap = LossRange {
            id: GapId::new(),
            session: batch.session,
            stream: batch.stream.clone(),
            stream_epoch: batch.stream_epoch,
            from: batch.from,
            to: batch.to,
            captured_from: batch.captured_from.clone(),
            captured_to: batch.captured_to.clone(),
            reason,
        };
        let tx = self.db.transaction()?;
        put_gap(&tx, gap)?;
        tx.execute("DELETE FROM chunks WHERE id=?1", [batch.id.to_string()])?;
        tx.execute(
            "INSERT OR REPLACE INTO warnings VALUES(1,'recording_disk_failure')",
            [],
        )?;
        tx.commit()?;
        match fs::remove_file(self.content_path(batch.id)) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        Ok(())
    }

    fn remove_orphans(&self) -> Result<()> {
        for entry in fs::read_dir(&self.config.directory)? {
            let entry = entry?;
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Some(id) = name
                .strip_suffix(".zst")
                .and_then(|text| text.parse::<qgenie_remote_proto::ChunkId>().ok())
            {
                let exists: bool = self.db.query_row(
                    "SELECT EXISTS(SELECT 1 FROM chunks WHERE id=?1)",
                    [id.to_string()],
                    |r| r.get(0),
                )?;
                if !exists {
                    fs::remove_file(entry.path())?;
                }
            }
        }
        Ok(())
    }

    fn reservations(&self, identity: &RecordingIdentity) -> Result<BTreeMap<u64, Timestamp>> {
        let prior: String = self.db.query_row(
            "SELECT identity FROM streams WHERE epoch=?1",
            [identity.stream_epoch.to_string()],
            |r| r.get(0),
        )?;
        ensure!(
            prior == serde_json::to_string(identity)?,
            "recording scope mismatch"
        );
        let mut query = self
            .db
            .prepare("SELECT seq,captured_at FROM reservations WHERE epoch=?1 ORDER BY seq")?;
        let rows = query.query_map([identity.stream_epoch.to_string()], |r| {
            Ok((r.get::<_, Vec<u8>>(0)?, r.get::<_, String>(1)?))
        })?;
        rows.map(|row| {
            let (seq, time) = row?;
            Ok((counter(&seq)?, time.try_into().map_err(anyhow::Error::msg)?))
        })
        .collect()
    }
    fn identity(tx: &Transaction<'_>, identity: &RecordingIdentity) -> Result<()> {
        let prior: String = tx.query_row(
            "SELECT identity FROM streams WHERE epoch=?1",
            [identity.stream_epoch.to_string()],
            |r| r.get(0),
        )?;
        ensure!(
            prior == serde_json::to_string(identity)?,
            "recording scope mismatch"
        );
        Ok(())
    }
    fn content_path(&self, id: qgenie_remote_proto::ChunkId) -> PathBuf {
        self.config.directory.join(format!("{id}.zst"))
    }
    fn observe_error<T>(&mut self, result: Result<T>) -> Result<T> {
        if result.as_ref().err().is_some_and(|e| {
            e.chain()
                .any(|c| c.is::<std::io::Error>() || c.is::<rusqlite::Error>())
        }) {
            self.unknown = true;
            let _ = self.db.execute(
                "INSERT OR REPLACE INTO warnings VALUES(1,'recording_durability_unknown')",
                [],
            );
        }
        result
    }
}

fn counter(bytes: &[u8]) -> Result<u64> {
    Ok(u64::from_be_bytes(bytes.try_into().map_err(|_| {
        anyhow::anyhow!("invalid recording frontier")
    })?))
}

fn put_gap(tx: &Transaction<'_>, mut gap: LossRange) -> Result<()> {
    gap.validate()?;
    let previous:Option<(String,String)>=tx.query_row("SELECT id,metadata FROM gaps WHERE epoch=?1 AND offered=0 ORDER BY position DESC LIMIT 1",[gap.stream_epoch.to_string()],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
    if let Some((id, text)) = previous {
        let old: LossRange = serde_json::from_str(&text)?;
        if std::mem::discriminant(&old.reason) == std::mem::discriminant(&gap.reason)
            && gap.from.0 <= old.to.0.saturating_add(1)
            && old.from.0 <= gap.to.0.saturating_add(1)
        {
            gap.from = gap.from.min(old.from);
            gap.to = gap.to.max(old.to);
            gap.captured_from = gap.captured_from.min(old.captured_from);
            gap.captured_to = gap.captured_to.max(old.captured_to);
            // Never rewrite an immutable ID. Only unoffered ranges can be superseded atomically.
            tx.execute("DELETE FROM gaps WHERE id=?1", [id])?;
        }
    }
    let count: u32 = tx.query_row("SELECT count(*) FROM gaps", [], |r| r.get(0))?;
    ensure!(count < 4096, "recording loss metadata capacity reached");
    tx.execute(
        "INSERT INTO gaps(id,epoch,metadata) VALUES(?1,?2,?3)",
        params![
            gap.id.to_string(),
            gap.stream_epoch.to_string(),
            serde_json::to_string(&gap)?
        ],
    )?;
    Ok(())
}

fn validate_group(
    identity: &RecordingIdentity,
    batches: &[RecordingBatch],
    reservations: &BTreeMap<u64, Timestamp>,
) -> Result<BTreeSet<u64>> {
    let mut complete = BTreeSet::new();
    let mut current = reservations
        .keys()
        .next()
        .copied()
        .context("no recording reservation")?;
    let mut fragment = 0;
    let mut ids = BTreeSet::new();
    for batch in batches {
        batch.validate()?;
        ensure!(
            ids.insert(batch.id)
                && batch.session == identity.session
                && batch.machine == identity.machine
                && batch.stream_epoch == identity.stream_epoch
                && serde_json::to_string(&batch.stream)?
                    == serde_json::to_string(&identity.stream)?
                && matches!(batch.codec, RecordingCodec::ZstdJsonl),
            "recording batch scope mismatch"
        );
        let compressed = STANDARD.decode(&batch.data)?;
        ensure!(
            format!("{:x}", Sha256::digest(&compressed)) == batch.digest,
            "recording digest mismatch"
        );
        ensure!(
            zstd::zstd_safe::find_frame_compressed_size(&compressed)
                .map_err(|_| anyhow::anyhow!("invalid recording compression frame"))?
                == compressed.len(),
            "recording must contain exactly one compression frame"
        );
        let raw = zstd::bulk::decompress(&compressed, 1024 * 1024)?;
        ensure!(
            raw.len() == batch.uncompressed_bytes as usize && raw.last() == Some(&b'\n'),
            "invalid recording content size"
        );
        let mut first = None;
        let mut last = None;
        let mut times = Vec::new();
        for line in raw.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
            let record: Record = serde_json::from_slice(line)?;
            record.validate(&identity.stream)?;
            ensure!(
                record.v == 1
                    && record.seq.0 == current
                    && reservations.get(&current) == Some(&record.captured_at),
                "recording reservation mismatch"
            );
            first.get_or_insert(record.seq);
            last = Some(record.seq);
            times.push(record.captured_at);
            let done = match record.content {
                Content::Output {
                    data,
                    fragment: part,
                    last,
                }
                | Content::AgentText {
                    data,
                    fragment: part,
                    last,
                    ..
                } => {
                    ensure!(
                        part == fragment && STANDARD.decode(&data)?.len() <= 16 * 1024,
                        "invalid recording fragment"
                    );
                    if last {
                        true
                    } else {
                        fragment = fragment
                            .checked_add(1)
                            .context("recording fragment overflow")?;
                        false
                    }
                }
                Content::Resize { rows, cols } => {
                    ensure!(
                        fragment == 0 && rows > 0 && cols > 0,
                        "invalid recording resize"
                    );
                    true
                }
            };
            if done {
                ensure!(complete.insert(current), "recording sequence repeated");
                current = current.saturating_add(1);
                fragment = 0;
            }
        }
        ensure!(
            first == Some(batch.from)
                && last == Some(batch.to)
                && times.iter().min() == Some(&batch.captured_from)
                && times.iter().max() == Some(&batch.captured_to),
            "recording immutable bounds mismatch"
        );
    }
    ensure!(
        fragment == 0 && !complete.is_empty(),
        "recording group ends within a fragmented record"
    );
    Ok(complete)
}

#[cfg(test)]
mod tests;
