//! Fixed-cardinality, nonblocking observations; no content or principal labels.
use qgenie_remote_proto::wire::Message;
use qgenie_remote_transport::ws::{Metrics, Priority, TransportEvent};
use std::{
    fmt::Write,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

const BUCKETS: [(u64, &str); 7] = [
    (10_000, "0.01"),
    (50_000, "0.05"),
    (100_000, "0.1"),
    (500_000, "0.5"),
    (1_000_000, "1"),
    (5_000_000, "5"),
    (30_000_000, "30"),
];
#[derive(Default)]
pub(super) struct Observations {
    connected: AtomicU64,
    closed: AtomicU64,
    bytes_in: AtomicU64,
    bytes_out: AtomicU64,
    heartbeat_timeout: AtomicU64,
    queue_full: [AtomicU64; 3],
    requests: AtomicU64,
    replays: AtomicU64,
    approval_choices: AtomicU64,
    delivery: Mutex<Delivery>,
}
#[derive(Default)]
struct Delivery {
    count: u64,
    micros: u64,
    buckets: [u64; 7],
}
impl Observations {
    pub(super) fn hook(self: &Arc<Self>) -> Metrics {
        let observations = self.clone();
        Arc::new(move |event| {
            let (counter, amount) = match event {
                TransportEvent::Connected => (&observations.connected, 1),
                TransportEvent::Closed => (&observations.closed, 1),
                TransportEvent::BytesIn(bytes) => (&observations.bytes_in, bytes as u64),
                TransportEvent::BytesOut(bytes) => (&observations.bytes_out, bytes as u64),
                TransportEvent::HeartbeatTimeout => (&observations.heartbeat_timeout, 1),
                TransportEvent::QueueFull(priority) => (
                    &observations.queue_full[match priority {
                        Priority::Control => 0,
                        Priority::Event => 1,
                        Priority::Bulk => 2,
                    }],
                    1,
                ),
            };
            counter.fetch_add(amount, Ordering::Relaxed);
        })
    }
    pub(super) fn request(&self, message: &Message) {
        self.requests.fetch_add(1, Ordering::Relaxed);
        if matches!(
            message,
            Message::Attach { .. } | Message::Capture | Message::ReplayRequest { .. }
        ) {
            self.replays.fetch_add(1, Ordering::Relaxed);
        }
        if matches!(message, Message::ApprovalResolve { .. }) {
            self.approval_choices.fetch_add(1, Ordering::Relaxed);
        }
    }
    pub(super) fn delivery(&self, elapsed: Duration) {
        let micros = elapsed.as_micros().min(u64::MAX as u128) as u64;
        // One short snapshot lock keeps cumulative histogram buckets coherent.
        // The transport callback above only touches atomics and never takes it.
        let mut delivery = self.delivery.lock().unwrap();
        delivery.count = delivery.count.saturating_add(1);
        delivery.micros = delivery.micros.saturating_add(micros);
        for (index, (bound, _)) in BUCKETS.iter().enumerate() {
            if micros <= *bound {
                delivery.buckets[index] = delivery.buckets[index].saturating_add(1);
            }
        }
    }
    pub(super) fn text(&self) -> String {
        let mut text = String::new();
        for (name, counter) in [
            ("qgr_transport_connections_total", &self.connected),
            ("qgr_transport_closed_total", &self.closed),
            ("qgr_transport_bytes_in_total", &self.bytes_in),
            ("qgr_transport_bytes_out_total", &self.bytes_out),
            (
                "qgr_transport_heartbeat_timeouts_total",
                &self.heartbeat_timeout,
            ),
            ("qgr_client_requests_total", &self.requests),
            ("qgr_replay_requests_total", &self.replays),
            ("qgr_approval_choice_attempts_total", &self.approval_choices),
        ] {
            writeln!(
                text,
                "# TYPE {name} counter\n{name} {}",
                counter.load(Ordering::Relaxed)
            )
            .unwrap();
        }
        text.push_str("# TYPE qgr_transport_queue_full_total counter\n");
        for (index, priority) in ["control", "event", "bulk"].into_iter().enumerate() {
            writeln!(
                text,
                "qgr_transport_queue_full_total{{priority=\"{priority}\"}} {}",
                self.queue_full[index].load(Ordering::Relaxed)
            )
            .unwrap();
        }
        text.push_str("# TYPE qgr_native_approval_delivery_attempt_seconds histogram\n");
        let delivery = self.delivery.lock().unwrap();
        for (index, (_, label)) in BUCKETS.iter().enumerate() {
            writeln!(
                text,
                "qgr_native_approval_delivery_attempt_seconds_bucket{{le=\"{label}\"}} {}",
                delivery.buckets[index]
            )
            .unwrap();
        }
        let count = delivery.count;
        writeln!(text,"qgr_native_approval_delivery_attempt_seconds_bucket{{le=\"+Inf\"}} {count}\nqgr_native_approval_delivery_attempt_seconds_count {count}\nqgr_native_approval_delivery_attempt_seconds_sum {}", delivery.micros as f64 / 1_000_000.0).unwrap();
        text
    }
}
