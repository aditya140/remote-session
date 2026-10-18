use super::*;
use crate::{
    CaptureContent, RecordingCapture, RecordingIdentity, RecordingWorker, RedactionRules,
    SpoolConfig,
};
use qgenie_remote_proto::wire::{RecordingStream, Timestamp};

#[derive(Clone)]
pub struct RecordingSettings {
    pub content_bytes: u64,
    pub metadata_bytes: u64,
    pub redaction: RedactionRules,
}
impl Default for RecordingSettings {
    fn default() -> Self {
        Self {
            content_bytes: 256 * 1024 * 1024,
            metadata_bytes: 16 * 1024 * 1024,
            redaction: RedactionRules::default(),
        }
    }
}
pub(super) enum Worker {
    Empty,
    Starting,
    Ready(Arc<RecordingWorker>),
    Failed(Instant),
}
#[derive(Clone)]
pub(super) struct Capture {
    pub source: RecordingCapture,
    clock: Arc<Mutex<CaptureClock>>,
}
struct CaptureClock {
    began: Duration,
    elapsed: Counter,
    at: Timestamp,
}
impl Capture {
    pub fn last_at(&self) -> Timestamp {
        self.clock
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .at
            .clone()
    }
    pub fn push(&mut self, content: CaptureContent) -> bool {
        // Clones share timing and admission order. Approval RPCs and the native
        // drain hold this only around clocks and nonblocking queue admission.
        let Ok(mut clock) = self.clock.lock() else {
            return false;
        };
        let elapsed = clock::elapsed()
            .ok()
            .and_then(|v| v.checked_sub(clock.began))
            .and_then(|v| u64::try_from(v.as_millis()).ok())
            .map(Counter);
        let timing = elapsed.is_some_and(|v| v >= clock.elapsed);
        if let Some(elapsed) = elapsed.filter(|v| *v >= clock.elapsed) {
            clock.elapsed = elapsed;
        }
        let at = wall_now();
        let wall = at.is_some();
        if let Some(at) = at {
            clock.at = at;
        }
        let admitted = self
            .source
            .capture(clock.at.clone(), clock.elapsed, content);
        admitted && timing && wall
    }
}
/// Before handoff, every error finishes the unlaunched stream. After handoff the native
/// supervisor owns completion; its guard persists a conservative tail if it cannot see EOF.
pub(super) struct StartGuard(Option<RecordingCapture>);
impl StartGuard {
    pub fn new(capture: Option<RecordingCapture>) -> Self {
        Self(capture)
    }
    pub fn handed_off(&mut self) {
        self.0 = None;
    }
}
impl Drop for StartGuard {
    fn drop(&mut self) {
        if let Some(capture) = &self.0 {
            capture.finish();
        }
    }
}
pub(super) struct NativeGuard(Option<(RecordingCapture, Timestamp)>);
impl NativeGuard {
    #[cfg(all(feature = "managed-codex", target_os = "linux"))]
    pub fn from_capture(capture: Option<&Capture>) -> Self {
        Self(capture.map(|capture| (capture.source.clone(), capture.last_at())))
    }
    pub fn new(state: &Session) -> Self {
        Self(
            state
                .recording
                .as_ref()
                .map(|c| (c.source.clone(), c.last_at())),
        )
    }
    pub fn complete(&self) {
        if let Some((capture, _)) = &self.0 {
            capture.finish();
        }
    }
}
impl Drop for NativeGuard {
    fn drop(&mut self) {
        if let Some((capture, fallback)) = &self.0 {
            capture.finish_uncertain(wall_now().unwrap_or_else(|| fallback.clone()));
        }
    }
}
impl Runtime {
    pub(super) fn recording_acknowledge(
        self: &Arc<Self>,
        context: qgenie_remote_proto::ipc::RemoteContext,
        challenge: qgenie_remote_proto::ChallengeId,
        receipt: qgenie_remote_proto::ipc::RecordingReceipt,
    ) -> Result<qgenie_remote_proto::ipc::DispatchFrame, RemoteError> {
        let epoch = lock(&self.authority)?.recording_epoch(&context, challenge)?;
        let worker = self.recording_worker()?;
        let runtime = Arc::downgrade(self);
        let expected = epoch.clone();
        let reclaimed = worker
            .reclaim(context.scope.clone(), receipt, epoch, move || {
                let runtime = runtime
                    .upgrade()
                    .ok_or_else(|| anyhow::anyhow!("native runtime stopped"))?;
                let current = lock(&runtime.authority)?.recording_epoch(&context, challenge)?;
                anyhow::ensure!(
                    current == expected,
                    "recording epoch changed before reclamation"
                );
                Ok(())
            })
            .map_err(storage_error)?;
        Ok(qgenie_remote_proto::ipc::DispatchFrame::RecordingAcknowledged { reclaimed })
    }
    pub(super) fn recording_read(
        &self,
        context: &qgenie_remote_proto::ipc::RemoteContext,
        max_bytes: Option<u32>,
    ) -> Result<qgenie_remote_proto::ipc::DispatchFrame, RemoteError> {
        use qgenie_remote_proto::ipc::{DispatchFrame, RecordingStatus, RecordingUpload};
        lock(&self.authority)?.validate(context, Instant::now())?;
        if max_bytes.is_some_and(|v| v != 64 * 1024) {
            return Err(RemoteError::rejected(
                ErrorCode::InvalidFrame,
                "native recording fetch requires the fixed 64 KiB content budget",
            ));
        }
        // Hold neither authority nor session locks over worker/SQLite/file work. A second
        // original-nonce check after I/O prevents queued reads surviving replacement/expiry.
        let result = self.recording_worker().and_then(|worker| {
            if max_bytes.is_some() {
                worker
                    .fetch_scoped(context.scope.clone())
                    .map(|item| {
                        DispatchFrame::RecordingFetched(item.map(|item| match item {
                            crate::SpoolItem::Content(batch) => RecordingUpload::Content(batch),
                            crate::SpoolItem::Gap(gap) => RecordingUpload::Gap(gap),
                        }))
                    })
                    .map_err(storage_error)
            } else {
                worker
                    .status_scoped(context.scope.clone())
                    .map(|s| {
                        DispatchFrame::RecordingStatusReply(RecordingStatus {
                            pending_bytes: Counter(s.pending_bytes),
                            chunks: Counter(s.chunks),
                            gaps: Counter(s.gaps),
                            warning: s.warning,
                            durability_unknown: s.durability_unknown,
                        })
                    })
                    .map_err(storage_error)
            }
        });
        lock(&self.authority)?.validate(context, Instant::now())?;
        result
    }
    pub(super) fn recording_worker(&self) -> Result<Arc<RecordingWorker>, RemoteError> {
        {
            let mut worker = lock(&self.recording)?;
            match &*worker {
                Worker::Ready(worker) => return Ok(worker.clone()),
                Worker::Starting => {
                    return Err(RemoteError::rejected(
                        ErrorCode::Overloaded,
                        "recording worker is initializing",
                    ));
                }
                Worker::Failed(at) if at.elapsed() < Duration::from_secs(1) => {
                    return Err(RemoteError::rejected(
                        ErrorCode::Overloaded,
                        "recording worker is recovering",
                    ));
                }
                _ => *worker = Worker::Starting,
            }
        }
        let mut config = SpoolConfig::new(self.config.state_dir.join("recordings"));
        config.content_bytes = self.config.recording.content_bytes;
        config.metadata_bytes = self.config.recording.metadata_bytes;
        let result = wall_now()
            .ok_or_else(|| anyhow::anyhow!("recording clock unavailable"))
            .and_then(|at| RecordingWorker::spawn(config, at));
        let mut worker = lock(&self.recording)?;
        match result {
            Ok(value) => {
                let value = Arc::new(value);
                *worker = Worker::Ready(value.clone());
                Ok(value)
            }
            Err(_) => {
                *worker = Worker::Failed(Instant::now());
                Err(RemoteError::rejected(
                    ErrorCode::Overloaded,
                    "recording storage is unavailable before launch",
                ))
            }
        }
    }
    pub(super) fn begin_capture(
        &self,
        info: &SessionInfo,
        scope: &RemoteScope,
        rows: u16,
        cols: u16,
    ) -> Result<Capture, RemoteError> {
        let began = clock::elapsed()?;
        if wall_now().is_none() {
            return Err(RemoteError::rejected(
                ErrorCode::AccessStale,
                "recording capture clock unavailable",
            ));
        }
        let at = Timestamp::from_unix_millis(info.created_at_ms.0)?;
        let identity = RecordingIdentity {
            origin: scope.origin.clone(),
            owner: scope.owner,
            machine: scope.machine,
            session: info.id,
            stream: if info.agent.is_some() {
                RecordingStream::Events
            } else {
                RecordingStream::Pty
            },
            stream_epoch: RecordingEpochId::new(),
        };
        let source = self
            .recording_worker()?
            .register(
                identity,
                self.config.recording.redaction.clone(),
                at.clone(),
            )
            .map_err(storage_error)?;
        let mut capture = Capture {
            source,
            clock: Arc::new(Mutex::new(CaptureClock {
                began,
                elapsed: Counter(0),
                at,
            })),
        };
        if info.agent.is_none() {
            capture.push(CaptureContent::Resize { rows, cols });
        }
        Ok(capture)
    }
    pub(super) fn record(state: &mut Session, content: CaptureContent) {
        if let Some(capture) = &mut state.recording
            && !capture.push(content)
        {
            state.info.recording_warning.get_or_insert_with(|| {
                "Recording capture or timing is degraded; inspect recording status and gaps".into()
            });
        }
    }
    pub(super) fn recording_warning(&self, state: &mut Session) {
        if state.scope.is_some() {
            let unknown = lock(&self.recording).is_ok_and(|worker| match &*worker {
                Worker::Ready(worker) => worker.durability_unknown() || worker.warning_seen(),
                Worker::Starting | Worker::Failed(_) => true,
                Worker::Empty => false,
            });
            if unknown
                || state
                    .recording
                    .as_ref()
                    .is_some_and(|capture| capture.source.backpressure_seen())
            {
                state.info.recording_warning.get_or_insert_with(|| {
                    "Recording loss or durability warning on this device; live session remains available".into()
                });
            }
        }
    }
}

pub(super) fn wall_now() -> Option<Timestamp> {
    Timestamp::from_unix_millis(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()?
            .as_millis()
            .try_into()
            .ok()?,
    )
    .ok()
}

#[cfg(test)]
mod tests;
