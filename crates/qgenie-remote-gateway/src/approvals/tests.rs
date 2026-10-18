use super::*;
use sqlx::Connection;

fn request(scope: &ControllerScope) -> ApprovalRequest {
    ApprovalRequest {
        id: ApprovalId::new(),
        session: scope.session,
        owner: scope.owner,
        native_id: MessageId::new().to_string(),
        kind: "command".into(),
        summary: "synthetic native command approval".into(),
        request_hash: "a".repeat(64),
        policy_revision: Counter(1),
        requested_at: Timestamp::from_unix_millis(1).unwrap(),
        expires_at: None,
        allowed_decisions: vec![ApprovalDecision::AllowOnce, ApprovalDecision::Deny],
    }
}
pub(crate) async fn database_contract(store: Store, admin: &sqlx::PgPool, scope: ControllerScope) {
    let approval = request(&scope);
    let (a, b) = tokio::join!(
        store.register_approval(&scope, &approval),
        store.register_approval(&scope, &approval)
    );
    a.unwrap();
    b.unwrap();
    let mut changed = approval.clone();
    changed.summary = "different native request".into();
    assert!(matches!(
        store.register_approval(&scope, &changed).await,
        Err(Error::Conflict)
    ));
    let mut foreign = scope.clone();
    foreign.owner = UserId::new();
    assert!(matches!(
        store
            .choose_approval(
                &foreign,
                ClientId::new(),
                approval.id,
                &approval.request_hash,
                ApprovalDecision::AllowOnce
            )
            .await,
        Err(Error::Forbidden)
    ));
    let waiting = request(&scope);
    store.register_approval(&scope, &waiting).await.unwrap();
    let mut locker = sqlx::PgConnection::connect_with(&admin.connect_options())
        .await
        .unwrap();
    let mut locked = locker.begin().await.unwrap();
    sqlx::query("SELECT id FROM qgr.approval_requests WHERE id=$1 FOR UPDATE")
        .bind(waiting.id.to_string())
        .execute(&mut *locked)
        .await
        .unwrap();
    let waiting_id = waiting.id;
    let worker_store = store.clone();
    let worker_scope = scope.clone();
    let expires = std::time::Instant::now() + std::time::Duration::from_millis(250);
    let worker = tokio::spawn(async move {
        worker_store
            .choose_approval_guarded(
                &worker_scope,
                ClientId::new(),
                waiting.id,
                &waiting.request_hash,
                ApprovalDecision::Deny,
                || {
                    if std::time::Instant::now() < expires {
                        Ok(())
                    } else {
                        Err(Error::Forbidden)
                    }
                },
            )
            .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(3),async {
        loop {
            let blocked:bool=sqlx::query_scalar("SELECT EXISTS(SELECT FROM pg_stat_activity WHERE usename='qgr_test_app' AND wait_event_type='Lock' AND query LIKE 'SELECT body,state,expires_at%')").fetch_one(admin).await.unwrap();
            if blocked { break; }
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
    }).await.unwrap();
    tokio::time::sleep_until(tokio::time::Instant::from_std(expires)).await;
    locked.commit().await.unwrap();
    assert!(matches!(worker.await.unwrap(), Err(Error::Forbidden)));
    let partial:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM qgr.approval_decisions WHERE approval=$1)+(SELECT count(*) FROM qgr.approval_outbox WHERE approval=$1)").bind(waiting_id.to_string()).fetch_one(admin).await.unwrap();
    assert_eq!(
        partial, 0,
        "expired original access committed a choice after a row-lock wait"
    );
    assert!(matches!(
        store
            .choose_approval(
                &scope,
                ClientId::new(),
                approval.id,
                "b",
                ApprovalDecision::AllowOnce
            )
            .await,
        Err(Error::Conflict)
    ));
    assert!(matches!(
        store
            .choose_approval(
                &scope,
                ClientId::new(),
                approval.id,
                &approval.request_hash,
                ApprovalDecision::AllowSession
            )
            .await,
        Err(Error::Forbidden)
    ));
    // No controller lease exists for any of these clients. Existing cwd is outside roots.
    let mut contenders = tokio::task::JoinSet::new();
    for index in 0..16 {
        let store = store.clone();
        let scope = scope.clone();
        let approval = approval.clone();
        contenders.spawn(async move {
            store
                .choose_approval(
                    &scope,
                    ClientId::new(),
                    approval.id,
                    &approval.request_hash,
                    if index % 2 == 0 {
                        ApprovalDecision::AllowOnce
                    } else {
                        ApprovalDecision::Deny
                    },
                )
                .await
                .unwrap()
        });
    }
    let mut winners = std::collections::BTreeSet::new();
    let mut chosen = 0;
    while let Some(result) = contenders.join_next().await {
        let choice = result.unwrap();
        winners.insert(choice.winner.id.to_string());
        chosen += usize::from(choice.chosen);
    }
    assert_eq!(chosen, 1);
    assert_eq!(winners.len(), 1);
    let audited: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM qgr.audit_events WHERE action='approval_decided' AND session=$1",
    )
    .bind(scope.session.to_string())
    .fetch_one(admin)
    .await
    .unwrap();
    assert_eq!(audited, 1);
    let status = store.approval_status(&scope, approval.id).await.unwrap();
    assert!(status.winner.is_some() && status.outcome.is_none());
    let missing = request(&scope);
    store.register_approval(&scope, &missing).await.unwrap();
    let removed = store.reconcile_native_approvals(&scope, &[]).await.unwrap();
    assert!(removed.contains(&missing.id));
    assert!(!removed.contains(&approval.id));
    assert!(
        !store
            .approval_status(&scope, approval.id)
            .await
            .unwrap()
            .cancelled
    );
    let node: String = sqlx::query_scalar("SELECT node FROM qgr.machine_routes WHERE machine=$1")
        .bind(scope.machine.to_string())
        .fetch_one(admin)
        .await
        .unwrap();
    assert_eq!(
        store
            .due_approval_scopes(&node, &[scope.machine], 1)
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(
        store
            .due_approval_scopes("wrong-node", &[scope.machine], 1)
            .await
            .unwrap()
            .is_empty()
    );
    assert!(
        store
            .due_approval_scopes(&node, &[MachineId::new()], 1)
            .await
            .unwrap()
            .is_empty()
    );
    let (a, b) = tokio::join!(
        store.claim_approvals(&scope, 32),
        store.claim_approvals(&scope, 32)
    );
    let mut claims = a.unwrap();
    claims.extend(b.unwrap());
    assert_eq!(claims.len(), 1);
    let first = claims.pop().unwrap();
    assert_eq!(first.attempts, 1);
    // The durable obligation survives all runtime pool backends being disconnected.
    sqlx::query("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname=current_database() AND usename='qgr_test_app'").execute(admin).await.unwrap();
    assert!(
        store
            .approval_status(&scope, approval.id)
            .await
            .unwrap()
            .winner
            .is_some()
    );
    assert!(store.claim_approvals(&scope, 32).await.unwrap().is_empty());
    sqlx::query("UPDATE qgr.approval_outbox SET claim_until=clock_timestamp()-interval '1 second' WHERE approval=$1").bind(approval.id.to_string()).execute(admin).await.unwrap();
    let second = store
        .claim_approvals(&scope, 1)
        .await
        .unwrap()
        .pop()
        .unwrap();
    assert_ne!(first.claim, second.claim);
    assert_eq!(second.attempts, 2);
    assert!(matches!(
        store
            .acknowledge_approval(&scope, &first, ApplicationOutcome::Applied, None)
            .await,
        Err(Error::Conflict)
    ));
    store.retry_approval(&scope, &second).await.unwrap();
    assert!(store.claim_approvals(&scope, 1).await.unwrap().is_empty());
    sqlx::query("UPDATE qgr.approval_outbox SET due_at=clock_timestamp()-interval '1 second' WHERE approval=$1").bind(approval.id.to_string()).execute(admin).await.unwrap();
    let delivery = store
        .claim_approvals(&scope, 1)
        .await
        .unwrap()
        .pop()
        .unwrap();
    store
        .acknowledge_approval(
            &scope,
            &delivery,
            ApplicationOutcome::OutcomeUnknown,
            Some("native write has no execution proof"),
        )
        .await
        .unwrap();
    store
        .acknowledge_approval(&scope, &delivery, ApplicationOutcome::OutcomeUnknown, None)
        .await
        .unwrap();
    assert!(matches!(
        store
            .acknowledge_approval(&scope, &delivery, ApplicationOutcome::Applied, None)
            .await,
        Err(Error::Conflict)
    ));
    assert!(store.claim_approvals(&scope, 32).await.unwrap().is_empty());
    assert!(matches!(
        store
            .approval_status(&scope, approval.id)
            .await
            .unwrap()
            .outcome,
        Some(ApplicationOutcome::OutcomeUnknown)
    ));
    let cancelled = request(&scope);
    store.register_approval(&scope, &cancelled).await.unwrap();
    store
        .cancel_approval(
            &scope,
            cancelled.id,
            &cancelled.request_hash,
            "native cancelled before choice",
        )
        .await
        .unwrap();
    assert!(matches!(
        store
            .choose_approval(
                &scope,
                ClientId::new(),
                cancelled.id,
                &cancelled.request_hash,
                ApprovalDecision::AllowOnce
            )
            .await,
        Err(Error::Conflict)
    ));
    let expired = request(&scope);
    store.register_approval(&scope, &expired).await.unwrap();
    sqlx::query("UPDATE qgr.approval_requests SET expires_at=clock_timestamp()-interval '1 second' WHERE id=$1").bind(expired.id.to_string()).execute(admin).await.unwrap();
    assert!(matches!(
        store
            .choose_approval(
                &scope,
                ClientId::new(),
                expired.id,
                &expired.request_hash,
                ApprovalDecision::AllowOnce
            )
            .await,
        Err(Error::Conflict)
    ));
    let policy = request(&scope);
    store.register_approval(&scope, &policy).await.unwrap();
    sqlx::query("UPDATE qgr.machine_routes SET native_access=jsonb_set(native_access,'{policy_revision}','\"2\"') WHERE machine=$1").bind(scope.machine.to_string()).execute(admin).await.unwrap();
    assert!(matches!(
        store
            .choose_approval(
                &scope,
                ClientId::new(),
                policy.id,
                &policy.request_hash,
                ApprovalDecision::AllowOnce
            )
            .await,
        Err(Error::Forbidden)
    ));
    sqlx::query("UPDATE qgr.machine_routes SET native_access=jsonb_set(native_access,'{policy_revision}','\"1\"') WHERE machine=$1").bind(scope.machine.to_string()).execute(admin).await.unwrap();
    let atomic = request(&scope);
    store.register_approval(&scope, &atomic).await.unwrap();
    sqlx::raw_sql("CREATE FUNCTION qgr.test_approval_audit() RETURNS trigger LANGUAGE plpgsql AS $$ BEGIN RAISE EXCEPTION 'synthetic unavailable audit'; END $$; CREATE TRIGGER test_approval_audit BEFORE INSERT ON qgr.audit_events FOR EACH ROW WHEN(NEW.action='approval_decided') EXECUTE FUNCTION qgr.test_approval_audit();").execute(admin).await.unwrap();
    assert!(matches!(
        store
            .choose_approval(
                &scope,
                ClientId::new(),
                atomic.id,
                &atomic.request_hash,
                ApprovalDecision::Deny
            )
            .await,
        Err(Error::Database)
    ));
    let partial:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM qgr.approval_decisions WHERE approval=$1)+(SELECT count(*) FROM qgr.approval_outbox WHERE approval=$1)").bind(atomic.id.to_string()).fetch_one(admin).await.unwrap();
    assert_eq!(partial, 0);
    sqlx::raw_sql("DROP TRIGGER test_approval_audit ON qgr.audit_events; DROP FUNCTION qgr.test_approval_audit();").execute(admin).await.unwrap();
    store
        .choose_approval(
            &scope,
            ClientId::new(),
            atomic.id,
            &atomic.request_hash,
            ApprovalDecision::Deny,
        )
        .await
        .unwrap();
    let list = store.list_approvals(scope.owner, None, 200).await.unwrap();
    let reconciled = store
        .owner_approval_status(scope.owner, approval.id)
        .await
        .unwrap();
    assert_eq!(reconciled.machine, scope.machine);
    assert_eq!(
        reconciled.winner.as_ref().unwrap().id,
        status.winner.as_ref().unwrap().id
    );
    assert!(matches!(
        store
            .owner_approval_status(UserId::new(), approval.id)
            .await,
        Err(Error::NotFound)
    ));
    sqlx::query("UPDATE qgr.machine_routes SET activated=false WHERE machine=$1")
        .bind(scope.machine.to_string())
        .execute(admin)
        .await
        .unwrap();
    assert!(store.approval_status(&scope, approval.id).await.is_err());
    assert!(
        store
            .owner_approval_status(scope.owner, approval.id)
            .await
            .unwrap()
            .winner
            .is_some(),
        "owner reconciliation must survive an offline host"
    );
    sqlx::query("UPDATE qgr.machine_routes SET activated=true WHERE machine=$1")
        .bind(scope.machine.to_string())
        .execute(admin)
        .await
        .unwrap();
    assert!(
        list.iter()
            .any(|p| p.request.id == expired.id && p.cancelled)
    );
    sqlx::query("UPDATE qgr.machines SET disabled=true WHERE id=$1")
        .bind(scope.machine.to_string())
        .execute(admin)
        .await
        .unwrap();
    assert!(matches!(
        store.owner_approval_status(scope.owner, approval.id).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        store
            .choose_approval(
                &scope,
                ClientId::new(),
                atomic.id,
                &atomic.request_hash,
                ApprovalDecision::Deny
            )
            .await,
        Err(Error::Forbidden)
    ));
    assert!(
        store
            .list_approvals(scope.owner, None, 200)
            .await
            .unwrap()
            .iter()
            .all(|p| p.request.session != scope.session)
    );
    sqlx::query("UPDATE qgr.machines SET disabled=false WHERE id=$1")
        .bind(scope.machine.to_string())
        .execute(admin)
        .await
        .unwrap();
}
