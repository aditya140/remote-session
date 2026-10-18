-- Preserve original native-observed end time when available; legacy/uncertain sessions
-- keep the conservative final projection upper bound without claiming exact capture.
CREATE OR REPLACE FUNCTION qgr_recordings.ingest_scope_base(machine_id text,key_id text,session_id text,claimed_owner text,token_expiry bigint)
RETURNS TABLE(owner_id text,created_at timestamptz,ended_upper timestamptz,recovery_epoch text,durability text,lifecycle_ready boolean,recovery_pending boolean,retention_days integer,retry_not_before timestamptz,synchronous_ready boolean)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE globally_disabled boolean; session_body jsonb; session_state text; last_projection timestamptz;
BEGIN
    SELECT a.disabled INTO STRICT globally_disabled FROM qgr.access_state a FOR SHARE;
    IF globally_disabled OR token_expiry<=extract(epoch FROM clock_timestamp()) THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
    -- Global -> user -> machine -> key -> session. Never invert primary domain lock order.
    PERFORM 1 FROM qgr.users u WHERE u.id=claimed_owner AND NOT u.disabled AND u.directory_enabled IS DISTINCT FROM false FOR SHARE;
    IF NOT FOUND THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
    PERFORM 1 FROM qgr.machines m WHERE m.id=machine_id AND m.owner=claimed_owner AND m.active_key=key_id AND NOT m.disabled FOR SHARE;
    IF NOT FOUND THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
    PERFORM 1 FROM qgr.machine_keys k WHERE k.id=key_id AND k.machine=machine_id AND NOT k.revoked FOR SHARE;
    IF NOT FOUND THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
    SELECT s.body,s.state,s.updated_at INTO session_body,session_state,last_projection FROM qgr.sessions s
      WHERE s.id=session_id AND s.machine=machine_id AND s.owner=claimed_owner FOR SHARE;
    IF NOT FOUND THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
    IF session_body->>'created_at' IS NULL THEN RAISE EXCEPTION 'missing original session lifetime' USING ERRCODE='22023'; END IF;
    PERFORM pg_advisory_xact_lock(5856751401703146820);
    PERFORM set_config('qgr.machine',machine_id,true),set_config('qgr.owner',claimed_owner,true);
    -- Even a duplicate writes: a new remote-apply commit must cover a prior receipt whose
    -- original COMMIT wait may have been interrupted after local commit.
    UPDATE qgr_recordings.commit_barrier SET touched_at=clock_timestamp() WHERE singleton;
    RETURN QUERY SELECT claimed_owner,(session_body->>'created_at')::timestamptz,
      CASE WHEN session_state IN ('exited','failed','lost','outcome_unknown') THEN COALESCE((session_body->>'ended_at')::timestamptz,last_projection) ELSE NULL::timestamptz END,
      c.recovery_epoch,c.durability,c.lifecycle_ready,c.recovery_pending,c.retention_days,c.retry_not_before,
      current_setting('synchronous_standby_names')<>'' AND EXISTS(SELECT 1 FROM pg_stat_replication r WHERE r.state='streaming' AND r.sync_state IN ('sync','quorum'))
    FROM qgr_recordings.state c WHERE c.singleton FOR SHARE OF c;
END $$;
REVOKE ALL ON FUNCTION qgr_recordings.ingest_scope_base(text,text,text,text,bigint) FROM PUBLIC,qgr_record_ingest;
