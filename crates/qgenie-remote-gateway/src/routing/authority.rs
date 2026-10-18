//! Authority monitoring is independent of application dispatch, inventory pages
//! and Redis. Denial aborts the actual transport driver and its queued frames.
use super::{hub::Hub, now_ms};
use crate::{Error, MachinePrincipal};
use qgenie_remote_proto::{Counter, UserId};
use qgenie_remote_transport::ws::ConnectionCancellation;
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{sync::watch, task::JoinHandle};
mod gate;
pub(super) use gate::Gate;

pub(super) enum Actor {
    User(UserId),
    Host(MachinePrincipal, Arc<AtomicU64>),
}
pub(super) struct Watch(JoinHandle<()>);
struct Fence {
    stop: watch::Sender<bool>,
    driver: ConnectionCancellation,
    gate: Arc<Gate>,
}
impl Drop for Fence {
    fn drop(&mut self) {
        self.gate.deny();
        self.stop.send_replace(true);
        self.driver.cancel();
    }
}
impl Drop for Watch {
    fn drop(&mut self) {
        self.0.abort();
    }
}
impl Watch {
    pub async fn admit(hub: &Hub, actor: &Actor, expires: &AtomicU64) -> Result<Arc<Gate>, Error> {
        let gate = Arc::new(Gate::new(expires.load(Ordering::Acquire))?);
        check(hub, actor, expires, &gate).await?;
        Ok(gate)
    }
    pub fn start(
        hub: Arc<Hub>,
        actor: Actor,
        expires: Arc<AtomicU64>,
        stop: watch::Sender<bool>,
        driver: ConnectionCancellation,
        gate: Arc<Gate>,
    ) -> Self {
        let task = tokio::spawn(async move {
            let _fence = Fence {
                stop: stop.clone(),
                driver,
                gate: gate.clone(),
            };
            let mut cancelled = stop.subscribe();
            let mut draining = hub.stop.subscribe();
            let mut tick = tokio::time::interval(Duration::from_secs(2));
            tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                if *cancelled.borrow() || *draining.borrow() {
                    break;
                }
                let result = tokio::select! {
                    biased;
                    _=cancelled.changed()=>break,
                    _=draining.changed()=>break,
                    result=async {
                        tick.tick().await;
                        check(&hub, &actor, &expires, &gate).await
                    }=>result,
                };
                if result.is_err() {
                    break;
                }
            }
        });
        Self(task)
    }
}
pub(super) async fn check(
    hub: &Hub,
    actor: &Actor,
    expires: &AtomicU64,
    gate: &Gate,
) -> Result<(), Error> {
    let started = elapsed()?;
    tokio::time::timeout(Duration::from_secs(3), async {
        if expires.load(Ordering::Acquire).saturating_mul(1000) <= now_ms()? {
            return Err(Error::Unauthenticated);
        }
        let identity = hub.gateway.identity.as_ref().ok_or(Error::Configuration)?;
        let owner = match actor {
            Actor::User(owner) => {
                identity.assert_user(*owner).await?;
                *owner
            }
            Actor::Host(host, generation) => {
                identity.assert_machine(host).await?;
                let generation = generation.load(Ordering::Acquire);
                if generation != 0
                    && !hub
                        .gateway
                        .store
                        .host_current(host, Counter(generation))
                        .await?
                {
                    return Err(Error::Forbidden);
                }
                host.owner
            }
        };
        let directory_until = if let Some(directory) = &identity.directory {
            let before = elapsed()?;
            Some(before + directory.assert(owner).await?)
        } else {
            None
        };
        if elapsed()?
            .checked_sub(started)
            .is_none_or(|age| age >= Duration::from_secs(3))
        {
            return Err(Error::Unavailable);
        }
        gate.renew(started, directory_until)
    })
    .await
    .map_err(|_| Error::Unavailable)?
}
fn elapsed() -> Result<Duration, Error> {
    qgenie_remote_transport::elapsed::now().map_err(|_| Error::Unavailable)
}
