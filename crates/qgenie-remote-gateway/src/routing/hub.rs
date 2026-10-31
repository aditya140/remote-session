use super::{
    RoutingConfig,
    broker::{Broker, Inbox, node_channel, user_channel},
    message::{Body, Packet, ReplayWindow},
    now_ms,
};
use crate::{Error, Gateway, MachinePrincipal};
use qgenie_remote_proto::{
    ClientId, Counter, MachineId, MessageId, SessionId, UserId,
    wire::{Envelope, Machine, Message},
};
use qgenie_remote_transport::ws::{Frame, Limits, Priority, Sender, SharedBudget};
use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};
use tokio::{
    sync::{Semaphore, mpsc, watch},
    task::JoinHandle,
};

#[derive(Clone)]
pub(super) struct Client {
    pub session_pages: bool,
    pub gate: Arc<super::authority::Gate>,
    pub owner: UserId,
    pub expires: Arc<AtomicU64>,
    pub sender: Sender,
    pub driver: qgenie_remote_transport::ws::ConnectionCancellation,
    pub stop: watch::Sender<bool>,
    pub subscriptions: HashMap<SessionId, MachineId>,
    pub inventory: Arc<tokio::sync::Mutex<Option<super::inventory::Sent>>>,
}
#[derive(Clone)]
pub(super) struct Host {
    pub agents: bool,
    pub approvals: bool,
    pub native: bool,
    pub controllers: bool,
    pub replay: bool,
    pub grants: Arc<Mutex<super::activation::Grants>>,
    pub gate: Arc<super::authority::Gate>,
    pub actor: MachinePrincipal,
    pub id: ClientId,
    pub expires: Arc<AtomicU64>,
    pub generation: Counter,
    pub sender: Sender,
    pub driver: qgenie_remote_transport::ws::ConnectionCancellation,
    pub stop: watch::Sender<bool>,
    pub hint: Option<Vec<u8>>,
}
#[derive(Clone)]
pub(super) struct Pending {
    pub owner: UserId,
    pub client: ClientId,
    pub machine: MachineId,
    pub session: Option<SessionId>,
    pub source: String,
    pub destination: Option<(String, Counter)>,
    pub deadline: Instant,
    pub replies: super::pending::Replies,
}
pub(super) struct Peers {
    pub clients: HashMap<ClientId, Client>,
    pub hosts: HashMap<MachineId, Host>,
    pub clients_pending: HashMap<MessageId, Pending>,
    pub hosts_pending: HashMap<MessageId, Pending>,
    pub replay: ReplayWindow,
}
pub(super) struct Hub {
    pub approval_status_enabled: bool,
    pub native_enabled: bool,
    pub controllers_enabled: bool,
    pub gateway: Gateway,
    pub node: String,
    pub broker: Broker,
    pub peers: Mutex<Peers>,
    pub controls: Mutex<HashMap<MessageId, super::control::Waiting>>,
    pub approval_metadata: Arc<Semaphore>,
    pub cross_node_requests: AtomicU64,
    pub observations: Arc<super::metrics::Observations>,
    pub connections: Arc<Semaphore>,
    pub snapshots: Arc<Semaphore>,
    pub stop: watch::Sender<bool>,
    pub subscribed: Mutex<HashSet<String>>,
    pub wake: tokio::sync::Notify,
    pub gaps: Mutex<HashMap<MachineId, (UserId, Counter, MessageId)>>,
    pub inventory_reads: Mutex<HashMap<UserId, Arc<tokio::sync::Mutex<u64>>>>,
    shared: SharedBudget,
    incoming: Arc<Semaphore>,
}
pub(crate) struct Routing {
    pub(super) hub: Arc<Hub>,
    worker: Mutex<Option<JoinHandle<()>>>,
}
impl Routing {
    pub(crate) fn operational_metrics(&self) -> String {
        self.hub.observations.text()
    }
    pub(crate) async fn broker_available(&self) -> bool {
        self.hub.broker.healthy().await
    }
    pub(crate) fn counts(&self) -> (usize, usize, usize, u64) {
        let peers = self.hub.peers.lock().unwrap();
        (
            peers.clients.len(),
            peers.hosts.len(),
            peers.clients_pending.len() + peers.hosts_pending.len(),
            self.hub.cross_node_requests.load(Ordering::Relaxed),
        )
    }
    pub(crate) fn fence(&self, target: &qgenie_remote_proto::rest::RevocationTarget) {
        use qgenie_remote_proto::rest::RevocationTarget;
        let peers = self.hub.peers.lock().unwrap();
        for (id, client) in &peers.clients {
            let matches = match target {
                RevocationTarget::Global => true,
                RevocationTarget::User { user } => client.owner == *user,
                RevocationTarget::Machine { machine } => {
                    client.subscriptions.values().any(|id| id == machine)
                        || peers
                            .clients_pending
                            .values()
                            .any(|pending| pending.client == *id && pending.machine == *machine)
                }
            };
            if matches {
                client.gate.deny();
                client.stop.send_replace(true);
                client.driver.cancel();
            }
        }
        for host in peers.hosts.values() {
            let matches = match target {
                RevocationTarget::Global => true,
                RevocationTarget::User { user } => host.actor.owner == *user,
                RevocationTarget::Machine { machine } => host.actor.machine == *machine,
            };
            if matches {
                host.gate.deny();
                host.stop.send_replace(true);
                host.driver.cancel();
            }
        }
    }
    pub(crate) async fn machines(&self, owner: UserId) -> Result<Vec<Machine>, Error> {
        let _permit = self
            .hub
            .snapshots
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Overloaded)?;
        let machines = self.hub.machines(owner).await?;
        if serde_json::to_vec(&machines)
            .map_err(|_| Error::Invalid)?
            .len()
            > 512 * 1024
        {
            return Err(Error::Overloaded);
        }
        Ok(machines)
    }
    pub async fn start(gateway: Gateway, config: RoutingConfig) -> Result<Self, Error> {
        if gateway.identity.is_none() {
            return Err(Error::Configuration);
        }
        let schema: Option<i32> =
            sqlx::query_scalar("SELECT max(version) FROM qgr.schema_versions")
                .fetch_one(&gateway.store.pool)
                .await
                .map_err(|_| Error::Database)?;
        if !schema.is_some_and(|schema| schema >= 4) {
            return Err(Error::Schema);
        }
        if gateway
            .identity
            .as_ref()
            .is_some_and(|identity| identity.directory.is_some())
            && !schema.is_some_and(|schema| schema >= 7)
        {
            return Err(Error::Schema);
        }
        let (stop, _) = watch::channel(false);
        let hub = Arc::new(Hub {
            approval_status_enabled: schema.is_some_and(|version| version >= 15)
                && gateway
                    .identity
                    .as_ref()
                    .is_some_and(|identity| identity.directory.is_some()),
            controllers_enabled: schema.is_some_and(|version| version >= 8),
            native_enabled: schema.is_some_and(|version| version >= 7)
                && gateway
                    .identity
                    .as_ref()
                    .is_some_and(|identity| identity.directory.is_some()),
            gateway,
            node: format!(
                "{}_{}",
                config.node,
                MessageId::new().to_string().replace('-', "")
            ),
            broker: Broker::new(&config),
            peers: Mutex::new(Peers {
                clients: HashMap::new(),
                hosts: HashMap::new(),
                clients_pending: HashMap::new(),
                hosts_pending: HashMap::new(),
                replay: ReplayWindow::new(),
            }),
            controls: Mutex::new(HashMap::new()),
            approval_metadata: Arc::new(Semaphore::new(16)),
            cross_node_requests: AtomicU64::new(0),
            observations: Arc::new(super::metrics::Observations::default()),
            connections: Arc::new(Semaphore::new(4096)),
            snapshots: Arc::new(Semaphore::new(4)),
            stop,
            subscribed: Mutex::new(HashSet::new()),
            wake: tokio::sync::Notify::new(),
            gaps: Mutex::new(HashMap::new()),
            inventory_reads: Mutex::new(HashMap::new()),
            shared: SharedBudget::new(8 * 1024 * 1024, 16 * 1024 * 1024, 32 * 1024 * 1024)
                .map_err(|_| Error::Configuration)?,
            incoming: Arc::new(Semaphore::new(16 * 1024 * 1024)),
        });
        let worker = tokio::spawn(run(hub.clone()));
        Ok(Self {
            hub,
            worker: Mutex::new(Some(worker)),
        })
    }
    pub fn drain(&self) {
        self.hub.stop.send_replace(true);
        self.hub.stop_peers();
    }
    pub async fn shutdown(&self) {
        self.drain();
        let worker = self.worker.lock().unwrap().take();
        if let Some(mut worker) = worker
            && tokio::time::timeout(Duration::from_secs(3), &mut worker)
                .await
                .is_err()
        {
            worker.abort();
            let _ = worker.await;
        }
    }
}
impl Drop for Routing {
    fn drop(&mut self) {
        self.drain();
        if let Some(worker) = self.worker.get_mut().unwrap().take() {
            worker.abort();
        }
    }
}
impl Hub {
    pub(super) async fn publish_hint(&self, host: &Host) -> Result<(), Error> {
        let access = host.grants.lock().unwrap().current().ok();
        let bytes = self
            .broker
            .set_hint(
                &self.node,
                host.actor.owner,
                host.actor.machine,
                host.id,
                host.generation,
                access,
            )
            .await?;
        if let Some(current) = self
            .peers
            .lock()
            .unwrap()
            .hosts
            .get_mut(&host.actor.machine)
            && current.id == host.id
        {
            current.hint = Some(bytes);
        }
        Ok(())
    }
    pub async fn await_subscription(&self, owner: UserId) {
        self.wake.notify_one();
        let _ = tokio::time::timeout(Duration::from_secs(3), async {
            while !self.broker.healthy().await
                || !self
                    .subscribed
                    .lock()
                    .unwrap()
                    .contains(&user_channel(owner))
            {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await;
    }
    pub fn limits(&self) -> Limits {
        Limits {
            shared_outgoing: Some(self.shared.clone()),
            shared_incoming: Some(self.incoming.clone()),
            ..Limits::default()
        }
    }
    pub fn stop_peers(&self) {
        let peers = self.peers.lock().unwrap();
        for peer in peers.clients.values() {
            peer.gate.deny();
            peer.stop.send_replace(true);
            peer.driver.cancel();
        }
        for peer in peers.hosts.values() {
            peer.gate.deny();
            peer.stop.send_replace(true);
            peer.driver.cancel();
        }
    }
    pub fn deliver(
        sender: &Sender,
        stop: &watch::Sender<bool>,
        frame: Frame,
        priority: Priority,
    ) -> Result<(), Error> {
        if *stop.borrow() {
            return Err(Error::Forbidden);
        }
        sender.try_send(priority, frame).map(|_| ()).map_err(|_| {
            stop.send_replace(true);
            Error::Overloaded
        })
    }
    pub fn client_send(&self, id: ClientId, frame: Frame, priority: Priority) -> Result<(), Error> {
        let peer = self
            .peers
            .lock()
            .unwrap()
            .clients
            .get(&id)
            .cloned()
            .ok_or(Error::NotFound)?;
        if peer.expires.load(Ordering::Acquire).saturating_mul(1000) <= now_ms()? {
            peer.stop.send_replace(true);
            return Err(Error::Unauthenticated);
        }
        Self::deliver(&peer.sender, &peer.stop, frame, priority)
    }
    pub fn resync(&self) {
        let peers = self.peers.lock().unwrap();
        for (id, peer) in &peers.clients {
            let remote = peer
                .subscriptions
                .values()
                .any(|machine| !peers.hosts.contains_key(machine))
                || peers.clients_pending.values().any(|pending| {
                    pending.client == *id && !peers.hosts.contains_key(&pending.machine)
                });
            if !remote {
                continue;
            }
            let error = qgenie_remote_proto::RemoteError {
                code: qgenie_remote_proto::ErrorCode::HistoryTruncated,
                outcome: qgenie_remote_proto::OperationOutcome::Unknown,
                message: "routing interrupted; reconnect and replay from the last sequence".into(),
                retryable: false,
            };
            let _ = Self::deliver(
                &peer.sender,
                &peer.stop,
                Frame::Text(Box::new(Envelope::new(Message::Error(error)))),
                Priority::Control,
            );
            // Close even if the resync notification itself cannot be delivered.
            peer.stop.send_replace(true);
        }
    }
    pub fn mark_gap(
        &self,
        owner: UserId,
        machine: MachineId,
        generation: Counter,
    ) -> Result<(), Error> {
        let mut gaps = self.gaps.lock().unwrap();
        if gaps.len() >= 4096 && !gaps.contains_key(&machine) {
            return Err(Error::Overloaded);
        }
        gaps.insert(machine, (owner, generation, MessageId::new()));
        Ok(())
    }
    pub fn resync_machine(&self, owner: UserId, machine: MachineId) {
        let peers = self.peers.lock().unwrap();
        for client in peers.clients.values().filter(|client| {
            client.owner == owner
                && client
                    .subscriptions
                    .values()
                    .any(|target| *target == machine)
        }) {
            let error = qgenie_remote_proto::RemoteError {
                code: qgenie_remote_proto::ErrorCode::HistoryTruncated,
                outcome: qgenie_remote_proto::OperationOutcome::Unknown,
                message: "routed output interrupted; reconnect and replay".into(),
                retryable: false,
            };
            let _ = Self::deliver(
                &client.sender,
                &client.stop,
                Frame::Text(Box::new(Envelope::new(Message::Error(error)))),
                Priority::Control,
            );
            client.stop.send_replace(true);
        }
    }
    pub async fn machines(&self, owner: UserId) -> Result<Vec<Machine>, Error> {
        let mut machines = self
            .gateway
            .identity
            .as_ref()
            .ok_or(Error::Configuration)?
            .machines(owner)
            .await?;
        self.gateway
            .store
            .enrich_machines(owner, &mut machines)
            .await?;
        for machine in &mut machines {
            machine.online = self.presence(owner, machine.id).await?;
        }
        Ok(machines)
    }
    pub async fn presence(&self, owner: UserId, machine: MachineId) -> Result<bool, Error> {
        let local = self.peers.lock().unwrap().hosts.get(&machine).cloned();
        if let Some(host) = local
            && host.actor.owner == owner
            && host.expires.load(Ordering::Acquire).saturating_mul(1000) > now_ms()?
            && !*host.stop.borrow()
        {
            return self
                .gateway
                .store
                .host_current(&host.actor, host.generation)
                .await;
        }
        let Some(hint) = self.broker.hint(machine).await.unwrap_or(None) else {
            return Ok(false);
        };
        if let Body::Hint {
            owner: hint_owner,
            machine: hint_machine,
            generation,
            ..
        } = hint.body
            && owner == hint_owner
            && machine == hint_machine
        {
            return self
                .gateway
                .store
                .route_matches(machine, &hint.source, generation)
                .await;
        }
        Ok(false)
    }
    pub async fn changed(&self, owner: UserId, machine: MachineId) {
        let packet = Packet {
            id: MessageId::new(),
            source: self.node.clone(),
            destination: user_channel(owner),
            at_ms: Counter(now_ms().unwrap_or(0)),
            body: Body::Changed { owner, machine },
        };
        let _ = self.broker.publish(&packet, Priority::Event).await;
    }
}

async fn run(hub: Arc<Hub>) {
    let mut stop = hub.stop.subscribe();
    let (sender, mut inbox) = mpsc::channel::<Inbox>(512);
    // Preserve response/event order independently of request admission. Keep each
    // Inbox byte permit through its work so decoded queues cannot evade the quota.
    let (ordered, mut ordered_inbox) = mpsc::channel::<(Packet, Inbox)>(512);
    let ordered_hub = hub.clone();
    let mut ordered_tasks = tokio::task::JoinSet::new();
    ordered_tasks.spawn(async move {
        while let Some((packet, _bytes)) = ordered_inbox.recv().await {
            let _ = ordered_hub.packet(packet).await;
        }
    });
    let mut requests = tokio::task::JoinSet::new();
    let mut maintenance = tokio::task::JoinSet::new();
    let mut approvals = tokio::task::JoinSet::new();
    let mut approval_hints = tokio::task::JoinSet::new();
    let mut subscriber = None;
    let mut channels = HashSet::new();
    let mut tick = tokio::time::interval(Duration::from_secs(2));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            _=stop.changed()=>break,
            _=ordered_tasks.join_next(),if !ordered_tasks.is_empty()=>{hub.stop.send_replace(true);break;},
            _=requests.join_next(),if !requests.is_empty()=>{},
            _=maintenance.join_next(),if !maintenance.is_empty()=>{},
            _=approvals.join_next(),if !approvals.is_empty()=>{},
            _=approval_hints.join_next(),if !approval_hints.is_empty()=>{},
            item=inbox.recv()=> {if let Some(item)=item && let Ok(packet)=hub.broker.signer.decode(&item.bytes,&item.channel,now_ms().unwrap_or(0)) {
                if matches!(packet.body,Body::ApprovalChanged{..}) {
                    if approval_hints.len()<16 {let current=hub.clone();approval_hints.spawn(async move {let _bytes=item;let _=current.packet(packet).await;});}
                } else if matches!(packet.body,Body::Request{..}) {
                    if requests.len()<32 {let current=hub.clone();requests.spawn(async move {let _bytes=item;let _=current.packet(packet).await;});}
                    else {hub.broker.lost.store(true,Ordering::Release);hub.resync();}
                } else if ordered.try_send((packet,item)).is_err() {hub.broker.lost.store(true,Ordering::Release);hub.resync();}
            }},
            _=async {tokio::select! {_=tick.tick()=>{},_=hub.wake.notified()=>{}}}=> {
                if hub.broker.lost.swap(false,Ordering::AcqRel) {hub.resync();hub.subscribed.lock().unwrap().clear();hub.broker.disconnect().await;subscriber=None;channels.clear();}
                let desired={let peers=hub.peers.lock().unwrap();let mut set=HashSet::from([node_channel(&hub.node)]);for client in peers.clients.values() {set.insert(user_channel(client.owner));}set};
                let mut failed=false;
                if subscriber.is_none() {
                    match hub.broker.connect(sender.clone(),&desired.iter().cloned().collect::<Vec<_>>()).await {Ok(connection)=> {subscriber=Some(connection);channels=desired.clone();},Err(_)=>failed=true}
                } else if let Some(connection)=&mut subscriber {
                    let add=desired.difference(&channels).cloned().collect::<Vec<_>>();let remove=channels.difference(&desired).cloned().collect::<Vec<_>>();
                    if hub.broker.subscribe(connection,&add).await.is_err() || hub.broker.unsubscribe(connection,&remove).await.is_err() || redis::cmd("PING").exec_async(connection).await.is_err() {failed=true;}else{channels=desired;}
                }
                if failed {if subscriber.is_some() {hub.resync();}hub.subscribed.lock().unwrap().clear();hub.broker.disconnect().await;subscriber=None;channels.clear();}
                *hub.subscribed.lock().unwrap()=channels.clone();
                if maintenance.is_empty() {let current=hub.clone();maintenance.spawn(async move {reconcile(&current).await;flush_gaps(&current).await;});}
                if approvals.is_empty() {let current=hub.clone();approvals.spawn(async move {let _=current.deliver_approvals().await;});}
            }
        }
    }
    hub.stop_peers();
    requests.abort_all();
    maintenance.abort_all();
    approvals.abort_all();
    approval_hints.abort_all();
    ordered_tasks.abort_all();
    while requests.join_next().await.is_some() {}
    while maintenance.join_next().await.is_some() {}
    while approvals.join_next().await.is_some() {}
    while approval_hints.join_next().await.is_some() {}
    while ordered_tasks.join_next().await.is_some() {}
    hub.broker.disconnect().await;
}

async fn reconcile(hub: &Hub) {
    hub.expire_pending();
    let hosts = hub
        .peers
        .lock()
        .unwrap()
        .hosts
        .values()
        .cloned()
        .collect::<Vec<_>>();
    for host in hosts {
        if host.expires.load(Ordering::Acquire).saturating_mul(1000) <= now_ms().unwrap_or(u64::MAX)
            || !hub
                .gateway
                .store
                .host_current(&host.actor, host.generation)
                .await
                .unwrap_or(false)
        {
            host.stop.send_replace(true);
            continue;
        }
        let _ = hub
            .gateway
            .store
            .seen(host.actor.machine, host.generation)
            .await;
        let _ = hub.publish_hint(&host).await;
    }
    let owners = {
        let peers = hub.peers.lock().unwrap();
        let owners = peers
            .clients
            .values()
            .map(|peer| peer.owner)
            .collect::<HashSet<_>>();
        hub.inventory_reads
            .lock()
            .unwrap()
            .retain(|owner, _| owners.contains(owner));
        owners
    };
    for owner in owners {
        if hub.reconcile_inventory(owner).await.is_err() {
            let peers = hub.peers.lock().unwrap();
            for client in peers
                .clients
                .values()
                .filter(|client| client.owner == owner)
            {
                client.stop.send_replace(true);
            }
        }
    }
}

async fn flush_gaps(hub: &Hub) {
    let gaps = hub
        .gaps
        .lock()
        .unwrap()
        .iter()
        .map(|(machine, (owner, generation, revision))| (*machine, *owner, *generation, *revision))
        .collect::<Vec<_>>();
    for (machine, owner, generation, revision) in gaps {
        match hub
            .gateway
            .store
            .route_matches(machine, &hub.node, generation)
            .await
        {
            Ok(false) => {
                hub.gaps.lock().unwrap().remove(&machine);
                continue;
            }
            Err(_) => continue,
            Ok(true) => {}
        }
        let packet = Packet {
            id: MessageId::new(),
            source: hub.node.clone(),
            destination: user_channel(owner),
            at_ms: Counter(now_ms().unwrap_or(0)),
            body: Body::Resync {
                owner,
                machine,
                generation,
            },
        };
        if hub.broker.publish(&packet, Priority::Control).await.is_ok() {
            let mut gaps = hub.gaps.lock().unwrap();
            if gaps.get(&machine) == Some(&(owner, generation, revision)) {
                gaps.remove(&machine);
            }
        }
    }
}
