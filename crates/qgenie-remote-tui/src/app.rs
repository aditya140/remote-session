use anyhow::Result;
use base64::{Engine, engine::general_purpose::STANDARD};
use crossterm::{
    cursor::Show,
    event::{
        self, DisableBracketedPaste, EnableBracketedPaste, Event, KeyCode, KeyEvent, KeyEventKind,
        KeyModifiers,
    },
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use qgenie_remote_client::{Client, Controller};
use qgenie_remote_proto::*;
use ratatui::{Terminal, backend::CrosstermBackend};
use std::{
    io::{self, Stdout},
    time::{Duration, Instant},
};

pub(crate) struct Screen {
    pub id: SessionId,
    pub cursor: Counter,
    pub parser: vt100::Parser,
    pub info: SessionInfo,
}
pub(crate) enum Confirm {
    Terminate(SessionId),
    Takeover(SessionId),
}
pub(crate) struct App {
    pub status: HostStatus,
    pub sessions: Vec<SessionInfo>,
    pub selected: usize,
    pub screen: Option<Screen>,
    pub controller: Option<Controller>,
    pub message: String,
    pub confirm: Option<Confirm>,
    pub prefix: bool,
    pub new_cwd: Option<String>,
    pub approvals: bool,
    pub cwd: String,
}

pub(crate) struct TerminalGuard {
    pub terminal: Terminal<CrosstermBackend<Stdout>>,
}

pub(crate) struct SignalGuard {
    task: tokio::task::JoinHandle<()>,
    pub requested: std::sync::Arc<std::sync::atomic::AtomicBool>,
}
impl SignalGuard {
    pub fn start() -> Result<Self> {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };
        use tokio::signal::unix::{SignalKind, signal};
        let mut terminate = signal(SignalKind::terminate())?;
        let mut interrupt = signal(SignalKind::interrupt())?;
        let requested = Arc::new(AtomicBool::new(false));
        let flag = requested.clone();
        let task = tokio::spawn(async move {
            tokio::select! { _=terminate.recv()=>{}, _=interrupt.recv()=>{} }
            flag.store(true, Ordering::Release);
        });
        Ok(Self { task, requested })
    }
}
impl Drop for SignalGuard {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl TerminalGuard {
    pub fn enter() -> Result<Self> {
        enable_raw_mode()?;
        if let Err(error) = execute!(io::stdout(), EnterAlternateScreen, EnableBracketedPaste) {
            let _ = disable_raw_mode();
            return Err(error.into());
        }
        match Terminal::new(CrosstermBackend::new(io::stdout())) {
            Ok(terminal) => Ok(Self { terminal }),
            Err(error) => {
                let _ = execute!(
                    io::stdout(),
                    LeaveAlternateScreen,
                    DisableBracketedPaste,
                    Show
                );
                let _ = disable_raw_mode();
                Err(error.into())
            }
        }
    }
}
impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(
            io::stdout(),
            DisableBracketedPaste,
            LeaveAlternateScreen,
            Show
        );
    }
}

pub async fn run(client: Client, cwd: String, initial: Option<SessionId>) -> Result<()> {
    let status = client.status().await?;
    let sessions = client.sessions().await?;
    let mut app = App {
        status,
        sessions,
        selected: 0,
        screen: None,
        controller: None,
        message: "Press n to start a shell. Sessions continue after the TUI closes.".into(),
        confirm: None,
        prefix: false,
        new_cwd: None,
        approvals: false,
        cwd,
    };
    if let Some(id) = initial {
        app.attach(&client, id).await?;
    }
    let mut terminal = TerminalGuard::enter()?;
    let signals = SignalGuard::start()?;
    let mut refreshed = Instant::now();
    loop {
        if signals.requested.load(std::sync::atomic::Ordering::Acquire) {
            break;
        }
        terminal
            .terminal
            .draw(|frame| crate::view::draw(frame, &app))?;
        if event::poll(Duration::from_millis(30))? {
            match event::read()? {
                Event::Key(key) if key.kind != KeyEventKind::Release => {
                    match app.key(&client, key).await {
                        Ok(true) => break,
                        Ok(false) => {}
                        Err(error) => app.message = error.to_string(),
                    }
                }
                Event::Resize(cols, rows) => {
                    if let Some(controller) = &app.controller {
                        let (rows, cols) = dimensions(rows, cols);
                        if let Err(error) = controller.resize(rows, cols).await {
                            app.message = error.to_string();
                        }
                    }
                }
                Event::Paste(text) => {
                    if let Some(cwd) = &mut app.new_cwd {
                        if cwd.len() + text.len() < 4096 {
                            cwd.push_str(&text.replace(['\n', '\r'], ""));
                        }
                    } else if let Some(controller) = &mut app.controller {
                        let mut bytes = text.into_bytes();
                        if app
                            .screen
                            .as_ref()
                            .is_some_and(|s| s.parser.screen().bracketed_paste())
                        {
                            bytes = [b"\x1b[200~".as_slice(), &bytes, b"\x1b[201~"].concat();
                        }
                        if bytes.len() > MAX_PTY_BYTES {
                            app.message = "Paste exceeds the 32 KiB input limit.".into();
                        } else if let Err(error) = controller.input(&bytes).await {
                            app.message = error.to_string();
                        }
                    }
                }
                _ => {}
            }
        }
        if refreshed.elapsed() >= Duration::from_millis(200) {
            if let Err(error) = app.refresh(&client).await {
                app.message = error.to_string();
            }
            refreshed = Instant::now();
        }
    }
    if let Some(screen) = &app.screen {
        let _ = client.release(screen.id).await;
    }
    Ok(())
}

impl App {
    async fn attach(&mut self, client: &Client, id: SessionId) -> Result<()> {
        let attachment = client.attach(id).await?;
        let parser = match attachment.snapshot.format {
            SnapshotFormat::Vt100StateV1 => {
                vt100::Parser::from_checkpoint(&STANDARD.decode(&attachment.snapshot.data)?)?
            }
            SnapshotFormat::AgentStateV1 => {
                anyhow::bail!("use the structured agent view for this session")
            }
        };
        anyhow::ensure!(
            parser.screen().size() == (attachment.snapshot.rows, attachment.snapshot.cols),
            "snapshot dimensions do not match"
        );
        self.screen = Some(Screen {
            id,
            cursor: attachment.snapshot.seq,
            parser,
            info: attachment.session,
        });
        self.approvals = false;
        self.message =
            "Observing. Ctrl-] c: request control; Ctrl-] d: detach; Ctrl-] q: quit.".into();
        Ok(())
    }
    async fn refresh(&mut self, client: &Client) -> Result<()> {
        if let Some(screen) = &mut self.screen {
            let batch = client.replay(screen.id, screen.cursor).await?;
            let needs_snapshot = batch.truncated
                || batch
                    .events
                    .iter()
                    .any(|e| matches!(e.body, EventBody::Gap { .. }));
            if needs_snapshot {
                let id = screen.id;
                self.attach(client, id).await?;
                self.message =
                    "Earlier output was truncated or unavailable. Restored the current screen."
                        .into();
                return Ok(());
            }
            for event in batch.events {
                if event.seq <= screen.cursor {
                    continue;
                }
                if screen.cursor.0.checked_add(1) != Some(event.seq.0) {
                    let id = screen.id;
                    self.attach(client, id).await?;
                    return Ok(());
                }
                match event.body {
                    EventBody::Agent(_) => {
                        anyhow::bail!("structured agent events require the agent view")
                    }
                    EventBody::PtyOutput(data) => screen.parser.process(&STANDARD.decode(data)?),
                    EventBody::Resize { rows, cols } => {
                        screen.parser.screen_mut().set_size(rows, cols)
                    }
                    EventBody::Exited { code } => {
                        self.message = format!(
                            "Session exited with code {code}. Ctrl-] d returns to machines."
                        );
                        self.controller = None;
                    }
                    EventBody::Gap { .. } => {}
                }
                screen.cursor = event.seq;
            }
            screen.info = batch.session;
            if self.controller.is_some()
                && !screen
                    .info
                    .controller
                    .as_ref()
                    .is_some_and(|l| l.holder == client.id())
            {
                self.controller = None;
                self.message =
                    "Control expired or another client took over. Observation continues.".into();
            }
        } else {
            self.sessions = client.sessions().await?;
            self.selected = self.selected.min(self.sessions.len().saturating_sub(1));
        }
        Ok(())
    }

    async fn key(&mut self, client: &Client, key: KeyEvent) -> Result<bool> {
        if let Some(confirm) = self.confirm.take() {
            if key.code == KeyCode::Char('y') || key.code == KeyCode::Char('Y') {
                match confirm {
                    Confirm::Terminate(id) => {
                        client.terminate(id, true).await?;
                        self.message = "Termination requested.".into();
                    }
                    Confirm::Takeover(id) => {
                        self.controller = Some(client.take_control(id, true).await?);
                        self.message =
                            "Control acquired. Ctrl-] d detaches without ending the session."
                                .into();
                    }
                }
            } else {
                self.message = "Cancelled.".into();
            }
            return Ok(false);
        }
        if let Some(cwd) = &mut self.new_cwd {
            match key.code {
                KeyCode::Esc => self.new_cwd = None,
                KeyCode::Backspace => {
                    cwd.pop();
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL) && cwd.len() < 4096 =>
                {
                    cwd.push(c)
                }
                KeyCode::Enter => {
                    let cwd = self.new_cwd.take().unwrap();
                    let (cols, rows) = crossterm::terminal::size()?;
                    let (rows, cols) = dimensions(rows, cols);
                    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
                    let spec = StartSpec {
                        key: IdempotencyKey::new(),
                        command: vec![shell],
                        cwd,
                        rows,
                        cols,
                    };
                    let session = client.start(spec).await?;
                    self.attach(client, session.id).await?;
                    self.controller = Some(client.take_control(session.id, false).await?);
                    self.message =
                        "Shell started. Ctrl-] d detaches; Ctrl-] q closes the TUI.".into();
                }
                _ => {}
            }
            return Ok(false);
        }
        if self.screen.is_some() {
            let id = self.screen.as_ref().unwrap().id;
            if self.prefix {
                self.prefix = false;
                match key.code {
                    KeyCode::Char('d') => {
                        client.release(id).await?;
                        self.controller = None;
                        self.screen = None;
                        self.message = "Detached. The session continues on this machine.".into();
                        self.refresh(client).await?;
                    }
                    KeyCode::Char('q') => return Ok(true),
                    KeyCode::Char('c') => match client.take_control(id, false).await {
                        Ok(control) => {
                            self.controller = Some(control);
                            self.message = "Control acquired. Input goes to the shell.".into();
                        }
                        Err(error) if matches!(error.code, ErrorCode::ControlHeld) => {
                            self.confirm = Some(Confirm::Takeover(id))
                        }
                        Err(error) => return Err(error.into()),
                    },
                    KeyCode::Char('o') => {
                        client.release(id).await?;
                        self.controller = None;
                        self.message =
                            "Observing. Owner approvals remain available for supported agents."
                                .into();
                    }
                    KeyCode::Char('k') => self.confirm = Some(Confirm::Terminate(id)),
                    KeyCode::Char('i') => {
                        if let Some(control) = &self.controller {
                            control.interrupt().await?;
                        }
                    }
                    _ => {}
                }
                return Ok(false);
            }
            if is_command_prefix(key) {
                self.prefix = true;
                self.message="Ctrl-] commands: d detach · c control · o observe · i interrupt · k terminate · q quit".into();
            } else if let Some(control) = &mut self.controller {
                let application_cursor = self
                    .screen
                    .as_ref()
                    .is_some_and(|s| s.parser.screen().application_cursor());
                let bytes = encode_key(key, application_cursor);
                if !bytes.is_empty() {
                    control.input(&bytes).await?;
                }
            }
            return Ok(false);
        }
        match key.code {
            KeyCode::Char('q') | KeyCode::Esc => return Ok(true),
            KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return Ok(true),
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected = (self.selected + 1).min(self.sessions.len().saturating_sub(1))
            }
            KeyCode::Up | KeyCode::Char('k') => self.selected = self.selected.saturating_sub(1),
            KeyCode::Char('n') => {
                self.new_cwd = Some(self.cwd.clone());
                self.approvals = false;
            }
            KeyCode::Enter | KeyCode::Char('a') | KeyCode::Char('o') => {
                if let Some(session) = self.sessions.get(self.selected) {
                    self.attach(client, session.id).await?;
                }
            }
            KeyCode::Char('x') => {
                if let Some(session) = self.sessions.get(self.selected) {
                    self.confirm = Some(Confirm::Terminate(session.id));
                }
            }
            KeyCode::Char('p') => self.approvals = !self.approvals,
            _ => {}
        }
        Ok(false)
    }
}

fn dimensions(rows: u16, cols: u16) -> (u16, u16) {
    (
        rows.saturating_sub(8).clamp(1, 128),
        cols.saturating_sub(2).clamp(1, 256),
    )
}
pub(crate) fn is_command_prefix(key: KeyEvent) -> bool {
    // Traditional terminals encode Ctrl-] as 0x1d; crossterm represents that
    // byte as Ctrl-5. Enhanced keyboard protocols can report Ctrl-] directly.
    matches!(key.code, KeyCode::Char(']' | '5')) && key.modifiers.contains(KeyModifiers::CONTROL)
}
pub(crate) fn encode_key(key: KeyEvent, application_cursor: bool) -> Vec<u8> {
    let bytes = match key.code {
        KeyCode::Char(c) if key.modifiers.contains(KeyModifiers::CONTROL) && c.is_ascii() => {
            vec![match c {
                '4' => 28,
                '5' => 29,
                '6' => 30,
                '7' => 31,
                _ => (c.to_ascii_lowercase() as u8) & 0x1f,
            }]
        }
        KeyCode::Char(c) => c.to_string().into_bytes(),
        KeyCode::Enter => vec![b'\r'],
        KeyCode::Backspace => vec![127],
        KeyCode::Tab => vec![b'\t'],
        KeyCode::Esc => vec![27],
        KeyCode::Up => if application_cursor {
            b"\x1bOA"
        } else {
            b"\x1b[A"
        }
        .to_vec(),
        KeyCode::Down => if application_cursor {
            b"\x1bOB"
        } else {
            b"\x1b[B"
        }
        .to_vec(),
        KeyCode::Right => if application_cursor {
            b"\x1bOC"
        } else {
            b"\x1b[C"
        }
        .to_vec(),
        KeyCode::Left => if application_cursor {
            b"\x1bOD"
        } else {
            b"\x1b[D"
        }
        .to_vec(),
        KeyCode::Home => b"\x1b[H".to_vec(),
        KeyCode::End => b"\x1b[F".to_vec(),
        KeyCode::Delete => b"\x1b[3~".to_vec(),
        KeyCode::PageUp => b"\x1b[5~".to_vec(),
        KeyCode::PageDown => b"\x1b[6~".to_vec(),
        _ => Vec::new(),
    };
    if key.modifiers.contains(KeyModifiers::ALT) {
        [vec![27], bytes].concat()
    } else {
        bytes
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn encodes_shell_control_keys_and_unicode_without_losing_bytes() {
        assert_eq!(
            encode_key(
                KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
                false
            ),
            [3]
        );
        assert_eq!(
            encode_key(KeyEvent::new(KeyCode::Char('é'), KeyModifiers::NONE), false),
            "é".as_bytes()
        );
        assert_eq!(
            encode_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), false),
            b"\x1b[A"
        );
        assert!(is_command_prefix(KeyEvent::new(
            KeyCode::Char('5'),
            KeyModifiers::CONTROL
        )));
        assert!(is_command_prefix(KeyEvent::new(
            KeyCode::Char(']'),
            KeyModifiers::CONTROL
        )));
        assert!(!is_command_prefix(KeyEvent::new(
            KeyCode::Char('5'),
            KeyModifiers::NONE
        )));
        assert_eq!(
            encode_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE), true),
            b"\x1bOA"
        );
    }
}
