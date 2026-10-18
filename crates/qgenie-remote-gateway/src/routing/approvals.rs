//! Owner reads reconcile durable approval state independently of an attachment or host.
use super::hub::{Client, Hub};
use crate::Error;
use qgenie_remote_proto::{
    ApprovalId, ClientId, UserId,
    wire::{Envelope, Message},
};
use qgenie_remote_transport::ws::{Frame, Priority};
use std::time::Duration;
mod delivery;
impl Hub {
    pub(super) async fn approval_resolve(
        &self,
        client: ClientId,
        owner: UserId,
        request: &Envelope,
    ) -> Result<(), Error> {
        if !self.approval_status_enabled || request.re.is_some() {
            return Err(Error::Unsupported);
        }
        let Message::ApprovalResolve {
            approval,
            request_hash,
            decision,
        } = &request.message
        else {
            return Err(Error::Invalid);
        };
        let peer = self
            .peers
            .lock()
            .unwrap()
            .clients
            .get(&client)
            .cloned()
            .ok_or(Error::NotFound)?;
        if peer.owner != owner || *peer.stop.borrow() {
            return Err(Error::Forbidden);
        }
        let (_, deadline) = peer.gate.budget()?;
        let scope = self.gateway.store.approval_scope(owner, *approval).await?;
        if request.machine != Some(scope.machine) || request.session != Some(scope.session) {
            return Err(Error::Forbidden);
        }
        let guard = || {
            if *peer.stop.borrow()
                || qgenie_remote_transport::elapsed::now().map_err(|_| Error::Unavailable)?
                    >= deadline
            {
                return Err(Error::Unavailable);
            }
            peer.gate.budget().map(|_| ())
        };
        let result = self
            .gateway
            .store
            .choose_approval_guarded(&scope, client, *approval, request_hash, *decision, &guard)
            .await;
        let frame = match result {
            Ok(choice) => {
                self.wake.notify_one();
                let _ = self.approval_notify(&scope, *approval).await;
                if guard().is_err() {
                    super::route::error_frame(Some(request.id), Error::Unavailable, true)
                } else {
                    let mut reply = Envelope::new(Message::ApprovalCommitted(choice.winner));
                    reply.re = Some(request.id);
                    reply.machine = request.machine;
                    reply.session = request.session;
                    Frame::Text(Box::new(reply))
                }
            }
            Err(error) => {
                let unknown = matches!(error, Error::Database | Error::Unavailable);
                super::route::error_frame(Some(request.id), error, unknown)
            }
        };
        self.client_send(client, frame, Priority::Control)
    }
    pub(super) async fn approval_snapshot(
        &self,
        actor: &crate::MachinePrincipal,
        generation: qgenie_remote_proto::Counter,
        host: &super::hub::Host,
        session: qgenie_remote_proto::SessionId,
        requests: &[qgenie_remote_proto::wire::ApprovalRequest],
    ) -> Result<(), Error> {
        if !self.approval_status_enabled || !host.approvals || requests.len() > 32 {
            return Err(Error::Unsupported);
        }
        let scope = crate::ControllerScope {
            owner: actor.owner,
            machine: actor.machine,
            session,
            host_generation: generation,
            host_connection: host.id,
        };
        let mut present = std::collections::BTreeSet::new();
        for request in requests {
            if !present.insert(request.id) {
                return Err(Error::Invalid);
            }
            if self
                .gateway
                .store
                .register_approval_once(&scope, request)
                .await?
            {
                let _ = self.approval_notify(&scope, request.id).await;
            }
        }
        let present = present.into_iter().collect::<Vec<_>>();
        let cancelled = self
            .gateway
            .store
            .reconcile_native_approvals(&scope, &present)
            .await?;
        for id in cancelled {
            let _ = self.approval_notify(&scope, id).await;
        }
        Ok(())
    }
    pub(super) async fn approval_emit(
        &self,
        owner: UserId,
        machine: qgenie_remote_proto::MachineId,
        session: qgenie_remote_proto::SessionId,
        approval: ApprovalId,
    ) -> Result<(), Error> {
        let state = self
            .gateway
            .store
            .owner_approval_status(owner, approval)
            .await?;
        if state.machine != machine || state.request.session != session {
            return Err(Error::Forbidden);
        }
        let mut messages = Vec::new();
        if let Some(winner) = state.winner {
            if let Some(outcome) = state.outcome {
                messages.push(Message::ApprovalApplication {
                    approval,
                    decision: winner.id,
                    outcome,
                    reason: state.reason,
                });
            }
            messages.insert(0, Message::ApprovalCommitted(winner));
        } else if state.cancelled {
            messages.push(Message::ApprovalCancelled {
                approval,
                reason: state
                    .reason
                    .unwrap_or_else(|| "Native request is no longer pending".into()),
            });
        } else {
            messages.push(Message::ApprovalRequested(state.request));
        }
        for message in messages {
            let mut event = Envelope::new(message);
            event.machine = Some(machine);
            event.session = Some(session);
            self.fanout(
                owner,
                machine,
                Some(session),
                super::message::Data::Json(Box::new(event)),
            )
            .await?;
        }
        Ok(())
    }
    async fn approval_notify(
        &self,
        scope: &crate::ControllerScope,
        approval: ApprovalId,
    ) -> Result<(), Error> {
        let _ = self
            .approval_emit(scope.owner, scope.machine, scope.session, approval)
            .await;
        let packet = super::message::Packet {
            id: qgenie_remote_proto::MessageId::new(),
            source: self.node.clone(),
            destination: super::broker::user_channel(scope.owner),
            at_ms: qgenie_remote_proto::Counter(super::now_ms()?),
            body: super::message::Body::ApprovalChanged {
                owner: scope.owner,
                machine: scope.machine,
                session: scope.session,
                approval,
            },
        };
        self.broker
            .publish(&packet, Priority::Event)
            .await
            .map(|_| ())
    }
    fn approval_reader(
        &self,
        client: ClientId,
        owner: UserId,
        request: &Envelope,
    ) -> Result<(Client, Duration), Error> {
        if !self.approval_status_enabled
            || request.re.is_some()
            || request.machine.is_some()
            || request.session.is_some()
        {
            return Err(Error::Unsupported);
        }
        let peer = self
            .peers
            .lock()
            .unwrap()
            .clients
            .get(&client)
            .cloned()
            .ok_or(Error::NotFound)?;
        if peer.owner != owner || *peer.stop.borrow() {
            return Err(Error::Forbidden);
        }
        let (_, deadline) = peer.gate.budget()?;
        Ok((peer, deadline))
    }
    fn approval_read_reply(
        &self,
        peer: &Client,
        deadline: Duration,
        client: ClientId,
        request: &Envelope,
        message: Message,
    ) -> Result<(), Error> {
        if *peer.stop.borrow()
            || qgenie_remote_transport::elapsed::now().map_err(|_| Error::Unavailable)? >= deadline
        {
            return Err(Error::Unavailable);
        }
        peer.gate.budget()?;
        let mut reply = Envelope::new(message);
        reply.re = Some(request.id);
        self.client_send(client, Frame::Text(Box::new(reply)), Priority::Event)
    }
    pub(super) async fn approval_status(
        &self,
        client: ClientId,
        owner: UserId,
        request: &Envelope,
        approval: ApprovalId,
    ) -> Result<(), Error> {
        let (peer, deadline) = self.approval_reader(client, owner, request)?;
        let status = self
            .gateway
            .store
            .owner_approval_status(owner, approval)
            .await?;
        self.approval_read_reply(
            &peer,
            deadline,
            client,
            request,
            Message::ApprovalStatus(status),
        )
    }
    pub(super) async fn approval_index(
        &self,
        client: ClientId,
        owner: UserId,
        request: &Envelope,
        after: Option<ApprovalId>,
        limit: u16,
    ) -> Result<(), Error> {
        let (peer, deadline) = self.approval_reader(client, owner, request)?;
        let states = self
            .gateway
            .store
            .list_approvals(owner, after, limit)
            .await?;
        let next = if states.len() == usize::from(limit) {
            states.last().map(|state| state.request.id)
        } else {
            None
        };
        self.approval_read_reply(
            &peer,
            deadline,
            client,
            request,
            Message::ApprovalPage { states, next },
        )
    }
}
