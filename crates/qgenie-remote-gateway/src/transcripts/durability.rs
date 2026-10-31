//! Actual synchronous replication and primary-volume loss proof. Synthetic single-server
//! qualification is never used here. Run only through tests/e2e/recording_durability.py.
use crate::{
    Config, Gateway, IdentityService, JwtVerifier, RecordingDatabase, RecordingRedaction,
    RecordingRole, RecordingValidation, SecretDsn, Store,
    identity::{
        jwt::tests as jwt,
        tests::{proof, request},
    },
};
use base64::{Engine, engine::general_purpose::STANDARD};
use qgenie_remote_proto::{
    rest::{AdminHistoryQuery, HistoryQuery, HoldChange},
    wire::*,
    *,
};
use sha2::{Digest, Sha256};
use sqlx::Row;
use std::{path::Path, sync::Arc, time::Duration};

fn role(binding: &SecretDsn, user: &str) -> SecretDsn {
    let mut url = url::Url::parse(&binding.0).unwrap();
    url.set_username(user).unwrap();
    url.set_password(Some("test-only")).unwrap();
    SecretDsn(url.to_string())
}
async fn control(action: &str) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream =
        tokio::net::UnixStream::connect(std::env::var_os("QGR_SYNC_CONTROL_SOCKET").unwrap())
            .await
            .unwrap();
    stream
        .write_all(format!("{{\"action\":\"{action}\"}}\n").as_bytes())
        .await
        .unwrap();
    let mut reply = Vec::new();
    tokio::time::timeout(Duration::from_secs(30), stream.read_to_end(&mut reply))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&reply).unwrap(),
        serde_json::json!({"ok":true})
    );
}
async fn waiting(admin: &sqlx::PgPool, role: &str, expected: bool) {
    let until = std::time::Instant::now() + Duration::from_secs(6);
    loop {
        let waiting:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM pg_stat_activity WHERE datname=current_database() AND usename=$1 AND wait_event='SyncRep')").bind(role).fetch_one(admin).await.unwrap();
        if waiting == expected {
            return;
        }
        assert!(
            std::time::Instant::now() < until,
            "recording commit cancellation did not settle"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}
fn timestamp() -> Timestamp {
    Timestamp::from_unix_millis(
        (time::OffsetDateTime::now_utc().unix_timestamp_nanos() / 1_000_000) as u64,
    )
    .unwrap()
}
fn batch(
    actor: &crate::MachinePrincipal,
    session: SessionId,
    epoch: RecordingEpochId,
    seq: u64,
) -> RecordingBatch {
    batch_at(actor, session, epoch, seq, timestamp())
}
fn batch_at(
    actor: &crate::MachinePrincipal,
    session: SessionId,
    epoch: RecordingEpochId,
    seq: u64,
    at: Timestamp,
) -> RecordingBatch {
    let record = RecordingRecord {
        v: 1,
        seq: Counter(seq),
        captured_at: at.clone(),
        elapsed_ms: Counter(seq),
        content: RecordedContent::Output {
            data: STANDARD.encode(b"before fixture-secret after"),
            fragment: 0,
            last: true,
        },
    };
    let raw = if seq == 1 {
        // A valid request larger than the ordinary 64 KiB HTTP budget exercises
        // the upload route's explicit limit without relaxing unrelated routes.
        use rand::RngCore;
        let mut raw = String::new();
        for fragment in 0..3 {
            let mut data = vec![0; 16 * 1024];
            rand::rng().fill_bytes(&mut data);
            data[..14].copy_from_slice(b"fixture-secret");
            let mut part = record.clone();
            part.content = RecordedContent::Output {
                data: STANDARD.encode(data),
                fragment,
                last: fragment == 2,
            };
            raw.push_str(&serde_json::to_string(&part).unwrap());
            raw.push('\n');
        }
        raw
    } else {
        format!("{}\n", serde_json::to_string(&record).unwrap())
    };
    let bytes = zstd::bulk::compress(raw.as_bytes(), 3).unwrap();
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
        digest: format!("{:x}", Sha256::digest(&bytes)),
        data: STANDARD.encode(bytes),
    }
}
async fn prepared(batch: RecordingBatch, created: Timestamp) -> crate::ProtectedRecording {
    let validated = RecordingValidation::default()
        .prepare(batch, timestamp(), created, None)
        .await
        .unwrap();
    RecordingRedaction::literals(vec![b"fixture-secret".to_vec()])
        .unwrap()
        .protect(validated)
        .await
        .unwrap()
}
async fn compose(
    config: &Config,
    store: &Store,
    fixture: &crate::identity::DirectoryFixture,
    key: &Path,
) -> Gateway {
    let mut jwt_config = jwt::config();
    jwt_config.tenant = fixture.config.tenant.clone();
    let verifier =
        Arc::new(JwtVerifier::with_keys(jwt_config, jwt::keys(), store.clone()).unwrap());
    let mut gateway = Gateway::connect(config, verifier).await.unwrap();
    gateway.identity = Some(Arc::new(
        IdentityService::from_file(
            gateway.store.clone(),
            "https://gateway.example.invalid".into(),
            key,
        )
        .await
        .unwrap(),
    ));
    gateway
        .enable_directory(fixture.config.clone())
        .await
        .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires disposable synchronous pair and private control socket; tests/e2e/recording_durability.py"]
async fn actual_synchronous_receipts_survive_primary_volume_loss() {
    let primary = SecretDsn::from_file(Path::new(
        &std::env::var_os("QGR_SYNC_PRIMARY_URL_FILE").unwrap(),
    ))
    .unwrap();
    let standby = SecretDsn::from_file(Path::new(
        &std::env::var_os("QGR_SYNC_STANDBY_URL_FILE").unwrap(),
    ))
    .unwrap();
    let parent = sqlx::PgPool::connect(&primary.0).await.unwrap();
    let database = format!(
        "qgr_sync_{}",
        UserId::new()
            .as_bytes()
            .iter()
            .map(|v| format!("{v:02x}"))
            .collect::<String>()
    );
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {database}")))
        .execute(&parent)
        .await
        .unwrap();
    parent.close().await;
    let mut url = url::Url::parse(&primary.0).unwrap();
    url.set_path(&database);
    let migration = SecretDsn(url.to_string());
    Store::migrate(&migration).await.unwrap();
    let admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(&migration.0)
        .await
        .unwrap();
    sqlx::raw_sql("CREATE ROLE qgr_sync_app LOGIN PASSWORD 'test-only'; GRANT qgr_app TO qgr_sync_app;
      CREATE ROLE qgr_sync_ingest LOGIN PASSWORD 'test-only'; GRANT qgr_record_ingest TO qgr_sync_ingest;
      CREATE ROLE qgr_sync_owner LOGIN PASSWORD 'test-only'; GRANT qgr_history_owner TO qgr_sync_owner;
      CREATE ROLE qgr_sync_admin LOGIN PASSWORD 'test-only'; GRANT qgr_history_admin TO qgr_sync_admin;
      CREATE ROLE qgr_sync_lifecycle LOGIN PASSWORD 'test-only'; GRANT qgr_record_lifecycle TO qgr_sync_lifecycle;").execute(&admin).await.unwrap();
    let ready:bool=sqlx::query_scalar("SELECT current_setting('synchronous_standby_names')<>'' AND EXISTS(SELECT 1 FROM pg_stat_replication WHERE state='streaming' AND sync_state='sync')").fetch_one(&admin).await.unwrap();
    assert!(ready);
    sqlx::raw_sql("BEGIN; SELECT pg_advisory_xact_lock(5856751401703146820); UPDATE qgr_recordings.state SET durability='synchronous_standby',lifecycle_ready=true,recovery_pending=false,recovery_epoch='sync-proof-1'; COMMIT;").execute(&admin).await.unwrap();
    let store = Store::connect(&role(&migration, "qgr_sync_app"), 4, false)
        .await
        .unwrap();
    let fixture = crate::identity::DirectoryFixture::new().await;
    let key_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(key_file.path(), STANDARD.encode([79; 32])).unwrap();
    let mut config = Config {
        node_id: "sync-proof".into(),
        listen: "127.0.0.1:8081".parse().unwrap(),
        health_listen: "127.0.0.1:9091".parse().unwrap(),
        database: role(&migration, "qgr_sync_app"),
        audit_database: None,
        pool_size: 4,
    };
    let gateway = compose(&config, &store, &fixture, key_file.path())
        .await
        .enable_recording_lifecycle(&role(&migration, "qgr_sync_lifecycle"))
        .await
        .unwrap()
        .enable_recordings(
            &role(&migration, "qgr_sync_ingest"),
            &role(&migration, "qgr_sync_owner"),
            &role(&migration, "qgr_sync_admin"),
            RecordingRedaction::literals(vec![b"fixture-secret".to_vec()]).unwrap(),
        )
        .await
        .unwrap();
    let mut claims = jwt::claims("synchronous-recording-owner");
    claims["tid"] = serde_json::json!(fixture.config.tenant);
    claims["oid"] = serde_json::json!("cccccccc-cccc-4ccc-8ccc-cccccccccccc");
    let owner_token = jwt::sign(&claims);
    let owner = gateway
        .auth
        .authenticate(&jwt::sign(&claims))
        .await
        .unwrap();
    claims["roles"] = serde_json::json!([jwt::config().admin_role]);
    let admin_token = jwt::sign(&claims);
    let administrator = gateway.auth.authenticate(&admin_token).await.unwrap();
    let identity = gateway.identity.as_ref().unwrap();
    let key = ed25519_dalek::SigningKey::from_bytes(&rand::random());
    let enrollment = request(&key);
    let challenge = identity
        .enrollment_challenge(owner.user, enrollment.clone())
        .await
        .unwrap();
    let enrolled = identity
        .enroll(owner.user, proof(&key, &challenge))
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
        .connect_host(&actor, "sync-proof", ClientId::new())
        .await
        .unwrap();
    let created = timestamp();
    let session = Session {
        metadata: Default::default(),
        id: SessionId::new(),
        machine: actor.machine,
        owner: owner.user,
        kind: LaunchKind::Pty {
            command: vec!["synthetic durability fixture".into()],
        },
        cwd: "/synthetic".into(),
        origin: Origin::Managed,
        state: SessionState::Running,
        created_at: created.clone(),
        ended_at: None,
        head: Counter(0),
        capabilities: Capabilities::default(),
        recording_warning: None,
    };
    store.project(&actor, host, &session).await.unwrap();
    let ingest = RecordingDatabase::connect(
        &role(&migration, "qgr_sync_ingest"),
        RecordingRole::Ingest,
        2,
    )
    .await
    .unwrap();
    let reader = RecordingDatabase::connect(
        &role(&migration, "qgr_sync_owner"),
        RecordingRole::OwnerRead,
        2,
    )
    .await
    .unwrap();
    let privileged = RecordingDatabase::connect(
        &role(&migration, "qgr_sync_admin"),
        RecordingRole::AdminRead,
        2,
    )
    .await
    .unwrap();
    let epoch = RecordingEpochId::new();
    let first = batch(&actor, session.id, epoch, 1);
    let receipt = ingest
        .commit_recording(
            identity,
            &actor,
            prepared(first.clone(), created.clone()).await,
        )
        .await
        .unwrap();
    assert!(matches!(receipt.disposition, ReceiptDisposition::Stored));
    let http_receipt = super::api::tests::production_recording_routes(
        &gateway,
        &token.token,
        &owner_token,
        &admin_token,
        first.clone(),
        &receipt,
    )
    .await;
    assert_eq!(http_receipt.backend_epoch, "sync-proof-1");
    let history = HistoryQuery {
        stream_epoch: epoch,
        after: None,
        limit: 1,
    };
    assert_eq!(
        reader
            .owner_history(identity, &owner, session.id, history.clone())
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    let admin_query = AdminHistoryQuery {
        history: history.clone(),
        reason: "actual synchronous audit qualification".into(),
        case_ref: None,
        include_held: false,
    };
    assert_eq!(
        privileged
            .admin_history(identity, &administrator, session.id, admin_query.clone())
            .await
            .unwrap()
            .items
            .len(),
        1
    );
    use tower::ServiceExt;
    let hold = HoldChange {
        id: HoldId::new(),
        user: owner.user,
        revision: Counter(1),
        active: true,
        captured_from: None,
        captured_to: None,
        case_ref: Some("sync-proof-case".into()),
        reason: "actual synchronous placement".into(),
    };
    let response = gateway
        .router()
        .oneshot(
            axum::http::Request::builder()
                .method("POST")
                .uri("/v1/admin/holds")
                .header("authorization", format!("Bearer {admin_token}"))
                .body(axum::body::Body::from(serde_json::to_vec(&hold).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), axum::http::StatusCode::OK);
    let lifecycle_binding = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        lifecycle_binding.path(),
        role(&migration, "qgr_sync_lifecycle").0.as_bytes(),
    )
    .unwrap();
    let job = tokio::process::Command::new(std::env::var_os("QGR_GATEWAY_BINARY").unwrap())
        .args(["jobs", "--database-url-file"])
        .arg(lifecycle_binding.path())
        .args(["--max-steps", "32"])
        .output()
        .await
        .unwrap();
    assert!(job.status.success(), "qualified lifecycle process failed");
    let held: i64 =
        sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.held_content WHERE id=$1")
            .bind(first.id.to_string())
            .fetch_one(&admin)
            .await
            .unwrap();
    assert_eq!(held, 1);
    // Discover held-only history without exposing it to the owner or another case.
    let old =
        Timestamp::from_unix_millis(timestamp().unix_millis() - 120 * 24 * 60 * 60 * 1000).unwrap();
    let mut historical = session.clone();
    historical.id = SessionId::new();
    historical.created_at = old.clone();
    historical.ended_at = Some(old.clone());
    historical.state = SessionState::Exited;
    store.project(&actor, host, &historical).await.unwrap();
    let old_epoch = RecordingEpochId::new();
    let old_receipt = ingest
        .commit_recording(
            identity,
            &actor,
            prepared(
                batch_at(&actor, historical.id, old_epoch, 1, old.clone()),
                old.clone(),
            )
            .await,
        )
        .await
        .unwrap();
    assert!(matches!(
        old_receipt.disposition,
        ReceiptDisposition::Expired
    ));
    let streams = qgenie_remote_proto::rest::HistoryStreamsQuery {
        after: None,
        limit: 32,
    };
    assert!(
        reader
            .owner_history_streams(identity, &owner, historical.id, streams.clone())
            .await
            .unwrap()
            .streams
            .is_empty()
    );
    let held_query = qgenie_remote_proto::rest::AdminHistoryStreamsQuery {
        history: streams.clone(),
        reason: "held-only stream discovery".into(),
        case_ref: Some("sync-proof-case".into()),
        include_held: true,
    };
    let indexed = privileged
        .admin_history_streams(identity, &administrator, historical.id, held_query.clone())
        .await
        .unwrap();
    assert_eq!(indexed.streams.len(), 1);
    assert_eq!(indexed.streams[0].stream_epoch, old_epoch);
    let mut wrong = held_query;
    wrong.case_ref = Some("unrelated-case".into());
    assert!(matches!(
        privileged
            .admin_history_streams(identity, &administrator, historical.id, wrong)
            .await,
        Err(crate::Error::Forbidden)
    ));
    println!(
        "Production ingest, audited history, signed hold HTTP and actual jobs process passed synchronous qualification"
    );
    ingest
        .preflight_start(identity, owner.user, actor.machine, owner.expires_at)
        .await
        .unwrap();
    assert!(matches!(
        ingest
            .preflight_start(identity, owner.user, MachineId::new(), owner.expires_at)
            .await,
        Err(crate::Error::Forbidden)
    ));
    control("pause_standby").await;
    assert!(
        tokio::time::timeout(
            Duration::from_secs(6),
            ingest.preflight_start(identity, owner.user, actor.machine, owner.expires_at)
        )
        .await
        .unwrap()
        .is_err()
    );
    // Authority commits own cancellation even if their request future disappears.
    // A size-one app pool must recover its slot after a paused synchronous commit.
    let authority = Store::connect(&role(&migration, "qgr_sync_app"), 1, false)
        .await
        .unwrap();
    let mut tx = authority.transaction().await.unwrap();
    sqlx::query("SET LOCAL synchronous_commit='remote_apply'")
        .execute(&mut *tx)
        .await
        .unwrap();
    Store::append_audit(
        &mut tx,
        &crate::AuditEvent {
            actor: owner.user,
            action: crate::AuditAction::SessionStarted,
            machine: Some(actor.machine),
            session: Some(session.id),
            metadata: Default::default(),
        },
    )
    .await
    .unwrap();
    let caller = tokio::spawn(tx.commit());
    waiting(&admin, "qgr_sync_app", true).await;
    caller.abort();
    let _ = caller.await;
    waiting(&admin, "qgr_sync_app", false).await;
    tokio::time::timeout(
        Duration::from_secs(3),
        sqlx::query("SELECT 1").execute(&authority.pool),
    )
    .await
    .unwrap()
    .unwrap();
    authority.close().await;
    let second = batch(&actor, session.id, epoch, 2);
    for attempt in 0..2 {
        let started = std::time::Instant::now();
        let committed = tokio::time::timeout(
            Duration::from_secs(8),
            ingest.commit_recording(
                identity,
                &actor,
                prepared(second.clone(), created.clone()).await,
            ),
        )
        .await
        .expect("recording COMMIT must have a bounded total lifetime");
        assert!(
            committed.is_err(),
            "paused replica produced a reclaimable receipt on attempt {attempt}"
        );
        assert!(started.elapsed() < Duration::from_secs(6));
    }
    let original:serde_json::Value=sqlx::query_scalar("SELECT receipt FROM qgr_recordings.receipt_epochs WHERE id=$1 AND recovery_epoch='sync-proof-1'").bind(second.id.to_string()).fetch_one(&admin).await.unwrap();
    let third = batch(&actor, session.id, epoch, 3);
    let owned_ingest = ingest.clone();
    let owned_identity = identity.clone();
    let owned_actor = actor.clone();
    let upload = third.clone();
    let birth = created.clone();
    let caller = tokio::spawn(async move {
        owned_ingest
            .commit_recording(&owned_identity, &owned_actor, prepared(upload, birth).await)
            .await
    });
    waiting(&admin, "qgr_sync_ingest", true).await;
    caller.abort();
    let _ = caller.await;
    waiting(&admin, "qgr_sync_ingest", false).await;
    let audit_page = tokio::time::timeout(
        Duration::from_secs(6),
        privileged.admin_history(identity, &administrator, session.id, admin_query.clone()),
    )
    .await
    .unwrap();
    assert!(
        audit_page.is_err(),
        "admin page bytes must not be released while the audit commit cannot replicate"
    );
    let index = privileged
        .admin_history_streams(
            identity,
            &administrator,
            session.id,
            qgenie_remote_proto::rest::AdminHistoryStreamsQuery {
                history: streams,
                reason: "index audit cannot release metadata without replication".into(),
                case_ref: None,
                include_held: false,
            },
        )
        .await;
    assert!(
        index.is_err(),
        "stream metadata must wait for its audit commit too"
    );
    let mut interrupted =
        tokio::process::Command::new(std::env::var_os("QGR_GATEWAY_BINARY").unwrap())
            .args(["jobs", "--database-url-file"])
            .arg(lifecycle_binding.path())
            .args(["--max-steps", "32"])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
    waiting(&admin, "qgr_sync_lifecycle", true).await;
    let signal = tokio::process::Command::new("kill")
        .args(["-TERM", &interrupted.id().unwrap().to_string()])
        .status()
        .await
        .unwrap();
    assert!(signal.success());
    let status = tokio::time::timeout(Duration::from_secs(6), interrupted.wait())
        .await
        .unwrap()
        .unwrap();
    assert!(
        !status.success(),
        "interrupted maintenance must not report a completed job"
    );
    waiting(&admin, "qgr_sync_lifecycle", false).await;
    control("resume_standby").await;
    let resumed = ingest
        .commit_recording(
            identity,
            &actor,
            prepared(second.clone(), created.clone()).await,
        )
        .await
        .unwrap();
    assert_eq!(serde_json::to_value(&resumed).unwrap(), original);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.receipts WHERE id=$1")
        .bind(second.id.to_string())
        .fetch_one(&admin)
        .await
        .unwrap();
    assert_eq!(count, 1);
    ingest
        .commit_recording(identity, &actor, prepared(third, created.clone()).await)
        .await
        .unwrap();
    let before = reader
        .owner_history(identity, &owner, session.id, history.clone())
        .await
        .unwrap();
    let old_cursor = before.next.unwrap();
    println!(
        "Paused replication denied both original and duplicate receipts; resumed retry reconciled the same receipt"
    );
    control("promote_after_primary_loss").await;
    let mut target = url::Url::parse(&standby.0).unwrap();
    target.set_path(&database);
    let promoted = SecretDsn(target.to_string());
    let restored = sqlx::PgPool::connect(&promoted.0).await.unwrap();
    let rows=sqlx::query("SELECT c.id,c.payload,c.stored_digest FROM qgr_recordings.content c WHERE c.session=$1 ORDER BY c.id").bind(session.id.to_string()).fetch_all(&restored).await.unwrap();
    assert_eq!(rows.len(), 3);
    for row in rows {
        let bytes: Vec<u8> = row.get("payload");
        assert_eq!(
            format!("{:x}", Sha256::digest(&bytes)),
            row.get::<String, _>("stored_digest")
        );
        let raw = zstd::bulk::decompress(&bytes, 1024 * 1024).unwrap();
        for line in raw.split(|b| *b == b'\n').filter(|v| !v.is_empty()) {
            let record: RecordingRecord = serde_json::from_slice(line).unwrap();
            if let RecordedContent::Output { data, .. } = record.content {
                assert!(
                    !STANDARD
                        .decode(data)
                        .unwrap()
                        .windows(14)
                        .any(|w| w == b"fixture-secret")
                );
            }
        }
    }
    let preserved: i64 =
        sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.held_cases WHERE hold=$1")
            .bind(hold.id.to_string())
            .fetch_one(&restored)
            .await
            .unwrap();
    assert_eq!(preserved, 4);
    let audits:i64=sqlx::query_scalar("SELECT count(*) FROM qgr.audit_events WHERE action IN ('transcript_read','hold_changed','recording_maintenance')").fetch_one(&restored).await.unwrap();
    assert!(audits >= 3);
    sqlx::raw_sql("BEGIN; SELECT pg_advisory_xact_lock(5856751401703146820); UPDATE qgr_recordings.state SET recovery_pending=true,lifecycle_ready=false,recovery_epoch='sync-proof-promoted'; COMMIT;").execute(&restored).await.unwrap();
    config.database = role(&promoted, "qgr_sync_app");
    let recovered_store = Store::connect(&config.database, 4, false).await.unwrap();
    let recovered_gateway = compose(&config, &recovered_store, &fixture, key_file.path()).await;
    let recovered_identity = recovered_gateway.identity.as_ref().unwrap();
    let recovered_reader = RecordingDatabase::connect(
        &role(&promoted, "qgr_sync_owner"),
        RecordingRole::OwnerRead,
        1,
    )
    .await
    .unwrap();
    assert!(matches!(
        recovered_reader
            .owner_history(recovered_identity, &owner, session.id, history.clone())
            .await,
        Err(crate::Error::Unavailable)
    ));
    // Only after receipt/hold/audit coverage is checked may the operator reconcile this epoch.
    // The promoted single server still cannot produce new reclaimable receipts/admin audits.
    sqlx::raw_sql("BEGIN; SELECT pg_advisory_xact_lock(5856751401703146820); UPDATE qgr_recordings.state SET recovery_pending=false,lifecycle_ready=true; COMMIT;").execute(&restored).await.unwrap();
    assert!(matches!(
        recovered_reader
            .owner_history(
                recovered_identity,
                &owner,
                session.id,
                HistoryQuery {
                    after: Some(old_cursor),
                    ..history.clone()
                }
            )
            .await,
        Err(crate::Error::HistoryTruncated)
    ));
    let page = recovered_reader
        .owner_history(
            recovered_identity,
            &owner,
            session.id,
            HistoryQuery {
                limit: 4,
                ..history
            },
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 3);
    let recovered_ingest = RecordingDatabase::connect(
        &role(&promoted, "qgr_sync_ingest"),
        RecordingRole::Ingest,
        1,
    )
    .await
    .unwrap();
    assert!(matches!(
        recovered_ingest
            .commit_recording(recovered_identity, &actor, prepared(second, created).await)
            .await,
        Err(crate::Error::Unavailable)
    ));
    let recovered_admin = RecordingDatabase::connect(
        &role(&promoted, "qgr_sync_admin"),
        RecordingRole::AdminRead,
        1,
    )
    .await
    .unwrap();
    assert!(matches!(
        recovered_admin
            .admin_history(recovered_identity, &administrator, session.id, admin_query)
            .await,
        Err(crate::Error::Unavailable)
    ));
    for pool in [
        &ingest,
        &reader,
        &privileged,
        &recovered_reader,
        &recovered_ingest,
        &recovered_admin,
    ] {
        pool.close().await;
    }
    gateway.shutdown().await;
    recovered_gateway.shutdown().await;
    store.close().await;
    recovered_store.close().await;
    admin.close().await;
    restored.close().await;
    std::fs::write(
        std::env::var_os("QGR_SYNC_PROOF_FILE").unwrap(),
        b"{\"complete\":true}",
    )
    .unwrap();
    println!(
        "Acknowledged redacted content, held references and audits survived loss of the primary tmpfs volume; quarantine and new epoch were enforced"
    );
}
