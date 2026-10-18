use super::*;
use crate::{Authenticator, Gateway, IdentityService, JwtVerifier};

#[test]
fn suspend_elapsed_freshness_is_anchored_to_the_read_start() {
    let status = Status {
        proof: Some(DirectoryObservation {
            enabled: true,
            started_elapsed: Duration::from_secs(100),
        }),
        ..Status::default()
    };
    assert_eq!(
        status.evidence(Duration::from_secs(105)).unwrap(),
        Duration::from_secs(835)
    );
    // Awake time, token expiry and response receipt do not enter the calculation.
    assert!(status.evidence(Duration::from_secs(939)).is_ok());
    assert!(matches!(
        status.evidence(Duration::from_secs(940)),
        Err(Error::Unavailable)
    ));
    assert!(matches!(
        status.evidence(Duration::from_secs(99)),
        Err(Error::Unavailable)
    ));
    let disabled = Status {
        proof: Some(DirectoryObservation {
            enabled: false,
            started_elapsed: Duration::from_secs(100),
        }),
        ..Status::default()
    };
    assert!(matches!(
        disabled.evidence(Duration::from_secs(101)),
        Err(Error::Forbidden)
    ));
}

/// Real HTTPS provider and application-role PostgreSQL, called by the disposable
/// database contract. Persisted UTC state is never restored as local authority.
pub(crate) async fn database_contract(store: Store, admin: &sqlx::PgPool) {
    let fixture = crate::identity::directory::tests::Fixture::new().await;
    let user = UserId::new();
    const OBJECT: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
    sqlx::query("INSERT INTO qgr.users(id,issuer,tenant,subject,directory_object) VALUES($1,'https://directory-fixture.invalid',$2,$1,$3::uuid)")
        .bind(user.to_string()).bind(&fixture.config.tenant).bind(OBJECT).execute(admin).await.unwrap();
    let service = DirectoryFreshness::start(
        store.clone(),
        GraphDirectory::connect(fixture.config.clone()).unwrap(),
    )
    .await
    .unwrap();
    let remaining = service.assert(user).await.unwrap();
    assert!(remaining <= FRESH && remaining > FRESH - Duration::from_secs(5));
    let projected: bool = sqlx::query_scalar(
        "SELECT directory_enabled AND directory_verified_at IS NOT NULL FROM qgr.users WHERE id=$1",
    )
    .bind(user.to_string())
    .fetch_one(admin)
    .await
    .unwrap();
    assert!(projected);
    service.shutdown().await;
    fixture.respond(503, "{}");
    let service = DirectoryFreshness::start(
        store.clone(),
        GraphDirectory::connect(fixture.config.clone()).unwrap(),
    )
    .await
    .unwrap();
    assert!(matches!(
        service.assert(user).await,
        Err(Error::Unavailable)
    ));
    let entry = service.inner.entry(user).await.unwrap();
    assert!(entry.status.lock().unwrap().proof.is_none());
    entry.status.lock().unwrap().attempt = None;
    let renewed_remaining = service.assert(user).await.unwrap();
    let proof = entry.status.lock().unwrap().proof.unwrap();
    fixture.respond(429, "{}");
    entry.status.lock().unwrap().proof = None;
    entry.status.lock().unwrap().attempt = None;
    // Restore the good proof after forcing a due attempt: failed reads cannot
    // update its original start time or extend its 840-second lifetime.
    service.inner.refresh(entry.clone()).await.unwrap_err();
    assert!(entry.status.lock().unwrap().proof.is_none());
    entry.status.lock().unwrap().proof = Some(proof);
    assert!(service.assert(user).await.unwrap() <= renewed_remaining);
    sqlx::query("UPDATE qgr.users SET disabled=true WHERE id=$1")
        .bind(user.to_string())
        .execute(admin)
        .await
        .unwrap();
    entry.status.lock().unwrap().proof = None;
    entry.status.lock().unwrap().attempt = None;
    service.inner.refresh(entry.clone()).await.unwrap();
    let manual: bool = sqlx::query_scalar("SELECT disabled FROM qgr.users WHERE id=$1")
        .bind(user.to_string())
        .fetch_one(admin)
        .await
        .unwrap();
    assert!(manual);
    entry.status.lock().unwrap().proof = None;
    entry.status.lock().unwrap().attempt = None;
    fixture.respond(
        200,
        format!(r#"{{"id":"{OBJECT}","accountEnabled":false}}"#),
    );
    assert!(matches!(service.assert(user).await, Err(Error::Forbidden)));
    let projected: bool = sqlx::query_scalar("SELECT directory_enabled FROM qgr.users WHERE id=$1")
        .bind(user.to_string())
        .fetch_one(admin)
        .await
        .unwrap();
    assert!(!projected);
    // Cached disabled evidence remains denied until a later authoritative read.
    assert!(matches!(service.assert(user).await, Err(Error::Forbidden)));
    assert!(!entry.status.lock().unwrap().busy);
    service.shutdown().await;
}

pub(crate) async fn http_contract(store: Store, admin: &sqlx::PgPool, mut gateway: Gateway) {
    use crate::identity::{
        jwt::tests as jwt,
        tests::{proof, request},
    };
    use axum::{
        body::Body,
        http::{Request, StatusCode},
    };
    use base64::{Engine, engine::general_purpose::STANDARD};
    use tower::ServiceExt;
    let fixture = crate::identity::directory::tests::Fixture::new().await;
    const OBJECT: &str = "cccccccc-cccc-4ccc-8ccc-cccccccccccc";
    let mut config = jwt::config();
    config.tenant = fixture.config.tenant.clone();
    let verifier = Arc::new(JwtVerifier::with_keys(config, jwt::keys(), store.clone()).unwrap());
    let mut claims = jwt::claims("directory-http-owner");
    claims["tid"] = serde_json::json!(fixture.config.tenant);
    claims["oid"] = serde_json::json!(OBJECT);
    let token = jwt::sign(&claims);
    let owner = verifier.authenticate(&token).await.unwrap().user;
    let key_file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(key_file.path(), STANDARD.encode([37; 32])).unwrap();
    gateway.auth = verifier;
    gateway.identity = Some(Arc::new(
        IdentityService::from_file(
            store,
            "https://gateway.example.invalid".into(),
            key_file.path(),
        )
        .await
        .unwrap(),
    ));
    gateway = gateway
        .enable_directory(fixture.config.clone())
        .await
        .unwrap();
    let identity = gateway.identity.as_ref().unwrap();
    let directory = identity.directory.as_ref().unwrap();
    async fn get(gateway: &Gateway, token: &str) -> StatusCode {
        gateway
            .router()
            .oneshot(
                Request::builder()
                    .uri("/v1/machines")
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }
    assert_eq!(get(&gateway, &token).await, StatusCode::OK);
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
    let machine_token = identity
        .machine_token(proof(&key, &challenge))
        .await
        .unwrap();
    let entry = directory.inner.entry(owner).await.unwrap();
    let invalidate = || {
        let mut status = entry.status.lock().unwrap();
        status.proof = None;
        status.attempt = None;
    };
    invalidate();
    fixture.respond(
        200,
        format!(r#"{{"id":"{OBJECT}","accountEnabled":false}}"#),
    );
    assert_eq!(get(&gateway, &token).await, StatusCode::FORBIDDEN);
    assert!(matches!(
        identity.authenticate_machine(&machine_token.token).await,
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        identity
            .machine_challenge(enrolled.machine, enrollment.key)
            .await,
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        identity.enrollment_challenge(owner, enrollment).await,
        Err(Error::Forbidden)
    ));
    // A successful directory read cannot clear a manual user revocation.
    sqlx::query("UPDATE qgr.users SET disabled=true WHERE id=$1")
        .bind(owner.to_string())
        .execute(admin)
        .await
        .unwrap();
    invalidate();
    directory.assert(owner).await.unwrap();
    assert_eq!(get(&gateway, &token).await, StatusCode::FORBIDDEN);
    sqlx::query("UPDATE qgr.users SET disabled=false WHERE id=$1")
        .bind(owner.to_string())
        .execute(admin)
        .await
        .unwrap();
    assert_eq!(get(&gateway, &token).await, StatusCode::OK);
    // An initially valid token must expire while HTTPS verification is waiting.
    let mut expiring = claims;
    let now = crate::identity::jwt::unix_seconds().unwrap();
    expiring["iat"] = serde_json::json!(now);
    expiring["nbf"] = serde_json::json!(now - 1);
    expiring["exp"] = serde_json::json!(now + 1);
    invalidate();
    fixture.respond_after(
        Duration::from_secs(2),
        200,
        format!(r#"{{"id":"{OBJECT}","accountEnabled":true}}"#),
    );
    assert_eq!(
        get(&gateway, &jwt::sign(&expiring)).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(get(&gateway, &token).await, StatusCode::OK);
    directory.shutdown().await;
}
