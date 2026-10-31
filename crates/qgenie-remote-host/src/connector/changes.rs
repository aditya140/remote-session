//! Live updates do not scan historical sessions. History is backfilled one page at a time.
use super::*;
use crate::{NativeAccess, NativeClient};
use qgenie_remote_proto::SessionId;
use std::{collections::BTreeMap, sync::Mutex};

#[derive(Clone, Default)]
pub(super) struct State {
    active: BTreeMap<SessionId, (Counter, Vec<u8>)>,
    refreshed: Option<Instant>,
    history_after: Option<SessionId>,
    history_at: Option<Instant>,
}
async fn read(
    client: &NativeClient,
    access: &NativeAccess,
    message: Message,
) -> Result<Envelope, Error> {
    let mut request = Envelope::new(message);
    request.machine = Some(client.scope().machine);
    client
        .read(access, client.scope().owner, access.connection(), request)
        .await
}
async fn publish(
    sender: &ws::Sender,
    machine: qgenie_remote_proto::MachineId,
    sessions: Vec<qgenie_remote_proto::wire::Session>,
) -> Result<(), Error> {
    if sessions.is_empty() {
        return Ok(());
    }
    let mut envelope = Envelope::new(Message::SessionUpdates { sessions });
    envelope.machine = Some(machine);
    transmit(sender, envelope, Priority::Event).await
}
pub(super) async fn follow(
    sender: ws::Sender,
    client: NativeClient,
    access: NativeAccess,
    cache: Arc<Mutex<State>>,
) -> Result<(), Error> {
    let mut changes = client.watch_changes(&access).await?;
    loop {
        let pages = client.active_session_pages(&access).await?;
        let previous = cache.lock().map_err(|_| Error::Unavailable)?.clone();
        let mut current = previous.clone();
        current.active.clear();
        let mut metadata_changed = previous.active.len()
            != pages.iter().map(Vec::len).sum::<usize>()
            || previous
                .refreshed
                .is_none_or(|at| at.elapsed() >= Duration::from_secs(3));
        let mut advanced = Vec::new();
        for session in pages.iter().flatten() {
            let mut metadata = session.clone();
            metadata.head = Counter(0);
            let metadata = serde_json::to_vec(&metadata).map_err(|_| Error::Protocol)?;
            metadata_changed |= previous
                .active
                .get(&session.id)
                .is_none_or(|old| old.1 != metadata);
            if previous
                .active
                .get(&session.id)
                .is_some_and(|old| session.head > old.0)
            {
                advanced.push(session.id);
            }
            current.active.insert(session.id, (session.head, metadata));
        }
        // A session leaving live inventory still needs its final durable projection.
        for id in previous
            .active
            .keys()
            .filter(|id| !current.active.contains_key(id))
        {
            let reply = read(&client, &access, Message::SessionGet { session: *id }).await?;
            let Message::SessionFound(session) = reply.message else {
                return Err(Error::Protocol);
            };
            publish(&sender, client.scope().machine, vec![session]).await?;
        }
        if metadata_changed {
            for sessions in pages {
                publish(&sender, client.scope().machine, sessions).await?;
            }
            current.refreshed = Some(Instant::now());
        }
        for session in advanced {
            transmit(
                &sender,
                client.output_head(&access, session).await?,
                Priority::Event,
            )
            .await?;
        }
        // At most one history page per pass. Periodic full traversal also repairs
        // sessions that both started and ended while the connector was absent.
        if current.history_at.is_none_or(|at| Instant::now() >= at) {
            let reply = read(
                &client,
                &access,
                Message::SessionListPage {
                    after: current.history_after,
                    limit: 32,
                    active_only: false,
                },
            )
            .await?;
            let Message::SessionPage { sessions, next } = reply.message else {
                return Err(Error::Protocol);
            };
            publish(&sender, client.scope().machine, sessions).await?;
            current.history_after = next;
            current.history_at = Some(
                Instant::now()
                    + if next.is_some() {
                        Duration::from_millis(100)
                    } else {
                        Duration::from_secs(30)
                    },
            );
        }
        let refresh_at = current.refreshed.unwrap_or_else(Instant::now) + Duration::from_secs(3);
        let reconcile_at = current.history_at.unwrap_or(refresh_at).min(refresh_at);
        *cache.lock().map_err(|_| Error::Unavailable)? = current;
        // Native changes wake this task immediately. Reconcile metadata and
        // advance history on their own deadlines when the owner is idle.
        if let Ok(result) = tokio::time::timeout_at(reconcile_at, changes.changed()).await {
            result?;
        }
    }
}
