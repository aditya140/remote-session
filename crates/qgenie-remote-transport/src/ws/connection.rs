pub use super::queues::{Sender, Transmission};
use super::{CloseReason, Error, Limits, queues::Queues};
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use qgenie_remote_proto::{
    ErrorCode, MAX_JSON_BYTES, PtyFrame, RemoteError, decode_json, wire::Envelope,
};
use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};
#[cfg(feature = "server-ws")]
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::{
    sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot},
    task::JoinHandle,
    time::{sleep, timeout},
};
use tokio_tungstenite::tungstenite::{
    self, Message,
    protocol::{CloseFrame, WebSocketConfig, frame::coding::CloseCode},
};

/// Hook runs inline and must be nonblocking. Events never include payloads or credentials.
pub type Metrics = Arc<dyn Fn(TransportEvent) + Send + Sync>;
#[derive(Debug, Clone, Copy)]
pub enum TransportEvent {
    Connected,
    BytesIn(usize),
    BytesOut(usize),
    QueueFull(super::Priority),
    Closed,
    HeartbeatTimeout,
}
#[derive(Debug, Clone)]
pub enum Frame {
    Text(Box<Envelope>),
    Binary(PtyFrame),
}
impl Frame {
    pub(super) fn encode(self) -> Result<Message, RemoteError> {
        match self {
            Self::Text(envelope) => {
                envelope.validate()?;
                let bytes = serde_json::to_string(&envelope).map_err(|_| {
                    RemoteError::rejected(ErrorCode::InvalidFrame, "invalid JSON envelope")
                })?;
                if bytes.len() > MAX_JSON_BYTES {
                    return Err(RemoteError::rejected(
                        ErrorCode::FrameTooLarge,
                        "JSON frame too large",
                    ));
                }
                Ok(Message::Text(bytes.into()))
            }
            Self::Binary(frame) => Ok(Message::Binary(frame.encode()?.into())),
        }
    }
}
/// Holds its queue byte budget until dropped. Copying payloads creates caller-owned memory.
#[derive(Debug)]
pub struct ReceivedFrame {
    pub frame: Frame,
    _bytes: OwnedSemaphorePermit,
    _shared: Option<OwnedSemaphorePermit>,
}
pub struct Connection {
    sender: Sender,
    incoming: mpsc::Receiver<ReceivedFrame>,
    close: Option<oneshot::Sender<()>>,
    task: Option<JoinHandle<Result<CloseReason, Error>>>,
    result: Option<Result<CloseReason, Error>>,
}
/// Cancels the driver even while its owner is awaiting application work or the
/// socket writer is stalled. Dropping this handle has no effect.
#[derive(Clone)]
pub struct ConnectionCancellation(Option<tokio::task::AbortHandle>);
impl ConnectionCancellation {
    pub fn cancel(&self) {
        if let Some(task) = &self.0 {
            task.abort();
        }
    }
}
impl Connection {
    /// Adopt a framework WebSocket after the caller has authenticated its upgrade.
    pub fn adopt<S, E>(socket: S, limits: Limits, metrics: Option<Metrics>) -> Result<Self, Error>
    where
        S: Stream<Item = Result<Message, E>> + Sink<Message, Error = E> + Unpin + Send + 'static,
        E: Into<Error> + Send + 'static,
    {
        limits.validate()?;
        Ok(start(socket, limits, metrics))
    }
    pub fn sender(&self) -> Sender {
        self.sender.clone()
    }
    pub fn cancellation(&self) -> ConnectionCancellation {
        ConnectionCancellation(self.task.as_ref().map(JoinHandle::abort_handle))
    }
    pub async fn recv(&mut self) -> Option<ReceivedFrame> {
        self.incoming.recv().await
    }
    pub async fn closed(&mut self) -> Result<CloseReason, Error> {
        if let Some(result) = &self.result {
            return result.clone();
        }
        let result = self
            .task
            .as_mut()
            .ok_or(Error::Task)?
            .await
            .unwrap_or(Err(Error::Task));
        self.task.take();
        self.result = Some(result.clone());
        result
    }
    pub async fn close(mut self) -> Result<CloseReason, Error> {
        if let Some(close) = self.close.take() {
            let _ = close.send(());
        }
        self.closed().await
    }
}
impl Drop for Connection {
    fn drop(&mut self) {
        if let Some(task) = &self.task {
            task.abort();
        }
    }
}
pub(super) fn config() -> WebSocketConfig {
    WebSocketConfig::default()
        .read_buffer_size(16 * 1024)
        .write_buffer_size(16 * 1024)
        .max_write_buffer_size(2 * MAX_JSON_BYTES)
        .max_message_size(Some(MAX_JSON_BYTES))
        .max_frame_size(Some(MAX_JSON_BYTES))
}
#[cfg(feature = "server-ws")]
/// Accept an already authenticated stream. TLS termination and authorization belong to the caller.
pub async fn accept<S>(
    stream: S,
    limits: Limits,
    metrics: Option<Metrics>,
) -> Result<Connection, Error>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    limits.validate()?;
    let socket = timeout(
        std::time::Duration::from_secs(10),
        tokio_tungstenite::accept_async_with_config(stream, Some(config())),
    )
    .await
    .map_err(|_| Error::Network)?
    .map_err(ws_error)?;
    Ok(start(socket, limits, metrics))
}
enum Internal {
    Message(Message),
    PeerClose(oneshot::Sender<()>),
}
pub(super) fn start<S, E>(socket: S, limits: Limits, metrics: Option<Metrics>) -> Connection
where
    S: Stream<Item = Result<Message, E>> + Sink<Message, Error = E> + Unpin + Send + 'static,
    E: Into<Error> + Send + 'static,
{
    let (sender, mut queues) = Queues::new(&limits, metrics.clone());
    let (incoming_tx, incoming) = mpsc::channel(limits.incoming_messages);
    let budget = Arc::new(Semaphore::new(limits.incoming_bytes));
    let (close, mut closing) = oneshot::channel();
    let (internal, mut commands) = mpsc::channel(16);
    let task = tokio::spawn(async move {
        if let Some(hook) = &metrics {
            hook(TransportEvent::Connected);
        }
        let (mut sink, mut stream) = socket.split();
        let wanted = AtomicU64::new(0);
        let acknowledged = AtomicU64::new(0);
        let reader = async {
            while let Some(message) = stream.next().await {
                let message = message.map_err(classify)?;
                let size = message.len();
                let frame = match message {
                    Message::Text(text) => {
                        let envelope: Envelope = decode_json(text.as_bytes())
                            .map_err(|error| Error::Protocol(error.code))?;
                        envelope
                            .validate()
                            .map_err(|error| Error::Protocol(error.code))?;
                        Some(Frame::Text(Box::new(envelope)))
                    }
                    Message::Binary(bytes) => Some(Frame::Binary(
                        PtyFrame::decode(&bytes).map_err(|error| Error::Protocol(error.code))?,
                    )),
                    Message::Ping(bytes) => {
                        internal
                            .try_send(Internal::Message(Message::Pong(bytes)))
                            .map_err(|_| Error::SlowConsumer)?;
                        None
                    }
                    Message::Pong(bytes) => {
                        if let Ok(bytes) = <&[u8; 8]>::try_from(bytes.as_ref()) {
                            let nonce = u64::from_be_bytes(*bytes);
                            if nonce == wanted.load(Ordering::Acquire) {
                                acknowledged.store(nonce, Ordering::Release);
                            }
                        }
                        None
                    }
                    Message::Close(reason) => {
                        let code = reason.map_or(1005, |reason| reason.code.into());
                        let (done, wait) = oneshot::channel();
                        internal
                            .try_send(Internal::PeerClose(done))
                            .map_err(|_| Error::SlowConsumer)?;
                        let _ = timeout(limits.write_timeout, wait).await;
                        return Ok(CloseReason { code, remote: true });
                    }
                    Message::Frame(_) => return Err(Error::Protocol(ErrorCode::InvalidFrame)),
                };
                if let Some(frame) = frame {
                    let permit = budget
                        .clone()
                        .try_acquire_many_owned(size as u32)
                        .map_err(|_| Error::SlowConsumer)?;
                    let shared = limits
                        .shared_incoming
                        .as_ref()
                        .map(|budget| {
                            budget
                                .clone()
                                .try_acquire_many_owned(size as u32)
                                .map_err(|_| Error::SlowConsumer)
                        })
                        .transpose()?;
                    incoming_tx
                        .try_send(ReceivedFrame {
                            frame,
                            _bytes: permit,
                            _shared: shared,
                        })
                        .map_err(|_| Error::SlowConsumer)?;
                    if let Some(hook) = &metrics {
                        hook(TransportEvent::BytesIn(size));
                    }
                }
            }
            Ok(CloseReason {
                code: 1006,
                remote: true,
            })
        };
        let writer = async {
            loop {
                tokio::select! {
                    biased;
                    _=&mut closing=> {
                        timeout(limits.write_timeout,sink.send(Message::Close(Some(CloseFrame { code:CloseCode::Normal,reason:"".into() })))).await.map_err(|_| Error::WriteTimeout)?.map_err(classify)?;
                        return Ok(CloseReason { code:1000,remote:false });
                    },
                    Some(command)=commands.recv()=>match command {
                        Internal::Message(message)=>timeout(limits.write_timeout,sink.send(message)).await.map_err(|_| Error::WriteTimeout)?.map_err(classify)?,
                        Internal::PeerClose(done)=> {
                            // Tungstenite queued the RFC-required close reply when reading Close.
                            let _=timeout(limits.write_timeout,sink.flush()).await;
                            let _=done.send(());
                        },
                    },
                    item=queues.next()=> {
                        let Some(item)=item else { return Ok(CloseReason { code:1000,remote:false }); };
                        let size=item.message.len();
                        let result=timeout(limits.write_timeout,sink.send(item.message)).await.map_err(|_| Error::WriteTimeout).and_then(|result| result.map_err(classify));
                        let _=item.done.send(result.as_ref().map(|_| ()).map_err(|_| RemoteError::unknown("connection write failed; transmission outcome unknown")));
                        result?;
                        if let Some(hook)=&metrics { hook(TransportEvent::BytesOut(size)); }
                    },
                }
            }
        };
        let heartbeat = async {
            let mut nonce = 0u64;
            let mut ticks = tokio::time::interval_at(
                tokio::time::Instant::now() + limits.heartbeat_interval,
                limits.heartbeat_interval,
            );
            ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticks.tick().await;
                nonce = nonce.checked_add(1).ok_or(Error::HeartbeatTimeout)?;
                wanted.store(nonce, Ordering::Release);
                internal
                    .try_send(Internal::Message(Message::Ping(
                        nonce.to_be_bytes().to_vec().into(),
                    )))
                    .map_err(|_| Error::SlowConsumer)?;
                sleep(limits.pong_timeout).await;
                if acknowledged.load(Ordering::Acquire) != nonce {
                    if let Some(hook) = &metrics {
                        hook(TransportEvent::HeartbeatTimeout);
                    }
                    return Err(Error::HeartbeatTimeout);
                }
            }
        };
        let result = tokio::select! { result=reader=>result, result=writer=>result, result=heartbeat=>result };
        if let Some(hook) = &metrics {
            hook(TransportEvent::Closed);
        }
        result
    });
    Connection {
        sender,
        incoming,
        close: Some(close),
        task: Some(task),
        result: None,
    }
}
fn classify<E: Into<Error>>(error: E) -> Error {
    error.into()
}
impl From<tungstenite::Error> for Error {
    fn from(error: tungstenite::Error) -> Self {
        ws_error(error)
    }
}
pub(super) fn ws_error(error: tungstenite::Error) -> Error {
    match error {
        tungstenite::Error::Http(response) => Error::UpgradeRejected(response.status().as_u16()),
        tungstenite::Error::Capacity(_) => Error::Protocol(ErrorCode::FrameTooLarge),
        tungstenite::Error::Io(_) => Error::Network,
        _ => Error::Protocol(ErrorCode::InvalidFrame),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qgenie_remote_proto::{Counter, OperationOutcome, PtyPayload, SessionId};
    use tokio_tungstenite::WebSocketStream;
    #[tokio::test]
    async fn stalled_writer_has_a_deadline_and_reports_unknown() {
        let (stream, _unread_peer) = tokio::io::duplex(64);
        let socket = WebSocketStream::from_raw_socket(
            stream,
            tungstenite::protocol::Role::Server,
            Some(config()),
        )
        .await;
        let limits = Limits {
            write_timeout: std::time::Duration::from_millis(30),
            ..Limits::default()
        };
        let mut connection = start(socket, limits, None);
        let receipt = connection
            .sender()
            .try_send(
                super::super::Priority::Bulk,
                Frame::Binary(PtyFrame {
                    session: SessionId::new(),
                    seq: Counter(1),
                    payload: PtyPayload::Output(vec![1; 32768]),
                }),
            )
            .unwrap();
        assert!(matches!(
            receipt.transmitted().await.unwrap_err().outcome,
            OperationOutcome::Unknown
        ));
        assert!(matches!(
            connection.closed().await,
            Err(Error::WriteTimeout)
        ));
    }
    #[tokio::test]
    async fn independent_cancellation_stops_a_stalled_writer_and_discards_queues() {
        let (stream, _unread_peer) = tokio::io::duplex(64);
        let socket = WebSocketStream::from_raw_socket(
            stream,
            tungstenite::protocol::Role::Server,
            Some(config()),
        )
        .await;
        let mut connection = start(socket, Limits::default(), None);
        let receipt = connection
            .sender()
            .try_send(
                super::super::Priority::Bulk,
                Frame::Binary(PtyFrame {
                    session: SessionId::new(),
                    seq: Counter(1),
                    payload: PtyPayload::Output(vec![1; 32768]),
                }),
            )
            .unwrap();
        tokio::task::yield_now().await;
        connection.cancellation().cancel();
        assert!(matches!(
            tokio::time::timeout(std::time::Duration::from_millis(100), connection.closed())
                .await
                .unwrap(),
            Err(Error::Task)
        ));
        assert!(matches!(
            receipt.transmitted().await.unwrap_err().outcome,
            OperationOutcome::Unknown
        ));
        assert!(connection.recv().await.is_none());
    }
}
