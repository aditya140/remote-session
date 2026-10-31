//! Cross-machine work inbox. Each page owns and closes its observation connection.
use crate::{app::TerminalGuard, display::multiline as clean};
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use qgenie_remote_client::{
    ApprovalSummary, GatewayClient, UserCredentials, WorkAcknowledgements, WorkCursor, WorkItem,
    WorkPage,
};
use qgenie_remote_proto::{ErrorCode, RemoteError};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::{task::JoinSet, time::Instant};

pub(crate) enum Selection {
    Session(Box<WorkItem>),
    Approvals,
}
async fn page(
    api: GatewayClient,
    credentials: UserCredentials,
    after: Option<WorkCursor>,
) -> Result<WorkPage, RemoteError> {
    let machines = api.machines(&credentials.current()?).await?;
    let mut client = api.connect_authenticated(&credentials).await?;
    let handle = client.handle();
    let scan = async {
        let approvals = match handle.approval_summary().await {
            Ok(summary) => summary,
            Err(error)
                if matches!(
                    error.code,
                    ErrorCode::UnsupportedCapability | ErrorCode::Overloaded | ErrorCode::Internal
                ) =>
            {
                ApprovalSummary::default()
            }
            Err(error) => return Err(error),
        };
        handle.work_page(&machines, after, &approvals).await
    };
    let result = crate::remote_dialog::drain(Some(&mut client), scan).await;
    client.close().await;
    result
}

pub(crate) async fn run(
    terminal: &mut TerminalGuard,
    signal: &AtomicBool,
    api: &GatewayClient,
    credentials: &UserCredentials,
    acknowledged: &mut WorkAcknowledgements,
) -> Result<Option<Selection>> {
    let mut jobs = JoinSet::new();
    jobs.spawn(page(api.clone(), credentials.clone(), None));
    let mut current: Option<WorkPage> = None;
    let mut selected = 0usize;
    let mut attention_only = true;
    let mut message =
        "Scanning owner work across machines. Partial coverage never means an empty inbox."
            .to_owned();
    let mut observed = Instant::now();
    let mut first_page = true;
    let mut ticks = tokio::time::interval(Duration::from_millis(30));
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let result = loop {
        tokio::select! {
            _ = ticks.tick() => {
                if signal.load(Ordering::Acquire) { break None; }
                let visible: Vec<_> = current.as_ref().into_iter().flat_map(|page| page.items.iter()).filter(|item| !attention_only || item.needs_attention(acknowledged.contains(item))).collect();
                selected = selected.min(visible.len().saturating_sub(1));
                if event::poll(Duration::ZERO)? && let Event::Key(key) = event::read()? && key.kind != KeyEventKind::Release {
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('q') => break None,
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {signal.store(true, Ordering::Release); break None;},
                        KeyCode::Up => selected = selected.saturating_sub(1),
                        KeyCode::Down => selected = (selected+1).min(visible.len().saturating_sub(1)),
                        KeyCode::Char('!') => { attention_only = !attention_only; selected = 0; },
                        KeyCode::Char('a') => break Some(Selection::Approvals),
                        KeyCode::Enter if jobs.is_empty() => if let Some(item) = visible.get(selected) { break Some(Selection::Session(Box::new((*item).clone()))); },
                        KeyCode::Char('x') => if let Some(item) = visible.get(selected) {
                            message = if acknowledged.acknowledge(item) { "Completion acknowledged for this TUI instance. Approvals and failures remain visible." } else { "Only an exact successful completion can be acknowledged (limit 4096)." }.into();
                        },
                        KeyCode::Char('r' | 'n') if jobs.is_empty() => {
                            let after = if key.code == KeyCode::Char('r') {None} else {current.as_ref().and_then(|page|page.next)};
                            if key.code == KeyCode::Char('r') || after.is_some() {
                                first_page = after.is_none(); current = None; selected = 0;
                                jobs.spawn(page(api.clone(), credentials.clone(), after));
                                message = "Scanning. Reads do not acquire control.".into();
                                continue;
                            }
                        },
                        _ => {},
                    }
                }
                terminal.terminal.draw(|frame| {
                    let areas = Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(4), Constraint::Length(6)]).split(frame.area());
                    let items: Vec<_> = visible.iter().map(|item| {
                        let s = &item.session;
                        let name = s.metadata.name.clone().unwrap_or_else(||s.id.to_string());
                        ListItem::new(format!("{} / {} · process {:?} · {}{} · approvals {}{}{}",
                            clean(&item.machine.name), clean(&name), s.state, item.activity.label(),
                            if item.activity_observed {""} else {" (not observed)"}, item.pending_approvals,
                            if item.approvals_complete {""} else {" (partial)"},
                            if s.recording_warning.is_some() {" · recording warning"} else {""}))
                    }).collect();
                    frame.render_stateful_widget(List::new(items).block(Block::default().borders(Borders::ALL).title(if attention_only {"Work inbox · needs attention"} else {"Work inbox · all scanned sessions"})).highlight_style(Style::default().fg(Color::Cyan)).highlight_symbol("› "), areas[0], &mut ListState::default().with_selected(Some(selected)));
                    let coverage = current.as_ref().map(|page|format!("{} sessions / {} machines on this page · {} · {} unavailable machines · approvals {} · snapshot {}s old",
                        page.items.len(), page.scanned_machines, if page.next.is_some() {"more pages"} else if first_page {"scan end"} else {"scan end; previous pages are older"}, page.unavailable_machines.len(), if page.approvals_complete {"scanned"} else {"partial"}, observed.elapsed().as_secs())).unwrap_or_else(||if jobs.is_empty(){"Coverage unknown"}else{"Loading; coverage unknown"}.into());
                    frame.render_widget(Paragraph::new(format!("{coverage}\n{}\n↑/↓ select · Enter session/history · a approvals · x acknowledge completion · ! all/attention · n next · r rescan · Esc back",clean(&message))).wrap(Wrap{trim:false}).block(Block::default().borders(Borders::TOP)), areas[1]);
                })?;
            },
            update = jobs.join_next(), if !jobs.is_empty() => {
                current = None;
                match update {
                    Some(Ok(Ok(page))) => { current=Some(page); observed=Instant::now(); message="Observations can change. Refresh before acting; approval choices use the existing owner inbox.".into(); },
                    Some(Ok(Err(error))) => message=format!("{:?}: {}. Coverage unknown; r retries observation.",error.code, clean(&error.message)),
                    _ => message="Observation interrupted. Coverage unknown; r retries.".into(),
                }
            }
        }
    };
    jobs.abort_all();
    while jobs.join_next().await.is_some() {}
    Ok(result)
}
