use super::{
    config::RoutingConfig,
    message::{Body, MAX_ROUTED_BYTES, Packet, Signer},
    now_ms,
};
use crate::Error;
use qgenie_remote_proto::{ClientId, Counter, MachineId, MessageId, UserId};
use qgenie_remote_transport::ws::Priority;
use redis::{
    AsyncConnectionConfig, PushInfo, PushKind,
    aio::{AsyncPushSender, MultiplexedConnection, SendError},
};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::sync::{OwnedSemaphorePermit, RwLock, Semaphore, mpsc};

pub(super) struct Inbox {
    pub channel: String,
    pub bytes: Vec<u8>,
    _bytes: OwnedSemaphorePermit,
}
struct Push {
    sender: mpsc::Sender<Inbox>,
    quota: Arc<Semaphore>,
    lost: Arc<AtomicBool>,
    active: Arc<AtomicBool>,
}
impl AsyncPushSender for Push {
    fn send(&self, info: PushInfo) -> Result<(), SendError> {
        if !self.active.load(Ordering::Acquire) {
            return Ok(());
        }
        if info.kind == PushKind::Disconnection {
            self.lost.store(true, Ordering::Release);
            return Err(SendError);
        }
        if info.kind != PushKind::SMessage {
            return Ok(());
        }
        let mut fields = info.data.into_iter();
        let channel = fields
            .next()
            .and_then(|value| redis::from_redis_value::<String>(value).ok())
            .ok_or(SendError)?;
        let bytes = fields
            .next()
            .and_then(|value| redis::from_redis_value::<Vec<u8>>(value).ok())
            .ok_or(SendError)?;
        if bytes.len() > MAX_ROUTED_BYTES + 32 {
            self.lost.store(true, Ordering::Release);
            return Err(SendError);
        }
        let permit = self
            .quota
            .clone()
            .try_acquire_many_owned(bytes.len() as u32)
            .map_err(|_| {
                self.lost.store(true, Ordering::Release);
                SendError
            })?;
        self.sender
            .try_send(Inbox {
                channel,
                bytes,
                _bytes: permit,
            })
            .map_err(|_| {
                self.lost.store(true, Ordering::Release);
                SendError
            })
    }
}
struct Publishers {
    connections: [MultiplexedConnection; 3],
    active: Arc<AtomicBool>,
}
pub(super) struct Broker {
    client: redis::Client,
    pub signer: Signer,
    publishers: RwLock<Option<Publishers>>,
    permits: [Arc<Semaphore>; 3],
    bytes: [Arc<Semaphore>; 3],
    pub lost: Arc<AtomicBool>,
}
impl Broker {
    pub fn new(config: &RoutingConfig) -> Self {
        Self {
            client: config.client.clone(),
            signer: Signer::new(config.key),
            publishers: RwLock::new(None),
            permits: [32, 32, 32].map(|count| Arc::new(Semaphore::new(count))),
            bytes: [2 * 1024 * 1024, 4 * 1024 * 1024, 16 * 1024 * 1024]
                .map(|bytes| Arc::new(Semaphore::new(bytes))),
            lost: Arc::new(AtomicBool::new(false)),
        }
    }
    pub async fn connect(
        &self,
        sender: mpsc::Sender<Inbox>,
        channels: &[String],
    ) -> Result<MultiplexedConnection, Error> {
        let config = AsyncConnectionConfig::new()
            .set_connection_timeout(Some(Duration::from_secs(2)))
            .set_response_timeout(Some(Duration::from_secs(2)))
            .set_pipeline_buffer_size(32)
            .set_concurrency_limit(32)
            .set_write_backpressure_boundary(16384);
        let (control, event, bulk) = tokio::try_join!(
            self.client
                .get_multiplexed_async_connection_with_config(&config),
            self.client
                .get_multiplexed_async_connection_with_config(&config),
            self.client
                .get_multiplexed_async_connection_with_config(&config)
        )
        .map_err(|_| Error::Unavailable)?;
        let active = Arc::new(AtomicBool::new(true));
        let result = async {
            let config = config.set_push_sender(Push {
                sender,
                quota: Arc::new(Semaphore::new(16 * 1024 * 1024)),
                lost: self.lost.clone(),
                active: active.clone(),
            });
            let mut subscriber = self
                .client
                .get_multiplexed_async_connection_with_config(&config)
                .await
                .map_err(|_| Error::Unavailable)?;
            self.subscribe(&mut subscriber, channels).await?;
            *self.publishers.write().await = Some(Publishers {
                connections: [control, event, bulk],
                active: active.clone(),
            });
            Ok(subscriber)
        }
        .await;
        if result.is_err() {
            active.store(false, Ordering::Release);
        }
        result
    }
    pub async fn healthy(&self) -> bool {
        self.publishers
            .read()
            .await
            .as_ref()
            .is_some_and(|state| state.active.load(Ordering::Acquire))
            && !self.lost.load(Ordering::Acquire)
    }
    pub async fn disconnect(&self) {
        if let Some(state) = self.publishers.write().await.take() {
            state.active.store(false, Ordering::Release);
        }
        self.lost.store(false, Ordering::Release);
    }
    fn failed(&self, active: &AtomicBool) -> Error {
        if active.load(Ordering::Acquire) {
            self.lost.store(true, Ordering::Release);
        }
        Error::Unavailable
    }
    pub async fn subscribe(
        &self,
        subscriber: &mut MultiplexedConnection,
        channels: &[String],
    ) -> Result<(), Error> {
        if !channels.is_empty() {
            redis::cmd("SSUBSCRIBE")
                .arg(channels)
                .exec_async(subscriber)
                .await
                .map_err(|_| Error::Unavailable)?;
        }
        Ok(())
    }
    pub async fn unsubscribe(
        &self,
        subscriber: &mut MultiplexedConnection,
        channels: &[String],
    ) -> Result<(), Error> {
        if !channels.is_empty() {
            redis::cmd("SUNSUBSCRIBE")
                .arg(channels)
                .exec_async(subscriber)
                .await
                .map_err(|_| Error::Unavailable)?;
        }
        Ok(())
    }
    async fn connection(
        &self,
        index: usize,
    ) -> Result<(MultiplexedConnection, Arc<AtomicBool>), Error> {
        self.publishers
            .read()
            .await
            .as_ref()
            .map(|state| (state.connections[index].clone(), state.active.clone()))
            .ok_or(Error::Unavailable)
    }
    pub async fn publish(&self, packet: &Packet, priority: Priority) -> Result<usize, Error> {
        let index = match priority {
            Priority::Control => 0,
            Priority::Event => 1,
            Priority::Bulk => 2,
        };
        let _count = self.permits[index]
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Overloaded)?;
        let bytes = self.signer.encode(packet)?;
        let _bytes = self.bytes[index]
            .clone()
            .try_acquire_many_owned(bytes.len() as u32)
            .map_err(|_| Error::Overloaded)?;
        let (mut connection, active) = self.connection(index).await?;
        redis::cmd("SPUBLISH")
            .arg(&packet.destination)
            .arg(bytes)
            .query_async(&mut connection)
            .await
            .map_err(|_| self.failed(&active))
    }
    pub async fn hint(&self, machine: MachineId) -> Result<Option<Packet>, Error> {
        let (mut connection, active) = self.connection(0).await?;
        let value: Option<Vec<u8>> = redis::cmd("HGET")
            .arg(hint_key(machine))
            .arg("value")
            .query_async(&mut connection)
            .await
            .map_err(|_| self.failed(&active))?;
        value
            .map(|bytes| self.signer.decode(&bytes, &hint_key(machine), now_ms()?))
            .transpose()
    }
    pub async fn set_hint(
        &self,
        node: &str,
        owner: UserId,
        machine: MachineId,
        connection: ClientId,
        generation: Counter,
        access: Option<qgenie_remote_proto::wire::HostAccess>,
    ) -> Result<(), Error> {
        let packet = Packet {
            id: MessageId::new(),
            source: node.into(),
            destination: hint_key(machine),
            at_ms: Counter(now_ms()?),
            body: Body::Hint {
                owner,
                machine,
                connection,
                generation,
                access,
            },
        };
        let bytes = self.signer.encode(&packet)?;
        let (mut connection, active) = self.connection(0).await?;
        let script = redis::Script::new(
            "local old=redis.call('HGET',KEYS[1],'generation'); if not old or ARGV[1]>=old then redis.call('HSET',KEYS[1],'generation',ARGV[1],'value',ARGV[2]); redis.call('EXPIRE',KEYS[1],25); return 1 end; return 0",
        );
        let _: u8 = script
            .key(hint_key(machine))
            .arg(format!("{:020}", generation.0))
            .arg(&bytes)
            .invoke_async(&mut connection)
            .await
            .map_err(|_| self.failed(&active))?;
        Ok(())
    }
    pub async fn clear_hint(&self, machine: MachineId, generation: Counter) -> Result<(), Error> {
        let (mut connection, active) = self.connection(0).await?;
        let script = redis::Script::new(
            "if redis.call('HGET',KEYS[1],'generation')==ARGV[1] then return redis.call('DEL',KEYS[1]) end; return 0",
        );
        let _: u8 = script
            .key(hint_key(machine))
            .arg(format!("{:020}", generation.0))
            .invoke_async(&mut connection)
            .await
            .map_err(|_| self.failed(&active))?;
        Ok(())
    }
}
pub(super) fn hint_key(machine: MachineId) -> String {
    format!("qgr:machine:{{{machine}}}")
}
pub(super) fn node_channel(node: &str) -> String {
    format!("qgr:node:{{{node}}}")
}
pub(super) fn user_channel(owner: UserId) -> String {
    format!("qgr:user:{{{owner}}}")
}
