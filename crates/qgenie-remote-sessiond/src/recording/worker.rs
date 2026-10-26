use super::{
    CaptureContent, CapturedRecord, RecordingEncoder, RecordingIdentity, RecordingSpool,
    RedactionRules, SpoolConfig, SpoolItem, SpoolStatus,
};
use anyhow::{Context, Result, bail, ensure};
use qgenie_remote_proto::ipc::RemoteScope;
use qgenie_remote_proto::{
    Counter, RecordingEpochId,
    wire::{CommitReceipt, GapCommitReceipt, RecordingStream, Timestamp},
};
use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

struct Admission {
    last: u64,
    queued: u64,
    at: Timestamp,
    closed: bool,
    finished: bool,
}
struct Capture {
    epoch: RecordingEpochId,
    ordinal: u64,
    at: Timestamp,
    elapsed: Counter,
    content: CaptureContent,
}

/// Clones share a short admission lock. No file/database/compression/network work occurs here.
#[derive(Clone)]
pub struct RecordingCapture {
    identity: RecordingIdentity,
    admission: Arc<Mutex<Admission>>,
    captures: SyncSender<Capture>,
    backpressure: Arc<AtomicBool>,
    worker: thread::Thread,
}
impl RecordingCapture {
    /// False means loss is tracked, not that capture may be disabled. The worker persists a gap.
    pub fn capture(&self, at: Timestamp, elapsed: Counter, content: CaptureContent) -> bool {
        let valid = match (&self.identity.stream, &content) {
            (
                RecordingStream::Pty | RecordingStream::Stdout | RecordingStream::Stderr,
                CaptureContent::Output(bytes),
            ) => bytes.len() <= 32 * 1024,
            (RecordingStream::Events, CaptureContent::AgentText { bytes, .. }) => {
                bytes.len() <= 32 * 1024
            }
            (RecordingStream::Pty, CaptureContent::Resize { rows, cols }) => *rows > 0 && *cols > 0,
            _ => false,
        };
        let Ok(mut admission) = self.admission.lock() else {
            self.backpressure.store(true, Ordering::Release);
            return false;
        };
        if admission.closed {
            return false;
        }
        let Some(ordinal) = admission.last.checked_add(1) else {
            self.backpressure.store(true, Ordering::Release);
            return false;
        };
        admission.last = ordinal;
        admission.at = at.clone();
        if !valid {
            self.backpressure.store(true, Ordering::Release);
            drop(admission);
            self.worker.unpark();
            return false;
        }
        let accepted = match self.captures.try_send(Capture {
            epoch: self.identity.stream_epoch,
            ordinal,
            at,
            elapsed,
            content,
        }) {
            Ok(()) => {
                admission.queued = ordinal;
                true
            }
            Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) => {
                self.backpressure.store(true, Ordering::Release);
                false
            }
        };
        drop(admission);
        self.worker.unpark();
        accepted
    }
    pub fn finish(&self) {
        if let Ok(mut admission) = self.admission.lock() {
            admission.closed = true;
            admission.finished = true;
        }
        self.worker.unpark();
    }
    /// The owner closed capture without observing a complete native EOF. The worker
    /// persists an explicit conservative tail gap before freeing the active stream slot.
    pub fn finish_uncertain(&self, at: Timestamp) {
        if let Ok(mut admission) = self.admission.lock()
            && !admission.closed
        {
            admission.at = at;
            admission.closed = true;
            admission.finished = false;
        }
        self.worker.unpark();
    }
    pub fn backpressure_seen(&self) -> bool {
        self.backpressure.load(Ordering::Acquire)
    }
}

enum Command {
    Reclaim(
        RemoteScope,
        qgenie_remote_proto::ipc::RecordingReceipt,
        String,
        Box<dyn FnOnce() -> Result<()> + Send>,
        SyncSender<Result<bool>>,
    ),
    Register {
        identity: RecordingIdentity,
        rules: RedactionRules,
        at: Timestamp,
        reply: SyncSender<Result<RecordingCapture>>,
    },
    Fetch(SyncSender<Result<Option<SpoolItem>>>),
    Ack(CommitReceipt, String, SyncSender<Result<bool>>),
    GapAck(GapCommitReceipt, String, SyncSender<Result<bool>>),
    Status(SyncSender<Result<SpoolStatus>>),
    ScopedFetch(RemoteScope, SyncSender<Result<Option<SpoolItem>>>),
    ScopedAck(RemoteScope, CommitReceipt, String, SyncSender<Result<bool>>),
    ScopedGapAck(
        RemoteScope,
        GapCommitReceipt,
        String,
        SyncSender<Result<bool>>,
    ),
    ScopedStatus(RemoteScope, SyncSender<Result<SpoolStatus>>),
}
struct Stream {
    capture: RecordingCapture,
    rules: RedactionRules,
    encoder: RecordingEncoder,
    processed: u64,
    at: Timestamp,
    sealed: bool,
}

/// One explicitly owned blocking worker, a 64-record (<=2 MiB input) queue and four RPC slots.
/// Owner shutdown closes admissions, drains accepted records and joins; viewers do not own it.
pub struct RecordingWorker {
    commands: SyncSender<Command>,
    stop: Arc<AtomicBool>,
    unknown: Arc<AtomicBool>,
    warning: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}
impl RecordingWorker {
    pub(crate) fn reclaim(
        &self,
        scope: RemoteScope,
        receipt: qgenie_remote_proto::ipc::RecordingReceipt,
        epoch: String,
        guard: impl FnOnce() -> Result<()> + Send + 'static,
    ) -> Result<bool> {
        self.request(|reply| Command::Reclaim(scope, receipt, epoch, Box::new(guard), reply))
    }
    pub fn spawn(config: SpoolConfig, now: Timestamp) -> Result<Self> {
        let (commands, requests) = mpsc::sync_channel(4);
        let (captures, events) = mpsc::sync_channel(64);
        let (ready, initialized) = mpsc::sync_channel(1);
        let stop = Arc::new(AtomicBool::new(false));
        let unknown = Arc::new(AtomicBool::new(false));
        let warning = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let worker_unknown = unknown.clone();
        let worker_warning = warning.clone();
        let thread = thread::Builder::new()
            .name("qgr-recording".into())
            .spawn(move || {
                let spool = match RecordingSpool::open(config, now) {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = ready.send(Err(e));
                        return;
                    }
                };
                worker_warning.store(
                    spool
                        .status()
                        .map(|s| s.warning.is_some() || s.gaps > 0)
                        .unwrap_or(true),
                    Ordering::Release,
                );
                let _ = ready.send(Ok(()));
                run(
                    spool,
                    captures,
                    events,
                    requests,
                    worker_stop,
                    worker_unknown,
                    worker_warning,
                );
            })?;
        // Durable startup occurs before attaching any live session output hook.
        match initialized.recv() {
            Ok(Ok(())) => Ok(Self {
                commands,
                stop,
                unknown,
                warning,
                thread: Some(thread),
            }),
            Ok(Err(e)) => {
                let _ = thread.join();
                Err(e)
            }
            Err(_) => {
                let _ = thread.join();
                bail!("recording worker startup failed")
            }
        }
    }
    pub fn register(
        &self,
        identity: RecordingIdentity,
        rules: RedactionRules,
        at: Timestamp,
    ) -> Result<RecordingCapture> {
        self.request(|reply| Command::Register {
            identity,
            rules,
            at,
            reply,
        })
    }
    pub fn fetch(&self) -> Result<Option<SpoolItem>> {
        self.request(Command::Fetch)
    }
    pub fn acknowledge(&self, receipt: CommitReceipt, authenticated_epoch: String) -> Result<bool> {
        self.request(|reply| Command::Ack(receipt, authenticated_epoch, reply))
    }
    pub fn acknowledge_gap(
        &self,
        receipt: GapCommitReceipt,
        authenticated_epoch: String,
    ) -> Result<bool> {
        self.request(|reply| Command::GapAck(receipt, authenticated_epoch, reply))
    }
    pub fn status(&self) -> Result<SpoolStatus> {
        self.request(Command::Status)
    }
    pub fn fetch_scoped(&self, scope: RemoteScope) -> Result<Option<SpoolItem>> {
        self.request(|reply| Command::ScopedFetch(scope, reply))
    }
    pub fn acknowledge_scoped(
        &self,
        scope: RemoteScope,
        receipt: CommitReceipt,
        authenticated_epoch: String,
    ) -> Result<bool> {
        self.request(|reply| Command::ScopedAck(scope, receipt, authenticated_epoch, reply))
    }
    pub fn acknowledge_gap_scoped(
        &self,
        scope: RemoteScope,
        receipt: GapCommitReceipt,
        authenticated_epoch: String,
    ) -> Result<bool> {
        self.request(|reply| Command::ScopedGapAck(scope, receipt, authenticated_epoch, reply))
    }
    pub fn status_scoped(&self, scope: RemoteScope) -> Result<SpoolStatus> {
        self.request(|reply| Command::ScopedStatus(scope, reply))
    }
    pub fn durability_unknown(&self) -> bool {
        self.unknown.load(Ordering::Acquire)
    }
    pub fn warning_seen(&self) -> bool {
        self.warning.load(Ordering::Acquire)
    }
    fn request<T>(&self, command: impl FnOnce(SyncSender<Result<T>>) -> Command) -> Result<T> {
        ensure!(
            !self.stop.load(Ordering::Acquire),
            "recording worker stopped"
        );
        let (sender, receiver) = mpsc::sync_channel(1);
        self.commands
            .try_send(command(sender))
            .map_err(|_| anyhow::anyhow!("recording worker busy"))?;
        if let Some(thread) = &self.thread {
            thread.thread().unpark();
        }
        receiver
            .recv_timeout(Duration::from_secs(3))
            .context("recording worker reply unknown")?
    }
    pub fn shutdown(mut self) -> Result<()> {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            thread
                .join()
                .map_err(|_| anyhow::anyhow!("recording worker panicked"))?;
        }
        Ok(())
    }
}
impl Drop for RecordingWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            thread.thread().unpark();
            let _ = thread.join();
        }
    }
}

fn run(
    mut spool: RecordingSpool,
    captures: SyncSender<Capture>,
    events: Receiver<Capture>,
    commands: Receiver<Command>,
    stop: Arc<AtomicBool>,
    unknown: Arc<AtomicBool>,
    warning: Arc<AtomicBool>,
) {
    let mut streams: BTreeMap<RecordingEpochId, Stream> = BTreeMap::new();
    let mut flush = Instant::now();
    let mut retry = Instant::now();
    let mut degraded = false;
    loop {
        for _ in 0..4 {
            let Ok(command) = commands.try_recv() else {
                break;
            };
            match command {
                Command::Reclaim(scope, receipt, epoch, guard, reply) => {
                    let _ = reply.send(spool.reclaim(&scope, &receipt, &epoch, guard));
                }
                Command::Register {
                    identity,
                    rules,
                    at,
                    reply,
                } => {
                    let result: Result<_> = (|| {
                        if let Some(existing) = streams.get(&identity.stream_epoch) {
                            ensure!(
                                serde_json::to_vec(&identity)?
                                    == serde_json::to_vec(&existing.capture.identity)?,
                                "recording identity conflict"
                            );
                            return Ok(existing.capture.clone());
                        }
                        ensure!(
                            !degraded && !stop.load(Ordering::Acquire) && streams.len() < 64,
                            "recording capture registration unavailable"
                        );
                        spool.register(&identity, at.clone())?;
                        let sealed = spool.is_sealed(&identity)?;
                        let capture = RecordingCapture {
                            identity: identity.clone(),
                            admission: Arc::new(Mutex::new(Admission {
                                last: 0,
                                queued: 0,
                                at: at.clone(),
                                closed: sealed,
                                finished: sealed,
                            })),
                            captures: captures.clone(),
                            backpressure: Arc::new(AtomicBool::new(false)),
                            worker: thread::current(),
                        };
                        if !sealed {
                            streams.insert(
                                identity.stream_epoch,
                                Stream {
                                    capture: capture.clone(),
                                    encoder: RecordingEncoder::new(identity, rules.clone()),
                                    rules,
                                    processed: 0,
                                    at,
                                    sealed: false,
                                },
                            );
                        }
                        Ok(capture)
                    })();
                    let _ = reply.send(result);
                }
                Command::Fetch(reply) => {
                    let _ = reply.send(spool.fetch());
                }
                Command::Ack(receipt, epoch, reply) => {
                    let _ = reply.send(spool.acknowledge(&receipt, &epoch));
                }
                Command::GapAck(receipt, epoch, reply) => {
                    let _ = reply.send(spool.acknowledge_gap(&receipt, &epoch));
                }
                Command::Status(reply) => {
                    let _ = reply.send(spool.status());
                }
                Command::ScopedFetch(scope, reply) => {
                    let _ = reply.send(spool.fetch_scoped(&scope));
                }
                Command::ScopedAck(scope, receipt, epoch, reply) => {
                    let _ = reply.send(spool.acknowledge_scoped(&scope, &receipt, &epoch));
                }
                Command::ScopedGapAck(scope, receipt, epoch, reply) => {
                    let _ = reply.send(spool.acknowledge_gap_scoped(&scope, &receipt, &epoch));
                }
                Command::ScopedStatus(scope, reply) => {
                    let _ = reply.send(spool.status_scoped(&scope));
                }
            }
        }
        degraded |= spool.durability_unknown();
        if stop.load(Ordering::Acquire) {
            for stream in streams.values() {
                if let Ok(mut admission) = stream.capture.admission.lock() {
                    admission.closed = true;
                }
            }
        }
        let mut drained = true;
        for _ in 0..64 {
            let Ok(event) = events.try_recv() else {
                break;
            };
            drained = false;
            let Some(stream) = streams.get_mut(&event.epoch) else {
                continue;
            };
            if degraded {
                stream.processed = event.ordinal;
                stream.at = event.at;
                continue;
            }
            let result: Result<_> = (|| {
                fill_loss(&mut spool, stream, event.ordinal - 1, event.at.clone())?;
                let seq = spool.reserve(&stream.capture.identity, event.at.clone())?;
                let mut batches = stream.encoder.push(CapturedRecord {
                    seq,
                    captured_at: event.at.clone(),
                    elapsed_ms: event.elapsed,
                    content: event.content,
                })?;
                if !batches.is_empty() {
                    batches.extend(stream.encoder.flush()?);
                    spool.commit(&stream.capture.identity, &batches)?;
                }
                Ok(())
            })();
            stream.processed = event.ordinal;
            stream.at = event.at;
            if result.is_err() {
                degraded = true;
                spool.mark_unavailable();
            }
        }
        if degraded && retry.elapsed() >= Duration::from_secs(1) {
            retry = Instant::now();
            if let Ok(now) = utc_now()
                && spool.reconcile(now).is_ok()
            {
                for stream in streams.values_mut().filter(|s| !s.sealed) {
                    stream.encoder = RecordingEncoder::new(
                        stream.capture.identity.clone(),
                        stream.rules.clone(),
                    );
                }
                degraded = false;
            }
        }
        if !degraded {
            for stream in streams.values_mut().filter(|s| !s.sealed) {
                let snapshot = stream
                    .capture
                    .admission
                    .lock()
                    .ok()
                    .map(|a| (a.last, a.queued, a.at.clone(), a.closed, a.finished));
                if let Some((last, queued, at, closed, finished)) = snapshot
                    && queued <= stream.processed
                {
                    let result: Result<_> = (|| {
                        fill_loss(&mut spool, stream, last, at.clone())?;
                        if closed {
                            let batches = stream.encoder.finish()?;
                            if !batches.is_empty() {
                                spool.commit(&stream.capture.identity, &batches)?;
                            }
                            if !finished {
                                spool.capture_loss_kind(
                                    &stream.capture.identity,
                                    1,
                                    stream.at.clone(),
                                    at,
                                    qgenie_remote_proto::wire::LossReason::RecoveryLoss,
                                )?;
                            }
                            // If either commit fails, reopen still sees an unsealed stream.
                            spool.seal(&stream.capture.identity)?;
                            stream.sealed = true;
                        } else if flush.elapsed() >= Duration::from_secs(2) {
                            let batches = stream.encoder.flush()?;
                            if !batches.is_empty() {
                                spool.commit(&stream.capture.identity, &batches)?;
                            }
                        }
                        Ok(())
                    })();
                    if result.is_err() {
                        degraded = true;
                        spool.mark_unavailable();
                        break;
                    }
                }
            }
        }
        // Sealing follows queue drain and a durable commit. SQLite retains epoch
        // identity; old capture handles retain their closed admission flag.
        streams.retain(|_, stream| !stream.sealed);
        if flush.elapsed() >= Duration::from_secs(2) {
            if spool
                .status()
                .map(|s| s.warning.is_some() || s.gaps > 0)
                .unwrap_or(true)
            {
                warning.store(true, Ordering::Release);
            }
            flush = Instant::now();
        }
        unknown.store(degraded || spool.durability_unknown(), Ordering::Release);
        if stop.load(Ordering::Acquire) && drained {
            break;
        }
        if drained {
            // Producers leave a wake token after publishing work, including
            // completion with no queued record. Idle time still advances flush
            // and recovery deadlines without polling the empty queues.
            thread::park_timeout(Duration::from_secs(1));
        }
    }
}

fn fill_loss(
    spool: &mut RecordingSpool,
    stream: &mut Stream,
    through: u64,
    at: Timestamp,
) -> Result<()> {
    if through <= stream.processed {
        return Ok(());
    }
    let batches = stream.encoder.finish()?;
    if !batches.is_empty() {
        spool.commit(&stream.capture.identity, &batches)?;
    }
    spool.capture_loss(
        &stream.capture.identity,
        through - stream.processed,
        stream.at.clone(),
        at.clone(),
    )?;
    stream.encoder = RecordingEncoder::new(stream.capture.identity.clone(), stream.rules.clone());
    stream.processed = through;
    stream.at = at;
    Ok(())
}
fn utc_now() -> Result<Timestamp> {
    Timestamp::from_unix_millis(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)?
            .as_millis()
            .try_into()?,
    )
    .map_err(Into::into)
}

#[cfg(test)]
mod tests;
