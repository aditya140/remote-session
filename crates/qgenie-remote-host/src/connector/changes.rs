//! Fresh native state follows coalesced notifications, never a mutation retry.
use super::*;
use crate::{NativeAccess, NativeClient};
use qgenie_remote_proto::SessionId;
use std::{collections::BTreeMap, sync::Mutex};

#[derive(Clone, Default)]
pub(super) struct State(BTreeMap<SessionId, (Counter, Vec<u8>)>, Option<Instant>);
pub(super) async fn follow(
    sender: ws::Sender,
    client: NativeClient,
    access: NativeAccess,
    cache: Arc<Mutex<State>>,
) -> Result<(), Error> {
    let mut changes = client.watch_changes(&access).await?;
    loop {
        // Subscribe before reading. A concurrent native commit remains pending.
        let mut request = Envelope::new(Message::SessionList);
        request.machine = Some(client.scope().machine);
        let mut state = client
            .read(&access, client.scope().owner, access.connection(), request)
            .await?;
        state.re = None;
        let Message::Sessions { sessions } = &state.message else {
            return Err(Error::Protocol);
        };
        let previous = cache.lock().map_err(|_| Error::Unavailable)?.clone();
        let mut current = State(BTreeMap::new(), previous.1);
        let mut metadata_changed = previous.0.len() != sessions.len()
            || previous
                .1
                .is_none_or(|last| last.elapsed() >= Duration::from_secs(3));
        let mut advanced = Vec::new();
        for session in sessions {
            let mut metadata = session.clone();
            metadata.head = Counter(0);
            let metadata = serde_json::to_vec(&metadata).map_err(|_| Error::Protocol)?;
            metadata_changed |= previous
                .0
                .get(&session.id)
                .is_none_or(|old| old.1 != metadata);
            if previous
                .0
                .get(&session.id)
                .is_some_and(|old| session.head > old.0)
            {
                advanced.push(session.id);
            }
            current.0.insert(session.id, (session.head, metadata));
        }
        if metadata_changed {
            // Owner-wide metadata changes stay distinct from high-rate output.
            transmit(&sender, state, Priority::Event).await?;
            current.1 = Some(Instant::now());
        }
        for session in advanced {
            let head = client.output_head(&access, session).await?;
            transmit(&sender, head, Priority::Event).await?;
        }
        // Commit only after queue admission. Cancellation/replacement cannot
        // suppress an unoffered final update in the next subscription.
        *cache.lock().map_err(|_| Error::Unavailable)? = current;
        changes.changed().await?;
    }
}
