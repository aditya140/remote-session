use super::*;
use crate::{Activity, ApprovalSummary, WorkAcknowledgements};
use base64::{Engine, engine::general_purpose::STANDARD};

fn machine(id: MachineId, owner: UserId) -> Machine {
    Machine {
        id,
        owner,
        name: id.to_string(),
        hostname: "fixture".into(),
        os: "linux".into(),
        arch: "test".into(),
        cli_version: "0.1.0".into(),
        labels: Default::default(),
        agents: Default::default(),
        online: true,
        last_seen: Timestamp::from_unix_millis(1).unwrap(),
        sessions: 0,
    }
}
#[tokio::test]
async fn work_inbox_crosses_machines_and_pages_and_keeps_failed_turns_in_live_processes() {
    let owner = UserId::new();
    let mut ids = [MachineId::new(), MachineId::new()];
    ids.sort();
    let machines = vec![machine(ids[0], owner), machine(ids[1], owner)];
    let mut sessions: Vec<_> = (0..17)
        .map(|_| {
            let mut s = session(ids[0]);
            s.owner = owner;
            s
        })
        .collect();
    sessions.sort_by_key(|s| s.id);
    let failed_id = sessions.last().unwrap().id;
    sessions.last_mut().unwrap().kind = LaunchKind::Agent {
        agent: "codex".into(),
        prompt: None,
        native_resume_id: None,
    };
    let mut completed = session(ids[1]);
    completed.owner = owner;
    completed.kind = LaunchKind::Agent {
        agent: "codex".into(),
        prompt: None,
        native_resume_id: None,
    };
    let completed_id = completed.id;
    sessions.push(completed);
    let mut fixture = fixture_with_discovery(
        move |mut connection| async move {
            while let Some(frame) = connection.recv().await {
                let Frame::Text(request) = frame.frame else {
                    panic!("expected text");
                };
                match &request.message {
                    Message::SessionListPage {
                        after,
                        limit,
                        active_only: false,
                    } => {
                        let remaining: Vec<_> = sessions
                            .iter()
                            .filter(|s| {
                                Some(s.machine) == request.machine
                                    && after.is_none_or(|after| s.id > after)
                            })
                            .collect();
                        let result: Vec<_> = remaining
                            .iter()
                            .take(*limit as usize)
                            .map(|s| (*s).clone())
                            .collect();
                        let next =
                            (remaining.len() > result.len()).then(|| result.last().unwrap().id);
                        send(
                            &connection,
                            response(
                                &request,
                                Message::SessionPage {
                                    sessions: result,
                                    next,
                                },
                            ),
                        )
                        .await;
                    }
                    Message::Attach { after: Counter(0) } => {
                        let mut state = AgentState::default();
                        state
                            .push(SequencedEvent {
                                seq: Counter(1),
                                at: Timestamp::from_unix_millis(1).unwrap(),
                                event: AgentEvent::TurnCompleted {
                                    turn: "exact-task".into(),
                                    input_tokens: Counter(0),
                                    output_tokens: Counter(0),
                                    outcome: Some(if request.session == Some(failed_id) {
                                        TurnOutcome::Failed
                                    } else {
                                        TurnOutcome::Completed
                                    }),
                                },
                            })
                            .unwrap();
                        send(
                            &connection,
                            response(
                                &request,
                                Message::Snapshot {
                                    snapshot: SnapshotPage {
                                        epoch: "work-page".into(),
                                        seq: Counter(1),
                                        page: 0,
                                        total_pages: 1,
                                        rows: 24,
                                        cols: 80,
                                        format: SnapshotFormat::AgentStateV1,
                                        data: STANDARD.encode(state.encode().unwrap()),
                                    },
                                },
                            ),
                        )
                        .await;
                        send(
                            &connection,
                            response(
                                &request,
                                Message::Head {
                                    floor: Counter(1),
                                    head: Counter(1),
                                    last_input: Counter(0),
                                },
                            ),
                        )
                        .await;
                    }
                    Message::Detach => {}
                    other => panic!("work inbox sent unexpected operation {other:?}"),
                }
            }
        },
        true,
        true,
        true,
    )
    .await;
    let client = fixture.api.connect(&token()).await.unwrap();
    let handle = client.handle();
    let summary = ApprovalSummary {
        complete: true,
        ..Default::default()
    };
    let first = handle.work_page(&machines, None, &summary).await.unwrap();
    assert_eq!(first.items.len(), 16);
    let cursor = first.next.unwrap();
    assert_eq!(cursor.machine, ids[0]);
    let second = handle
        .work_page(&machines, Some(cursor), &summary)
        .await
        .unwrap();
    assert!(second.next.is_none());
    assert_eq!(second.items.len(), 2);
    let failed = &second.items[0];
    assert_eq!(failed.session.id, failed_id);
    assert!(matches!(failed.session.state, SessionState::Running));
    assert_eq!(failed.activity, Activity::Failed);
    assert!(failed.activity_observed);
    assert!(failed.needs_attention(false));
    let completed = &second.items[1];
    assert_eq!(completed.session.id, completed_id);
    let mut seen = WorkAcknowledgements::default();
    assert!(completed.needs_attention(seen.contains(completed)));
    assert!(seen.acknowledge(completed));
    assert!(seen.acknowledge(completed));
    assert!(!completed.needs_attention(seen.contains(completed)));
    let mut approval = completed.clone();
    approval.pending_approvals = 1;
    assert!(approval.needs_attention(seen.contains(&approval)));
    approval.pending_approvals = 0;
    approval.approvals_complete = false;
    assert!(approval.needs_attention(seen.contains(&approval)));
    let mut foreign = completed.clone();
    foreign.session.owner = UserId::new();
    assert!(!seen.contains(&foreign));
    foreign = completed.clone();
    foreign.activity = Activity::Completed {
        turn: "next-task".into(),
    };
    assert!(!seen.contains(&foreign));
    assert!(!seen.acknowledge(failed));
    client.close().await;
    tokio::time::timeout(Duration::from_secs(2), &mut fixture.task)
        .await
        .unwrap()
        .unwrap();
}
#[tokio::test]
async fn interrupted_work_observation_is_unknown_and_never_a_confirmed_empty_inbox() {
    let id = MachineId::new();
    let machine = machine(id, UserId::new());
    let mut value = session(id);
    value.kind = LaunchKind::Agent {
        agent: "codex".into(),
        prompt: None,
        native_resume_id: None,
    };
    let fixture = fixture_with_discovery(
        move |mut connection| async move {
            let request = read(&mut connection).await;
            send(
                &connection,
                response(
                    &request,
                    Message::SessionPage {
                        sessions: vec![value],
                        next: None,
                    },
                ),
            )
            .await;
            assert!(matches!(
                read(&mut connection).await.message,
                Message::Attach { .. }
            ));
            connection.close().await.unwrap();
        },
        true,
        false,
        true,
    )
    .await;
    let client = fixture.api.connect(&token()).await.unwrap();
    let page = client
        .handle()
        .work_page(&[machine], None, &ApprovalSummary::default())
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    assert!(!page.items[0].activity_observed);
    assert_eq!(page.items[0].activity, Activity::Unknown);
    assert!(page.items[0].needs_attention(false));
    assert!(!page.approvals_complete);
    client.close().await;
}
