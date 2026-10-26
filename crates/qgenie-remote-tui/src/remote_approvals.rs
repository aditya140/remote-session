//! Owner-wide approval inbox. Choices never acquire a controller or retry.
use crate::{app::TerminalGuard, display::multiline as clean};
use anyhow::Result;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use qgenie_remote_client::{ApprovalPage, LiveClient, LiveFrame};
use qgenie_remote_proto::{
    wire::{ApprovalDecision, ApprovalState, ApprovalWinner, Message},
    *,
};
use ratatui::{
    layout::{Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use std::{
    collections::HashSet,
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};
use tokio::task::JoinSet;

enum Update {
    Page(ApprovalPage),
    State(Box<ApprovalState>),
    Choice(ApprovalId, Result<ApprovalWinner, RemoteError>),
}
fn status(state: &ApprovalState) -> String {
    if let Some(outcome) = &state.outcome {
        format!("{outcome:?}")
    } else if state.winner.is_some() {
        "Committed; awaiting native application".into()
    } else if state.cancelled {
        "Cancelled".into()
    } else {
        "Pending".into()
    }
}
fn pending(state: &ApprovalState, blocked: &HashSet<ApprovalId>) -> bool {
    blocked.len() < 4096
        && !state.cancelled
        && state.winner.is_none()
        && !blocked.contains(&state.request.id)
        && state.request.expires_at.as_ref().is_none_or(|expiry| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .is_ok_and(|now| now.as_millis() < u128::from(expiry.unix_millis()))
        })
}
// An older snapshot/event must never reopen a committed/cancelled local view.
fn merge(old: &mut ApprovalState, new: ApprovalState) {
    if old.request != new.request || old.machine != new.machine {
        return;
    }
    if old.winner.is_some() && new.winner.is_none() {
        return;
    }
    if old.cancelled && !new.cancelled && new.winner.is_none() {
        return;
    }
    if old.outcome.is_some() && new.outcome.is_none() {
        return;
    }
    *old = new;
}
pub(crate) async fn run(
    terminal: &mut TerminalGuard,
    signal: &AtomicBool,
    client: &mut LiveClient,
) -> Result<String> {
    let handle = client.handle();
    let mut jobs: JoinSet<Result<Update, RemoteError>> = JoinSet::new();
    let initial = handle.clone();
    jobs.spawn(async move { initial.approval_index(None, 200).await.map(Update::Page) });
    let mut states: Vec<ApprovalState> = Vec::new();
    let mut next = None;
    let mut selected = 0usize;
    let mut scroll = 0u16;
    let mut blocked = HashSet::new();
    let mut confirmation: Option<(ApprovalState, ApprovalDecision)> = None;
    let mut message = "Owner approvals do not require control of a session.".to_owned();
    let mut refresh = None;
    let mut ticks = tokio::time::interval(Duration::from_millis(30));
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = ticks.tick() => {
                if signal.load(Ordering::Acquire) { break; }
                if event::poll(Duration::ZERO)? && let Event::Key(key) = event::read()? && key.kind != KeyEventKind::Release {
                    if key.code==KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {signal.store(true,Ordering::Release);continue;}
                    if matches!(key.code,KeyCode::Esc|KeyCode::Char('q')) {
                        if confirmation.take().is_none() { break; }
                        message = "Choice cancelled.".into();
                    } else if let Some((state, decision)) = confirmation.take() {
                        if key.code == KeyCode::Char('y') && jobs.is_empty() && pending(&state,&blocked) {
                            blocked.insert(state.request.id);
                            let handle = handle.clone();
                            jobs.spawn(async move { Ok(Update::Choice(state.request.id,handle.resolve_approval(&state,decision).await)) });
                            message = "One choice submitted. A commit does not prove native execution.".into();
                        } else if key.code != KeyCode::Char('n') { confirmation = Some((state,decision)); }
                    } else {
                        match key.code {
                            KeyCode::Up => {selected=selected.saturating_sub(1);scroll=0;},
                            KeyCode::Down => {selected=(selected+1).min(states.len().saturating_sub(1));scroll=0;},
                            KeyCode::Home => {selected=0;scroll=0;},
                            KeyCode::End => {selected=states.len().saturating_sub(1);scroll=0;},
                            KeyCode::PageUp => scroll=scroll.saturating_sub(10),
                            KeyCode::PageDown => scroll=scroll.saturating_add(10),
                            KeyCode::Char('r') if jobs.is_empty() => refresh=states.get(selected).map(|state|state.request.id),
                            KeyCode::Char('p'|'n') if jobs.is_empty() => {
                                let cursor=if key.code==KeyCode::Char('p') {None} else {next};
                                if key.code==KeyCode::Char('p') || cursor.is_some() {
                                    let handle=handle.clone();jobs.spawn(async move {handle.approval_index(cursor,200).await.map(Update::Page)});
                                }
                            },
                            KeyCode::Char(c @ '1'..='9') if jobs.is_empty() && handle.approvals_available() => {
                                if let Some(state)=states.get(selected) && pending(state,&blocked)
                                    && let Some(decision)=state.request.allowed_decisions.get(c as usize-'1' as usize) {
                                    confirmation=Some((state.clone(),*decision));
                                }
                            },
                            _=>{},
                        }
                    }
                }
                if jobs.is_empty() && let Some(id)=refresh.take() {
                    let handle=handle.clone();
                    jobs.spawn(async move {handle.approval_status(id).await.map(|state|Update::State(Box::new(state)))});
                }
                terminal.terminal.draw(|frame| {
                    let areas=Layout::default().direction(Direction::Vertical).constraints([Constraint::Min(5),Constraint::Length(5)]).split(frame.area());
                    let panes=Layout::default().direction(Direction::Horizontal).constraints([Constraint::Percentage(35),Constraint::Percentage(65)]).split(areas[0]);
                    let items:Vec<_>=states.iter().map(|state| ListItem::new(format!("{} · {}",status(state),clean(&state.request.summary).lines().next().unwrap_or("")))).collect();
                    frame.render_stateful_widget(List::new(items).block(Block::default().borders(Borders::ALL).title("Owner approvals")).highlight_style(Style::default().fg(Color::Cyan)).highlight_symbol("› "),panes[0],&mut ListState::default().with_selected(Some(selected)));
                    let detail=states.get(selected).map(|state| {
                        let menu=if pending(state,&blocked) && handle.approvals_available() {state.request.allowed_decisions.iter().enumerate().map(|(index,decision)|format!("{}: {decision:?}",index+1)).collect::<Vec<_>>().join(" · ")} else {"Read-only reconciliation; no answer will be resent.".into()};
                        clean(&format!("{}\nMachine: {}\nSession: {}\nPolicy revision: {}\n{}\n\n{}\n\n{}\n{}",state.request.id,state.machine,state.request.session,state.request.policy_revision.0,status(state),state.request.summary,menu,state.reason.as_deref().unwrap_or("")))
                    }).unwrap_or_else(|| "No approvals on this page.".into());
                    frame.render_widget(Paragraph::new(detail).wrap(Wrap{trim:false}).scroll((scroll,0)).block(Block::default().borders(Borders::ALL).title("Immutable native request")),panes[1]);
                    let footer=confirmation.as_ref().map(|(_,decision)|format!("Commit {decision:?}? y confirms · n cancels")).unwrap_or_else(||format!("↑/↓ select · 1–9 offered choice · r status · p first · n next · Esc close\n{}{}",clean(&message),if jobs.is_empty(){""}else{" · waiting"}));
                    frame.render_widget(Paragraph::new(footer).wrap(Wrap{trim:false}).block(Block::default().borders(Borders::ALL)),areas[1]);
                })?;
            },
            update=jobs.join_next(),if !jobs.is_empty()=>{
                match update {
                    Some(Ok(Ok(Update::Page(page))))=>{
                        let mut new=page.states;
                        for state in &mut new {if let Some(old)=states.iter().find(|old|old.request.id==state.request.id){let incoming=state.clone();*state=old.clone();merge(state,incoming);}}
                        states=new;next=page.next;selected=0;scroll=0;
                    },
                    Some(Ok(Ok(Update::State(state))))=>{
                        if let Some(old)=states.iter_mut().find(|old|old.request.id==state.request.id){merge(old,*state);}
                        else if states.len()<200 {states.push(*state);}
                        else {message="An approval changed outside this page; use p/n to browse.".into();}
                    },
                    Some(Ok(Ok(Update::Choice(id,result))))=>{
                        match result {
                            Ok(winner)=>{if let Some(state)=states.iter_mut().find(|state|state.request.id==id){state.winner=Some(winner);}message="Choice committed; awaiting native application status.".into();},
                            Err(error)=>message=format!("{:?}: {}. Read status; this view will not send another answer.",error.outcome,clean(&error.message)),
                        }
                        refresh=Some(id);
                    },
                    Some(Ok(Err(error)))=>message=format!("{:?}: {}",error.code,clean(&error.message)),
                    _=>message="Request ended; read status to reconcile.".into(),
                }
            },
            incoming=client.next()=>{
                let Some(event)=incoming else {message="Connection closed; reopen and read durable status. Choices were not resent.".into();break;};
                if let LiveFrame::Text(envelope)=&event.frame {
                    let id=match &envelope.message {
                        Message::ApprovalRequested(request)=>Some(request.id),
                        Message::ApprovalCommitted(winner)=>Some(winner.approval),
                        Message::ApprovalApplication{approval,..}|Message::ApprovalCancelled{approval,..}=>Some(*approval),
                        _=>None,
                    };
                    if let Some(id)=id {
                        if states.get(selected).is_some_and(|state|state.request.id==id) {refresh=Some(id);}
                        else if jobs.is_empty() {let handle=handle.clone();jobs.spawn(async move{handle.approval_status(id).await.map(|state|Update::State(Box::new(state)))});}
                    }
                }
            }
        }
    }
    jobs.abort_all();
    while jobs.join_next().await.is_some() {}
    Ok(message)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stale_views_never_reopen_a_committed_native_choice() {
        let Message::ApprovalStatus(mut state) = qgenie_remote_proto::wire::Envelope::decode(
            include_bytes!("../../../conformance/json/network-approval-status.json"),
        )
        .unwrap()
        .message
        else {
            panic!("fixture");
        };
        let winner = state.winner.clone();
        let mut stale = state.clone();
        stale.winner = None;
        stale.outcome = None;
        merge(&mut state, stale);
        assert_eq!(state.winner, winner);
        assert!(!pending(&state, &HashSet::new()));
    }
}
