//! A public mutation retains both its original host fence and native lease.
use super::*;
use qgenie_remote_proto::{
    ipc::{DispatchFrame, RemoteContext},
    wire::{self, Message},
};

pub(super) struct Execution<'a> {
    pub(super) context: &'a RemoteContext,
    pub(super) fence: authority::Fence,
}
impl Execution<'_> {
    pub fn check_scope(&self, state: &Session) -> Result<(), RemoteError> {
        self.fence.check()?;
        if state.scope.as_ref() != Some(&self.context.scope) {
            return Err(forbidden());
        }
        Ok(())
    }
    pub fn check_controller(
        &self,
        state: &Session,
        generation: Counter,
    ) -> Result<(), RemoteError> {
        self.check_scope(state)?;
        if self.context.controller_generation != Some(generation)
            || state.remote_controller.is_none_or(|controller| {
                controller.host != self.context.host_generation
                    || controller.connection != self.context.connection
            })
        {
            return Err(RemoteError::rejected(
                ErrorCode::StaleGeneration,
                "native controller binding is stale",
            ));
        }
        Ok(())
    }
}
pub(super) fn perform<T>(
    remote: Option<&Execution<'_>>,
    operation: impl FnOnce() -> Result<T, RemoteError>,
) -> Result<T, RemoteError> {
    match remote {
        Some(remote) => remote.fence.perform(operation),
        None => operation(),
    }
}
pub(super) fn deadline(
    remote: Option<&Execution<'_>>,
    at: Instant,
    elapsed: Duration,
) -> Result<Instant, RemoteError> {
    let deadline = input_deadline(at, elapsed)?;
    match remote {
        Some(remote) => Ok(deadline.min(remote.fence.check()?)),
        None => Ok(deadline),
    }
}
impl Runtime {
    pub(super) fn remote_effect(
        self: &Arc<Self>,
        context: &RemoteContext,
        message: wire::Envelope,
    ) -> Result<DispatchFrame, RemoteError> {
        message.validate()?;
        if message.machine != Some(context.scope.machine) || message.re.is_some() {
            return Err(forbidden());
        }
        let session = message.session.ok_or_else(forbidden)?;
        let execution = Execution {
            context,
            fence: lock(&self.authority)?.fence(context)?,
        };
        #[cfg(all(feature = "managed-codex", target_os = "linux"))]
        if matches!(
            &message.message,
            Message::Input {
                input: wire::SessionInput::Prompt(_)
                    | wire::SessionInput::PromptReceipt(_)
                    | wire::SessionInput::Steer(_),
                ..
            }
        ) || (matches!(message.message, Message::Interrupt { .. })
            && lock(self.session(session)?.as_ref())?.info.agent.is_some())
        {
            return self.managed_effect(context, message, &execution);
        }
        let authority = match &message.message {
            Message::Input { authority, .. }
            | Message::Resize { authority, .. }
            | Message::Interrupt { authority } => Some(authority),
            _ => None,
        };
        if let Some(authority) = authority
            && (authority.host != context.host_generation
                || authority.controller != context.controller_generation)
        {
            return Err(forbidden());
        }
        let generation = context.controller_generation.unwrap_or(Counter(0));
        let (operation, action) = match message.message {
            Message::Input {
                input_seq,
                input: wire::SessionInput::Pty(data),
                ..
            } => (
                Operation::Input {
                    session,
                    generation,
                    input_seq,
                    data,
                },
                None,
            ),
            Message::Resize { rows, cols, .. } => (
                Operation::Resize {
                    session,
                    generation,
                    rows,
                    cols,
                },
                Some(wire::SessionAction::Resize),
            ),
            Message::Interrupt { .. } => (
                Operation::Interrupt {
                    session,
                    generation,
                },
                Some(wire::SessionAction::Interrupt),
            ),
            Message::Terminate {
                host_generation,
                confirmed,
                ..
            } if host_generation == context.host_generation
                && context.controller_generation.is_none() =>
            {
                (
                    Operation::Terminate { session, confirmed },
                    Some(wire::SessionAction::Terminate),
                )
            }
            _ => {
                return Err(RemoteError::rejected(
                    ErrorCode::UnsupportedCapability,
                    "native PTY effect is unsupported",
                ));
            }
        };
        // Scope is checked again after any session-lock wait and SQLite input intent.
        let reply = self.execute_guarded(context.client, operation, Some(&execution))?;
        execution.fence.check().map_err(|mut error| {
            error.outcome = OperationOutcome::Unknown;
            error.message =
                "native effect completed without a fresh reply; do not resend input".into();
            error
        })?;
        let body = match reply {
            Reply::InputAck { input_seq, outcome } => Message::InputAck {
                turn: None,
                generation,
                input_seq,
                outcome,
            },
            Reply::Ok => Message::ActionResult {
                action: action.ok_or_else(forbidden)?,
                outcome: ApplicationOutcome::Applied,
            },
            _ => return Err(RemoteError::unknown("native effect result is uncertain")),
        };
        let mut reply = wire::Envelope::new(body);
        reply.re = Some(message.id);
        reply.machine = message.machine;
        reply.session = Some(session);
        Ok(DispatchFrame::RemoteReply(Box::new(reply)))
    }
}
fn forbidden() -> RemoteError {
    RemoteError::rejected(ErrorCode::Forbidden, "native effect scope differs")
}
