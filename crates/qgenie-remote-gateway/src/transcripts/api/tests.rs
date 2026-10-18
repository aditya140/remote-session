use crate::{
    Authenticator, Gateway, IdentityService, JwtVerifier, SecretDsn, Store,
    identity::jwt::tests as jwt,
};
use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use qgenie_remote_proto::{Counter, HoldId, rest::HoldChange};
use std::sync::Arc;
use tower::ServiceExt;

async fn post(
    gateway: &Gateway,
    token: Option<&str>,
    path: &str,
    body: Vec<u8>,
) -> (StatusCode, serde_json::Value) {
    let mut request = Request::builder().method("POST").uri(path);
    if let Some(token) = token {
        request = request.header("authorization", format!("Bearer {token}"));
    }
    let response = gateway
        .router()
        .oneshot(request.body(Body::from(body)).unwrap())
        .await
        .unwrap();
    let status = response.status();
    if status.is_success() {
        assert_eq!(response.headers()["cache-control"], "private, no-store");
    }
    let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}

/// Successful public paths run only on the owned real synchronous pair. No test-only
/// switch can qualify these HTTP handlers against the single-server fixture.
pub(crate) async fn production_recording_routes(
    gateway: &Gateway,
    machine: &str,
    owner: &str,
    admin: &str,
    batch: qgenie_remote_proto::wire::RecordingBatch,
    receipt: &qgenie_remote_proto::wire::CommitReceipt,
) -> qgenie_remote_proto::rest::RecordingAuthority {
    use qgenie_remote_proto::{rest::*, wire::*, *};
    let body = serde_json::to_vec(&batch).unwrap();
    assert!(body.len() > 64 * 1024 && body.len() < 128 * 1024);
    for token in [None, Some(owner), Some(admin)] {
        assert_eq!(
            post(gateway, token, "/v1/recordings/chunks", body.clone())
                .await
                .0,
            StatusCode::UNAUTHORIZED
        );
    }
    let (status, duplicate) = post(
        gateway,
        Some(machine),
        "/v1/recordings/chunks",
        body.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        duplicate,
        serde_json::to_value(receipt).unwrap(),
        "lost HTTP ACK must reconcile the same durable receipt"
    );
    let mut forged = batch.clone();
    forged.machine = MachineId::new();
    assert_eq!(
        post(
            gateway,
            Some(machine),
            "/v1/recordings/chunks",
            serde_json::to_vec(&forged).unwrap()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(
            gateway,
            Some(machine),
            "/v1/recordings/chunks",
            br#"{"id":"one","id":"two"}"#.to_vec()
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        post(
            gateway,
            Some(machine),
            "/v1/recordings/chunks",
            vec![b' '; 128 * 1024 + 1]
        )
        .await
        .0,
        StatusCode::PAYLOAD_TOO_LARGE
    );
    let query = HistoryQuery {
        stream_epoch: batch.stream_epoch,
        after: None,
        limit: 4,
    };
    let path = format!("/v1/sessions/{}/history", batch.session);
    assert_eq!(
        post(
            gateway,
            Some(machine),
            &path,
            serde_json::to_vec(&query).unwrap()
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    for token in [owner, admin] {
        let (status, page) = post(
            gateway,
            Some(token),
            &path,
            serde_json::to_vec(&query).unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let page: HistoryPage = serde_json::from_value(page).unwrap();
        assert_eq!(page.items.len(), 1);
        let HistoryItem::Content(chunk) = &page.items[0] else {
            panic!("missing content");
        };
        let bytes = STANDARD.decode(&chunk.data).unwrap();
        let raw = zstd::bulk::decompress(&bytes, 1024 * 1024).unwrap();
        for line in raw.split(|b| *b == b'\n').filter(|v| !v.is_empty()) {
            let record: RecordingRecord = qgenie_remote_proto::decode_json(line).unwrap();
            let RecordedContent::Output { data, .. } = record.content else {
                panic!("wrong content");
            };
            assert!(
                !STANDARD
                    .decode(data)
                    .unwrap()
                    .windows(14)
                    .any(|v| v == b"fixture-secret")
            );
        }
    }
    let admin_path = format!("/v1/admin/sessions/{}/history", batch.session);
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(owner.split('.').nth(1).unwrap())
        .unwrap();
    let mut claims: serde_json::Value = serde_json::from_slice(&payload).unwrap();
    claims["oid"] = serde_json::json!("dddddddd-dddd-4ddd-8ddd-dddddddddddd");
    claims["sub"] = serde_json::json!("different-history-owner");
    let foreign = jwt::sign(&claims);
    assert_eq!(
        post(
            gateway,
            Some(&foreign),
            &path,
            serde_json::to_vec(&query).unwrap()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let admin_query = AdminHistoryQuery {
        history: query,
        reason: "public route audit".into(),
        case_ref: None,
        include_held: false,
    };
    assert_eq!(
        post(
            gateway,
            Some(owner),
            &admin_path,
            serde_json::to_vec(&admin_query).unwrap()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(
            gateway,
            Some(admin),
            &admin_path,
            serde_json::to_vec(&admin_query).unwrap()
        )
        .await
        .0,
        StatusCode::OK
    );
    let mut invalid = admin_query;
    invalid.reason.clear();
    assert_eq!(
        post(
            gateway,
            Some(admin),
            &admin_path,
            serde_json::to_vec(&invalid).unwrap()
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let gap = LossRange {
        id: GapId::new(),
        session: batch.session,
        stream: RecordingStream::Pty,
        from: Counter(1),
        to: Counter(2),
        stream_epoch: RecordingEpochId::new(),
        captured_from: batch.captured_from.clone(),
        captured_to: batch.captured_to.clone(),
        reason: LossReason::CaptureBackpressure,
    };
    let gap_body = serde_json::to_vec(&gap).unwrap();
    let (status, first) = post(
        gateway,
        Some(machine),
        "/v1/recordings/gaps",
        gap_body.clone(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        post(gateway, Some(machine), "/v1/recordings/gaps", gap_body)
            .await
            .1,
        first
    );
    let challenge = ChallengeId::new();
    let streams_path = format!("/v1/sessions/{}/history/streams", batch.session);
    let query = HistoryStreamsQuery {
        after: None,
        limit: 1,
    };
    let (status, page) = post(
        gateway,
        Some(owner),
        &streams_path,
        serde_json::to_vec(&query).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let first: HistoryStreamsPage = serde_json::from_value(page).unwrap();
    assert_eq!(first.streams.len(), 1);
    let (status, page) = post(
        gateway,
        Some(owner),
        &streams_path,
        serde_json::to_vec(&HistoryStreamsQuery {
            after: first.next.clone(),
            limit: 1,
        })
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let second: HistoryStreamsPage = serde_json::from_value(page).unwrap();
    assert_eq!(second.streams.len(), 1);
    assert!(second.next.is_none());
    let epochs = std::collections::BTreeSet::from([
        first.streams[0].stream_epoch,
        second.streams[0].stream_epoch,
    ]);
    assert_eq!(
        epochs,
        std::collections::BTreeSet::from([batch.stream_epoch, gap.stream_epoch])
    );
    assert_eq!(
        post(
            gateway,
            Some(&foreign),
            &streams_path,
            serde_json::to_vec(&query).unwrap()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    let admin_streams = format!("/v1/admin/sessions/{}/history/streams", batch.session);
    let admin_index = AdminHistoryStreamsQuery {
        history: query,
        reason: "public stream discovery audit".into(),
        case_ref: None,
        include_held: false,
    };
    assert_eq!(
        post(
            gateway,
            Some(owner),
            &admin_streams,
            serde_json::to_vec(&admin_index).unwrap()
        )
        .await
        .0,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(
            gateway,
            Some(admin),
            &admin_streams,
            serde_json::to_vec(&admin_index).unwrap()
        )
        .await
        .0,
        StatusCode::OK
    );
    let path = format!("/v1/recordings/sessions/{}/authority", batch.session);
    let (status, binding) = post(
        gateway,
        Some(machine),
        &path,
        serde_json::to_vec(&RecordingAuthorityRequest { challenge }).unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let binding: RecordingAuthority = serde_json::from_value(binding).unwrap();
    assert_eq!(binding.challenge, challenge);
    assert_eq!(binding.machine, batch.machine);
    assert_eq!(binding.backend_epoch, receipt.recovery_epoch);
    binding
}

pub(crate) async fn database_contract(
    store: Store,
    admin: &sqlx::PgPool,
    migration: &SecretDsn,
    mut gateway: Gateway,
) {
    let fixture = crate::identity::DirectoryFixture::new().await;
    let mut config = jwt::config();
    config.tenant = fixture.config.tenant.clone();
    let verifier = Arc::new(JwtVerifier::with_keys(config, jwt::keys(), store.clone()).unwrap());
    let mut claims = jwt::claims("hold-http-user");
    claims["tid"] = serde_json::json!(fixture.config.tenant);
    claims["oid"] = serde_json::json!("cccccccc-cccc-4ccc-8ccc-cccccccccccc");
    let user_token = jwt::sign(&claims);
    let user = verifier.authenticate(&user_token).await.unwrap().user;
    claims["roles"] = serde_json::json!([jwt::config().admin_role]);
    let admin_token = jwt::sign(&claims);
    let key = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(key.path(), STANDARD.encode([76; 32])).unwrap();
    gateway.auth = verifier;
    gateway.identity = Some(Arc::new(
        IdentityService::from_file(store, "https://gateway.example.invalid".into(), key.path())
            .await
            .unwrap(),
    ));
    gateway = gateway
        .enable_directory(fixture.config.clone())
        .await
        .unwrap();
    let mut url = url::Url::parse(&migration.0).unwrap();
    url.set_username("qgr_test_record_lifecycle").unwrap();
    url.set_password(Some("test-only")).unwrap();
    gateway = gateway
        .enable_recording_lifecycle(&crate::tests::recording_secret(url.as_str()))
        .await
        .unwrap();
    let change = HoldChange {
        id: HoldId::new(),
        user,
        revision: Counter(1),
        active: true,
        captured_from: None,
        captured_to: None,
        case_ref: None,
        reason: "signed HTTP role boundary".into(),
    };
    let body = serde_json::to_string(&change).unwrap();
    async fn post(gateway: &Gateway, token: Option<&str>, body: &str) -> StatusCode {
        let mut request = Request::builder().method("POST").uri("/v1/admin/holds");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        gateway
            .router()
            .oneshot(request.body(Body::from(body.to_owned())).unwrap())
            .await
            .unwrap()
            .status()
    }
    assert_eq!(post(&gateway, None, &body).await, StatusCode::UNAUTHORIZED);
    assert_eq!(
        post(&gateway, Some(&user_token), &body).await,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        post(&gateway, Some(&admin_token), r#"{"id":"a","id":"b"}"#).await,
        StatusCode::BAD_REQUEST
    );
    // No HTTP flag can enable the private synthetic single-server path.
    super::super::ingest::tests::state(admin, "http-must-not-qualify-synthetic", false).await;
    assert_eq!(
        post(&gateway, Some(&admin_token), &body).await,
        StatusCode::SERVICE_UNAVAILABLE
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.holds WHERE id=$1")
        .bind(change.id.to_string())
        .fetch_one(admin)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let binding = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(
        binding.path(),
        crate::tests::recording_secret(url.as_str()).0.as_bytes(),
    )
    .unwrap();
    let job = tokio::process::Command::new(std::env::var_os("QGR_GATEWAY_BINARY").unwrap())
        .args(["jobs", "--database-url-file"])
        .arg(binding.path())
        .args(["--max-steps", "1"])
        .output()
        .await
        .unwrap();
    assert!(
        !job.status.success(),
        "production job must also reject synthetic qualification"
    );
    let database = gateway.recording_lifecycle.as_ref().unwrap();
    database.close().await;
    gateway
        .identity
        .as_ref()
        .unwrap()
        .directory
        .as_ref()
        .unwrap()
        .shutdown()
        .await;
    super::super::ingest::tests::state(admin, "http-recovery-quarantine", true).await;
}
