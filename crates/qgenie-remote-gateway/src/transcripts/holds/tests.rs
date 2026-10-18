use super::*;
use crate::{Authenticator, JwtVerifier, SecretDsn, identity::jwt::tests as jwt};
use base64::{Engine, engine::general_purpose::STANDARD};
use qgenie_remote_proto::{
    ChunkId, Counter, GapId, HoldId, KeyId, MachineId, RecordingEpochId, SessionId, UserId,
    rest::LifecycleStage,
    wire::{LossRange, LossReason, RecordingStream},
};
use sqlx::PgPool;

async fn count(admin: &PgPool, table: &str, id: ChunkId) -> i64 {
    let sql = match table {
        "content" => "SELECT count(*) FROM qgr_recordings.content WHERE id=$1",
        "held" => "SELECT count(*) FROM qgr_recordings.held_content WHERE id=$1",
        _ => "SELECT count(*) FROM qgr_recordings.held_cases WHERE id=$1",
    };
    sqlx::query_scalar(sql)
        .bind(id.to_string())
        .fetch_one(admin)
        .await
        .unwrap()
}
async fn seed(
    admin: &PgPool,
    owner: UserId,
    machine: MachineId,
    session: SessionId,
    at: time::OffsetDateTime,
) -> ChunkId {
    let id = ChunkId::new();
    sqlx::query("INSERT INTO qgr_recordings.receipts(machine,session,owner,kind,id,immutable,receipt,captured_from,captured_to) VALUES($1,$2,$3,'content',$4,'{}','{}',$5,$5)").bind(machine.to_string()).bind(session.to_string()).bind(owner.to_string()).bind(id.to_string()).bind(at).execute(admin).await.unwrap();
    sqlx::query("INSERT INTO qgr_recordings.content(machine,session,owner,id,captured_from,captured_to,payload,stored_digest) VALUES($1,$2,$3,$4,$5,$5,$6,$7)").bind(machine.to_string()).bind(session.to_string()).bind(owner.to_string()).bind(id.to_string()).bind(at).bind(b"synthetic preservation fixture".to_vec()).bind("ab".repeat(32)).execute(admin).await.unwrap();
    id
}
async fn step(
    db: &RecordingDatabase,
    identity: &IdentityService,
    actor: &Principal,
) -> qgenie_remote_proto::rest::LifecycleProgress {
    db.maintenance_step(identity, actor, "synthetic retention race", 1, true)
        .await
        .unwrap()
}
async fn idle(db: &RecordingDatabase, identity: &IdentityService, actor: &Principal) {
    for _ in 0..100 {
        if step(db, identity, actor).await.stage == LifecycleStage::Idle {
            return;
        }
    }
    panic!("bounded synthetic lifecycle work did not drain");
}

pub(crate) async fn database_contract(store: Store, admin: &PgPool, migration: &SecretDsn) {
    let verifier = JwtVerifier::with_keys(jwt::config(), jwt::keys(), store.clone()).unwrap();
    let owner = verifier
        .authenticate(&jwt::sign(&jwt::claims("lifecycle-owner")))
        .await
        .unwrap();
    let mut claims = jwt::claims("lifecycle-platform-admin");
    claims["roles"] = serde_json::json!([jwt::config().admin_role]);
    let actor = verifier.authenticate(&jwt::sign(&claims)).await.unwrap();
    let secret = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(secret.path(), STANDARD.encode([75; 32])).unwrap();
    let identity = IdentityService::from_file(
        store,
        "https://gateway.example.invalid".into(),
        secret.path(),
    )
    .await
    .unwrap();
    let mut url = url::Url::parse(&migration.0).unwrap();
    url.set_username("qgr_test_record_lifecycle").unwrap();
    url.set_password(Some("test-only")).unwrap();
    let dsn = crate::tests::recording_secret(url.as_str());
    let db = RecordingDatabase::connect(&dsn, RecordingRole::Lifecycle, 2)
        .await
        .unwrap();
    assert!(
        sqlx::query("SELECT * FROM qgr_recordings.holds")
            .execute(&db.pool)
            .await
            .is_err()
    );
    assert!(
        sqlx::query("SELECT * FROM qgr_recordings.lifecycle_step(1)")
            .execute(&db.pool)
            .await
            .is_err()
    );
    let machine = MachineId::new();
    let session = SessionId::new();
    sqlx::query("INSERT INTO qgr.machines(id,owner,installation,os_user,active_key,hostname,name,os,arch,cli_version) VALUES($1,$2,$1,'fixture',$3,'fixture','fixture','linux','test','test')").bind(machine.to_string()).bind(owner.user.to_string()).bind(KeyId::new().to_string()).execute(admin).await.unwrap();
    sqlx::query(
        "INSERT INTO qgr.sessions(id,machine,owner,body,state) VALUES($1,$2,$3,'{}','running')",
    )
    .bind(session.to_string())
    .bind(machine.to_string())
    .bind(owner.user.to_string())
    .execute(admin)
    .await
    .unwrap();
    // Ingest tests retire their own relative historical month. Choose an earlier,
    // distinct month so a midnight/day-count boundary cannot reuse that retirement.
    let old:time::Date=sqlx::query_scalar("SELECT LEAST(date_trunc('month',clock_timestamp() AT TIME ZONE 'UTC')-interval '6 months',COALESCE((SELECT min(month)::timestamp-interval '1 month' FROM qgr_recordings.partitions),date_trunc('month',clock_timestamp() AT TIME ZONE 'UTC')-interval '6 months'))::date").fetch_one(admin).await.unwrap();
    let boundary:time::Date=sqlx::query_scalar("SELECT date_trunc('month',(clock_timestamp()-interval '90 days') AT TIME ZONE 'UTC')::date").fetch_one(admin).await.unwrap();
    for month in [old, boundary] {
        sqlx::query("SELECT qgr_recordings.ensure_month($1)")
            .bind(month)
            .execute(admin)
            .await
            .unwrap();
    }
    let first = old.midnight().assume_utc() + time::Duration::days(2);
    let second = first + time::Duration::days(10);
    let a = seed(admin, owner.user, machine, session, first).await;
    let b = seed(admin, owner.user, machine, session, second).await;
    let gap = LossRange {
        id: GapId::new(),
        session,
        stream: RecordingStream::Pty,
        stream_epoch: RecordingEpochId::new(),
        from: Counter(1),
        to: Counter(2),
        captured_from: Timestamp::from_unix_millis((first.unix_timestamp() * 1000) as u64).unwrap(),
        captured_to: Timestamp::from_unix_millis((second.unix_timestamp() * 1000) as u64).unwrap(),
        reason: LossReason::CaptureBackpressure,
    };
    let body = serde_json::to_value(&gap).unwrap();
    sqlx::query("INSERT INTO qgr_recordings.receipts(machine,session,owner,kind,id,immutable,receipt,captured_from,captured_to) VALUES($1,$2,$3,'gap',$4,$5,'{}',$6,$7)").bind(machine.to_string()).bind(session.to_string()).bind(owner.user.to_string()).bind(gap.id.to_string()).bind(&body).bind(first).bind(second).execute(admin).await.unwrap();
    sqlx::query("INSERT INTO qgr_recordings.gaps(machine,session,owner,id,captured_from,captured_to,body) VALUES($1,$2,$3,$4,$5,$6,$7)").bind(machine.to_string()).bind(session.to_string()).bind(owner.user.to_string()).bind(gap.id.to_string()).bind(first).bind(second).bind(body).execute(admin).await.unwrap();
    let mut hold = HoldChange {
        id: HoldId::new(),
        user: owner.user,
        revision: Counter(1),
        active: true,
        captured_from: Some(
            Timestamp::from_unix_millis((first.unix_timestamp() * 1000) as u64).unwrap(),
        ),
        captured_to: Some(
            Timestamp::from_unix_millis((first.unix_timestamp() * 1000) as u64).unwrap(),
        ),
        case_ref: Some("case-one".into()),
        reason: "synthetic preservation".into(),
    };
    assert!(matches!(
        db.change_hold(&identity, &actor, hold.clone()).await,
        Err(Error::Configuration)
    ));
    assert!(matches!(
        db.hold_change(&identity, &owner, hold.clone(), true).await,
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        db.hold_change(&identity, &actor, hold.clone(), true).await,
        Err(Error::Unavailable)
    ));
    super::super::ingest::tests::state(admin, "synthetic-lifecycle", false).await;
    // Audit failure rolls back the hold itself. A success cannot leave preservation unfenced.
    sqlx::raw_sql("REVOKE INSERT(actor,action,machine,session,metadata) ON qgr.audit_events FROM qgr_record_lifecycle").execute(admin).await.unwrap();
    assert!(matches!(
        db.hold_change(&identity, &actor, hold.clone(), true).await,
        Err(Error::Database)
    ));
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM qgr_recordings.holds WHERE id=$1)")
            .bind(hold.id.to_string())
            .fetch_one(admin)
            .await
            .unwrap();
    assert!(!exists);
    sqlx::raw_sql("GRANT INSERT(actor,action,machine,session,metadata) ON qgr.audit_events TO qgr_record_lifecycle").execute(admin).await.unwrap();
    db.hold_change(&identity, &actor, hold.clone(), true)
        .await
        .unwrap();
    db.hold_change(&identity, &actor, hold.clone(), true)
        .await
        .unwrap();
    let audits:i64=sqlx::query_scalar("SELECT count(*) FROM qgr.audit_events WHERE action='hold_changed' AND metadata->'recording_lifecycle'->'hold'->>'id'=$1").bind(hold.id.to_string()).fetch_one(admin).await.unwrap();
    assert_eq!(audits, 1);
    let mut conflict = hold.clone();
    conflict.case_ref = Some("changed without new revision".into());
    assert!(matches!(
        db.hold_change(&identity, &actor, conflict, true).await,
        Err(Error::Conflict)
    ));
    // A cancelled uncommitted copy cannot establish preservation; a committed copy survives
    // the worker closing/reopening its pool before final retirement.
    let (mut tx, _) = db.begin_lifecycle(&identity, &actor, true).await.unwrap();
    sqlx::query("SELECT * FROM qgr_recordings.lifecycle_step(1)")
        .execute(&mut *tx)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    assert_eq!(count(admin, "held", a).await, 0);
    for _ in 0..32 {
        if count(admin, "held", a).await == 1 {
            break;
        }
        assert_eq!(
            step(&db, &identity, &actor).await.stage,
            LifecycleStage::PreservedContent
        );
    }
    assert_eq!(count(admin, "held", a).await, 1);
    assert_eq!(count(admin, "held", b).await, 0);
    db.close().await;
    let db = RecordingDatabase::connect(&dsn, RecordingRole::Lifecycle, 2)
        .await
        .unwrap();
    let mut retiring = false;
    for _ in 0..32 {
        let progress = step(&db, &identity, &actor).await;
        if progress.stage == LifecycleStage::Retiring {
            assert_eq!(progress.content_month, Some(old.to_string()));
            retiring = true;
            break;
        }
        assert!(matches!(
            progress.stage,
            LifecycleStage::PreservedContent | LifecycleStage::PreservedGaps
        ));
    }
    assert!(retiring);
    // A new hold committed between copy and DROP must participate in preservation. Null case
    // references preserve data but authorize no held reads until a case is assigned.
    let mut overlap = HoldChange {
        id: HoldId::new(),
        user: owner.user,
        revision: Counter(1),
        active: true,
        captured_from: None,
        captured_to: None,
        case_ref: None,
        reason: "synthetic overlapping preservation".into(),
    };
    let (mut hold_tx, _) = db.begin_lifecycle(&identity, &actor, true).await.unwrap();
    sqlx::query("SELECT qgr_recordings.change_hold($1,$2,1,true,NULL,NULL,NULL,$3)")
        .bind(overlap.id.to_string())
        .bind(overlap.user.to_string())
        .bind(&overlap.reason)
        .execute(&mut *hold_tx)
        .await
        .unwrap();
    Store::append_audit(
        &mut hold_tx,
        &AuditEvent {
            actor: actor.user,
            action: AuditAction::HoldChanged,
            machine: None,
            session: None,
            metadata: AuditMetadata {
                subject: Some(overlap.user),
                recording_lifecycle: Some(RecordingLifecycleAudit {
                    reason: overlap.reason.clone(),
                    hold: Some(overlap.clone()),
                    progress: None,
                }),
                ..Default::default()
            },
        },
    )
    .await
    .unwrap();
    // Start retirement while placement is still uncommitted. It must wait on the
    // database lifecycle lock (or broker backend), then observe the committed hold.
    let concurrent =
        db.maintenance_step(&identity, &actor, "concurrent hold and retirement", 1, true);
    tokio::pin!(concurrent);
    tokio::select! {
        result=&mut concurrent=>panic!("retirement bypassed an uncommitted hold: {result:?}"),
        _=tokio::time::sleep(std::time::Duration::from_millis(50))=>{}
    }
    hold_tx.commit().await.unwrap();
    assert!(matches!(
        concurrent.await.unwrap().stage,
        LifecycleStage::PreservedContent | LifecycleStage::PreservedGaps
    ));
    for _ in 0..32 {
        let gap_coverage: i64 =
            sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.held_gap_cases WHERE id=$1")
                .bind(gap.id.to_string())
                .fetch_one(admin)
                .await
                .unwrap();
        if count(admin, "held", b).await == 1
            && count(admin, "coverage", a).await == 2
            && gap_coverage == 2
        {
            break;
        }
        assert!(matches!(
            step(&db, &identity, &actor).await.stage,
            LifecycleStage::PreservedContent | LifecycleStage::PreservedGaps
        ));
    }
    assert_eq!(count(admin, "held", b).await, 1);
    assert_eq!(count(admin, "coverage", a).await, 2);
    // Corruption of a held copy cannot authorize source deletion.
    sqlx::query("UPDATE qgr_recordings.held_content SET payload=$2 WHERE id=$1")
        .bind(a.to_string())
        .bind(b"bad copy".to_vec())
        .execute(admin)
        .await
        .unwrap();
    assert!(matches!(
        db.maintenance_step(&identity, &actor, "synthetic corruption", 1, true)
            .await,
        Err(Error::Conflict)
    ));
    assert_eq!(count(admin, "content", a).await, 1);
    sqlx::query("UPDATE qgr_recordings.held_content h SET payload=c.payload FROM qgr_recordings.content c WHERE h.id=$1 AND c.id=h.id").bind(a.to_string()).execute(admin).await.unwrap();
    // DROP is transactional too: failure to commit its audit keeps the partition intact.
    sqlx::raw_sql("REVOKE INSERT(actor,action,machine,session,metadata) ON qgr.audit_events FROM qgr_record_lifecycle").execute(admin).await.unwrap();
    assert!(matches!(
        db.maintenance_step(&identity, &actor, "synthetic audit failure", 1, true)
            .await,
        Err(Error::Database)
    ));
    assert_eq!(count(admin, "content", a).await, 1);
    sqlx::raw_sql("GRANT INSERT(actor,action,machine,session,metadata) ON qgr.audit_events TO qgr_record_lifecycle").execute(admin).await.unwrap();
    assert_eq!(
        step(&db, &identity, &actor).await.stage,
        LifecycleStage::Retired
    );
    assert_eq!(count(admin, "content", a).await, 0);
    assert_eq!(count(admin, "held", a).await, 1);
    assert_eq!(count(admin, "held", b).await, 1);
    let recreated: bool = sqlx::query_scalar("SELECT qgr_recordings.ensure_month($1)")
        .bind(old)
        .fetch_one(admin)
        .await
        .unwrap();
    assert!(!recreated);
    assert!(matches!(
        db.scheduled_retention_step("must not qualify synthetic mode", 1)
            .await,
        Err(Error::Unavailable)
    ));
    let service = db
        .scheduled_step("synthetic scheduled gap expiry", 1, true)
        .await
        .unwrap();
    assert_eq!(service.stage, LifecycleStage::ExpiredGaps);
    let system_audit:i64=sqlx::query_scalar("SELECT count(*) FROM qgr.audit_events WHERE actor='u_00000000-0000-0000-0000-000000000000' AND action='retention_deleted'").fetch_one(admin).await.unwrap();
    assert_eq!(system_audit, 1);
    let mut service_context = db.pool.begin().await.unwrap();
    sqlx::query("SELECT * FROM qgr_recordings.maintenance_scope()")
        .execute(&mut *service_context)
        .await
        .unwrap();
    let denied=sqlx::query("SELECT qgr_recordings.change_hold($1,$2,2,false,NULL,NULL,NULL,'cannot mutate holds as maintenance')").bind(overlap.id.to_string()).bind(owner.user.to_string()).execute(&mut *service_context).await.unwrap_err();
    assert_eq!(
        denied.as_database_error().unwrap().code().as_deref(),
        Some("42501")
    );
    service_context.rollback().await.unwrap();
    // Releasing one of two holds never deletes the remaining coverage.
    hold.revision = Counter(2);
    hold.active = false;
    db.hold_change(&identity, &actor, hold.clone(), true)
        .await
        .unwrap();
    idle(&db, &identity, &actor).await;
    assert_eq!(count(admin, "coverage", a).await, 1);
    assert_eq!(count(admin, "held", a).await, 1);
    overlap.revision = Counter(2);
    overlap.case_ref = Some("case-two".into());
    db.hold_change(&identity, &actor, overlap.clone(), true)
        .await
        .unwrap();
    let allowed: bool =
        sqlx::query_scalar("SELECT qgr_recordings.held_case_allowed($1,$2,$3,'case-two')")
            .bind(machine.to_string())
            .bind(session.to_string())
            .bind(a.to_string())
            .fetch_one(admin)
            .await
            .unwrap();
    assert!(allowed);
    overlap.revision = Counter(3);
    overlap.case_ref = Some("case-three".into());
    db.hold_change(&identity, &actor, overlap.clone(), true)
        .await
        .unwrap();
    let allowed: bool =
        sqlx::query_scalar("SELECT qgr_recordings.held_case_allowed($1,$2,$3,'case-two')")
            .bind(machine.to_string())
            .bind(session.to_string())
            .bind(a.to_string())
            .fetch_one(admin)
            .await
            .unwrap();
    assert!(!allowed);
    // Another hold after normal retirement covers the existing held copy before old release.
    let mut last = overlap.clone();
    last.id = HoldId::new();
    last.revision = Counter(1);
    last.case_ref = Some("case-last".into());
    db.hold_change(&identity, &actor, last.clone(), true)
        .await
        .unwrap();
    overlap.revision = Counter(4);
    overlap.active = false;
    db.hold_change(&identity, &actor, overlap, true)
        .await
        .unwrap();
    idle(&db, &identity, &actor).await;
    assert_eq!(count(admin, "held", a).await, 1);
    assert_eq!(count(admin, "coverage", a).await, 1);
    last.revision = Counter(2);
    last.active = false;
    db.hold_change(&identity, &actor, last, true).await.unwrap();
    // Two independent workers race, each under database coordination, until released copies
    // expire. Neither worker has a Redis leadership dependency.
    for _ in 0..10 {
        let (x, y) = tokio::join!(
            db.maintenance_step(&identity, &actor, "worker one", 1, true),
            db.maintenance_step(&identity, &actor, "worker two", 1, true)
        );
        x.unwrap();
        y.unwrap();
    }
    idle(&db, &identity, &actor).await;
    assert_eq!(count(admin, "held", a).await, 0);
    assert_eq!(count(admin, "held", b).await, 0);
    let phase: String =
        sqlx::query_scalar("SELECT state FROM qgr_recordings.partitions WHERE month=$1")
            .bind(boundary)
            .fetch_one(admin)
            .await
            .unwrap();
    assert_eq!(phase, "active");
    let references: i64 =
        sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.receipts WHERE session=$1")
            .bind(session.to_string())
            .fetch_one(admin)
            .await
            .unwrap();
    assert_eq!(references, 3);
    let remaining:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM qgr_recordings.held_gaps WHERE id=$1)+(SELECT count(*) FROM qgr_recordings.gaps WHERE id=$1)").bind(gap.id.to_string()).fetch_one(admin).await.unwrap();
    assert_eq!(remaining, 0);
    let audit: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM qgr.audit_events WHERE actor=$1 AND action='hold_changed'",
    )
    .bind(actor.user.to_string())
    .fetch_one(admin)
    .await
    .unwrap();
    assert!(audit >= 7);
    super::super::ingest::tests::state(admin, "quarantine-lifecycle", true).await;
    assert!(matches!(
        db.maintenance_step(&identity, &actor, "quarantined job", 1, true)
            .await,
        Err(Error::Unavailable)
    ));
    db.close().await;
}
