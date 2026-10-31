//! A start is submitted once. Uncertain results retain the original key for lookup.
use crate::app::TerminalGuard;
use crate::display::single_line as clean;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use qgenie_remote_client::{LiveHandle, RemoteStartIdentity};
use qgenie_remote_proto::{
    wire::{DirectoryEntry, Launch, LaunchKind, Machine, Session, StartStage},
    *,
};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use std::{
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::task::JoinSet;

enum Update {
    Roots(Vec<String>),
    Identities(Vec<RemoteStartIdentity>, Option<IdempotencyKey>),
    Directory(String, Vec<DirectoryEntry>, Option<String>, bool),
    Started(Session),
    Status(StartStage, Option<SessionId>),
}
struct Picker {
    agent: Option<String>,
    resume: Option<String>,
    path: String,
    loaded: Option<String>,
    entries: Vec<DirectoryEntry>,
    next: Option<String>,
    roots: Vec<String>,
    choosing_root: bool,
    selected: usize,
    editing: bool,
    submitted: bool,
    key: IdempotencyKey,
    message: String,
}
impl Picker {
    fn show_roots(&mut self) {
        if self.roots.is_empty() {
            self.editing = true;
            return;
        }
        self.entries = self
            .roots
            .iter()
            .map(|path| DirectoryEntry {
                name: path.clone(),
                path: path.clone(),
                directory: true,
            })
            .collect();
        self.path = "Choose an allowed root".into();
        self.loaded = None;
        self.next = None;
        self.selected = 0;
        self.editing = false;
        self.choosing_root = true;
        self.message =
            "Select a root with Enter. The host checks the directory again before launch.".into();
    }

    fn draw(&self, frame: &mut ratatui::Frame<'_>, machine: &Machine, busy: bool) {
        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(5),
                Constraint::Min(3),
                Constraint::Length(5),
            ])
            .split(frame.area());
        frame.render_widget(
            Paragraph::new(format!(
                "{} · {}\nDirectory: {}{}\nStart identity: {}",
                self.agent
                    .as_ref()
                    .map(|name| format!(
                        "{} · {} {}",
                        if self.resume.is_some() {
                            "Resume agent history"
                        } else {
                            "New remote agent"
                        },
                        clean(name),
                        machine
                            .agents
                            .get(name)
                            .map(|agent| clean(&agent.version))
                            .unwrap_or_default()
                    ))
                    .unwrap_or_else(|| "New remote terminal".into()),
                clean(&machine.name),
                clean(&self.path),
                if self.editing { " ▌" } else { "" },
                self.key
            ))
            .block(Block::default().borders(Borders::ALL)),
            areas[0],
        );
        let items: Vec<_> = self
            .entries
            .iter()
            .map(|entry| {
                ListItem::new(format!(
                    "{} {}",
                    if entry.directory { "▸" } else { " " },
                    clean(&entry.name)
                ))
            })
            .collect();
        let mut selected =
            ListState::default().with_selected((!items.is_empty()).then_some(self.selected));
        frame.render_stateful_widget(
            List::new(items).highlight_symbol("› ").block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(" Allowed host directories "),
            ),
            areas[1],
            &mut selected,
        );
        let keys = if self.submitted {
            "l looks up original start · Esc returns (never resends)"
        } else if self.editing {
            "Enter browses · Esc returns"
        } else if self.resume.is_some() {
            "Enter opens folder · p edits path · Ctrl-N resumes history in a new session · Esc returns"
        } else {
            "Enter opens folder · F1 roots · p edits path · F5 terminal/advertised agent · Ctrl-N starts selected runtime · Esc returns"
        };
        frame.render_widget(
            Paragraph::new(format!(
                "{}{}\n{}",
                if busy { "Working… " } else { "" },
                clean(&self.message),
                keys
            ))
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL)),
            areas[2],
        );
    }
}
async fn browse(
    handle: LiveHandle,
    machine: MachineId,
    path: String,
    cursor: Option<String>,
    append: bool,
) -> Result<Update, RemoteError> {
    let (entries, next) = handle.browse(machine, path.clone(), cursor, 128).await?;
    Ok(Update::Directory(path, entries, next, append))
}
pub(crate) enum Start {
    New(String),
    Resume(Box<Session>),
}
pub(crate) async fn run(
    terminal: &mut TerminalGuard,
    signal: &AtomicBool,
    handle: LiveHandle,
    machine: Machine,
    state: &Path,
    initial: Start,
) -> Result<Option<Session>> {
    let (initial, agent, resume) = match initial {
        Start::New(path) => (path, None, None),
        Start::Resume(session) => match session.kind {
            LaunchKind::Agent {
                agent,
                native_resume_id: Some(thread),
                ..
            } if session.capabilities.resume && handle.agents_available() => {
                (session.cwd, Some(agent), Some(thread))
            }
            _ => {
                return Err(RemoteError::rejected(
                    ErrorCode::UnsupportedCapability,
                    "historical resume is unavailable for this session",
                )
                .into());
            }
        },
    };
    let message = if resume.is_some() {
        "Resume the saved conversation in a new session. Select an allowed directory."
    } else {
        "Select an allowed directory. Browsing and launch enforce its current policy."
    }
    .into();
    let mut picker = Picker {
        agent,
        resume,
        path: initial,
        loaded: None,
        entries: Vec::new(),
        next: None,
        roots: Vec::new(),
        choosing_root: false,
        selected: 0,
        editing: true,
        submitted: false,
        key: IdempotencyKey::new(),
        message,
    };
    let agents: Vec<_> = machine
        .agents
        .keys()
        .filter(|_| handle.agents_available())
        .cloned()
        .collect();
    let mut jobs: JoinSet<Result<Update, RemoteError>> = JoinSet::new();
    {
        let handle = handle.clone();
        let id = machine.id;
        jobs.spawn(async move { Ok(Update::Roots(handle.allowed_roots(id).await?)) });
    }

    let mut ticks = tokio::time::interval(Duration::from_millis(30));
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut result = None;
    loop {
        tokio::select! {
            _=ticks.tick()=>{
                if signal.load(Ordering::Acquire){break;}
                if event::poll(Duration::ZERO)? {match event::read()? {
                    Event::Key(key) if key.kind!=KeyEventKind::Release=>{
                        if key.code==KeyCode::Esc {break;}
                        if jobs.is_empty() {
                            if picker.submitted {
                                if key.code==KeyCode::Char('l') {let handle=handle.clone();let machine=machine.id;let key=picker.key;jobs.spawn(async move {let (stage,session)=handle.lookup_start(machine,key).await?;Ok(Update::Status(stage,session))});}
                            }else if picker.editing {
                                match key.code {
                                    KeyCode::F(1)=>picker.show_roots(),
                                    KeyCode::Enter=>{jobs.spawn(browse(handle.clone(),machine.id,picker.path.clone(),None,false));},
                                    KeyCode::Backspace=>{picker.path.pop();},
                                    KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL)=>picker.path.clear(),
                                    KeyCode::Char(c) if !key.modifiers.intersects(KeyModifiers::CONTROL|KeyModifiers::ALT) && !c.is_control() && picker.path.len()+c.len_utf8()<=4096=>picker.path.push(c),
                                    _=>{},
                                }
                            }else {match key.code {
                                KeyCode::Up=>picker.selected=picker.selected.saturating_sub(1),
                                KeyCode::Down=>picker.selected=(picker.selected+1).min(picker.entries.len().saturating_sub(1)),
                                KeyCode::Char('p')=>{if picker.choosing_root{picker.path.clear();}picker.editing=true;},
                                KeyCode::F(5) if picker.resume.is_none()=>{picker.agent=match picker.agent.as_ref().and_then(|selected|agents.iter().position(|agent|agent==selected)){Some(index)=>agents.get(index+1).cloned(),None=>agents.first().cloned()};},
                                KeyCode::Enter=>{if let Some(entry)=picker.entries.get(picker.selected).filter(|entry|entry.directory){jobs.spawn(browse(handle.clone(),machine.id,entry.path.clone(),None,false));}},
                                KeyCode::F(1)=>picker.show_roots(),
                                KeyCode::Backspace if !picker.choosing_root=>{if picker.roots.contains(&picker.path){picker.show_roots();}else if let Some(parent)=Path::new(&picker.path).parent().and_then(Path::to_str){jobs.spawn(browse(handle.clone(),machine.id,parent.to_owned(),None,false));}},
                                KeyCode::Char('n') if !key.modifiers.contains(KeyModifiers::CONTROL)=>{if picker.entries.len()<4096 && let Some(cursor)=picker.next.clone(){jobs.spawn(browse(handle.clone(),machine.id,picker.path.clone(),Some(cursor),true));}},
                                KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) && picker.loaded.as_ref()==Some(&picker.path)=>{
                                    let size=terminal.terminal.size()?;
                                    let kind=picker.agent.as_ref().map(|agent|LaunchKind::Agent{agent:agent.clone(),prompt:None,native_resume_id:picker.resume.clone()}).unwrap_or_else(||LaunchKind::Pty{command:vec!["/bin/sh".into()]});
                                    let launch=Launch{key:picker.key,kind,cwd:picker.path.clone(),rows:size.height.saturating_sub(8).clamp(1,128),cols:size.width.saturating_sub(2).clamp(1,256)};
                                    picker.submitted=true;picker.message="Persisting start identity before submission. This view never automatically retries a launch.".into();
                                    let handle=handle.clone();let state=state.to_owned();let owner=machine.owner;let machine=machine.id;
                                    jobs.spawn(async move {Ok(Update::Started(handle.start(state,owner,machine,launch).await?))});
                                },
                                _=>{},
                            }}
                        }
                    },
                    Event::Paste(text) if jobs.is_empty() && picker.editing && !picker.submitted=>{for c in text.chars().filter(|c|!c.is_control()){if picker.path.len()+c.len_utf8()>4096 {break;}picker.path.push(c);}},
                    _=>{},
                }}
                terminal.terminal.draw(|frame|picker.draw(frame,&machine,!jobs.is_empty()))?;
            },
            update=jobs.join_next(),if !jobs.is_empty()=>match update {
                Some(Ok(Ok(Update::Roots(roots))))=>{
                    picker.roots=roots;
                    if picker.roots.iter().any(|root| Path::new(&picker.path).starts_with(root)) {
                        picker.message="Enter browses this directory; F1 selects another allowed root.".into();
                    } else {picker.show_roots();}
                },
                Some(Ok(Ok(Update::Directory(path,mut entries,next,append))))=>{
                    if append {picker.entries.append(&mut entries);}else{picker.entries=entries;picker.selected=0;}
                    picker.choosing_root=false;picker.path=path;picker.loaded=Some(picker.path.clone());picker.next=next;picker.editing=false;
                    picker.message=if picker.resume.is_some(){"Ctrl-N resumes history in a new session. Input starts in observe mode."}else{"Ctrl-N creates a new terminal in this directory. Input starts in observe mode."}.into();
                },
                Some(Ok(Ok(Update::Started(session))))=>{result=Some(session);break;},
                Some(Ok(Ok(Update::Status(stage,id))))=>{
                    picker.message=format!("Original start: {stage:?}{}; no launch was resent.",id.map(|id|format!(" · {id}")).unwrap_or_default());
                    if let Some(id)=id {let handle=handle.clone();let machine=machine.id;jobs.spawn(async move {let session=handle.session(machine,id).await?;Ok(Update::Started(session))});}
                },
                Some(Ok(Ok(Update::Identities(..))))=>{},
                Some(Ok(Err(error)))=>{picker.message=if picker.submitted {format!("{:?} ({:?}): {}. Original identity retained; l looks it up.",error.code,error.outcome,error.message)}else{format!("{:?}: {}. Edit the path with p or retry browsing.",error.code,error.message)};},
                _=>picker.message="Request stopped; original start identity is retained if submitted.".into(),
            }
        }
    }
    jobs.abort_all();
    while jobs.join_next().await.is_some() {}
    Ok(result)
}

/// Reopening a journal offers lookup only. It cannot submit any stored launch.
pub(crate) async fn recover(
    terminal: &mut TerminalGuard,
    signal: &AtomicBool,
    handle: LiveHandle,
    machine: Machine,
    state: &Path,
) -> Result<Option<Session>> {
    let mut jobs: JoinSet<Result<Update, RemoteError>> = JoinSet::new();
    let reader = handle.clone();
    let state = state.to_owned();
    let owner = machine.owner;
    let id = machine.id;
    let initial_state = state.clone();
    jobs.spawn(async move {
        let (records, next) = reader
            .start_identities_page(initial_state, owner, id, None, 64)
            .await?;
        Ok(Update::Identities(records, next))
    });
    let mut next_page = None;
    let mut identities: Vec<RemoteStartIdentity> = Vec::new();
    let mut selected = 0usize;
    let mut message = "Loading private start identities…".to_owned();
    let mut ticks = tokio::time::interval(Duration::from_millis(30));
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut result = None;
    loop {
        tokio::select! {
            _=ticks.tick()=>{
                if signal.load(Ordering::Acquire){break;}
                if event::poll(Duration::ZERO)? && let Event::Key(key)=event::read()? && key.kind!=KeyEventKind::Release {
                    match key.code {
                        KeyCode::Esc=>break,
                        KeyCode::Up=>selected=selected.saturating_sub(1),
                        KeyCode::Down=>selected=(selected+1).min(identities.len().saturating_sub(1)),
                        KeyCode::Char('n')|KeyCode::Home if jobs.is_empty()=>{
                            let after=if key.code==KeyCode::Home{None}else if let Some(next)=next_page{Some(next)}else{continue;};
                            let reader=handle.clone();let state=state.clone();
                            jobs.spawn(async move {let(records,next)=reader.start_identities_page(state,owner,id,after,64).await?;Ok(Update::Identities(records,next))});
                        },
                        KeyCode::Enter if jobs.is_empty()=>{if let Some(identity)=identities.get(selected){let key=identity.key;let handle=handle.clone();jobs.spawn(async move {let (stage,session)=handle.lookup_start(id,key).await?;Ok(Update::Status(stage,session))});}},
                        _=>{},
                    }
                }
                terminal.terminal.draw(|frame|{
                    let areas=Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(3),Constraint::Length(4)]).split(frame.area());
                    let items:Vec<_>=identities.iter().map(|identity|ListItem::new(identity.key.to_string())).collect();
                    frame.render_stateful_widget(List::new(items).highlight_symbol("› ").block(Block::default().borders(Borders::ALL).title(" Persisted start identities · lookup only ")),areas[0],&mut ListState::default().with_selected((!identities.is_empty()).then_some(selected)));
                    frame.render_widget(Paragraph::new(format!("{}\n↑/↓ select · Enter looks up original start · n next · Home first · Esc returns\nAn identity does not prove launch. No launch is resent.",clean(&message))).wrap(Wrap{trim:false}).block(Block::default().borders(Borders::TOP)),areas[1]);
                })?;
            },
            update=jobs.join_next(),if !jobs.is_empty()=>match update {
                Some(Ok(Ok(Update::Identities(records,next))))=>{next_page=next;selected=0;identities=records;message=format!("{} identities for this gateway, owner and machine.",identities.len());},
                Some(Ok(Ok(Update::Status(stage,session))))=>{message=format!("Original start: {stage:?}; no launch resent.");if let Some(session)=session {let handle=handle.clone();jobs.spawn(async move {let session=handle.session(id,session).await?;if session.owner!=owner{return Err(RemoteError::rejected(ErrorCode::Forbidden,"session owner changed"));}Ok(Update::Started(session))});}},
                Some(Ok(Ok(Update::Started(session))))=>{result=Some(session);break;},
                Some(Ok(Err(error)))=>message=format!("{:?}: {}",error.code,error.message),
                _=>message="Journal lookup stopped; no launch resent.".into(),
            }
        }
    }
    jobs.abort_all();
    while jobs.join_next().await.is_some() {}
    Ok(result)
}
