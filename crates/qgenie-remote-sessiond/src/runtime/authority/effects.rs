//! A scope can fence native operations without holding the authority mutex over I/O.
//! Invalidation stops new operations first, then waits for an already admitted OS call
//! before the replacement floor/ACK. Store locks must never span this barrier wait.
use super::*;
use std::sync::{RwLock, RwLockWriteGuard};
pub(super) struct Effects {
    alive: AtomicBool,
    barrier: RwLock<()>,
}
impl Effects {
    pub fn new() -> Self {
        Self {
            alive: AtomicBool::new(true),
            barrier: RwLock::new(()),
        }
    }
    pub fn alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }
    pub fn stop(&self) {
        self.alive.store(false, Ordering::Release);
    }
    pub fn invalidate(&self) -> Result<RwLockWriteGuard<'_, ()>, RemoteError> {
        self.stop();
        self.barrier.write().map_err(|_| expired())
    }
}
#[derive(Clone)]
pub(in super::super) struct Fence {
    effects: Arc<Effects>,
    issued_elapsed: Duration,
    deadline: Instant,
    elapsed_deadline: Duration,
}
impl Fence {
    pub(in super::super) fn check(&self) -> Result<Instant, RemoteError> {
        let elapsed = clock::elapsed()?;
        if !self.effects.alive() {
            return Err(stale());
        }
        if Instant::now() >= self.deadline
            || elapsed < self.issued_elapsed
            || elapsed >= self.elapsed_deadline
        {
            return Err(expired());
        }
        Ok(self
            .deadline
            .min(Instant::now() + (self.elapsed_deadline - elapsed)))
    }
    /// Only the native OS effect belongs here. Release the guard before any store work.
    pub(in super::super) fn perform<T>(
        &self,
        operation: impl FnOnce() -> Result<T, RemoteError>,
    ) -> Result<T, RemoteError> {
        self.check()?;
        let _effect = self.effects.barrier.read().map_err(|_| expired())?;
        self.check()?;
        operation()
    }
    pub(in super::super) fn try_io(
        &self,
        effect: &mut dyn FnMut() -> std::io::Result<usize>,
    ) -> std::io::Result<usize> {
        // Preserve the actual OS result (including WouldBlock) while the
        // original suspend-aware deadline and replacement barrier cover the
        // syscall. No authority/SQLite lock spans native I/O.
        let denied = || std::io::Error::from(std::io::ErrorKind::PermissionDenied);
        self.check().map_err(|_| denied())?;
        let _effect = self
            .effects
            .barrier
            .try_read()
            .map_err(|error| match error {
                std::sync::TryLockError::WouldBlock => std::io::ErrorKind::WouldBlock.into(),
                std::sync::TryLockError::Poisoned(_) => denied(),
            })?;
        self.check().map_err(|_| denied())?;
        effect()
    }
}
#[cfg(feature = "managed-codex")]
impl qgenie_remote_adapters::NativeEffectAuthorization for Fence {
    fn perform(
        &self,
        effect: &mut dyn FnMut() -> std::io::Result<usize>,
    ) -> std::io::Result<usize> {
        self.try_io(effect)
    }
}
impl Authority {
    pub(in super::super) fn fence(&self, context: &RemoteContext) -> Result<Fence, RemoteError> {
        let deadline = self.validate(context, Instant::now())?;
        let active = self.active.get(&context.scope).ok_or_else(expired)?;
        let grant = active
            .grants
            .get(&context.access_nonce)
            .ok_or_else(expired)?;
        Ok(Fence {
            effects: active.effects.clone(),
            issued_elapsed: grant.issued_elapsed,
            deadline,
            elapsed_deadline: grant.expires_elapsed.min(
                grant.issued_elapsed
                    + Duration::from_millis(u64::from(context.remaining_access_ms)),
            ),
        })
    }
}

#[cfg(all(test, feature = "managed-codex"))]
mod native_io_tests {
    use super::*;
    use qgenie_remote_adapters::NativeEffectAuthorization;
    #[test]
    fn native_io_preserves_backpressure_and_never_calls_a_fenced_effect() {
        let elapsed = clock::elapsed().unwrap();
        let effects = Arc::new(Effects::new());
        let fence = Fence {
            effects: effects.clone(),
            issued_elapsed: elapsed,
            deadline: Instant::now() + Duration::from_secs(5),
            elapsed_deadline: elapsed + Duration::from_secs(5),
        };
        let mut called = 0;
        let result = NativeEffectAuthorization::perform(&fence, &mut || {
            called += 1;
            Err(std::io::ErrorKind::WouldBlock.into())
        });
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
        {
            let _busy = effects.barrier.write().unwrap();
            let result = NativeEffectAuthorization::perform(&fence, &mut || {
                panic!("busy barrier admitted an OS write")
            });
            assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::WouldBlock);
        }
        effects.stop();
        let result = NativeEffectAuthorization::perform(&fence, &mut || {
            called += 1;
            Ok(1)
        });
        assert_eq!(
            result.unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        assert_eq!(called, 1);
    }
}
