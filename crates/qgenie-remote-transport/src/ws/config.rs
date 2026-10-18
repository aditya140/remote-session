use super::Error;
use std::{sync::Arc, time::Duration};
use tokio::sync::Semaphore;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    Control,
    Event,
    Bulk,
}

/// Optional service-wide lane budgets. Bulk traffic cannot borrow control capacity.
#[derive(Debug, Clone)]
pub struct SharedBudget {
    pub(super) lanes: [Arc<Semaphore>; 3],
}
impl SharedBudget {
    pub fn new(control: usize, event: usize, bulk: usize) -> Result<Self, Error> {
        if [control, event, bulk]
            .iter()
            .any(|bytes| *bytes == 0 || *bytes > 512 * 1024 * 1024)
        {
            return Err(Error::Configuration);
        }
        Ok(Self {
            lanes: [control, event, bulk].map(|bytes| Arc::new(Semaphore::new(bytes))),
        })
    }
}
#[derive(Debug, Clone)]
pub struct Limits {
    pub control_messages: usize,
    pub control_bytes: usize,
    pub event_messages: usize,
    pub event_bytes: usize,
    pub bulk_messages: usize,
    pub bulk_bytes: usize,
    pub incoming_messages: usize,
    pub incoming_bytes: usize,
    pub heartbeat_interval: Duration,
    pub pong_timeout: Duration,
    pub write_timeout: Duration,
    pub shared_outgoing: Option<SharedBudget>,
    pub shared_incoming: Option<Arc<Semaphore>>,
}
impl Default for Limits {
    fn default() -> Self {
        Self {
            control_messages: 128,
            control_bytes: 1024 * 1024,
            event_messages: 512,
            event_bytes: 2 * 1024 * 1024,
            bulk_messages: 256,
            bulk_bytes: 8 * 1024 * 1024,
            incoming_messages: 512,
            incoming_bytes: 2 * 1024 * 1024,
            heartbeat_interval: Duration::from_secs(20),
            pong_timeout: Duration::from_secs(5),
            write_timeout: Duration::from_secs(5),
            shared_outgoing: None,
            shared_incoming: None,
        }
    }
}
impl Limits {
    pub fn validate(&self) -> Result<(), Error> {
        for (count, bytes) in [
            (self.control_messages, self.control_bytes),
            (self.event_messages, self.event_bytes),
            (self.bulk_messages, self.bulk_bytes),
            (self.incoming_messages, self.incoming_bytes),
        ] {
            if count == 0 || count > 4096 || bytes == 0 || bytes > 64 * 1024 * 1024 {
                return Err(Error::Configuration);
            }
        }
        if self.heartbeat_interval.is_zero()
            || self.heartbeat_interval > Duration::from_secs(25)
            || self.pong_timeout.is_zero()
            || self.pong_timeout > Duration::from_secs(10)
            || self.write_timeout.is_zero()
            || self.write_timeout > Duration::from_secs(10)
        {
            return Err(Error::Configuration);
        }
        Ok(())
    }
}

/// Jitter only. Calling this never reconnects or repeats an operation.
#[derive(Debug, Clone)]
pub struct Backoff {
    minimum: Duration,
    maximum: Duration,
    attempt: u32,
}
impl Backoff {
    pub fn new(minimum: Duration, maximum: Duration) -> Result<Self, Error> {
        if minimum.is_zero() || maximum < minimum || maximum > Duration::from_secs(300) {
            return Err(Error::Configuration);
        }
        Ok(Self {
            minimum,
            maximum,
            attempt: 0,
        })
    }
    pub fn next_delay(&mut self, entropy: u64) -> Duration {
        let ceiling = self
            .minimum
            .saturating_mul(1u32.checked_shl(self.attempt.min(31)).unwrap_or(u32::MAX))
            .min(self.maximum);
        self.attempt = self.attempt.saturating_add(1);
        let span = ceiling.as_millis().max(1) as u64;
        Duration::from_millis(1 + entropy % span)
    }
    pub fn reset(&mut self) {
        self.attempt = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jitter_is_bounded_and_heartbeat_cannot_exceed_proxy_budget() {
        let mut backoff =
            Backoff::new(Duration::from_millis(100), Duration::from_secs(30)).unwrap();
        for entropy in 0..1000 {
            assert!(backoff.next_delay(entropy) <= Duration::from_secs(30));
        }
        backoff.reset();
        assert!(backoff.next_delay(u64::MAX) <= Duration::from_millis(100));
        let limits = Limits {
            heartbeat_interval: Duration::from_secs(26),
            ..Limits::default()
        };
        assert!(limits.validate().is_err());
    }
}
