use super::*;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use tower::ServiceExt;

async fn post(gateway: &Gateway, token: &str, request: &RevokeAccess) -> (StatusCode, Vec<u8>) {
    let response = gateway
        .router()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1/admin/revocations")
                .header("authorization", format!("Bearer {token}"))
                .body(Body::from(serde_json::to_vec(request).unwrap()))
                .unwrap(),
        )
        .await
        .unwrap();
    (
        response.status(),
        to_bytes(response.into_body(), 65536)
            .await
            .unwrap()
            .to_vec(),
    )
}

pub(crate) async fn database_contract(store: Store, admin: &sqlx::PgPool, mut gateway: Gateway) {
    use crate::Authenticator;
    use crate::identity::jwt::tests as jwt;
    let verifier =
        Arc::new(crate::JwtVerifier::with_keys(jwt::config(), jwt::keys(), store.clone()).unwrap());
    let mut claims = jwt::claims("revocation-admin");
    claims["roles"] = serde_json::json!(["Remote.PlatformAdmin"]);
    let token = jwt::sign(&claims);
    let actor = verifier.authenticate(&token).await.unwrap().user;
    let owner_token = jwt::sign(&jwt::claims("revocation-target"));
    let user = verifier.authenticate(&owner_token).await.unwrap().user;
    gateway.auth = verifier;
    let mut request = RevokeAccess {
        request: RequestId::new(),
        target: RevocationTarget::User { user },
        confirmed: true,
    };
    assert_eq!(
        post(&gateway, &owner_token, &request).await.0,
        StatusCode::FORBIDDEN
    );
    request.confirmed = false;
    assert_eq!(
        post(&gateway, &token, &request).await.0,
        StatusCode::BAD_REQUEST
    );
    request.confirmed = true;
    let (one, two) = tokio::join!(
        post(&gateway, &token, &request),
        post(&gateway, &token, &request)
    );
    assert_eq!(one.0, StatusCode::OK);
    assert_eq!(two.0, StatusCode::OK);
    assert_eq!(one.1, two.1);
    let result: AccessRevocation = decode_json(&one.1).unwrap();
    assert!(matches!(result.propagation, RevocationPropagation::Pending));
    assert_eq!(result.target, request.target);
    // Lost HTTP acknowledgment: the same key returns exactly the durable result.
    assert_eq!(post(&gateway, &token, &request).await.1, one.1);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr.audit_events WHERE actor=$1 AND action='user_revoked' AND metadata->>'request'=$2")
        .bind(actor.to_string()).bind(request.request.to_string()).fetch_one(admin).await.unwrap();
    assert_eq!(count, 1);
    let response = gateway
        .router()
        .oneshot(
            Request::builder()
                .uri(format!("/v1/admin/revocations/{}", request.request))
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), 65536)
            .await
            .unwrap()
            .as_ref(),
        one.1
    );
    let original = request.request;
    request.target = RevocationTarget::Global;
    assert_eq!(
        post(&gateway, &token, &request).await.0,
        StatusCode::CONFLICT
    );
    request.request = RequestId::new();
    request.target = RevocationTarget::User {
        user: UserId::new(),
    };
    assert_eq!(
        post(&gateway, &token, &request).await.0,
        StatusCode::NOT_FOUND
    );
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM qgr.access_revocations WHERE actor=$1")
            .bind(actor.to_string())
            .fetch_one(admin)
            .await
            .unwrap();
    assert_eq!(count, 1);
    let machine: String = sqlx::query_scalar("SELECT id FROM qgr.machines LIMIT 1")
        .fetch_one(admin)
        .await
        .unwrap();
    request.target = RevocationTarget::Machine {
        machine: machine.parse().unwrap(),
    };
    assert_eq!(post(&gateway, &token, &request).await.0, StatusCode::OK);
    let revoked: bool =
        sqlx::query_scalar("SELECT bool_and(revoked) FROM qgr.machine_keys WHERE machine=$1")
            .bind(machine)
            .fetch_one(admin)
            .await
            .unwrap();
    assert!(revoked);
    request.request = RequestId::new();
    request.target = RevocationTarget::Global;
    assert_eq!(post(&gateway, &token, &request).await.0, StatusCode::OK);
    let disabled: bool = sqlx::query_scalar("SELECT disabled FROM qgr.access_state")
        .fetch_one(admin)
        .await
        .unwrap();
    assert!(disabled);
    // Test-fixture reset only; no public unrevocation or key resurrection API.
    sqlx::query("UPDATE qgr.access_state SET disabled=false,revision=18446744073709551615")
        .execute(admin)
        .await
        .unwrap();
    request.request = RequestId::new();
    request.target = RevocationTarget::User { user: actor };
    assert_eq!(
        post(&gateway, &token, &request).await.0,
        StatusCode::TOO_MANY_REQUESTS
    );
    let enabled: bool = sqlx::query_scalar("SELECT NOT disabled FROM qgr.users WHERE id=$1")
        .bind(actor.to_string())
        .fetch_one(admin)
        .await
        .unwrap();
    assert!(enabled);
    request.request = original;
    request.target = RevocationTarget::User { user };
    assert_eq!(post(&gateway, &token, &request).await.0, StatusCode::OK);
    sqlx::query("UPDATE qgr.access_state SET revision=1")
        .execute(admin)
        .await
        .unwrap();
}
use std::sync::Arc;
