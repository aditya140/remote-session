use super::*;
use crate::{
    Authenticator, JwtVerifier, RecordingRedaction, RecordingValidation, SecretDsn, Store,
    identity::{
        jwt::tests as jwt,
        tests::{proof, request},
    },
};
use base64::{Engine, engine::general_purpose::STANDARD};
use qgenie_remote_proto::{
    ChunkId, ClientId, Counter, GapId, RecordedContent, RecordingEpochId, RecordingRecord,
    wire::{
        Capabilities, LaunchKind, LossReason, Origin, RecordingBatch, RecordingCodec,
        RecordingStream, Session,
    },
};
use sha2::{Digest, Sha256};

fn batch(
    actor: &MachinePrincipal,
    session: SessionId,
    epoch: RecordingEpochId,
    seq: u64,
    at: Timestamp,
    bytes: &[u8],
) -> RecordingBatch {
    let record = RecordingRecord {
        v: 1,
        seq: Counter(seq),
        captured_at: at.clone(),
        elapsed_ms: Counter(0),
        content: RecordedContent::Output {
            data: STANDARD.encode(bytes),
            fragment: 0,
            last: true,
        },
    };
    let raw = format!("{}\n", serde_json::to_string(&record).unwrap());
    let compressed = zstd::bulk::compress(raw.as_bytes(), 3).unwrap();
    RecordingBatch {
        id: ChunkId::new(),
        session,
        machine: actor.machine,
        stream: RecordingStream::Pty,
        stream_epoch: epoch,
        from: Counter(seq),
        to: Counter(seq),
        captured_from: at.clone(),
        captured_to: at,
        codec: RecordingCodec::ZstdJsonl,
        uncompressed_bytes: raw.len() as u32,
        digest: format!("{:x}", Sha256::digest(&compressed)),
        data: STANDARD.encode(compressed),
    }
}
async fn protected(batch: RecordingBatch, created: Timestamp) -> ProtectedRecording {
    let validated = RecordingValidation::default()
        .prepare(
            batch,
            timestamp(OffsetDateTime::now_utc()).unwrap(),
            created,
            None,
        )
        .await
        .unwrap();
    RecordingRedaction::literals(vec![b"fixture-secret".to_vec()])
        .unwrap()
        .protect(validated)
        .await
        .unwrap()
}
pub(crate) async fn state(admin: &sqlx::PgPool, epoch: &str, pending: bool) {
    let mut tx = admin.begin().await.unwrap();
    sqlx::query("SELECT pg_advisory_xact_lock(5856751401703146820)")
        .execute(&mut *tx)
        .await
        .unwrap();
    sqlx::query("UPDATE qgr_recordings.state SET durability='synthetic_single_server',lifecycle_ready=true,recovery_epoch=$1,recovery_pending=$2").bind(epoch).bind(pending).execute(&mut *tx).await.unwrap();
    tx.commit().await.unwrap();
}

pub(crate) async fn database_contract(store: Store, admin: &sqlx::PgPool, migration: &SecretDsn) {
    let verifier = JwtVerifier::with_keys(jwt::config(), jwt::keys(), store.clone()).unwrap();
    let owner = verifier
        .authenticate(&jwt::sign(&jwt::claims("recording-ingest-owner")))
        .await
        .unwrap()
        .user;
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), STANDARD.encode([73; 32])).unwrap();
    let identity = IdentityService::from_file(
        store.clone(),
        "https://gateway.example.invalid".into(),
        file.path(),
    )
    .await
    .unwrap();
    let key = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let enrollment = request(&key);
    let challenge = identity
        .enrollment_challenge(owner, enrollment.clone())
        .await
        .unwrap();
    let enrolled = identity
        .enroll(owner, proof(&key, &challenge))
        .await
        .unwrap();
    let challenge = identity
        .machine_challenge(enrolled.machine, enrollment.key)
        .await
        .unwrap();
    let token = identity
        .machine_token(proof(&key, &challenge))
        .await
        .unwrap();
    let actor = identity.authenticate_machine(&token.token).await.unwrap();
    let host = store
        .connect_host(&actor, "recording-fixture", ClientId::new())
        .await
        .unwrap();
    let created = timestamp(OffsetDateTime::now_utc() - time::Duration::seconds(10)).unwrap();
    let session = Session {
        metadata: Default::default(),
        id: SessionId::new(),
        machine: actor.machine,
        owner,
        kind: LaunchKind::Pty {
            command: vec!["synthetic fixture".into()],
        },
        cwd: "/synthetic".into(),
        origin: Origin::Managed,
        state: qgenie_remote_proto::SessionState::Running,
        created_at: created.clone(),
        ended_at: None,
        head: Counter(0),
        capabilities: Capabilities::default(),
        recording_warning: None,
    };
    store.project(&actor, host, &session).await.unwrap();
    let mut changed_lifetime = session.clone();
    changed_lifetime.created_at =
        timestamp(datetime(&created).unwrap() + time::Duration::seconds(1)).unwrap();
    assert!(matches!(
        store.project(&actor, host, &changed_lifetime).await,
        Err(Error::Forbidden)
    ));
    let mut url = url::Url::parse(&migration.0).unwrap();
    url.set_username("qgr_test_record_ingest").unwrap();
    url.set_password(Some("test-only")).unwrap();
    let database = RecordingDatabase::connect(
        &crate::tests::recording_secret(url.as_str()),
        RecordingRole::Ingest,
        2,
    )
    .await
    .unwrap();
    let capture = timestamp(OffsetDateTime::now_utc() - time::Duration::seconds(1)).unwrap();
    let epoch = RecordingEpochId::new();
    let first = batch(
        &actor,
        session.id,
        epoch,
        1,
        capture.clone(),
        b"before fixture-secret after",
    );
    assert!(matches!(
        database
            .commit_content(
                &identity,
                &actor,
                protected(first.clone(), created.clone()).await,
                true
            )
            .await,
        Err(Error::Unavailable)
    ));
    state(admin, "synthetic-ingest-1", false).await;
    let mut ended = session.clone();
    ended.id = SessionId::new();
    ended.created_at = timestamp(OffsetDateTime::now_utc() - time::Duration::seconds(610)).unwrap();
    ended.ended_at =
        Some(timestamp(OffsetDateTime::now_utc() - time::Duration::seconds(600)).unwrap());
    ended.state = qgenie_remote_proto::SessionState::Exited;
    store.project(&actor, host, &ended).await.unwrap();
    let first_projection: time::OffsetDateTime =
        sqlx::query_scalar("SELECT updated_at FROM qgr.sessions WHERE id=$1")
            .bind(ended.id.to_string())
            .fetch_one(admin)
            .await
            .unwrap();
    store.project(&actor, host, &ended).await.unwrap();
    let retried_projection: time::OffsetDateTime =
        sqlx::query_scalar("SELECT updated_at FROM qgr.sessions WHERE id=$1")
            .bind(ended.id.to_string())
            .fetch_one(admin)
            .await
            .unwrap();
    assert_eq!(
        first_projection, retried_projection,
        "repeated final inventory must not extend an unknown lifetime either"
    );
    let late = batch(
        &actor,
        ended.id,
        RecordingEpochId::new(),
        1,
        capture.clone(),
        b"after native exit",
    );
    assert!(matches!(
        database
            .commit_content(
                &identity,
                &actor,
                protected(late, ended.created_at.clone()).await,
                true
            )
            .await,
        Err(Error::Invalid)
    ));
    let mut rewritten = ended.clone();
    rewritten.ended_at = None;
    assert!(matches!(
        store.project(&actor, host, &rewritten).await,
        Err(Error::Forbidden)
    ));
    rewritten.ended_at = Some(capture.clone());
    assert!(matches!(
        store.project(&actor, host, &rewritten).await,
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        database
            .commit_recording(
                &identity,
                &actor,
                protected(first.clone(), created.clone()).await
            )
            .await,
        Err(Error::Configuration)
    ));
    let committed = database
        .commit_content(
            &identity,
            &actor,
            protected(first.clone(), created.clone()).await,
            true,
        )
        .await
        .unwrap();
    assert!(matches!(committed.disposition, ReceiptDisposition::Stored));
    let retry = database
        .commit_content(
            &identity,
            &actor,
            protected(first.clone(), created.clone()).await,
            true,
        )
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(&committed).unwrap(),
        serde_json::to_value(&retry).unwrap()
    );
    let row=sqlx::query("SELECT c.payload,c.stored_digest,r.immutable FROM qgr_recordings.content c JOIN qgr_recordings.receipts r ON r.machine=c.machine AND r.session=c.session AND r.id=c.id AND r.kind='content' WHERE c.id=$1").bind(first.id.to_string()).fetch_one(admin).await.unwrap();
    let payload: Vec<u8> = row.get("payload");
    assert_eq!(
        format!("{:x}", Sha256::digest(&payload)),
        row.get::<String, _>("stored_digest")
    );
    let raw = zstd::bulk::decompress(&payload, 1024 * 1024).unwrap();
    let record: RecordingRecord = serde_json::from_slice(&raw).unwrap();
    let RecordedContent::Output { data, .. } = record.content else {
        panic!("output");
    };
    assert_eq!(
        STANDARD.decode(data).unwrap(),
        b"before ************** after"
    );
    assert_eq!(row.get::<Value, _>("immutable")["batch"]["data"], "");
    let mut conflict = batch(
        &actor,
        session.id,
        epoch,
        1,
        capture.clone(),
        b"changed source",
    );
    conflict.id = first.id;
    assert!(matches!(
        database
            .commit_content(
                &identity,
                &actor,
                protected(conflict, created.clone()).await,
                true
            )
            .await,
        Err(Error::Conflict)
    ));
    let overlap = batch(
        &actor,
        session.id,
        epoch,
        1,
        capture.clone(),
        b"different batch at same fragment",
    );
    assert!(matches!(
        database
            .commit_content(
                &identity,
                &actor,
                protected(overlap, created.clone()).await,
                true
            )
            .await,
        Err(Error::Conflict)
    ));
    let second = batch(
        &actor,
        session.id,
        epoch,
        2,
        capture.clone(),
        b"receipt transaction rollback",
    );
    sqlx::raw_sql("REVOKE INSERT ON qgr_recordings.receipt_epochs FROM qgr_record_ingest")
        .execute(admin)
        .await
        .unwrap();
    assert!(
        database
            .commit_content(
                &identity,
                &actor,
                protected(second.clone(), created.clone()).await,
                true
            )
            .await
            .is_err()
    );
    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM qgr_recordings.receipts WHERE id=$1)")
            .bind(second.id.to_string())
            .fetch_one(admin)
            .await
            .unwrap();
    assert!(!exists);
    sqlx::raw_sql("GRANT INSERT ON qgr_recordings.receipt_epochs TO qgr_record_ingest")
        .execute(admin)
        .await
        .unwrap();
    database
        .commit_content(
            &identity,
            &actor,
            protected(second, created.clone()).await,
            true,
        )
        .await
        .unwrap();
    let gap = LossRange {
        id: GapId::new(),
        session: session.id,
        stream: RecordingStream::Pty,
        stream_epoch: epoch,
        from: Counter(3),
        to: Counter(4),
        captured_from: capture.clone(),
        captured_to: capture.clone(),
        reason: LossReason::CaptureBackpressure,
    };
    let receipt = database
        .commit_loss(&identity, &actor, gap.clone(), true)
        .await
        .unwrap();
    let repeat = database
        .commit_loss(&identity, &actor, gap.clone(), true)
        .await
        .unwrap();
    assert_eq!(
        serde_json::to_value(receipt).unwrap(),
        serde_json::to_value(repeat).unwrap()
    );
    let mut wrong = gap.clone();
    wrong.to = Counter(5);
    assert!(matches!(
        database.commit_loss(&identity, &actor, wrong, true).await,
        Err(Error::Conflict)
    ));
    state(admin, "synthetic-ingest-2", true).await;
    assert!(matches!(
        database
            .commit_content(
                &identity,
                &actor,
                protected(first.clone(), created.clone()).await,
                true
            )
            .await,
        Err(Error::Unavailable)
    ));
    state(admin, "synthetic-ingest-2", false).await;
    let recovered = database
        .commit_content(
            &identity,
            &actor,
            protected(first.clone(), created.clone()).await,
            true,
        )
        .await
        .unwrap();
    assert_eq!(recovered.recovery_epoch, "synthetic-ingest-2");
    assert_ne!(recovered.id, committed.id);
    assert_eq!(recovered.digest, committed.digest);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.receipts WHERE id=$1")
        .bind(first.id.to_string())
        .fetch_one(admin)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.receipt_epochs WHERE id=$1")
            .bind(first.id.to_string())
            .fetch_one(admin)
            .await
            .unwrap();
    assert_eq!(count, 2);
    // Restore drill component: quarantine, simulate missing retained content/index, then
    // reupload under a new epoch. This does not qualify a real volume/standby restore.
    state(admin, "synthetic-ingest-3", true).await;
    sqlx::query("DELETE FROM qgr_recordings.fragments WHERE chunk=$1")
        .bind(first.id.to_string())
        .execute(admin)
        .await
        .unwrap();
    sqlx::query("DELETE FROM qgr_recordings.content WHERE id=$1")
        .bind(first.id.to_string())
        .execute(admin)
        .await
        .unwrap();
    state(admin, "synthetic-ingest-3", false).await;
    let restored = database
        .commit_content(
            &identity,
            &actor,
            protected(first.clone(), created.clone()).await,
            true,
        )
        .await
        .unwrap();
    assert_eq!(restored.recovery_epoch, "synthetic-ingest-3");
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.content WHERE id=$1")
        .bind(first.id.to_string())
        .fetch_one(admin)
        .await
        .unwrap();
    assert_eq!(count, 1);
    // Gap uploads may precede chunks. Fragment identities accept out-of-order parts while
    // rejecting reuse/contradictory last-part evidence; readers must still expose holes.
    for part in [2, 0, 1] {
        let mut b = batch(
            &actor,
            session.id,
            epoch,
            10,
            capture.clone(),
            b"fragment payload",
        );
        let record = RecordingRecord {
            v: 1,
            seq: Counter(10),
            captured_at: capture.clone(),
            elapsed_ms: Counter(0),
            content: RecordedContent::Output {
                data: STANDARD.encode(b"fragment payload"),
                fragment: part,
                last: part == 2,
            },
        };
        let raw = format!("{}\n", serde_json::to_string(&record).unwrap());
        let compressed = zstd::bulk::compress(raw.as_bytes(), 3).unwrap();
        b.data = STANDARD.encode(&compressed);
        b.digest = format!("{:x}", Sha256::digest(compressed));
        b.uncompressed_bytes = raw.len() as u32;
        database
            .commit_content(&identity, &actor, protected(b, created.clone()).await, true)
            .await
            .unwrap();
    }
    let parts:Vec<i32>=sqlx::query_scalar("SELECT fragment FROM qgr_recordings.fragments WHERE machine=$1 AND session=$2 AND stream_epoch=$3 AND seq=10 ORDER BY fragment").bind(actor.machine.to_string()).bind(session.id.to_string()).bind(epoch.to_string()).fetch_all(admin).await.unwrap();
    assert_eq!(parts, vec![0, 1, 2]);
    let mut old_session = session.clone();
    old_session.id = SessionId::new();
    old_session.created_at =
        timestamp(OffsetDateTime::now_utc() - time::Duration::days(200)).unwrap();
    store.project(&actor, host, &old_session).await.unwrap();
    let old_batch = batch(
        &actor,
        old_session.id,
        RecordingEpochId::new(),
        1,
        timestamp(datetime(&old_session.created_at).unwrap() + time::Duration::seconds(1)).unwrap(),
        b"late old capture",
    );
    let late = database
        .commit_content(
            &identity,
            &actor,
            protected(old_batch.clone(), old_session.created_at.clone()).await,
            true,
        )
        .await
        .unwrap();
    assert!(matches!(late.disposition, ReceiptDisposition::Expired));
    assert_eq!(late.captured_from, old_batch.captured_from);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.content WHERE id=$1")
        .bind(old_batch.id.to_string())
        .fetch_one(admin)
        .await
        .unwrap();
    assert_eq!(count, 0);
    // A private hold preserves new late content directly, without recreating a retired
    // ordinary partition or changing the affected user's public expiry disposition.
    let hold = qgenie_remote_proto::HoldId::new();
    sqlx::query("INSERT INTO qgr_recordings.holds(id,owner,revision,active,case_ref,reason,actor) VALUES($1,$2,1,true,'late-ingest-case','synthetic late-ingest coverage',$2)").bind(hold.to_string()).bind(owner.to_string()).execute(admin).await.unwrap();
    let month = Date::from_calendar_date(
        datetime(&old_batch.captured_from).unwrap().year(),
        datetime(&old_batch.captured_from).unwrap().month(),
        1,
    )
    .unwrap();
    sqlx::query("INSERT INTO qgr_recordings.partitions(month,name,state) VALUES($1,$2,'retired') ON CONFLICT DO NOTHING").bind(month).bind(format!("content_{:04}{:02}",month.year(),u8::from(month.month()))).execute(admin).await.unwrap();
    let held_batch = batch(
        &actor,
        old_session.id,
        old_batch.stream_epoch,
        2,
        old_batch.captured_from.clone(),
        b"late covered content",
    );
    let held_receipt = database
        .commit_content(
            &identity,
            &actor,
            protected(held_batch.clone(), old_session.created_at.clone()).await,
            true,
        )
        .await
        .unwrap();
    assert!(matches!(
        held_receipt.disposition,
        ReceiptDisposition::Expired
    ));
    let kept: i64 =
        sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.held_content WHERE id=$1")
            .bind(held_batch.id.to_string())
            .fetch_one(admin)
            .await
            .unwrap();
    assert_eq!(kept, 1);
    let ordinary: i64 =
        sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.content WHERE id=$1")
            .bind(held_batch.id.to_string())
            .fetch_one(admin)
            .await
            .unwrap();
    assert_eq!(ordinary, 0);
    let loss = LossRange {
        id: GapId::new(),
        session: old_session.id,
        stream: RecordingStream::Pty,
        stream_epoch: old_batch.stream_epoch,
        from: Counter(3),
        to: Counter(4),
        captured_from: old_batch.captured_from.clone(),
        captured_to: old_batch.captured_to.clone(),
        reason: LossReason::DiskFailure,
    };
    let receipt = database
        .commit_loss(&identity, &actor, loss.clone(), true)
        .await
        .unwrap();
    assert!(matches!(receipt.disposition, ReceiptDisposition::Expired));
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.held_gaps WHERE id=$1")
        .bind(loss.id.to_string())
        .fetch_one(admin)
        .await
        .unwrap();
    assert_eq!(kept, 1);
    let recreated: bool = sqlx::query_scalar("SELECT qgr_recordings.ensure_month($1)")
        .bind(month)
        .fetch_one(admin)
        .await
        .unwrap();
    assert!(!recreated);
    // A total deadline terminates active work even though every individual statement stays
    // below its two-second statement limit. The uncommitted barrier cannot escape rollback.
    let before: OffsetDateTime =
        sqlx::query_scalar("SELECT touched_at FROM qgr_recordings.commit_barrier")
            .fetch_one(admin)
            .await
            .unwrap();
    let (mut timed, _, _) = database
        .begin_ingest(&identity, &actor, session.id, true)
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let mut timed_out = false;
    for _ in 0..4 {
        if let Err(error) = sqlx::query("SELECT pg_sleep(1.2)")
            .execute(&mut *timed)
            .await
        {
            assert_eq!(
                error.as_database_error().and_then(|e| e.code()).as_deref(),
                Some("25P04")
            );
            timed_out = true;
            break;
        }
    }
    assert!(timed_out && started.elapsed() < Duration::from_secs(5));
    drop(timed);
    let after: OffsetDateTime =
        sqlx::query_scalar("SELECT touched_at FROM qgr_recordings.commit_barrier")
            .fetch_one(admin)
            .await
            .unwrap();
    assert_eq!(before, after);
    let max = batch(
        &actor,
        session.id,
        RecordingEpochId::new(),
        u64::MAX,
        capture.clone(),
        b"full u64",
    );
    database
        .commit_content(
            &identity,
            &actor,
            protected(max, created.clone()).await,
            true,
        )
        .await
        .unwrap();
    let mut foreign = first.clone();
    foreign.session = SessionId::new();
    assert!(matches!(
        database
            .commit_content(
                &identity,
                &actor,
                protected(foreign, created.clone()).await,
                true
            )
            .await,
        Err(Error::Forbidden)
    ));
    sqlx::query("UPDATE qgr.users SET disabled=true WHERE id=$1")
        .bind(owner.to_string())
        .execute(admin)
        .await
        .unwrap();
    assert!(matches!(
        database
            .commit_content(&identity, &actor, protected(first, created).await, true)
            .await,
        Err(Error::Forbidden)
    ));
    sqlx::query("UPDATE qgr.users SET disabled=false WHERE id=$1")
        .bind(owner.to_string())
        .execute(admin)
        .await
        .unwrap();
    state(admin, "unqualified-after-fixture", true).await;
    sqlx::query("UPDATE qgr_recordings.state SET durability='unqualified',lifecycle_ready=false")
        .execute(admin)
        .await
        .unwrap();
    database.close().await;
}
