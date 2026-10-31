//! Explicit bounded observation waits. This module owns cancellation for these commands.
use anyhow::{Result, ensure};
use clap::{Subcommand, ValueEnum};
use qgenie_remote_client::{
    GatewayClient, OutputCursor, OutputLiteral, OutputMatchOutcome, ReadPlan, UserCredentials,
    WaitOutcome, WaitTarget, observation_can_reconnect,
};
use qgenie_remote_proto::{wire::LaunchKind, *};
use serde::Serialize;
use std::{str::FromStr, time::Duration};
use tokio::task::JoinSet;

#[derive(Subcommand)]
pub(crate) enum Action {
    /// Wait for a literal in fresh PTY output after an explicit event cursor.
    WaitOutput {
        machine: MachineId,
        session: SessionId,
        #[arg(long)]
        after: u64,
        #[arg(long)]
        literal: String,
        #[arg(long,default_value_t=300,value_parser=clap::value_parser!(u16).range(1..=3600))]
        timeout: u16,
    },
    /// Observe up to 16 exact targets. A resolved target may have failed or need approval.
    WaitMany {
        #[arg(long,value_enum,default_value_t=Mode::All)]
        mode: Mode,
        /// Repeat MACHINE/SESSION/exit or MACHINE/SESSION/turn:TURN_ID.
        #[arg(long, required = true)]
        target: Vec<TaskTarget>,
        #[arg(long,default_value_t=300,value_parser=clap::value_parser!(u16).range(1..=3600))]
        timeout: u16,
    },
}
#[derive(Clone, Copy, Debug, ValueEnum, Serialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Mode {
    Any,
    All,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "turn", rename_all = "snake_case")]
enum Condition {
    Turn(String),
    Exit,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct TaskTarget {
    machine: MachineId,
    session: SessionId,
    condition: Condition,
}
impl FromStr for TaskTarget {
    type Err = String;
    fn from_str(value: &str) -> std::result::Result<Self, String> {
        let mut parts = value.splitn(3, '/');
        let machine = parts
            .next()
            .ok_or("missing machine")?
            .parse()
            .map_err(|_| "invalid machine ID")?;
        let session = parts
            .next()
            .ok_or("missing session")?
            .parse()
            .map_err(|_| "invalid session ID")?;
        let condition = match parts.next() {
            Some("exit") => Condition::Exit,
            Some(value) if value.starts_with("turn:") => {
                let turn = &value[5..];
                if turn.is_empty() || turn.len() > 512 || turn.chars().any(char::is_control) {
                    return Err("turn must be 1..=512 bytes without control characters".into());
                }
                Condition::Turn(turn.into())
            }
            _ => {
                return Err(
                    "target must be MACHINE/SESSION/exit or MACHINE/SESSION/turn:TURN_ID".into(),
                );
            }
        };
        Ok(Self {
            machine,
            session,
            condition,
        })
    }
}
#[derive(Serialize)]
struct TargetResult {
    target: TaskTarget,
    outcome: Option<WaitOutcome>,
    pending_approvals: Option<usize>,
    approval_status_complete: bool,
    observation_error: Option<ErrorCode>,
}
fn emit(value: serde_json::Value) -> Result<()> {
    super::remote_observe::emit(value)
}
pub(crate) async fn run(
    api: GatewayClient,
    credentials: UserCredentials,
    action: Action,
) -> Result<()> {
    match action {
        Action::WaitMany {
            mode,
            target,
            timeout,
        } => many(api, credentials, mode, target, timeout).await,
        Action::WaitOutput {
            machine,
            session,
            after,
            literal,
            timeout,
        } => output(api, credentials, machine, session, after, &literal, timeout).await,
    }
}
async fn sample(
    api: GatewayClient,
    credentials: UserCredentials,
    target: TaskTarget,
) -> std::result::Result<(Option<WaitOutcome>, Option<usize>, bool), RemoteError> {
    let mut client = api.connect_authenticated(&credentials).await?;
    let handle = client.handle();
    let read = async {
        let current = handle.session(target.machine, target.session).await?;
        let condition = match &target.condition {
            Condition::Turn(turn) => WaitTarget::Turn(turn.clone()),
            Condition::Exit => WaitTarget::Exit,
        };
        let agent = if matches!(target.condition, Condition::Turn(_))
            && !matches!(
                current.state,
                SessionState::Lost | SessionState::OutcomeUnknown | SessionState::Failed
            ) {
            if !matches!(current.kind, LaunchKind::Agent { .. }) {
                return Err(RemoteError::rejected(
                    ErrorCode::UnsupportedCapability,
                    "turn targets require an agent session",
                ));
            }
            match super::remote_observe::checkpoint(&handle, target.machine, target.session).await {
                Ok(snapshot) => Some(snapshot.agent_state()?),
                Err(error)
                    if error.code == ErrorCode::HistoryTruncated
                        || (error.code == ErrorCode::NotFound
                            && matches!(current.state, SessionState::Exited)) =>
                {
                    None
                }
                Err(error) => return Err(error),
            }
        } else {
            None
        };
        if let Some(outcome) = condition.outcome(&current.state, agent.as_ref(), 0) {
            return Ok((Some(outcome), None, false));
        }
        let approvals = match handle.approval_summary().await {
            Ok(summary) => Some(summary),
            Err(error) if error.code == ErrorCode::UnsupportedCapability => None,
            Err(error) => return Err(error),
        };
        let pending = approvals
            .as_ref()
            .map(|summary| summary.sessions.get(&target.session).copied().unwrap_or(0));
        Ok((
            condition.outcome(&current.state, agent.as_ref(), pending.unwrap_or(0)),
            pending,
            approvals.is_some_and(|summary| summary.complete),
        ))
    };
    let result = super::remote_observe::drain(&mut client, read).await;
    client.close().await;
    result
}
async fn collect(
    api: &GatewayClient,
    credentials: &UserCredentials,
    mode: Mode,
    results: &mut [TargetResult],
) -> Result<&'static str> {
    loop {
        let indices: Vec<_> = results
            .iter()
            .enumerate()
            .filter_map(|(index, result)| result.outcome.is_none().then_some(index))
            .collect();
        if indices.is_empty() {
            return Ok("all_resolved");
        }
        let mut queue = indices.into_iter();
        let mut jobs = JoinSet::new();
        loop {
            while jobs.len() < 2
                && let Some(index) = queue.next()
            {
                let api = api.clone();
                let credentials = credentials.clone();
                let target = results[index].target.clone();
                jobs.spawn(async move { (index, sample(api, credentials, target).await) });
            }
            let Some(result) = jobs.join_next().await else {
                break;
            };
            let (index, value) = result?;
            let result = &mut results[index];
            match value {
                Ok((outcome, pending, complete)) => {
                    result.outcome = outcome;
                    result.pending_approvals = pending;
                    result.approval_status_complete = complete;
                    result.observation_error = None;
                }
                Err(error) => {
                    if !observation_can_reconnect(&error) {
                        result.outcome = Some(WaitOutcome::HistoryUnavailable);
                    }
                    result.observation_error = Some(error.code);
                }
            }
            if mode == Mode::Any && result.outcome.is_some() {
                return Ok("any_resolved");
            }
        }
        if results.iter().all(|result| result.outcome.is_some()) {
            return Ok("all_resolved");
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
}
async fn many(
    api: GatewayClient,
    credentials: UserCredentials,
    mode: Mode,
    targets: Vec<TaskTarget>,
    timeout: u16,
) -> Result<()> {
    ensure!(
        !targets.is_empty() && targets.len() <= 16,
        "select 1..=16 explicit targets"
    );
    for (index, target) in targets.iter().enumerate() {
        ensure!(!targets[..index].contains(target), "duplicate wait target");
    }
    let mut results: Vec<_> = targets
        .into_iter()
        .map(|target| TargetResult {
            target,
            outcome: None,
            pending_approvals: None,
            approval_status_complete: false,
            observation_error: None,
        })
        .collect();
    let reason = {
        let work = collect(&api, &credentials, mode, &mut results);
        tokio::pin!(work);
        tokio::select! {
            result=&mut work=>result?,
            _=tokio::time::sleep(Duration::from_secs(u64::from(timeout)))=>"timeout",
            result=super::host::shutdown_signal()=>{result?;"cancelled"},
        }
    };
    for result in &mut results {
        if result.outcome.is_none() {
            result.outcome = match reason {
                "timeout" => Some(WaitOutcome::Timeout),
                "cancelled" => Some(WaitOutcome::Cancelled),
                _ => None,
            };
        }
    }
    emit(
        serde_json::json!({"mode":mode,"reason":reason,"targets":results,"native_sessions":"retained"}),
    )
}
async fn output(
    api: GatewayClient,
    credentials: UserCredentials,
    machine: MachineId,
    session: SessionId,
    after: u64,
    literal: &str,
    timeout: u16,
) -> Result<()> {
    let mut matcher = OutputLiteral::new(Counter(after), literal)?;
    let mut cursor = OutputCursor::resume(machine, session, Counter(0));
    let outcome = {
        let work = output_follow(
            &api,
            &credentials,
            machine,
            session,
            &mut cursor,
            &mut matcher,
        );
        tokio::pin!(work);
        tokio::select! {
            result=&mut work=>result?,
            _=tokio::time::sleep(Duration::from_secs(u64::from(timeout)))=>OutputMatchOutcome::Timeout,
            result=super::host::shutdown_signal()=>{result?;OutputMatchOutcome::Cancelled},
        }
    };
    emit(
        serde_json::json!({"machine":machine,"session":session,"after":Counter(after),"observed_through":matcher.cursor(),"outcome":outcome,"native_sessions":"retained"}),
    )
}
async fn output_follow(
    api: &GatewayClient,
    credentials: &UserCredentials,
    machine: MachineId,
    session: SessionId,
    cursor: &mut OutputCursor,
    matcher: &mut OutputLiteral,
) -> Result<OutputMatchOutcome> {
    let mut client = match api.connect_authenticated(credentials).await {
        Ok(client) => client,
        Err(error) if observation_can_reconnect(&error) => {
            api.reconnect_observation(credentials).await?
        }
        Err(error) => return Err(error.into()),
    };
    loop {
        let handle = client.handle();
        let read = async {
            let current = handle.session(machine, session).await?;
            if matches!(
                current.state,
                SessionState::Lost | SessionState::OutcomeUnknown | SessionState::Failed
            ) {
                return Err(RemoteError::rejected(
                    ErrorCode::HistoryTruncated,
                    "native output lifetime is unavailable",
                ));
            }
            if !matches!(current.kind, LaunchKind::Pty { .. }) {
                return Err(RemoteError::rejected(
                    ErrorCode::UnsupportedCapability,
                    "literal waits require a PTY session",
                ));
            }
            let attachment = match cursor.read_plan() {
                ReadPlan::Attach { after } => handle.attach(machine, session, after).await,
                ReadPlan::Capture | ReadPlan::UnconfirmedTail => Err(RemoteError::rejected(
                    ErrorCode::HistoryTruncated,
                    "output continuity unavailable",
                )),
                _ => handle.replay(machine, session, cursor.cursor()).await,
            }?;
            Ok((current, attachment))
        };
        match super::remote_observe::drain(&mut client, read).await {
            Ok((current, attachment)) => {
                let update = match cursor.attachment(attachment) {
                    Ok(update) => update,
                    Err(error)
                        if matches!(
                            error.code,
                            ErrorCode::HistoryTruncated | ErrorCode::Overloaded
                        ) =>
                    {
                        return Ok(OutputMatchOutcome::HistoryUnavailable);
                    }
                    Err(error) => return Err(error.into()),
                };
                if update.snapshot.is_some()
                    || matches!(
                        cursor.read_plan(),
                        ReadPlan::Capture | ReadPlan::UnconfirmedTail
                    )
                {
                    return Ok(OutputMatchOutcome::HistoryUnavailable);
                }
                for item in update.events {
                    if let Some(outcome) = matcher.event(&item)? {
                        client.close().await;
                        return Ok(outcome);
                    }
                }
                if matches!(current.state, SessionState::Exited)
                    && cursor.cursor() >= cursor.head()
                    && cursor.cursor() >= current.head
                {
                    client.close().await;
                    return Ok(OutputMatchOutcome::EndedBeforeMatch);
                }
            }
            Err(error) if error.code == ErrorCode::HistoryTruncated => {
                return Ok(OutputMatchOutcome::HistoryUnavailable);
            }
            Err(error) if observation_can_reconnect(&error) => {
                client.close().await;
                cursor.disconnected();
                client = api.reconnect_observation(credentials).await?;
                continue;
            }
            Err(error) => return Err(error.into()),
        }
        if matches!(cursor.read_plan(), ReadPlan::Idle) {
            // Replay remains the authority; drain live events while waiting to avoid an unbounded subscriber.
            let result = super::remote_observe::drain(&mut client, async {
                tokio::time::sleep(Duration::from_millis(200)).await;
                Ok(())
            })
            .await;
            if let Err(error) = result {
                if observation_can_reconnect(&error) {
                    client.close().await;
                    cursor.disconnected();
                    client = api.reconnect_observation(credentials).await?;
                } else {
                    return Err(error.into());
                }
            }
        }
    }
}
