//! PostgreSQL disables its ordinary statement/transaction timers during synchronous COMMIT.
//! Own that wait independently of HTTP cancellation, send protocol CancelRequest on deadline,
//! retire the frontend, and never acknowledge a canceled local commit as durable.
use crate::Error;
use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use std::{
    ops::{Deref, DerefMut},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{sync::oneshot, task::JoinSet, time::Instant};

#[derive(Default)]
pub(crate) struct Commits {
    stopped: AtomicBool,
    tasks: Mutex<JoinSet<()>>,
}
impl Commits {
    fn submit(
        &self,
        transaction: Transaction<'static, Postgres>,
        deadline: Instant,
    ) -> Result<oneshot::Receiver<Result<(), Error>>, Error> {
        let mut tasks = self.tasks.lock().unwrap_or_else(|p| p.into_inner());
        while tasks.try_join_next().is_some() {}
        if self.stopped.load(Ordering::Acquire) {
            return Err(Error::Draining);
        }
        let (send, receive) = oneshot::channel();
        // Each task retains its checked-out pool connection. Pool size therefore bounds
        // these owned tasks, including when callers disconnect before collecting a result.
        tasks.spawn(async move {
            let result = commit(transaction, deadline).await;
            let _ = send.send(result);
        });
        Ok(receive)
    }
    pub(crate) async fn close(&self) {
        let mut tasks = {
            let mut tasks = self.tasks.lock().unwrap_or_else(|p| p.into_inner());
            self.stopped.store(true, Ordering::Release);
            std::mem::take(&mut *tasks)
        };
        while tasks.join_next().await.is_some() {}
    }
}
pub(crate) struct AuthorityTransaction {
    inner: Transaction<'static, Postgres>,
    deadline: Instant,
    commits: Arc<Commits>,
}
impl Deref for AuthorityTransaction {
    type Target = PgConnection;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl DerefMut for AuthorityTransaction {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}
impl AuthorityTransaction {
    pub async fn commit(self) -> Result<(), Error> {
        let Self {
            inner,
            deadline,
            commits,
        } = self;
        commits
            .submit(inner, deadline)?
            .await
            .map_err(|_| Error::Database)?
    }
    #[cfg(test)]
    pub async fn rollback(self) -> Result<(), sqlx::Error> {
        self.inner.rollback().await
    }
}
pub(crate) async fn begin(
    pool: &PgPool,
    commits: Arc<Commits>,
) -> Result<AuthorityTransaction, Error> {
    if commits.stopped.load(Ordering::Acquire) {
        return Err(Error::Draining);
    }
    let deadline = Instant::now() + Duration::from_secs(3);
    let inner = tokio::time::timeout_at(deadline, pool.begin())
        .await
        .map_err(|_| Error::Database)?
        .map_err(database_error)?;
    Ok(AuthorityTransaction {
        inner,
        deadline,
        commits,
    })
}
async fn commit(
    transaction: Transaction<'static, Postgres>,
    deadline: Instant,
) -> Result<(), Error> {
    if Instant::now() >= deadline {
        let _ = tokio::time::timeout(Duration::from_secs(1), transaction.rollback()).await;
        return Err(Error::Database);
    }
    let cancellation = transaction.cancel_token();
    let mut committing = std::pin::pin!(transaction.commit());
    tokio::select! {
        biased;
        _=tokio::time::sleep_until(deadline)=>{
            // CancelRequest is connection-specific and works through transaction pooling
            // without needing another SQL backend or broad pg_signal_backend privilege.
            let _=tokio::time::timeout(Duration::from_secs(1),cancellation.cancel_query()).await;
            let _=tokio::time::timeout(Duration::from_secs(1),committing.as_mut()).await;
            // A canceled synchronous wait may return COMMIT success with a warning after
            // committing only locally. Its result is always uncertainty, never a receipt.
            Err(Error::Database)
        }
        result=committing.as_mut()=>result.map_err(database_error),
    }
}

fn database_error(error: sqlx::Error) -> Error {
    match error.as_database_error().and_then(|e| e.code()).as_deref() {
        Some("42501") => Error::Forbidden,
        Some("22023") => Error::Invalid,
        Some("23505") | Some("23514") => Error::Conflict,
        _ => Error::Database,
    }
}
