use super::*;
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use qgenie_remote_proto::UserId;
use std::{io::Write, sync::Arc};
use tower::ServiceExt;
struct TestAuth(UserId);
impl Authenticator for TestAuth {
    fn authenticate<'a>(
        &'a self,
        token: &'a str,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Principal, Error>> + Send + 'a>>
    {
        Box::pin(async move {
            match token {
                "admin" => Ok(Principal {
                    user: self.0,
                    platform_admin: true,
                    expires_at: 4102444800,
                }),
                "owner" => Ok(Principal {
                    user: self.0,
                    platform_admin: false,
                    expires_at: 4102444800,
                }),
                _ => Err(Error::Unauthenticated),
            }
        })
    }
}
pub(crate) fn secret(text: &str) -> SecretDsn {
    let mut file = tempfile::NamedTempFile::new().unwrap();
    file.write_all(text.as_bytes()).unwrap();
    SecretDsn::from_file(file.path()).unwrap()
}
// Recording pools can be directed through a real transaction-pooling broker while
// authority/migration and outage fixtures retain their independent PostgreSQL connections.
pub(crate) fn recording_secret(text: &str) -> SecretDsn {
    let Some(path) = std::env::var_os("QGR_PGBOUNCER_TEST_URL_FILE") else {
        return secret(text);
    };
    let original = url::Url::parse(text).unwrap();
    let binding = SecretDsn::from_file(std::path::Path::new(&path)).unwrap();
    let mut broker = url::Url::parse(&binding.0).unwrap();
    broker.set_path(original.path());
    broker.set_username(original.username()).unwrap();
    broker.set_password(original.password()).unwrap();
    secret(broker.as_str())
}
#[tokio::test]
#[ignore = "requires disposable QGR_PG_TEST_URL_FILE, QGR_REDIS_TEST_URL_FILE, and QGR_GATEWAY_BINARY"]
async fn postgres_migrations_atomic_audit_roles_auth_and_readiness() {
    let file = std::env::var_os("QGR_PG_TEST_URL_FILE").expect("set QGR_PG_TEST_URL_FILE");
    let parent_secret = SecretDsn::from_file(std::path::Path::new(&file)).unwrap();
    let parent = sqlx::PgPool::connect(&parent_secret.0).await.unwrap();
    let nonce = UserId::new();
    let suffix = nonce
        .as_bytes()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let database = format!("qgr_test_{suffix}");
    // Audited identifier construction: fixed prefix plus generated lowercase hex only.
    sqlx::query(sqlx::AssertSqlSafe(format!("CREATE DATABASE {database}")))
        .execute(&parent)
        .await
        .unwrap();
    let mut url = url::Url::parse(&parent_secret.0).unwrap();
    url.set_path(&database);
    let migration = secret(url.as_str());
    Store::migrate(&migration).await.unwrap();
    Store::migrate(&migration).await.unwrap();
    let admin = sqlx::postgres::PgPoolOptions::new()
        .max_connections(1)
        .connect(&migration.0)
        .await
        .unwrap();
    sqlx::raw_sql("DO $$ BEGIN IF NOT EXISTS(SELECT FROM pg_roles WHERE rolname='qgr_test_app') THEN CREATE ROLE qgr_test_app LOGIN PASSWORD 'test-only'; END IF; IF NOT EXISTS(SELECT FROM pg_roles WHERE rolname='qgr_test_audit') THEN CREATE ROLE qgr_test_audit LOGIN PASSWORD 'test-only'; END IF; END $$; GRANT qgr_app TO qgr_test_app; GRANT qgr_audit TO qgr_test_audit; CREATE TABLE IF NOT EXISTS qgr.atomic_probe(value integer NOT NULL); GRANT INSERT,SELECT ON qgr.atomic_probe TO qgr_app;").execute(&admin).await.unwrap();
    let mut url = url::Url::parse(&migration.0).unwrap();
    url.set_username("qgr_test_app").unwrap();
    url.set_password(Some("test-only")).unwrap();
    let app_secret = secret(url.as_str());
    url.set_username("qgr_test_audit").unwrap();
    let audit_secret = secret(url.as_str());
    assert!(matches!(
        Store::connect(&migration, 2, false).await,
        Err(Error::Role)
    ));
    let store = Store::connect(&app_secret, 4, false).await.unwrap();
    store.compatible().await.unwrap();
    crate::identity::jwt::tests::trusted_jwks_contract(store.clone()).await;
    let verifier = JwtVerifier::with_keys(
        crate::identity::jwt::tests::config(),
        crate::identity::jwt::tests::keys(),
        store.clone(),
    )
    .unwrap();
    let jwt_claims = crate::identity::jwt::tests::claims("signed-owner");
    let jwt = crate::identity::jwt::tests::sign(&jwt_claims);
    let principal = verifier.authenticate(&jwt).await.unwrap();
    assert!(!principal.platform_admin);
    assert_eq!(
        verifier.authenticate(&jwt).await.unwrap().user,
        principal.user
    );
    let mut paired = jwt_claims.clone();
    paired["sub"] = serde_json::json!("another-client-subject");
    assert_eq!(
        verifier
            .authenticate(&crate::identity::jwt::tests::sign(&paired))
            .await
            .unwrap()
            .user,
        principal.user
    );
    let mut admin_claims = jwt_claims.clone();
    admin_claims["roles"] = serde_json::json!(["Remote.PlatformAdmin"]);
    assert!(
        verifier
            .authenticate(&crate::identity::jwt::tests::sign(&admin_claims))
            .await
            .unwrap()
            .platform_admin
    );
    sqlx::query("UPDATE qgr.users SET disabled=true WHERE id=$1")
        .bind(principal.user.to_string())
        .execute(&admin)
        .await
        .unwrap();
    assert!(matches!(
        verifier.authenticate(&jwt).await,
        Err(Error::Forbidden)
    ));
    assert!(matches!(
        verifier
            .authenticate(&crate::identity::jwt::tests::sign(&paired))
            .await,
        Err(Error::Forbidden)
    ));
    let mut foreign = jwt_claims;
    foreign["tid"] = serde_json::json!("foreign");
    assert!(matches!(
        verifier
            .authenticate(&crate::identity::jwt::tests::sign(&foreign))
            .await,
        Err(Error::Unauthenticated)
    ));
    let actor = UserId::new();
    let event = AuditEvent {
        actor,
        action: AuditAction::MachineEnrolled,
        machine: None,
        session: None,
        metadata: AuditMetadata::default(),
    };
    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr.audit_events")
        .fetch_one(&admin)
        .await
        .unwrap();
    let mut transaction = store.pool.begin().await.unwrap();
    sqlx::query("INSERT INTO qgr.atomic_probe(value) VALUES(1)")
        .execute(&mut *transaction)
        .await
        .unwrap();
    Store::append_audit(&mut transaction, &event).await.unwrap();
    transaction.rollback().await.unwrap();
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr.audit_events")
        .fetch_one(&admin)
        .await
        .unwrap();
    assert_eq!(before, count);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr.atomic_probe")
        .fetch_one(&admin)
        .await
        .unwrap();
    assert_eq!(count, 0);
    let mut transaction = store.pool.begin().await.unwrap();
    sqlx::query("INSERT INTO qgr.atomic_probe(value) VALUES(1)")
        .execute(&mut *transaction)
        .await
        .unwrap();
    Store::append_audit(&mut transaction, &event).await.unwrap();
    transaction.commit().await.unwrap();
    assert!(
        sqlx::query("SELECT * FROM qgr.audit_events")
            .fetch_all(&store.pool)
            .await
            .is_err()
    );
    for query in [
        "UPDATE qgr.audit_events SET action='rewritten'",
        "DELETE FROM qgr.audit_events",
        "TRUNCATE qgr.audit_events",
        "INSERT INTO qgr.audit_events(event_id,actor,action,metadata) VALUES(1,'forged','forged','{}')",
    ] {
        assert!(
            sqlx::query(query).execute(&store.pool).await.is_err(),
            "{query}"
        );
    }
    // Committed audit cursors cannot jump past another uncommitted writer.
    let mut first = store.pool.begin().await.unwrap();
    Store::append_audit(&mut first, &event).await.unwrap();
    let second_store = store.clone();
    let second_event = event.clone();
    let mut second = tokio::spawn(async move {
        let mut transaction = second_store.pool.begin().await.unwrap();
        Store::append_audit(&mut transaction, &second_event)
            .await
            .unwrap();
        transaction.commit().await.unwrap();
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(50), &mut second)
            .await
            .is_err()
    );
    first.commit().await.unwrap();
    second.await.unwrap();
    let config = Config {
        node_id: "test-node".into(),
        listen: "127.0.0.1:8081".parse().unwrap(),
        health_listen: "127.0.0.1:9091".parse().unwrap(),
        database: app_secret,
        audit_database: Some(audit_secret),
        pool_size: 4,
    };
    let gateway = Gateway::connect(&config, Arc::new(TestAuth(actor)))
        .await
        .unwrap();
    let response = gateway
        .router()
        .oneshot(
            Request::builder()
                .uri("/v1/admin/audit")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let response = gateway
        .router()
        .oneshot(
            Request::builder()
                .uri("/v1/admin/audit")
                .header("authorization", "Bearer owner")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let response = gateway
        .router()
        .oneshot(
            Request::builder()
                .uri("/v1/admin/audit?after=0&limit=200")
                .header("authorization", "Bearer admin")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let records: Vec<AuditRecord> =
        serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await.unwrap())
            .unwrap();
    assert!(
        records
            .iter()
            .any(|record| matches!(record.event.action, AuditAction::AuditRead))
    );
    let response = gateway
        .health_router()
        .oneshot(
            Request::builder()
                .uri("/ready")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    crate::identity::tests::enrollment_contract(store.clone(), &admin, gateway.clone()).await;
    crate::identity::freshness::tests::database_contract(store.clone(), &admin).await;
    crate::identity::freshness::tests::http_contract(store.clone(), &admin, gateway.clone()).await;
    crate::revocation::tests::database_contract(store.clone(), &admin, gateway.clone()).await;
    crate::routing::tests::routing_contract(store.clone(), &admin, &config).await;
    crate::routing::activation::tests::database_contract(store.clone(), &admin).await;
    crate::controllers::tests::database_contract(store.clone(), &admin).await;
    crate::transcripts::database::tests::database_contract(&admin, &migration, &store).await;
    crate::transcripts::ingest::tests::database_contract(store.clone(), &admin, &migration).await;
    crate::transcripts::read::tests::database_contract(store.clone(), &admin, &migration).await;
    crate::transcripts::holds::tests::database_contract(store.clone(), &admin, &migration).await;
    crate::transcripts::api::tests::database_contract(
        store.clone(),
        &admin,
        &migration,
        gateway.clone(),
    )
    .await;
    sqlx::query("INSERT INTO qgr.schema_versions(version,checksum) VALUES(99,'future')")
        .execute(&admin)
        .await
        .unwrap();
    assert!(matches!(gateway.ready().await, Err(Error::Schema)));
    assert!(matches!(
        Store::migrate(&migration).await,
        Err(Error::Schema)
    ));
    sqlx::query("DELETE FROM qgr.schema_versions WHERE version=99")
        .execute(&admin)
        .await
        .unwrap();
    let checksum: String =
        sqlx::query_scalar("SELECT checksum FROM qgr.schema_versions WHERE version=1")
            .fetch_one(&admin)
            .await
            .unwrap();
    sqlx::query("UPDATE qgr.schema_versions SET checksum='changed' WHERE version=1")
        .execute(&admin)
        .await
        .unwrap();
    assert!(matches!(
        Store::migrate(&migration).await,
        Err(Error::Schema)
    ));
    sqlx::query("UPDATE qgr.schema_versions SET checksum=$1 WHERE version=1")
        .bind(checksum)
        .execute(&admin)
        .await
        .unwrap();
    // Actual connection loss: prevent reconnects and terminate only this disposable DB.
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "ALTER DATABASE {database} ALLOW_CONNECTIONS false"
    )))
    .execute(&parent)
    .await
    .unwrap();
    sqlx::query("SELECT pg_terminate_backend(pid) FROM pg_stat_activity WHERE datname=$1")
        .bind(&database)
        .execute(&parent)
        .await
        .unwrap();
    assert!(matches!(gateway.ready().await, Err(Error::Database)));
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "ALTER DATABASE {database} ALLOW_CONNECTIONS true"
    )))
    .execute(&parent)
    .await
    .unwrap();
    gateway.ready().await.unwrap();
    gateway.drain();
    assert!(matches!(gateway.ready().await, Err(Error::Draining)));
    let response = gateway
        .health_router()
        .oneshot(Request::builder().uri("/live").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    gateway.shutdown().await;
    store.close().await;
    admin.close().await;
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "DROP DATABASE {database} WITH (FORCE)"
    )))
    .execute(&parent)
    .await
    .unwrap();
    parent.close().await;
}
