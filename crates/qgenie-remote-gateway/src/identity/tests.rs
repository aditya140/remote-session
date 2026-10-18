use super::*;
use crate::{Authenticator, Gateway};
use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signer, SigningKey};
use qgenie_remote_proto::{
    InstallationId, KeyId,
    rest::{EnrollmentRequest, MachinePatch, Proof, Recovery},
};
use std::{collections::BTreeMap, io::Write, sync::Arc};
pub(crate) fn request(key: &SigningKey) -> EnrollmentRequest {
    EnrollmentRequest {
        installation: InstallationId::new(),
        key: KeyId::new(),
        public_key: STANDARD.encode(key.verifying_key().as_bytes()),
        os_user: "fixture-user".into(),
        hostname: "same-hostname".into(),
        os: "linux".into(),
        arch: "x86_64".into(),
        cli_version: "0.1.0".into(),
        name: "lab host".into(),
        labels: BTreeMap::new(),
        recovery: None,
    }
}
pub(crate) fn proof(key: &SigningKey, challenge: &qgenie_remote_proto::rest::Challenge) -> Proof {
    Proof {
        challenge: challenge.challenge,
        signature: STANDARD.encode(key.sign(&challenge.signing_bytes().unwrap()).to_bytes()),
    }
}
/// Runs inside the foundation test's generated disposable PostgreSQL database.
pub(crate) async fn enrollment_contract(store: Store, admin: &sqlx::PgPool, mut gateway: Gateway) {
    let verifier = Arc::new(
        jwt::JwtVerifier::with_keys(jwt::tests::config(), jwt::tests::keys(), store.clone())
            .unwrap(),
    );
    let owner_token = jwt::tests::sign(&jwt::tests::claims("enrollment-owner"));
    let foreign_token = jwt::tests::sign(&jwt::tests::claims("enrollment-foreign"));
    let owner = verifier.authenticate(&owner_token).await.unwrap().user;
    let foreign = verifier.authenticate(&foreign_token).await.unwrap().user;
    let mut key_file = tempfile::NamedTempFile::new().unwrap();
    write!(key_file, "{}", STANDARD.encode([42u8; 32])).unwrap();
    let service = Arc::new(
        IdentityService::from_file(
            store.clone(),
            "https://gateway.example.invalid".into(),
            key_file.path(),
        )
        .await
        .unwrap(),
    );
    let key = SigningKey::from_bytes(&rand::random());
    let initial = request(&key);
    let first = service
        .enrollment_challenge(owner, initial.clone())
        .await
        .unwrap();
    assert!(matches!(
        service.enroll(foreign, proof(&key, &first)).await,
        Err(Error::Forbidden)
    ));
    let wrong = SigningKey::from_bytes(&rand::random());
    assert!(matches!(
        service.enroll(owner, proof(&wrong, &first)).await,
        Err(Error::Unauthenticated)
    ));
    let enrolled = service.enroll(owner, proof(&key, &first)).await.unwrap();
    assert!(matches!(
        service.enroll(owner, proof(&key, &first)).await,
        Err(Error::Unauthenticated)
    ));
    let repeated = service
        .enrollment_challenge(owner, initial.clone())
        .await
        .unwrap();
    assert_eq!(
        service
            .enroll(owner, proof(&key, &repeated))
            .await
            .unwrap()
            .machine,
        enrolled.machine
    );
    service
        .patch(
            owner,
            enrolled.machine,
            MachinePatch {
                name: Some("renamed display".into()),
                labels: Some([("purpose".into(), "test".into())].into()),
            },
        )
        .await
        .unwrap();
    let mut renamed = initial.clone();
    renamed.hostname = "changed-hostname".into();
    let challenge = service.enrollment_challenge(owner, renamed).await.unwrap();
    service
        .enroll(owner, proof(&key, &challenge))
        .await
        .unwrap();
    let listed = service.machines(owner).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].name, "renamed display");
    assert_eq!(listed[0].hostname, "changed-hostname");
    assert_eq!(listed[0].labels["purpose"], "test");
    assert!(service.machines(foreign).await.unwrap().is_empty());
    assert!(matches!(
        service
            .patch(
                foreign,
                enrolled.machine,
                MachinePatch {
                    name: Some("stolen".into()),
                    labels: None
                }
            )
            .await,
        Err(Error::NotFound)
    ));
    let auth = service
        .machine_challenge(enrolled.machine, initial.key)
        .await
        .unwrap();
    let token = service.machine_token(proof(&key, &auth)).await.unwrap();
    assert_eq!(
        service
            .authenticate_machine(&token.token)
            .await
            .unwrap()
            .machine,
        enrolled.machine
    );
    assert!(!format!("{token:?}").contains(&token.token));
    assert!(matches!(
        service.machine_token(proof(&key, &auth)).await,
        Err(Error::Unauthenticated)
    ));
    assert!(matches!(
        verifier.authenticate(&token.token).await,
        Err(Error::Unauthenticated)
    ));
    assert!(matches!(
        service.authenticate_machine(&owner_token).await,
        Err(Error::Unauthenticated)
    ));
    let expired = service
        .machine_challenge(enrolled.machine, initial.key)
        .await
        .unwrap();
    sqlx::query("UPDATE qgr.machine_challenges SET issued_at_ms=issued_at_ms-120000,expires_at_ms=expires_at_ms-120000 WHERE id=$1").bind(expired.challenge.to_string()).execute(admin).await.unwrap();
    assert!(matches!(
        service.machine_token(proof(&key, &expired)).await,
        Err(Error::Unauthenticated)
    ));
    let replacement = SigningKey::from_bytes(&rand::random());
    let mut recovery = initial.clone();
    recovery.key = KeyId::new();
    recovery.public_key = STANDARD.encode(replacement.verifying_key().as_bytes());
    assert!(matches!(
        service.enrollment_challenge(owner, recovery.clone()).await,
        Err(Error::Conflict)
    ));
    recovery.recovery = Some(Recovery {
        machine: enrolled.machine,
        confirmed: true,
    });
    let challenge = service
        .enrollment_challenge(owner, recovery.clone())
        .await
        .unwrap();
    assert_eq!(
        service
            .enroll(owner, proof(&replacement, &challenge))
            .await
            .unwrap()
            .machine,
        enrolled.machine
    );
    assert!(matches!(
        service.authenticate_machine(&token.token).await,
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        service.enrollment_challenge(owner, initial.clone()).await,
        Err(Error::Conflict)
    ));
    let key2 = SigningKey::from_bytes(&rand::random());
    let second = request(&key2);
    let challenge1 = service
        .enrollment_challenge(owner, second.clone())
        .await
        .unwrap();
    let challenge2 = service
        .enrollment_challenge(owner, second.clone())
        .await
        .unwrap();
    assert_eq!(challenge1.machine, challenge2.machine);
    let (one, two) = tokio::join!(
        service.enroll(owner, proof(&key2, &challenge1)),
        service.enroll(owner, proof(&key2, &challenge2))
    );
    let second_id = one.unwrap().machine;
    assert_eq!(second_id, two.unwrap().machine);
    assert_ne!(second_id, enrolled.machine);
    assert_eq!(service.machines(owner).await.unwrap().len(), 2);
    // Exercise the HTTP boundary with the same actual JWT verifier and authority service.
    gateway.auth = verifier;
    gateway.identity = Some(service.clone());
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let response = gateway
        .router()
        .oneshot(
            Request::builder()
                .uri("/v1/machines")
                .header("authorization", format!("Bearer {owner_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let machines: Vec<qgenie_remote_proto::wire::Machine> =
        serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap()).unwrap();
    assert_eq!(machines.len(), 2);
    let response = gateway
        .router()
        .oneshot(
            Request::builder()
                .uri("/v1/machines")
                .header("authorization", format!("Bearer {}", token.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let pending = service
        .machine_challenge(enrolled.machine, recovery.key)
        .await
        .unwrap();
    sqlx::query("UPDATE qgr.machine_keys SET revoked=true WHERE id=$1")
        .bind(recovery.key.to_string())
        .execute(admin)
        .await
        .unwrap();
    assert!(matches!(
        service.machine_token(proof(&replacement, &pending)).await,
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        service.enrollment_challenge(owner, recovery).await,
        Err(Error::Forbidden)
    ));
    let recovered: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM qgr.audit_events WHERE actor=$1 AND action='machine_key_recovered'",
    )
    .bind(owner.to_string())
    .fetch_one(admin)
    .await
    .unwrap();
    assert_eq!(recovered, 1);
}
