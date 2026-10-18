//! The framework socket checks this deadline on every I/O poll, including after
//! suspension, before an old queued frame can leave the gateway.
use crate::Error;
use std::{
    sync::Mutex,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Clone, Copy)]
struct Window {
    started: Duration,
    until: Duration,
}
struct State {
    denied: bool,
    credential: Window,
    expires_at: u64,
    authority: Option<Window>,
}
pub(crate) struct Gate(Mutex<State>);
fn credential(expires: u64) -> Result<Window, Error> {
    let started = super::elapsed()?;
    let wall = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| Error::Unavailable)?;
    let left = Duration::from_secs(expires)
        .checked_sub(wall)
        .filter(|left| !left.is_zero())
        .ok_or(Error::Unauthenticated)?;
    Ok(Window {
        started,
        until: started.checked_add(left).ok_or(Error::Unavailable)?,
    })
}
impl Gate {
    pub fn new(expires: u64) -> Result<Self, Error> {
        Ok(Self(Mutex::new(State {
            denied: false,
            credential: credential(expires)?,
            expires_at: expires,
            authority: None,
        })))
    }
    pub fn refresh_credential(&self, expires: u64) -> Result<(), Error> {
        let mut fresh = credential(expires)?;
        let mut state = self.0.lock().unwrap();
        if state.denied {
            return Err(Error::Forbidden);
        }
        if state.expires_at == expires {
            fresh.until = fresh.until.min(state.credential.until);
        }
        state.credential = fresh;
        state.expires_at = expires;
        Ok(())
    }
    pub fn renew(&self, started: Duration, directory_until: Option<Duration>) -> Result<(), Error> {
        let now = super::elapsed()?;
        let mut state = self.0.lock().unwrap();
        if state.denied || now < started || now < state.credential.started {
            return Err(Error::Forbidden);
        }
        let mut until = (started
            + Duration::from_millis(qgenie_remote_proto::MAX_ACCESS_LIFETIME_MS as u64))
        .min(state.credential.until);
        if let Some(deadline) = directory_until {
            until = until.min(deadline);
        }
        if until <= now {
            return Err(Error::Unavailable);
        }
        state.authority = Some(Window { started, until });
        Ok(())
    }
    pub fn deny(&self) {
        self.0.lock().unwrap().denied = true;
    }
    pub fn budget(&self) -> Result<(u32, Duration), Error> {
        let now = super::elapsed()?;
        self.assert_at(now)?;
        let state = self.0.lock().unwrap();
        if state.denied {
            return Err(Error::Forbidden);
        }
        let until = state
            .authority
            .ok_or(Error::Unavailable)?
            .until
            .min(state.credential.until);
        let ms: u32 = until
            .checked_sub(now)
            .ok_or(Error::Unavailable)?
            .as_millis()
            .try_into()
            .map_err(|_| Error::Unavailable)?;
        if ms == 0 || ms > qgenie_remote_proto::MAX_ACCESS_LIFETIME_MS {
            return Err(Error::Unavailable);
        }
        Ok((ms, until))
    }
    pub fn assert(&self) -> Result<(), Error> {
        self.assert_at(super::elapsed()?)
    }
    fn assert_at(&self, now: Duration) -> Result<(), Error> {
        let state = self.0.lock().unwrap();
        let window = state.authority.ok_or(Error::Unavailable)?;
        if state.denied
            || now < window.started
            || now < state.credential.started
            || now >= window.until
            || now >= state.credential.until
        {
            return Err(Error::Unavailable);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_io_cannot_outlive_original_evidence_or_revive_after_deny() {
        let gate = Gate(Mutex::new(State {
            denied: false,
            credential: Window {
                started: Duration::from_secs(100),
                until: Duration::from_secs(1000),
            },
            expires_at: 0,
            authority: Some(Window {
                started: Duration::from_secs(100),
                until: Duration::from_secs(110),
            }),
        }));
        assert!(gate.assert_at(Duration::from_secs(109)).is_ok());
        assert!(gate.assert_at(Duration::from_secs(110)).is_err());
        // Simulated elapsed suspension while awake-time timers remain pending.
        assert!(gate.assert_at(Duration::from_secs(950)).is_err());
        assert!(gate.assert_at(Duration::from_secs(99)).is_err());
        gate.deny();
        assert!(gate.renew(super::super::elapsed().unwrap(), None).is_err());
    }
}
