use super::{
    broker::{node_channel, user_channel},
    hub::{Hub, Pending},
    message::{Body, Data, Packet},
    now_ms,
};
use crate::{Error, MachinePrincipal};
use qgenie_remote_proto::{
    ClientId, Counter, ErrorCode, MachineId, MessageId, OperationOutcome, PtyPayload, RemoteError,
    wire::{Envelope, Message},
};
use qgenie_remote_transport::ws::{Frame, Priority};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};

fn native_continuation(message: &Message) -> bool {
    matches!(
        message,
        Message::Input { .. } | Message::Capture | Message::ReplayRequest { .. }
    )
}

pub(super) fn error_frame(re: Option<MessageId>, error: Error, unknown: bool) -> Frame {
    let code = match error {
        Error::Unsupported => ErrorCode::UnsupportedCapability,
        Error::ControlHeld => ErrorCode::ControlHeld,
        Error::StaleGeneration => ErrorCode::StaleGeneration,
        Error::Forbidden => ErrorCode::Forbidden,
        Error::NotFound => ErrorCode::NotFound,
        Error::Unauthenticated => ErrorCode::Unauthenticated,
        Error::Overloaded => ErrorCode::Overloaded,
        Error::Invalid => ErrorCode::InvalidFrame,
        Error::Conflict => ErrorCode::IdempotencyConflict,
        Error::AmbiguousName => ErrorCode::AmbiguousName,
        _ => ErrorCode::AccessStale,
    };
    let mut envelope = Envelope::new(Message::Error(RemoteError {
        code,
        outcome: if matches!(error, Error::Unsupported) {
            OperationOutcome::NotSent
        } else if unknown {
            OperationOutcome::Unknown
        } else {
            OperationOutcome::Rejected
        },
        message: error.to_string(),
        retryable: false,
    }));
    envelope.re = re;
    Frame::Text(Box::new(envelope))
}
impl Hub {
    /// Choose the nonce first, then cap its original budget before any routing
    /// queue. Destination gateways must preserve this identity/budget.
    fn bind_access(
        &self,
        client: ClientId,
        access: Option<qgenie_remote_proto::wire::HostAccess>,
    ) -> Result<Option<qgenie_remote_proto::wire::HostAccess>, Error> {
        let peer = self
            .peers
            .lock()
            .unwrap()
            .clients
            .get(&client)
            .cloned()
            .ok_or(Error::NotFound)?;
        if *peer.stop.borrow() {
            return Err(Error::Forbidden);
        }
        let budget = peer.gate.budget()?.0;
        Ok(access.map(|mut access| {
            access.remaining_ms = access.remaining_ms.min(budget);
            access
        }))
    }
    pub async fn client_frame(&self, client: ClientId, frame: Frame) -> Result<(), Error> {
        let actor = self
            .peers
            .lock()
            .unwrap()
            .clients
            .get(&client)
            .cloned()
            .ok_or(Error::NotFound)?;
        let continuation =
            matches!(&frame,Frame::Text(envelope) if native_continuation(&envelope.message));
        actor.gate.budget()?;
        if !continuation {
            self.gateway
                .identity
                .as_ref()
                .ok_or(Error::Configuration)?
                .assert_user(actor.owner)
                .await?;
        }
        let Frame::Text(envelope) = frame else {
            return self.unsupported(client, None);
        };
        let id = envelope.id;
        self.observations.request(&envelope.message);
        match &envelope.message {
            Message::ApprovalResolve { .. } => {
                return self.approval_resolve(client, actor.owner, &envelope).await;
            }
            Message::ApprovalIndex { after, limit } => {
                return self
                    .approval_index(client, actor.owner, &envelope, *after, *limit)
                    .await;
            }
            Message::ApprovalGet { approval } => {
                return self
                    .approval_status(client, actor.owner, &envelope, *approval)
                    .await;
            }
            Message::MachineList => return self.pages(client, actor.owner).await,
            Message::Detach => {
                self.peers
                    .lock()
                    .unwrap()
                    .clients
                    .get_mut(&client)
                    .ok_or(Error::NotFound)?
                    .subscriptions
                    .remove(&envelope.session.ok_or(Error::Invalid)?);
                return Ok(());
            }
            Message::SessionList
            | Message::SessionListPage { .. }
            | Message::SessionGet { .. }
            | Message::SessionResolve { .. }
            | Message::SessionMetadataUpdate { .. }
            | Message::Start(_)
            | Message::LeaseRequest { .. }
            | Message::LeaseRenew { .. }
            | Message::LeaseRelease { .. }
            | Message::Input { .. }
            | Message::Resize { .. }
            | Message::Interrupt { .. }
            | Message::Terminate { .. }
            | Message::HostGenerationQuery
            | Message::StartLookup { .. }
            | Message::Attach { .. }
            | Message::Capture
            | Message::ReplayRequest { .. }
            | Message::FsList { .. }
            | Message::FsRoots
            | Message::ApprovalList => {}
            _ => return self.unsupported(client, Some(id)),
        }
        if envelope.re.is_some() {
            return Err(Error::Invalid);
        }
        if !self.native_enabled
            && matches!(
                envelope.message,
                Message::LeaseRequest { .. }
                    | Message::LeaseRenew { .. }
                    | Message::LeaseRelease { .. }
                    | Message::Input { .. }
                    | Message::Resize { .. }
                    | Message::Interrupt { .. }
                    | Message::Terminate { .. }
                    | Message::Capture
                    | Message::ReplayRequest { .. }
                    | Message::HostGenerationQuery
            )
        {
            return self.unsupported(client, Some(id));
        }
        let machine = envelope.machine.ok_or(Error::Invalid)?;
        let session = envelope.session;
        if let Message::Start(_) = &envelope.message {
            if session.is_some() {
                return self.unsupported(client, Some(id));
            }
            if self.gateway.recordings.is_none() || !self.native_enabled {
                return Err(Error::Unavailable);
            }
        }
        if !continuation {
            self.gateway
                .store
                .route_access(actor.owner, machine, session)
                .await?;
        }
        let page_reply = match &envelope.message {
            Message::SessionMetadataUpdate { change } => {
                let session = envelope.session.ok_or(Error::Invalid)?;
                let updated = self
                    .gateway
                    .store
                    .update_session_metadata(actor.owner, machine, session, change, id)
                    .await;
                let updated = match updated {
                    Ok(updated) => updated,
                    Err(error @ (Error::Database | Error::Draining)) => {
                        let Frame::Text(mut reply) = error_frame(Some(id), error, true) else {
                            unreachable!()
                        };
                        reply.machine = Some(machine);
                        reply.session = Some(session);
                        return self.client_send(client, Frame::Text(reply), Priority::Control);
                    }
                    Err(error) => return Err(error),
                };
                self.metadata_changed(actor.owner, machine, session).await;
                Some(Message::SessionFound(updated))
            }
            Message::SessionResolve { name } => Some(Message::SessionFound(
                self.gateway
                    .store
                    .resolve_session_name(actor.owner, machine, name)
                    .await?,
            )),
            Message::SessionListPage {
                after,
                limit,
                active_only,
            } => {
                let (sessions, next) = self
                    .gateway
                    .store
                    .session_page(actor.owner, machine, *after, *limit, *active_only)
                    .await?;
                Some(Message::SessionPage { sessions, next })
            }
            Message::SessionGet { session } => Some(Message::SessionFound(
                self.gateway
                    .store
                    .session_get(actor.owner, machine, *session)
                    .await?,
            )),
            _ => None,
        };
        if let Some(body) = page_reply {
            let mut reply = Envelope::new(body);
            reply.re = Some(id);
            reply.machine = Some(machine);
            reply.session = envelope.session;
            return self.client_send(client, Frame::Text(Box::new(reply)), Priority::Event);
        }
        if matches!(envelope.message, Message::SessionList) {
            let sessions = self
                .gateway
                .store
                .session_projection(actor.owner, machine)
                .await?;
            let mut reply = Envelope::new(Message::Sessions { sessions });
            reply.re = Some(id);
            reply.machine = Some(machine);
            return self.client_send(client, Frame::Text(Box::new(reply)), Priority::Event);
        }
        if matches!(
            envelope.message,
            Message::Capture | Message::ReplayRequest { .. }
        ) && actor.subscriptions.get(&session.ok_or(Error::Invalid)?) != Some(&machine)
        {
            return Err(Error::Forbidden);
        }
        if matches!(envelope.message, Message::Attach { .. }) {
            let mut peers = self.peers.lock().unwrap();
            let peer = peers.clients.get_mut(&client).ok_or(Error::NotFound)?;
            if peer.subscriptions.len() >= 128 {
                return Err(Error::Overloaded);
            }
            peer.subscriptions
                .insert(session.ok_or(Error::Invalid)?, machine);
        }
        let pending = Pending {
            owner: actor.owner,
            client,
            machine,
            session,
            source: self.node.clone(),
            destination: None,
            deadline: Instant::now() + Duration::from_secs(10),
            replies: super::pending::Replies::new(&envelope.message),
        };
        {
            let mut peers = self.peers.lock().unwrap();
            if peers.clients_pending.len() >= 4096
                || peers
                    .clients_pending
                    .values()
                    .filter(|pending| pending.client == client)
                    .count()
                    >= 128
            {
                return Err(Error::Overloaded);
            }
            if peers.clients_pending.contains_key(&id) {
                return Err(Error::Conflict);
            }
            peers.clients_pending.insert(id, pending.clone());
        }
        let local = self.peers.lock().unwrap().hosts.get(&machine).cloned();
        let result = if let Some(host) = local {
            async {
                let access = if host.native {
                    Some(host.grants.lock().unwrap().current()?)
                } else {
                    None
                };
                let access = self.bind_access(client, access)?;
                self.dispatch(
                    &self.node,
                    actor.owner,
                    client,
                    actor.expires.load(Ordering::Acquire),
                    machine,
                    host.generation,
                    access,
                    Data::Json(envelope),
                )
                .await
            }
            .await
        } else {
            self.remote_request(
                actor.owner,
                client,
                actor.expires.load(Ordering::Acquire),
                machine,
                Data::Json(envelope),
            )
            .await
        };
        if let Err(error) = result {
            self.peers.lock().unwrap().clients_pending.remove(&id);
            self.client_send(
                client,
                error_frame(Some(id), error.clone(), matches!(error, Error::Unavailable)),
                Priority::Control,
            )?;
        }
        Ok(())
    }
    fn unsupported(&self, client: ClientId, re: Option<MessageId>) -> Result<(), Error> {
        let mut envelope = Envelope::new(Message::Error(RemoteError {
            code: ErrorCode::UnsupportedCapability,
            outcome: OperationOutcome::NotSent,
            message: "requested operation is unavailable on this gateway".into(),
            retryable: false,
        }));
        envelope.re = re;
        self.client_send(client, Frame::Text(Box::new(envelope)), Priority::Control)
    }
    async fn remote_request(
        &self,
        owner: qgenie_remote_proto::UserId,
        client: ClientId,
        expires: u64,
        machine: MachineId,
        frame: Data,
    ) -> Result<(), Error> {
        if !self
            .subscribed
            .lock()
            .unwrap()
            .contains(&user_channel(owner))
        {
            return Err(Error::Unavailable);
        }
        let continuation =
            matches!(&frame,Data::Json(envelope) if native_continuation(&envelope.message));
        let hint = self.broker.hint(machine).await?.ok_or(Error::NotFound)?;
        let Body::Hint {
            owner: hint_owner,
            machine: hint_machine,
            generation,
            connection,
            access: hint_access,
        } = hint.body
        else {
            return Err(Error::Invalid);
        };
        if owner != hint_owner || machine != hint_machine {
            return Err(Error::Forbidden);
        }
        if !continuation
            && !self
                .gateway
                .store
                .route_matches(machine, &hint.source, generation)
                .await?
        {
            return Err(Error::Unavailable);
        }
        let access = if continuation {
            let access = hint_access.ok_or(Error::Unavailable)?;
            access.validate().map_err(|_| Error::Invalid)?;
            if access.host_generation != generation || access.connection != connection {
                return Err(Error::Forbidden);
            }
            if let Data::Json(envelope) = &frame
                && let Some(pending) = self
                    .peers
                    .lock()
                    .unwrap()
                    .clients_pending
                    .get_mut(&envelope.id)
            {
                pending.destination = Some((hint.source.clone(), generation));
            }
            Some(access)
        } else {
            self.gateway
                .store
                .routed_native_access(owner, machine, &hint.source, generation)
                .await?
        };
        let access = self.bind_access(client, access)?;
        let packet = Packet {
            id: MessageId::new(),
            source: self.node.clone(),
            destination: node_channel(&hint.source),
            at_ms: Counter(now_ms()?),
            body: Body::Request {
                owner,
                client,
                expires: Counter(expires),
                machine,
                generation,
                access,
                frame,
            },
        };
        if self.broker.publish(&packet, Priority::Control).await? == 0 {
            return Err(Error::Unavailable);
        }
        if hint.source != self.node {
            self.cross_node_requests
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
        Ok(())
    }
    #[allow(clippy::too_many_arguments)]
    async fn dispatch(
        &self,
        source: &str,
        owner: qgenie_remote_proto::UserId,
        client: ClientId,
        expires: u64,
        machine: MachineId,
        generation: Counter,
        access: Option<qgenie_remote_proto::wire::HostAccess>,
        data: Data,
    ) -> Result<(), Error> {
        if expires.saturating_mul(1000) <= now_ms()? {
            return Err(Error::Unauthenticated);
        }
        let frame = data.frame()?;
        let Frame::Text(envelope) = &frame else {
            return Err(Error::Invalid);
        };
        if !matches!(
            envelope.message,
            Message::SessionList
                | Message::SessionListPage { .. }
                | Message::SessionGet { .. }
                | Message::Start(_)
                | Message::LeaseRequest { .. }
                | Message::LeaseRenew { .. }
                | Message::LeaseRelease { .. }
                | Message::Input { .. }
                | Message::Resize { .. }
                | Message::Interrupt { .. }
                | Message::Terminate { .. }
                | Message::HostGenerationQuery
                | Message::StartLookup { .. }
                | Message::Attach { .. }
                | Message::Capture
                | Message::ReplayRequest { .. }
                | Message::FsList { .. }
                | Message::FsRoots
                | Message::ApprovalList
        ) || envelope.machine != Some(machine)
            || envelope.re.is_some()
        {
            return Err(Error::Forbidden);
        }
        let continuation = native_continuation(&envelope.message);
        if !continuation {
            self.gateway
                .store
                .route_access(owner, machine, envelope.session)
                .await?;
        }
        let host = self
            .peers
            .lock()
            .unwrap()
            .hosts
            .get(&machine)
            .cloned()
            .ok_or(Error::NotFound)?;
        if host.actor.owner != owner
            || host.generation != generation
            || *host.stop.borrow()
            || host.expires.load(Ordering::Acquire).saturating_mul(1000) <= now_ms()?
            || (!continuation
                && !self
                    .gateway
                    .store
                    .host_current(&host.actor, generation)
                    .await?)
        {
            return Err(Error::Forbidden);
        }
        host.gate.budget()?;
        let id = envelope.id;
        if host.native {
            let access = access.as_ref().ok_or(Error::Unavailable)?;
            if access.connection != host.id || access.host_generation != generation {
                return Err(Error::Forbidden);
            }
            host.grants.lock().unwrap().accepts(access)?;
        } else if access.is_some() {
            return Err(Error::Forbidden);
        }
        if matches!(envelope.message, Message::HostGenerationQuery) {
            if !host.native || !host.controllers {
                return Err(Error::Unsupported);
            }
            let mut reply = Envelope::new(Message::HostGeneration { generation });
            reply.re = Some(id);
            reply.machine = Some(machine);
            reply.session = envelope.session;
            return self
                .routed_response(
                    source,
                    owner,
                    client,
                    machine,
                    generation,
                    Data::Json(Box::new(reply)),
                )
                .await;
        }
        if matches!(
            envelope.message,
            Message::LeaseRequest { .. }
                | Message::LeaseRenew { .. }
                | Message::LeaseRelease { .. }
        ) {
            let result = self
                .controller(
                    &host,
                    client,
                    expires,
                    access.clone().ok_or(Error::Unavailable)?,
                    envelope,
                )
                .await;
            let frame = match result {
                Ok(message) => {
                    let mut reply = Envelope::new(message);
                    reply.re = Some(id);
                    reply.machine = Some(machine);
                    reply.session = envelope.session;
                    Frame::Text(Box::new(reply))
                }
                Err(error) => {
                    let uncertain = matches!(error, Error::Unavailable | Error::Database);
                    let mut frame = error_frame(Some(id), error, uncertain);
                    if let Frame::Text(reply) = &mut frame {
                        reply.machine = Some(machine);
                        reply.session = envelope.session;
                    }
                    frame
                }
            };
            return self
                .routed_response(
                    source,
                    owner,
                    client,
                    machine,
                    generation,
                    Data::from_frame(frame)?,
                )
                .await;
        }
        if matches!(
            envelope.message,
            Message::Input { .. }
                | Message::Resize { .. }
                | Message::Interrupt { .. }
                | Message::Terminate { .. }
        ) && (!host.native || !host.controllers)
        {
            return Err(Error::Unsupported);
        }
        if let Message::Terminate {
            host_generation,
            confirmed,
            ..
        } = &envelope.message
        {
            if !confirmed || *host_generation != generation {
                return Err(Error::Forbidden);
            }
            self.gateway
                .store
                .audit_session_action(
                    &crate::ControllerScope {
                        owner,
                        machine,
                        session: envelope.session.ok_or(Error::Invalid)?,
                        host_generation: generation,
                        host_connection: host.id,
                    },
                    client,
                    id,
                    crate::AuditAction::SessionTerminationRequested,
                )
                .await?;
            host.grants
                .lock()
                .unwrap()
                .accepts(access.as_ref().ok_or(Error::Unavailable)?)?;
        }
        if host.native
            && matches!(
                envelope.message,
                Message::Attach { .. } | Message::Capture | Message::ReplayRequest { .. }
            )
        {
            if !host.replay {
                return Err(Error::Unsupported);
            }
            if matches!(envelope.message, Message::Attach { .. }) {
                self.gateway
                    .store
                    .audit_session_action(
                        &crate::ControllerScope {
                            owner,
                            machine,
                            session: envelope.session.ok_or(Error::Invalid)?,
                            host_generation: generation,
                            host_connection: host.id,
                        },
                        client,
                        id,
                        crate::AuditAction::SessionAttached,
                    )
                    .await?;
            }
            host.grants
                .lock()
                .unwrap()
                .accepts(access.as_ref().ok_or(Error::Unavailable)?)?;
        }
        if matches!(
            envelope.message,
            Message::Input {
                input: qgenie_remote_proto::wire::SessionInput::PromptReceipt(_),
                ..
            }
        ) && !host.prompt_receipts
        {
            return Err(Error::Unsupported);
        }
        if let Message::Start(launch) = &envelope.message {
            if !host.native || envelope.session.is_some() {
                return Err(Error::Forbidden);
            }
            if let qgenie_remote_proto::wire::LaunchKind::Agent { agent, .. } = &launch.kind
                && (!host.agents
                    || !self
                        .gateway
                        .store
                        .supports_agent(machine, generation, agent)
                        .await?)
            {
                return Err(Error::Unsupported);
            }
            self.gateway
                .recordings
                .as_ref()
                .ok_or(Error::Unavailable)?
                .ingest
                .preflight_start(
                    self.gateway
                        .identity
                        .as_deref()
                        .ok_or(Error::Configuration)?,
                    owner,
                    machine,
                    expires,
                )
                .await?;
            // Preflight may wait for SQL or replication. Preserve the original nonce;
            // accepting a newer grant here would renew a delayed mutation's authority.
            host.grants
                .lock()
                .unwrap()
                .accepts(access.as_ref().ok_or(Error::Unavailable)?)?;
            host.gate.budget()?;
        }
        let pending = Pending {
            owner,
            client,
            machine,
            session: envelope.session,
            source: source.into(),
            destination: Some((self.node.clone(), generation)),
            deadline: Instant::now() + Duration::from_secs(10),
            replies: super::pending::Replies::new(&envelope.message),
        };
        {
            let mut peers = self.peers.lock().unwrap();
            if peers.hosts_pending.len() >= 4096 {
                return Err(Error::Overloaded);
            }
            if peers.hosts_pending.contains_key(&id) {
                return Err(Error::Conflict);
            }
            peers.hosts_pending.insert(id, pending);
        }
        let Frame::Text(message) = frame else {
            return Err(Error::Invalid);
        };
        let mut envelope = Envelope::new(Message::HostDispatch {
            context: qgenie_remote_proto::wire::HostContext {
                actor: owner,
                client,
                host_generation: generation,
                expires_at: qgenie_remote_proto::wire::Timestamp::from_unix_millis(
                    expires.saturating_mul(1000),
                )
                .map_err(|_| Error::Invalid)?,
                access,
            },
            message,
        });
        envelope.machine = Some(machine);
        if let Err(error) = Self::deliver(
            &host.sender,
            &host.stop,
            Frame::Text(Box::new(envelope)),
            Priority::Control,
        ) {
            self.peers.lock().unwrap().hosts_pending.remove(&id);
            return Err(error);
        }
        Ok(())
    }
    pub async fn host_frame(
        &self,
        actor: &MachinePrincipal,
        generation: Counter,
        frame: Frame,
    ) -> Result<(), Error> {
        let continuation = match &frame {
            Frame::Text(envelope) => envelope
                .re
                .and_then(|id| {
                    self.peers
                        .lock()
                        .unwrap()
                        .hosts_pending
                        .get(&id)
                        .map(|pending| {
                            pending.replies.is_continuation()
                                && pending.machine == actor.machine
                                && pending.owner == actor.owner
                                && pending.destination.as_ref()
                                    == Some(&(self.node.clone(), generation))
                        })
                })
                .unwrap_or(false),
            _ => false,
        };
        if !continuation && !self.gateway.store.host_current(actor, generation).await? {
            return Err(Error::Forbidden);
        }
        if let Frame::Text(envelope) = &frame
            && matches!(
                envelope.message,
                Message::HostAccessChallenge { .. } | Message::HostActivated { .. }
            )
        {
            return self.native_activation(actor, generation, envelope).await;
        }
        if !continuation {
            self.gateway
                .identity
                .as_ref()
                .ok_or(Error::Configuration)?
                .assert_directory(actor.owner)
                .await?;
        }
        let host = self
            .peers
            .lock()
            .unwrap()
            .hosts
            .get(&actor.machine)
            .cloned()
            .ok_or(Error::NotFound)?;
        if host.generation != generation || host.actor.owner != actor.owner || *host.stop.borrow() {
            return Err(Error::Forbidden);
        }
        host.gate.budget()?;
        if host.native
            && !matches!(&frame, Frame::Text(envelope) if matches!(envelope.message, Message::Inventory {..}))
        {
            host.grants.lock().unwrap().current()?;
        }
        if let Frame::Text(reply) = &frame
            && self.control_response(actor, generation, &host, reply)?
        {
            return Ok(());
        }
        let (session, reply) = match &frame {
            Frame::Binary(pty) => {
                if !matches!(pty.payload, PtyPayload::Output(_)) {
                    return Err(Error::Forbidden);
                }
                (Some(pty.session), None)
            }
            Frame::Text(envelope) => {
                if envelope.machine != Some(actor.machine) {
                    return Err(Error::Forbidden);
                }
                match &envelope.message {
                    Message::Inventory { agents } => {
                        if envelope.re.is_some() {
                            return Err(Error::Invalid);
                        }
                        self.gateway
                            .store
                            .inventory(actor, generation, agents)
                            .await?;
                        self.changed(actor.owner, actor.machine).await;
                        return Ok(());
                    }
                    Message::Approvals { approvals } if envelope.re.is_none() => {
                        return self
                            .approval_snapshot(
                                actor,
                                generation,
                                &host,
                                envelope.session.ok_or(Error::Invalid)?,
                                approvals,
                            )
                            .await;
                    }
                    Message::Sessions { sessions } | Message::SessionUpdates { sessions } => {
                        if sessions.len() > 1024 {
                            return Err(Error::Overloaded);
                        }
                        for session in sessions {
                            self.gateway
                                .store
                                .project(actor, generation, session)
                                .await?;
                            if host.approvals
                                && matches!(
                                    session.kind,
                                    qgenie_remote_proto::wire::LaunchKind::Agent { .. }
                                )
                                && matches!(
                                    session.state,
                                    qgenie_remote_proto::SessionState::Exited
                                        | qgenie_remote_proto::SessionState::Failed
                                        | qgenie_remote_proto::SessionState::Lost
                                )
                            {
                                self.approval_snapshot(actor, generation, &host, session.id, &[])
                                    .await?;
                            }
                        }
                    }
                    Message::Started(session) => {
                        self.gateway
                            .store
                            .project(actor, generation, session)
                            .await?
                    }
                    Message::Exited { code: _ } => {
                        self.gateway
                            .store
                            .project_exit(
                                actor,
                                generation,
                                envelope.session.ok_or(Error::Invalid)?,
                            )
                            .await?
                    }
                    Message::StartStatus { .. }
                    | Message::InputAck { .. }
                    | Message::ActionResult { .. }
                    | Message::FsListed { .. }
                    | Message::FsRootsListed { .. }
                    | Message::SessionPage { .. }
                    | Message::SessionFound(_)
                    | Message::Replay { .. }
                    | Message::Head { .. }
                    | Message::Snapshot { .. }
                    | Message::Approvals { .. }
                    | Message::Event(_)
                    | Message::Error(_) => {}
                    _ => return Err(Error::Forbidden),
                }
                (envelope.session, envelope.re)
            }
        };
        if let Some(session) = session
            && !continuation
        {
            self.gateway
                .store
                .route_access(actor.owner, actor.machine, Some(session))
                .await?;
        }
        let data = Data::from_frame(frame)?;
        if let Some(reply) = reply {
            let Data::Json(envelope) = &data else {
                return Err(Error::Invalid);
            };
            let Some(pending) = super::pending::response(
                &mut self.peers.lock().unwrap().hosts_pending,
                reply,
                &envelope.message,
            )?
            else {
                return Ok(());
            };
            if pending.machine != actor.machine
                || pending.owner != actor.owner
                || pending.session != session
                || pending.deadline <= Instant::now()
            {
                return Err(Error::Forbidden);
            }
            if pending.source == self.node {
                return self
                    .response(pending.owner, pending.client, pending.machine, data)
                    .await;
            }
            let packet = Packet {
                id: MessageId::new(),
                source: self.node.clone(),
                destination: node_channel(&pending.source),
                at_ms: Counter(now_ms()?),
                body: Body::Response {
                    owner: pending.owner,
                    client: pending.client,
                    machine: pending.machine,
                    generation,
                    frame: data,
                },
            };
            self.broker.publish(&packet, Priority::Event).await?;
            return Ok(());
        }
        // A host may project its own inventory and session metadata; unsolicited file/list
        // replies cannot become an arbitrary user broadcast.
        if let Data::Json(envelope) = &data
            && !matches!(
                envelope.message,
                Message::Event(_)
                    | Message::Head { .. }
                    | Message::Exited { .. }
                    | Message::Started(_)
                    | Message::Sessions { .. }
                    | Message::SessionUpdates { .. }
            )
        {
            return Err(Error::Forbidden);
        }
        self.fanout(actor.owner, actor.machine, session, data.clone())
            .await?;
        let packet = Packet {
            id: MessageId::new(),
            source: self.node.clone(),
            destination: user_channel(actor.owner),
            at_ms: Counter(now_ms()?),
            body: Body::Event {
                owner: actor.owner,
                machine: actor.machine,
                generation,
                session,
                frame: data,
            },
        };
        if self.broker.publish(&packet, Priority::Bulk).await.is_err() {
            self.mark_gap(actor.owner, actor.machine, generation)?;
        }
        self.changed(actor.owner, actor.machine).await;
        Ok(())
    }
    pub(super) async fn response(
        &self,
        owner: qgenie_remote_proto::UserId,
        client: ClientId,
        machine: MachineId,
        data: Data,
    ) -> Result<(), Error> {
        let frame = data.frame()?;
        let Frame::Text(envelope) = &frame else {
            return Err(Error::Invalid);
        };
        let re = envelope.re.ok_or(Error::Invalid)?;
        let Some(pending) = super::pending::response(
            &mut self.peers.lock().unwrap().clients_pending,
            re,
            &envelope.message,
        )?
        else {
            return Ok(());
        };
        if pending.owner != owner
            || pending.client != client
            || pending.machine != machine
            || pending.session != envelope.session
            || envelope.machine != Some(machine)
            || pending.deadline <= Instant::now()
        {
            return Err(Error::Forbidden);
        }
        if !pending.replies.is_continuation() {
            self.gateway
                .store
                .route_access(owner, machine, pending.session)
                .await?;
        }
        let priority = match envelope.message {
            Message::Snapshot { .. } | Message::Replay { .. } | Message::Head { .. } => {
                Priority::Bulk
            }
            Message::InputAck { .. }
            | Message::ActionResult { .. }
            | Message::LeaseGranted(_)
            | Message::LeaseRevoked { .. }
            | Message::Error(_) => Priority::Control,
            _ => Priority::Event,
        };
        let mut frame = frame;
        if let Frame::Text(envelope) = &mut frame {
            self.gateway
                .store
                .decorate_session_metadata(owner, machine, &mut envelope.message)
                .await?;
        }
        self.client_send(client, frame, priority)
    }
    pub(super) async fn fanout(
        &self,
        owner: qgenie_remote_proto::UserId,
        machine: MachineId,
        session: Option<qgenie_remote_proto::SessionId>,
        data: Data,
    ) -> Result<(), Error> {
        self.gateway
            .store
            .route_access(owner, machine, session)
            .await?;
        let mut frame = data.frame()?;
        if let Frame::Text(envelope) = &mut frame {
            self.gateway
                .store
                .decorate_session_metadata(owner, machine, &mut envelope.message)
                .await?;
        }
        let owner_wide = matches!(&frame,Frame::Text(envelope) if matches!(envelope.message,Message::Sessions{..}|Message::SessionUpdates{..}|Message::ApprovalRequested(_)|Message::ApprovalCommitted(_)|Message::ApprovalApplication{..}|Message::ApprovalCancelled{..}));
        let partial = matches!(&frame, Frame::Text(envelope) if matches!(envelope.message, Message::SessionUpdates {..}));
        let priority = if matches!(frame, Frame::Binary(_)) {
            Priority::Bulk
        } else {
            Priority::Event
        };
        let peers = self.peers.lock().unwrap();
        for peer in peers.clients.values().filter(|peer| {
            peer.owner == owner
                && (!partial || peer.session_pages)
                && peer.expires.load(Ordering::Acquire).saturating_mul(1000)
                    > now_ms().unwrap_or(u64::MAX)
                && (owner_wide
                    || session
                        .is_some_and(|session| peer.subscriptions.get(&session) == Some(&machine)))
        }) {
            let _ = Self::deliver(&peer.sender, &peer.stop, frame.clone(), priority);
        }
        Ok(())
    }
    pub async fn packet(&self, packet: Packet) -> Result<(), Error> {
        let now = now_ms()?;
        if packet.at_ms.0 > now.saturating_add(5000) || now.saturating_sub(packet.at_ms.0) > 10000 {
            return Err(Error::Unavailable);
        }
        let expected = match &packet.body {
            Body::Request { .. } | Body::Response { .. } => node_channel(&self.node),
            Body::Event { owner, .. }
            | Body::ApprovalChanged { owner, .. }
            | Body::SessionMetadataChanged { owner, .. }
            | Body::Changed { owner, .. }
            | Body::Resync { owner, .. } => user_channel(*owner),
            Body::Hint { .. } => return Err(Error::Invalid),
        };
        if packet.destination != expected {
            return Err(Error::Forbidden);
        }
        match packet.body {
            Body::SessionMetadataChanged {
                owner,
                machine,
                session,
            } => {
                if packet.source == self.node {
                    return Ok(());
                }
                self.metadata_emit(owner, machine, session).await
            }
            Body::ApprovalChanged {
                owner,
                machine,
                session,
                approval,
            } => {
                if packet.source == self.node {
                    return Ok(());
                }
                self.approval_emit(owner, machine, session, approval).await
            }
            Body::Request {
                owner,
                client,
                expires,
                machine,
                generation,
                access,
                frame,
            } => {
                self.peers.lock().unwrap().replay.accept(packet.id)?;
                let re = match &frame {
                    Data::Json(envelope) => Some(envelope.id),
                    _ => None,
                };
                let session = match &frame {
                    Data::Json(envelope) => envelope.session,
                    _ => None,
                };
                if let Err(error) = self
                    .dispatch(
                        &packet.source,
                        owner,
                        client,
                        expires.0,
                        machine,
                        generation,
                        access,
                        frame,
                    )
                    .await
                {
                    let mut envelope = match error_frame(re, error, false) {
                        Frame::Text(envelope) => envelope,
                        _ => unreachable!(),
                    };
                    envelope.machine = Some(machine);
                    envelope.session = session;
                    // Preserve the requested target on a routed failure.
                    let packet = Packet {
                        id: MessageId::new(),
                        source: self.node.clone(),
                        destination: node_channel(&packet.source),
                        at_ms: Counter(now_ms()?),
                        body: Body::Response {
                            owner,
                            client,
                            machine,
                            generation,
                            frame: Data::Json(envelope),
                        },
                    };
                    self.broker.publish(&packet, Priority::Control).await?;
                }
                Ok(())
            }
            Body::Response {
                owner,
                client,
                machine,
                generation,
                frame,
            } => {
                let continuation = match &frame {
                    Data::Json(envelope) => envelope
                        .re
                        .and_then(|id| {
                            self.peers
                                .lock()
                                .unwrap()
                                .clients_pending
                                .get(&id)
                                .map(|pending| {
                                    pending.replies.is_continuation()
                                        && pending.owner == owner
                                        && pending.client == client
                                        && pending.machine == machine
                                        && pending.destination.as_ref()
                                            == Some(&(packet.source.clone(), generation))
                                })
                        })
                        .unwrap_or(false),
                    _ => false,
                };
                if !continuation
                    && !self
                        .gateway
                        .store
                        .route_matches(machine, &packet.source, generation)
                        .await?
                {
                    return Err(Error::Forbidden);
                }
                self.response(owner, client, machine, frame).await
            }
            Body::Event {
                owner,
                machine,
                session,
                generation,
                frame,
            } => {
                if !self
                    .gateway
                    .store
                    .route_matches(machine, &packet.source, generation)
                    .await?
                {
                    return Err(Error::Forbidden);
                }
                if packet.source == self.node {
                    return Ok(());
                }
                self.fanout(owner, machine, session, frame).await
            }
            Body::Resync {
                owner,
                machine,
                generation,
            } => {
                if packet.source == self.node {
                    return Ok(());
                }
                self.gateway
                    .store
                    .route_access(owner, machine, None)
                    .await?;
                if self
                    .gateway
                    .store
                    .route_matches(machine, &packet.source, generation)
                    .await?
                {
                    self.resync_machine(owner, machine);
                }
                Ok(())
            }
            Body::Changed { .. } => Ok(()), // Periodic authoritative reconciliation also closes snapshot/subscription races.
            Body::Hint { .. } => Err(Error::Invalid),
        }
    }
    pub fn expire_pending(&self) {
        let expired = {
            let mut peers = self.peers.lock().unwrap();
            let now = Instant::now();
            peers
                .hosts_pending
                .retain(|_, pending| pending.deadline > now);
            let ids = peers
                .clients_pending
                .iter()
                .filter(|(_, pending)| pending.deadline <= now)
                .map(|(id, _)| *id)
                .collect::<Vec<_>>();
            ids.into_iter()
                .filter_map(|id| {
                    peers
                        .clients_pending
                        .remove(&id)
                        .map(|pending| (id, pending))
                })
                .collect::<Vec<_>>()
        };
        for (id, pending) in expired {
            let _ = self.client_send(
                pending.client,
                error_frame(Some(id), Error::Unavailable, true),
                Priority::Control,
            );
        }
    }
}
