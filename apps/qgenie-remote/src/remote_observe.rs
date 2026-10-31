//! Bounded read-only commands. Only connection and observation requests can restart.
use anyhow::{Result, ensure};
use clap::Subcommand;
use qgenie_remote_client::{
    Activity, GatewayClient, LiveClient, LiveHandle, OutputCursor, ReadPlan, RemoteAttachment,
    RemoteSnapshot, UserCredentials, WaitOutcome, WaitTarget, observation_can_reconnect,
};
use qgenie_remote_proto::{wire::LaunchKind, *};
use serde_json::json;
use std::{future::Future, io::Write, time::Duration};

#[derive(Subcommand)]
pub(crate) enum Action {
    /// Read a bounded plain-text checkpoint without controlling or scrolling the session.
    Read {
        machine: MachineId,
        session: SessionId,
        #[arg(long,default_value_t=100,value_parser=clap::value_parser!(u16).range(1..=1000))]
        lines: u16,
    },
    /// Emit bounded JSON lines. Checkpoint records explicitly mark unavailable event history.
    Events {
        machine: MachineId,
        session: SessionId,
        #[arg(long, default_value_t = 0)]
        after: u64,
        #[arg(long,default_value_t=100,value_parser=clap::value_parser!(u16).range(1..=1000))]
        limit: u16,
        #[arg(long,default_value_t=10,value_parser=clap::value_parser!(u16).range(1..=3600))]
        timeout: u16,
    },
    /// Report typed activity, native lifetime, and the bounded durable approval summary.
    Activity {
        machine: MachineId,
        session: SessionId,
    },
    /// Wait for one exact turn or native exit. Timeout and cancellation leave work running.
    Wait {
        machine: MachineId,
        session: SessionId,
        #[arg(long, required_unless_present = "exit", conflicts_with = "exit")]
        turn: Option<String>,
        #[arg(long, required_unless_present = "turn", conflicts_with = "turn")]
        exit: bool,
        #[arg(long,default_value_t=300,value_parser=clap::value_parser!(u16).range(1..=3600))]
        timeout: u16,
    },
}
impl Action {
    fn target(&self) -> (MachineId, SessionId) {
        match self {
            Self::Read {
                machine, session, ..
            }
            | Self::Events {
                machine, session, ..
            }
            | Self::Activity { machine, session }
            | Self::Wait {
                machine, session, ..
            } => (*machine, *session),
        }
    }
    fn timeout(&self) -> Duration {
        Duration::from_secs(u64::from(match self {
            Self::Events { timeout, .. } | Self::Wait { timeout, .. } => *timeout,
            _ => 30,
        }))
    }
}

async fn drain<T>(
    client: &mut LiveClient,
    read: impl Future<Output = Result<T, RemoteError>>,
) -> Result<T, RemoteError> {
    tokio::pin!(read);
    loop {
        tokio::select! {biased;
            result = &mut read => return if result.is_err() && !matches!(*client.status().borrow(),qgenie_remote_client::LiveStatus::Connected{..}) {Err(client.disconnect_error())} else {result},
            event = client.next() => if event.is_none() { return Err(client.disconnect_error()); }
        }
    }
}
async fn checkpoint(
    handle: &LiveHandle,
    machine: MachineId,
    session: SessionId,
) -> Result<RemoteSnapshot, RemoteError> {
    match handle.attach(machine, session, Counter(0)).await? {
        RemoteAttachment::Snapshot(snapshot) => Ok(snapshot),
        RemoteAttachment::Replay(_) => match handle.capture(machine, session).await? {
            RemoteAttachment::Snapshot(snapshot) => Ok(snapshot),
            _ => Err(RemoteError::rejected(
                ErrorCode::InvalidFrame,
                "capture returned no checkpoint",
            )),
        },
    }
}
fn emit(value: serde_json::Value) -> Result<()> {
    let mut stdout = std::io::stdout().lock();
    serde_json::to_writer(&mut stdout, &value)?;
    writeln!(stdout)?;
    stdout.flush()?;
    Ok(())
}

pub(crate) async fn run(
    api: GatewayClient,
    credentials: UserCredentials,
    action: Action,
) -> Result<()> {
    if let Action::Wait {
        turn: Some(turn), ..
    } = &action
    {
        ensure!(
            !turn.is_empty() && turn.len() <= 512 && !turn.chars().any(char::is_control),
            "turn identity must be 1..=512 bytes without control characters"
        );
    }
    let (machine, session) = action.target();
    let duration = action.timeout();
    let mut cursor = match &action {
        Action::Events { after, .. } => OutputCursor::resume(machine, session, Counter(*after)),
        _ => OutputCursor::checkpointed(machine, session),
    };
    let mut last_emitted = cursor.cursor();
    let mut emitted = 0usize;
    let mut bytes = 0usize;
    let work = async {
        let mut client = match api.connect_authenticated(&credentials).await {
            Ok(client) => client,
            Err(error) if observation_can_reconnect(&error) => {
                api.reconnect_observation(&credentials).await?
            }
            Err(error) => return Err(error.into()),
        };
        loop {
            let handle = client.handle();
            let result: Result<bool, RemoteError> = match &action {
                Action::Events { limit, .. } => {
                    let plan = cursor.read_plan();
                    let after = cursor.cursor();
                    let read = async {
                        match plan {
                            ReadPlan::Attach { after } => {
                                handle.attach(machine, session, after).await
                            }
                            ReadPlan::Capture => handle.capture(machine, session).await,
                            ReadPlan::UnconfirmedTail => Err(RemoteError::rejected(
                                ErrorCode::HistoryTruncated,
                                "native tail is unconfirmed",
                            )),
                            _ => handle.replay(machine, session, after).await,
                        }
                    };
                    match drain(&mut client, read).await {
                        Ok(attachment) => {
                            let update = cursor.attachment(attachment)?;
                            if let Some(snapshot) = update.snapshot {
                                emit(
                                    json!({"type":"checkpoint","after":after,"at":snapshot.seq,"head":snapshot.head,"history_truncated":true,"read_command":"remote read"}),
                                )?;
                                last_emitted = snapshot.seq;
                                emitted += 1;
                            }
                            for item in update.events {
                                let value = json!({"type":"event","seq":item.seq,"at":item.at,"event":item.event});
                                let size = serde_json::to_vec(&value)?.len() + 1;
                                if emitted >= usize::from(*limit) || bytes + size > 1024 * 1024 {
                                    emit(json!({"type":"limit","after":last_emitted}))?;
                                    client.close().await;
                                    return Ok(());
                                }
                                emit(value)?;
                                last_emitted = item.seq;
                                emitted += 1;
                                bytes += size;
                            }
                            Ok(emitted >= usize::from(*limit))
                        }
                        Err(error) => Err(error),
                    }
                }
                _ => {
                    let read = async {
                        let current = handle.session(machine, session).await?;
                        let lifetime_only = matches!(action, Action::Wait { turn: None, .. })
                            || (matches!(action, Action::Wait { .. })
                                && matches!(
                                    current.state,
                                    SessionState::Lost
                                        | SessionState::OutcomeUnknown
                                        | SessionState::Failed
                                ));
                        let snapshot = if lifetime_only {
                            None
                        } else {
                            match checkpoint(&handle, machine, session).await {
                                Ok(snapshot) => Some(snapshot),
                                Err(error)
                                    if matches!(action, Action::Wait { .. })
                                        && (error.code == ErrorCode::HistoryTruncated
                                            || (matches!(current.state, SessionState::Exited)
                                                && error.code == ErrorCode::NotFound)) =>
                                {
                                    None
                                }
                                Err(error) => return Err(error),
                            }
                        };
                        let approvals = if matches!(action, Action::Read { .. })
                            || (matches!(action, Action::Wait { .. })
                                && matches!(
                                    current.state,
                                    SessionState::Exited
                                        | SessionState::Failed
                                        | SessionState::Lost
                                        | SessionState::OutcomeUnknown
                                )) {
                            None
                        } else {
                            match handle.approval_summary().await {
                                Ok(summary) => Some(summary),
                                Err(error) if error.code == ErrorCode::UnsupportedCapability => {
                                    None
                                }
                                Err(error) => return Err(error),
                            }
                        };
                        Ok((current, snapshot, approvals))
                    };
                    match drain(&mut client, read).await {
                        Ok((current, snapshot, approvals)) => {
                            if let Action::Read { lines, .. } = &action {
                                println!(
                                    "{}",
                                    qgenie_remote_tui::render_remote_snapshot(
                                        snapshot
                                            .as_ref()
                                            .expect("read always requests a checkpoint"),
                                        usize::from(*lines)
                                    )?
                                );
                                Ok(true)
                            } else {
                                let agent = if matches!(current.kind, LaunchKind::Agent { .. }) {
                                    snapshot
                                        .as_ref()
                                        .map(RemoteSnapshot::agent_state)
                                        .transpose()?
                                } else {
                                    None
                                };
                                let activity = agent
                                    .as_ref()
                                    .map_or(Activity::Unknown, Activity::from_agent);
                                let pending = approvals
                                    .as_ref()
                                    .and_then(|summary| summary.sessions.get(&session))
                                    .copied()
                                    .unwrap_or(0);
                                let outcome = match &action {
                                    Action::Wait { turn, .. } => turn
                                        .as_ref()
                                        .map_or(WaitTarget::Exit, |turn| {
                                            WaitTarget::Turn(turn.clone())
                                        })
                                        .outcome(&current.state, agent.as_ref(), pending),
                                    _ => None,
                                };
                                if matches!(action, Action::Activity { .. }) || outcome.is_some() {
                                    emit(
                                        json!({"session":session,"lifetime":current.state,"activity":activity,"pending_approvals":pending,"approval_status_complete":approvals.as_ref().is_some_and(|summary|summary.complete),"history_truncated":agent.as_ref().map(|state|state.history_truncated),"unconfirmed_tail":agent.as_ref().map(|state|state.unconfirmed_tail),"outcome":outcome,"recording_warning":current.recording_warning}),
                                    )?;
                                    Ok(true)
                                } else {
                                    Ok(false)
                                }
                            }
                        }
                        Err(error) => Err(error),
                    }
                }
            };
            match result {
                Ok(true) => {
                    client.close().await;
                    return Ok(());
                }
                Ok(false) => {
                    // Drain unsolicited traffic while waiting for the next bounded observation.
                    drain(&mut client, async {
                        tokio::time::sleep(Duration::from_secs(1)).await;
                        Ok(())
                    })
                    .await
                    .or_else(|error| {
                        if observation_can_reconnect(&error) {
                            Ok(())
                        } else {
                            Err(error)
                        }
                    })?;
                }
                Err(error) if observation_can_reconnect(&error) => {
                    eprintln!("Observation connection lost; reconnecting. Native work continues.");
                    client.close().await;
                    cursor.disconnected();
                    client = api.reconnect_observation(&credentials).await?;
                }
                Err(error) => return Err(error.into()),
            }
        }
    };
    let result = tokio::select! {
        result = tokio::time::timeout(duration, work) => result,
        signal = cancelled(), if matches!(action, Action::Wait { .. }) => {
            signal?;
            return emit(json!({"session":session,"outcome":WaitOutcome::Cancelled,"native_sessions":"retained"}));
        }
    };
    match result {
        Ok(result) => result,
        Err(_) if matches!(action, Action::Wait { .. }) => emit(
            json!({"session":session,"outcome":WaitOutcome::Timeout,"native_sessions":"retained"}),
        ),
        Err(_) if matches!(action, Action::Events { .. }) => {
            emit(json!({"type":"timeout","native_sessions":"retained"}))
        }
        Err(_) => anyhow::bail!("observation timed out; native work continues"),
    }
}

async fn cancelled() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut terminate =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = tokio::signal::ctrl_c() => result,
            _ = terminate.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    tokio::signal::ctrl_c().await
}
