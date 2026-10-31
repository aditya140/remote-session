//! Scriptable owner operations use the same SDK and persisted start identities as the TUI.
use anyhow::Result;
use clap::Subcommand;
use qgenie_remote_client::{GatewayClient, GatewayConfig, RemoteAttachment, UserCredentials};
use qgenie_remote_proto::{
    wire::{Launch, LaunchKind},
    *,
};
use serde_json::{Value, json};
use std::path::Path;

#[derive(Subcommand)]
pub(crate) enum Action {
    #[command(flatten)]
    WaitSet(super::remote_wait::Action),
    #[command(flatten)]
    Observe(super::remote_observe::Action),
    Machines,
    /// Resolve an exact name within this gateway owner and machine. Reject duplicates.
    FindSession {
        machine: MachineId,
        name: String,
    },
    NameSession {
        machine: MachineId,
        session: SessionId,
        #[arg(required_unless_present = "clear", conflicts_with = "clear")]
        name: Option<String>,
        #[arg(long)]
        clear: bool,
    },
    LabelSession {
        machine: MachineId,
        session: SessionId,
        key: String,
        value: String,
    },
    RemoveSessionLabel {
        machine: MachineId,
        session: SessionId,
        key: String,
    },
    Rename {
        machine: MachineId,
        name: String,
    },
    Label {
        machine: MachineId,
        key: String,
        value: String,
    },
    RemoveLabel {
        machine: MachineId,
        key: String,
    },
    /// Revoke remote access without terminating the device's native sessions.
    Revoke {
        machine: MachineId,
        #[arg(long)]
        yes: bool,
    },
    HistoryStreams {
        session: SessionId,
    },
    /// Read and verify one protected history page; data remains normalized/base64 in JSON.
    History {
        session: SessionId,
        #[arg(long)]
        stream_epoch: RecordingEpochId,
        #[arg(long)]
        after: Option<String>,
        #[arg(long, default_value_t = 4)]
        limit: u16,
    },
    Sessions {
        machine: MachineId,
        /// Return one page with an explicit continuation cursor.
        #[arg(long)]
        page: bool,
        #[arg(long, requires = "page")]
        after: Option<SessionId>,
        #[arg(long)]
        active: bool,
        /// Match session ID, directory, name, or label text across the listed pages.
        #[arg(long)]
        filter: Option<String>,
    },
    Roots {
        machine: MachineId,
    },
    Browse {
        machine: MachineId,
        path: String,
        #[arg(long)]
        after: Option<String>,
    },
    /// Launch once using a persisted scoped key. Reconcile an uncertain result by that key.
    Run {
        machine: MachineId,
        #[arg(long)]
        cwd: String,
        #[arg(long)]
        key: Option<IdempotencyKey>,
        #[arg(long, default_value_t = 24)]
        rows: u16,
        #[arg(long, default_value_t = 80)]
        cols: u16,
        #[arg(long, conflicts_with = "command")]
        agent: Option<String>,
        /// Resume a confirmed ended thread in a new owned session.
        #[arg(long, requires = "agent")]
        resume_id: Option<String>,
        /// Initial agent task. This text is not a permission boundary.
        #[arg(long, requires = "agent")]
        task: Option<String>,
        #[arg(last = true, conflicts_with = "agent")]
        command: Vec<String>,
    },
    Reconcile {
        machine: MachineId,
        key: IdempotencyKey,
    },
    /// List original journal identities for this gateway/owner/machine; never resends them.
    Starts {
        machine: MachineId,
        #[arg(long)]
        page: bool,
        #[arg(long, requires = "page")]
        after: Option<IdempotencyKey>,
    },
    /// Send one explicitly controlled input; lost replies never cause a retry.
    Send {
        machine: MachineId,
        session: SessionId,
        #[arg(required_unless_present = "stdin", conflicts_with = "stdin")]
        text: Option<String>,
        /// Read at most 32 KiB from stdin before acquiring control.
        #[arg(long, required_unless_present = "text", conflicts_with = "text")]
        stdin: bool,
        #[arg(long)]
        enter: bool,
        #[arg(long,conflicts_with_all=["steer","enter"])]
        prompt: bool,
        /// Observe this prompt's exact native turn after releasing control.
        #[arg(long, requires = "prompt")]
        wait: bool,
        #[arg(long, requires = "wait", default_value_t = 300, value_parser=clap::value_parser!(u16).range(1..=3600))]
        timeout: u16,
        #[arg(long,conflicts_with_all=["prompt","enter"])]
        steer: bool,
        #[arg(long)]
        takeover: bool,
    },
    Interrupt {
        machine: MachineId,
        session: SessionId,
        #[arg(long)]
        takeover: bool,
    },
    Terminate {
        machine: MachineId,
        session: SessionId,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        external_confirmed: bool,
    },
    /// Read one bounded native output page/checkpoint. Does not acquire control.
    Replay {
        machine: MachineId,
        session: SessionId,
        #[arg(long, default_value_t = 0)]
        after: u64,
    },
}
impl Action {
    pub(crate) fn wait_session(&self) -> Option<SessionId> {
        match self {
            Self::Observe(super::remote_observe::Action::Wait { session, .. })
            | Self::Send {
                session,
                wait: true,
                ..
            } => Some(*session),
            _ => None,
        }
    }
}

pub(crate) async fn run_authenticated(
    config: GatewayConfig,
    credentials: UserCredentials,
    state: &Path,
    action: Action,
) -> Result<()> {
    let api = GatewayClient::new(config)?;
    if let Action::WaitSet(action) = action {
        return super::remote_wait::run(api, credentials, action).await;
    }
    if let Action::Observe(action) = action {
        return super::remote_observe::run(api, credentials, action).await;
    }
    let token = credentials.current()?;
    if let Action::HistoryStreams { session } = &action {
        println!(
            "{}",
            serde_json::to_string_pretty(
                &api.history_streams(
                    &token,
                    *session,
                    qgenie_remote_proto::rest::HistoryStreamsQuery {
                        after: None,
                        limit: 64
                    }
                )
                .await?
            )?
        );
        return Ok(());
    }
    if let Action::History {
        session,
        stream_epoch,
        after,
        limit,
    } = &action
    {
        use qgenie_remote_proto::rest::{HistoryItem, HistoryQuery};
        let page = api
            .history(
                &token,
                *session,
                HistoryQuery {
                    stream_epoch: *stream_epoch,
                    after: after.clone(),
                    limit: *limit,
                },
            )
            .await?;
        let mut items = Vec::new();
        for item in page.items {
            match item {
                HistoryItem::Gap(gap) => items.push(json!({"kind":"gap","gap":gap})),
                HistoryItem::Content(chunk) => {
                    let id = chunk.id;
                    items.push(json!({"kind":"content","chunk":id,"records":api.decode_history(chunk).await?}));
                }
            }
        }
        println!(
            "{}",
            serde_json::to_string_pretty(
                &json!({"session":session,"stream_epoch":stream_epoch,"items":items,"next":page.next})
            )?
        );
        return Ok(());
    }
    let direct = match &action {
        Action::Machines => Some(api.machines(&token).await.map(|value| json!(value))),
        Action::Rename { machine, name } => Some(api.patch_machine(&token, *machine, qgenie_remote_proto::rest::MachinePatch { name: Some(name.clone()), labels: None }).await.map(|value| json!(value))),
        Action::Revoke { machine, yes } => Some(api.revoke_machine(&token, *machine, *yes).await.map(|_| json!({"machine":machine,"revocation":"committed","native_sessions":"retained"}))),
        Action::Label { machine, key, .. } | Action::RemoveLabel { machine, key } => Some(async {
            let mut labels = api.machines(&token).await?.into_iter().find(|value| value.id == *machine).ok_or_else(|| RemoteError::rejected(ErrorCode::NotFound, "machine absent from current owner inventory"))?.labels;
            if let Action::Label { value, .. } = &action { labels.insert(key.clone(), value.clone()); } else { labels.remove(key); }
            Ok(json!(api.patch_machine(&token, *machine, qgenie_remote_proto::rest::MachinePatch { name: None, labels: Some(labels) }).await?))
        }.await),
        _ => None,
    };
    if let Some(result) = direct {
        return finish(result);
    }
    let mut live = api.connect_authenticated(&credentials).await?;
    let handle = live.handle();
    // Preserve the original JSON array output while reading bounded pages.
    if matches!(
        &action,
        Action::Sessions { page: false, .. } | Action::Starts { page: false, .. }
    ) {
        let result: Result<()> = draining(&mut live, async {
            let mut first = true;
            print!("[");
            match &action {
                Action::Sessions {
                    machine,
                    active,
                    filter,
                    ..
                } => {
                    let mut after = None;
                    loop {
                        let (mut sessions, next) =
                            handle.session_page(*machine, after, 64, *active).await?;
                        filter_sessions(&mut sessions, filter.as_deref());
                        write_items(&sessions, &mut first)?;
                        after = next;
                        if after.is_none() {
                            break;
                        }
                    }
                }
                Action::Starts { machine, .. } => {
                    let owner = api
                        .machines(&token)
                        .await?
                        .into_iter()
                        .find(|m| m.id == *machine)
                        .ok_or_else(|| {
                            RemoteError::rejected(
                                ErrorCode::NotFound,
                                "machine absent from current owner inventory",
                            )
                        })?
                        .owner;
                    let mut after = None;
                    loop {
                        let (identities, next) = handle
                            .start_identities_page(state, owner, *machine, after, 64)
                            .await?;
                        write_items(&identities, &mut first)?;
                        after = next;
                        if after.is_none() {
                            break;
                        }
                    }
                }
                _ => unreachable!(),
            }
            println!("]");
            Ok(())
        })
        .await;
        live.close().await;
        return result;
    }
    let mut wait_target = None;
    let result:Result<Value,RemoteError>=draining(&mut live, async {
        Ok(match action {
            Action::Observe(_) | Action::WaitSet(_) => unreachable!("observation dispatch returns before mutations"),
            Action::Machines|Action::Rename{..}|Action::Label{..}|Action::RemoveLabel{..}|Action::Revoke{..}|Action::HistoryStreams{..}|Action::History{..}=>unreachable!(),
            Action::Sessions{machine,after,active,filter,..}=>{let(mut sessions,next)=handle.session_page(machine,after,64,active).await?;filter_sessions(&mut sessions,filter.as_deref());json!({"sessions":sessions,"next":next})},
            Action::FindSession{machine,name}=>json!(handle.resolve_session_name(machine,&name).await?),
            Action::NameSession{machine,session,name,..}=>json!(handle.update_session_metadata(machine,session,qgenie_remote_proto::wire::SessionMetadataChange::Rename(name)).await?),
            Action::LabelSession{machine,session,key,value}=>json!(handle.update_session_metadata(machine,session,qgenie_remote_proto::wire::SessionMetadataChange::Label{key,value:Some(value)}).await?),
            Action::RemoveSessionLabel{machine,session,key}=>json!(handle.update_session_metadata(machine,session,qgenie_remote_proto::wire::SessionMetadataChange::Label{key,value:None}).await?),
            Action::Roots{machine}=>json!({"roots":handle.allowed_roots(machine).await?}),
            Action::Browse{machine,path,after}=>{let(entries,next)=handle.browse(machine,path,after,128).await?;json!({"entries":entries,"next":next})},
            Action::Run{machine,cwd,key,rows,cols,command,agent,resume_id,task}=>{
                let owner=api.machines(&token).await?.into_iter().find(|m|m.id==machine).ok_or_else(||RemoteError::rejected(ErrorCode::NotFound,"machine absent from current owner inventory"))?.owner;
                let key=key.unwrap_or_default();eprintln!("Original start identity: {key}");
                let kind=agent.map(|agent|LaunchKind::Agent{agent,prompt:task,native_resume_id:resume_id}).unwrap_or_else(||LaunchKind::Pty{command:if command.is_empty(){vec!["/bin/sh".into()]}else{command}});
                let session=handle.start(state,owner,machine,Launch{key,kind,cwd,rows,cols}).await?;
                json!({"key":key,"session":session})
            },
            Action::Reconcile{machine,key}=>{let (stage,session)=handle.lookup_start(machine,key).await?;json!({"key":key,"stage":stage,"session":session})},
            Action::Starts{machine,after,..}=>{let owner=api.machines(&token).await?.into_iter().find(|m|m.id==machine).ok_or_else(||RemoteError::rejected(ErrorCode::NotFound,"machine absent from current owner inventory"))?.owner;{let(identities,next)=handle.start_identities_page(state,owner,machine,after,64).await?;json!({"identities":identities,"next":next})}},
            Action::Send{machine,session,text,stdin,enter,takeover,prompt,steer,wait,timeout}=>{
                let mut bytes=if stdin {tokio::task::spawn_blocking(|| {use std::io::Read;let mut bytes=Vec::new();std::io::stdin().lock().take(32769).read_to_end(&mut bytes).map(|_|bytes)}).await.map_err(|_|RemoteError::rejected(ErrorCode::Internal,"stdin reader failed before input"))?.map_err(|_|RemoteError::rejected(ErrorCode::Internal,"stdin read failed before input"))?}else{text.unwrap_or_default().into_bytes()};
                if enter{bytes.push(b'\r');}
                if bytes.is_empty()||bytes.len()>32768 {bytes.fill(0);return Err(RemoteError{code:ErrorCode::InvalidFrame,outcome:OperationOutcome::NotSent,message:"input must contain 1..32768 bytes including the optional Enter".into(),retryable:false});}
                if (prompt||steer) && (std::str::from_utf8(&bytes).is_err() || bytes.contains(&0)) {bytes.fill(0);return Err(RemoteError{code:ErrorCode::InvalidFrame,outcome:OperationOutcome::NotSent,message:"agent text must be UTF-8 without NUL".into(),retryable:false});}
                let mut controller=match handle.take_control(machine,session,takeover).await {Ok(control)=>control,Err(error)=>{bytes.fill(0);return Err(error)}};
                let result=if steer {controller.steer(std::str::from_utf8(&bytes).unwrap()).await.map(|_|None)}else if prompt {controller.prompt_receipt(std::str::from_utf8(&bytes).unwrap()).await.map(Some)}else{controller.input(&bytes).await.map(|_|None)};bytes.fill(0);let release=controller.release().await;let turn=result?;
                if wait && let Some(turn)=&turn { wait_target=Some(super::remote_observe::Action::Wait{machine,session,turn:Some(turn.clone()),exit:false,timeout}); }
                json!({"input":"acknowledged","machine":machine,"session":session,"turn":turn,"control_released":release.is_ok()})
            },
            Action::Interrupt{machine,session,takeover}=>{let controller=handle.take_control(machine,session,takeover).await?;let result=controller.interrupt().await;let _=controller.release().await;result?;json!({"interrupt":"requested"})},
            Action::Terminate{machine,session,yes,external_confirmed}=>{handle.terminate_current(machine,session,yes,external_confirmed).await?;json!({"termination":"requested"})},
            Action::Replay{machine,session,after}=>match handle.attach(machine,session,Counter(after)).await? {
                RemoteAttachment::Replay(page)=>json!({"floor":page.floor,"head":page.head,"events":page.events,"more":page.more,"history_truncated":page.history_truncated}),
                RemoteAttachment::Snapshot(snapshot)=>{use base64::{Engine,engine::general_purpose::STANDARD};json!({"snapshot":{"epoch":snapshot.epoch,"seq":snapshot.seq,"rows":snapshot.rows,"cols":snapshot.cols,"format":snapshot.format,"data":STANDARD.encode(snapshot.data)},"floor":snapshot.floor,"head":snapshot.head})},
            },
        })
    }).await;
    live.close().await;
    finish(result)?;
    if let Some(wait) = wait_target {
        super::remote_observe::run(api, credentials, wait).await?;
    }
    Ok(())
}
fn finish(result: Result<Value, RemoteError>) -> Result<()> {
    match result {
        Ok(value) => println!("{}", serde_json::to_string_pretty(&value)?),
        Err(error) => {
            println!("{}", serde_json::to_string(&json!({"error":error}))?);
            return Err(error.into());
        }
    }
    Ok(())
}

fn write_items<T: serde::Serialize>(items: &[T], first: &mut bool) -> Result<()> {
    for item in items {
        if !*first {
            print!(",");
        }
        *first = false;
        print!("{}", serde_json::to_string(item)?);
    }
    Ok(())
}

// The driver still resolves interrupted mutation requests with their original outcome.
// Draining unsolicited traffic here does not retry or replace those requests.
async fn draining<T>(
    live: &mut qgenie_remote_client::LiveClient,
    work: impl std::future::Future<Output = T>,
) -> T {
    tokio::pin!(work);
    let mut connected = true;
    loop {
        tokio::select! { biased;
            result = &mut work => return result,
            event = live.next(), if connected => if event.is_none() { connected = false; }
        }
    }
}

fn filter_sessions(sessions: &mut Vec<qgenie_remote_proto::wire::Session>, filter: Option<&str>) {
    if let Some(filter) = filter {
        let filter = filter.to_lowercase();
        sessions.retain(|session| {
            session.id.to_string().contains(&filter)
                || session.cwd.to_lowercase().contains(&filter)
                || session.metadata.matches(&filter)
        });
    }
}
