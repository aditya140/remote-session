//! Process ownership survives a failed native protocol connection. Only the
//! owning supervisor's explicit shutdown/drop requests native termination.
use super::{CodexError, NativeEffectAuthorization, endpoint::SocketGuard};
use std::sync::{Arc, Mutex};
use tokio::{
    process::Child,
    sync::{mpsc, oneshot, watch},
    task::JoinHandle,
};

pub(super) struct ProcessOwner {
    pid: u32,
    running: watch::Receiver<bool>,
    stop: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<Result<(), CodexError>>>,
    socket: Arc<Mutex<Option<SocketGuard>>>,
    control: CodexProcessControl,
    exited: watch::Receiver<Option<CodexExit>>,
}
#[derive(Clone, Debug)]
pub struct CodexExit {
    pub code: u32,
    /// First native observation, never the time a later inventory query runs.
    pub observed_at_ms: Option<qgenie_remote_proto::Counter>,
}
struct Terminate {
    authorization: Arc<dyn NativeEffectAuthorization>,
    reply: oneshot::Sender<Result<(), CodexError>>,
}
/// Process operations carry no ownership. Dropping a clone never terminates
/// the native process. Each remote termination needs its own original grant.
#[derive(Clone)]
pub struct CodexProcessControl {
    requests: mpsc::Sender<Terminate>,
}
impl CodexProcessControl {
    pub async fn terminate(
        &self,
        authorization: Arc<dyn NativeEffectAuthorization>,
    ) -> Result<(), CodexError> {
        let (reply, result) = oneshot::channel();
        self.requests
            .try_send(Terminate {
                authorization,
                reply,
            })
            .map_err(|_| CodexError::NotSent)?;
        result.await.unwrap_or(Err(CodexError::Unknown))
    }
}
fn stop_owned_group(child: &mut Child) -> std::io::Result<()> {
    let Some(pid) = child.id() else {
        return Ok(());
    };
    let pid = i32::try_from(pid).map_err(|_| std::io::ErrorKind::InvalidInput)?;
    // SAFETY: launch creates this child as a private process-group leader.
    // Its unreaped Child retains the PID, so the group ID cannot be reused.
    // Never use a saved PID after child.wait() has completed.
    if unsafe { libc::kill(-pid, libc::SIGKILL) } == 0 {
        return Ok(());
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        child.start_kill()
    } else {
        Err(error)
    }
}
impl ProcessOwner {
    pub fn start(mut child: Child, directory: Option<std::fs::File>) -> Result<Self, CodexError> {
        let pid = child.id().ok_or(CodexError::Process)?;
        let (stop, mut stopping) = oneshot::channel();
        let (status, running) = watch::channel(true);
        let (ended, exited) = watch::channel(None);
        let (requests, mut commands) = mpsc::channel::<Terminate>(4);
        let control = CodexProcessControl { requests };
        let socket = Arc::new(Mutex::new(None));
        let held_socket = socket.clone();
        let task = tokio::spawn(async move {
            // Keep socket identity/cleanup ownership until the actual process is
            // reaped. A disconnected driver must not unlink a live owner's IPC.
            let _directory = directory;
            let result = loop {
                tokio::select! {
                    result = child.wait() => break result,
                    _ = &mut stopping => {
                        let _ = stop_owned_group(&mut child);
                        let _ = child.start_kill();
                        // Retain ownership through reap, even if the caller goes away.
                        break child.wait().await;
                    },
                    Some(request) = commands.recv() => {
                        let mut attempted = false;
                        let result = request.authorization.perform(&mut || {
                            attempted = true;
                            stop_owned_group(&mut child).map(|_| 0)
                        });
                        match result {
                            Ok(_) => {
                                let _ = request.reply.send(Ok(()));
                                break child.wait().await;
                            },
                            Err(_) => {
                                let _ = request.reply.send(Err(if attempted { CodexError::Unknown } else { CodexError::NotSent }));
                            },
                        }
                    },
                }
            };
            if let Ok(exit) = &result {
                use std::os::unix::process::ExitStatusExt;
                let code = exit
                    .code()
                    .and_then(|code| u32::try_from(code).ok())
                    .unwrap_or_else(|| {
                        128 + exit
                            .signal()
                            .and_then(|signal| u32::try_from(signal).ok())
                            .unwrap_or(127)
                    });
                let observed_at_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()
                    .and_then(|elapsed| u64::try_from(elapsed.as_millis()).ok())
                    .map(qgenie_remote_proto::Counter);
                ended.send_replace(Some(CodexExit {
                    code,
                    observed_at_ms,
                }));
            }
            status.send_replace(false);
            held_socket.lock().unwrap().take();
            result.map(|_| ()).map_err(|_| CodexError::Process)
        });
        Ok(Self {
            pid,
            running,
            stop: Some(stop),
            task: Some(task),
            socket,
            control,
            exited,
        })
    }
    pub fn pid(&self) -> Option<u32> {
        (*self.running.borrow()).then_some(self.pid)
    }
    pub fn exit(&self) -> Option<CodexExit> {
        self.exited.borrow().clone()
    }
    pub fn liveness(&self) -> watch::Receiver<bool> {
        self.running.clone()
    }
    pub fn control(&self) -> CodexProcessControl {
        self.control.clone()
    }
    pub fn retain_socket(&self, guard: SocketGuard) -> Result<(), CodexError> {
        let mut slot = self.socket.lock().map_err(|_| CodexError::Process)?;
        if !*self.running.borrow() {
            return Err(CodexError::Process);
        }
        *slot = Some(guard);
        Ok(())
    }
    pub fn socket_identity(&self) -> Result<super::endpoint::SocketIdentity, CodexError> {
        self.socket
            .lock()
            .map_err(|_| CodexError::Process)?
            .as_ref()
            .map(SocketGuard::identity)
            .ok_or(CodexError::Process)
    }
    pub async fn shutdown(&mut self) -> Result<(), CodexError> {
        if let Some(stop) = self.stop.take() {
            let _ = stop.send(());
        }
        match self.task.take() {
            Some(task) => task.await.map_err(|_| CodexError::Process)?,
            None => Ok(()),
        }
    }
}
impl Drop for ProcessOwner {
    fn drop(&mut self) {
        // Do not abort the reap worker: it owns native cleanup independently of
        // cancellation of the requesting owner future.
        self.stop.take();
    }
}
