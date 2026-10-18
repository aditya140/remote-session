//! Each gateway owns its directory evidence. Restart requires a new read; UTC
//! projections and successful JWT authentication cannot extend these deadlines.
use super::directory::{DirectoryObservation, GraphDirectory};
use crate::{Error, Store};
use qgenie_remote_proto::UserId;
use sqlx::Row;
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use tokio::{
    sync::{Semaphore, watch},
    task::{JoinHandle, JoinSet},
};
mod metrics;

const MAX_USERS: usize = 4096;
const FRESH: Duration = Duration::from_secs(840);
const REFRESH: Duration = Duration::from_secs(300);
const RETRY: Duration = Duration::from_secs(30);
const ACTIVE: Duration = Duration::from_secs(900);
const EVICT: Duration = Duration::from_secs(1800);

fn now() -> Result<Duration, Error> {
    qgenie_remote_transport::elapsed::now().map_err(|_| Error::Unavailable)
}
#[derive(Default)]
struct Status {
    proof: Option<DirectoryObservation>,
    attempt: Option<Duration>,
    used: Duration,
    busy: bool,
}
impl Status {
    fn evidence(&self, now: Duration) -> Result<Duration, Error> {
        let proof = self.proof.ok_or(Error::Unavailable)?;
        let age = now
            .checked_sub(proof.started_elapsed)
            .ok_or(Error::Unavailable)?;
        if age >= FRESH {
            return Err(Error::Unavailable);
        }
        if !proof.enabled {
            return Err(Error::Forbidden);
        }
        Ok(FRESH - age)
    }
    fn due(&self, now: Duration) -> bool {
        !self.busy
            && self
                .attempt
                .is_none_or(|at| now.checked_sub(at).is_some_and(|age| age >= RETRY))
            && self.proof.is_none_or(|p| {
                now.checked_sub(p.started_elapsed)
                    .is_none_or(|age| age >= REFRESH)
            })
    }
}
struct Entry {
    user: UserId,
    object: String,
    status: Mutex<Status>,
}
/// Cancellation releases single-flight ownership, preserving the original proof.
struct Flight(Arc<Entry>);
impl Drop for Flight {
    fn drop(&mut self) {
        self.0.status.lock().unwrap().busy = false;
    }
}
struct Inner {
    store: Store,
    graph: GraphDirectory,
    users: Mutex<HashMap<UserId, Arc<Entry>>>,
    lookups: Arc<Semaphore>,
}
pub(crate) struct DirectoryFreshness {
    inner: Arc<Inner>,
    stop: watch::Sender<bool>,
    worker: Mutex<Option<JoinHandle<()>>>,
}
impl DirectoryFreshness {
    pub async fn start(store: Store, graph: GraphDirectory) -> Result<Self, Error> {
        let version: Option<i32> =
            sqlx::query_scalar("SELECT max(version) FROM qgr.schema_versions")
                .fetch_one(&store.pool)
                .await
                .map_err(|_| Error::Database)?;
        if !version.is_some_and(|v| v >= 5) {
            return Err(Error::Schema);
        }
        // Fail at composition, not at the first request, if the elapsed clock is unavailable.
        now()?;
        let inner = Arc::new(Inner {
            store,
            graph,
            users: Mutex::new(HashMap::new()),
            lookups: Arc::new(Semaphore::new(16)),
        });
        let (stop, receiver) = watch::channel(false);
        let worker = tokio::spawn(run(inner.clone(), receiver));
        Ok(Self {
            inner,
            stop,
            worker: Mutex::new(Some(worker)),
        })
    }
    /// Fresh reads return immediately while the owned worker refreshes them.
    /// First contact/stale evidence performs one bounded verification or denies.
    pub async fn assert(&self, user: UserId) -> Result<Duration, Error> {
        if *self.stop.borrow() {
            return Err(Error::Draining);
        }
        let entry = self.inner.entry(user).await?;
        {
            let mut status = entry.status.lock().unwrap();
            status.used = now()?;
            if let Ok(remaining) = status.evidence(status.used) {
                return Ok(remaining);
            }
            if !status.due(status.used) {
                return status.evidence(status.used);
            }
        }
        self.inner.refresh(entry.clone()).await?;
        entry.status.lock().unwrap().evidence(now()?)
    }
    pub async fn shutdown(&self) {
        self.stop.send_replace(true);
        let task = self.worker.lock().unwrap().take();
        if let Some(task) = task {
            let _ = task.await;
        }
    }
    /// Check an established proof without starting database/HTTPS work while a domain TX
    /// holds primary row locks. A stale/missing proof denies; callers refresh before the TX.
    pub(crate) fn cached(&self, user: UserId) -> Result<Duration, Error> {
        if *self.stop.borrow() {
            return Err(Error::Draining);
        }
        let entry = self
            .inner
            .users
            .lock()
            .unwrap()
            .get(&user)
            .cloned()
            .ok_or(Error::Unavailable)?;
        let mut status = entry.status.lock().unwrap();
        status.used = now()?;
        status.evidence(status.used)
    }
}
impl Drop for DirectoryFreshness {
    fn drop(&mut self) {
        self.stop.send_replace(true);
        if let Some(task) = self.worker.get_mut().unwrap().take() {
            task.abort();
        }
    }
}
impl Inner {
    async fn entry(&self, user: UserId) -> Result<Arc<Entry>, Error> {
        if let Some(entry) = self.users.lock().unwrap().get(&user).cloned() {
            return Ok(entry);
        }
        let _permit = self
            .lookups
            .clone()
            .try_acquire_owned()
            .map_err(|_| Error::Overloaded)?;
        let row = tokio::time::timeout(
            Duration::from_secs(3),
            sqlx::query("SELECT tenant,directory_object FROM qgr.users WHERE id=$1")
                .bind(user.to_string())
                .fetch_optional(&self.store.pool),
        )
        .await
        .map_err(|_| Error::Database)?
        .map_err(|_| Error::Database)?
        .ok_or(Error::Forbidden)?;
        if row.get::<String, _>("tenant") != self.graph.tenant() {
            return Err(Error::Forbidden);
        }
        let object = row
            .get::<Option<sqlx::types::Uuid>, _>("directory_object")
            .ok_or(Error::Forbidden)?
            .to_string();
        let mut users = self.users.lock().unwrap();
        if users.len() >= MAX_USERS {
            let now = now()?;
            users.retain(|_, entry| {
                let status = entry.status.lock().unwrap();
                status.busy || now.checked_sub(status.used).is_none_or(|age| age < EVICT)
            });
        }
        if !users.contains_key(&user) && users.len() >= MAX_USERS {
            return Err(Error::Overloaded);
        }
        Ok(users
            .entry(user)
            .or_insert_with(|| {
                Arc::new(Entry {
                    user,
                    object,
                    status: Mutex::new(Status::default()),
                })
            })
            .clone())
    }
    async fn refresh(&self, entry: Arc<Entry>) -> Result<(), Error> {
        {
            let mut status = entry.status.lock().unwrap();
            let now = now()?;
            if !status.due(now) {
                return Err(Error::Unavailable);
            }
            status.busy = true;
            status.attempt = Some(now);
        }
        let _flight = Flight(entry.clone());
        let started_utc = time::OffsetDateTime::now_utc();
        let observation = self.graph.observe(&entry.object).await?;
        // Verify elapsed freshness again before accepting any positive result.
        {
            let mut status = entry.status.lock().unwrap();
            if now()?
                .checked_sub(observation.started_elapsed)
                .is_none_or(|age| age >= FRESH)
            {
                return Err(Error::Unavailable);
            }
            status.proof = Some(observation);
        }
        // Observability only; another node's late response cannot roll this back.
        // No manual/global/machine revocation column is updated here.
        let mut tx = self.store.transaction().await?;
        sqlx::query("UPDATE qgr.users SET directory_enabled=$2,directory_verified_at=$3 WHERE id=$1 AND (directory_verified_at IS NULL OR directory_verified_at<=$3)")
            .bind(entry.user.to_string()).bind(observation.enabled).bind(started_utc)
            .execute(&mut *tx).await.map_err(|_| Error::Database)?;
        tx.commit().await?;
        Ok(())
    }
}
async fn run(inner: Arc<Inner>, mut stop: watch::Receiver<bool>) {
    let mut tasks = JoinSet::new();
    let mut tick = tokio::time::interval(Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            _=stop.changed()=>break,
            _=tasks.join_next(), if !tasks.is_empty()=>{},
            _=tick.tick()=> {
                let Ok(now) = now() else { continue; };
                let entries = inner.users.lock().unwrap().values().filter(|entry| {
                    let status = entry.status.lock().unwrap();
                    status.proof.is_some() && now.checked_sub(status.used).is_some_and(|age| age < ACTIVE) && status.due(now)
                }).take(16usize.saturating_sub(tasks.len())).cloned().collect::<Vec<_>>();
                for entry in entries {
                    let inner = inner.clone();
                    tasks.spawn(async move { let _ = inner.refresh(entry).await; });
                }
            }
        }
    }
    tasks.shutdown().await;
}

#[cfg(test)]
pub(crate) mod tests;
