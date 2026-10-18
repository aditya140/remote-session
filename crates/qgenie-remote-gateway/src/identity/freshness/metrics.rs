//! Aggregate only: observing freshness never refreshes or extends evidence.
use super::*;

#[derive(Default)]
pub(crate) struct DirectoryMetrics {
    pub active: usize,
    pub stale: usize,
    pub max_age_seconds: f64,
}
impl DirectoryMetrics {
    fn record(&mut self, status: &Status, at: Duration) -> Result<(), Error> {
        if at.checked_sub(status.used).ok_or(Error::Unavailable)? >= ACTIVE {
            return Ok(());
        }
        self.active += 1;
        if let Some(proof) = status.proof {
            let age = at
                .checked_sub(proof.started_elapsed)
                .ok_or(Error::Unavailable)?;
            self.max_age_seconds = self.max_age_seconds.max(age.as_secs_f64());
            self.stale += usize::from(age >= FRESH);
        } else {
            self.stale += 1;
        }
        Ok(())
    }
}
impl DirectoryFreshness {
    pub(crate) fn metrics(&self) -> Result<DirectoryMetrics, Error> {
        let at = now()?;
        let entries = self
            .inner
            .users
            .lock()
            .map_err(|_| Error::Unavailable)?
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut sample = DirectoryMetrics::default();
        for entry in entries {
            let status = entry.status.lock().map_err(|_| Error::Unavailable)?;
            sample.record(&status, at)?;
        }
        Ok(sample)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn freshness_metrics_preserve_original_deadlines_and_exclude_inactive_entries() {
        let mut status = Status {
            used: Duration::from_secs(900),
            proof: Some(DirectoryObservation {
                enabled: true,
                started_elapsed: Duration::from_secs(100),
            }),
            ..Status::default()
        };
        let mut sample = DirectoryMetrics::default();
        sample.record(&status, Duration::from_secs(939)).unwrap();
        assert_eq!((sample.active, sample.stale), (1, 0));
        assert_eq!(sample.max_age_seconds, 839.0);
        sample.record(&status, Duration::from_secs(940)).unwrap();
        assert_eq!((sample.active, sample.stale), (2, 1));
        assert!(status.evidence(Duration::from_secs(940)).is_err());
        status.proof = None;
        sample.record(&status, Duration::from_secs(941)).unwrap();
        assert_eq!((sample.active, sample.stale), (3, 2));
        sample.record(&status, Duration::from_secs(1800)).unwrap();
        assert_eq!((sample.active, sample.stale), (3, 2));
        assert!(sample.record(&status, Duration::from_secs(899)).is_err());
    }
}
