use crate::{Config, Runtime};
use qgenie_remote_proto::{
    IPC_MAJOR, IPC_MINOR, decode_json,
    ipc::{AdminCommand, AdminRequest, AdminResponse, DispatchFrame},
};
use qgenie_remote_transport::ipc::{
    Listener, read_dispatch, read_frame, server_handshake_with_capabilities, write_dispatch,
    write_frame,
};
use std::sync::Arc;
use tokio::{
    sync::{Semaphore, mpsc},
    task::JoinSet,
    time::{Duration, sleep},
};

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
            accepted=data.accept() => {
                let Ok(mut stream)=accepted else { continue };
                let Ok(permit)=permits.clone().try_acquire_owned() else { continue };
                let runtime=runtime.clone();
                let recording_permits=recording_permits.clone();
                let change_permits=change_permits.clone();
                connections.spawn(async move {
                    let _permit=permit;
                    let mut supported=vec!["local-v1","native-authority-v1","native-controller-v1","recording-read-v1","recording-reclaim-v1","native-floor-v1","native-start-v1","native-effects-v1","native-replay-v1"];
                    if runtime.managed_enabled() { supported.extend(["native-agent-v1","native-approvals-v1"]); }
                    supported.push("native-changes-v1");
                    let capabilities=server_handshake_with_capabilities(&mut stream,IPC_MINOR,&supported).await?;
                    let frame=read_dispatch(&mut stream).await?;
                    if let DispatchFrame::WatchChanges {context}=&frame {
                        anyhow::ensure!(capabilities.iter().any(|cap|cap=="native-changes-v1") && capabilities.iter().any(|cap|cap=="native-authority-v1"),"native changes capability was not negotiated");
                        let _change=change_permits.try_acquire_owned()?;
                        return runtime.serve_changes(&mut stream,context.clone()).await;
                    }
                    let agent=matches!(&frame,DispatchFrame::Remote{message,..} if matches!(message.message,qgenie_remote_proto::wire::Message::Start(qgenie_remote_proto::wire::Launch{kind:qgenie_remote_proto::wire::LaunchKind::Agent{..},..})|qgenie_remote_proto::wire::Message::Input{input:qgenie_remote_proto::wire::SessionInput::Prompt(_)|qgenie_remote_proto::wire::SessionInput::Steer(_),..}));
                    let agent=agent || match &frame {DispatchFrame::AgentInventory{..}=>true,DispatchFrame::ReadSnapshot{session,..}=>runtime.is_agent_session(*session),DispatchFrame::Remote{message,..} if matches!(message.message,qgenie_remote_proto::wire::Message::Attach{..})=>message.session.is_some_and(|session|runtime.is_agent_session(session)),_=>false};
                    let approval=matches!(&frame,DispatchFrame::Remote{message,..} if matches!(message.message,qgenie_remote_proto::wire::Message::ApprovalList|qgenie_remote_proto::wire::Message::ApprovalCommitted(_)));
                    if (agent && !capabilities.iter().any(|cap|cap=="native-agent-v1")) || (approval && !capabilities.iter().any(|cap|cap=="native-approvals-v1")) {
                        write_dispatch(&mut stream,&DispatchFrame::Error(qgenie_remote_proto::RemoteError::rejected(qgenie_remote_proto::ErrorCode::UnsupportedCapability,"managed native capability was not negotiated"))).await?;
                        return Ok(());
                    }
                    let start=matches!(&frame,DispatchFrame::Remote{message,..} if matches!(message.message,qgenie_remote_proto::wire::Message::Start(_)));
                    let effect=matches!(&frame,DispatchFrame::Remote{message,..} if matches!(message.message,qgenie_remote_proto::wire::Message::Input{..}|qgenie_remote_proto::wire::Message::Resize{..}|qgenie_remote_proto::wire::Message::Interrupt{..}|qgenie_remote_proto::wire::Message::Terminate{..}));
                    let replay=matches!(&frame,DispatchFrame::ReadSnapshot{..}|DispatchFrame::ReadHead{..}) || matches!(&frame,DispatchFrame::Remote{message,..} if matches!(message.message,qgenie_remote_proto::wire::Message::Attach{..}));
                    let floor=matches!(&frame,DispatchFrame::QueryHostFloor{..});
                    let controller=matches!(&frame,DispatchFrame::QueryControl {..}|DispatchFrame::ActivateControl(_)|DispatchFrame::DeactivateControl {..});
                    let reclaim=matches!(&frame,DispatchFrame::RecordingEpochChallenge {..}|DispatchFrame::BindRecordingEpoch {..}|DispatchFrame::RecordingAcknowledge {..});
                    let recording=reclaim || matches!(&frame,DispatchFrame::RecordingFetch {..}|DispatchFrame::RecordingStatus {..});
                    if !matches!(frame,DispatchFrame::Local(_)) && (!capabilities.iter().any(|cap|cap=="native-authority-v1") || (controller && !capabilities.iter().any(|cap|cap=="native-controller-v1")) || (recording && !capabilities.iter().any(|cap|cap=="recording-read-v1")) || (reclaim && !capabilities.iter().any(|cap|cap=="recording-reclaim-v1")) || (floor && !capabilities.iter().any(|cap|cap=="native-floor-v1")) || (start && !capabilities.iter().any(|cap|cap=="native-start-v1")) || (replay && !capabilities.iter().any(|cap|cap=="native-replay-v1")) || (effect && (!capabilities.iter().any(|cap|cap=="native-effects-v1") || !capabilities.iter().any(|cap|cap=="native-controller-v1")))) {
                        write_dispatch(&mut stream,&DispatchFrame::Error(qgenie_remote_proto::RemoteError::rejected(qgenie_remote_proto::ErrorCode::UnsupportedCapability,"required native capability was not negotiated"))).await?;
                        return Ok(());
                    }
                    let _recording_permit=if recording {
                        match recording_permits.try_acquire_owned() {
                            Ok(permit)=>Some(permit),
                            Err(_)=>{write_dispatch(&mut stream,&DispatchFrame::Error(qgenie_remote_proto::RemoteError::rejected(qgenie_remote_proto::ErrorCode::Overloaded,"native recording read capacity exceeded"))).await?;return Ok(());}
                        }
                    } else {None};
                    let response=tokio::task::spawn_blocking(move ||runtime.dispatch_frame(frame)).await?;
                    write_dispatch(&mut stream,&response).await?;
                    Ok::<_,anyhow::Error>(())
                });
            }
            accepted=admin.accept() => {
                let Ok(mut stream)=accepted else { continue };
                let Ok(permit)=admin_permits.clone().try_acquire_owned() else { continue };
                let runtime=runtime.clone();
                let stop_tx=stop_tx.clone();
                connections.spawn(async move {
                    let _permit=permit;
                    let request=decode_json::<AdminRequest>(&read_frame(&mut stream).await?)?;
                    let shutdown=matches!(&request.command,AdminCommand::Shutdown {..});
                    let result=if request.ipc_major!=IPC_MAJOR {
                        Err(qgenie_remote_proto::RemoteError::rejected(qgenie_remote_proto::ErrorCode::UnsupportedVersion,"unsupported IPC version"))
                    } else {
                        match request.command {
                            AdminCommand::Shutdown { terminate_sessions }=>tokio::task::spawn_blocking(move ||runtime.shutdown(terminate_sessions)).await?,
                            AdminCommand::RegisterRemoteBinding {scope}=>tokio::task::spawn_blocking(move ||runtime.register_remote_binding(scope)).await?,
                        }
                    };
                    let stop=shutdown && result.is_ok();
                    let written=write_frame(&mut stream,&serde_json::to_vec(&AdminResponse { result })?).await;
                    // A lost admin acknowledgment cannot undo committed shutdown.
                    if stop { let _=stop_tx.send(()).await; }
                    written?;
                    Ok::<_,anyhow::Error>(())
                });
            }
            Some(())=stop_rx.recv()=>break,
            Some(_)=connections.join_next(),if !connections.is_empty()=>{}
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
