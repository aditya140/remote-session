use crate::{Config, Runtime};
use qgenie_remote_proto::{
    ErrorCode, IPC_MAJOR, IPC_MINOR, RemoteError, decode_json,
    ipc::{AdminCommand, AdminRequest, AdminResponse, DispatchFrame},
};
use qgenie_remote_transport::ipc::{
    Listener, read_dispatch, read_frame, server_handshake_with_capabilities, write_dispatch,
    write_frame,
};
use std::sync::Arc;
use tokio::{
    net::UnixStream,
    sync::{OwnedSemaphorePermit, Semaphore, mpsc},
    task::JoinSet,
    time::{Duration, sleep},
};

mod capabilities;

#[cfg(test)]
mod tests;

pub async fn serve(config: Config) -> anyhow::Result<()> {
    let data = Listener::bind(&config.state_dir.join("dispatch.sock"))?;
    let admin = Listener::bind(&config.state_dir.join("admin.sock"))?;
    let runtime =
        tokio::task::spawn_blocking(move || Runtime::open_after_previous_owner(config)).await??;
    runtime.prepare_agents().await?;
    serve_peers(data, admin, runtime).await
}
#[cfg(test)]
pub(crate) async fn serve_existing(
    runtime: Arc<Runtime>,
    directory: std::path::PathBuf,
) -> anyhow::Result<()> {
    let data = Listener::bind(&directory.join("dispatch.sock"))?;
    let admin = Listener::bind(&directory.join("admin.sock"))?;
    serve_peers(data, admin, runtime).await
}
/// Serve an already owned runtime, including locally injected agent profiles.
/// Binding the same private endpoints still enforces the single native owner.
pub async fn serve_runtime(runtime: Arc<Runtime>) -> anyhow::Result<()> {
    runtime.prepare_agents().await?;
    let data = Listener::bind(&runtime.state_dir().join("dispatch.sock"))?;
    let admin = Listener::bind(&runtime.state_dir().join("admin.sock"))?;
    serve_peers(data, admin, runtime).await
}
async fn serve_peers(data: Listener, admin: Listener, runtime: Arc<Runtime>) -> anyhow::Result<()> {
    let permits = Arc::new(Semaphore::new(64));
    let admin_permits = Arc::new(Semaphore::new(4));
    let recording_permits = Arc::new(Semaphore::new(4));
    let change_permits = Arc::new(Semaphore::new(4));
    let (stop_tx, mut stop_rx) = mpsc::channel(1);
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            accepted = data.accept() => {
                let Ok(stream) = accepted else { continue };
                let Ok(permit) = permits.clone().try_acquire_owned() else { continue };
                let runtime = runtime.clone();
                let recording_permits = recording_permits.clone();
                let change_permits = change_permits.clone();
                connections.spawn(async move {
                    serve_data(stream, runtime, permit, recording_permits, change_permits).await
                });
            }
            accepted = admin.accept() => {
                let Ok(stream) = accepted else { continue };
                let Ok(permit) = admin_permits.clone().try_acquire_owned() else { continue };
                let runtime = runtime.clone();
                let stop_tx = stop_tx.clone();
                connections.spawn(async move {
                    serve_admin(stream, runtime, permit, stop_tx).await
                });
            }
            Some(()) = stop_rx.recv() => break,
            Some(_) = connections.join_next(), if !connections.is_empty() => {}
        }
    }
    drop(data);
    drop(admin);
    connections.abort_all();
    while connections.join_next().await.is_some() {}
    // Confirmed shutdown must keep the owner process/executors alive through
    // native startup completion and actual child reap. An arbitrary grace
    // period can strand a child still being initialized.
    while !runtime.readers_finished() {
        sleep(Duration::from_millis(10)).await;
    }
    Ok(())
}

async fn serve_data(
    mut stream: UnixStream,
    runtime: Arc<Runtime>,
    permit: OwnedSemaphorePermit,
    recording_permits: Arc<Semaphore>,
    change_permits: Arc<Semaphore>,
) -> anyhow::Result<()> {
    let supported = capabilities::offered(runtime.managed_enabled());
    let negotiated = server_handshake_with_capabilities(&mut stream, IPC_MINOR, &supported).await?;
    let frame = read_dispatch(&mut stream).await?;
    if let DispatchFrame::WatchChanges { context } = &frame {
        // Keep the subscription's admission permits until its socket closes.
        let _permit = permit;
        capabilities::check(&runtime, &frame, &negotiated)?;
        let _change = change_permits.try_acquire_owned()?;
        return runtime.serve_changes(&mut stream, context.clone()).await;
    }
    let (response, _permit, _recording_permit) = tokio::task::spawn_blocking(move || {
        // The worker owns capacity through cancellation. Successful callers keep
        // it until the reply is written, so slow readers remain bounded too.
        let mut recording_permit = None;
        let response = (|| {
            capabilities::check(&runtime, &frame, &negotiated)?;
            if capabilities::recording(&frame) {
                recording_permit = Some(recording_permits.try_acquire_owned().map_err(|_| {
                    RemoteError::rejected(
                        ErrorCode::Overloaded,
                        "native recording read capacity exceeded",
                    )
                })?);
            }
            Ok(runtime.dispatch_frame(frame))
        })()
        .unwrap_or_else(DispatchFrame::Error);
        (response, permit, recording_permit)
    })
    .await?;
    write_dispatch(&mut stream, &response).await?;
    Ok(())
}

async fn serve_admin(
    mut stream: UnixStream,
    runtime: Arc<Runtime>,
    permit: OwnedSemaphorePermit,
    stop_tx: mpsc::Sender<()>,
) -> anyhow::Result<()> {
    let request = decode_json::<AdminRequest>(&read_frame(&mut stream).await?)?;
    let shutdown = matches!(&request.command, AdminCommand::Shutdown { .. });
    let (result, _permit) = tokio::task::spawn_blocking(move || {
        let result = if request.ipc_major != IPC_MAJOR {
            Err(RemoteError::rejected(
                ErrorCode::UnsupportedVersion,
                "unsupported IPC version",
            ))
        } else {
            match request.command {
                AdminCommand::Shutdown { terminate_sessions } => {
                    runtime.shutdown(terminate_sessions)
                }
                AdminCommand::RegisterRemoteBinding { scope } => {
                    runtime.register_remote_binding(scope)
                }
            }
        };
        (result, permit)
    })
    .await?;
    let stop = shutdown && result.is_ok();
    let written = write_frame(&mut stream, &serde_json::to_vec(&AdminResponse { result })?).await;
    // A lost admin acknowledgment cannot undo committed shutdown.
    if stop {
        let _ = stop_tx.send(()).await;
    }
    written?;
    Ok(())
}
