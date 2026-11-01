//! Private Unix WebSocket app-server integration; native protocol structures stay in this module.
mod authorization;
mod driver;
pub use authorization::NativeEffectAuthorization;
mod endpoint;
mod normalize;
mod owner;
#[cfg(target_os = "linux")]
mod process_tree;
use crate::LaunchEnvironment;
pub use owner::{CodexExit, CodexProcessControl};
use qgenie_remote_proto::{
    ApplicationOutcome,
    wire::{AgentEvent, ApprovalDecision, Capabilities},
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::AsyncReadExt,
    process::Command,
    sync::{Semaphore, mpsc, oneshot, watch},
    task::JoinHandle,
    time::timeout,
};

pub const PINNED_CODEX_VERSION: &str = "0.159.2";
#[derive(Debug, Clone, thiserror::Error)]
pub enum CodexError {
    #[error("invalid managed Codex configuration")]
    Configuration,
    #[error("Codex binary does not match pinned version 0.159.2")]
    Version,
    #[error("native Codex process or pipe failed")]
    Process,
    #[error("native request queue full; operation not sent")]
    NotSent,
    #[error("native outcome unknown; reconcile existing native identity")]
    Unknown,
    #[error("native request rejected (code {0})")]
    Rejected(i64),
    #[error("native protocol frame invalid or oversized")]
    Protocol,
    #[error("native event consumer exceeded its bounded queue")]
    SlowConsumer,
    #[error("native relay is not confirmed disabled")]
    Relay,
    #[error("unsupported native operation or approval decision")]
    Unsupported,
}
/// Endpoint/credentials are injected by the host composition, never public protocol.
#[derive(Clone)]
pub struct Provider {
    endpoint: String,
    model: String,
    credential: String,
    allow_loopback: bool,
}
impl std::fmt::Debug for Provider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Provider([redacted])")
    }
}
impl Provider {
    pub fn new(endpoint: String, model: String, credential: String) -> Self {
        Self {
            endpoint,
            model,
            credential,
            allow_loopback: false,
        }
    }
    /// Bounded display metadata. This does not disclose endpoint or credential fields
    /// and does not establish which model the provider ultimately serves.
    pub fn requested_model(&self) -> Option<&str> {
        (!self.model.is_empty()
            && self.model.len() <= 128
            && self.model.bytes().all(|b| {
                b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/' | b':' | b'@')
            }))
        .then_some(self.model.as_str())
    }
    pub fn allow_loopback_fixture(mut self, allow: bool) -> Self {
        self.allow_loopback = allow;
        self
    }
    fn validate(&self) -> Result<(), CodexError> {
        let url = url::Url::parse(&self.endpoint).map_err(|_| CodexError::Configuration)?;
        let loopback = url.host_str().is_some_and(|host| {
            host.trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
        });
        if !(url.scheme() == "https" || self.allow_loopback && url.scheme() == "http" && loopback)
            || !url.username().is_empty()
            || url.password().is_some()
            || url.fragment().is_some()
            || url.query().is_some()
            || self.endpoint.len() > 4096
            || self.model.is_empty()
            || self.model.len() > 128
            || self.credential.is_empty()
            || self.credential.len() > 16384
        {
            return Err(CodexError::Configuration);
        }
        Ok(())
    }
}
pub struct CodexConfig {
    /// Linux owned-supervisor mode: the spawning thread must live through reap.
    pub kill_on_owner_thread_exit: bool,
    pub binary: PathBuf,
    pub home: PathBuf,
    pub cwd: PathBuf,
    pub provider: Provider,
    pub environment: LaunchEnvironment,
}
#[derive(Clone)]
pub struct NativeApproval {
    pub native_id: String,
    pub thread: String,
    pub item: String,
    pub kind: String,
    pub summary: String,
    pub allowed_decisions: Vec<ApprovalDecision>,
}
#[derive(Clone)]
pub enum NativeEvent {
    Event {
        thread: String,
        event: AgentEvent,
    },
    Approval(NativeApproval),
    /// Resolution alone does not prove execution; it can represent cancellation.
    ApprovalResolved {
        native_id: String,
        thread: String,
    },
}
impl std::fmt::Debug for NativeApproval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeApproval")
            .field("kind", &self.kind)
            .field("summary", &"[redacted]")
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for NativeEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Event { .. } => "NativeEvent::Event([content])",
            Self::Approval(_) => "NativeEvent::Approval([content])",
            Self::ApprovalResolved { .. } => "NativeEvent::ApprovalResolved",
        })
    }
}
struct Work {
    body: Value,
    done: oneshot::Sender<Result<Value, CodexError>>,
    _bytes: tokio::sync::OwnedSemaphorePermit,
    authorization: Option<Arc<dyn NativeEffectAuthorization>>,
}
/// Cloning a request handle grants no process ownership. The native owner must
/// independently drain events even while requests or persistence are waiting.
#[derive(Clone)]
pub struct CodexHandle {
    work: mpsc::Sender<Work>,
    budget: Arc<Semaphore>,
    authorization: Option<Arc<dyn NativeEffectAuthorization>>,
}
pub struct Codex {
    handle: CodexHandle,
    events: mpsc::Receiver<NativeEvent>,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<Result<(), CodexError>>>,
    process: owner::ProcessOwner,
}
struct DriverParts {
    handle: CodexHandle,
    events: mpsc::Receiver<NativeEvent>,
    stop: oneshot::Sender<()>,
    task: JoinHandle<Result<(), CodexError>>,
    observed: watch::Receiver<bool>,
}
impl DriverParts {
    fn start(
        socket: tokio_tungstenite::WebSocketStream<authorization::NativeStream>,
        running: watch::Receiver<bool>,
        scope: authorization::WriteScope,
    ) -> Self {
        let (work, requests) = mpsc::channel(32);
        let (output, events) = mpsc::channel(64);
        let (stop, stopping) = oneshot::channel();
        let (relay, observed) = watch::channel(false);
        let task = tokio::spawn(driver::run(
            socket, requests, output, stopping, relay, running, scope,
        ));
        Self {
            handle: CodexHandle {
                work,
                budget: Arc::new(Semaphore::new(256 * 1024)),
                authorization: None,
            },
            events,
            stop,
            task,
            observed,
        }
    }
}
impl std::ops::Deref for Codex {
    type Target = CodexHandle;
    fn deref(&self) -> &Self::Target {
        &self.handle
    }
}
impl Codex {
    pub async fn launch(config: CodexConfig) -> Result<Self, CodexError> {
        Self::launch_owned(config, None, None).await
    }
    /// Launch against the retained directory inode. Linux's qualified /proc
    /// descriptor path stays owned until native process reap, including canceled
    /// startup or native IPC failure. Root/policy checks belong to the caller.
    #[cfg(target_os = "linux")]
    pub async fn launch_at(
        mut config: CodexConfig,
        directory: std::fs::File,
    ) -> Result<Self, CodexError> {
        use std::os::fd::AsRawFd;
        if !directory
            .metadata()
            .map_err(|_| CodexError::Configuration)?
            .is_dir()
        {
            return Err(CodexError::Configuration);
        }
        config.cwd = PathBuf::from(format!(
            "/proc/{}/fd/{}",
            std::process::id(),
            directory.as_raw_fd()
        ));
        Self::launch_owned(config, Some(directory), None).await
    }
    /// A remote owner reserves its supervisor/journal first, then retains this
    /// original grant through both version-probe and native process creation.
    #[cfg(target_os = "linux")]
    pub async fn launch_at_authorized(
        mut config: CodexConfig,
        directory: std::fs::File,
        authorization: Arc<dyn NativeEffectAuthorization>,
    ) -> Result<Self, CodexError> {
        use std::os::fd::AsRawFd;
        if !directory
            .metadata()
            .map_err(|_| CodexError::Configuration)?
            .is_dir()
        {
            return Err(CodexError::Configuration);
        }
        config.cwd = PathBuf::from(format!(
            "/proc/{}/fd/{}",
            std::process::id(),
            directory.as_raw_fd()
        ));
        Self::launch_owned(config, Some(directory), Some(authorization)).await
    }
    async fn launch_owned(
        config: CodexConfig,
        directory: Option<std::fs::File>,
        authorization: Option<Arc<dyn NativeEffectAuthorization>>,
    ) -> Result<Self, CodexError> {
        config.provider.validate()?;
        if !config.cwd.is_absolute() || !config.cwd.is_dir() {
            return Err(CodexError::Configuration);
        }
        private_home(&config.home)?;
        verify_binary(
            &config.binary,
            authorization.as_ref(),
            config.kill_on_owner_thread_exit,
        )
        .await?;
        let mut command = Command::new(config.binary);
        command.args([
            "--strict-config",
            "--disable",
            "remote_control",
            "--disable",
            "daemon_auto_start",
        ]);
        for (key, value) in [
            ("model_provider", json!("qgenie")),
            ("model", json!(config.provider.model)),
            ("model_providers.qgenie.name", json!("QGenie")),
            (
                "model_providers.qgenie.base_url",
                json!(config.provider.endpoint),
            ),
            (
                "model_providers.qgenie.env_key",
                json!("QGENIE_REMOTE_CODEX_KEY"),
            ),
            ("model_providers.qgenie.wire_api", json!("responses")),
        ] {
            command.arg("-c").arg(format!("{key}={value}"));
        }
        let socket_path = endpoint::socket_path(&config.home)?;
        command
            .args(["app-server", "--listen"])
            .arg(format!(
                "unix://{}",
                socket_path.to_str().ok_or(CodexError::Configuration)?
            ))
            .current_dir(config.cwd)
            .env("CODEX_HOME", config.home)
            .env("QGENIE_REMOTE_CODEX_KEY", config.provider.credential)
            .env("CODEX_INTERNAL_APP_SERVER_REMOTE_CONTROL_DISABLED", "1");
        for (name, value) in config.environment.0 {
            if [
                "CODEX_HOME",
                "QGENIE_REMOTE_CODEX_KEY",
                "CODEX_INTERNAL_APP_SERVER_REMOTE_CONTROL_DISABLED",
            ]
            .contains(&name.as_str())
            {
                return Err(CodexError::Configuration);
            }
            command.env(name, value);
        }
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .process_group(0);
        bind_owner_death(&mut command, config.kill_on_owner_thread_exit);
        let child = spawn_authorized(&mut command, authorization.as_ref())?;
        let mut process = owner::ProcessOwner::start(child, directory)?;
        let scope = authorization::WriteScope::default();
        let connected = endpoint::connect_owned(
            &socket_path,
            process.pid().ok_or(CodexError::Process)?,
            process.liveness(),
            scope.clone(),
        )
        .await;
        let (socket, guard) = match connected {
            Ok(connected) => connected,
            Err(error) => {
                let _ = process.shutdown().await;
                return Err(error);
            }
        };
        if let Err(error) = process.retain_socket(guard) {
            let _ = process.shutdown().await;
            return Err(error);
        }
        let DriverParts {
            handle,
            events,
            stop,
            task,
            observed,
        } = DriverParts::start(socket, process.liveness(), scope);
        let adapter = Self {
            handle,
            events,
            stop: Some(stop),
            task: Some(task),
            process,
        };
        if let Err(error) = adapter.initialize(observed).await {
            let _ = adapter.shutdown().await;
            return Err(error);
        }
        Ok(adapter)
    }
    async fn initialize(&self, mut observed: watch::Receiver<bool>) -> Result<(), CodexError> {
        self.request("initialize",json!({"clientInfo":{"name":"qgenie_remote","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":false}})).await?;
        self.notify("initialized", json!({})).await?;
        timeout(Duration::from_secs(5), async {
            while !*observed.borrow_and_update() {
                observed.changed().await.map_err(|_| CodexError::Relay)?;
            }
            Ok::<_, CodexError>(())
        })
        .await
        .map_err(|_| CodexError::Relay)?
    }
    /// Explicitly reconnect this exact still-owned endpoint after IPC closure.
    /// Old handles stay closed; no mutation or event timeline is replayed. The
    /// supervisor must record a gap, reconcile saved thread IDs and bind fresh
    /// per-operation authority before delivering another native mutation.
    pub async fn reconnect(&mut self) -> Result<(), CodexError> {
        if self.task.as_ref().is_some_and(|task| !task.is_finished()) {
            return Err(CodexError::NotSent);
        }
        if self.task.is_some() {
            let _ = self.closed().await;
        }
        let identity = self.process.socket_identity()?;
        let scope = authorization::WriteScope::default();
        let socket = endpoint::connect_retained(
            &identity,
            self.process.pid().ok_or(CodexError::Process)?,
            self.process.liveness(),
            scope.clone(),
        )
        .await?;
        let DriverParts {
            handle,
            events,
            stop,
            task,
            observed,
        } = DriverParts::start(socket, self.process.liveness(), scope);
        self.handle = handle;
        self.events = events;
        self.stop = Some(stop);
        self.task = Some(task);
        let result = self.initialize(observed).await;
        if result.is_err() {
            let _ = self.disconnect().await;
        }
        result
    }
    pub fn handle(&self) -> CodexHandle {
        self.handle.clone()
    }
    /// Local diagnostic identity of this exact owned process, never a PID read
    /// from disk or authority to adopt/signal an unrelated process.
    pub fn native_process_id(&self) -> Option<u32> {
        self.process.pid()
    }
    pub fn process_control(&self) -> CodexProcessControl {
        self.process.control()
    }
    pub fn native_exit(&self) -> Option<CodexExit> {
        self.process.exit()
    }
}
fn spawn_authorized(
    command: &mut Command,
    authorization: Option<&Arc<dyn NativeEffectAuthorization>>,
) -> Result<tokio::process::Child, CodexError> {
    let mut child = None;
    let mut attempted = false;
    let mut effect = || {
        attempted = true;
        child = Some(command.spawn()?);
        Ok(0)
    };
    let result = match authorization {
        Some(authorization) => authorization.perform(&mut effect),
        None => effect(),
    };
    // Once spawn succeeds, always hand the exact child to the reap owner; an
    // authorization implementation cannot discard ownership after its effect.
    match child {
        Some(child) => Ok(child),
        None => {
            result.map_err(|_| {
                if attempted {
                    CodexError::Process
                } else {
                    CodexError::NotSent
                }
            })?;
            Err(CodexError::Process)
        }
    }
}
impl CodexHandle {
    /// Retain this exact authorization through each queued request and actual
    /// native write. Domain scope, controller and policy checks remain required.
    pub fn authorized(&self, authorization: Arc<dyn NativeEffectAuthorization>) -> Self {
        Self {
            work: self.work.clone(),
            budget: self.budget.clone(),
            authorization: Some(authorization),
        }
    }
    pub fn capabilities(&self) -> Capabilities {
        Capabilities {
            input: true,
            steer: true,
            interrupt: true,
            terminate: true,
            resume: true,
            approvals: ["command".into()].into(),
            ..Capabilities::default()
        }
    }
    async fn submit(&self, body: Value) -> Result<Value, CodexError> {
        let size = serde_json::to_vec(&body)
            .map_err(|_| CodexError::Protocol)?
            .len();
        if size > 64 * 1024 {
            return Err(CodexError::NotSent);
        }
        let permit = self
            .budget
            .clone()
            .try_acquire_many_owned(size as u32)
            .map_err(|_| CodexError::NotSent)?;
        let (done, result) = oneshot::channel();
        self.work
            .try_send(Work {
                body,
                done,
                _bytes: permit,
                authorization: self.authorization.clone(),
            })
            .map_err(|_| CodexError::NotSent)?;
        result.await.unwrap_or(Err(CodexError::Unknown))
    }
    async fn request(&self, method: &str, params: Value) -> Result<Value, CodexError> {
        self.submit(json!({"method":method,"params":params,"id":0}))
            .await
    }
    async fn notify(&self, method: &str, params: Value) -> Result<(), CodexError> {
        self.submit(json!({"method":method,"params":params}))
            .await
            .map(|_| ())
    }
    pub async fn start_thread(&self, cwd: &Path) -> Result<String, CodexError> {
        if !cwd.is_absolute() {
            return Err(CodexError::Configuration);
        }
        id(
            self.request(
                "thread/start",
                json!({"cwd":cwd,"approvalPolicy":"untrusted","sandbox":"workspace-write"}),
            )
            .await?,
            "thread",
        )
    }
    pub async fn resume_thread(&self, thread: &str) -> Result<String, CodexError> {
        native_id(thread)?;
        id(self.request("thread/resume",json!({"threadId":thread,"excludeTurns":true,"approvalPolicy":"untrusted","sandbox":"workspace-write"})).await?,"thread")
    }
    /// Resume owned history under a freshly retained working-directory descriptor.
    pub async fn resume_thread_at(&self, thread: &str, cwd: &Path) -> Result<String, CodexError> {
        native_id(thread)?;
        if !cwd.is_absolute() {
            return Err(CodexError::Configuration);
        }
        id(self.request("thread/resume",json!({"threadId":thread,"cwd":cwd,"excludeTurns":true,"approvalPolicy":"untrusted","sandbox":"workspace-write"})).await?,"thread")
    }
    pub async fn prompt(&self, thread: &str, text: &str) -> Result<String, CodexError> {
        native_id(thread)?;
        id(
            self.request(
                "turn/start",
                json!({"threadId":thread,"input":[{"type":"text","text":text,"text_elements":[]}]}),
            )
            .await?,
            "turn",
        )
    }
    pub async fn steer(&self, thread: &str, turn: &str, text: &str) -> Result<(), CodexError> {
        native_id(thread)?;
        native_id(turn)?;
        self.request("turn/steer",json!({"threadId":thread,"expectedTurnId":turn,"input":[{"type":"text","text":text,"text_elements":[]}]})).await.map(|_| ())
    }
    pub async fn interrupt(&self, thread: &str, turn: &str) -> Result<(), CodexError> {
        native_id(thread)?;
        native_id(turn)?;
        self.request("turn/interrupt", json!({"threadId":thread,"turnId":turn}))
            .await
            .map(|_| ())
    }
    /// Writes a response once. The native protocol provides no durable decision-ID ack.
    pub async fn answer(
        &self,
        approval: &NativeApproval,
        decision: ApprovalDecision,
    ) -> Result<ApplicationOutcome, CodexError> {
        let value = driver::decision(approval, &decision)?;
        let native: Value =
            serde_json::from_str(&approval.native_id).map_err(|_| CodexError::Protocol)?;
        let reply = self
            .submit(json!({"id":native,"result":{"decision":value}}))
            .await?;
        Ok(if reply["application"] == "cancelled" {
            ApplicationOutcome::Cancelled
        } else {
            ApplicationOutcome::OutcomeUnknown
        })
    }
}
impl Codex {
    pub async fn recv(&mut self) -> Option<NativeEvent> {
        self.events.recv().await
    }
    pub async fn shutdown(mut self) -> Result<(), CodexError> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        let driver = if self.task.is_some() {
            self.closed().await
        } else {
            Ok(())
        };
        let process = self.process.shutdown().await;
        driver.and(process)
    }
    /// Close native IPC while retaining the owned process. Pending mutations
    /// remain uncertain; callers must retain native IDs and record a replay gap.
    /// Reconnection/adoption is not implicitly attempted.
    pub async fn disconnect(&mut self) -> Result<(), CodexError> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        self.closed().await
    }
    /// Wait for native protocol closure. This does not terminate the process;
    /// the owning supervisor must still explicitly shut down or retain it.
    pub async fn closed(&mut self) -> Result<(), CodexError> {
        let result = self
            .task
            .as_mut()
            .ok_or(CodexError::Process)?
            .await
            .map_err(|_| CodexError::Process)?;
        self.task.take();
        result
    }
}
impl Drop for Codex {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
fn native_id(id: &str) -> Result<(), CodexError> {
    if id.is_empty() || id.len() > 256 {
        Err(CodexError::Protocol)
    } else {
        Ok(())
    }
}
fn id(result: Value, kind: &str) -> Result<String, CodexError> {
    let id = result[kind]["id"].as_str().ok_or(CodexError::Protocol)?;
    native_id(id)?;
    Ok(id.into())
}
fn private_home(path: &Path) -> Result<(), CodexError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, MetadataExt};
        if !path.is_absolute() {
            return Err(CodexError::Configuration);
        }
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(path)
            .map_err(|_| CodexError::Configuration)?;
        let metadata = std::fs::symlink_metadata(path).map_err(|_| CodexError::Configuration)?;
        // SAFETY: geteuid has no arguments or preconditions.
        if !metadata.is_dir()
            || metadata.mode() & 0o077 != 0
            || metadata.uid() != unsafe { libc::geteuid() }
        {
            return Err(CodexError::Configuration);
        }
        let canonical = std::fs::canonicalize(path).map_err(|_| CodexError::Configuration)?;
        for ancestor in canonical.ancestors().skip(1) {
            let metadata = std::fs::metadata(ancestor).map_err(|_| CodexError::Configuration)?;
            if metadata.mode() & 0o022 != 0 && metadata.mode() & 0o1000 == 0 {
                return Err(CodexError::Configuration);
            }
        }
        Ok(())
    }
    #[cfg(not(unix))]
    {
        let _ = path;
        Err(CodexError::Unsupported)
    }
}

impl Codex {
    /// Trusted startup discovery. Launch repeats this check under its original grant.
    pub async fn verify_version(binary: &Path) -> Result<&'static str, CodexError> {
        verify_binary(binary, None, false).await?;
        Ok(PINNED_CODEX_VERSION)
    }
}
async fn verify_binary(
    binary: &Path,
    authorization: Option<&Arc<dyn NativeEffectAuthorization>>,
    owner_death: bool,
) -> Result<(), CodexError> {
    let mut version_command = Command::new(binary);
    bind_owner_death(&mut version_command, owner_death);
    version_command
        .arg("--version")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let mut version = spawn_authorized(&mut version_command, authorization)?;
    let mut bytes = Vec::new();
    let probe = async {
        timeout(
            Duration::from_secs(5),
            version
                .stdout
                .take()
                .ok_or(CodexError::Process)?
                .take(256)
                .read_to_end(&mut bytes),
        )
        .await
        .map_err(|_| CodexError::Version)?
        .map_err(|_| CodexError::Version)?;
        let status = timeout(Duration::from_secs(5), version.wait())
            .await
            .map_err(|_| CodexError::Version)?
            .map_err(|_| CodexError::Version)?;
        if !status.success() || bytes != format!("codex-cli {PINNED_CODEX_VERSION}\n").as_bytes() {
            return Err(CodexError::Version);
        }
        Ok::<_, CodexError>(())
    }
    .await;
    if let Err(error) = probe {
        // kill_on_drop alone does not guarantee reap before a caller closes
        // its executor. Failed startup retains ownership through wait.
        let _ = version.start_kill();
        let _ = version.wait().await;
        return Err(error);
    }
    Ok(())
}
fn bind_owner_death(command: &mut Command, enabled: bool) {
    #[cfg(target_os = "linux")]
    if enabled {
        let parent = std::process::id() as libc::pid_t;
        // SAFETY: only async-signal-safe kernel calls occur between fork/exec.
        // The dedicated managed owner thread remains alive through native reap.
        unsafe {
            command.pre_exec(move || {
                if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::getppid() != parent {
                    return Err(std::io::Error::from_raw_os_error(libc::ESRCH));
                }
                Ok(())
            });
        }
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (command, enabled);
}

#[cfg(test)]
mod requested_model_metadata_tests {
    use super::Provider;
    #[test]
    fn configured_model_metadata_does_not_expose_connection_secrets() {
        let provider = Provider::new(
            "https://private-endpoint.example/v1".into(),
            "vendor/model-v2:alias".into(),
            "secret-credential".into(),
        );
        assert_eq!(provider.requested_model(), Some("vendor/model-v2:alias"));
        let serialized = serde_json::to_string(&provider.requested_model()).unwrap();
        assert!(!serialized.contains("private-endpoint"));
        assert!(!serialized.contains("secret-credential"));
        for model in ["".to_string(), "x".repeat(129), "unsafe\nvalue".to_string()] {
            assert!(
                Provider::new("https://example.test".into(), model, "key".into())
                    .requested_model()
                    .is_none()
            );
        }
    }
}
