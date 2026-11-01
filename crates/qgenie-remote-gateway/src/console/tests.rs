use super::{read, types::*};
#[test]
fn pagination_rejects_invalid_limits_and_cursors_and_keeps_stable_order() {
    for limit in [0, 101, usize::MAX] {
        let q = Query {
            limit,
            cursor: None,
            owner: None,
            machine: None,
        };
        assert!(q.validate().is_err());
    }
    let q = Query {
        limit: 2,
        cursor: Some("a".into()),
        owner: None,
        machine: None,
    };
    q.validate().unwrap();
    let coverage = Coverage {
        complete: true,
        users: 0,
        machines: 0,
        sessions: 3,
        limit: 500,
        freshness_ms: 30_000,
    };
    let p = read::paginate(
        vec![
            "a".to_string(),
            "b".to_string(),
            "c".to_string(),
            "d".to_string(),
        ],
        &q,
        |v| v,
        1,
        coverage,
    );
    assert_eq!(p.items, vec!["b", "c"]);
    assert_eq!(p.next.as_deref(), Some("c"));
    for cursor in ["", "id/../../secret", "bad&cursor"] {
        assert!(
            Query {
                cursor: Some(cursor.into()),
                ..q.clone()
            }
            .validate()
            .is_err()
        );
    }
}

pub(crate) async fn database_contract(gateway: &crate::Gateway, db: &sqlx::PgPool) {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use qgenie_remote_proto::{KeyId, MachineId, SessionId, UserId};
    use tower::ServiceExt;
    for (token, expected) in [
        (None, StatusCode::UNAUTHORIZED),
        (Some("owner"), StatusCode::FORBIDDEN),
        (Some("admin"), StatusCode::OK),
    ] {
        let mut request = Request::builder().uri("/v1/admin/health");
        if let Some(token) = token {
            request = request.header("authorization", format!("Bearer {token}"));
        }
        let response = gateway
            .router()
            .oneshot(request.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), expected);
        if expected == StatusCode::OK {
            assert_eq!(response.headers()["cache-control"], "private, no-store");
            let value: serde_json::Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 8192).await.unwrap())
                    .unwrap();
            assert_eq!(value["scope"], "accepting_gateway_process");
        }
    }
    let health = super::health::sample(gateway).await.unwrap();
    assert_eq!(health["components"]["database"]["status"], "ready");
    assert_eq!(health["components"]["routing"]["configured"], false);
    assert_eq!(
        health["components"]["recording"]["commit_durability"],
        "not_probed"
    );
    assert_eq!(health["metrics_endpoint_public"], false);
    let serialized_health = health.to_string();
    assert!(!serialized_health.contains("postgres://"));
    assert!(!serialized_health.contains("127.0.0.1"));

    use serde_json::json;
    let owner = UserId::new();
    let other = UserId::new();
    let machine = MachineId::new();
    let foreign = MachineId::new();
    let session = SessionId::new();
    for user in [owner, other] {
        sqlx::query("INSERT INTO qgr.users(id,issuer,tenant,subject) VALUES($1,'console-test','fixture',$1)")
            .bind(user.to_string()).execute(db).await.unwrap();
    }
    for (id, user) in [(machine, owner), (foreign, other)] {
        sqlx::query("INSERT INTO qgr.machines(id,owner,installation,os_user,active_key,hostname,name,os,arch,cli_version,labels) VALUES($1,$2,$1,'fixture',$3,'console.local','Console host','Linux','x86_64','test','{\"device_class\":\"server\"}')")
            .bind(id.to_string()).bind(user.to_string()).bind(KeyId::new().to_string()).execute(db).await.unwrap();
        sqlx::query("INSERT INTO qgr.machine_routes(machine,generation,node,connection,activated,agents) VALUES($1,1,'fixture','fixture',true,'{\"codex\":{\"version\":\"1.0\",\"capabilities\":{}}}')")
            .bind(id.to_string()).execute(db).await.unwrap();
    }
    let body = json!({"kind":{"kind":"agent","agent":"codex","prompt":"never-expose-this-prompt","native_resume_id":"private-native-id"},"cwd":"/private/path","requested_model":"configured-safe-model","state":"running","origin":"managed","created_at":"2026-10-31T13:42:00.000Z","capabilities":{"terminate":true},"recording_warning":null});
    sqlx::query(
        "INSERT INTO qgr.sessions(id,machine,owner,body,state) VALUES($1,$2,$3,$4,'running')",
    )
    .bind(session.to_string())
    .bind(machine.to_string())
    .bind(owner.to_string())
    .bind(body)
    .execute(db)
    .await
    .unwrap();
    let snapshot = read::snapshot(gateway, Some(owner)).await.unwrap();
    assert_eq!(snapshot.users.len(), 1);
    assert_eq!(snapshot.users[0].id, owner.to_string());
    assert_eq!(snapshot.machines.len(), 1);
    assert_eq!(snapshot.machines[0].id, machine.to_string());
    assert!(snapshot.machines[0].connected);
    assert_eq!(snapshot.machines[0].device_class, "server");
    assert_eq!(snapshot.users[0].active_sessions, 1);
    assert_eq!(snapshot.sessions[0].harness, "codex");
    assert_eq!(snapshot.sessions[0].harness_version.as_deref(), Some("1.0"));
    assert_eq!(snapshot.sessions[0].origin, "managed");
    assert!(snapshot.sessions[0].terminate);
    assert_eq!(snapshot.sessions[0].created_at_ms, Some(1793454120000));
    assert!(snapshot.sessions[0].model.is_none());
    assert_eq!(
        snapshot.sessions[0].requested_model.as_deref(),
        Some("configured-safe-model")
    );
    assert!(snapshot.sessions[0].effective_model.is_none());
    assert_eq!(snapshot.sessions[0].model_scope, "session_configuration");
    assert_eq!(snapshot.sessions[0].task_state, "unknown");
    let mut activation = gateway.store.transaction().await.unwrap();
    crate::Store::append_audit(
        &mut activation,
        &crate::AuditEvent {
            actor: owner,
            action: crate::AuditAction::HostActivated,
            machine: Some(machine),
            session: None,
            metadata: crate::AuditMetadata::default(),
        },
    )
    .await
    .unwrap();
    activation.commit().await.unwrap();
    let mut connection_snapshot = read::snapshot(gateway, Some(owner)).await.unwrap();
    read::last_connections(gateway, owner, &mut connection_snapshot.machines)
        .await
        .unwrap();
    assert!(connection_snapshot.machines[0].last_connected_ms.is_some());
    for uri in [
        format!("/v1/admin/machines?owner={owner}"),
        format!("/v1/admin/users/{owner}/devices"),
    ] {
        let response = gateway
            .router()
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header("authorization", "Bearer admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let value: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 16 * 1024).await.unwrap())
                .unwrap();
        assert_eq!(value["items"].as_array().unwrap().len(), 1);
        assert_eq!(value["items"][0]["id"], machine.to_string());
        assert_eq!(
            value["items"][0]["last_connected_ms"],
            connection_snapshot.machines[0].last_connected_ms.unwrap()
        );
        assert!(!value.to_string().contains(&foreign.to_string()));
    }

    let mut events = gateway.store.transaction().await.unwrap();
    let repeated_session = SessionId::new();
    for i in 0..21 {
        crate::Store::append_audit(
            &mut events,
            &crate::AuditEvent {
                actor: owner,
                action: crate::AuditAction::SessionStarted,
                machine: Some(machine),
                session: Some(if i < 2 {
                    repeated_session
                } else {
                    SessionId::new()
                }),
                metadata: crate::AuditMetadata::default(),
            },
        )
        .await
        .unwrap();
    }
    for _ in 0..3 {
        crate::Store::append_audit(
            &mut events,
            &crate::AuditEvent {
                actor: owner,
                action: crate::AuditAction::MachineEnrolled,
                machine: Some(MachineId::new()),
                session: None,
                metadata: crate::AuditMetadata::default(),
            },
        )
        .await
        .unwrap();
    }
    events.commit().await.unwrap();
    let (signals, complete) =
        super::telemetry::burst_signals(gateway, owner, read::now_ms().unwrap())
            .await
            .unwrap();
    assert!(complete);
    let start_signal = signals
        .iter()
        .find(|s| s["subject"] == owner.to_string() && s["kind"] == "session_start_burst")
        .unwrap();
    assert_eq!(start_signal["evidence"]["distinct_resources"], 20);
    assert_eq!(start_signal["automatic_action"], false);
    assert!(
        signals
            .iter()
            .any(|s| s["subject"] == owner.to_string() && s["kind"] == "device_enrollment_burst")
    );
    let serialized = serde_json::to_string(&snapshot).unwrap();
    assert!(!serialized.contains("never-expose"));
    assert!(!serialized.contains("private-native"));
    assert!(!serialized.contains("private/path"));
    assert!(!serialized.contains(&other.to_string()));
    assert!(!serialized.contains(&foreign.to_string()));
    sqlx::query(
        "UPDATE qgr.sessions SET updated_at=clock_timestamp()-interval '45 seconds' WHERE id=$1",
    )
    .bind(session.to_string())
    .execute(db)
    .await
    .unwrap();
    let stale = read::snapshot(gateway, Some(owner)).await.unwrap();
    assert_eq!(stale.sessions[0].state, "unknown");
    assert!(!stale.sessions[0].terminate);
    assert_eq!(stale.users[0].active_sessions, 0);
    assert_eq!(stale.users[0].unknown_sessions, 1);
    sqlx::query("UPDATE qgr.sessions SET state='exited' WHERE id=$1")
        .bind(session.to_string())
        .execute(db)
        .await
        .unwrap();
    assert_eq!(
        read::snapshot(gateway, Some(owner)).await.unwrap().sessions[0].state,
        "exited"
    );
    sqlx::query(
        "UPDATE qgr.machines SET last_seen=clock_timestamp()-interval '45 seconds' WHERE id=$1",
    )
    .bind(machine.to_string())
    .execute(db)
    .await
    .unwrap();
    assert!(!read::snapshot(gateway, Some(owner)).await.unwrap().machines[0].connected);
    sqlx::query("DELETE FROM qgr.sessions WHERE id=$1")
        .bind(session.to_string())
        .execute(db)
        .await
        .unwrap();
    sqlx::query("DELETE FROM qgr.machine_routes WHERE machine=ANY($1)")
        .bind(vec![machine.to_string(), foreign.to_string()])
        .execute(db)
        .await
        .unwrap();
    sqlx::query("DELETE FROM qgr.machines WHERE id=ANY($1)")
        .bind(vec![machine.to_string(), foreign.to_string()])
        .execute(db)
        .await
        .unwrap();
    sqlx::query("DELETE FROM qgr.users WHERE id=ANY($1)")
        .bind(vec![owner.to_string(), other.to_string()])
        .execute(db)
        .await
        .unwrap();
}
