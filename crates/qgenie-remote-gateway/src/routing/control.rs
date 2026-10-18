//! Host-side control RPCs are consumed here, never forwarded to owner clients.
use super::{
    hub::{Host, Hub},
    now_ms,
};
use crate::{ControllerRequest, ControllerScope, Error, MachinePrincipal};
use qgenie_remote_proto::{
    wire::{
        ControlLease, Envelope, HostAccess, HostContext, HostControlRequest as Request,
        HostControlResponse as Response, Message, Timestamp,
    },
    *,
};
use qgenie_remote_transport::ws::{Frame, Priority};
use tokio::{
    sync::oneshot,
    time::{Duration, Instant},
};

pub(super) struct Waiting {
    expected: Expected,
    owner: UserId,
    machine: MachineId,
    session: SessionId,
    generation: Counter,
    connection: ClientId,
    deadline: Instant,
    send: oneshot::Sender<Envelope>,
}
enum Expected {
    Control,
    Approval(ApprovalId, DecisionId),
}
impl Expected {
    fn accepts(&self, message: &Message) -> bool {
        matches!(
            (self, message),
            (_, Message::Error(_)) | (Self::Control, Message::HostControlResponse(_))
        ) || matches!((self,message), (Self::Approval(expected,decision),Message::ApprovalApplication{approval,decision:actual,..}) if expected==approval && decision==actual)
    }
}
struct Registration<'a> {
    hub: &'a Hub,
    id: MessageId,
}
impl Drop for Registration<'_> {
    fn drop(&mut self) {
        self.hub.controls.lock().unwrap().remove(&self.id);
    }
}
impl Hub {
    pub(super) async fn routed_response(
        &self,
        source: &str,
        owner: UserId,
        client: ClientId,
        machine: MachineId,
        generation: Counter,
        frame: super::message::Data,
    ) -> Result<(), Error> {
        if source == self.node {
            return self.response(owner, client, machine, frame).await;
        }
        let packet = super::message::Packet {
            id: MessageId::new(),
            source: self.node.clone(),
            destination: super::broker::node_channel(source),
            at_ms: Counter(now_ms()?),
            body: super::message::Body::Response {
                owner,
                client,
                machine,
                generation,
                frame,
            },
        };
        self.broker.publish(&packet, Priority::Control).await?;
        Ok(())
    }
    pub(super) fn control_response(
        &self,
        actor: &MachinePrincipal,
        generation: Counter,
        host: &Host,
        reply: &Envelope,
    ) -> Result<bool, Error> {
        let Some(id) = reply.re else { return Ok(false) };
        let mut pending = self.controls.lock().unwrap();
        let Some(wait) = pending.get(&id) else {
            return Ok(matches!(
                reply.message,
                Message::HostControlResponse(_) | Message::ApprovalApplication { .. }
            ));
        };
        if wait.owner != actor.owner
            || wait.machine != actor.machine
            || reply.machine != Some(actor.machine)
            || wait.session != reply.session.ok_or(Error::Invalid)?
            || wait.generation != generation
            || wait.connection != host.id
        {
            return Err(Error::Forbidden);
        }
        if !wait.expected.accepts(&reply.message) {
            return Err(Error::Invalid);
        }
        reply.validate().map_err(|_| Error::Invalid)?;
        let wait = pending.remove(&id).ok_or(Error::Unavailable)?;
        if wait.deadline > Instant::now() {
            let _ = wait.send.send(reply.clone());
        }
        Ok(true)
    }
    async fn control_rpc(
        &self,
        host: &Host,
        context: &HostContext,
        session: SessionId,
        request: Request,
        deadline: Instant,
    ) -> Result<Response, Error> {
        let reply = self
            .native_rpc(
                host,
                context,
                session,
                Message::HostControlRequest(request),
                deadline,
            )
            .await?;
        match reply.message {
            Message::HostControlResponse(response) => Ok(response),
            Message::Error(error) => Err(match error.code {
                ErrorCode::ControlHeld => Error::ControlHeld,
                ErrorCode::StaleGeneration => Error::StaleGeneration,
                ErrorCode::Forbidden => Error::Forbidden,
                ErrorCode::IdempotencyConflict => Error::Conflict,
                _ => Error::Unavailable,
            }),
            _ => Err(Error::Invalid),
        }
    }
    pub(super) async fn native_rpc(
        &self,
        host: &Host,
        context: &HostContext,
        session: SessionId,
        request: Message,
        deadline: Instant,
    ) -> Result<Envelope, Error> {
        if !host.native {
            return Err(Error::Unsupported);
        }
        let expected = match &request {
            Message::HostControlRequest(_) if host.controllers => Expected::Control,
            Message::ApprovalCommitted(winner) => Expected::Approval(winner.approval, winner.id),
            _ => return Err(Error::Unsupported),
        };
        if *host.stop.borrow()
            || context.actor != host.actor.owner
            || context.host_generation != host.generation
            || context.expires_at.unix_millis() <= now_ms()?
        {
            return Err(Error::Forbidden);
        }
        host.gate.budget()?;
        host.grants
            .lock()
            .unwrap()
            .accepts(context.access.as_ref().ok_or(Error::Unavailable)?)?;
        let mut message = Envelope::new(request);
        message.machine = Some(host.actor.machine);
        message.session = Some(session);
        let id = message.id;
        let (send, receive) = oneshot::channel();
        {
            let mut pending = self.controls.lock().unwrap();
            if pending.len() >= 4096 {
                return Err(Error::Overloaded);
            }
            pending.insert(
                id,
                Waiting {
                    expected,
                    owner: context.actor,
                    machine: host.actor.machine,
                    session,
                    generation: host.generation,
                    connection: host.id,
                    deadline,
                    send,
                },
            );
        }
        let _registered = Registration { hub: self, id };
        let mut envelope = Envelope::new(Message::HostDispatch {
            context: context.clone(),
            message: Box::new(message),
        });
        envelope.machine = Some(host.actor.machine);
        Self::deliver(
            &host.sender,
            &host.stop,
            Frame::Text(Box::new(envelope)),
            Priority::Control,
        )?;
        let reply = tokio::time::timeout_at(deadline, receive)
            .await
            .map_err(|_| Error::Unavailable)?
            .map_err(|_| Error::Unavailable)?;
        host.gate.budget()?;
        host.grants
            .lock()
            .unwrap()
            .accepts(context.access.as_ref().ok_or(Error::Unavailable)?)?;
        Ok(reply)
    }
    pub(super) async fn controller(
        &self,
        host: &Host,
        client: ClientId,
        expires: u64,
        access: HostAccess,
        request: &Envelope,
    ) -> Result<Message, Error> {
        let owner = host.actor.owner;
        let session = request.session.ok_or(Error::Invalid)?;
        let deadline = Instant::now() + Duration::from_secs(8);
        let context = HostContext {
            actor: owner,
            client,
            host_generation: host.generation,
            expires_at: Timestamp::from_unix_millis(expires.saturating_mul(1000))
                .map_err(|_| Error::Invalid)?,
            access: Some(access),
        };
        let scope = ControllerScope {
            owner,
            machine: host.actor.machine,
            session,
            host_generation: host.generation,
            host_connection: host.id,
        };
        let store = &self.gateway.store;
        let (proposal, takeover, renewal) = match request.message {
            Message::LeaseRelease { generation } => {
                let Some(previous) = store
                    .current_controller(&scope, client, generation, false)
                    .await?
                else {
                    return Ok(Message::LeaseRevoked {
                        generation,
                        reason: "released".into(),
                    });
                };
                match self
                    .control_rpc(
                        host,
                        &context,
                        session,
                        Request::Release { generation },
                        deadline,
                    )
                    .await?
                {
                    Response::Released { generation: actual } if actual == generation => {}
                    _ => return Err(Error::Invalid),
                }
                store.release_controller(&previous).await?;
                return Ok(Message::LeaseRevoked {
                    generation,
                    reason: "released".into(),
                });
            }
            Message::LeaseRequest { takeover } => {
                let Response::State { floor, holder, .. } = self
                    .control_rpc(host, &context, session, Request::Query, deadline)
                    .await?
                else {
                    return Err(Error::Invalid);
                };
                (
                    store
                        .propose_controller(ControllerRequest {
                            scope,
                            holder: client,
                            request: request.id,
                            native_floor: floor,
                            native_holder: holder,
                            takeover,
                        })
                        .await?,
                    takeover,
                    false,
                )
            }
            Message::LeaseRenew { generation } => {
                let previous = store
                    .current_controller(&scope, client, generation, true)
                    .await?
                    .ok_or(Error::StaleGeneration)?;
                let Response::State { floor, holder, .. } = self
                    .control_rpc(host, &context, session, Request::Query, deadline)
                    .await?
                else {
                    return Err(Error::Invalid);
                };
                (
                    store
                        .renew_controller(&previous, request.id, floor, holder)
                        .await?,
                    false,
                    true,
                )
            }
            _ => return Err(Error::Invalid),
        };
        let generation = proposal.generation;
        let Response::Activated {
            holder,
            generation: actual,
            remaining_ms,
            last_input,
        } = self
            .control_rpc(
                host,
                &context,
                session,
                Request::Activate {
                    generation,
                    remaining_ms: proposal.remaining_budget()?,
                    takeover,
                    renewal,
                },
                deadline,
            )
            .await?
        else {
            return Err(Error::Invalid);
        };
        if holder != client || actual != generation {
            return Err(Error::Forbidden);
        }
        let confirmed = store.confirm_controller(&proposal, remaining_ms).await?;
        if Instant::now() >= deadline {
            return Err(Error::Unavailable);
        }
        host.gate.budget()?;
        host.grants
            .lock()
            .unwrap()
            .accepts(context.access.as_ref().ok_or(Error::Unavailable)?)?;
        // Advertise only the earlier primary/native boundary. This wall time is
        // presentation; native and primary elapsed deadlines enforce authority.
        let remaining = confirmed.remaining_budget()?;
        Ok(Message::LeaseGranted(ControlLease {
            holder,
            host_generation: host.generation,
            generation,
            expires_at: Timestamp::from_unix_millis(now_ms()?.saturating_add(u64::from(remaining)))
                .map_err(|_| Error::Unavailable)?,
            last_input,
        }))
    }
}
