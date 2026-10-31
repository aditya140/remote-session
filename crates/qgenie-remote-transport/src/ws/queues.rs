use super::{Frame, Limits, Metrics, Priority, SharedBudget, TransportEvent};
use qgenie_remote_proto::{ErrorCode, OperationOutcome, RemoteError};
use std::sync::Arc;
use tokio::sync::{OwnedSemaphorePermit, Semaphore, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;

pub(super) struct Queued {
    pub message: Message,
    pub done: oneshot::Sender<Result<(), RemoteError>>,
    pub _bytes: OwnedSemaphorePermit,
    pub _shared: Option<OwnedSemaphorePermit>,
}
#[derive(Clone)]
struct Lane {
    sender: mpsc::Sender<Queued>,
    bytes: Arc<Semaphore>,
}
/// Cloneable bounded submission handle. A full queue rejects the operation as not sent.
#[derive(Clone)]
pub struct Sender {
    lanes: [Lane; 3],
    metrics: Option<Metrics>,
    shared: Option<SharedBudget>,
}
/// A successful write means transmitted, not authorized, committed, or applied.
pub struct Transmission(oneshot::Receiver<Result<(), RemoteError>>);
impl Transmission {
    pub async fn transmitted(self) -> Result<(), RemoteError> {
        self.0.await.unwrap_or_else(|_| {
            Err(RemoteError::unknown(
                "connection closed; transmission outcome unknown",
            ))
        })
    }
}
impl Sender {
    /// True after the driver drops its outbound queues. Unlike a full queue,
    /// this condition cannot recover on the same connection.
    pub fn is_closed(&self) -> bool {
        self.lanes[0].sender.is_closed()
    }
    pub fn try_send(&self, priority: Priority, frame: Frame) -> Result<Transmission, RemoteError> {
        let message = frame.encode()?;
        let lane = &self.lanes[index(priority)];
        let permit = lane
            .bytes
            .clone()
            .try_acquire_many_owned(message.len() as u32)
            .map_err(|_| self.full(priority))?;
        let shared = self
            .shared
            .as_ref()
            .map(|budget| {
                budget.lanes[index(priority)]
                    .clone()
                    .try_acquire_many_owned(message.len() as u32)
                    .map_err(|_| self.full(priority))
            })
            .transpose()?;
        let (done, receiver) = oneshot::channel();
        lane.sender
            .try_send(Queued {
                message,
                done,
                _bytes: permit,
                _shared: shared,
            })
            .map_err(|_| self.full(priority))?;
        Ok(Transmission(receiver))
    }
    fn full(&self, priority: Priority) -> RemoteError {
        if let Some(metrics) = &self.metrics {
            metrics(TransportEvent::QueueFull(priority));
        }
        RemoteError {
            code: ErrorCode::Overloaded,
            outcome: OperationOutcome::NotSent,
            message: "connection queue unavailable; operation not sent".into(),
            retryable: false,
        }
    }
}
fn index(priority: Priority) -> usize {
    match priority {
        Priority::Control => 0,
        Priority::Event => 1,
        Priority::Bulk => 2,
    }
}

pub(super) struct Queues {
    lanes: [mpsc::Receiver<Queued>; 3],
    cursor: usize,
}
const SCHEDULE: [usize; 13] = [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2];
impl Queues {
    pub fn new(limits: &Limits, metrics: Option<Metrics>) -> (Sender, Self) {
        let mut senders = Vec::new();
        let mut receivers = Vec::new();
        for (count, bytes) in [
            (limits.control_messages, limits.control_bytes),
            (limits.event_messages, limits.event_bytes),
            (limits.bulk_messages, limits.bulk_bytes),
        ] {
            let (sender, receiver) = mpsc::channel(count);
            senders.push(Lane {
                sender,
                bytes: Arc::new(Semaphore::new(bytes)),
            });
            receivers.push(receiver);
        }
        (
            Sender {
                lanes: senders.try_into().ok().unwrap(),
                metrics,
                shared: limits.shared_outgoing.clone(),
            },
            Self {
                lanes: receivers.try_into().ok().unwrap(),
                cursor: 0,
            },
        )
    }
    pub async fn next(&mut self) -> Option<Queued> {
        for _ in 0..SCHEDULE.len() {
            let lane = SCHEDULE[self.cursor];
            self.cursor = (self.cursor + 1) % SCHEDULE.len();
            if let Ok(item) = self.lanes[lane].try_recv() {
                return Some(item);
            }
        }
        let [control, event, bulk] = &mut self.lanes;
        tokio::select! {
            item = control.recv() => item,
            item = event.recv() => item,
            item = bulk.recv() => item,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use qgenie_remote_proto::{Counter, PtyFrame, PtyPayload, SessionId};
    fn frame(byte: u8) -> Frame {
        Frame::Binary(PtyFrame {
            session: SessionId::new(),
            seq: Counter(1),
            payload: PtyPayload::Output(vec![byte]),
        })
    }
    #[tokio::test]
    async fn service_budgets_bound_connections_without_borrowing_control_bytes() {
        let limits = Limits {
            shared_outgoing: Some(SharedBudget::new(58, 58, 29).unwrap()),
            ..Limits::default()
        };
        let (first, queue1) = Queues::new(&limits, None);
        let (second, mut queue2) = Queues::new(&limits, None);
        let _first = first.try_send(Priority::Bulk, frame(1)).unwrap();
        assert!(second.try_send(Priority::Bulk, frame(2)).is_err());
        let control = second.try_send(Priority::Control, frame(0)).unwrap();
        let item = queue2.next().await.unwrap();
        item.done.send(Ok(())).unwrap();
        control.transmitted().await.unwrap();
        drop(queue1);
        let bulk = second.try_send(Priority::Bulk, frame(2)).unwrap();
        let item = queue2.next().await.unwrap();
        item.done.send(Ok(())).unwrap();
        bulk.transmitted().await.unwrap();
    }
    #[tokio::test]
    async fn bounded_lanes_keep_control_capacity_and_weighted_fairness() {
        let limits = Limits {
            control_messages: 16,
            event_messages: 16,
            bulk_messages: 1,
            bulk_bytes: 29,
            ..Limits::default()
        };
        let (sender, mut queues) = Queues::new(&limits, None);
        let receipt = sender.try_send(Priority::Bulk, frame(2)).unwrap();
        let error = sender.try_send(Priority::Bulk, frame(3)).err().unwrap();
        assert!(matches!(error.outcome, OperationOutcome::NotSent));
        for _ in 0..8 {
            let _receipt = sender.try_send(Priority::Control, frame(0)).unwrap();
        }
        for _ in 0..4 {
            let _receipt = sender.try_send(Priority::Event, frame(1)).unwrap();
        }
        for expected in [0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2] {
            let queued = queues.next().await.unwrap();
            assert_eq!(queued.message.into_data().last(), Some(&expected));
            let _ = queued.done.send(Ok(()));
        }
        receipt.transmitted().await.unwrap();
        let unknown = sender.try_send(Priority::Bulk, frame(2)).unwrap();
        drop(queues);
        assert!(matches!(
            unknown.transmitted().await.unwrap_err().outcome,
            OperationOutcome::Unknown
        ));
    }
}
