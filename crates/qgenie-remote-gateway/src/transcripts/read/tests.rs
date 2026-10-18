use super::*;
use crate::{Authenticator, JwtVerifier, SecretDsn, identity::jwt::tests as jwt};

#[test]
fn cursors_keep_full_precision_and_cannot_change_shape_or_read_context() {
    let position = Position {
        seq: Counter(u64::MAX),
        fragment: 32,
        kind: 1,
        id: ChunkId::new().to_string(),
    };
    let token = encode_cursor(Cursor {
        v: 1,
        session: SessionId::new(),
        stream_epoch: RecordingEpochId::new(),
        backend_epoch: "test-epoch".into(),
        held: false,
        case_hash: None,
        position: position.clone(),
    })
    .unwrap();
    assert_eq!(decode_cursor(&token).unwrap().position, position);
    for malformed in ["", "A", "===", "!!"] {
        assert!(decode_cursor(malformed).is_err());
    }
    let mut cursor = decode_cursor(&token).unwrap();
    cursor.position.kind = 0;
    assert!(decode_cursor(&encode_cursor(cursor).unwrap()).is_err());
    assert!(admin_context(" ".into(), None, false).is_err());
    assert!(admin_context("reason".into(), None, true).is_err());
    assert!(
        HistoryQuery {
            stream_epoch: RecordingEpochId::new(),
            after: Some(" ".into()),
            limit: 4
        }
        .validate()
        .is_err()
    );
}

pub(crate) async fn database_contract(store: Store, admin: &sqlx::PgPool, migration: &SecretDsn) {
    let verifier = JwtVerifier::with_keys(jwt::config(), jwt::keys(), store.clone()).unwrap();
    let owner = verifier
        .authenticate(&jwt::sign(&jwt::claims("recording-ingest-owner")))
        .await
        .unwrap();
    let foreign = verifier
        .authenticate(&jwt::sign(&jwt::claims("history-foreign-owner")))
        .await
        .unwrap();
    let mut claims = jwt::claims("history-platform-admin");
    claims["roles"] = serde_json::json!([jwt::config().admin_role]);
    let administrator = verifier.authenticate(&jwt::sign(&claims)).await.unwrap();
    assert!(administrator.platform_admin);
    let record=sqlx::query("SELECT machine,session,id,immutable->'batch'->>'stream_epoch' AS epoch FROM qgr_recordings.receipts WHERE owner=$1 AND kind='content' AND immutable->'batch'->>'from'='1' AND captured_from>clock_timestamp()-interval '1 day' ORDER BY committed_at LIMIT 1").bind(owner.user.to_string()).fetch_one(admin).await.unwrap();
    let machine: MachineId = record.get::<String, _>("machine").parse().unwrap();
    let session: SessionId = record.get::<String, _>("session").parse().unwrap();
    let epoch: RecordingEpochId = record.get::<String, _>("epoch").parse().unwrap();
    let chunk: ChunkId = record.get::<String, _>("id").parse().unwrap();
    let file = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(file.path(), STANDARD.encode([74; 32])).unwrap();
    let identity =
        IdentityService::from_file(store, "https://gateway.example.invalid".into(), file.path())
            .await
            .unwrap();
    let mut url = url::Url::parse(&migration.0).unwrap();
    url.set_username("qgr_test_history_owner").unwrap();
    url.set_password(Some("test-only")).unwrap();
    let reader = RecordingDatabase::connect(
        &crate::tests::recording_secret(url.as_str()),
        RecordingRole::OwnerRead,
        1,
    )
    .await
    .unwrap();
    url.set_username("qgr_test_history_admin").unwrap();
    let privileged = RecordingDatabase::connect(
        &crate::tests::recording_secret(url.as_str()),
        RecordingRole::AdminRead,
        1,
    )
    .await
    .unwrap();
    crate::transcripts::ingest::tests::state(admin, "synthetic-history-1", false).await;
    let query = HistoryQuery {
        stream_epoch: epoch,
        after: None,
        limit: 1,
    };
    assert!(matches!(
        reader
            .owner_history(&identity, &owner, session, query.clone())
            .await,
        Err(Error::Configuration)
    ));
    assert!(matches!(
        reader
            .history(&identity, &foreign, session, query.clone(), None, true)
            .await,
        Err(Error::Forbidden)
    ));
    let mut page = reader
        .history(&identity, &owner, session, query.clone(), None, true)
        .await
        .unwrap();
    assert!(matches!(&page.items[0],HistoryItem::Content(c) if c.id==chunk));
    let old_cursor = page.next.clone().unwrap();
    let mut count = page.items.len();
    let mut output = Vec::new();
    loop {
        for item in &page.items {
            if let HistoryItem::Content(c) = item {
                let raw = zstd::bulk::decompress(&STANDARD.decode(&c.data).unwrap(), 1024 * 1024)
                    .unwrap();
                for line in raw.split(|b| *b == b'\n').filter(|l| !l.is_empty()) {
                    let r: qgenie_remote_proto::RecordingRecord =
                        serde_json::from_slice(line).unwrap();
                    if let qgenie_remote_proto::RecordedContent::Output { data, .. } = r.content {
                        output.extend(STANDARD.decode(data).unwrap());
                    }
                }
            }
        }
        let Some(after) = page.next else {
            break;
        };
        page = reader
            .history(
                &identity,
                &owner,
                session,
                HistoryQuery {
                    after: Some(after),
                    ..query.clone()
                },
                None,
                true,
            )
            .await
            .unwrap();
        count += page.items.len();
    }
    assert_eq!(count, 6);
    assert!(
        output
            .windows(b"**************".len())
            .any(|w| w == b"**************")
    );
    assert!(
        !output
            .windows(b"fixture-secret".len())
            .any(|w| w == b"fixture-secret")
    );
    let normal = admin_context("bounded synthetic investigation".into(), None, false).unwrap();
    let page = privileged
        .history(
            &identity,
            &administrator,
            session,
            query.clone(),
            Some(normal),
            true,
        )
        .await
        .unwrap();
    assert_eq!(page.items.len(), 1);
    let audit:Value=sqlx::query_scalar("SELECT metadata FROM qgr.audit_events WHERE actor=$1 AND action='transcript_read' ORDER BY event_id DESC LIMIT 1").bind(administrator.user.to_string()).fetch_one(admin).await.unwrap();
    assert_eq!(
        audit["transcript"]["reason"],
        "bounded synthetic investigation"
    );
    assert_eq!(audit["transcript"]["content"][0], chunk.to_string());
    sqlx::raw_sql("REVOKE INSERT(actor,action,machine,session,metadata) ON qgr.audit_events FROM qgr_history_admin").execute(admin).await.unwrap();
    assert!(matches!(
        privileged
            .history(
                &identity,
                &administrator,
                session,
                query.clone(),
                Some(admin_context("must not release bytes".into(), None, false).unwrap()),
                true
            )
            .await,
        Err(Error::Database)
    ));
    sqlx::raw_sql("GRANT INSERT(actor,action,machine,session,metadata) ON qgr.audit_events TO qgr_history_admin").execute(admin).await.unwrap();
    let gap: GapId = sqlx::query_scalar::<_, String>(
        "SELECT id FROM qgr_recordings.gaps WHERE machine=$1 AND session=$2 LIMIT 1",
    )
    .bind(machine.to_string())
    .bind(session.to_string())
    .fetch_one(admin)
    .await
    .unwrap()
    .parse()
    .unwrap();
    sqlx::query("INSERT INTO qgr_recordings.holds(id,owner,revision,active,case_ref,reason,actor) VALUES('hold_00000000-0000-4000-8000-000000000002',$1,1,true,'history-case-1','synthetic read fixture',$2)").bind(owner.user.to_string()).bind(administrator.user.to_string()).execute(admin).await.unwrap();
    sqlx::query("INSERT INTO qgr_recordings.held_content(machine,session,owner,id,captured_from,captured_to,payload,stored_digest) SELECT machine,session,owner,id,captured_from,captured_to,payload,stored_digest FROM qgr_recordings.content WHERE id=$1").bind(chunk.to_string()).execute(admin).await.unwrap();
    sqlx::query("INSERT INTO qgr_recordings.held_cases(machine,session,id,hold,case_ref) VALUES($1,$2,$3,'hold_00000000-0000-4000-8000-000000000002','history-case-1')").bind(machine.to_string()).bind(session.to_string()).bind(chunk.to_string()).execute(admin).await.unwrap();
    sqlx::query("INSERT INTO qgr_recordings.held_gaps(machine,session,owner,id,captured_from,captured_to,body) SELECT machine,session,owner,id,captured_from,captured_to,body FROM qgr_recordings.gaps WHERE id=$1").bind(gap.to_string()).execute(admin).await.unwrap();
    sqlx::query("INSERT INTO qgr_recordings.held_gap_cases(machine,session,id,hold,case_ref) VALUES($1,$2,$3,'hold_00000000-0000-4000-8000-000000000002','history-case-1')").bind(machine.to_string()).bind(session.to_string()).bind(gap.to_string()).execute(admin).await.unwrap();
    sqlx::query("DELETE FROM qgr_recordings.content WHERE id=$1")
        .bind(chunk.to_string())
        .execute(admin)
        .await
        .unwrap();
    sqlx::query("DELETE FROM qgr_recordings.gaps WHERE id=$1")
        .bind(gap.to_string())
        .execute(admin)
        .await
        .unwrap();
    let public = reader
        .history(
            &identity,
            &owner,
            session,
            HistoryQuery {
                limit: 4,
                ..query.clone()
            },
            None,
            true,
        )
        .await
        .unwrap();
    assert!(
        !public
            .items
            .iter()
            .any(|item| matches!(item,HistoryItem::Content(c) if c.id==chunk)
                || matches!(item,HistoryItem::Gap(g) if g.id==gap))
    );
    let serialized = serde_json::to_value(&public).unwrap();
    assert!(serialized.get("hold").is_none());
    assert!(serialized.get("case_ref").is_none());
    assert!(serialized.get("reason").is_none());
    assert!(matches!(
        privileged
            .history(
                &identity,
                &administrator,
                session,
                query.clone(),
                Some(admin_context("wrong case".into(), Some("other-case".into()), true).unwrap()),
                true
            )
            .await,
        Err(Error::Forbidden)
    ));
    let private = privileged
        .history(
            &identity,
            &administrator,
            session,
            HistoryQuery {
                limit: 4,
                ..query.clone()
            },
            Some(
                admin_context("correct case".into(), Some("history-case-1".into()), true).unwrap(),
            ),
            true,
        )
        .await
        .unwrap();
    assert!(
        private
            .items
            .iter()
            .any(|item| matches!(item,HistoryItem::Content(c) if c.id==chunk))
    );
    assert!(
        private
            .items
            .iter()
            .any(|item| matches!(item,HistoryItem::Gap(g) if g.id==gap))
    );
    sqlx::query("UPDATE qgr.access_state SET disabled=true")
        .execute(admin)
        .await
        .unwrap();
    assert!(matches!(
        reader
            .history(&identity, &owner, session, query.clone(), None, true)
            .await,
        Err(Error::Forbidden)
    ));
    privileged
        .history(
            &identity,
            &administrator,
            session,
            query.clone(),
            Some(admin_context("administrator plane".into(), None, false).unwrap()),
            true,
        )
        .await
        .unwrap();
    sqlx::query("UPDATE qgr.access_state SET disabled=false")
        .execute(admin)
        .await
        .unwrap();
    sqlx::query("SELECT set_config('qgr.owner',$1,false),set_config('qgr.transaction',pg_current_xact_id()::text,false)").bind(owner.user.to_string()).execute(&reader.pool).await.unwrap();
    let visible: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.content")
        .fetch_one(&reader.pool)
        .await
        .unwrap();
    assert_eq!(visible, 0);
    if std::env::var_os("QGR_PGBOUNCER_TEST_URL_FILE").is_some() {
        url.set_username("qgr_test_history_owner").unwrap();
        let peer = RecordingDatabase::connect(
            &crate::tests::recording_secret(url.as_str()),
            RecordingRole::OwnerRead,
            1,
        )
        .await
        .unwrap();
        let first: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&reader.pool)
            .await
            .unwrap();
        let second: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
            .fetch_one(&peer.pool)
            .await
            .unwrap();
        assert_eq!(
            first, second,
            "two frontends must reuse the same transaction-pool backend"
        );
        let visible: i64 = sqlx::query_scalar("SELECT count(*) FROM qgr_recordings.content")
            .fetch_one(&peer.pool)
            .await
            .unwrap();
        assert_eq!(
            visible, 0,
            "stale session context must not grant a different frontend access"
        );
        peer.close().await;
    }
    crate::transcripts::ingest::tests::state(admin, "synthetic-history-2", false).await;
    assert!(matches!(
        reader
            .history(
                &identity,
                &owner,
                session,
                HistoryQuery {
                    after: Some(old_cursor),
                    ..query
                },
                None,
                true
            )
            .await,
        Err(Error::HistoryTruncated)
    ));
    crate::transcripts::ingest::tests::state(admin, "unqualified-after-history", true).await;
    sqlx::query("UPDATE qgr_recordings.state SET durability='unqualified',lifecycle_ready=false")
        .execute(admin)
        .await
        .unwrap();
    reader.close().await;
    privileged.close().await;
}
