//! Native remote terminal view; accepted processes outlive this view and its jobs.
use crate::display::single_line as safe;
use crate::{
    app::{TerminalGuard, encode_key, is_command_prefix},
    view::draw_terminal,
};
use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use qgenie_remote_client::{
    GatewayClient, LiveClient, LiveFrame, LiveHandle, OutputCursor, OutputUpdate, ReadPlan,
    RemoteAttachment, RemoteController, UserCredentials, observation_can_reconnect,
};
use qgenie_remote_proto::{
    wire::{AgentEvent, LaunchKind, Message, Session},
    *,
};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    widgets::{Block, Borders, Paragraph},
};
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::{task::JoinSet, time::Instant};

#[derive(Clone, Copy)]
enum Action {
    Take(bool),
    Release,
    Interrupt,
    Resize(u16, u16),
    Renew,
    Terminate,
}
enum Mutation {
    Control(RemoteController),
    Retained(RemoteController, Result<(), RemoteError>, bool),
    Released(Result<(), RemoteError>),
    Terminated(Result<(), RemoteError>),
}
struct Pane {
    agent: bool,
    agent_view: Option<crate::remote_agent::AgentView>,
    agent_scroll: usize,
    editor: Vec<u8>,
    steer: bool,
    output: OutputCursor,
    parser: Option<vt100::Parser>,
    controller: Option<RemoteController>,
    owned: bool,
    connected: bool,
    stopped: bool,
    state: SessionState,
    recording_warning: Option<String>,
    inputs: VecDeque<QueuedInput>,
    bytes: usize,
    action: Option<Action>,
    confirm: Option<bool>,
    message: String,
    next_read: Instant,
    next_renew: Instant,
    force_capture: bool,
    read_paused: bool,
}
struct QueuedInput {
    bytes: Vec<u8>,
    steer: bool,
}
enum Checkpoint {
    Terminal(Box<vt100::Parser>),
    Agent(AgentState),
}
impl Pane {
    fn metadata(&mut self, current: &mut Session, session: &Session) {
        use SessionState::*;
        // Start replies and inventory notifications travel independently. A late
        // startup snapshot must not replace capabilities from a running session.
        // Running and Idle can alternate; current capability changes still apply.
        if matches!(
            (&self.state, &session.state),
            (Launching, Accepted)
                | (Running | Idle, Accepted | Launching)
                | (
                    Exited | Failed | Lost | OutcomeUnknown,
                    Accepted | Launching | Running | Idle
                )
        ) {
            return;
        }
        *current = session.clone();
        if !self.stopped
            || matches!(
                session.state,
                SessionState::Exited
                    | SessionState::Failed
                    | SessionState::Lost
                    | SessionState::OutcomeUnknown
            )
        {
            self.state = session.state.clone();
        }
        self.recording_warning = session.recording_warning.clone();
        if matches!(
            session.state,
            SessionState::Exited
                | SessionState::Failed
                | SessionState::Lost
                | SessionState::OutcomeUnknown
        ) {
            self.stopped = true;
            self.owned = false;
            self.controller = None;
            self.action = None;
            self.clear_input();
            if matches!(
                session.state,
                SessionState::Lost | SessionState::OutcomeUnknown
            ) {
                self.read_paused = true;
                self.message = "Native lifetime is unconfirmed. Displayed output may have an unknown tail; recorded history remains available.".into();
            }
        }
    }
    fn clear_input(&mut self) {
        for mut input in self.inputs.drain(..) {
            input.bytes.fill(0);
        }
        self.editor.fill(0);
        self.editor.clear();
        self.bytes = 0;
    }
    fn queue(&mut self, bytes: Vec<u8>) {
        if bytes.is_empty() {
            return;
        }
        if !self.owned || !self.connected || self.stopped {
            self.message = if self.controller.as_ref().is_some_and(|control|control.input_uncertain()) {"Previous input is uncertain; unsent input was discarded. F2 explicitly reacquires control."}else{"Observe only. F2 requests control."}.into();
            return;
        }
        if bytes.len() > MAX_PTY_BYTES
            || self.inputs.len() >= 256
            || self.bytes + bytes.len() > 64 * 1024
        {
            self.clear_input();
            self.owned = false;
            self.message =
                "Unsent input exceeded its limit and was discarded. F2 reacquires control.".into();
            return;
        }
        self.bytes += bytes.len();
        self.inputs.push_back(QueuedInput {
            bytes,
            steer: self.agent && self.steer,
        });
    }
    fn paint_update(
        &mut self,
        update: OutputUpdate,
        checkpoint: Option<Checkpoint>,
    ) -> Result<(), RemoteError> {
        if update.snapshot.is_some() {
            match checkpoint {
                Some(Checkpoint::Terminal(parser)) => self.parser = Some(*parser),
                Some(Checkpoint::Agent(state)) => {
                    self.agent_view = Some(crate::remote_agent::AgentView::new(state))
                }
                None => {}
            }
        }
        for item in update.events {
            if let Some(view) = &mut self.agent_view {
                let ended = matches!(item.event, AgentEvent::Exited { .. });
                view.push(qgenie_remote_proto::wire::SequencedEvent {
                    seq: item.seq,
                    at: item.at.ok_or_else(|| {
                        RemoteError::rejected(
                            ErrorCode::InvalidFrame,
                            "structured event lacks capture time",
                        )
                    })?,
                    event: item.event,
                })?;
                if ended {
                    self.state = SessionState::Exited;
                    self.stopped = true;
                    self.owned = false;
                    self.controller = None;
                    self.clear_input();
                }
                continue;
            }
            match item.event {
                AgentEvent::PtyOutput { data } => {
                    let data = STANDARD.decode(data).map_err(|_| {
                        RemoteError::rejected(
                            ErrorCode::InvalidFrame,
                            "invalid native output encoding",
                        )
                    })?;
                    if let Some(parser) = &mut self.parser {
                        parser.process(&data);
                    }
                }
                AgentEvent::Resize { rows, cols } => {
                    validate_size(rows, cols)?;
                    if let Some(parser) = &mut self.parser {
                        parser.screen_mut().set_size(rows, cols);
                    }
                }
                AgentEvent::Exited { code } => {
                    self.state = SessionState::Exited;
                    self.stopped = true;
                    self.owned = false;
                    self.controller = None;
                    self.clear_input();
                    self.message = format!(
                        "Native process exited ({code}). Recorded history remains available."
                    );
                }
                AgentEvent::Gap { reason, .. } => {
                    self.message = format!("Output gap: {reason}. Requesting a checkpoint.");
                }
                _ => {}
            }
        }
        if let Some(view) = &mut self.agent_view {
            view.rebuild();
        }
        Ok(())
    }
    fn attachment(&mut self, result: Result<RemoteAttachment, RemoteError>) {
        let result = (|| {
            let attachment = result?;
            if let RemoteAttachment::Replay(page) = &attachment
                && page
                    .events
                    .iter()
                    .any(|event| matches!(event.event, AgentEvent::Exited { .. }))
            {
                self.stopped = true;
                self.owned = false;
                self.controller = None;
                self.clear_input();
                self.message = "Native process exited; recorded history remains available.".into();
            }

            let checkpoint = if let RemoteAttachment::Snapshot(snapshot) = &attachment {
                if self.agent {
                    Some(Checkpoint::Agent(snapshot.agent_state()?))
                } else {
                    if snapshot.format != SnapshotFormat::Vt100StateV1 {
                        return Err(RemoteError::rejected(
                            ErrorCode::InvalidFrame,
                            "terminal received a structured checkpoint",
                        ));
                    }
                    let parser = vt100::Parser::from_checkpoint(&snapshot.data).map_err(|_| {
                        RemoteError::rejected(
                            ErrorCode::HistoryTruncated,
                            "native checkpoint is invalid",
                        )
                    })?;
                    if parser.screen().size() != (snapshot.rows, snapshot.cols) {
                        return Err(RemoteError::rejected(
                            ErrorCode::InvalidFrame,
                            "native checkpoint dimensions differ",
                        ));
                    }
                    Some(Checkpoint::Terminal(Box::new(parser)))
                }
            } else {
                None
            };
            let update = self.output.attachment(attachment)?;
            self.paint_update(update, checkpoint)
        })();
        if let Err(error) = result {
            self.message = format!("{:?}: {} · F8 refreshes output.", error.code, error.message);
            self.read_paused = true;
        }
    }
    fn draw(&self, frame: &mut ratatui::Frame<'_>, session: &Session) {
        let areas = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(2),
                Constraint::Min(2),
                Constraint::Length(4),
            ])
            .split(frame.area());
        frame.render_widget(
            Paragraph::new(format!(
                "Remote {} · {} · {:?} · {}{}",
                if self.agent { "agent" } else { "terminal" },
                if self.owned {
                    "CONTROLLING"
                } else {
                    "OBSERVING"
                },
                self.state,
                safe(&session.cwd),
                if self
                    .agent_view
                    .as_ref()
                    .is_some_and(|view| view.state.unconfirmed_tail)
                {
                    " · UNCONFIRMED TAIL"
                } else if self
                    .agent_view
                    .as_ref()
                    .is_some_and(|view| view.state.history_truncated)
                {
                    " · PARTIAL HISTORY"
                } else {
                    ""
                },
            ))
            .block(Block::default().borders(Borders::BOTTOM)),
            areas[0],
        );
        let block = Block::default()
            .borders(Borders::ALL)
            .title(" Live output ");
        let inner = block.inner(areas[1]);
        frame.render_widget(block, areas[1]);
        if let Some(view) = &self.agent_view {
            let count = usize::from(inner.height);
            let end = view.lines.len().saturating_sub(self.agent_scroll);
            let start = end.saturating_sub(count);
            frame.render_widget(Paragraph::new(view.lines[start..end].join("\n")), inner);
        } else if let Some(parser) = &self.parser {
            draw_terminal(frame, parser.screen(), inner, self.owned);
        } else {
            frame.render_widget(Paragraph::new("Reading native checkpoint…"), inner);
        }
        let recording = self
            .recording_warning
            .as_deref()
            .map(safe)
            .unwrap_or_else(|| "Recording enabled · available history can contain gaps".into());
        let hint = if self.agent {
            format!(
                "{}: {} ▌ · Enter sends · F5 prompt/steer · F2 control · F3 release · F4 interrupt · F6 terminate · F10 detach",
                if self.steer { "Steer" } else { "Prompt" },
                safe(&String::from_utf8_lossy(&self.editor))
            )
        } else {
            "F2 control · F3 release · F4 interrupt · F6 terminate · F8 refresh · F10/Ctrl-] detach"
                .into()
        };
        frame.render_widget(
            Paragraph::new(format!("{}\n{}\n{hint}", safe(&self.message), recording))
                .block(Block::default().borders(Borders::TOP)),
            areas[2],
        );
    }
}
async fn mutation(
    handle: LiveHandle,
    machine: MachineId,
    session: SessionId,
    action: Action,
    controller: Option<RemoteController>,
) -> Result<Mutation, RemoteError> {
    match action {
        Action::Take(takeover) => Ok(Mutation::Control(
            handle.take_control(machine, session, takeover).await?,
        )),
        Action::Terminate => Ok(Mutation::Terminated(
            handle
                .terminate_current(machine, session, true, false)
                .await,
        )),
        _ => {
            let mut controller = controller.ok_or_else(|| {
                RemoteError::rejected(ErrorCode::StaleGeneration, "no current controller")
            })?;
            if matches!(action, Action::Release) {
                return Ok(Mutation::Released(controller.release().await));
            }
            let renew = matches!(action, Action::Renew);
            let result = match action {
                Action::Interrupt => controller.interrupt().await,
                Action::Resize(rows, cols) => controller.resize(rows, cols).await,
                Action::Renew => controller.renew().await,
                _ => unreachable!(),
            };
            Ok(Mutation::Retained(controller, result, renew))
        }
    }
}
pub(crate) async fn run(
    terminal: &mut TerminalGuard,
    signal: &AtomicBool,
    client: &mut LiveClient,
    mut session: Session,
    api: &GatewayClient,
    credentials: &UserCredentials,
) -> Result<()> {
    let agent = matches!(session.kind, LaunchKind::Agent { .. });
    let machine = session.machine;
    let id = session.id;
    let mut handle = client.handle();
    let mut pane = Pane {
        agent,
        agent_view: agent.then(|| crate::remote_agent::AgentView::new(AgentState::default())),
        agent_scroll: 0,
        editor: Vec::new(),
        steer: false,
        output: OutputCursor::checkpointed(machine, id),
        parser: None,
        controller: None,
        owned: false,
        connected: true,
        stopped: matches!(
            session.state,
            SessionState::Exited
                | SessionState::Failed
                | SessionState::Lost
                | SessionState::OutcomeUnknown
        ),
        state: session.state.clone(),
        recording_warning: session.recording_warning.clone(),
        inputs: VecDeque::new(),
        bytes: 0,
        action: None,
        confirm: None,
        message: if handle.approvals_available() {
            "Observe only. F2 requests control; F7 reviews owner approvals; F10 detaches."
        } else {
            "Observe only. F2 requests control; F10 detaches."
        }
        .into(),
        next_read: Instant::now(),
        next_renew: Instant::now(),
        force_capture: false,
        read_paused: false,
    };
    let mut reads = JoinSet::new();
    let mut recovery: JoinSet<Result<LiveClient, RemoteError>> = JoinSet::new();
    let mut metadata = JoinSet::new();
    let mut next_metadata = Instant::now();
    let mut mutations: JoinSet<Result<Mutation, RemoteError>> = JoinSet::new();
    let mut ticks = tokio::time::interval(Duration::from_millis(30));
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _=ticks.tick()=>{
                if signal.load(Ordering::Acquire){break;}
                if !handle.session_changes_available() && metadata.is_empty() && pane.connected && Instant::now()>=next_metadata {
                    let handle=handle.clone();metadata.spawn(async move {handle.session(machine,id).await});next_metadata=Instant::now()+Duration::from_secs(3);
                }
                if event::poll(Duration::ZERO)? {match event::read()? {
                    Event::Key(key) if key.kind!=KeyEventKind::Release=>{
                        if key.code==KeyCode::F(10)||is_command_prefix(key){break;}
                        if let Some(terminate)=pane.confirm {
                            match key.code {KeyCode::Char('y')=>{pane.confirm=None;pane.action=Some(if terminate {Action::Terminate}else{Action::Take(true)});},KeyCode::Char('n')|KeyCode::Esc=>{pane.confirm=None;pane.message="Action cancelled.".into();},_=>{}}
                        }else {match key.code {
                            KeyCode::F(2) if !pane.owned && pane.connected && !pane.stopped && session.capabilities.input=>{pane.action=Some(Action::Take(false));pane.clear_input();},
                            KeyCode::F(3)=>{pane.clear_input();pane.action=Some(Action::Release);},
                            KeyCode::F(4) if pane.owned && session.capabilities.interrupt=>pane.action=Some(Action::Interrupt),
                            KeyCode::F(5) if pane.agent && session.capabilities.steer=>{pane.steer = !pane.steer;},
                            KeyCode::PageUp if pane.agent=>{pane.agent_scroll=(pane.agent_scroll+10).min(pane.agent_view.as_ref().map_or(0,|view|view.lines.len().saturating_sub(1)));},
                            KeyCode::PageDown if pane.agent=>{pane.agent_scroll=pane.agent_scroll.saturating_sub(10);},
                            KeyCode::Home if pane.agent=>{pane.agent_scroll=pane.agent_view.as_ref().map_or(0,|view|view.lines.len().saturating_sub(1));},
                            KeyCode::End if pane.agent=>{pane.agent_scroll=0;},
                            KeyCode::F(6) if pane.connected && !matches!(pane.state,SessionState::Exited|SessionState::Failed) && session.capabilities.terminate=>{pane.confirm=Some(true);pane.message="Terminate this managed native process? y confirms; n cancels.".into();},
                            KeyCode::F(7) if pane.connected && handle.approvals_available() && mutations.is_empty()=>{
                                pane.clear_input();reads.abort_all();while reads.join_next().await.is_some(){}
                                pane.message=crate::remote_approvals::run(terminal,signal,client).await?;
                                pane.force_capture=true;pane.read_paused=false;pane.next_read=Instant::now();
                            },
                            KeyCode::F(8)=>{if !pane.connected && recovery.is_empty(){let api=api.clone();let credentials=credentials.clone();recovery.spawn(async move{api.reconnect_observation(&credentials).await});pane.message="Reconnecting observation.".into();}reads.abort_all();while reads.join_next().await.is_some(){}pane.read_paused=false;pane.force_capture=true;pane.next_read=Instant::now();},
                            _ if pane.agent=>{
                                if pane.owned && !pane.stopped {
                                    match key.code {
                                        KeyCode::Enter=>{let text=std::mem::take(&mut pane.editor);pane.queue(text);},
                                        KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL)=>{pane.editor.fill(0);pane.editor.clear();},
                                        KeyCode::Backspace=>{if let Some((index,_))=std::str::from_utf8(&pane.editor).ok().and_then(|text|text.char_indices().next_back()){pane.editor[index..].fill(0);pane.editor.truncate(index);}},
                                        KeyCode::Char(c) if !c.is_control() && !key.modifiers.intersects(KeyModifiers::CONTROL|KeyModifiers::ALT) && pane.editor.len()+c.len_utf8()<=32768=>{pane.editor.extend(c.to_string().as_bytes());},
                                        _=>{},
                                    }
                                } else {pane.message="Observe only. F2 requests control before entering a prompt.".into();}
                            },
                            _=>{let application=pane.parser.as_ref().is_some_and(|p|p.screen().application_cursor());pane.queue(encode_key(key,application));},
                        }}
                    },
                    Event::Paste(text) if pane.agent=>{if pane.owned && !pane.stopped && pane.editor.len()+text.len()<=32768 && !text.contains('\0'){pane.editor.extend(text.as_bytes());}else{pane.message="Paste was not accepted; control and a prompt under 32 KiB are required.".into();}},
                    Event::Paste(text)=>{let mut bytes=text.into_bytes();if pane.parser.as_ref().is_some_and(|p|p.screen().bracketed_paste()){bytes=[b"\x1b[200~".as_slice(),&bytes,b"\x1b[201~"].concat();}pane.queue(bytes);},
                    Event::Resize(cols,rows) if pane.owned && session.capabilities.resize=>pane.action=Some(Action::Resize(rows.saturating_sub(8).clamp(1,128),cols.saturating_sub(2).clamp(1,256))),
                    _=>{},
                }}
                if reads.is_empty() && pane.connected && !pane.read_paused && Instant::now()>=pane.next_read {
                    let plan=if pane.force_capture {pane.force_capture=false;ReadPlan::Capture}else{pane.output.read_plan()};
                    if !matches!(plan,ReadPlan::UnconfirmedTail) {let reader=handle.clone();let after=pane.output.cursor();reads.spawn(async move {match plan {ReadPlan::Attach{after}=>reader.attach(machine,id,after).await,ReadPlan::Capture=>reader.capture(machine,id).await,_=>reader.replay(machine,id,after).await}});}
                }
                if mutations.is_empty() && pane.connected {
                    if pane.action.is_none() && pane.owned && Instant::now()>=pane.next_renew {pane.action=Some(Action::Renew);}
                    if let Some(action)=pane.action.take() {
                        if !matches!(action,Action::Terminate) && (matches!(action,Action::Take(_)) || pane.controller.is_some()) {
                            let controller=pane.controller.take();mutations.spawn(mutation(handle.clone(),machine,id,action,controller));
                        }else if matches!(action,Action::Terminate) {mutations.spawn(mutation(handle.clone(),machine,id,action,None));}
                    }else if pane.owned && let Some(mut controller)=pane.controller.take() {
                        if let Some(mut input)=pane.inputs.pop_front(){pane.bytes-=input.bytes.len();let agent=pane.agent;mutations.spawn(async move {let result=if agent {match std::str::from_utf8(&input.bytes){Ok(text) if input.steer=>controller.steer(text).await,Ok(text)=>controller.prompt(text).await,Err(_)=>Err(RemoteError::rejected(ErrorCode::InvalidFrame,"agent prompt is not UTF-8"))}}else{controller.input(&input.bytes).await};input.bytes.fill(0);Ok(Mutation::Retained(controller,result,false))});}else{pane.controller=Some(controller);}
                    }
                }
                terminal.terminal.draw(|frame|pane.draw(frame,&session))?;
            },
            result=recovery.join_next(),if !recovery.is_empty()=>{match result {
                Some(Ok(Ok(replacement)))=>{*client=replacement;handle=client.handle();pane.connected=true;pane.read_paused=false;pane.force_capture=false;pane.next_read=Instant::now();let reader=handle.clone();metadata.spawn(async move{reader.session(machine,id).await});pane.message="Observation reconnected. F2 requests new control; previous actions were not resent.".into();},
                Some(Ok(Err(error)))=>pane.message=format!("{:?}: {}. F8 retries observation; F10 returns.",error.code,error.message),
                _=>pane.message="Recovery stopped. F8 retries observation.".into(),
            }},
            read=reads.join_next(),if !reads.is_empty()=>{match read {Some(Ok(result))=>pane.attachment(result),_=>{pane.message="Native read stopped; F8 retries this read.".into();pane.read_paused=true;}}pane.next_read=Instant::now()+if matches!(pane.output.read_plan(),ReadPlan::Idle){Duration::from_millis(250)}else{Duration::from_millis(10)};},
            result=metadata.join_next(),if !metadata.is_empty()=>{match result {
                Some(Ok(Ok(current)))=>{if current.owner==session.owner {pane.metadata(&mut session,&current);}else{pane.message="Native session is absent from current inventory; displayed output is retained.".into();}},
                Some(Ok(Err(error)))=>pane.message=format!("Session status unavailable: {:?}. Displayed output is retained.",error.code),
                _=>{},
            }},
            result=mutations.join_next(),if !mutations.is_empty()=>{match result {
                Some(Ok(Ok(Mutation::Control(controller))))=>{pane.controller=Some(controller);pane.owned = !pane.stopped;pane.next_renew=Instant::now()+Duration::from_secs(4);pane.message="Control granted.".into();if !pane.agent && session.capabilities.resize {let size=terminal.terminal.size()?;pane.action=Some(Action::Resize(size.height.saturating_sub(8).clamp(1,128),size.width.saturating_sub(2).clamp(1,256)));}},
                Some(Ok(Ok(Mutation::Retained(controller,result,renew))))=>{if let Err(error)=result {pane.owned=false;pane.clear_input();pane.action=None;pane.message=format!("{:?}: {}. Unsent input discarded; F2 explicitly reacquires control.",error.code,error.message);}else if renew{pane.next_renew=Instant::now()+Duration::from_secs(4);}
                if !pane.stopped{pane.controller=Some(controller);}},
                Some(Ok(Ok(Mutation::Released(result))))=>{pane.owned=false;pane.clear_input();pane.message=match result {Ok(())=>"Control released; observing.".into(),Err(error)=>format!("Release uncertain: {:?}; reacquire explicitly.",error.code)};},
                Some(Ok(Ok(Mutation::Terminated(result))))=>{pane.owned=false;pane.clear_input();pane.controller=None;pane.message=match result {Ok(())=>"Termination requested; waiting for native exit.".into(),Err(error)=>format!("Termination uncertain: {:?}; verify native state.",error.code)};},
                Some(Ok(Err(error))) if error.code==ErrorCode::ControlHeld=>{pane.owned=false;pane.confirm=Some(false);pane.message="Another controller owns this session. Take over? y confirms; n cancels.".into();},
                Some(Ok(Err(error)))=>{pane.owned=false;pane.clear_input();pane.action=None;pane.message=format!("{:?}: {}",error.code,error.message);},
                _=>{pane.owned=false;pane.clear_input();pane.message="Native action outcome unknown. F2 explicitly reacquires control.".into();},
            }},
            incoming=client.next(),if pane.connected=>match incoming {
                Some(event)=>{
                    if let LiveFrame::Text(envelope)=&event.frame
                        && let Message::Sessions{sessions}|Message::SessionUpdates{sessions}=&envelope.message
                        && let Some(current)=sessions.iter().find(|current|current.id==id && current.machine==machine && current.owner==session.owner) {pane.metadata(&mut session,current);}
                    if let LiveFrame::Text(envelope)=&event.frame && let Message::ApprovalRequested(request)=&envelope.message && request.session==id {pane.message="Approval pending · F7 reviews owner approvals without taking control.".into();}
                    match pane.output.frame(&event.frame){Ok(update)=>{if let Err(error)=pane.paint_update(update,None){pane.read_paused=true;pane.message=error.message;}},Err(error)=>pane.message=error.message}
                    if !matches!(pane.output.read_plan(),ReadPlan::Idle|ReadPlan::UnconfirmedTail) {pane.next_read=Instant::now();}
                },
                None=>{let error=client.disconnect_error();pane.connected=false;pane.owned=false;pane.controller=None;pane.action=None;pane.confirm=None;pane.clear_input();pane.output.disconnected();reads.abort_all();metadata.abort_all();mutations.abort_all();while reads.join_next().await.is_some(){}while metadata.join_next().await.is_some(){}while mutations.join_next().await.is_some(){}pane.message=format!("{:?}: {}. F10 returns.",error.code,error.message);if observation_can_reconnect(&error) && recovery.is_empty(){let api=api.clone();let credentials=credentials.clone();recovery.spawn(async move{api.reconnect_observation(&credentials).await});pane.message="Connection lost. Reconnecting observation; control and queued input were discarded. Previous actions may have an unknown outcome.".into();}},
            }
        }
    }
    reads.abort_all();
    recovery.abort_all();
    metadata.abort_all();
    mutations.abort_all();
    while reads.join_next().await.is_some() {}
    while recovery.join_next().await.is_some() {}
    while metadata.join_next().await.is_some() {}
    while mutations.join_next().await.is_some() {}
    pane.clear_input();
    if !signal.load(Ordering::Acquire) {
        if let Some(controller) = pane.controller.take() {
            let _ = tokio::time::timeout(Duration::from_secs(1), controller.release()).await;
        }
        let _ = handle.detach(machine, id).await;
    }
    Ok(())
}
