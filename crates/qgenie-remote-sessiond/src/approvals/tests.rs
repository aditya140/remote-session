use super::*;
use qgenie_remote_proto::wire::{ApprovalDecision, Timestamp};
use std::sync::atomic::{AtomicUsize, Ordering};
fn source() -> ApprovalSource {
    ApprovalSource {
        scope: RemoteScope {
            origin: "https://gateway.example.invalid".into(),
            machine: MachineId::new(),
            owner: UserId::new(),
        },
        session: SessionId::new(),
        instance: MessageId::new(),
    }
}
fn request(source: &ApprovalSource) -> ApprovalRequest {
    ApprovalRequest {
        id: ApprovalId::new(),
        session: source.session,
        owner: source.scope.owner,
        native_id: MessageId::new().to_string(),
        kind: "command".into(),
        summary: "synthetic command".into(),
        request_hash: "ab".repeat(32),
        policy_revision: Counter(1),
        requested_at: Timestamp::from_unix_millis(1).unwrap(),
        expires_at: None,
        allowed_decisions: vec![ApprovalDecision::AllowOnce, ApprovalDecision::Deny],
    }
}
fn winner(request: &ApprovalRequest) -> ApprovalWinner {
    ApprovalWinner {
        approval: request.id,
        id: DecisionId::new(),
        actor: request.owner,
        client: ClientId::new(),
        request_hash: request.request_hash.clone(),
        policy_revision: request.policy_revision,
        decision: ApprovalDecision::AllowOnce,
        committed_at: Timestamp::from_unix_millis(2).unwrap(),
    }
}
fn known(result: NativeApprovalReservation) -> ApplicationOutcome {
    match result {
        NativeApprovalReservation::Known(outcome) => outcome,
        NativeApprovalReservation::Send(_) => panic!("a native answer must not be offered twice"),
    }
}
#[test]
fn concurrent_delivery_reserves_one_native_write_and_reconciles_unknown_after_restart() {
    let directory = tempfile::Builder::new()
        .permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700))
        .tempdir()
        .unwrap();
    let journal = NativeApprovals::open(directory.path()).unwrap();
    let source = source();
    journal.register_source(&source).unwrap();
    let request = request(&source);
    journal.record(&source, &request).unwrap();
    let winner = winner(&request);
    assert!(NativeApprovals::open(directory.path()).is_err());
    let attempts = AtomicUsize::new(0);
    std::thread::scope(|threads| {
        for _ in 0..16 {
            let journal = &journal;
            let source = &source;
            let winner = &winner;
            let attempts = &attempts;
            threads.spawn(
                move || match journal.reserve(source, winner, Counter(1), 2).unwrap() {
                    NativeApprovalReservation::Send(intent) => {
                        attempts.fetch_add(1, Ordering::SeqCst);
                        intent
                            .complete(ApplicationOutcome::OutcomeUnknown, None)
                            .unwrap();
                    }
                    NativeApprovalReservation::Known(result) => {
                        assert!(matches!(result, ApplicationOutcome::OutcomeUnknown))
                    }
                },
            );
        }
    });
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    let mut changed = winner.clone();
    changed.decision = ApprovalDecision::Deny;
    assert_eq!(
        journal
            .reserve(&source, &changed, Counter(1), 3)
            .err()
            .unwrap()
            .code,
        ErrorCode::AlreadyResolved
    );
    drop(journal);
    let journal = NativeApprovals::open(directory.path()).unwrap();
    assert!(matches!(
        known(journal.reserve(&source, &winner, Counter(1), 3).unwrap()),
        ApplicationOutcome::OutcomeUnknown
    ));
    assert!(
        journal.register_source(&source).is_err(),
        "an old native source is never adopted after restart"
    );
}
#[test]
fn intent_loss_pending_restart_policy_timeout_and_native_resolution_never_approve() {
    let directory = tempfile::Builder::new()
        .permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700))
        .tempdir()
        .unwrap();
    let journal = NativeApprovals::open(directory.path()).unwrap();
    let source = source();
    journal.register_source(&source).unwrap();
    let offered = request(&source);
    journal.record(&source, &offered).unwrap();
    let offered_winner = winner(&offered);
    let intent = journal
        .reserve(&source, &offered_winner, Counter(1), 2)
        .unwrap();
    assert!(matches!(intent, NativeApprovalReservation::Send(_)));
    drop(intent);
    assert!(matches!(
        known(
            journal
                .reserve(&source, &offered_winner, Counter(1), 3)
                .unwrap()
        ),
        ApplicationOutcome::OutcomeUnknown
    ));
    let pending = request(&source);
    journal.record(&source, &pending).unwrap();
    let pending_winner = winner(&pending);
    let cancelled = request(&source);
    journal.record(&source, &cancelled).unwrap();
    journal.resolved(&source, cancelled.id).unwrap();
    assert!(matches!(
        known(
            journal
                .reserve(&source, &winner(&cancelled), Counter(1), 3)
                .unwrap()
        ),
        ApplicationOutcome::Cancelled
    ));
    let policy = request(&source);
    journal.record(&source, &policy).unwrap();
    let policy_winner = winner(&policy);
    assert!(matches!(
        known(
            journal
                .reserve(&source, &policy_winner, Counter(2), 3)
                .unwrap()
        ),
        ApplicationOutcome::Cancelled
    ));
    assert!(matches!(
        known(
            journal
                .reserve(&source, &policy_winner, Counter(1), 3)
                .unwrap()
        ),
        ApplicationOutcome::Cancelled
    ));
    let mut expired = request(&source);
    expired.expires_at = Some(Timestamp::from_unix_millis(3).unwrap());
    journal.record(&source, &expired).unwrap();
    assert!(matches!(
        known(
            journal
                .reserve(&source, &winner(&expired), Counter(1), 3)
                .unwrap()
        ),
        ApplicationOutcome::Cancelled
    ));
    let resolved = request(&source);
    journal.record(&source, &resolved).unwrap();
    let resolved_winner = winner(&resolved);
    let NativeApprovalReservation::Send(intent) = journal
        .reserve(&source, &resolved_winner, Counter(1), 2)
        .unwrap()
    else {
        panic!("first reservation")
    };
    journal.resolved(&source, resolved.id).unwrap();
    intent
        .complete(ApplicationOutcome::OutcomeUnknown, None)
        .unwrap();
    drop(journal);
    let journal = NativeApprovals::open(directory.path()).unwrap();
    assert!(matches!(
        known(
            journal
                .reserve(&source, &offered_winner, Counter(1), 4)
                .unwrap()
        ),
        ApplicationOutcome::OutcomeUnknown
    ));
    assert!(matches!(
        known(
            journal
                .reserve(&source, &pending_winner, Counter(1), 4)
                .unwrap()
        ),
        ApplicationOutcome::Cancelled
    ));
}
#[test]
fn immutable_sources_owner_machine_request_hash_and_native_menu_are_enforced() {
    let directory = tempfile::Builder::new()
        .permissions(<std::fs::Permissions as std::os::unix::fs::PermissionsExt>::from_mode(0o700))
        .tempdir()
        .unwrap();
    let journal = NativeApprovals::open(directory.path()).unwrap();
    let source = source();
    journal.register_source(&source).unwrap();
    let request = request(&source);
    journal.record(&source, &request).unwrap();
    journal.record(&source, &request).unwrap();
    let mut foreign = source.clone();
    foreign.scope.owner = UserId::new();
    assert!(journal.register_source(&foreign).is_err());
    assert!(journal.record(&foreign, &request).is_err());
    assert!(
        journal
            .reserve(&foreign, &winner(&request), Counter(1), 2)
            .is_err()
    );
    foreign = source.clone();
    foreign.scope.machine = MachineId::new();
    assert!(journal.record(&foreign, &request).is_err());
    foreign = source.clone();
    foreign.instance = MessageId::new();
    assert!(journal.record(&foreign, &request).is_err());
    let mut changed = request.clone();
    changed.summary = "changed".into();
    assert!(journal.record(&source, &changed).is_err());
    changed = request.clone();
    changed.id = ApprovalId::new();
    assert!(
        journal.record(&source, &changed).is_err(),
        "a native id cannot be rebound"
    );
    let mut wrong = winner(&request);
    wrong.request_hash = "bc".repeat(32);
    assert!(journal.reserve(&source, &wrong, Counter(1), 2).is_err());
    let mut unsupported = winner(&request);
    unsupported.decision = ApprovalDecision::AllowSession;
    assert!(matches!(
        known(
            journal
                .reserve(&source, &unsupported, Counter(1), 2)
                .unwrap()
        ),
        ApplicationOutcome::Cancelled
    ));
}
