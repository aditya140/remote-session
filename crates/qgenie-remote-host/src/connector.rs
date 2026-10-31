use crate::{Error, GatewayApi, MachineAuthentication, MachineCredentials, filesystem::Filesystem};
use qgenie_remote_policy::{Action, Facts, authorize};
use qgenie_remote_proto::{
    Counter, ErrorCode, Hello, RemoteError, Version,
    wire::{Envelope, HostContext, Message},
};
use qgenie_remote_transport::ws::{
    self, Backoff, ClientConfig, Connection, Frame, Limits, Priority, SecretHeader,
};
use std::{
    path::PathBuf,
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    sync::watch,
    time::{Instant, sleep, sleep_until, timeout},
};
mod approvals;
mod changes;
mod native_link;
mod replay;
use native_link::NativeLink;
struct Connected {
    connection: Connection,
    authentication: MachineAuthentication,
    generation: Counter,
    native: Option<NativeLink>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConnectionStatus {
    Disconnected,
    Authenticating,
    Connected {
        generation: Counter,
        activated: bool,
    },
    Denied,
    Stopped,
}
/// Exactly one outbound WSS connection. An optional same-user native bridge
/// activates scoped read authority; native effects and recording remain separate.
pub struct Connector {
    api: GatewayApi,
    credentials: Arc<MachineCredentials>,
    filesystem: Filesystem,
    ca: Option<PathBuf>,
    allow_browse: bool,
    native: Option<crate::NativeClient>,
}
impl Connector {
    pub fn new(
        api: GatewayApi,
        credentials: MachineCredentials,
        filesystem: Filesystem,
        ca: Option<PathBuf>,
        allow_browse: bool,
    ) -> Result<Self, Error> {
        if api.origin() != credentials.origin {
            return Err(Error::Configuration);
        }
        Ok(Self {
            api,
            credentials: Arc::new(credentials),
            filesystem,
            ca,
            allow_browse,
            native: None,
        })
    }
    pub fn with_native(mut self, native: crate::NativeClient) -> Result<Self, Error> {
        if native.scope().origin != self.credentials.origin
            || native.scope().owner != self.credentials.binding.owner
            || native.scope().machine != self.credentials.binding.machine
        {
            return Err(Error::Configuration);
        }
        self.native = Some(native);
        Ok(self)
    }
    pub async fn run(
        self,
        cancel: &mut watch::Receiver<bool>,
        status: watch::Sender<ConnectionStatus>,
    ) -> Result<(), Error> {
        let mut backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(30))
            .map_err(|_| Error::Configuration)?;
        loop {
            status.send_replace(ConnectionStatus::Authenticating);
            let connected = tokio::select! {biased;_=crate::oauth::cancelled(cancel)=>break,result=self.connect()=>result};
            let started = Instant::now();
            let result = match connected {
                Ok(Connected {
                    connection,
                    authentication,
                    generation,
                    native,
                }) => {
                    status.send_replace(ConnectionStatus::Connected {
                        generation,
                        activated: native.is_some(),
                    });
                    self.serve(connection, authentication, generation, native, cancel)
                        .await
                }
                Err(error) => Err(error),
            };
            if matches!(result, Err(Error::Cancelled)) || *cancel.borrow() {
                break;
            }
            let denied = matches!(
                result,
                Err(Error::Denied)
                    | Err(Error::Revoked)
                    | Err(Error::Rejected(RemoteError {
                        code: ErrorCode::Unauthenticated | ErrorCode::Forbidden,
                        ..
                    }))
            );
            status.send_replace(if denied {
                ConnectionStatus::Denied
            } else {
                ConnectionStatus::Disconnected
            });
            if started.elapsed() > Duration::from_secs(30) {
                backoff.reset();
            }
            let delay = if denied {
                Duration::from_secs(60)
            } else {
                backoff.next_delay(rand::random())
            };
            tokio::select! {biased;_=crate::oauth::cancelled(cancel)=>break,_=sleep(delay)=>{}}
        }
        status.send_replace(ConnectionStatus::Stopped);
        Ok(())
    }
    async fn connect(&self) -> Result<Connected, Error> {
        let authentication = self.api.authenticate_machine(&self.credentials).await?;
        let mut endpoint = format!(
            "{}/v1/connect/host",
            self.api.origin().replacen("https://", "wss://", 1)
        );
        if let Some(native) = &self.native {
            endpoint.push_str(&format!("?native_floor={}", native.host_floor().await?.0));
        }
        let mut config = ClientConfig::new(&endpoint)
            .map_err(|_| Error::Configuration)?
            .authorization(
                SecretHeader::bearer(authentication.token.as_str())
                    .map_err(|_| Error::Configuration)?,
            );
        if let Some(path) = &self.ca {
            config = config.ca_bundle(path);
        }
        let mut connection =
            ws::connect(config, Limits::default(), None)
                .await
                .map_err(|error| {
                    if matches!(error, ws::Error::UpgradeRejected(401 | 403)) {
                        Error::Denied
                    } else {
                        Error::Unavailable
                    }
                })?;
        let mut required: std::collections::BTreeSet<String> =
            ["routing-v1", "host-dispatch-v1", "auth-refresh-v1"]
                .map(str::to_owned)
                .into();
        if self.native.is_some() {
            required.insert("host-native-authority-v1".into());
            required.insert("host-session-pages-v1".into());
            required.insert("host-generation-floor-v1".into());
        }
        let mut optional: std::collections::BTreeSet<String> = if self.native.is_some() {
            ["host-controller-v1".into(), "host-replay-v1".into()].into()
        } else {
            Default::default()
        };
        if let Some(native) = &self.native {
            let capabilities = native.capabilities().await?;
            if capabilities.iter().any(|cap| cap == "native-changes-v1") {
                optional.insert("host-changes-v1".into());
            }
            if capabilities.iter().any(|cap| cap == "native-approvals-v1") {
                optional.insert("host-approvals-v1".into());
            }
            if capabilities.iter().any(|cap| cap == "native-agent-v1") {
                optional.insert("host-agent-v1".into());
            }
        }
        send(
            &connection,
            Envelope::new(Message::Hello(Hello {
                versions: vec![Version { major: 1, minor: 0 }],
                required: required.clone(),
                optional,
            })),
            Priority::Control,
        )
        .await?;
        let handshake = async {
            let first = connection.recv().await.ok_or(Error::Unavailable)?;
            let Frame::Text(first) = &first.frame else {
                return Err(Error::Protocol);
            };
            let Message::Welcome(welcome) = &first.message else {
                return Err(Error::Protocol);
            };
            if !required
                .iter()
                .all(|capability| welcome.capabilities.contains(capability))
            {
                return Err(Error::Protocol);
            }
            let ready = connection.recv().await.ok_or(Error::Unavailable)?;
            let Frame::Text(ready) = &ready.frame else {
                return Err(Error::Protocol);
            };
            let Message::ConnectionReady {
                host_generation: Some(generation),
                activated,
                client,
            } = ready.message
            else {
                return Err(Error::Protocol);
            };
            if generation.0 == 0 || activated {
                return Err(Error::Protocol);
            }
            Ok((
                generation,
                client,
                welcome.capabilities.contains("host-approvals-v1"),
                welcome.capabilities.contains("host-agent-v1"),
                welcome.capabilities.contains("host-changes-v1"),
            ))
        };
        let (generation, client, approvals, agents, changes) =
            timeout(Duration::from_secs(8), handshake)
                .await
                .map_err(|_| Error::Unavailable)??;
        let mut inventory = Envelope::new(Message::Inventory {
            agents: Default::default(),
        });
        inventory.machine = Some(self.credentials.binding.machine);
        send(&connection, inventory, Priority::Event).await?;
        let native = if let Some(native) = &self.native {
            let link = NativeLink::connect(
                native.clone(),
                client,
                generation,
                &mut connection,
                approvals,
                agents,
                changes,
            )
            .await?;
            send(&connection, link.inventory().await?, Priority::Event).await?;
            if let Some(agents) = link.agent_inventory().await? {
                send(&connection, agents, Priority::Event).await?;
            }
            Some(link)
        } else {
            None
        };
        Ok(Connected {
            connection,
            authentication,
            generation,
            native,
        })
    }
    async fn serve(
        &self,
        mut connection: Connection,
        mut auth: MachineAuthentication,
        generation: Counter,
        mut native: Option<NativeLink>,
        cancel: &mut watch::Receiver<bool>,
    ) -> Result<(), Error> {
        let mut jobs = tokio::task::JoinSet::new();
        let mut readers = tokio::task::JoinSet::new();
        let mut uploads = tokio::task::JoinSet::new();
        let mut approvals = tokio::task::JoinSet::new();
        let mut changes = tokio::task::JoinSet::new();
        let mut changes_supported = native.as_ref().is_some_and(NativeLink::changes_available);
        let change_state = Arc::new(std::sync::Mutex::new(changes::State::default()));
        let mut next_changes = Instant::now();
        let mut next_approvals = Instant::now();
        let mut approval_rotation = 0usize;
        let mut next_upload = Instant::now();
        let mut upload_backoff = Backoff::new(Duration::from_secs(1), Duration::from_secs(30))
            .map_err(|_| Error::Configuration)?;
        let filesystem = Arc::new(self.filesystem.clone());
        let mut refreshing = None;
        let mut refresh = renew_at(auth.expires);
        let result = loop {
            tokio::select! {biased;
                _=crate::oauth::cancelled(cancel)=>break Err(Error::Cancelled),
                _=sleep_until(auth.expires)=>break Err(Error::Expired),
                _=sleep_until(next_changes),if changes.is_empty() && changes_supported=> {
                    next_changes=Instant::now()+Duration::from_millis(250);
                    if let Some((client,access))=native.as_ref().and_then(NativeLink::recording_access) {
                        changes.spawn(changes::follow(connection.sender(),client,access,change_state.clone()));
                    }
                },
                changed=changes.join_next(),if !changes.is_empty()=> {
                    if matches!(changed,Some(Ok(Err(Error::Rejected(RemoteError {code:ErrorCode::UnsupportedCapability,..}))))) {changes_supported=false;}
                    next_changes=Instant::now()+Duration::from_millis(250);
                },
                _=sleep_until(next_approvals),if approvals.is_empty()=> {
                    next_approvals=Instant::now()+Duration::from_secs(2);
                    if let Some((client,access))=native.as_ref().and_then(NativeLink::approval_access) {
                        let sender=connection.sender();let rotation=approval_rotation;approval_rotation=approval_rotation.wrapping_add(4);
                        approvals.spawn(approvals::poll(sender,client,access,rotation));
                    }
                },
                _=approvals.join_next(),if !approvals.is_empty()=>{},
                _=sleep_until(next_upload),if native.is_some() && uploads.is_empty()=> {
                    next_upload=Instant::now()+Duration::from_secs(1);
                    if let Some((client,access))=native.as_ref().and_then(NativeLink::recording_access) {
                        let api=self.api.clone();let credentials=self.credentials.clone();
                        let authentication=MachineAuthentication{token:auth.token.clone(),expires:auth.expires};
                        uploads.spawn(async move {client.upload_recording(&api,&credentials,&authentication,&access).await});
                    }
                },
                result=uploads.join_next(),if !uploads.is_empty()=> {
                    let delay=match result {
                        Some(Ok(Ok(crate::UploadProgress::Idle)))=>{upload_backoff.reset();Duration::from_secs(2)},
                        Some(Ok(Ok(_)))=>{upload_backoff.reset();Duration::from_millis(25)},
                        _=>upload_backoff.next_delay(rand::random()),
                    };
                    next_upload=Instant::now()+delay;
                },
                _=async {if let Some(native)=&native {sleep_until(native.deadline()).await}else{std::future::pending().await}},if native.is_some()=> {
                    native.as_mut().ok_or(Error::Protocol)?.tick(&connection).await?;
                },
                _=sleep_until(refresh),if refreshing.is_none()=> {
                    let updated=tokio::select! {biased;_=crate::oauth::cancelled(cancel)=>break Err(Error::Cancelled),_=sleep_until(auth.expires)=>break Err(Error::Expired),result=self.api.authenticate_machine(&self.credentials)=>result?};
                    let envelope=Envelope::new(Message::Reauthenticate {token:updated.token.clone()});refreshing=Some((envelope.id,updated,Instant::now()+Duration::from_secs(5)));send(&connection,envelope,Priority::Control).await?;
                },
                _=async {if let Some((_,_,deadline))=&refreshing {sleep_until(*deadline).await}else{std::future::pending().await}},if refreshing.is_some()=>break Err(Error::Unavailable),
                frame=connection.recv()=> {
                    let Some(frame)=frame else{break Err(Error::Unavailable);};let Frame::Text(envelope)=&frame.frame else{break Err(Error::Protocol);};
                    match &envelope.message {
                        Message::AuthRefreshed {..}=> {let Some((id,updated,_))=refreshing.take() else{break Err(Error::Protocol);};if envelope.re!=Some(id) {break Err(Error::Protocol);}auth=updated;refresh=renew_at(auth.expires);},
                        Message::HostAccessGranted {..} | Message::HostActivationConfirmed {..}=> {
                            let link=native.as_mut().ok_or(Error::Protocol)?;
                            if link.frame(&connection,envelope).await? {
                                changes.abort_all();while changes.join_next().await.is_some(){}
                                next_changes=Instant::now();
                                if !changes_supported {send(&connection,link.inventory().await?,Priority::Event).await?;}
                            }
                        },
                        Message::HostDispatch {context,message}=> {
                            if envelope.machine!=Some(self.credentials.binding.machine) {break Err(Error::Protocol);}
                            if jobs.len()>=32 {let reply=failure(message,RemoteError::rejected(ErrorCode::Overloaded,"host dispatch capacity exceeded"));send(&connection,reply,Priority::Control).await?;continue;}
                            let native_access=if let Some(native)=&native {
                                match native.access(context) {Ok(access)=>Some(access),Err(error)=> {send(&connection,failure(message,native_error(error)),Priority::Control).await?;continue;}}
                            } else if context.access.is_some() {break Err(Error::Protocol);}else{None};
                            if !valid_context(&self.credentials.binding,generation,context,message) {send(&connection,failure(message,RemoteError::rejected(ErrorCode::Forbidden,"host dispatch authority rejected")),Priority::Control).await?;continue;}
                            if matches!(message.message,Message::Attach{..}|Message::Capture|Message::ReplayRequest{..}) && native_access.is_some() {
                                if readers.len()>=4 {send(&connection,failure(message,RemoteError::rejected(ErrorCode::Overloaded,"native capture capacity exceeded")),Priority::Control).await?;continue;}
                                let (client,access)=native_access.ok_or(Error::Protocol)?;
                                readers.spawn(replay::attachment(connection.sender(),client,access,context.clone(),*message.clone()));
                                continue;
                            }
                            let filesystem=filesystem.clone();let context=context.clone();let request=*message.clone();let binding=self.credentials.binding.clone();let allow=self.allow_browse;
                            jobs.spawn(async move {dispatch(&filesystem,&binding,generation,context,request,allow,native_access).await});
                        },
                        Message::Goodbye {..}=>break Err(Error::Unavailable),
                        Message::Error(_)=>break Err(Error::Denied),
                        _=>break Err(Error::Protocol),
                    }
                },
                reply=readers.join_next(),if !readers.is_empty()=> {reply.ok_or(Error::Unavailable)?.map_err(|_|Error::Unavailable)??;},
                reply=jobs.join_next(),if !jobs.is_empty()=> {let reply=reply.ok_or(Error::Unavailable)?.map_err(|_| Error::Unavailable)?;let priority=reply_priority(&reply);send(&connection,reply,priority).await?;}
            }
        };
        readers.abort_all();
        changes.abort_all();
        while changes.join_next().await.is_some() {}
        while readers.join_next().await.is_some() {}
        jobs.abort_all();
        uploads.abort_all();
        approvals.abort_all();
        while approvals.join_next().await.is_some() {}
        while uploads.join_next().await.is_some() {}
        while jobs.join_next().await.is_some() {}
        let _ = connection.close().await;
        result
    }
}
async fn send(
    connection: &Connection,
    envelope: Envelope,
    priority: Priority,
) -> Result<(), Error> {
    transmit(&connection.sender(), envelope, priority).await
}
fn reply_priority(envelope: &Envelope) -> Priority {
    match envelope.message {
        Message::InputAck { .. }
        | Message::ApprovalApplication { .. }
        | Message::ActionResult { .. }
        | Message::HostControlResponse(_)
        | Message::Error(_) => Priority::Control,
        Message::Snapshot { .. } | Message::Replay { .. } | Message::Head { .. } => Priority::Bulk,
        _ => Priority::Event,
    }
}
async fn transmit(
    sender: &qgenie_remote_transport::ws::Sender,
    envelope: Envelope,
    priority: Priority,
) -> Result<(), Error> {
    sender
        .try_send(priority, Frame::Text(Box::new(envelope)))
        .map_err(|_| Error::Unavailable)?
        .transmitted()
        .await
        .map_err(|_| Error::Unavailable)
}
fn unix_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|time| time.as_millis().try_into().ok())
        .unwrap_or(u64::MAX)
}
fn valid_context(
    binding: &crate::Binding,
    generation: Counter,
    context: &HostContext,
    request: &Envelope,
) -> bool {
    context.actor == binding.owner
        && context.host_generation == generation
        && request.machine == Some(binding.machine)
        && request.re.is_none()
        && context.expires_at.unix_millis() > unix_ms()
}

async fn dispatch(
    filesystem: &Filesystem,
    binding: &crate::Binding,
    generation: Counter,
    context: HostContext,
    request: Envelope,
    allow: bool,
    native: Option<(crate::NativeClient, crate::NativeAccess)>,
) -> Envelope {
    if !valid_context(binding, generation, &context, &request) {
        return failure(
            &request,
            RemoteError::rejected(ErrorCode::Forbidden, "host dispatch authority rejected"),
        );
    }
    if let Some((client, access)) = &native {
        if !access.is_fresh() {
            return failure(&request, native_error(Error::Expired));
        }
        if let Message::HostControlRequest(operation) = &request.message {
            let Some(session) = request.session else {
                return failure(&request, native_error(Error::Protocol));
            };
            use qgenie_remote_proto::wire::{
                HostControlRequest as Request, HostControlResponse as Response,
            };
            let result = async {
                let result = match operation {
                    Request::Query => {
                        let state = client
                            .controller_state(access, context.client, session)
                            .await?;
                        Response::State {
                            floor: state.floor,
                            holder: state.holder,
                            remaining_ms: state.remaining_ms,
                            last_input: state.last_input,
                        }
                    }
                    Request::Activate {
                        generation,
                        remaining_ms,
                        takeover,
                        renewal,
                    } => {
                        let lease = client
                            .activate_controller(
                                access,
                                context.client,
                                session,
                                crate::NativeControlProposal {
                                    generation: *generation,
                                    remaining_lease_ms: *remaining_ms,
                                    takeover: *takeover,
                                    renewal: *renewal,
                                },
                            )
                            .await?;
                        Response::Activated {
                            holder: lease.holder,
                            generation: lease.generation,
                            remaining_ms: lease.remaining_ms,
                            last_input: lease.last_input,
                        }
                    }
                    Request::Release { generation } => {
                        client
                            .release_controller(access, context.client, session, *generation)
                            .await?;
                        Response::Released {
                            generation: *generation,
                        }
                    }
                };
                if !access.is_fresh() {
                    return Err(Error::Expired);
                }
                Ok::<_, Error>(result)
            }
            .await;
            return match result {
                Ok(result) => {
                    let mut reply = Envelope::new(Message::HostControlResponse(result));
                    reply.re = Some(request.id);
                    reply.machine = request.machine;
                    reply.session = request.session;
                    reply
                }
                Err(error) => failure(&request, native_error(error)),
            };
        }
        if matches!(
            request.message,
            Message::Input { .. }
                | Message::Resize { .. }
                | Message::Interrupt { .. }
                | Message::Terminate { .. }
                | Message::ApprovalCommitted(_)
        ) {
            return match client.effect(access, context.client, request.clone()).await {
                Ok(reply) => reply,
                Err(error) => failure(&request, native_error(error)),
            };
        }
        if let Message::Start(launch) = &request.message {
            if request.session.is_some() {
                return failure(&request, native_error(Error::Protocol));
            }
            return match client.start(access, context.client, launch.clone()).await {
                Ok(session) => {
                    let mut reply = Envelope::new(Message::Started(session));
                    reply.re = Some(request.id);
                    reply.machine = request.machine;
                    reply
                }
                Err(error) => failure(&request, native_error(error)),
            };
        }
        if matches!(
            request.message,
            Message::SessionList
                | Message::SessionListPage { .. }
                | Message::SessionGet { .. }
                | Message::StartLookup { .. }
        ) {
            return match client
                .read(access, context.actor, context.client, request.clone())
                .await
            {
                Ok(response) => response,
                Err(error) => failure(&request, native_error(error)),
            };
        }
    }
    if !matches!(request.message, Message::FsList { .. } | Message::FsRoots)
        || request.session.is_some()
    {
        return failure(
            &request,
            RemoteError::rejected(
                ErrorCode::UnsupportedCapability,
                "unsupported host directory operation",
            ),
        );
    }
    let facts = Facts {
        owner: true,
        access_fresh: true,
        revoked: false,
        same_os_user: true,
        controller: false,
        within_roots: true,
        capability: true,
        host_allows: allow,
        external: false,
        termination_confirmed: false,
        external_termination_confirmed: false,
    };
    if let Err(error) = authorize(Action::Browse, facts) {
        return failure(&request, error);
    }
    if matches!(request.message, Message::FsRoots) {
        let mut reply = Envelope::new(Message::FsRootsListed {
            roots: filesystem
                .roots()
                .paths()
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
        });
        reply.re = Some(request.id);
        reply.machine = request.machine;
        return reply;
    }
    let Message::FsList {
        path,
        cursor,
        limit,
    } = &request.message
    else {
        unreachable!()
    };
    let page = filesystem.list(path.clone(), cursor.clone(), *limit).await;
    if native
        .as_ref()
        .is_some_and(|(_, access)| !access.is_fresh())
    {
        return failure(&request, native_error(Error::Expired));
    }
    match page {
        Ok(page) => {
            let mut envelope = Envelope::new(Message::FsListed {
                entries: page.entries,
                next: page.next,
            });
            envelope.re = Some(request.id);
            envelope.machine = request.machine;
            envelope.session = request.session;
            envelope
        }
        Err(error) => {
            let code = match error {
                crate::filesystem::Error::Invalid => ErrorCode::InvalidFrame,
                crate::filesystem::Error::Denied => ErrorCode::Forbidden,
                crate::filesystem::Error::Changed => ErrorCode::HistoryTruncated,
                crate::filesystem::Error::Capacity => ErrorCode::CapacityExhausted,
                crate::filesystem::Error::Unavailable => ErrorCode::NotFound,
            };
            failure(&request, RemoteError::rejected(code, error.to_string()))
        }
    }
}
fn failure(request: &Envelope, error: RemoteError) -> Envelope {
    let mut envelope = Envelope::new(Message::Error(error));
    envelope.re = Some(request.id);
    envelope.machine = request.machine;
    envelope.session = request.session;
    envelope
}
fn native_error(error: Error) -> RemoteError {
    match error {
        Error::Rejected(error) => error,
        Error::Unavailable | Error::Protocol => RemoteError::unknown(
            "native reply unavailable; reconcile the original identity without resending input",
        ),
        Error::Denied => RemoteError::rejected(ErrorCode::Forbidden, "native scope rejected"),
        _ => RemoteError::rejected(
            ErrorCode::AccessStale,
            "native authority is unavailable or expired",
        ),
    }
}

fn renew_at(expires: Instant) -> Instant {
    let remaining = expires.saturating_duration_since(Instant::now());
    expires
        .checked_sub((remaining / 2).min(Duration::from_secs(60)))
        .unwrap_or_else(Instant::now)
}

#[cfg(test)]
mod discovery_tests {
    use super::*;
    use qgenie_remote_proto::wire::Timestamp;
    use qgenie_remote_proto::{ClientId, InstallationId, KeyId, MachineId, UserId};

    #[tokio::test]
    async fn roots_use_browse_authority_and_support_an_empty_machine() {
        let directory = tempfile::tempdir().unwrap();
        let first = directory.path().join("first");
        let second = directory.path().join("second");
        std::fs::create_dir(&first).unwrap();
        std::fs::create_dir(&second).unwrap();
        let filesystem = Filesystem::new(
            crate::filesystem::Roots::open(&[first.clone(), second.clone()]).unwrap(),
        );
        let binding = crate::Binding {
            installation: InstallationId::new(),
            key: KeyId::new(),
            machine: MachineId::new(),
            owner: UserId::new(),
            status: crate::BindingStatus::Enrolled,
        };
        let context = HostContext {
            actor: binding.owner,
            client: ClientId::new(),
            host_generation: Counter(1),
            expires_at: Timestamp::from_unix_millis(unix_ms() + 30000).unwrap(),
            access: None,
        };
        let mut request = Envelope::new(Message::FsRoots);
        request.machine = Some(binding.machine);
        let reply = dispatch(
            &filesystem,
            &binding,
            Counter(1),
            context.clone(),
            request.clone(),
            true,
            None,
        )
        .await;
        assert_eq!(reply.re, Some(request.id));
        let Message::FsRootsListed { roots } = reply.message else {
            panic!("roots missing")
        };
        assert_eq!(
            roots,
            vec![
                first.canonicalize().unwrap().display().to_string(),
                second.canonicalize().unwrap().display().to_string(),
            ]
        );
        for (authority, allow) in [
            (
                HostContext {
                    actor: UserId::new(),
                    ..context.clone()
                },
                true,
            ),
            (context.clone(), false),
            (
                HostContext {
                    host_generation: Counter(2),
                    ..context.clone()
                },
                true,
            ),
        ] {
            let reply = dispatch(
                &filesystem,
                &binding,
                Counter(1),
                authority,
                request.clone(),
                allow,
                None,
            )
            .await;
            assert!(matches!(reply.message, Message::Error(_)));
        }
        request.message = Message::FsList {
            path: "/".into(),
            cursor: None,
            limit: 32,
        };
        assert!(matches!(
            dispatch(
                &filesystem,
                &binding,
                Counter(1),
                context.clone(),
                request.clone(),
                true,
                None
            )
            .await
            .message,
            Message::Error(_)
        ));
        request.message = Message::FsList {
            path: roots[0].clone(),
            cursor: None,
            limit: 32,
        };
        assert!(matches!(
            dispatch(
                &filesystem,
                &binding,
                Counter(1),
                context,
                request,
                true,
                None
            )
            .await
            .message,
            Message::FsListed { .. }
        ));
    }
}
