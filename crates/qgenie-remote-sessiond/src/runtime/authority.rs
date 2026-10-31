use super::*;
use qgenie_remote_proto::ipc::{DispatchFrame, HostActivation, RemoteContext};
const MAX_LIFETIME: Duration =
    Duration::from_millis(qgenie_remote_proto::MAX_ACCESS_LIFETIME_MS as u64);
mod effects;
mod recording;
pub(super) use effects::Fence;
#[derive(Clone)]
struct Challenge {
    scope: RemoteScope,
    connection: ClientId,
    generation: Counter,
    issued: Instant,
    issued_elapsed: Duration,
}
struct Grant {
    issued: Instant,
    expires: Instant,
    remaining_ms: u32,
    issued_elapsed: Duration,
    expires_elapsed: Duration,
    recording_challenge: Option<ChallengeId>,
    recording_epoch: Option<String>,
}
struct Active {
    effects: Arc<effects::Effects>,
    connection: ClientId,
    generation: Counter,
    grants: BTreeMap<ChallengeId, Grant>,
    recording_bound: Option<(Instant, ChallengeId)>,
}
#[derive(Default)]
pub(super) struct Authority {
    pending: BTreeMap<ChallengeId, Challenge>,
    active: BTreeMap<RemoteScope, Active>,
}
impl Authority {
    fn prune(&mut self, now: Instant, elapsed: Duration) {
        self.pending.retain(|_, challenge| {
            now.saturating_duration_since(challenge.issued) < MAX_LIFETIME
                && elapsed >= challenge.issued_elapsed
                && elapsed - challenge.issued_elapsed < MAX_LIFETIME
        });
        for active in self.active.values_mut() {
            active.grants.retain(|_, grant| {
                grant.expires > now
                    && elapsed >= grant.issued_elapsed
                    && elapsed < grant.expires_elapsed
            });
        }
    }
    fn challenge(
        &mut self,
        store: &Store,
        scope: RemoteScope,
        connection: ClientId,
        generation: Counter,
        now: Instant,
    ) -> Result<DispatchFrame, RemoteError> {
        let elapsed = clock::elapsed()?;
        self.prune(now, elapsed);
        scope.validate()?;
        if !store.bound(&scope).map_err(storage_error)? {
            return Err(forbidden());
        }
        let (floor, holder) = store.host_floor(&scope).map_err(storage_error)?;
        if generation.0 == 0
            || generation < floor
            || (generation == floor && holder != Some(connection))
        {
            return Err(stale());
        }
        if self.pending.len() >= 256
            || self
                .pending
                .values()
                .filter(|challenge| challenge.scope == scope)
                .count()
                >= 4
        {
            return Err(RemoteError::rejected(
                ErrorCode::Overloaded,
                "native access challenge capacity reached",
            ));
        }
        let nonce = ChallengeId::new();
        self.pending.insert(
            nonce,
            Challenge {
                scope,
                connection,
                generation,
                issued: now,
                issued_elapsed: elapsed,
            },
        );
        Ok(DispatchFrame::AccessChallengeIssued {
            nonce,
            policy_revision: Counter(1),
            max_remaining_ms: qgenie_remote_proto::MAX_ACCESS_LIFETIME_MS,
        })
    }
    #[cfg(test)]
    fn activate(
        &mut self,
        store: &mut Store,
        request: &HostActivation,
        renew_only: bool,
        clock: impl Fn() -> Instant,
    ) -> Result<DispatchFrame, RemoteError> {
        self.activate_with(|| Ok(store), request, renew_only, clock)
    }
    fn activate_with<S: std::ops::DerefMut<Target = Store>>(
        &mut self,
        open_store: impl FnOnce() -> Result<S, RemoteError>,
        request: &HostActivation,
        renew_only: bool,
        clock: impl Fn() -> Instant,
    ) -> Result<DispatchFrame, RemoteError> {
        let now = clock();
        let elapsed = clock::elapsed()?;
        self.prune(now, elapsed);
        if request.remaining_ms == 0
            || request.remaining_ms > qgenie_remote_proto::MAX_ACCESS_LIFETIME_MS
            || request.policy_revision != Counter(1)
        {
            return Err(RemoteError::rejected(
                ErrorCode::AccessStale,
                "invalid native access lifetime or policy revision",
            ));
        }
        if let Some(active) = self.active.get(&request.scope)
            && active.effects.alive()
            && active.generation == request.generation
            && active.connection == request.connection
            && let Some(grant) = active.grants.get(&request.nonce)
        {
            if grant.remaining_ms != request.remaining_ms {
                return Err(forbidden());
            }
            return Ok(DispatchFrame::HostActivated {
                nonce: request.nonce,
                generation: request.generation,
            });
        }
        if renew_only
            && !self.active.get(&request.scope).is_some_and(|active| {
                active.generation == request.generation && active.connection == request.connection
            })
        {
            return Err(stale());
        }
        let challenge = self
            .pending
            .get(&request.nonce)
            .cloned()
            .ok_or_else(expired)?;
        if challenge.scope != request.scope
            || challenge.connection != request.connection
            || challenge.generation != request.generation
        {
            return Err(forbidden());
        }
        let expires = challenge.issued + Duration::from_millis(u64::from(request.remaining_ms));
        let expires_elapsed = challenge
            .issued_elapsed
            .checked_add(Duration::from_millis(u64::from(request.remaining_ms)))
            .ok_or_else(expired)?;
        if expires <= now || expires_elapsed <= elapsed {
            return Err(expired());
        }
        let previous = self
            .active
            .get(&request.scope)
            .map(|active| {
                if request.generation < active.generation
                    || (request.generation == active.generation
                        && request.connection != active.connection)
                {
                    return Err(stale());
                }
                Ok((
                    active.effects.clone(),
                    request.generation > active.generation || !active.effects.alive(),
                ))
            })
            .transpose()?;
        let _replaced = previous
            .as_ref()
            .filter(|(_, replace)| *replace)
            .map(|(effects, _)| effects.invalidate())
            .transpose()?;
        let mut store = open_store()?;
        match store.activate_host_floor(&request.scope, request.generation, request.connection) {
            Ok(true) => {}
            Ok(false) => return Err(stale()),
            Err(error) => {
                if let Some(active) = self.active.get_mut(&request.scope) {
                    active.effects.stop();
                    active.grants.clear();
                    active.recording_bound = None;
                }
                return Err(storage_error(error));
            }
        }
        // The floor fences the old host even if persistence consumed the new
        // grant's entire lifetime. Do not acknowledge an expired delegation.
        let active = self
            .active
            .entry(request.scope.clone())
            .or_insert_with(|| Active {
                effects: Arc::new(effects::Effects::new()),
                connection: request.connection,
                generation: request.generation,
                grants: BTreeMap::new(),
                recording_bound: None,
            });
        if active.generation != request.generation
            || active.connection != request.connection
            || !active.effects.alive()
        {
            active.grants.clear();
            active.effects = Arc::new(effects::Effects::new());
            active.recording_bound = None;
        }
        active.generation = request.generation;
        active.connection = request.connection;
        self.pending.remove(&request.nonce);
        if expires <= clock() || expires_elapsed <= clock::elapsed()? {
            return Err(expired());
        }
        active.grants.retain(|_, grant| grant.expires > now);
        if active.grants.len() >= 4 {
            return Err(RemoteError::rejected(
                ErrorCode::Overloaded,
                "native access grant capacity reached",
            ));
        }
        active.grants.insert(
            request.nonce,
            Grant {
                issued: challenge.issued,
                expires,
                remaining_ms: request.remaining_ms,
                issued_elapsed: challenge.issued_elapsed,
                expires_elapsed,
                recording_challenge: None,
                recording_epoch: None,
            },
        );
        Ok(DispatchFrame::HostActivated {
            nonce: request.nonce,
            generation: request.generation,
        })
    }
    pub(super) fn control_deadlines(
        &self,
        context: &RemoteContext,
        remaining_ms: u32,
    ) -> Result<(Instant, Duration), RemoteError> {
        self.validate(context, Instant::now())?;
        if remaining_ms == 0 || remaining_ms > 15_000 {
            return Err(expired());
        }
        let grant = self
            .active
            .get(&context.scope)
            .and_then(|active| active.grants.get(&context.access_nonce))
            .ok_or_else(expired)?;
        let budget = Duration::from_millis(u64::from(remaining_ms));
        let instant = grant.issued + budget;
        let elapsed = grant
            .issued_elapsed
            .checked_add(budget)
            .ok_or_else(expired)?;
        if instant <= Instant::now() || elapsed <= clock::elapsed()? {
            return Err(expired());
        }
        Ok((instant, elapsed))
    }
    pub(super) fn validate(
        &self,
        context: &RemoteContext,
        now: Instant,
    ) -> Result<Instant, RemoteError> {
        self.validate_at(context, now, clock::elapsed()?)
    }
    fn validate_at(
        &self,
        context: &RemoteContext,
        now: Instant,
        elapsed: Duration,
    ) -> Result<Instant, RemoteError> {
        if context.actor != context.scope.owner || context.policy_revision != Counter(1) {
            return Err(forbidden());
        }
        let active = self.active.get(&context.scope).ok_or_else(expired)?;
        if !active.effects.alive() {
            return Err(stale());
        }
        if active.generation != context.host_generation || active.connection != context.connection {
            return Err(stale());
        }
        let grant = active
            .grants
            .get(&context.access_nonce)
            .ok_or_else(expired)?;
        if context.remaining_access_ms == 0 || context.remaining_access_ms > grant.remaining_ms {
            return Err(expired());
        }
        let deadline = grant
            .expires
            .min(grant.issued + Duration::from_millis(u64::from(context.remaining_access_ms)));
        if deadline <= now {
            return Err(expired());
        }
        let elapsed_deadline = grant.expires_elapsed.min(
            grant.issued_elapsed + Duration::from_millis(u64::from(context.remaining_access_ms)),
        );
        if elapsed < grant.issued_elapsed || elapsed_deadline <= elapsed {
            return Err(expired());
        }
        Ok(deadline.min(now + (elapsed_deadline - elapsed)))
    }
}
impl Runtime {
    /// Enrollment is local administration. Registering a second identity never
    /// adopts legacy/local sessions or rewrites an existing machine's owner.
    pub(crate) fn register_remote_binding(&self, scope: RemoteScope) -> Result<(), RemoteError> {
        scope.validate()?;
        let _authority = lock(&self.authority)?;
        let store = lock(&self.store)?;
        if store
            .binding_owner(&scope)
            .map_err(storage_error)?
            .is_some_and(|owner| owner != scope.owner)
        {
            return Err(forbidden());
        }
        store.bind(&scope).map_err(storage_error)
    }
    /// Single independently negotiated IPC operation. Native effects remain
    /// unsupported until controller and gateway dispatch activation are wired.
    pub(crate) fn dispatch_frame(self: &Arc<Self>, frame: DispatchFrame) -> DispatchFrame {
        let result = (|| match frame {
            DispatchFrame::Local(request) => Ok(DispatchFrame::LocalReply(self.dispatch(request))),
            DispatchFrame::AgentInventory { context } => {
                let fence = lock(&self.authority)?.fence(&context)?;
                #[allow(unused_mut)]
                let mut agents = BTreeMap::new();
                #[cfg(all(feature = "managed-codex", target_os = "linux"))]
                if self.agent_verified.load(Ordering::Acquire) {
                    agents.insert(
                        "codex".into(),
                        wire::AgentInventory {
                            version: qgenie_remote_adapters::PINNED_CODEX_VERSION.into(),
                            capabilities: wire::Capabilities {
                                resume: true,
                                input: true,
                                steer: true,
                                interrupt: true,
                                terminate: true,
                                approvals: ["command".into()].into(),
                                ..Default::default()
                            },
                        },
                    );
                }
                fence.check()?;
                Ok(DispatchFrame::AgentInventoryReply { agents })
            }
            DispatchFrame::QueryHostFloor { scope } => {
                scope.validate()?;
                let store = lock(&self.store)?;
                if !store.bound(&scope).map_err(storage_error)? {
                    return Err(forbidden());
                }
                Ok(DispatchFrame::HostFloor {
                    generation: store.host_floor(&scope).map_err(storage_error)?.0,
                })
            }
            DispatchFrame::AccessChallenge {
                scope,
                connection,
                host_generation,
            } => {
                let mut authority = lock(&self.authority)?;
                let store = lock(&self.store)?;
                authority.challenge(&store, scope, connection, host_generation, Instant::now())
            }
            DispatchFrame::ActivateHost(request) => {
                let mut authority = lock(&self.authority)?;
                authority.activate_with(|| lock(&self.store), &request, false, Instant::now)
            }
            DispatchFrame::AccessRenewal(request) => {
                let mut authority = lock(&self.authority)?;
                authority.activate_with(|| lock(&self.store), &request, true, Instant::now)
            }
            DispatchFrame::Remote { context, message }
                if matches!(message.message, wire::Message::Start(_)) =>
            {
                self.remote_start(&context, *message)
            }
            #[cfg(all(feature = "managed-codex", target_os = "linux"))]
            DispatchFrame::Remote { context, message }
                if matches!(
                    message.message,
                    wire::Message::ApprovalList | wire::Message::ApprovalCommitted(_)
                ) =>
            {
                self.native_approval_rpc(&context, *message)
            }
            DispatchFrame::Remote { context, message }
                if matches!(
                    message.message,
                    wire::Message::Input { .. }
                        | wire::Message::Resize { .. }
                        | wire::Message::Interrupt { .. }
                        | wire::Message::Terminate { .. }
                ) =>
            {
                self.remote_effect(&context, *message)
            }
            DispatchFrame::Remote { context, message }
                if matches!(message.message, wire::Message::Attach { .. }) =>
            {
                self.remote_replay(&context, *message)
            }
            DispatchFrame::ReadHead { context, session } => self.remote_head(&context, session),
            DispatchFrame::ReadSnapshot {
                context,
                session,
                epoch,
                page,
            } => self.remote_snapshot(&context, session, epoch, page),
            DispatchFrame::Remote { context, message } => self.remote_read(&context, *message),
            DispatchFrame::RecordingFetch { context, max_bytes } => {
                self.recording_read(&context, Some(max_bytes))
            }
            DispatchFrame::RecordingStatus { context } => self.recording_read(&context, None),
            DispatchFrame::RecordingEpochChallenge { context } => {
                lock(&self.authority)?.recording_challenge(&context)
            }
            DispatchFrame::BindRecordingEpoch { context, authority } => {
                lock(&self.authority)?.bind_recording(&context, &authority)
            }
            DispatchFrame::RecordingAcknowledge {
                context,
                challenge,
                receipt,
            } => self.recording_acknowledge(context, challenge, receipt),
            DispatchFrame::QueryControl { context, session } => {
                self.remote_control_state(&context, session)
            }
            DispatchFrame::ActivateControl(request) => self.activate_remote_control(request),
            DispatchFrame::DeactivateControl {
                context,
                session,
                generation,
            } => self.release_remote_control(&context, session, generation),
            _ => Err(RemoteError::rejected(
                ErrorCode::UnsupportedCapability,
                "native IPC operation is not implemented",
            )),
        })();
        result.unwrap_or_else(DispatchFrame::Error)
    }
    fn remote_read(
        &self,
        context: &RemoteContext,
        message: wire::Envelope,
    ) -> Result<DispatchFrame, RemoteError> {
        message.validate()?;
        let authority = lock(&self.authority)?;
        authority.validate(context, Instant::now())?;
        if message.machine != Some(context.scope.machine)
            || message.re.is_some()
            || message.session.is_some()
        {
            return Err(forbidden());
        }
        authorize(
            Action::Observe,
            Facts {
                owner: context.actor == context.scope.owner,
                access_fresh: true,
                within_roots: false,
                ..local_facts()
            },
        )?;
        let response = match message.message {
            wire::Message::SessionList => wire::Message::Sessions {
                sessions: self.remote_list(&context.scope)?,
            },
            wire::Message::SessionListPage {
                after,
                limit,
                active_only,
            } => {
                let (sessions, next) =
                    self.remote_page(&context.scope, after, limit, active_only)?;
                wire::Message::SessionPage { sessions, next }
            }
            wire::Message::SessionGet { session } => {
                wire::Message::SessionFound(self.remote_session(&context.scope, session)?)
            }
            wire::Message::StartLookup { key } => {
                let session = self
                    .lookup_start(Some(&context.scope), key)?
                    .ok_or_else(|| {
                        RemoteError::unknown(
                            "no native start is known in this immutable scope; no launch was sent",
                        )
                    })?;
                let state = lock(&session)?;
                let stage = match state.info.state {
                    SessionState::Accepted => wire::StartStage::Accepted,
                    SessionState::Launching => wire::StartStage::Launching,
                    SessionState::Running | SessionState::Idle | SessionState::Exited => {
                        wire::StartStage::Running
                    }
                    SessionState::Failed => wire::StartStage::Failed,
                    SessionState::Lost => wire::StartStage::Lost,
                    SessionState::OutcomeUnknown => wire::StartStage::OutcomeUnknown,
                };
                let session = state.info.id;
                wire::Message::StartStatus {
                    key,
                    stage,
                    session: Some(session),
                }
            }
            _ => {
                return Err(RemoteError::rejected(
                    ErrorCode::UnsupportedCapability,
                    "native remote effects/attachment require controller and gateway integration",
                ));
            }
        };
        authority.validate(context, Instant::now())?;
        let mut envelope = wire::Envelope::new(response);
        envelope.re = Some(message.id);
        envelope.machine = Some(context.scope.machine);
        envelope.session = message.session;
        Ok(DispatchFrame::RemoteReply(Box::new(envelope)))
    }
    fn remote_start(
        self: &Arc<Self>,
        context: &RemoteContext,
        message: wire::Envelope,
    ) -> Result<DispatchFrame, RemoteError> {
        message.validate()?;
        if message.machine != Some(context.scope.machine)
            || message.session.is_some()
            || message.re.is_some()
        {
            return Err(forbidden());
        }
        let fence = lock(&self.authority)?.fence(context)?;
        let wire::Message::Start(launch) = message.message else {
            return Err(forbidden());
        };
        let info = match &launch.kind {
            wire::LaunchKind::Pty { command } => self.start_guarded(
                StartSpec {
                    key: launch.key,
                    command: command.clone(),
                    cwd: launch.cwd,
                    rows: launch.rows,
                    cols: launch.cols,
                },
                Some(context.scope.clone()),
                Some(fence.clone()),
            )?,
            wire::LaunchKind::Agent { .. } => {
                #[cfg(all(feature = "managed-codex", target_os = "linux"))]
                {
                    self.start_agent(launch, Some(context.scope.clone()), Some(fence.clone()))?
                }
                #[cfg(not(all(feature = "managed-codex", target_os = "linux")))]
                {
                    return Err(RemoteError::rejected(
                        ErrorCode::UnsupportedCapability,
                        "managed agents are not enabled on this host",
                    ));
                }
            }
        };
        fence.check().map_err(|mut error| {
            error.outcome = OperationOutcome::Unknown;
            error.message =
                "accepted start lost its original authority; reconcile the original key".into();
            error
        })?;
        let owned = self.session(info.id)?;
        let capabilities = {
            let state = lock(&owned)?;
            session_capabilities(&state, self.can_resume_history())
        };
        let session = wire::Session {
            metadata: Default::default(),
            id: info.id,
            machine: info.machine,
            owner: context.scope.owner,
            kind: native_kind(&info),
            cwd: info.cwd,
            origin: wire::Origin::Managed,
            state: info.state,
            created_at: wire::Timestamp::from_unix_millis(info.created_at_ms.0)?,
            ended_at: info
                .ended_at_ms
                .map(|v| wire::Timestamp::from_unix_millis(v.0))
                .transpose()?,
            head: info.head,
            capabilities,
            recording_warning: info.recording_warning,
        };
        let mut response = wire::Envelope::new(wire::Message::Started(session));
        response.re = Some(message.id);
        response.machine = Some(context.scope.machine);
        Ok(DispatchFrame::RemoteReply(Box::new(response)))
    }
}
pub(super) fn native_kind(info: &SessionInfo) -> wire::LaunchKind {
    match &info.agent {
        Some(agent) => wire::LaunchKind::Agent {
            agent: agent.agent.clone(),
            prompt: None,
            native_resume_id: agent.native_thread.clone(),
        },
        None => wire::LaunchKind::Pty {
            command: info.command.iter().take(1).cloned().collect(),
        },
    }
}
pub(super) fn session_capabilities(state: &Session, can_resume: bool) -> wire::Capabilities {
    if state.info.agent.is_some() && !owns_process(state) {
        return wire::Capabilities {
            resume: can_resume
                && matches!(state.info.state, SessionState::Exited)
                && state.info.ended_at_ms.is_some()
                && state
                    .info
                    .agent
                    .as_ref()
                    .is_some_and(|agent| agent.native_thread.is_some()),
            ..Default::default()
        };
    }
    #[cfg(all(feature = "managed-codex", target_os = "linux"))]
    if state.agent_control.is_some() {
        let correlated = state
            .info
            .agent
            .as_ref()
            .is_some_and(|agent| agent.native_thread.is_some());
        let active =
            correlated && matches!(state.info.state, SessionState::Running | SessionState::Idle);
        return wire::Capabilities {
            input: active,
            steer: active,
            interrupt: active,
            terminate: true,
            approvals: if correlated {
                ["command".into()].into()
            } else {
                Default::default()
            },
            ..Default::default()
        };
    }
    wire::Capabilities {
        input: state.control.is_some(),
        resize: state.control.is_some(),
        interrupt: state.control.is_some(),
        terminate: state.control.is_some(),
        ..Default::default()
    }
}
fn stale() -> RemoteError {
    RemoteError::rejected(
        ErrorCode::StaleGeneration,
        "native host generation/connection is stale",
    )
}
fn expired() -> RemoteError {
    RemoteError::rejected(
        ErrorCode::AccessStale,
        "native access delegation expired or is unknown",
    )
}
fn forbidden() -> RemoteError {
    RemoteError::rejected(
        ErrorCode::Forbidden,
        "native scope or actor binding rejected",
    )
}

#[cfg(test)]
mod tests;
