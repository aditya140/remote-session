//! Real HTTPS + Unix IPC uploader composition with a recoverable seeded native spool.
//! Native PTY capture itself is exercised by sessiond's independent process/I/O tests.
use super::{NativeDaemon, fixture::*, metadata};
use axum::{
    body::Body,
    extract::{Request, State},
    middleware::{self, Next},
    response::Response,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use qgenie_remote_gateway::{
    Gateway, JwtConfig, RecordingRedaction, RejectAuthentication, SecretDsn,
};
use qgenie_remote_host::{
    CredentialOptions, GatewayApi, Identity, NativeClient, RecordingConsent, UploadProgress,
    oauth::OAuth,
};
use qgenie_remote_proto::{
    ipc::RemoteScope,
    rest::{HistoryItem, HistoryPage, HistoryQuery},
    wire::*,
    *,
};
use qgenie_remote_sessiond::{
    CaptureContent, CapturedRecord, RecordingEncoder, RecordingIdentity, RecordingSpool,
    RedactionRules, SpoolConfig,
};
use std::{
    os::unix::fs::PermissionsExt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};

fn private_directory() -> tempfile::TempDir {
    tempfile::Builder::new()
        .permissions(std::fs::Permissions::from_mode(0o700))
        .tempdir()
        .unwrap()
}

async fn lost_chunk_ack(
    State(pending): State<Arc<AtomicBool>>,
    request: Request,
    next: Next,
) -> Response {
    let lose = request.uri().path() == "/v1/recordings/chunks" && pending.load(Ordering::SeqCst);
    let response = next.run(request).await;
    if lose && response.status().is_success() && pending.swap(false, Ordering::SeqCst) {
        Response::new(Body::from_stream(futures_util::stream::once(async {
            Err::<Vec<u8>, _>(std::io::Error::new(
                std::io::ErrorKind::ConnectionReset,
                "synthetic lost chunk ACK",
            ))
        })))
    } else {
        response
    }
}
fn now() -> Timestamp {
    Timestamp::from_unix_millis(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64,
    )
    .unwrap()
}
fn seed(spool: &mut RecordingSpool, identity: RecordingIdentity, unfinished: bool) {
    let at = now();
    spool.register(&identity, at.clone()).unwrap();
    let seq = spool.reserve(&identity, at.clone()).unwrap();
    let mut encoder = RecordingEncoder::new(identity.clone(), RedactionRules::default());
    let mut batches = encoder
        .push(CapturedRecord {
            seq,
            captured_at: at.clone(),
            elapsed_ms: Counter(0),
            content: CaptureContent::Output(
                b"before pipeline-secret after\x1b]52;c;Zml4dHVyZQ==\x07".to_vec(),
            ),
        })
        .unwrap();
    batches.extend(encoder.finish().unwrap());
    spool.commit(&identity, &batches).unwrap();
    if unfinished {
        spool.reserve(&identity, at).unwrap();
    } else {
        spool.seal(&identity).unwrap();
    }
}

#[tokio::test]
#[ignore = "requires owned synchronous pair; tests/e2e/recording_durability.py"]
async fn native_spool_https_upload_lost_ack_and_epoch_reclamation() {
    assert!(
        std::env::var_os("QGR_SYNC_PIPELINE_PROOF_FILE").is_some(),
        "use the owned synchronous harness"
    );
    let database = Database::create().await;
    let synchronous:bool=sqlx::query_scalar("SELECT current_setting('synchronous_standby_names')<>'' AND EXISTS(SELECT FROM pg_stat_replication WHERE state='streaming' AND sync_state='sync')").fetch_one(&database.admin).await.unwrap();
    assert!(synchronous);
    sqlx::raw_sql("CREATE ROLE qgr_pipeline_ingest LOGIN PASSWORD 'test-only'; GRANT qgr_record_ingest TO qgr_pipeline_ingest;
        CREATE ROLE qgr_pipeline_owner LOGIN PASSWORD 'test-only'; GRANT qgr_history_owner TO qgr_pipeline_owner;
        CREATE ROLE qgr_pipeline_admin LOGIN PASSWORD 'test-only'; GRANT qgr_history_admin TO qgr_pipeline_admin;
        BEGIN; SELECT pg_advisory_xact_lock(5856751401703146820); UPDATE qgr_recordings.state SET durability='synchronous_standby',lifecycle_ready=true,recovery_pending=false,recovery_epoch='pipeline-epoch-1'; COMMIT;")
        .execute(&database.admin).await.unwrap();
    let name: String = sqlx::query_scalar("SELECT current_database()")
        .fetch_one(&database.admin)
        .await
        .unwrap();
    let path = std::env::var_os("QGR_PG_TEST_URL_FILE").unwrap();
    let mut url = url::Url::parse(std::fs::read_to_string(path).unwrap().trim()).unwrap();
    url.set_path(&name);
    url.set_password(Some("test-only")).unwrap();
    let mut role = |user: &str| {
        url.set_username(user).unwrap();
        SecretDsn::from_file(private(url.as_str().as_bytes()).path()).unwrap()
    };
    let ingest = role("qgr_pipeline_ingest");
    let owner = role("qgr_pipeline_owner");
    let admin = role("qgr_pipeline_admin");
    let tls = Tls::new();
    let provider = Provider::default();
    let issuer = tls.bind().await.serve(provider.router()).await;
    let oauth = OAuth::new(provider.config(&issuer.origin, tls.ca.path().into())).unwrap();
    let (_stop, mut cancel) = tokio::sync::watch::channel(false);
    let tokens = oauth
        .device(&mut cancel)
        .await
        .unwrap()
        .wait(&mut cancel)
        .await
        .unwrap();
    let edge = tls.bind().await;
    let origin = edge.origin();
    let key = private(STANDARD.encode([84; 32]).as_bytes());
    let graph_secret = private(b"synthetic-graph-application-secret");
    let routing_key = private(STANDARD.encode([83; 32]).as_bytes());
    let redis = std::env::var_os("QGR_REDIS_TEST_URL_FILE").unwrap();
    let mut nodes = Vec::new();
    for node in ["recording-ui-proof", "recording-control-peer"] {
        let gateway = Gateway::connect(&database.config, Arc::new(RejectAuthentication))
            .await
            .unwrap()
            .enable_identity(
                JwtConfig {
                    issuer: ISSUER.into(),
                    tenant: TENANT.into(),
                    audience: AUDIENCE.into(),
                    remote_scope: "Remote.Control".into(),
                    admin_role: "Remote.Admin".into(),
                    jwks_url: format!("{}/keys", issuer.origin),
                    ca_bundle: Some(tls.ca.path().into()),
                },
                origin.clone(),
                key.path(),
            )
            .await
            .unwrap()
            .enable_directory(qgenie_remote_gateway::DirectoryConfig {
                tenant: TENANT.into(),
                client_id: "33333333-3333-4333-8333-333333333333".into(),
                credential_file: graph_secret.path().into(),
                token_url: Some(format!("{}/graph-token", issuer.origin)),
                graph_base: Some(format!("{}/v1.0", issuer.origin)),
                ca_bundle: Some(tls.ca.path().into()),
            })
            .await
            .unwrap()
            .enable_recordings(
                &ingest,
                &owner,
                &admin,
                RecordingRedaction::literals(vec![b"pipeline-secret".to_vec()]).unwrap(),
            )
            .await
            .unwrap()
            .enable_routing(
                qgenie_remote_gateway::RoutingConfig::from_files(
                    node.into(),
                    std::path::Path::new(&redis),
                    routing_key.path(),
                    None,
                )
                .unwrap(),
            )
            .await
            .unwrap();
        nodes.push(gateway);
    }
    let peer_gateway = nodes.pop().unwrap();
    let gateway = nodes.pop().unwrap();
    let peer_server = tls.bind().await.serve(peer_gateway.router()).await;
    let lose = Arc::new(AtomicBool::new(true));
    let server = edge
        .serve(
            gateway
                .router()
                .layer(middleware::from_fn_with_state(lose.clone(), lost_chunk_ack)),
        )
        .await;
    let api = GatewayApi::new(&origin, Some(tls.ca.path())).unwrap();
    let directory = private_directory();
    let mut identity = Identity::open(
        directory.path(),
        &origin,
        "recording-pipeline",
        CredentialOptions {
            prefer_keyring: false,
            allow_private_file: true,
        },
    )
    .await
    .unwrap();
    let binding = api
        .enroll(
            &mut identity,
            &tokens.access,
            metadata("recording-pipeline"),
            RecordingConsent::acknowledged(),
        )
        .await
        .unwrap();
    let credentials = identity.machine_credentials().unwrap();
    let authentication = api.authenticate_machine(&credentials).await.unwrap();
    let state = private_directory();
    let native_path = std::fs::canonicalize(state.path()).unwrap();
    let at = now();
    let session = Session {
        metadata: Default::default(),
        id: SessionId::new(),
        machine: binding.machine,
        owner: binding.owner,
        kind: LaunchKind::Pty {
            command: vec!["recording fixture".into()],
        },
        cwd: native_path.to_str().unwrap().into(),
        origin: Origin::Managed,
        state: SessionState::Exited,
        created_at: at.clone(),
        ended_at: None,
        head: Counter(0),
        capabilities: Capabilities::default(),
        recording_warning: None,
    };
    // Synthetic authoritative projection for the pre-existing spool fixture. No local
    // preview session is adopted or exposed through this setup.
    sqlx::query(
        "INSERT INTO qgr.sessions(id,machine,owner,body,state) VALUES($1,$2,$3,$4,'exited')",
    )
    .bind(session.id.to_string())
    .bind(binding.machine.to_string())
    .bind(binding.owner.to_string())
    .bind(serde_json::to_value(&session).unwrap())
    .execute(&database.admin)
    .await
    .unwrap();
    let identity_record = RecordingIdentity {
        origin: origin.clone(),
        session: session.id,
        machine: binding.machine,
        owner: binding.owner,
        stream: RecordingStream::Pty,
        stream_epoch: RecordingEpochId::new(),
    };
    let foreign_scope = RemoteScope {
        origin: "https://other-recording.invalid".into(),
        owner: UserId::new(),
        machine: MachineId::new(),
    };
    let mut spool =
        RecordingSpool::open(SpoolConfig::new(native_path.join("recordings")), at.clone()).unwrap();
    seed(&mut spool, identity_record.clone(), true);
    seed(
        &mut spool,
        RecordingIdentity {
            origin: foreign_scope.origin.clone(),
            owner: foreign_scope.owner,
            machine: foreign_scope.machine,
            session: SessionId::new(),
            stream: RecordingStream::Pty,
            stream_epoch: RecordingEpochId::new(),
        },
        false,
    );
    drop(spool);
    let mut daemon = NativeDaemon::start(&native_path, &native_path);
    daemon
        .ready(&qgenie_remote_client::Client::local(&native_path))
        .await;
    let native = NativeClient::new(
        native_path.clone(),
        RemoteScope {
            origin: origin.clone(),
            owner: binding.owner,
            machine: binding.machine,
        },
    )
    .unwrap();
    native.register_binding().await.unwrap();
    let challenge = native.challenge(ClientId::new(), Counter(1)).await.unwrap();
    let access = native.activate(&challenge, 10_000, false).await.unwrap();
    let initial = native.recording_status(&access).await.unwrap();
    assert_eq!(initial.chunks, Counter(1));
    assert!(initial.gaps.0 > 0);
    while native.recording_status(&access).await.unwrap().gaps.0 > 0 {
        assert_eq!(
            native
                .upload_recording(&api, &credentials, &authentication, &access)
                .await
                .unwrap(),
            UploadProgress::Reclaimed
        );
    }
    assert!(
        native
            .upload_recording(&api, &credentials, &authentication, &access)
            .await
            .is_err()
    );
    assert!(!lose.load(Ordering::SeqCst));
    assert_eq!(
        native.recording_status(&access).await.unwrap().chunks,
        Counter(1)
    );
    let committed: i64 =
        sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.receipts r WHERE session=$1 AND EXISTS(SELECT 1 FROM qgr_recordings.content c WHERE c.id=r.id)")
            .bind(session.id.to_string())
            .fetch_one(&database.admin)
            .await
            .unwrap();
    assert_eq!(committed, 1);
    assert_eq!(
        native
            .upload_recording(&api, &credentials, &authentication, &access)
            .await
            .unwrap(),
        UploadProgress::Reclaimed
    );
    assert_eq!(
        native
            .upload_recording(&api, &credentials, &authentication, &access)
            .await
            .unwrap(),
        UploadProgress::Idle
    );
    let client = qgenie_remote_client::GatewayClient::new(qgenie_remote_client::GatewayConfig {
        origin: origin.clone(),
        ca_bundle: Some(tls.ca.path().into()),
    })
    .unwrap();
    assert_eq!(
        client.machines(&tokens.access).await.unwrap()[0].id,
        binding.machine
    );
    let streams = client
        .history_streams(
            &tokens.access,
            session.id,
            qgenie_remote_proto::rest::HistoryStreamsQuery {
                after: None,
                limit: 1,
            },
        )
        .await
        .unwrap();
    assert_eq!(streams.streams.len(), 1);
    assert_eq!(
        streams.streams[0].stream_epoch,
        identity_record.stream_epoch
    );
    assert!(streams.next.is_none());
    let query = HistoryQuery {
        stream_epoch: streams.streams[0].stream_epoch,
        after: None,
        limit: 4,
    };
    assert_eq!(
        client
            .history(&authentication.token, session.id, query.clone())
            .await
            .unwrap_err()
            .code,
        ErrorCode::Unauthenticated
    );
    let history: HistoryPage = client
        .history(&tokens.access, session.id, query.clone())
        .await
        .unwrap();
    for item in &history.items {
        if let HistoryItem::Content(chunk) = item {
            let records = client.decode_history(chunk.clone()).await.unwrap();
            for record in records {
                if let RecordedContent::Output { data, .. } = record.content {
                    assert!(
                        !STANDARD
                            .decode(data)
                            .unwrap()
                            .windows(15)
                            .any(|v| v == b"pipeline-secret")
                    );
                }
            }
            let mut forged = chunk.clone();
            forged.stored_digest = "0".repeat(64);
            assert_eq!(
                client.decode_history(forged).await.unwrap_err().code,
                ErrorCode::InvalidFrame
            );
        }
    }
    assert_eq!(
        client
            .admin_history(
                &tokens.access,
                session.id,
                qgenie_remote_proto::rest::AdminHistoryQuery {
                    history: query,
                    reason: "ordinary user cannot select admin path".into(),
                    case_ref: None,
                    include_held: false
                }
            )
            .await
            .unwrap_err()
            .code,
        ErrorCode::Forbidden
    );
    assert!(
        history
            .items
            .iter()
            .any(|v| matches!(v, HistoryItem::Content(_)))
    );
    assert!(
        history
            .items
            .iter()
            .any(|v| matches!(v, HistoryItem::Gap(_)))
    );
    let token_file = private(tokens.access.as_str().as_bytes());
    let ui = tokio::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/e2e/remote_tui_smoke.py"
        ))
        .arg(env!("CARGO_BIN_EXE_qgenie-remote"))
        .arg(&origin)
        .arg(token_file.path())
        .arg(tls.ca.path())
        .output()
        .await
        .unwrap();
    assert!(
        ui.status.success(),
        "{}",
        String::from_utf8_lossy(&ui.stderr)
    );
    // Public SDK -> WSS gateway -> activated connector -> private native journal.
    // The connector owns automatic upload; input uses public native-confirmed control.
    let connector = qgenie_remote_host::Connector::new(
        api.clone(),
        identity.machine_credentials().unwrap(),
        qgenie_remote_host::filesystem::Filesystem::new(
            qgenie_remote_host::filesystem::Roots::open(std::slice::from_ref(&native_path))
                .unwrap(),
        ),
        Some(tls.ca.path().into()),
        true,
    )
    .unwrap()
    .with_native(native.clone())
    .unwrap();
    let (host_stop, mut host_cancel) = tokio::sync::watch::channel(false);
    let (host_status, mut host_updates) =
        tokio::sync::watch::channel(qgenie_remote_host::ConnectionStatus::Disconnected);
    let host_task = tokio::spawn(async move { connector.run(&mut host_cancel, host_status).await });
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        while !matches!(
            *host_updates.borrow(),
            qgenie_remote_host::ConnectionStatus::Connected {
                activated: true,
                ..
            }
        ) {
            host_updates.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    let remote_sdk =
        qgenie_remote_client::GatewayClient::new(qgenie_remote_client::GatewayConfig {
            origin: peer_server.origin.clone(),
            ca_bundle: Some(tls.ca.path().into()),
        })
        .unwrap();
    let mut live = remote_sdk.connect(&tokens.access).await.unwrap();
    let handle = live.handle();
    let starts = private_directory();
    let local = qgenie_remote_client::Client::local(&native_path);
    assert!(local.sessions().await.unwrap().is_empty());
    let roots = handle.allowed_roots(binding.machine).await.unwrap();
    assert_eq!(roots, vec![native_path.to_str().unwrap().to_owned()]);
    let launch=Launch{key:IdempotencyKey::new(),kind:LaunchKind::Pty{command:vec!["/bin/sh".into(),"-c".into(),"stty -echo; printf READY; IFS= read -r value; printf 'pipeline-secret REAL_NATIVE_DONE\\n'".into()]},cwd:roots[0].clone(),rows:128,cols:512};
    sqlx::query("UPDATE qgr_recordings.state SET recovery_pending=true")
        .execute(&database.admin)
        .await
        .unwrap();
    assert!(
        handle
            .start(
                starts.path(),
                binding.owner,
                binding.machine,
                launch.clone()
            )
            .await
            .is_err()
    );
    assert!(
        local.sessions().await.unwrap().is_empty(),
        "unready storage must not launch a child"
    );
    sqlx::query("UPDATE qgr_recordings.state SET recovery_pending=false")
        .execute(&database.admin)
        .await
        .unwrap();
    let started = handle
        .start(
            starts.path(),
            binding.owner,
            binding.machine,
            launch.clone(),
        )
        .await
        .unwrap();
    assert_eq!(
        handle
            .start(
                starts.path(),
                binding.owner,
                binding.machine,
                launch.clone()
            )
            .await
            .unwrap()
            .id,
        started.id
    );
    let mut changed = launch.clone();
    changed.cols += 1;
    let conflict = handle
        .start(starts.path(), binding.owner, binding.machine, changed)
        .await
        .unwrap_err();
    assert_eq!(conflict.code, ErrorCode::IdempotencyConflict);
    assert!(matches!(conflict.outcome, OperationOutcome::NotSent));
    assert_eq!(
        handle
            .lookup_start(binding.machine, launch.key)
            .await
            .unwrap()
            .1,
        Some(started.id)
    );
    assert_eq!(local.sessions().await.unwrap().len(), 1);
    let until = std::time::Instant::now() + std::time::Duration::from_secs(4);
    loop {
        let replay = local.replay(started.id, Counter(0)).await.unwrap();
        let ready=replay.events.iter().any(|event|matches!(&event.body,EventBody::PtyOutput(data) if STANDARD.decode(data).unwrap().windows(5).any(|v|v==b"READY")));
        if ready {
            break;
        }
        assert!(std::time::Instant::now() < until);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let attached = handle
        .attach(binding.machine, started.id, Counter(0))
        .await
        .unwrap();
    assert!(
        matches!(attached,qgenie_remote_client::RemoteAttachment::Replay(page) if page.events.iter().any(|event|matches!(&event.event,AgentEvent::PtyOutput{data} if STANDARD.decode(data).unwrap().windows(5).any(|v|v==b"READY"))))
    );
    let qgenie_remote_client::RemoteAttachment::Snapshot(snapshot) =
        handle.capture(binding.machine, started.id).await.unwrap()
    else {
        panic!("explicit native checkpoint")
    };
    assert!(
        snapshot.data.len() > 256 * 1024,
        "proof must cross multiple native/network pages"
    );
    assert!(
        vt100::Parser::from_checkpoint(&snapshot.data)
            .unwrap()
            .screen()
            .contents()
            .contains("READY")
    );
    assert!(
        matches!(handle.replay(binding.machine,started.id,snapshot.seq).await.unwrap(),qgenie_remote_client::RemoteAttachment::Replay(page) if !page.more && page.events.is_empty())
    );
    let mut first = handle
        .take_control(binding.machine, started.id, false)
        .await
        .unwrap();
    let observer = remote_sdk.connect(&tokens.access).await.unwrap();
    let other = observer.handle();
    let contested = other.take_control(binding.machine, started.id, false).await;
    assert!(
        matches!(
            &contested,
            Err(RemoteError {
                code: ErrorCode::ControlHeld,
                ..
            })
        ),
        "unexpected control denial: {:?}",
        contested.as_ref().err()
    );
    let mut second = other
        .take_control(binding.machine, started.id, true)
        .await
        .unwrap();
    assert!(second.lease().generation > first.lease().generation);
    assert!(first.input(b"stale-controller-password\n").await.is_err());
    assert!(first.input_uncertain());
    second.resize(28, 100).await.unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let event = live.next().await.expect("native push connection ended");
            if let qgenie_remote_client::LiveFrame::Text(envelope) = &event.frame
                && envelope.machine == Some(binding.machine)
                && envelope.session == Some(started.id)
                && matches!(envelope.message,Message::Head{head,..} if head>snapshot.seq)
            {
                break;
            }
        }
    })
    .await
    .expect("native resize head was not pushed without a replay request");
    let generation = second.lease().generation;
    second.renew().await.unwrap();
    assert_eq!(second.lease().generation, generation);
    second.release().await.unwrap();
    let mut control = other
        .take_control(binding.machine, started.id, false)
        .await
        .unwrap();
    assert!(control.lease().generation > generation);
    // A locked primary session projection cannot sit in the native keystroke path.
    let mut metadata_lock = database.admin.begin().await.unwrap();
    sqlx::query("LOCK TABLE qgr.sessions IN ACCESS EXCLUSIVE MODE")
        .execute(&mut *metadata_lock)
        .await
        .unwrap();
    let (applied, ()) = tokio::join!(control.input(b"no-echo-native-password\n"), async {
        let until = std::time::Instant::now() + std::time::Duration::from_secs(1);
        loop {
            let replay = local.replay(started.id, Counter(0)).await.unwrap();
            if replay.events.iter().any(|event|matches!(&event.body,EventBody::PtyOutput(data) if STANDARD.decode(data).unwrap().windows(16).any(|v|v==b"REAL_NATIVE_DONE"))) {break;}
            assert!(
                std::time::Instant::now() < until,
                "native input waited for the primary session table"
            );
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        metadata_lock.rollback().await.unwrap();
    });
    applied.unwrap();
    loop {
        if local
            .sessions()
            .await
            .unwrap()
            .iter()
            .any(|s| s.id == started.id && matches!(s.state, SessionState::Exited))
        {
            break;
        }
        assert!(std::time::Instant::now() < until);
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    let until = std::time::Instant::now() + std::time::Duration::from_secs(12);
    loop {
        let sessions = handle.sessions(binding.machine).await.unwrap();
        if sessions
            .iter()
            .any(|s| s.id == started.id && s.ended_at.is_some())
        {
            let stored: i64 =
                sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.content WHERE session=$1")
                    .bind(started.id.to_string())
                    .fetch_one(&database.admin)
                    .await
                    .unwrap();
            if stored > 0 {
                break;
            }
        }
        assert!(
            std::time::Instant::now() < until,
            "connector did not project/upload native end"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let audits: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM qgr.audit_events WHERE action='session_started' AND session=$1",
    )
    .bind(started.id.to_string())
    .fetch_one(&database.admin)
    .await
    .unwrap();
    assert_eq!(audits, 1);
    // Owner termination remains independent of the control lease and requires
    // explicit confirmation plus a committed request audit before native effect.
    let sleeping = handle
        .start(
            starts.path(),
            binding.owner,
            binding.machine,
            Launch {
                key: IdempotencyKey::new(),
                kind: LaunchKind::Pty {
                    command: vec!["/bin/sh".into(), "-c".into(),
                        r#"stty -echo; cd ..; trap 'printf "OUTSIDE_ROOT_INT\n"' INT; printf 'OUTSIDE_CWD:%s\n' "$(pwd -P)"; while :; do IFS= read -r value || continue; printf 'OUTSIDE_INPUT:%s\n' "$value"; done"#.into()],
                },
                cwd: native_path.to_str().unwrap().into(),
                rows: 24,
                cols: 80,
            },
        )
        .await
        .unwrap();
    let outside = native_path.parent().unwrap();
    assert!(
        handle
            .browse(binding.machine, outside.to_str().unwrap().into(), None, 16)
            .await
            .is_err()
    );
    async fn public_output(
        handle: &qgenie_remote_client::LiveHandle,
        machine: qgenie_remote_proto::MachineId,
        session: SessionId,
    ) -> Vec<u8> {
        let qgenie_remote_client::RemoteAttachment::Replay(page) =
            handle.replay(machine, session, Counter(0)).await.unwrap()
        else {
            panic!("bounded outside-root terminal replay")
        };
        page.events
            .into_iter()
            .filter_map(|event| match event.event {
                AgentEvent::PtyOutput { data } => Some(STANDARD.decode(data).unwrap()),
                _ => None,
            })
            .flatten()
            .collect()
    }
    handle
        .attach(binding.machine, sleeping.id, Counter(0))
        .await
        .unwrap();
    let expected = format!("OUTSIDE_CWD:{}", outside.display());
    let until = std::time::Instant::now() + std::time::Duration::from_secs(4);
    loop {
        let output = public_output(&handle, binding.machine, sleeping.id).await;
        if String::from_utf8_lossy(&output)
            .lines()
            .any(|line| line.trim_end_matches('\r') == expected)
        {
            break;
        }
        assert!(
            std::time::Instant::now() < until,
            "terminal did not enter its outside-root cwd"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let mut outside_control = handle
        .take_control(binding.machine, sleeping.id, false)
        .await
        .unwrap();
    outside_control.resize(31, 101).await.unwrap();
    outside_control.interrupt().await.unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(4);
    loop {
        let output = public_output(&handle, binding.machine, sleeping.id).await;
        if output
            .windows(b"OUTSIDE_ROOT_INT".len())
            .any(|v| v == b"OUTSIDE_ROOT_INT")
        {
            break;
        }
        assert!(
            std::time::Instant::now() < until,
            "outside-root shell did not handle interrupt"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    outside_control
        .input(b"CONTROL_AFTER_OUTSIDE_INTERRUPT\n")
        .await
        .unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(4);
    loop {
        let output = public_output(&handle, binding.machine, sleeping.id).await;
        if output
            .windows(b"OUTSIDE_ROOT_INT".len())
            .any(|v| v == b"OUTSIDE_ROOT_INT")
            && output
                .windows(b"OUTSIDE_INPUT:CONTROL_AFTER_OUTSIDE_INTERRUPT".len())
                .any(|v| v == b"OUTSIDE_INPUT:CONTROL_AFTER_OUTSIDE_INTERRUPT")
        {
            break;
        }
        assert!(
            std::time::Instant::now() < until,
            "outside-root control did not preserve interrupt/input execution"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    outside_control.release().await.unwrap();
    let host_generation = control.lease().host_generation;
    assert!(
        handle
            .terminate(binding.machine, sleeping.id, host_generation, false, false)
            .await
            .is_err()
    );
    assert!(
        local
            .sessions()
            .await
            .unwrap()
            .iter()
            .any(|s| s.id == sleeping.id && matches!(s.state, SessionState::Running))
    );
    let terminator = remote_sdk.connect(&tokens.access).await.unwrap();
    terminator
        .handle()
        .terminate_current(binding.machine, sleeping.id, true, false)
        .await
        .unwrap();
    terminator.close().await;
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let sessions = handle.sessions(binding.machine).await.unwrap();
        let stored: i64 =
            sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.content WHERE session=$1")
                .bind(sleeping.id.to_string())
                .fetch_one(&database.admin)
                .await
                .unwrap();
        if sessions
            .iter()
            .any(|s| s.id == sleeping.id && s.ended_at.is_some())
            && stored > 0
        {
            break;
        }
        assert!(
            std::time::Instant::now() < until,
            "owner termination did not project/upload its native end"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    let requests:i64=sqlx::query_scalar("SELECT count(*) FROM qgr.audit_events WHERE action='session_termination_requested' AND session=$1").bind(sleeping.id.to_string()).fetch_one(&database.admin).await.unwrap();
    assert_eq!(requests, 1);
    let ui_path = native_path.join("ui-live");
    std::fs::create_dir(&ui_path).unwrap();
    let ui_session = handle
        .start(
            starts.path(),
            binding.owner,
            binding.machine,
            Launch {
                key: IdempotencyKey::new(),
                kind: LaunchKind::Pty {
                    command: vec![
                        "/bin/sh".into(),
                        "-c".into(),
                        "stty -echo; printf 'LIVE_UI_READY\\n'; exec /bin/sh".into(),
                    ],
                },
                cwd: ui_path.to_str().unwrap().into(),
                rows: 24,
                cols: 80,
            },
        )
        .await
        .unwrap();
    let ui = tokio::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/e2e/remote_live_tui_smoke.py"
        ))
        .arg(env!("CARGO_BIN_EXE_qgenie-remote"))
        .arg(&peer_server.origin)
        .arg(token_file.path())
        .arg(tls.ca.path())
        .arg("ui-live")
        .output()
        .await
        .unwrap();
    assert!(
        ui.status.success(),
        "{}",
        String::from_utf8_lossy(&ui.stderr)
    );
    assert!(
        local
            .sessions()
            .await
            .unwrap()
            .iter()
            .any(|s| s.id == ui_session.id && matches!(s.state, SessionState::Running)),
        "closing the remote TUI must preserve the native process"
    );
    let created: Vec<_> = handle
        .sessions(binding.machine)
        .await
        .unwrap()
        .into_iter()
        .filter(|session| session.cwd == ui_path.to_str().unwrap() && session.id != ui_session.id)
        .collect();
    assert_eq!(
        created.len(),
        1,
        "keyboard create must launch exactly one new native child"
    );
    assert!(
        matches!(created[0].state, SessionState::Exited) && created[0].ended_at.is_some(),
        "confirmed UI termination must project the actual native exit"
    );
    let replay = local.replay(ui_session.id, Counter(0)).await.unwrap();
    let mut displayed = Vec::new();
    for event in replay.events {
        if let EventBody::PtyOutput(data) = event.body {
            displayed.extend(STANDARD.decode(data).unwrap());
        }
    }
    assert!(displayed.windows(13).any(|v| v == b"LIVE_UI_TYPED"));
    assert!(
        !displayed
            .windows(23)
            .any(|v| v == b"UNSENT_READ_ONLY_MARKER")
    );
    handle
        .terminate_current(binding.machine, ui_session.id, true, false)
        .await
        .unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        let sessions = handle.sessions(binding.machine).await.unwrap();
        let receipts: i64 =
            sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.receipts WHERE session=$1")
                .bind(ui_session.id.to_string())
                .fetch_one(&database.admin)
                .await
                .unwrap();
        if sessions
            .iter()
            .any(|s| s.id == ui_session.id && s.ended_at.is_some())
            && receipts > 0
        {
            break;
        }
        assert!(
            std::time::Instant::now() < until,
            "remote TUI child did not project/upload its final native capture"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    // Trusted fixture inspection waits for native reclaim before stopping the
    // uploader. Reading this private SQLite file is not a public SDK API.
    let inspection = rusqlite::Connection::open_with_flags(
        native_path.join("recordings/metadata.sqlite"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    inspection
        .busy_timeout(std::time::Duration::from_millis(25))
        .unwrap();
    loop {
        let pending:i64=inspection.query_row("SELECT count(*) FROM (SELECT epoch FROM chunks UNION ALL SELECT epoch FROM gaps) p JOIN streams s ON s.epoch=p.epoch WHERE json_extract(s.identity,'$.origin')=?1 AND json_extract(s.identity,'$.machine')=?2 AND json_extract(s.identity,'$.owner')=?3",rusqlite::params![origin,binding.machine.to_string(),binding.owner.to_string()],|r|r.get(0)).unwrap();
        if pending == 0 {
            break;
        }
        assert!(
            std::time::Instant::now() < until,
            "connector stopped before native reclaim"
        );
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    drop(inspection);
    control.release().await.unwrap();
    handle.detach(binding.machine, started.id).await.unwrap();
    assert!(matches!(
        handle.capture(binding.machine, started.id).await,
        Err(RemoteError {
            code: ErrorCode::Forbidden,
            ..
        })
    ));
    let observed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM qgr.audit_events WHERE action='session_attached' AND session=$1",
    )
    .bind(started.id.to_string())
    .fetch_one(&database.admin)
    .await
    .unwrap();
    assert_eq!(
        observed, 1,
        "continuations must not create another attachment audit"
    );
    let service_session = handle.start(starts.path(),binding.owner,binding.machine,Launch{
        key:IdempotencyKey::new(),kind:LaunchKind::Pty{command:vec!["/bin/sh".into(),"-c".into(),"stty -echo; printf 'CLI_PROCESS_READY\\n'; while IFS= read -r value; do printf '%s\\n' \"$value\"; done".into()]},cwd:native_path.to_str().unwrap().into(),rows:24,cols:80,
    }).await.unwrap();
    observer.close().await;
    live.close().await;
    host_stop.send_replace(true);
    host_task.await.unwrap().unwrap();
    drop(identity);
    let settings=private(serde_json::to_vec(&serde_json::json!({
        "gateway":origin,"os_user":"recording-pipeline","name":"recording-pipeline","hostname":"pipeline-fixture","roots":[native_path],"ca_bundle":tls.ca.path(),"identity_directory":directory.path(),"allow_private_file":true
    })).unwrap().as_slice());
    let binary_path = super::host_binary();
    let binary = binary_path.as_path();
    let units = native_path.join("generated-user-services");
    for _ in 0..2 {
        let installed = tokio::process::Command::new(binary)
            .arg("--state-dir")
            .arg(&native_path)
            .arg("host")
            .arg("--config")
            .arg(settings.path())
            .args(["service", "install", "--directory"])
            .arg(&units)
            .output()
            .await
            .unwrap();
        assert!(
            installed.status.success(),
            "{}",
            String::from_utf8_lossy(&installed.stderr)
        );
    }
    if cfg!(target_os = "linux") {
        let native_unit = units.join("qgenie-remote-sessiond.service");
        let connector_unit = units.join("qgenie-remote-connector.service");
        let original = std::fs::read(&native_unit).unwrap();
        let connector_definition = std::fs::read_to_string(&connector_unit).unwrap();
        assert!(
            !connector_definition.contains("PartOf=")
                && !connector_definition.contains("Requires=")
                && !connector_definition.contains("BindsTo=")
        );
        let revised=private(serde_json::to_vec(&serde_json::json!({"gateway":origin,"os_user":"recording-pipeline","name":"revised-service-metadata","hostname":"pipeline-fixture","roots":[native_path],"ca_bundle":tls.ca.path(),"identity_directory":directory.path(),"allow_private_file":true})).unwrap().as_slice());
        let installed = tokio::process::Command::new(binary)
            .arg("--state-dir")
            .arg(&native_path)
            .arg("host")
            .arg("--config")
            .arg(revised.path())
            .args(["service", "install", "--directory"])
            .arg(&units)
            .output()
            .await
            .unwrap();
        assert!(
            installed.status.success(),
            "{}",
            String::from_utf8_lossy(&installed.stderr)
        );
        assert!(
            String::from_utf8_lossy(&installed.stdout).contains("Native update deferred: true")
        );
        assert_eq!(
            std::fs::read(&native_unit).unwrap(),
            original,
            "active native owner definition must not be upgraded implicitly"
        );
        let verified = tokio::process::Command::new("systemd-analyze")
            .args(["verify", "--man=no"])
            .arg(&native_unit)
            .arg(&connector_unit)
            .output()
            .await
            .unwrap();
        assert!(
            verified.status.success(),
            "{}",
            String::from_utf8_lossy(&verified.stderr)
        );
    }
    let start_connector = || {
        let mut command = tokio::process::Command::new(binary);
        command
            .arg("--state-dir")
            .arg(&native_path)
            .arg("host")
            .arg("--config")
            .arg(settings.path())
            .arg("connect")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit())
            .kill_on_drop(true);
        command.spawn().unwrap()
    };
    let previous_generation = native.host_floor().await.unwrap();
    let mut connector = start_connector();
    let service_client = client.connect(&tokens.access).await.unwrap();
    let service_handle = service_client.handle();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12);
    let generation = loop {
        assert!(
            connector.try_wait().unwrap().is_none(),
            "CLI connector exited before activation"
        );
        if let Ok(generation) = service_handle
            .host_generation(binding.machine, service_session.id)
            .await
            && generation > previous_generation
        {
            break generation;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "CLI connector did not activate"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    connector.kill().await.unwrap();
    connector.wait().await.unwrap();
    assert!(
        daemon.0.try_wait().unwrap().is_none(),
        "killing the CLI connector must not kill sessiond"
    );
    assert!(
        local
            .sessions()
            .await
            .unwrap()
            .iter()
            .any(|s| s.id == service_session.id && matches!(s.state, SessionState::Running))
    );
    let mut local_control = local.take_control(service_session.id, false).await.unwrap();
    local_control
        .input(b"CLI_CAPTURE_WHILE_PAUSED\r")
        .await
        .unwrap();
    local.release(service_session.id).await.unwrap();
    let mut connector = start_connector();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12);
    let second_generation = loop {
        assert!(connector.try_wait().unwrap().is_none());
        if let Ok(current) = service_handle
            .host_generation(binding.machine, service_session.id)
            .await
            && current > generation
        {
            break current;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "restarted CLI connector did not advance native generation"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    let paused = tokio::process::Command::new(binary)
        .arg("--state-dir")
        .arg(&native_path)
        .arg("host")
        .arg("--config")
        .arg(settings.path())
        .arg("down")
        .arg("--keep-sessions")
        .output()
        .await
        .unwrap();
    assert!(
        paused.status.success(),
        "{}",
        String::from_utf8_lossy(&paused.stderr)
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(5), connector.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    assert!(daemon.0.try_wait().unwrap().is_none());
    assert!(
        local
            .sessions()
            .await
            .unwrap()
            .iter()
            .any(|s| s.id == service_session.id && matches!(s.state, SessionState::Running))
    );
    // Process mode must also preserve the existing native owner and be idempotent.
    // A panic still stops the exact scoped background connector through its owned lock.
    struct PauseBackground(std::path::PathBuf, std::path::PathBuf, std::path::PathBuf);
    impl Drop for PauseBackground {
        fn drop(&mut self) {
            let _ = std::process::Command::new(&self.0)
                .arg("--state-dir")
                .arg(&self.1)
                .arg("host")
                .arg("--config")
                .arg(&self.2)
                .args(["down", "--keep-sessions"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        }
    }
    let background = PauseBackground(binary.into(), native_path.clone(), settings.path().into());
    for _ in 0..2 {
        let resumed = tokio::process::Command::new(binary)
            .arg("--state-dir")
            .arg(&native_path)
            .arg("host")
            .arg("--config")
            .arg(settings.path())
            .args(["up", "--no-service"])
            .output()
            .await
            .unwrap();
        assert!(
            resumed.status.success(),
            "{}",
            String::from_utf8_lossy(&resumed.stderr)
        );
    }
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12);
    let background_generation = loop {
        if let Ok(current) = service_handle
            .host_generation(binding.machine, service_session.id)
            .await
            && current > second_generation
        {
            break current;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "background connector did not reactivate"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    };
    assert!(daemon.0.try_wait().unwrap().is_none());
    assert_eq!(
        local
            .sessions()
            .await
            .unwrap()
            .iter()
            .filter(|s| s.id == service_session.id)
            .count(),
        1
    );
    drop(background);
    let mut connector = start_connector();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(12);
    while !service_handle
        .host_generation(binding.machine, service_session.id)
        .await
        .is_ok_and(|current| current > background_generation)
    {
        assert!(std::time::Instant::now() < deadline);
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    service_handle
        .terminate_current(binding.machine, service_session.id, true, false)
        .await
        .unwrap();
    let mut saw_paused = false;
    loop {
        if let Ok(streams) = client
            .history_streams(
                &tokens.access,
                service_session.id,
                qgenie_remote_proto::rest::HistoryStreamsQuery {
                    after: None,
                    limit: 4,
                },
            )
            .await
        {
            for stream in streams.streams {
                let page = client
                    .history(
                        &tokens.access,
                        service_session.id,
                        HistoryQuery {
                            stream_epoch: stream.stream_epoch,
                            after: None,
                            limit: 4,
                        },
                    )
                    .await
                    .unwrap();
                for item in page.items {
                    if let HistoryItem::Content(chunk) = item {
                        for record in client.decode_history(chunk).await.unwrap() {
                            if let RecordedContent::Output { data, .. } = record.content {
                                let data = STANDARD.decode(data).unwrap();
                                saw_paused |=
                                    data.windows(24).any(|v| v == b"CLI_CAPTURE_WHILE_PAUSED");
                            }
                        }
                    }
                }
            }
        }
        let final_state = local
            .sessions()
            .await
            .unwrap()
            .iter()
            .any(|s| s.id == service_session.id && s.ended_at_ms.is_some());
        let spool = rusqlite::Connection::open_with_flags(
            native_path.join("recordings/metadata.sqlite"),
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .unwrap();
        let pending:i64=spool.query_row("SELECT count(*) FROM (SELECT epoch FROM chunks UNION ALL SELECT epoch FROM gaps) p JOIN streams s ON s.epoch=p.epoch WHERE json_extract(s.identity,'$.session')=?1",[service_session.id.to_string()],|r|r.get(0)).unwrap();
        if saw_paused && final_state && pending == 0 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "CLI resume did not upload native capture retained across connector kill/pause"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    if let Some(report) = std::env::var_os("QGR_HOST_RESOURCE_REPORT") {
        let active = local.session_page(None, 64, true).await.unwrap();
        assert!(
            active.0.is_empty(),
            "footprint requires an idle native owner"
        );
        let measured = tokio::process::Command::new("python3")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/load/host_footprint.py"
            ))
            .arg("--native")
            .arg(daemon.0.id().to_string())
            .arg("--connector")
            .arg(connector.id().unwrap().to_string())
            .arg("--report")
            .arg(report)
            .kill_on_drop(true)
            .output()
            .await
            .unwrap();
        assert!(
            measured.status.success(),
            "{}",
            String::from_utf8_lossy(&measured.stderr)
        );
        assert!(daemon.0.try_wait().unwrap().is_none());
        assert!(connector.try_wait().unwrap().is_none());
        assert!(
            local
                .session_page(None, 64, true)
                .await
                .unwrap()
                .0
                .is_empty()
        );
        service_handle.allowed_roots(binding.machine).await.unwrap();
    }
    let retained = service_handle
        .start(
            starts.path(),
            binding.owner,
            binding.machine,
            Launch {
                key: IdempotencyKey::new(),
                kind: LaunchKind::Pty {
                    command: vec![
                        "/bin/sh".into(),
                        "-c".into(),
                        "stty -echo; exec /bin/sh".into(),
                    ],
                },
                cwd: native_path.to_str().unwrap().into(),
                rows: 24,
                cols: 80,
            },
        )
        .await
        .unwrap();
    // SIGTERM stops only the connector process; sessiond is still independently alive.
    unsafe {
        libc::kill(connector.id().unwrap() as i32, libc::SIGTERM);
    }
    assert!(
        tokio::time::timeout(std::time::Duration::from_secs(5), connector.wait())
            .await
            .unwrap()
            .unwrap()
            .success()
    );
    assert!(daemon.0.try_wait().unwrap().is_none());
    service_client.close().await;
    let streams = client
        .history_streams(
            &tokens.access,
            started.id,
            qgenie_remote_proto::rest::HistoryStreamsQuery {
                after: None,
                limit: 4,
            },
        )
        .await
        .unwrap();
    assert_eq!(streams.streams.len(), 1);
    let page = client
        .history(
            &tokens.access,
            started.id,
            HistoryQuery {
                stream_epoch: streams.streams[0].stream_epoch,
                after: None,
                limit: 4,
            },
        )
        .await
        .unwrap();
    let mut output = Vec::new();
    for item in page.items {
        if let HistoryItem::Content(chunk) = item {
            for record in client.decode_history(chunk).await.unwrap() {
                if let RecordedContent::Output { data, .. } = record.content {
                    output.extend(STANDARD.decode(data).unwrap());
                }
            }
        }
    }
    assert!(output.windows(16).any(|v| v == b"REAL_NATIVE_DONE"));
    for secret in [
        b"pipeline-secret".as_slice(),
        b"no-echo-native-password",
        b"stale-controller-password",
    ] {
        assert!(!output.windows(secret.len()).any(|v| v == secret));
    }
    let machine_ui = tokio::process::Command::new("python3")
        .arg(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../tests/e2e/remote_machine_tui_smoke.py"
        ))
        .arg(binary)
        .arg(&peer_server.origin)
        .arg(token_file.path())
        .arg(tls.ca.path())
        .arg(binding.machine.to_string())
        .output()
        .await
        .unwrap();
    assert!(
        machine_ui.status.success(),
        "{}",
        String::from_utf8_lossy(&machine_ui.stderr)
    );
    assert!(daemon.0.try_wait().unwrap().is_none());
    assert!(
        local
            .sessions()
            .await
            .unwrap()
            .iter()
            .any(|session| session.id == retained.id
                && matches!(session.state, SessionState::Running))
    );
    let mut retained_control = local.take_control(retained.id, false).await.unwrap();
    retained_control
        .input(b"printf 'LOCAL_AFTER_MACHINE_REVOKE\\n'\r")
        .await
        .unwrap();
    local.release(retained.id).await.unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let replay = local.replay(retained.id, Counter(0)).await.unwrap();
        if replay.events.iter().any(|event| matches!(&event.body, EventBody::PtyOutput(data) if STANDARD.decode(data).unwrap().windows(26).any(|part| part == b"LOCAL_AFTER_MACHINE_REVOKE"))) { break; }
        assert!(
            std::time::Instant::now() < until,
            "machine revocation prevented local native input"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    daemon.stop(&native_path).await;
    let spool =
        RecordingSpool::open(SpoolConfig::new(native_path.join("recordings")), now()).unwrap();
    assert!(spool.status().unwrap().chunks >= 2);
    assert!(
        spool.status_scoped(native.scope()).unwrap().chunks >= 1,
        "revoked native capture must remain locally durable"
    );
    assert_eq!(spool.status_scoped(&foreign_scope).unwrap().chunks, 1);
    drop(spool);
    #[cfg(all(feature = "managed-codex", target_os = "linux"))]
    if std::env::var_os("QGR_CODEX_BINARY").is_some() {
        super::managed::prove(
            api.clone(),
            remote_sdk.clone(),
            &tokens.access,
            &origin,
            tls.ca.path(),
            &database.admin,
        )
        .await;
    }
    // Inspect the actual private scrape endpoint after the composed native
    // workflows. Counters must describe real traffic, not synthetic increments.
    use tower::ServiceExt;
    let mut text = String::new();
    for node in [&gateway, &peer_gateway] {
        let response = node
            .health_router()
            .oneshot(
                Request::builder()
                    .uri("/metrics")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert!(response.status().is_success());
        let body = axum::body::to_bytes(response.into_body(), 64 * 1024)
            .await
            .unwrap();
        text.push_str(std::str::from_utf8(&body).unwrap());
    }
    let names = [
        "qgr_transport_connections_total",
        "qgr_transport_bytes_in_total",
        "qgr_transport_bytes_out_total",
        "qgr_replay_requests_total",
        "qgr_approval_choice_attempts_total",
        "qgr_native_approval_delivery_attempt_seconds_count",
        "qgr_native_approval_delivery_attempt_seconds_sum",
    ];
    let samples = names
        .into_iter()
        .map(|name| {
            let value = text
                .lines()
                .filter_map(|line| line.strip_prefix(&format!("{name} ")))
                .map(|value| value.parse::<f64>().unwrap())
                .sum::<f64>();
            assert!(value.is_finite() && value >= 0.0);
            (name, value)
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    for name in &names[..4] {
        assert!(samples[name] > 0.0, "actual traffic missing from {name}");
    }
    for priority in ["control", "event", "bulk"] {
        assert!(text.contains(&format!(
            "qgr_transport_queue_full_total{{priority=\"{priority}\"}} "
        )));
    }
    if cfg!(all(feature = "managed-codex", target_os = "linux"))
        && std::env::var_os("QGR_CODEX_BINARY").is_some()
    {
        assert!(samples["qgr_approval_choice_attempts_total"] >= 2.0);
        assert!(samples["qgr_native_approval_delivery_attempt_seconds_count"] >= 3.0);
        assert!(samples["qgr_native_approval_delivery_attempt_seconds_sum"] > 0.0);
    }
    std::fs::write(
        std::env::var_os("QGR_METRICS_PROOF_FILE").unwrap(),
        serde_json::to_vec(&samples).unwrap(),
    )
    .unwrap();
    peer_gateway.shutdown().await;
    gateway.shutdown().await;
    drop(peer_server);
    drop(server);
    database.destroy().await;
    std::fs::write(
        std::env::var_os("QGR_SYNC_PIPELINE_PROOF_FILE").unwrap(),
        b"{\"complete\":true}",
    )
    .unwrap();
}
