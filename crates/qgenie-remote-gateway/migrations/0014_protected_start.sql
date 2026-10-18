-- Public start must establish protected recording readiness before native launch.
-- This function discloses no transcript and grants no table mutation capability.
CREATE FUNCTION qgr_recordings.preflight_start(machine_id text,claimed_owner text,token_expiry bigint)
RETURNS TABLE(durability text,lifecycle_ready boolean,recovery_pending boolean,synchronous_ready boolean)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE globally_disabled boolean; active_key text;
BEGIN
    SELECT a.disabled INTO STRICT globally_disabled FROM qgr.access_state a FOR SHARE;
    IF globally_disabled OR token_expiry<=extract(epoch FROM clock_timestamp()) THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
    PERFORM 1 FROM qgr.users u WHERE u.id=claimed_owner AND NOT u.disabled AND u.directory_enabled IS TRUE AND u.directory_object IS NOT NULL FOR SHARE;
    IF NOT FOUND THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
    SELECT m.active_key INTO active_key FROM qgr.machines m WHERE m.id=machine_id AND m.owner=claimed_owner AND NOT m.disabled FOR SHARE;
    IF NOT FOUND THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
    PERFORM 1 FROM qgr.machine_keys k WHERE k.id=active_key AND k.machine=machine_id AND NOT k.revoked FOR SHARE;
    IF NOT FOUND THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
    PERFORM pg_advisory_xact_lock(5856751401703146820);
    -- A fresh write makes the subsequent remote_apply COMMIT a readiness barrier.
    UPDATE qgr_recordings.commit_barrier SET touched_at=clock_timestamp() WHERE singleton;
    RETURN QUERY SELECT c.durability,c.lifecycle_ready,c.recovery_pending,
      current_setting('synchronous_standby_names')<>'' AND EXISTS(SELECT 1 FROM pg_stat_replication r WHERE r.state='streaming' AND r.sync_state IN ('sync','quorum'))
    FROM qgr_recordings.state c WHERE c.singleton FOR SHARE OF c;
END $$;
REVOKE ALL ON FUNCTION qgr_recordings.preflight_start(text,text,bigint) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION qgr_recordings.preflight_start(text,text,bigint) TO qgr_record_ingest;
