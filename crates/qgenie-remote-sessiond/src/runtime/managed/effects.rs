//! The original host and controller grant covers each actual native OS write.
use super::*;
use qgenie_remote_proto::{
    ipc::{DispatchFrame, RemoteContext},
    wire::{self, Message, SessionInput},
};
use std::sync::Weak;

struct Grant {
    session: Weak<Mutex<Session>>,
    client: ClientId,
    generation: Counter,
    deadline: Instant,
    elapsed_deadline: Duration,
    remote: Option<(RemoteContext, authority::Fence)>,
}
impl NativeEffectAuthorization for Grant {
    fn perform(
        &self,
        effect: &mut dyn FnMut() -> std::io::Result<usize>,
    ) -> std::io::Result<usize> {
        let denied = || std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        input_deadline(self.deadline, self.elapsed_deadline).map_err(|_| denied())?;
        let session = self.session.upgrade().ok_or_else(denied)?;
        let mut state = session.try_lock().map_err(|error| match error {
            std::sync::TryLockError::WouldBlock => std::io::ErrorKind::WouldBlock.into(),
            std::sync::TryLockError::Poisoned(_) => denied(),
        })?;
        expire(&mut state);
        if !state
            .info
            .controller
            .as_ref()
            .is_some_and(|lease| lease.holder == self.client && lease.generation == self.generation)
            || state.agent_control.is_none()
            || input_deadline(self.deadline, self.elapsed_deadline)
                .map_or(true, |deadline| deadline <= Instant::now())
        {
            return Err(denied());
        }
        if let Some((context, fence)) = &self.remote {
            if state.scope.as_ref() != Some(&context.scope)
                || state.remote_controller.is_none_or(|binding| {
                    binding.host != context.host_generation
                        || binding.connection != context.connection
                })
            {
                return Err(denied());
            }
            // Session -> effect barrier is the fixed lock order. No store or authority
            // mutex is held, and the closure is one nonblocking OS operation.
            NativeEffectAuthorization::perform(fence, effect)
        } else if state.remote_controller.is_some() {
            Err(denied())
        } else {
            effect()
        }
    }
}

impl Runtime {
    pub(in super::super) fn managed_effect(
        &self,
        context: &RemoteContext,
        request: wire::Envelope,
        execution: &remote_effects::Execution<'_>,
    ) -> Result<DispatchFrame, RemoteError> {
        let session = request.session.ok_or_else(|| {
            RemoteError::rejected(ErrorCode::InvalidFrame, "missing agent session")
        })?;
        let generation = context.controller_generation.ok_or_else(|| {
            RemoteError::rejected(
                ErrorCode::StaleGeneration,
                "agent input requires a controller",
            )
        })?;
        let body = match request.message {
            Message::Input {
                authority,
                input_seq,
                input,
            } if authority.host == context.host_generation
                && authority.controller == Some(generation) =>
            {
                let receipt_requested = matches!(input, SessionInput::PromptReceipt(_));
                let receipt = self.agent_input(
                    context.client,
                    session,
                    generation,
                    input_seq,
                    input,
                    Some(execution),
                )?;
                Message::InputAck {
                    generation,
                    input_seq,
                    outcome: receipt.outcome,
                    turn: if receipt_requested {
                        receipt.turn
                    } else {
                        None
                    },
                }
            }
            Message::Interrupt { authority }
                if authority.host == context.host_generation
                    && authority.controller == Some(generation) =>
            {
                let (control, action, grant) =
                    self.agent_action(context.client, session, generation, None, Some(execution))?;
                let outcome = control.request(action, grant)?;
                Message::ActionResult {
                    action: wire::SessionAction::Interrupt,
                    outcome,
                }
            }
            _ => {
                return Err(RemoteError::rejected(
                    ErrorCode::InvalidFrame,
                    "invalid agent mutation authority",
                ));
            }
        };
        execution.fence.check().map_err(|_| {
            RemoteError::unknown("agent effect reply lost original access; do not resend")
        })?;
        let mut reply = wire::Envelope::new(body);
        reply.re = Some(request.id);
        reply.machine = request.machine;
        reply.session = Some(session);
        Ok(DispatchFrame::RemoteReply(Box::new(reply)))
    }

    pub(super) fn agent_action(
        &self,
        client: ClientId,
        id: SessionId,
        generation: Counter,
        input: Option<SessionInput>,
        remote: Option<&remote_effects::Execution<'_>>,
    ) -> Result<(Control, ActionRequest, Arc<dyn NativeEffectAuthorization>), RemoteError> {
        let owned = self.session(id)?;
        let mut state = lock(&owned)?;
        check_control(
            &mut state,
            client,
            generation,
            if input.is_some() {
                Action::Input
            } else {
                Action::Interrupt
            },
            remote,
        )?;
        let control = state.agent_control.clone().ok_or_else(|| {
            RemoteError::rejected(ErrorCode::UnsupportedCapability, "no owned managed agent")
        })?;
        let thread = state
            .info
            .agent
            .as_ref()
            .and_then(|agent| agent.native_thread.clone())
            .ok_or_else(|| RemoteError::unknown("native thread correlation unavailable"))?;
        let action = match input {
            Some(SessionInput::Prompt(text) | SessionInput::PromptReceipt(text)) => {
                ActionRequest::Prompt { thread, text }
            }
            Some(SessionInput::Steer(text)) => ActionRequest::Steer {
                thread,
                text,
                turn: lock(&control.turn)?.clone().ok_or_else(|| {
                    RemoteError::rejected(
                        ErrorCode::UnsupportedCapability,
                        "no observed active native turn",
                    )
                })?,
            },
            None => ActionRequest::Interrupt {
                thread,
                turn: lock(&control.turn)?.clone().ok_or_else(|| {
                    RemoteError::rejected(
                        ErrorCode::UnsupportedCapability,
                        "no observed active native turn",
                    )
                })?,
            },
            _ => {
                return Err(RemoteError::rejected(
                    ErrorCode::UnsupportedCapability,
                    "managed agent input must be structured",
                ));
            }
        };
        let grant = Arc::new(Grant {
            session: Arc::downgrade(&owned),
            client,
            generation,
            deadline: state.deadline.ok_or_else(|| {
                RemoteError::rejected(ErrorCode::StaleGeneration, "missing controller deadline")
            })?,
            elapsed_deadline: state.deadline_elapsed.ok_or_else(|| {
                RemoteError::rejected(ErrorCode::StaleGeneration, "missing controller clock")
            })?,
            remote: remote.map(|execution| (execution.context.clone(), execution.fence.clone())),
        });
        Ok((control, action, grant))
    }

    pub(super) fn agent_input(
        &self,
        client: ClientId,
        id: SessionId,
        generation: Counter,
        input_seq: Counter,
        input: SessionInput,
        remote: Option<&remote_effects::Execution<'_>>,
    ) -> Result<InputReceipt, RemoteError> {
        let text = match &input {
            SessionInput::Prompt(text)
            | SessionInput::PromptReceipt(text)
            | SessionInput::Steer(text) => text,
            _ => {
                return Err(RemoteError::rejected(
                    ErrorCode::UnsupportedCapability,
                    "agent requires prompt or steer",
                ));
            }
        };
        if text.is_empty() || text.len() > 32768 || text.contains('\0') {
            return Err(RemoteError::rejected(
                ErrorCode::InvalidFrame,
                "invalid agent text",
            ));
        }
        let encoded = serde_json::to_vec(&input).map_err(storage_error)?;
        use hmac::{Hmac, Mac};
        let mut digest = Hmac::<sha2::Sha256>::new_from_slice(self.input_key.as_slice())
            .map_err(storage_error)?;
        digest.update(b"qgenie-remote:agent-input:v1\0");
        digest.update(id.to_string().as_bytes());
        digest.update(&generation.0.to_be_bytes());
        digest.update(&input_seq.0.to_be_bytes());
        digest.update(&encoded);
        let digest = digest
            .finalize()
            .into_bytes()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let request = self.agent_action(client, id, generation, Some(input), remote);
        let owned = self.session(id)?;
        {
            let mut state = lock(&owned)?;
            check_control(&mut state, client, generation, Action::Input, remote)?;
            if let Some(outcome) = lock(&self.store)?
                .input_receipt(id, generation, input_seq, &digest)
                .map_err(|_| {
                    RemoteError::rejected(
                        ErrorCode::IdempotencyConflict,
                        "agent input identity conflicts",
                    )
                })?
            {
                check_control(&mut state, client, generation, Action::Input, remote)?;
                return Ok(outcome);
            }
            if state.input_seq.checked_add(1) != Some(input_seq.0) {
                return Err(RemoteError::rejected(
                    ErrorCode::StaleGeneration,
                    "agent input sequence is not next",
                ));
            }
            lock(&self.store)?
                .input_record(
                    id,
                    generation,
                    input_seq,
                    &digest,
                    ApplicationOutcome::OutcomeUnknown,
                )
                .map_err(storage_error)?;
            state.input_seq = input_seq.0;
            if let Some(lease) = &mut state.info.controller {
                lease.last_input = input_seq;
            }
        }
        // SQLite waits cannot refresh a queued original grant. Recheck and capture
        // both deadlines before dispatch; actual writes check them again.
        let receipt = match request {
            Ok((control, action, grant)) => match control.request_receipt(action, grant) {
                Ok(outcome) => outcome,
                Err(error) if matches!(error.outcome, OperationOutcome::Rejected) => InputReceipt {
                    outcome: ApplicationOutcome::Cancelled,
                    turn: None,
                },
                Err(_) => InputReceipt {
                    outcome: ApplicationOutcome::OutcomeUnknown,
                    turn: None,
                },
            },
            Err(_) => InputReceipt {
                outcome: ApplicationOutcome::Cancelled,
                turn: None,
            },
        };
        if lock(&self.store)?
            .record_input_receipt(id, generation, input_seq, &digest, receipt.clone())
            .is_err()
        {
            return Ok(InputReceipt {
                outcome: ApplicationOutcome::OutcomeUnknown,
                turn: None,
            });
        }
        Ok(receipt)
    }
}
