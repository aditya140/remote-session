//! Owner metadata and protected recorded history. Network work never holds the UI loop.
use crate::app::{SignalGuard, TerminalGuard};
use crate::display::multiline as clean;
use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use qgenie_remote_client::{
    GatewayClient, GatewayConfig, LiveClient, LiveFrame, UserCredentials, observation_can_reconnect,
};
use qgenie_remote_proto::{
    rest::{AccessToken, HistoryQuery, HistoryStream, HistoryStreamsQuery},
    wire::{Machine, Message, Session},
    *,
};
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use std::{path::Path, sync::atomic::Ordering, time::Duration};
use tokio::task::JoinSet;
#[derive(PartialEq)]
enum Mode {
    Machines,
    Sessions,
    Streams,
    History,
}
struct App {
    origin: String,
    approvals: crate::remote_badge::Badge,
    machines: Vec<Machine>,
    machine: usize,
    sessions: Vec<Session>,
    session_after: Option<SessionId>,
    session_next: Option<SessionId>,
    active_only: bool,
    attention_only: bool,
    selected: usize,
    streams: Vec<HistoryStream>,
    stream: usize,
    stream_next: Option<String>,
    history_next: Option<String>,
    lines: Vec<String>,
    scroll: u16,
    mode: Mode,
    message: String,
    busy: bool,
    reload: bool,
    machine_filter: String,
    session_filter: String,
    filter_backup: Option<String>,
    visible_machines: Vec<usize>,
    visible_sessions: Vec<usize>,
}
enum Update {
    Machines(Vec<Machine>),
    Sessions(MachineId, Vec<Session>, Option<SessionId>),
    Streams(SessionId, Vec<HistoryStream>, Option<String>, bool),
    History(
        SessionId,
        RecordingEpochId,
        Vec<String>,
        Option<String>,
        bool,
    ),
    Connected(LiveClient, Vec<Machine>),
}
impl App {
    fn matches_machine(machine: &Machine, query: &str) -> bool {
        query.is_empty()
            || machine.id.to_string().contains(query)
            || machine.name.to_lowercase().contains(query)
            || machine.hostname.to_lowercase().contains(query)
            || machine.labels.iter().any(|(key, value)| {
                key.to_lowercase().contains(query) || value.to_lowercase().contains(query)
            })
    }
    fn matches_session(session: &Session, query: &str) -> bool {
        query.is_empty()
            || session.id.to_string().contains(query)
            || session.cwd.to_lowercase().contains(query)
            || format!("{:?}", session.state)
                .to_lowercase()
                .contains(query)
            || match &session.kind {
                qgenie_remote_proto::wire::LaunchKind::Pty { command } => command
                    .first()
                    .is_some_and(|value| value.to_lowercase().contains(query)),
                qgenie_remote_proto::wire::LaunchKind::Agent { agent, .. } => {
                    agent.to_lowercase().contains(query)
                }
            }
    }
    fn selected_machine(&self) -> Option<&Machine> {
        self.machines
            .get(self.machine)
            .filter(|machine| Self::matches_machine(machine, &self.machine_filter.to_lowercase()))
    }
    fn needs_attention(&self, session: &Session) -> bool {
        self.approvals.pending(session.id) > 0
            || session.recording_warning.is_some()
            || matches!(
                session.state,
                SessionState::Failed | SessionState::Lost | SessionState::OutcomeUnknown
            )
    }
    fn selected_session(&self) -> Option<&Session> {
        self.sessions.get(self.selected).filter(|session| {
            Some(session.machine) == self.machine_id()
                && Self::matches_session(session, &self.session_filter.to_lowercase())
                && (!self.attention_only || self.needs_attention(session))
        })
    }
    fn refilter(&mut self) {
        let query = self.machine_filter.to_lowercase();
        self.visible_machines = self
            .machines
            .iter()
            .enumerate()
            .filter_map(|(index, machine)| Self::matches_machine(machine, &query).then_some(index))
            .collect();
        if !self.visible_machines.contains(&self.machine) {
            self.machine = self.visible_machines.first().copied().unwrap_or(0);
        }
        let machine = self.machine_id();
        let query = self.session_filter.to_lowercase();
        self.visible_sessions = self
            .sessions
            .iter()
            .enumerate()
            .filter_map(|(index, session)| {
                (Some(session.machine) == machine
                    && Self::matches_session(session, &query)
                    && (!self.attention_only || self.needs_attention(session)))
                .then_some(index)
            })
            .collect();
        if !self.visible_sessions.contains(&self.selected) {
            self.selected = self.visible_sessions.first().copied().unwrap_or(0);
        }
    }
    fn filter(&mut self) -> &mut String {
        if self.mode == Mode::Machines {
            &mut self.machine_filter
        } else {
            &mut self.session_filter
        }
    }
    fn move_filtered(indices: &[usize], selected: &mut usize, down: bool) {
        if let Some(index) = indices.iter().position(|index| index == selected) {
            *selected = indices[if down {
                (index + 1).min(indices.len() - 1)
            } else {
                index.saturating_sub(1)
            }];
        } else if let Some(first) = indices.first() {
            *selected = *first;
        }
    }
    fn machine_id(&self) -> Option<MachineId> {
        self.selected_machine().map(|m| m.id)
    }
    fn session_id(&self) -> Option<SessionId> {
        self.selected_session().map(|s| s.id)
    }
    fn apply(&mut self, update: Update) -> Option<LiveClient> {
        match update {
            Update::Machines(machines) => {
                let id = self.machine_id();
                self.machines = machines;
                self.machine = id
                    .and_then(|id| self.machines.iter().position(|m| m.id == id))
                    .unwrap_or(0);
                if id != self.machine_id() {
                    self.sessions.clear();
                    self.streams.clear();
                    self.lines.clear();
                    self.selected = 0;
                    self.mode = Mode::Machines;
                }
            }
            Update::Sessions(machine, sessions, next) if Some(machine) == self.machine_id() => {
                let selected = self.session_id();
                self.sessions = sessions;
                self.session_next = next;
                self.selected = selected
                    .and_then(|id| self.sessions.iter().position(|s| s.id == id))
                    .unwrap_or(0);
                self.mode = Mode::Sessions;
                self.message =
                    "v live view · Enter recordings · ! attention · b live/all history · n next page · Home first page".into();
            }
            Update::Streams(session, mut streams, next, append)
                if Some(session) == self.session_id() =>
            {
                if append {
                    self.streams.append(&mut streams);
                } else {
                    self.streams = streams;
                    self.stream = 0;
                }
                self.stream_next = next;
                self.mode = Mode::Streams;
                self.message =
                    "Select a recorded stream. Available history can contain gaps.".into();
            }
            Update::History(session, epoch, mut lines, next, append)
                if Some(session) == self.session_id()
                    && self
                        .streams
                        .get(self.stream)
                        .is_some_and(|s| s.stream_epoch == epoch) =>
            {
                if append {
                    self.lines.append(&mut lines);
                } else {
                    self.lines = lines;
                    self.scroll = 0;
                }
                let mut bytes = self.lines.iter().map(String::len).sum::<usize>();
                let mut removed = 0;
                while self.lines.len() - removed > 16384 || bytes > 1024 * 1024 {
                    bytes = bytes.saturating_sub(self.lines[removed].len());
                    removed += 1;
                    self.message = "Display limit reached; earlier page text was removed.".into();
                }
                self.lines.drain(..removed);
                self.history_next = next;
                self.mode = Mode::History;
            }
            Update::Connected(client, machines) => {
                self.approvals = crate::remote_badge::Badge::default();
                self.machines = machines;
                self.machine = self.machine.min(self.machines.len().saturating_sub(1));
                self.message = "Connected. Enter lists sessions; h opens recorded history.".into();
                self.refilter();
                return Some(client);
            }
            _ => {}
        }
        self.refilter();
        None
    }
    fn draw(&self, frame: &mut Frame<'_>) {
        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(3),
                Constraint::Min(3),
                Constraint::Length(3),
            ])
            .split(frame.area());
        frame.render_widget(
            Paragraph::new(format!(
                "QGENIE REMOTE · Recorded sessions · {}\n{}",
                self.approvals.label(),
                clean(&self.origin)
            ))
            .block(Block::default().borders(Borders::BOTTOM)),
            areas[0],
        );
        match self.mode {
            Mode::Machines => {
                let items: Vec<_> = self
                    .visible_machines
                    .iter()
                    .map(|index| {
                        let m = &self.machines[*index];
                        ListItem::new(format!(
                            "{} · {} · {} sessions\n  {} · {}/{} · CLI {} · last seen {}\n  Agents: {}",
                            clean(&m.name),
                            if m.online { "online" } else { "offline" },
                            m.sessions,
                            clean(&m.hostname),
                            clean(&m.os),
                            clean(&m.arch),
                            clean(&m.cli_version),
                            m.last_seen.as_str(),
                            if m.agents.is_empty() {"none advertised".into()}else{m.agents.iter().map(|(name,agent)|format!("{} {}",clean(name),clean(&agent.version))).collect::<Vec<_>>().join(" · ")}
                        ))
                    })
                    .collect();
                frame.render_stateful_widget(
                    List::new(items)
                        .block(Block::default().borders(Borders::ALL).title("Machines"))
                        .highlight_style(Style::default().fg(Color::Cyan))
                        .highlight_symbol("› "),
                    areas[1],
                    &mut ListState::default().with_selected(
                        self.visible_machines
                            .iter()
                            .position(|index| *index == self.machine),
                    ),
                );
            }
            Mode::Sessions => {
                let items: Vec<_> = self
                    .visible_sessions
                    .iter()
                    .map(|index| {
                        let s = &self.sessions[*index];
                        ListItem::new(format!(
                            "{:?} · {}{}{}",
                            s.state,
                            clean(&s.cwd),
                            if self.approvals.pending(s.id) > 0 {
                                format!(" · {} approval(s) pending", self.approvals.pending(s.id))
                            } else {
                                String::new()
                            },
                            s.recording_warning
                                .as_ref()
                                .map(|w| format!(" · {}", clean(w)))
                                .unwrap_or_default()
                        ))
                    })
                    .collect();
                frame.render_stateful_widget(
                    List::new(items)
                        .block(Block::default().borders(Borders::ALL).title(
                            if self.attention_only {
                                "Sessions · needs attention (! shows all)"
                            } else if self.active_only {
                                "Sessions · live"
                            } else {
                                "Sessions · history"
                            },
                        ))
                        .highlight_style(Style::default().fg(Color::Cyan))
                        .highlight_symbol("› "),
                    areas[1],
                    &mut ListState::default().with_selected(
                        self.visible_sessions
                            .iter()
                            .position(|index| *index == self.selected),
                    ),
                );
            }
            Mode::Streams => {
                let items: Vec<_> = self
                    .streams
                    .iter()
                    .map(|s| ListItem::new(format!("{:?} · {}", s.stream, s.stream_epoch)))
                    .collect();
                frame.render_stateful_widget(
                    List::new(items)
                        .block(
                            Block::default()
                                .borders(Borders::ALL)
                                .title("Recorded streams · Enter reads history"),
                        )
                        .highlight_style(Style::default().fg(Color::Cyan))
                        .highlight_symbol("› "),
                    areas[1],
                    &mut ListState::default().with_selected(Some(self.stream)),
                );
            }
            Mode::History => frame.render_widget(
                Paragraph::new(self.lines.join("\n"))
                    .wrap(Wrap { trim: false })
                    .scroll((self.scroll, 0))
                    .block(
                        Block::default()
                            .borders(Borders::ALL)
                            .title("Recorded history · gaps remain explicit"),
                    ),
                areas[1],
            ),
        }
        let hint = if self.mode == Mode::Machines {
            "↑/↓ select · / filter · Enter sessions · a approvals · F2 rename · F3 labels · F6 revoke · c create · k keys · r reconnect · q quit"
        } else if self.mode == Mode::History {
            "↑/↓ scroll · n next page · Esc streams · r reconnect · q quit"
        } else {
            "↑/↓ select · / filter · Enter history · a approvals · v live · c create · u resume history · k keys · b live/history · n next · Home first · Esc back · r reconnect · q quit"
        };
        let filter = if self.mode == Mode::Machines {
            &self.machine_filter
        } else {
            &self.session_filter
        };
        let hint = if self.filter_backup.is_some() {
            format!(
                "Filter: {} ▌ · Enter applies · Esc cancels · Ctrl-U clears",
                clean(filter)
            )
        } else if !filter.is_empty() && matches!(self.mode, Mode::Machines | Mode::Sessions) {
            format!("Filter: {} · {hint}", clean(filter))
        } else {
            hint.into()
        };
        frame.render_widget(
            Paragraph::new(format!(
                "{}{}\n{hint}",
                if self.busy { "Loading · " } else { "" },
                clean(&self.message)
            ))
            .block(Block::default().borders(Borders::TOP)),
            areas[2],
        );
    }
}
fn cancels_read(code: KeyCode) -> bool {
    matches!(
        code,
        KeyCode::Enter
            | KeyCode::Esc
            | KeyCode::Home
            | KeyCode::F(2 | 3 | 6)
            | KeyCode::Char('/' | 'a' | 'b' | 'h' | 'r' | 'n' | 'v' | 'c' | 'k' | 'u')
    )
}
async fn session_page(
    handle: qgenie_remote_client::LiveHandle,
    machine: MachineId,
    after: Option<SessionId>,
    active_only: bool,
) -> Result<Update, RemoteError> {
    let (sessions, next) = handle.session_page(machine, after, 64, active_only).await?;
    Ok(Update::Sessions(machine, sessions, next))
}
async fn history(
    api: GatewayClient,
    token: AccessToken,
    session: SessionId,
    epoch: RecordingEpochId,
    after: Option<String>,
    append: bool,
) -> Result<Update, RemoteError> {
    let page = api
        .history(
            &token,
            session,
            HistoryQuery {
                stream_epoch: epoch,
                after,
                limit: 4,
            },
        )
        .await?;
    let mut lines = std::collections::VecDeque::new();
    let mut displayed_bytes = 0;
    let mut trimmed = false;
    let mut gaps = Vec::new();
    for item in page.items {
        match item {
            qgenie_remote_proto::rest::HistoryItem::Gap(g) => {
                let line = format!(
                    "GAP {}..{} · {:?} · {}",
                    g.from.0,
                    g.to.0,
                    g.reason,
                    g.captured_from.as_str()
                );
                gaps.push(line.clone());
                push_line(&mut lines, &mut displayed_bytes, &mut trimmed, line);
            }
            qgenie_remote_proto::rest::HistoryItem::Content(chunk) => {
                for record in api.decode_history(chunk).await? {
                    match record.content {
                        RecordedContent::Output { data, .. }
                        | RecordedContent::AgentText { data, .. } => {
                            let bytes = STANDARD.decode(data).map_err(|_| {
                                RemoteError::rejected(
                                    ErrorCode::InvalidFrame,
                                    "invalid historical output",
                                )
                            })?;
                            for line in clean(&String::from_utf8_lossy(&bytes)).lines() {
                                push_line(
                                    &mut lines,
                                    &mut displayed_bytes,
                                    &mut trimmed,
                                    line.to_owned(),
                                );
                            }
                        }
                        RecordedContent::Resize { rows, cols } => push_line(
                            &mut lines,
                            &mut displayed_bytes,
                            &mut trimmed,
                            format!("[terminal resized to {cols}×{rows}]"),
                        ),
                    }
                }
            }
        }
    }
    let mut lines: Vec<_> = lines.into_iter().collect();
    if trimmed {
        gaps.insert(
            0,
            "Display limit: earlier text on this page was omitted.".into(),
        );
        gaps.append(&mut lines);
        lines = gaps;
    }
    Ok(Update::History(session, epoch, lines, page.next, append))
}
fn push_line(
    lines: &mut std::collections::VecDeque<String>,
    bytes: &mut usize,
    trimmed: &mut bool,
    line: String,
) {
    *bytes += line.len();
    lines.push_back(line);
    while lines.len() > 16384 || *bytes > 1024 * 1024 {
        if let Some(line) = lines.pop_front() {
            *bytes = bytes.saturating_sub(line.len());
            *trimmed = true;
        }
    }
}
/// Credentials are injected by the caller. This view stores no token and owns no native
/// sessions. Use `run_remote_authenticated` for credentials that change during a session.
pub async fn run_remote(config: GatewayConfig, token: AccessToken) -> Result<()> {
    run_remote_inner(config, UserCredentials::fixed(token), None).await
}
/// Caller-owned private state holds start identities, never access credentials or argv.
pub async fn run_remote_with_state(
    config: GatewayConfig,
    token: AccessToken,
    state: &Path,
) -> Result<()> {
    run_remote_inner(config, UserCredentials::fixed(token), Some(state)).await
}
/// Use caller-owned credential updates; this view never persists user credentials.
pub async fn run_remote_authenticated(
    config: GatewayConfig,
    credentials: UserCredentials,
    state: &Path,
) -> Result<()> {
    run_remote_inner(config, credentials, Some(state)).await
}
async fn run_remote_inner(
    config: GatewayConfig,
    credentials: UserCredentials,
    state: Option<&Path>,
) -> Result<()> {
    let api = GatewayClient::new(config.clone())?;
    let machines = api.machines(&credentials.current()?).await?;
    let mut live = Some(api.connect_authenticated(&credentials).await?);
    let mut app = App {
        origin: config.origin,
        approvals: crate::remote_badge::Badge::default(),
        machines,
        machine: 0,
        sessions: Vec::new(),
        session_after: None,
        session_next: None,
        active_only: true,
        attention_only: false,
        selected: 0,
        streams: Vec::new(),
        stream: 0,
        stream_next: None,
        history_next: None,
        lines: Vec::new(),
        scroll: 0,
        mode: Mode::Machines,
        message: "Enter lists sessions. Protected history uses your current owner identity.".into(),
        busy: false,
        reload: false,
        machine_filter: String::new(),
        session_filter: String::new(),
        filter_backup: None,
        visible_machines: Vec::new(),
        visible_sessions: Vec::new(),
    };
    app.refilter();
    let mut terminal = TerminalGuard::enter()?;
    let signals = SignalGuard::start()?;
    let mut jobs: JoinSet<Result<Update, RemoteError>> = JoinSet::new();
    let mut recovery: JoinSet<Result<LiveClient, RemoteError>> = JoinSet::new();
    let mut ticks = tokio::time::interval(Duration::from_millis(30));
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _=ticks.tick()=>{
                if signals.requested.load(Ordering::Acquire) {break;}
                app.approvals.tick(live.as_ref().map(LiveClient::handle));
                app.refilter();
                if event::poll(Duration::ZERO)? && let Event::Key(key)=event::read()? && key.kind!=KeyEventKind::Release {
                    if app.filter_backup.is_some() {
                        match key.code {
                            KeyCode::Esc=>{*app.filter()=app.filter_backup.take().unwrap();},
                            KeyCode::Enter=>{app.filter_backup=None;},
                            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL)=>{signals.requested.store(true,Ordering::Release);},
                            KeyCode::Backspace=>{app.filter().pop();},
                            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL)=>app.filter().clear(),
                            KeyCode::Char(c) if !c.is_control() && !key.modifiers.intersects(KeyModifiers::CONTROL|KeyModifiers::ALT) && app.filter().len()+c.len_utf8()<=256=>app.filter().push(c),
                            _=>{},
                        }
                        app.refilter(); terminal.terminal.draw(|frame|app.draw(frame))?; continue;
                    }
                    if key.code==KeyCode::Char('q') || (key.code==KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL)) {break;}
                    if cancels_read(key.code) && !jobs.is_empty() {
                        jobs.abort_all();while jobs.join_next().await.is_some() {}
                    }
                    match key.code {
                        KeyCode::Char('!') if app.mode==Mode::Sessions=>{app.attention_only = !app.attention_only;app.refilter();},
                        KeyCode::Char('/') if jobs.is_empty() && matches!(app.mode,Mode::Machines|Mode::Sessions)=>{app.filter_backup=Some(app.filter().clone());},
                        KeyCode::Char('a') if jobs.is_empty()=>{if let Some(client)=live.as_mut(){match crate::remote_approvals::run(&mut terminal,&signals.requested,client).await {Ok(message)=>app.message=message,Err(error)=>app.message=error.to_string()}app.approvals.changed();}},
                        KeyCode::F(2|3|6) if jobs.is_empty() && app.mode==Mode::Machines=>{
                            if let Some(machine)=app.selected_machine().cloned() {
                                let action=match key.code {KeyCode::F(2)=>crate::remote_machine::Action::Rename,KeyCode::F(3)=>crate::remote_machine::Action::Label,_=>crate::remote_machine::Action::Revoke};
                                match crate::remote_dialog::drain(live.as_mut(),crate::remote_machine::run(&mut terminal,&signals.requested,api.clone(),credentials.current()?,machine,action)).await {Ok(message)=>app.message=message,Err(error)=>app.message=error.to_string()}
                                app.reload=true;
                            }
                        },
                        KeyCode::Esc=>{jobs.abort_all();app.mode=match app.mode {Mode::History=>Mode::Streams,Mode::Streams=>Mode::Sessions,_=>Mode::Machines};},
                        KeyCode::Up=>match app.mode {Mode::Machines=>App::move_filtered(&app.visible_machines,&mut app.machine,false),Mode::Sessions=>App::move_filtered(&app.visible_sessions,&mut app.selected,false),Mode::Streams=>app.stream=app.stream.saturating_sub(1),Mode::History=>app.scroll=app.scroll.saturating_sub(1)},
                        KeyCode::Down=>match app.mode {Mode::Machines=>App::move_filtered(&app.visible_machines,&mut app.machine,true),Mode::Sessions=>App::move_filtered(&app.visible_sessions,&mut app.selected,true),Mode::Streams=>app.stream=(app.stream+1).min(app.streams.len().saturating_sub(1)),Mode::History=>app.scroll=app.scroll.saturating_add(1)},
                        KeyCode::Char('v') if jobs.is_empty() && app.mode==Mode::Sessions=>{if let (Some(session),Some(client))=(app.selected_session().cloned(),live.as_mut()){match crate::remote_terminal::run(&mut terminal,&signals.requested,client,session,&api,&credentials).await {Ok(())=>app.message="Detached from native view; session lifetime is unchanged.".into(),Err(error)=>app.message=error.to_string()}}},
                        KeyCode::Char('c'|'k'|'u') if jobs.is_empty() && matches!(app.mode,Mode::Machines|Mode::Sessions) && (key.code!=KeyCode::Char('u') || app.mode==Mode::Sessions)=>{
                            if let (Some(machine),Some(client),Some(state))=(app.selected_machine().cloned(),live.as_mut(),state) {
                                let initial=app.selected_session().map(|s|s.cwd.clone()).unwrap_or_else(||"/".into());
                                let handle=client.handle();
                                let result=if key.code==KeyCode::Char('k') {crate::remote_dialog::drain(Some(client),crate::remote_start::recover(&mut terminal,&signals.requested,handle,machine,state)).await}else{
                                    let start=if key.code==KeyCode::Char('u') {match app.selected_session().filter(|session|session.capabilities.resume).cloned(){Some(session)=>crate::remote_start::Start::Resume(Box::new(session)),None=>{app.message="Historical resume requires a confirmed ended agent session.".into();continue;}}}else{crate::remote_start::Start::New(initial)};
                                    crate::remote_dialog::drain(Some(client),crate::remote_start::run(&mut terminal,&signals.requested,handle,machine,state,start)).await
                                };
                                match result {
                                    Ok(Some(session))=>{let machine=session.machine;app.active_only=true;app.session_after=None;app.mode=Mode::Sessions;app.selected=app.sessions.len();app.sessions.push(session.clone());app.refilter();if let Err(error)=crate::remote_terminal::run(&mut terminal,&signals.requested,client,session,&api,&credentials).await{app.message=error.to_string();}let handle=client.handle();jobs.spawn(session_page(handle,machine,None,true));},
                                    Ok(None)=>app.message="Creation view closed. Submitted identities remain in your private start journal; uncertain launches are never resent.".into(),
                                    Err(error)=>app.message=error.to_string(),
                                }
                            }else{app.message="Creation requires a connection and a caller-provided private state directory.".into();}
                        },
                        KeyCode::Char('r') if jobs.is_empty() && recovery.is_empty()=>{let api=api.clone();let credentials=credentials.clone();jobs.spawn(async move {let machines=api.machines(&credentials.current()?).await?;Ok(Update::Connected(api.connect_authenticated(&credentials).await?,machines))});},
                        KeyCode::Enter if jobs.is_empty() && app.mode==Mode::Machines=>{app.active_only=true;app.session_after=None;if let (Some(machine),Some(client))=(app.machine_id(),live.as_ref()) {let handle=client.handle();jobs.spawn(session_page(handle,machine,None,true));}},
                        KeyCode::Char('b')|KeyCode::Home|KeyCode::Char('n') if jobs.is_empty() && app.mode==Mode::Sessions=>{
                            if key.code==KeyCode::Char('b') {app.active_only = !app.active_only;app.session_after=None;}
                            else if key.code==KeyCode::Home {app.session_after=None;}
                            else if let Some(next)=app.session_next {app.session_after=Some(next);}else{continue;}
                            if let (Some(machine),Some(client))=(app.machine_id(),live.as_ref()){jobs.spawn(session_page(client.handle(),machine,app.session_after,app.active_only));}
                        },
                        KeyCode::Enter|KeyCode::Char('h') if jobs.is_empty() && app.mode==Mode::Sessions=>{if let Some(session)=app.session_id() {let api=api.clone();let token=credentials.current()?;jobs.spawn(async move {let page=api.history_streams(&token,session,HistoryStreamsQuery{after:None,limit:64}).await?;Ok(Update::Streams(session,page.streams,page.next,false))});}},
                        KeyCode::Enter if jobs.is_empty() && app.mode==Mode::Streams=>{if let (Some(session),Some(stream))=(app.session_id(),app.streams.get(app.stream)) {let api=api.clone();let token=credentials.current()?;let epoch=stream.stream_epoch;jobs.spawn(history(api,token,session,epoch,None,false));}},
                        KeyCode::Char('n') if jobs.is_empty() && app.mode==Mode::Streams=>{if let (Some(session),Some(after))=(app.session_id(),app.stream_next.clone()) {if app.streams.len()>=4096 {app.message="Stream display limit reached; reopen history to browse again.".into();}else{let api=api.clone();let token=credentials.current()?;jobs.spawn(async move {let page=api.history_streams(&token,session,HistoryStreamsQuery{after:Some(after),limit:64}).await?;Ok(Update::Streams(session,page.streams,page.next,true))});}}},
                        KeyCode::Char('n') if jobs.is_empty() && app.mode==Mode::History=>{if let (Some(session),Some(stream),Some(after))=(app.session_id(),app.streams.get(app.stream),app.history_next.clone()) {jobs.spawn(history(api.clone(),credentials.current()?,session,stream.stream_epoch,Some(after),true));}},
                        _=>{},
                    }
                }
                if jobs.is_empty() && app.reload {app.reload=false;let api=api.clone();let token=credentials.current()?;jobs.spawn(async move {Ok(Update::Machines(api.machines(&token).await?))});}
                app.busy = !jobs.is_empty();terminal.terminal.draw(|frame|app.draw(frame))?;
            },
            result=recovery.join_next(),if !recovery.is_empty()=>{match result {
                Some(Ok(Ok(client)))=>{live=Some(client);app.reload=true;app.approvals.changed();app.message="Observation reconnected. Control requires an explicit request.".into();if let (Some(machine),Some(client))=(app.machine_id(),live.as_ref()) && app.mode==Mode::Sessions {jobs.spawn(session_page(client.handle(),machine,app.session_after,app.active_only));}},
                Some(Ok(Err(error)))=>app.message=format!("{:?}: {}. Press r to retry after resolving access or connectivity.",error.code,error.message),
                _=>app.message="Recovery stopped. Press r to retry.".into(),
            }},
            update=jobs.join_next(),if !jobs.is_empty()=>{match update {Some(Ok(Ok(update)))=>{if let Some(replacement)=app.apply(update) {drop(live.replace(replacement));}},Some(Ok(Err(error)))=>app.message=format!("{:?}: {}. Replace an expired credential and reopen this view.",error.code,error.message),_=>app.message="Remote request stopped; press r to reconnect.".into()}},
            incoming=async {match &mut live {Some(client)=>client.next().await,None=>std::future::pending().await}}=>{
                match incoming {Some(event)=>if let LiveFrame::Text(envelope)=&event.frame {app.approvals.event(&envelope.message);match &envelope.message {
                    Message::Sessions{..}|Message::SessionUpdates{..} if app.mode==Mode::Sessions && envelope.machine==app.machine_id() && jobs.is_empty()=>{if let (Some(machine),Some(client))=(app.machine_id(),live.as_ref()){jobs.spawn(session_page(client.handle(),machine,app.session_after,app.active_only));}},
                    Message::MachineUpdated(_)=>app.reload=true,Message::MachinePage{page,total_pages,..} if page+1==*total_pages=>app.reload=true,Message::Error(error)=>app.message=format!("{:?}: {}",error.code,clean(&error.message)),_=>{}}},None=>{if let Some(client)=live.take(){let error=client.disconnect_error();jobs.abort_all();while jobs.join_next().await.is_some(){}app.message=format!("{:?}: {}",error.code,error.message);if observation_can_reconnect(&error) && recovery.is_empty(){let api=api.clone();let credentials=credentials.clone();recovery.spawn(async move{api.reconnect_observation(&credentials).await});app.message="Connection lost. Reconnecting observation; native sessions continue.".into();}}}}
            }
        }
    }
    jobs.abort_all();
    recovery.abort_all();
    while jobs.join_next().await.is_some() {}
    while recovery.join_next().await.is_some() {}
    drop(terminal);
    if let Some(client) = live {
        client.close().await;
    }
    Ok(())
}
