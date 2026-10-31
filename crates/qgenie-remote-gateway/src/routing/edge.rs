use super::{
    hub::{Client, Host, Hub},
    now_ms,
    route::error_frame,
    socket::Socket,
};
use crate::{Error, Gateway, MachinePrincipal, Principal};
use axum::{
    extract::{RawQuery, State, WebSocketUpgrade, ws::WebSocket},
    http::HeaderMap,
    response::Response,
};
use qgenie_remote_proto::{
    ClientId, Version, negotiate,
    wire::{Envelope, Message, Timestamp},
};
use qgenie_remote_transport::ws::{Connection, Frame, Priority};
use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, mpsc, watch};

pub(crate) async fn client_upgrade(
    State(gateway): State<Gateway>,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, Error> {
    let actor = gateway.authenticate_client(&headers).await?;
    let hub = gateway
        .routing
        .as_ref()
        .ok_or(Error::Unavailable)?
        .hub
        .clone();
    let permit = hub
        .connections
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::Overloaded)?;
    Ok(configure(ws).on_upgrade(move |socket| async move {
        if let Err(error) = serve_client(hub, socket, actor, permit).await {
            tracing::debug!(?error, "client connection closed");
        }
    }))
}
pub(crate) async fn host_upgrade(
    State(gateway): State<Gateway>,
    RawQuery(query): RawQuery,
    headers: HeaderMap,
    ws: WebSocketUpgrade,
) -> Result<Response, Error> {
    let actor = gateway.authenticate_host(&headers).await?;
    let floor = host_floor(query.as_deref())?;
    let hub = gateway
        .routing
        .as_ref()
        .ok_or(Error::Unavailable)?
        .hub
        .clone();
    let permit = hub
        .connections
        .clone()
        .try_acquire_owned()
        .map_err(|_| Error::Overloaded)?;
    Ok(configure(ws).on_upgrade(move |socket| async move {
        if let Err(error) = serve_host(hub, socket, actor, floor, permit).await {
            tracing::debug!(?error, "host connection closed");
        }
    }))
}
fn host_floor(query: Option<&str>) -> Result<qgenie_remote_proto::Counter, Error> {
    let Some(query) = query else {
        return Ok(qgenie_remote_proto::Counter(0));
    };
    let value = query.strip_prefix("native_floor=").ok_or(Error::Invalid)?;
    if value.is_empty()
        || value.len() > 20
        || !value.bytes().all(|b| b.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return Err(Error::Invalid);
    }
    Ok(qgenie_remote_proto::Counter(
        value.parse().map_err(|_| Error::Invalid)?,
    ))
}
fn configure(ws: WebSocketUpgrade) -> WebSocketUpgrade {
    ws.read_buffer_size(16384)
        .write_buffer_size(16384)
        .max_write_buffer_size(2 * 1024 * 1024)
        .max_frame_size(1024 * 1024)
        .max_message_size(1024 * 1024)
}
struct Negotiated {
    native: bool,
    controllers: bool,
    replay: bool,
    approvals: bool,
    agents: bool,
    prompt_receipts: bool,
    session_pages: bool,
}
async fn hello(
    connection: &mut Connection,
    client: bool,
    native: bool,
    controllers: bool,
    approval_status: bool,
) -> Result<Negotiated, Error> {
    let frame = tokio::time::timeout(Duration::from_secs(5), connection.recv())
        .await
        .map_err(|_| Error::Invalid)?
        .ok_or(Error::Invalid)?;
    let Frame::Text(envelope) = &frame.frame else {
        return Err(Error::Invalid);
    };
    let Message::Hello(hello) = &envelope.message else {
        return Err(Error::Invalid);
    };
    let mut caps: std::collections::BTreeSet<String> = [
        "routing-v1",
        "auth-refresh-v1",
        "machine-pages-v1",
        "session-pages-v1",
        "host-session-pages-v1",
        "root-discovery-v1",
        "host-dispatch-v1",
    ]
    .map(str::to_owned)
    .into();
    if native {
        caps.insert("host-changes-v1".into());
        if !client
            || hello.required.contains("session-pages-v1")
            || hello.optional.contains("session-pages-v1")
        {
            caps.insert("session-changes-v1".into());
        }
        caps.insert("host-agent-v1".into());
        caps.insert("host-prompt-receipts-v1".into());
        caps.insert("prompt-receipts-v1".into());
        caps.insert("agent-events-v1".into());
        caps.insert("host-native-authority-v1".into());
        caps.insert("host-generation-floor-v1".into());
        caps.insert("host-replay-v1".into());
        caps.insert("session-replay-v1".into());
        if controllers {
            caps.insert("host-controller-v1".into());
            caps.insert("session-control-v1".into());
        }
    }
    if client && approval_status {
        caps.insert("approval-status-v1".into());
        caps.insert("durable-approvals-v1".into());
    }
    if native && approval_status {
        caps.insert("host-approvals-v1".into());
    }
    let welcome =
        negotiate(hello, &[Version { major: 1, minor: 0 }], &caps).map_err(|_| Error::Invalid)?;
    if !welcome.capabilities.contains("routing-v1")
        || (client && !welcome.capabilities.contains("machine-pages-v1"))
        || (!client && !welcome.capabilities.contains("host-dispatch-v1"))
        || (!client && native && !welcome.capabilities.contains("host-native-authority-v1"))
    {
        return Err(Error::Invalid);
    }
    let native = welcome.capabilities.contains("host-native-authority-v1");
    let controllers = welcome.capabilities.contains("host-controller-v1");
    let replay = welcome.capabilities.contains("host-replay-v1");
    let approvals = welcome.capabilities.contains("host-approvals-v1");
    let agents = welcome.capabilities.contains("host-agent-v1");
    let prompt_receipts = welcome.capabilities.contains("host-prompt-receipts-v1");
    let session_pages = welcome.capabilities.contains("session-pages-v1");
    connection
        .sender()
        .try_send(
            Priority::Control,
            Frame::Text(Box::new(Envelope::new(Message::Welcome(welcome)))),
        )
        .map_err(|_| Error::Overloaded)?;
    Ok(Negotiated {
        native,
        controllers,
        replay,
        approvals,
        agents,
        prompt_receipts,
        session_pages,
    })
}
struct Registration {
    hub: Arc<Hub>,
    id: ClientId,
    machine: Option<qgenie_remote_proto::MachineId>,
}
impl Drop for Registration {
    fn drop(&mut self) {
        let mut peers = self.hub.peers.lock().unwrap();
        if let Some(machine) = self.machine {
            if peers
                .hosts
                .get(&machine)
                .is_some_and(|host| host.id == self.id)
            {
                peers.hosts.remove(&machine);
            }
        } else {
            peers.clients.remove(&self.id);
            peers
                .clients_pending
                .retain(|_, pending| pending.client != self.id);
        }
    }
}
async fn serve_client(
    hub: Arc<Hub>,
    socket: WebSocket,
    actor: Principal,
    _permit: OwnedSemaphorePermit,
) -> Result<(), Error> {
    if *hub.stop.borrow() {
        return Err(Error::Draining);
    }
    let expires = Arc::new(AtomicU64::new(actor.expires_at));
    let watched = super::authority::Actor::User(actor.user);
    let gate = super::authority::Watch::admit(&hub, &watched, &expires).await?;
    let mut connection = Connection::adopt(
        Socket(socket, gate.clone()),
        hub.limits(),
        Some(hub.observations.hook()),
    )
    .map_err(|_| Error::Invalid)?;
    let (stop, mut cancelled) = watch::channel(false);
    let _authority = super::authority::Watch::start(
        hub.clone(),
        watched,
        expires.clone(),
        stop.clone(),
        connection.cancellation(),
        gate.clone(),
    );
    let negotiated = hello(
        &mut connection,
        true,
        hub.native_enabled,
        hub.controllers_enabled,
        hub.approval_status_enabled,
    )
    .await?;
    if *stop.borrow() {
        return Err(Error::Forbidden);
    }
    let id = ClientId::new();
    // A published peer can receive owner-wide events immediately. Finish the
    // handshake before publication; queue priority alone cannot prevent an event
    // from overtaking an untransmitted ConnectionReady frame.
    tokio::time::timeout(
        Duration::from_secs(5),
        connection
            .sender()
            .try_send(
                Priority::Control,
                Frame::Text(Box::new(Envelope::new(Message::ConnectionReady {
                    client: id,
                    host_generation: None,
                    activated: false,
                }))),
            )
            .map_err(|_| Error::Overloaded)?
            .transmitted(),
    )
    .await
    .map_err(|_| Error::Unavailable)?
    .map_err(|_| Error::Unavailable)?;
    {
        let mut peers = hub.peers.lock().unwrap();
        if peers
            .clients
            .values()
            .filter(|client| client.owner == actor.user)
            .count()
            >= 32
            || (!peers
                .clients
                .values()
                .any(|client| client.owner == actor.user)
                && peers
                    .clients
                    .values()
                    .map(|client| client.owner)
                    .collect::<std::collections::HashSet<_>>()
                    .len()
                    >= 128)
        {
            return Err(Error::Overloaded);
        }
        peers.clients.insert(
            id,
            Client {
                session_pages: negotiated.session_pages,
                owner: actor.user,
                expires: expires.clone(),
                sender: connection.sender(),
                driver: connection.cancellation(),
                gate: gate.clone(),
                stop,
                subscriptions: HashMap::new(),
                inventory: Arc::new(tokio::sync::Mutex::new(None)),
            },
        );
    }
    let _registration = Registration {
        hub: hub.clone(),
        id,
        machine: None,
    };
    hub.await_subscription(actor.user).await;
    hub.pages(id, actor.user).await?;
    let result = async { loop {
        tokio::select! {
            _=cancelled.changed()=>break Ok(()),
            received=connection.recv()=> {
                let Some(received)=received else {break Ok(());};
                if expires.load(Ordering::Acquire).saturating_mul(1000)<=now_ms()? {break Err(Error::Unauthenticated);}
                let re=if let Frame::Text(envelope)=&received.frame {Some(envelope.id)}else{None};
                let result=if let Frame::Text(envelope)=&received.frame && let Message::Reauthenticate {token}=&envelope.message {
                    let mut headers=HeaderMap::new();headers.insert(axum::http::header::AUTHORIZATION,format!("Bearer {}",token.as_str()).parse().map_err(|_| Error::Unauthenticated)?);
                    let fresh=hub.gateway.authenticate_client(&headers).await?;
                    if fresh.user!=actor.user {break Err(Error::Forbidden);}
                    gate.refresh_credential(fresh.expires_at)?;
                    expires.store(fresh.expires_at,Ordering::Release);
                    refreshed(&connection,fresh.expires_at,re)
                } else {hub.client_frame(id,received.frame.clone()).await};
                if let Err(error)=result {let _=hub.client_send(id,error_frame(re,error,false),Priority::Control);}
            }
        }
    }}.await;
    let _ = connection.close().await;
    result
}
async fn serve_host(
    hub: Arc<Hub>,
    socket: WebSocket,
    actor: MachinePrincipal,
    floor: qgenie_remote_proto::Counter,
    _permit: OwnedSemaphorePermit,
) -> Result<(), Error> {
    if *hub.stop.borrow() {
        return Err(Error::Draining);
    }
    let expires = Arc::new(AtomicU64::new(actor.expires_at));
    let generation_cell = Arc::new(AtomicU64::new(0));
    let watched = super::authority::Actor::Host(actor.clone(), generation_cell.clone());
    let gate = super::authority::Watch::admit(&hub, &watched, &expires).await?;
    let mut connection = Connection::adopt(
        Socket(socket, gate.clone()),
        hub.limits(),
        Some(hub.observations.hook()),
    )
    .map_err(|_| Error::Invalid)?;
    let (stop, mut cancelled) = watch::channel(false);
    let _authority = super::authority::Watch::start(
        hub.clone(),
        watched,
        expires.clone(),
        stop.clone(),
        connection.cancellation(),
        gate.clone(),
    );
    let Negotiated {
        native,
        controllers,
        replay,
        approvals,
        agents,
        prompt_receipts,
        ..
    } = hello(
        &mut connection,
        false,
        hub.native_enabled,
        hub.controllers_enabled,
        hub.approval_status_enabled,
    )
    .await?;
    let id = ClientId::new();
    let generation = hub
        .gateway
        .store
        .connect_host_with_floor(&actor, &hub.node, id, floor)
        .await?;
    generation_cell.store(generation.0, Ordering::Release);
    if *stop.borrow() {
        return Err(Error::Forbidden);
    }
    let host = Host {
        agents,
        prompt_receipts,
        approvals,
        native,
        controllers,
        replay,
        grants: Arc::new(std::sync::Mutex::new(Default::default())),
        actor: actor.clone(),
        id,
        expires: expires.clone(),
        generation,
        sender: connection.sender(),
        driver: connection.cancellation(),
        gate: gate.clone(),
        stop,
    };
    if let Some(old) = hub
        .peers
        .lock()
        .unwrap()
        .hosts
        .insert(actor.machine, host.clone())
    {
        old.gate.deny();
        old.stop.send_replace(true);
        old.driver.cancel();
    }
    let _registration = Registration {
        hub: hub.clone(),
        id,
        machine: Some(actor.machine),
    };
    connection
        .sender()
        .try_send(
            Priority::Control,
            Frame::Text(Box::new(Envelope::new(Message::ConnectionReady {
                client: id,
                host_generation: Some(generation),
                activated: false,
            }))),
        )
        .map_err(|_| Error::Overloaded)?;
    // Presence can rebuild from the authenticated live connection after Redis reconnects.
    let _ = hub.publish_hint(&host).await;
    hub.changed(actor.owner, actor.machine).await;
    let (metadata_send, mut metadata_receive) =
        mpsc::channel::<qgenie_remote_transport::ws::ReceivedFrame>(4);
    let mut metadata = tokio::task::JoinSet::new();
    let metadata_hub = hub.clone();
    let metadata_actor = actor.clone();
    metadata.spawn(async move {
        while let Some(received) = metadata_receive.recv().await {
            let Ok(_permit) = metadata_hub.approval_metadata.clone().try_acquire_owned() else {
                continue;
            };
            // Retain the transport's original byte permit through metadata work.
            let _ = metadata_hub
                .host_frame(&metadata_actor, generation, received.frame.clone())
                .await;
            drop(received);
        }
    });
    let result = async { loop {
        tokio::select! {
            _=cancelled.changed()=>break Ok(()),
            received=connection.recv()=> {
                let Some(received)=received else {break Ok(());};
                if expires.load(Ordering::Acquire).saturating_mul(1000)<=now_ms()? {break Err(Error::Unauthenticated);}
                if matches!(&received.frame,Frame::Text(envelope) if envelope.re.is_none() && matches!(envelope.message,Message::Sessions{..}|Message::SessionUpdates{..}|Message::Head{..}|Message::Approvals{..})) {
                    let _=metadata_send.try_send(received);
                } else if let Frame::Text(envelope)=&received.frame && let Message::Reauthenticate {token}=&envelope.message {
                    let identity=hub.gateway.identity.as_ref().ok_or(Error::Configuration)?;
                    let fresh=identity.authenticate_machine(token.as_str()).await?;
                    if fresh.machine!=actor.machine || fresh.owner!=actor.owner || fresh.key!=actor.key {break Err(Error::Forbidden);}
                    gate.refresh_credential(fresh.expires_at)?;
                    expires.store(fresh.expires_at,Ordering::Release);refreshed(&connection,fresh.expires_at,Some(envelope.id))?;
                } else if let Err(error)=hub.host_frame(&actor,generation,received.frame.clone()).await {break Err(error);}
            }
        }
    }}.await;
    metadata.abort_all();
    while metadata.join_next().await.is_some() {}
    drop(_registration);
    let _ = hub.broker.clear_hint(actor.machine, generation).await;
    hub.changed(actor.owner, actor.machine).await;
    let _ = connection.close().await;
    result
}
fn refreshed(
    connection: &Connection,
    expires: u64,
    re: Option<qgenie_remote_proto::MessageId>,
) -> Result<(), Error> {
    let mut envelope = Envelope::new(Message::AuthRefreshed {
        expires_at: Timestamp::from_unix_millis(expires.saturating_mul(1000))
            .map_err(|_| Error::Invalid)?,
    });
    envelope.re = re;
    connection
        .sender()
        .try_send(Priority::Control, Frame::Text(Box::new(envelope)))
        .map_err(|_| Error::Overloaded)?;
    Ok(())
}
