//! An owned read-only output subscription. Reconnection and all mutations stay
//! explicit; periodic continuation reads also discover an otherwise lost tail.
use crate::{GatewayClient, LiveClient, LiveHandle, OutputCursor, OutputUpdate, ReadPlan};
use qgenie_remote_proto::{rest::AccessToken, *};
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, watch},
    task::{JoinHandle, JoinSet},
    time::Instant,
};

const BYTES: usize = 16 * 1024 * 1024;
pub struct ObservedOutput {
    pub update: OutputUpdate,
    _bytes: OwnedSemaphorePermit,
}
impl std::fmt::Debug for ObservedOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObservedOutput")
            .field("snapshot", &self.update.snapshot.is_some())
            .field("events", &self.update.events.len())
            .finish()
    }
}
pub struct RemoteObserver {
    handle: LiveHandle,
    events: mpsc::Receiver<ObservedOutput>,
    failure: Arc<Mutex<Option<RemoteError>>>,
    stop: watch::Sender<bool>,
    task: Option<JoinHandle<()>>,
    error_reported: bool,
}
impl GatewayClient {
    /// Own one bounded WSS output subscription. Dropping it leaves native execution
    /// running; a new observer installs a fresh checkpoint and never restores input.
    pub async fn observe_session(
        &self,
        token: &AccessToken,
        machine: MachineId,
        session: SessionId,
    ) -> Result<RemoteObserver, RemoteError> {
        Ok(self.connect(token).await?.into_observer(machine, session))
    }
}
impl LiveClient {
    /// Transfer the connection to one output pump. Its handle remains available
    /// for explicit owner/controller operations and caller-supplied token refresh.
    pub fn into_observer(self, machine: MachineId, session: SessionId) -> RemoteObserver {
        let handle = self.handle();
        let (sender, events) = mpsc::channel(8);
        let (stop, cancel) = watch::channel(false);
        let failure = Arc::new(Mutex::new(None));
        let failed = failure.clone();
        let task = tokio::spawn(async move {
            let error = follow(self, machine, session, sender, cancel).await.err();
            *failed.lock().unwrap() = error;
        });
        RemoteObserver {
            handle,
            events,
            failure,
            stop,
            task: Some(task),
            error_reported: false,
        }
    }
}
impl RemoteObserver {
    pub fn handle(&self) -> LiveHandle {
        self.handle.clone()
    }
    pub fn failure(&self) -> Option<RemoteError> {
        self.failure.lock().ok().and_then(|failure| failure.clone())
    }
    pub async fn next(&mut self) -> Option<Result<ObservedOutput, RemoteError>> {
        if self.error_reported {
            return None;
        }
        if let Some(event) = self.events.recv().await {
            return Some(Ok(event));
        }
        // The final error cannot be lost behind a full subscriber queue.
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
        self.error_reported = true;
        self.failure().map(Err)
    }
    pub async fn close(mut self) {
        self.stop.send_replace(true);
        if let Some(task) = self.task.take() {
            let _ = task.await;
        }
    }
}
impl Drop for RemoteObserver {
    fn drop(&mut self) {
        self.stop.send_replace(true);
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}
fn error(code: ErrorCode, message: &str) -> RemoteError {
    RemoteError::rejected(code, message)
}
fn deliver(
    sender: &mpsc::Sender<ObservedOutput>,
    budget: &Arc<Semaphore>,
    update: OutputUpdate,
) -> Result<(), RemoteError> {
    if update.snapshot.is_none() && update.events.is_empty() {
        return Ok(());
    }
    let mut bytes = update
        .snapshot
        .as_ref()
        .map_or(0, |snapshot| snapshot.data.len() + 512);
    for event in &update.events {
        bytes = bytes.saturating_add(
            serde_json::to_vec(&event.event)
                .map_err(|_| error(ErrorCode::InvalidFrame, "output cannot be measured"))?
                .len()
                + 128,
        );
    }
    let count: u32 = bytes
        .try_into()
        .map_err(|_| error(ErrorCode::Overloaded, "output exceeds subscription budget"))?;
    let permit = budget.clone().try_acquire_many_owned(count).map_err(|_| {
        error(
            ErrorCode::Overloaded,
            "output subscriber exceeded its byte budget; reopen with a checkpoint",
        )
    })?;
    sender
        .try_send(ObservedOutput {
            update,
            _bytes: permit,
        })
        .map_err(|_| {
            error(
                ErrorCode::Overloaded,
                "output subscriber stalled; reopen with a checkpoint",
            )
        })
}
async fn follow(
    mut client: LiveClient,
    machine: MachineId,
    session: SessionId,
    sender: mpsc::Sender<ObservedOutput>,
    mut cancel: watch::Receiver<bool>,
) -> Result<(), RemoteError> {
    let handle = client.handle();
    let mut output = OutputCursor::checkpointed(machine, session);
    let mut reads = JoinSet::new();
    let budget = Arc::new(Semaphore::new(BYTES));
    let mut next = Instant::now();
    let mut connected = true;
    let result = loop {
        tokio::select! {biased;
            _=cancel.changed()=>break Ok(()),
            _=sender.closed()=>break Ok(()),
        _=async {},if !connected && reads.is_empty()=>break Err(error(ErrorCode::AccessStale,"output connection ended; reconnect explicitly with current credentials")),
        event=client.next(),if connected=>{
            let Some(event)=event else{connected=false;continue;};
                match output.frame(&event.frame).and_then(|update|deliver(&sender,&budget,update)) {Ok(())=>{},Err(error)=>break Err(error)}
                if !matches!(output.read_plan(),ReadPlan::Idle|ReadPlan::UnconfirmedTail) {next=Instant::now();}
            },
            result=reads.join_next(),if !reads.is_empty()=>{
                let result=result.ok_or_else(||error(ErrorCode::Internal,"output read stopped")).and_then(|result|result.map_err(|_|error(ErrorCode::Internal,"output reader stopped"))).and_then(|result|result).and_then(|attachment|output.attachment(attachment)).and_then(|update|deliver(&sender,&budget,update));
                if let Err(error)=result {break Err(error);}
                next=Instant::now()+if matches!(output.read_plan(),ReadPlan::Idle){Duration::from_millis(250)}else{Duration::from_millis(1)};
            },
            _=tokio::time::sleep_until(next),if reads.is_empty()=>{
                let plan=output.read_plan();
                if matches!(plan,ReadPlan::UnconfirmedTail){break Err(error(ErrorCode::HistoryTruncated,"native output tail is unconfirmed; reopen to install a new checkpoint"));}
                let handle=handle.clone();let after=output.cursor();
                reads.spawn(async move {match plan {ReadPlan::Attach{after}=>handle.attach(machine,session,after).await,ReadPlan::Capture=>handle.capture(machine,session).await,_=>handle.replay(machine,session,after).await}});
            }
        }
    };
    reads.abort_all();
    while reads.join_next().await.is_some() {}
    let _ = tokio::time::timeout(Duration::from_secs(1), handle.detach(machine, session)).await;
    client.close().await;
    result
}
