//! Explicit native control; dropping/reconnecting never resends an input or takes over.
use crate::LiveHandle;
use base64::{Engine, engine::general_purpose::STANDARD};
use qgenie_remote_proto::{
    wire::{self, ControlLease, Envelope, Message, SessionAction},
    *,
};

pub struct RemoteController {
    handle: LiveHandle,
    machine: MachineId,
    session: SessionId,
    lease: ControlLease,
    next: Counter,
    uncertain: bool,
}
fn refused(code: ErrorCode, message: &str) -> RemoteError {
    RemoteError {
        code,
        outcome: OperationOutcome::NotSent,
        message: message.into(),
        retryable: false,
    }
}
fn lease(handle: &LiveHandle, reply: Option<Envelope>) -> Result<ControlLease, RemoteError> {
    match reply.map(|v| v.message) {
        Some(Message::LeaseGranted(lease))
            if lease.holder == handle.id()
                && lease.host_generation.0 > 0
                && lease.generation.0 > 0 =>
        {
            Ok(lease)
        }
        _ => Err(RemoteError::unknown(
            "native control activation was not confirmed; reacquire explicitly",
        )),
    }
}
impl LiveHandle {
    /// Current route metadata, never a mutation permit or a controller lease.
    pub async fn host_generation(
        &self,
        machine: MachineId,
        session: SessionId,
    ) -> Result<Counter, RemoteError> {
        self.control_capability()?;
        match self
            .session_action(machine, session, Message::HostGenerationQuery)
            .await?
            .message
        {
            Message::HostGeneration { generation } if generation.0 > 0 => Ok(generation),
            _ => Err(RemoteError::unknown(
                "current native host generation was not confirmed",
            )),
        }
    }
    pub async fn terminate_current(
        &self,
        machine: MachineId,
        session: SessionId,
        confirmed: bool,
        external_confirmed: bool,
    ) -> Result<(), RemoteError> {
        if !confirmed {
            return Err(refused(
                ErrorCode::Forbidden,
                "explicit termination confirmation is required",
            ));
        }
        let host = self.host_generation(machine, session).await?;
        self.terminate(machine, session, host, confirmed, external_confirmed)
            .await
    }
    fn control_capability(&self) -> Result<(), RemoteError> {
        if !self.controls {
            return Err(refused(
                ErrorCode::UnsupportedCapability,
                "gateway did not negotiate session-control-v1",
            ));
        }
        Ok(())
    }
    async fn session_action(
        &self,
        machine: MachineId,
        session: SessionId,
        body: Message,
    ) -> Result<Envelope, RemoteError> {
        self.control_capability()?;
        let mut request = Envelope::new(body);
        request.machine = Some(machine);
        request.session = Some(session);
        self.submit(request)
            .await?
            .pop()
            .ok_or_else(|| RemoteError::unknown("native action result missing"))
    }
    pub async fn take_control(
        &self,
        machine: MachineId,
        session: SessionId,
        takeover: bool,
    ) -> Result<RemoteController, RemoteError> {
        let activated = lease(
            self,
            Some(
                self.session_action(machine, session, Message::LeaseRequest { takeover })
                    .await?,
            ),
        )?;
        let next = activated
            .last_input
            .0
            .checked_add(1)
            .ok_or_else(|| refused(ErrorCode::CapacityExhausted, "input sequence exhausted"))?;
        Ok(RemoteController {
            handle: self.clone(),
            machine,
            session,
            lease: activated,
            next: Counter(next),
            uncertain: false,
        })
    }
    /// Owner authorization is independent of the controller lease. Confirmation
    /// remains explicit; native scope and supported termination policy are mandatory.
    pub async fn terminate(
        &self,
        machine: MachineId,
        session: SessionId,
        host_generation: Counter,
        confirmed: bool,
        external_confirmed: bool,
    ) -> Result<(), RemoteError> {
        let reply = self
            .session_action(
                machine,
                session,
                Message::Terminate {
                    host_generation,
                    confirmed,
                    external_confirmed,
                },
            )
            .await?;
        action_result(reply, SessionAction::Terminate)
    }
}
impl RemoteController {
    pub fn session(&self) -> SessionId {
        self.session
    }
    pub fn lease(&self) -> &ControlLease {
        &self.lease
    }
    pub fn input_uncertain(&self) -> bool {
        self.uncertain
    }
    fn authority(&self) -> wire::Generations {
        wire::Generations {
            host: self.lease.host_generation,
            controller: Some(self.lease.generation),
        }
    }
    pub async fn input(&mut self, bytes: &[u8]) -> Result<(), RemoteError> {
        if bytes.is_empty() || bytes.len() > MAX_PTY_BYTES {
            return Err(refused(
                ErrorCode::FrameTooLarge,
                "input must contain 1..=32768 bytes",
            ));
        }
        self.send_input(wire::SessionInput::Pty(STANDARD.encode(bytes)))
            .await
    }
    pub async fn prompt(&mut self, text: &str) -> Result<(), RemoteError> {
        self.agent_input(text, false).await
    }
    pub async fn steer(&mut self, text: &str) -> Result<(), RemoteError> {
        self.agent_input(text, true).await
    }
    async fn agent_input(&mut self, text: &str, steer: bool) -> Result<(), RemoteError> {
        if !self.handle.agents {
            return Err(refused(
                ErrorCode::UnsupportedCapability,
                "gateway did not negotiate agent-events-v1",
            ));
        }
        if text.is_empty() || text.len() > 32768 || text.contains('\0') {
            return Err(refused(
                ErrorCode::FrameTooLarge,
                "agent input must contain 1..=32768 bytes",
            ));
        }
        self.send_input(if steer {
            wire::SessionInput::Steer(text.into())
        } else {
            wire::SessionInput::Prompt(text.into())
        })
        .await
    }
    async fn send_input(&mut self, input: wire::SessionInput) -> Result<(), RemoteError> {
        if self.uncertain {
            return Err(refused(
                ErrorCode::Internal,
                "previous input is uncertain; release and reacquire control explicitly",
            ));
        }
        // Poison before the first await, including cancellation and lost replies.
        self.uncertain = true;
        let reply = self
            .handle
            .session_action(
                self.machine,
                self.session,
                Message::Input {
                    authority: self.authority(),
                    input_seq: self.next,
                    input,
                },
            )
            .await?;
        match reply.message {
            Message::InputAck {
                generation,
                input_seq,
                outcome: ApplicationOutcome::Applied | ApplicationOutcome::AlreadyApplied,
            } if generation == self.lease.generation && input_seq == self.next => {
                self.lease.last_input = input_seq;
                self.next = Counter(self.next.0.checked_add(1).ok_or_else(|| {
                    refused(ErrorCode::CapacityExhausted, "input sequence exhausted")
                })?);
                self.uncertain = false;
                Ok(())
            }
            _ => Err(RemoteError::unknown(
                "native input application is uncertain; bytes were not retried",
            )),
        }
    }
    pub async fn renew(&mut self) -> Result<(), RemoteError> {
        let next = lease(
            &self.handle,
            Some(
                self.handle
                    .session_action(
                        self.machine,
                        self.session,
                        Message::LeaseRenew {
                            generation: self.lease.generation,
                        },
                    )
                    .await?,
            ),
        )?;
        if next.host_generation != self.lease.host_generation
            || next.generation != self.lease.generation
            || next.last_input < self.lease.last_input
        {
            return Err(RemoteError::unknown(
                "native lease renewal differs; reacquire explicitly",
            ));
        }
        self.lease = next;
        Ok(())
    }
    pub async fn release(self) -> Result<(), RemoteError> {
        match self
            .handle
            .session_action(
                self.machine,
                self.session,
                Message::LeaseRelease {
                    generation: self.lease.generation,
                },
            )
            .await?
            .message
        {
            Message::LeaseRevoked { generation, .. } if generation == self.lease.generation => {
                Ok(())
            }
            _ => Err(RemoteError::unknown(
                "native release is uncertain; reacquire explicitly",
            )),
        }
    }
    pub async fn resize(&self, rows: u16, cols: u16) -> Result<(), RemoteError> {
        let reply = self
            .handle
            .session_action(
                self.machine,
                self.session,
                Message::Resize {
                    authority: self.authority(),
                    rows,
                    cols,
                },
            )
            .await?;
        action_result(reply, SessionAction::Resize)
    }
    pub async fn interrupt(&self) -> Result<(), RemoteError> {
        let reply = self
            .handle
            .session_action(
                self.machine,
                self.session,
                Message::Interrupt {
                    authority: self.authority(),
                },
            )
            .await?;
        action_result(reply, SessionAction::Interrupt)
    }
}
fn action_result(reply: Envelope, expected: SessionAction) -> Result<(), RemoteError> {
    match reply.message {
        Message::ActionResult {
            action,
            outcome: ApplicationOutcome::Applied | ApplicationOutcome::AlreadyApplied,
        } if std::mem::discriminant(&action) == std::mem::discriminant(&expected) => Ok(()),
        _ => Err(RemoteError::unknown(
            "native action application is uncertain; action was not retried",
        )),
    }
}
