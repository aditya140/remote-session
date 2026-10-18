use super::*;
use qgenie_remote_adapters::ScreenSnapshot;

const PAGE_BYTES: usize = 256 * 1024;
struct Captured {
    format: SnapshotFormat,
    session: SessionId,
    client: ClientId,
    created: Instant,
    seq: Counter,
    rows: u16,
    cols: u16,
    data: Vec<u8>,
}
#[derive(Default)]
pub(super) struct Snapshots(BTreeMap<MessageId, Captured>);
impl Snapshots {
    fn expire(&mut self) {
        self.0
            .retain(|_, snapshot| snapshot.created.elapsed() < Duration::from_secs(30));
    }
    pub(super) fn capture(
        &mut self,
        session: SessionId,
        client: ClientId,
        seq: Counter,
        screen: ScreenSnapshot,
    ) -> Result<Snapshot, RemoteError> {
        self.expire();
        self.0.retain(|_, value| value.client != client);
        let used: usize = self.0.values().map(|value| value.data.len()).sum();
        if self.0.len() >= 32 || used + screen.data.len() > 32 * 1024 * 1024 {
            return Err(RemoteError::rejected(
                ErrorCode::Overloaded,
                "snapshot assembly capacity unavailable",
            ));
        }
        let epoch = MessageId::new();
        self.0.insert(
            epoch,
            Captured {
                format: screen.format,
                session,
                client,
                created: Instant::now(),
                seq,
                rows: screen.rows,
                cols: screen.cols,
                data: screen.data,
            },
        );
        self.page(session, client, epoch, 0)
    }
    pub(super) fn page(
        &mut self,
        session: SessionId,
        client: ClientId,
        epoch: MessageId,
        page: u16,
    ) -> Result<Snapshot, RemoteError> {
        self.expire();
        let value = self
            .0
            .get(&epoch)
            .filter(|value| value.client == client && value.session == session)
            .ok_or_else(|| {
                RemoteError::rejected(
                    ErrorCode::HistoryTruncated,
                    "snapshot expired; request a new attachment",
                )
            })?;
        let total_pages = value.data.len().div_ceil(PAGE_BYTES) as u16;
        if page >= total_pages {
            return Err(RemoteError::rejected(
                ErrorCode::InvalidFrame,
                "snapshot page is outside its epoch",
            ));
        }
        let start = usize::from(page) * PAGE_BYTES;
        Ok(Snapshot {
            seq: value.seq,
            rows: value.rows,
            cols: value.cols,
            format: value.format,
            epoch,
            page,
            total_pages,
            data: STANDARD.encode(&value.data[start..(start + PAGE_BYTES).min(value.data.len())]),
        })
    }
}

impl Runtime {
    pub(super) fn snapshot(&self, state: &Session) -> Result<ScreenSnapshot, RemoteError> {
        if state.info.agent.is_some() {
            return self.agent_snapshot(state);
        }
        if matches!(
            state.info.state,
            SessionState::Lost | SessionState::OutcomeUnknown
        ) {
            return Err(RemoteError::rejected(
                ErrorCode::HistoryTruncated,
                "native owner was lost; the final output tail is unconfirmed",
            ));
        }
        if let Some(screen) = &state.screen {
            return screen.snapshot();
        }
        let store = lock(&self.store)?;
        let checkpoint = store.checkpoint(state.info.id).map_err(storage_error)?;
        let (mut cursor, mut screen) = match checkpoint {
            Some((seq, bytes)) => (seq, Screen::from_checkpoint(&bytes)?),
            None => (Counter(0), Screen::new(state.spec.rows, state.spec.cols)?),
        };
        if cursor > state.info.head {
            return Err(incomplete());
        }
        for event in store
            .snapshot_tail(state.info.id, cursor)
            .map_err(storage_error)?
        {
            if cursor.0.checked_add(1) != Some(event.seq.0)
                || matches!(event.body, EventBody::Gap { .. })
            {
                return Err(incomplete());
            }
            apply_screen(&mut screen, &event.body)?;
            cursor = event.seq;
        }
        if cursor != state.info.head {
            return Err(incomplete());
        }
        screen.snapshot()
    }
}

impl Runtime {
    fn agent_snapshot(&self, state: &Session) -> Result<ScreenSnapshot, RemoteError> {
        let mut view = if let Some(view) = &state.agent_state {
            view.clone()
        } else {
            let store = lock(&self.store)?;
            let mut view = match store.checkpoint(state.info.id).map_err(storage_error)? {
                Some((seq, bytes)) => AgentState::decode(&bytes, seq)?,
                None => AgentState::default(),
            };
            if view.seq > state.info.head {
                return Err(incomplete());
            }
            for event in store
                .snapshot_tail(state.info.id, view.seq)
                .map_err(storage_error)?
            {
                view.push(public_event(&event)?)?;
            }
            view
        };
        if view.seq != state.info.head {
            return Err(incomplete());
        }
        view.history_truncated |= state
            .info
            .agent
            .as_ref()
            .is_some_and(|agent| agent.native_home.is_some());
        view.unconfirmed_tail |= matches!(
            state.info.state,
            SessionState::Lost | SessionState::OutcomeUnknown
        );
        Ok(ScreenSnapshot {
            format: SnapshotFormat::AgentStateV1,
            rows: state.spec.rows,
            cols: state.spec.cols,
            data: view.encode()?,
        })
    }
}
fn incomplete() -> RemoteError {
    RemoteError::rejected(
        ErrorCode::HistoryTruncated,
        "complete terminal checkpoint and contiguous replay are unavailable",
    )
}
