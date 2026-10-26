//! One-shot owner metadata/access changes, independent of a writable attachment.
use crate::app::TerminalGuard;
use crate::display::single_line as clean;
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use qgenie_remote_client::GatewayClient;
use qgenie_remote_proto::{
    OperationOutcome, RemoteError,
    rest::{AccessToken, MachinePatch},
    wire::Machine,
};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    widgets::{Block, Borders, Paragraph, Wrap},
};
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::task::JoinSet;

pub(crate) enum Action {
    Rename,
    Label,
    Revoke,
}
enum Stage {
    Name,
    Key,
    Value,
    Revoke,
    Remove,
    Sent,
}
enum Change {
    Patch(MachinePatch),
    Revoke,
}
pub(crate) async fn run(
    terminal: &mut TerminalGuard,
    signal: &AtomicBool,
    api: GatewayClient,
    token: AccessToken,
    machine: Machine,
    action: Action,
) -> Result<String> {
    let mut stage = match action {
        Action::Rename => Stage::Name,
        Action::Label => Stage::Key,
        Action::Revoke => Stage::Revoke,
    };
    let mut name = machine.name.clone();
    let mut key = String::new();
    let mut value = String::new();
    let mut message = "Changes use your current owner access.".to_owned();
    let mut submitted = false;
    let mut jobs: JoinSet<Result<(), RemoteError>> = JoinSet::new();
    let mut ticks = tokio::time::interval(Duration::from_millis(30));
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = ticks.tick() => {
                if signal.load(Ordering::Acquire) {
                    message = if submitted && !jobs.is_empty() { "Machine change submitted; completion unknown. Reconcile inventory before another change." } else { "Machine action closed." }.into();
                    break;
                }
                let mut change = None;
                if event::poll(Duration::ZERO)? && let Event::Key(input) = event::read()? && input.kind != KeyEventKind::Release {
                    if input.code == KeyCode::Char('c') && input.modifiers.contains(KeyModifiers::CONTROL) { signal.store(true, Ordering::Release); continue; }
                    if input.code == KeyCode::Esc {
                        if jobs.is_empty() { if !submitted { message = "Machine change cancelled.".into(); } break; }
                        message = "Waiting for the original request; it will not be resent.".into();
                    } else if !submitted {
                        match (&stage, input.code) {
                            (Stage::Name, KeyCode::Enter) if !name.is_empty() => change = Some(Change::Patch(MachinePatch { name: Some(name.clone()), labels: None })),
                            (Stage::Key, KeyCode::Enter) if !key.is_empty() => { value = machine.labels.get(&key).cloned().unwrap_or_default(); stage = Stage::Value; },
                            (Stage::Value, KeyCode::Enter) => {
                                let mut labels = machine.labels.clone(); labels.insert(key.clone(), value.clone());
                                change = Some(Change::Patch(MachinePatch { name: None, labels: Some(labels) }));
                            },
                            (Stage::Value, KeyCode::Char('d')) if input.modifiers.contains(KeyModifiers::CONTROL) => {
                                if machine.labels.contains_key(&key) { stage = Stage::Remove; } else { message = "This label does not exist; Esc cancels.".into(); }
                            },
                            (Stage::Revoke, KeyCode::Char('y')) => change = Some(Change::Revoke),
                            (Stage::Remove, KeyCode::Char('y')) => { let mut labels = machine.labels.clone(); labels.remove(&key); change = Some(Change::Patch(MachinePatch { name: None, labels: Some(labels) })); },
                            (Stage::Revoke | Stage::Remove, KeyCode::Char('n')) => { message = "Machine change cancelled.".into(); break; },
                            (Stage::Name | Stage::Key | Stage::Value, code) => {
                                let (text, limit) = match stage { Stage::Name => (&mut name, 128), Stage::Key => (&mut key, 64), _ => (&mut value, 128) };
                                match code {
                                    KeyCode::Backspace => { text.pop(); },
                                    KeyCode::Char('u') if input.modifiers.contains(KeyModifiers::CONTROL) => text.clear(),
                                    KeyCode::Char(c) if !input.modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) && !c.is_control() && text.len() + c.len_utf8() <= limit => text.push(c),
                                    _ => {},
                                }
                            },
                            _ => {},
                        }
                    } else if input.code == KeyCode::Enter && jobs.is_empty() { break; }
                }
                if let Some(change) = change {
                    // No navigation, cancellation or reconnect can submit this view twice.
                    submitted = true; stage = Stage::Sent; message = "One request submitted. Closing does not retry or undo it.".into();
                    let api = api.clone(); let token = token.clone(); let id = machine.id;
                    jobs.spawn(async move { match change {
                        Change::Patch(patch) => api.patch_machine(&token, id, patch).await.map(|_| ()),
                        Change::Revoke => api.revoke_machine(&token, id, true).await,
                    }});
                }
                terminal.terminal.draw(|frame| {
                    let areas = Layout::default().direction(Direction::Vertical).constraints([Constraint::Length(4), Constraint::Min(3), Constraint::Length(4)]).split(frame.area());
                    let (title, instructions) = match stage {
                        Stage::Name => ("Rename machine", "Enter saves · Ctrl-U clears · Esc cancels"),
                        Stage::Key => ("Edit machine label · key", "Enter selects a key · Ctrl-U clears · Esc cancels"),
                        Stage::Value => ("Edit machine label · value", "Enter saves · Ctrl-D removes an existing label · Esc cancels"),
                        Stage::Revoke => ("Revoke machine access", "y confirms revocation · n or Esc cancels"),
                        Stage::Remove => ("Remove machine label", "y confirms removal · n or Esc cancels"),
                        Stage::Sent => ("Machine change submitted once", "Enter or Esc returns after the result · no automatic retry"),
                    };
                    frame.render_widget(Paragraph::new(format!("{title}\n{} · {} · {} {}", clean(&machine.name), clean(&machine.hostname), clean(&machine.os), clean(&machine.arch))).block(Block::default().borders(Borders::ALL)), areas[0]);
                    let contents = match stage {
                        Stage::Name => format!("Name: {} ▌", clean(&name)),
                        Stage::Key | Stage::Value | Stage::Remove => format!("Key: {}{}\nValue: {}{}\n\nCurrent labels:\n{}", clean(&key), if matches!(stage, Stage::Key) { " ▌" } else { "" }, clean(&value), if matches!(stage, Stage::Value) { " ▌" } else { "" }, machine.labels.iter().map(|(key,value)| format!("{} = {}", clean(key),clean(value))).collect::<Vec<_>>().join("\n")),
                        Stage::Revoke => "Revoke this machine's remote access?\nNative sessions continue on the device. Recorded history keeps its access and retention rules.".into(),
                        Stage::Sent => clean(&message),
                    };
                    frame.render_widget(Paragraph::new(contents).wrap(Wrap { trim: false }).block(Block::default().borders(Borders::ALL)), areas[1]);
                    frame.render_widget(Paragraph::new(format!("{}{}\n{instructions}", if jobs.is_empty() { "" } else { "Waiting · " }, clean(&message))).wrap(Wrap { trim: false }).block(Block::default().borders(Borders::TOP)), areas[2]);
                })?;
            },
            result = jobs.join_next(), if !jobs.is_empty() => {
                message = match result {
                    Some(Ok(Ok(()))) => match action { Action::Revoke => "Machine remote access revoked. Native sessions remain on the device. Reconcile inventory on return.".into(), _ => "Machine metadata saved. Inventory refreshes on return.".into() },
                    Some(Ok(Err(error))) if matches!(error.outcome, OperationOutcome::Unknown) => "Outcome unknown. Reconcile owner inventory before another change; no request was retried.".into(),
                    Some(Ok(Err(error))) => format!("{:?}: {}. No request was retried.", error.code, clean(&error.message)),
                    _ => "Completion unknown. Reconcile inventory; no request was retried.".into(),
                };
            },
        }
    }
    jobs.abort_all();
    while jobs.join_next().await.is_some() {}
    Ok(message)
}
