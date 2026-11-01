use super::*;
use crate::RemoteReplay;
use qgenie_remote_proto::wire::{Envelope, SequencedEvent};
fn event(seq: u64, text: &str) -> SequencedEvent {
    SequencedEvent {
        seq: Counter(seq),
        at: Timestamp::from_unix_millis(seq).unwrap(),
        event: AgentEvent::PtyOutput {
            data: STANDARD.encode(text),
        },
    }
}
fn replay(after: u64, head: u64, events: Vec<SequencedEvent>) -> RemoteAttachment {
    RemoteAttachment::Replay(RemoteReplay {
        requested_after: Counter(after),
        floor: Counter(1),
        head: Counter(head),
        events,
        more: false,
        history_truncated: false,
    })
}
fn frame(machine: MachineId, session: SessionId, event: SequencedEvent) -> LiveFrame {
    let mut envelope = Envelope::new(Message::Event(event.clone()));
    envelope.machine = Some(machine);
    envelope.session = Some(session);
    envelope.seq = Some(event.seq);
    LiveFrame::Text(Box::new(envelope))
}
fn snapshot(seq: u64, head: u64) -> RemoteAttachment {
    RemoteAttachment::Snapshot(RemoteSnapshot {
        requested_after: None,
        epoch: "fixed-capture".into(),
        seq: Counter(seq),
        rows: 24,
        cols: 80,
        format: SnapshotFormat::Vt100StateV1,
        data: vec![1],
        floor: Counter(1),
        head: Counter(head),
        last_input: Counter(0),
    })
}
#[test]
fn pushed_session_metadata_discovers_a_tail_without_releasing_unverified_output() {
    use qgenie_remote_proto::wire::{Capabilities, LaunchKind, Origin, Session};
    let machine = MachineId::new();
    let session = SessionId::new();
    let mut output = OutputCursor::checkpointed(machine, session);
    output.attachment(snapshot(10, 10)).unwrap();
    let state = Session {
        requested_model: None,
        metadata: Default::default(),
        id: session,
        machine,
        owner: UserId::new(),
        kind: LaunchKind::Pty {
            command: vec!["/bin/sh".into()],
        },
        cwd: "/tmp".into(),
        origin: Origin::Managed,
        state: SessionState::Running,
        created_at: Timestamp::from_unix_millis(1).unwrap(),
        ended_at: None,
        head: Counter(12),
        capabilities: Capabilities::default(),
        recording_warning: None,
    };
    let mut envelope = Envelope::new(Message::Sessions {
        sessions: vec![state.clone()],
    });
    envelope.machine = Some(machine);
    assert!(
        output
            .frame(&LiveFrame::Text(Box::new(envelope)))
            .unwrap()
            .events
            .is_empty()
    );
    assert_eq!(output.read_plan(), ReadPlan::Replay { after: Counter(10) });
    let ready = output
        .attachment(replay(
            10,
            12,
            vec![event(11, "eleven"), event(12, "twelve")],
        ))
        .unwrap();
    assert_eq!(ready.events.len(), 2);
    output.observe_head(Counter(10), Counter(12)).unwrap();
    let mut old = state;
    old.head = Counter(1);
    let mut envelope = Envelope::new(Message::Sessions {
        sessions: vec![old],
    });
    envelope.machine = Some(machine);
    assert!(output.frame(&LiveFrame::Text(Box::new(envelope))).is_ok());
    assert_eq!(output.read_plan(), ReadPlan::Idle);
    assert_eq!(output.cursor(), Counter(12));
}
#[test]
fn reorder_replay_and_binary_duplicates_release_each_output_once() {
    let machine = MachineId::new();
    let session = SessionId::new();
    let mut output = OutputCursor::resume(machine, session, Counter(5));
    assert_eq!(
        output
            .attachment(replay(5, 7, vec![event(6, "six"), event(7, "seven")]))
            .unwrap()
            .events
            .len(),
        2
    );
    assert!(
        output
            .frame(&LiveFrame::Binary(PtyFrame {
                session,
                seq: Counter(6),
                payload: PtyPayload::Output(b"six".to_vec())
            }))
            .unwrap()
            .events
            .is_empty()
    );
    assert!(
        output
            .frame(&frame(machine, session, event(9, "nine")))
            .unwrap()
            .events
            .is_empty()
    );
    assert_eq!(output.read_plan(), ReadPlan::Replay { after: Counter(7) });
    let ready = output
        .attachment(replay(7, 9, vec![event(8, "eight"), event(9, "nine")]))
        .unwrap();
    assert_eq!(
        ready.events.iter().map(|v| v.seq.0).collect::<Vec<_>>(),
        vec![8, 9]
    );
    assert!(
        output
            .frame(&frame(machine, session, event(8, "changed")))
            .is_err()
    );
    assert_eq!(output.read_plan(), ReadPlan::UnconfirmedTail);
}
#[test]
fn fixed_checkpoint_cutover_discards_prefix_and_keeps_the_live_suffix() {
    let machine = MachineId::new();
    let session = SessionId::new();
    let mut output = OutputCursor::terminal(machine, session);
    assert!(
        output
            .frame(&frame(machine, session, event(11, "tail")))
            .unwrap()
            .events
            .is_empty()
    );
    assert!(
        output
            .attachment(replay(0, 10, vec![event(1, "prefix")]))
            .unwrap()
            .events
            .is_empty()
    );
    assert_eq!(output.read_plan(), ReadPlan::Capture);
    let ready = output.attachment(snapshot(10, 11)).unwrap();
    assert_eq!(ready.snapshot.unwrap().seq, Counter(10));
    assert_eq!(ready.events.len(), 1);
    assert_eq!(ready.events[0].seq, Counter(11));
    assert!(
        output
            .frame(&frame(machine, session, event(11, "tail")))
            .unwrap()
            .events
            .is_empty()
    );
    assert_eq!(output.read_plan(), ReadPlan::Idle);
}
#[test]
fn an_idle_head_exposes_a_dropped_final_frame_and_reconnect_keeps_only_output() {
    let machine = MachineId::new();
    let session = SessionId::new();
    let mut output = OutputCursor::resume(machine, session, Counter(5));
    output.attachment(replay(5, 5, vec![])).unwrap();
    let mut head = Envelope::new(Message::Head {
        floor: Counter(1),
        head: Counter(6),
        last_input: Counter(99),
    });
    head.machine = Some(machine);
    head.session = Some(session);
    output.frame(&LiveFrame::Text(Box::new(head))).unwrap();
    assert_eq!(output.read_plan(), ReadPlan::Replay { after: Counter(5) });
    assert_eq!(
        output
            .attachment(replay(5, 6, vec![event(6, "last frame")]))
            .unwrap()
            .events
            .len(),
        1
    );
    output.disconnected();
    assert_eq!(output.read_plan(), ReadPlan::Attach { after: Counter(6) });
}
#[test]
fn an_explicit_gap_stops_painting_until_a_new_checkpoint() {
    let machine = MachineId::new();
    let session = SessionId::new();
    let mut output = OutputCursor::resume(machine, session, Counter(15));
    output.attachment(replay(15, 15, vec![])).unwrap();
    let mut gap = event(16, "");
    gap.event = AgentEvent::Gap {
        from: Counter(16),
        to: Counter(16),
        reason: "native replay loss".into(),
    };
    assert_eq!(
        output
            .frame(&frame(machine, session, gap))
            .unwrap()
            .events
            .len(),
        1
    );
    assert_eq!(output.read_plan(), ReadPlan::Capture);
    assert!(
        output
            .frame(&frame(machine, session, event(17, "must wait")))
            .unwrap()
            .events
            .is_empty()
    );
    assert!(
        output
            .attachment(snapshot(20, 20))
            .unwrap()
            .events
            .is_empty()
    );
    assert_eq!(output.cursor(), Counter(20));
    assert_eq!(
        output
            .frame(&frame(machine, session, event(21, "new output")))
            .unwrap()
            .events
            .len(),
        1
    );
}
#[test]
fn older_in_flight_reads_do_not_rewind_live_output_but_correlated_regression_fails() {
    let machine = MachineId::new();
    let session = SessionId::new();
    let mut output = OutputCursor::resume(machine, session, Counter(5));
    output
        .attachment(replay(5, 7, vec![event(6, "six"), event(7, "seven")]))
        .unwrap();
    output
        .frame(&frame(machine, session, event(8, "eight")))
        .unwrap();
    assert!(
        output
            .attachment(replay(5, 7, vec![event(6, "six"), event(7, "seven")]))
            .unwrap()
            .events
            .is_empty()
    );
    assert!(
        output
            .attachment(snapshot(7, 7))
            .unwrap()
            .snapshot
            .is_none()
    );
    assert_eq!(output.cursor(), Counter(8));
    assert!(output.attachment(replay(8, 7, vec![])).is_err());
    assert_eq!(output.read_plan(), ReadPlan::UnconfirmedTail);
}
#[test]
fn reorder_capacity_and_foreign_target_frames_cannot_silently_change_the_cursor() {
    let machine = MachineId::new();
    let session = SessionId::new();
    let mut output = OutputCursor::events(machine, session);
    output.attachment(replay(0, 0, vec![])).unwrap();
    assert!(
        output
            .frame(&frame(MachineId::new(), session, event(1, "foreign")))
            .unwrap()
            .events
            .is_empty()
    );
    for seq in 2..=257 {
        output
            .frame(&frame(machine, session, event(seq, "out of order")))
            .unwrap();
    }
    assert_eq!(
        output
            .frame(&frame(machine, session, event(258, "overflow")))
            .err()
            .unwrap()
            .code,
        ErrorCode::Overloaded
    );
    assert_eq!(output.cursor(), Counter(0));
    assert_eq!(output.read_plan(), ReadPlan::Capture);
}
