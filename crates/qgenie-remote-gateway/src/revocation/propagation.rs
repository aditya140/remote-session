//! Completion uses expiry of every pre-observation authority window, never UTC
//! age, Redis delivery or process death. Restart/eviction conservatively wait again.
use super::*;
use std::{collections::BTreeMap, sync::Mutex, time::Duration};

const CAP: usize = 4096;
const HORIZON: Duration = Duration::from_millis(qgenie_remote_proto::MAX_ACCESS_LIFETIME_MS as u64);
struct Observation {
    revision: Counter,
    committed_at: Timestamp,
    target: RevocationTarget,
    at: Duration,
}
#[derive(Default)]
struct State {
    entries: BTreeMap<(UserId, RequestId), Observation>,
    last: Option<Duration>,
    failed: bool,
}
#[derive(Default)]
pub(crate) struct Propagation(Mutex<State>);
impl Propagation {
    /// The caller must have just committed or read this exact primary row.
    pub(crate) fn observe(
        &self,
        actor: UserId,
        result: &mut AccessRevocation,
    ) -> Result<(), Error> {
        let mut state = self.0.lock().map_err(|_| Error::Unavailable)?;
        // Sample while serialized: a later caller must not acquire this lock
        // first and turn an earlier healthy sample into a false regression.
        let at = match qgenie_remote_transport::elapsed::now() {
            Ok(at) => at,
            Err(_) => {
                state.failed = true;
                return Err(Error::Unavailable);
            }
        };
        state.observe(actor, result, at)
    }
    #[cfg(test)]
    fn observe_at(
        &self,
        actor: UserId,
        result: &mut AccessRevocation,
        at: Duration,
    ) -> Result<(), Error> {
        self.0
            .lock()
            .map_err(|_| Error::Unavailable)?
            .observe(actor, result, at)
    }
}
impl State {
    fn observe(
        &mut self,
        actor: UserId,
        result: &mut AccessRevocation,
        at: Duration,
    ) -> Result<(), Error> {
        if self.failed || self.last.is_some_and(|last| at < last) {
            self.failed = true;
            return Err(Error::Unavailable);
        }
        self.last = Some(at);
        let key = (actor, result.request);
        if self.entries.get(&key).is_some_and(|old| {
            old.revision != result.revision
                || old.committed_at != result.committed_at
                || old.target != result.target
        }) {
            self.entries.remove(&key);
        }
        if !self.entries.contains_key(&key) && self.entries.len() == CAP {
            let oldest = self
                .entries
                .iter()
                .min_by_key(|(_, observation)| observation.at)
                .map(|(key, _)| *key)
                .ok_or(Error::Unavailable)?;
            self.entries.remove(&oldest);
        }
        let observed = self.entries.entry(key).or_insert_with(|| Observation {
            revision: result.revision,
            committed_at: result.committed_at.clone(),
            target: result.target.clone(),
            at,
        });
        result.propagation = if at.checked_sub(observed.at).ok_or(Error::Unavailable)? >= HORIZON {
            RevocationPropagation::Completed
        } else {
            RevocationPropagation::Pending
        };
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn result() -> AccessRevocation {
        AccessRevocation {
            request: RequestId::new(),
            target: RevocationTarget::Global,
            revision: Counter(1),
            // Even an arbitrarily old UTC commit is not completion evidence.
            committed_at: Timestamp::from_unix_millis(0).unwrap(),
            propagation: RevocationPropagation::Pending,
        }
    }
    #[test]
    fn original_observation_survives_polling_and_restart_waits_again() {
        let progress = Propagation::default();
        let actor = UserId::new();
        let mut value = result();
        for seconds in [100, 105, 109] {
            progress
                .observe_at(actor, &mut value, Duration::from_secs(seconds))
                .unwrap();
            assert!(matches!(value.propagation, RevocationPropagation::Pending));
        }
        progress
            .observe_at(actor, &mut value, Duration::from_secs(110))
            .unwrap();
        assert!(matches!(
            value.propagation,
            RevocationPropagation::Completed
        ));
        // A large suspend-aware elapsed jump also expires the original horizon.
        progress
            .observe_at(actor, &mut value, Duration::from_secs(10_000))
            .unwrap();
        assert!(matches!(
            value.propagation,
            RevocationPropagation::Completed
        ));
        Propagation::default()
            .observe_at(actor, &mut value, Duration::from_secs(20_000))
            .unwrap();
        assert!(matches!(value.propagation, RevocationPropagation::Pending));
    }
    #[test]
    fn identity_changes_and_eviction_cannot_reuse_an_old_completion() {
        let progress = Propagation::default();
        let actor = UserId::new();
        let mut value = result();
        progress
            .observe_at(actor, &mut value, Duration::ZERO)
            .unwrap();
        progress.observe_at(actor, &mut value, HORIZON).unwrap();
        assert!(matches!(
            value.propagation,
            RevocationPropagation::Completed
        ));
        value.revision = Counter(2);
        progress.observe_at(actor, &mut value, HORIZON).unwrap();
        assert!(matches!(value.propagation, RevocationPropagation::Pending));
        for _ in 0..CAP {
            progress
                .observe_at(actor, &mut result(), HORIZON + Duration::from_secs(1))
                .unwrap();
        }
        assert_eq!(progress.0.lock().unwrap().entries.len(), CAP);
        progress.observe_at(actor, &mut value, HORIZON * 3).unwrap();
        assert!(matches!(value.propagation, RevocationPropagation::Pending));
    }
    #[test]
    fn elapsed_regression_permanently_prevents_completion() {
        let progress = Propagation::default();
        let actor = UserId::new();
        let mut value = result();
        progress.observe_at(actor, &mut value, HORIZON).unwrap();
        assert!(
            progress
                .observe_at(actor, &mut value, Duration::ZERO)
                .is_err()
        );
        assert!(progress.observe_at(actor, &mut value, HORIZON * 3).is_err());
    }
    #[test]
    fn concurrent_status_observations_sample_time_in_lock_order() {
        let progress = Propagation::default();
        let actor = UserId::new();
        let value = result();
        let barrier = std::sync::Barrier::new(32);
        std::thread::scope(|threads| {
            for _ in 0..32 {
                let mut value = value.clone();
                let (progress, barrier) = (&progress, &barrier);
                threads.spawn(move || {
                    barrier.wait();
                    progress.observe(actor, &mut value).unwrap();
                    assert!(matches!(value.propagation, RevocationPropagation::Pending));
                });
            }
        });
        let state = progress.0.lock().unwrap();
        assert!(!state.failed);
        assert_eq!(state.entries.len(), 1);
    }
}
