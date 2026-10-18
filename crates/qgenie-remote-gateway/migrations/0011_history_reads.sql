-- Bind RLS context to the actual transaction as well as the principal. This also denies
-- stale session-level context accidentally left on a PgBouncer server connection.
CREATE FUNCTION qgr_recordings.context_current() RETURNS boolean LANGUAGE sql STABLE SET search_path=pg_catalog AS $$
 SELECT nullif(current_setting('qgr.transaction',true),'')=pg_current_xact_id()::text
$$;
CREATE FUNCTION qgr_recordings.ingest_context(row_machine text,row_owner text) RETURNS boolean LANGUAGE sql STABLE SET search_path=pg_catalog AS $$
 SELECT qgr_recordings.context_current() AND row_machine=nullif(current_setting('qgr.machine',true),'') AND row_owner=nullif(current_setting('qgr.owner',true),'')
$$;
CREATE FUNCTION qgr_recordings.owner_context(row_owner text) RETURNS boolean LANGUAGE sql STABLE SET search_path=pg_catalog AS $$
 SELECT qgr_recordings.context_current() AND row_owner=nullif(current_setting('qgr.owner',true),'')
$$;
CREATE FUNCTION qgr_recordings.admin_context() RETURNS boolean LANGUAGE sql STABLE SET search_path=pg_catalog AS $$
 SELECT qgr_recordings.context_current() AND nullif(current_setting('qgr.actor',true),'') IS NOT NULL AND nullif(btrim(current_setting('qgr.reason',true)),'') IS NOT NULL
$$;
REVOKE ALL ON FUNCTION qgr_recordings.context_current(),qgr_recordings.ingest_context(text,text),qgr_recordings.owner_context(text),qgr_recordings.admin_context() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION qgr_recordings.context_current() TO qgr_record_ingest,qgr_history_owner,qgr_history_admin,qgr_record_lifecycle;
GRANT EXECUTE ON FUNCTION qgr_recordings.ingest_context(text,text) TO qgr_record_ingest;
GRANT EXECUTE ON FUNCTION qgr_recordings.owner_context(text) TO qgr_history_owner;
GRANT EXECUTE ON FUNCTION qgr_recordings.admin_context() TO qgr_history_admin;
ALTER POLICY receipt_ingest ON qgr_recordings.receipts USING(qgr_recordings.ingest_context(machine,owner)) WITH CHECK(qgr_recordings.ingest_context(machine,owner));
ALTER POLICY receipt_owner ON qgr_recordings.receipts USING(qgr_recordings.owner_context(owner));
ALTER POLICY receipt_admin ON qgr_recordings.receipts USING(qgr_recordings.admin_context());
ALTER POLICY epoch_ingest ON qgr_recordings.receipt_epochs USING(qgr_recordings.ingest_context(machine,owner)) WITH CHECK(qgr_recordings.ingest_context(machine,owner));
ALTER POLICY stream_ingest ON qgr_recordings.streams USING(qgr_recordings.ingest_context(machine,owner)) WITH CHECK(qgr_recordings.ingest_context(machine,owner));
ALTER POLICY fragment_ingest ON qgr_recordings.fragments USING(qgr_recordings.ingest_context(machine,owner)) WITH CHECK(qgr_recordings.ingest_context(machine,owner));
ALTER POLICY content_ingest ON qgr_recordings.content WITH CHECK(qgr_recordings.ingest_context(machine,owner));
ALTER POLICY content_owner ON qgr_recordings.content USING(qgr_recordings.owner_context(owner));
ALTER POLICY content_admin ON qgr_recordings.content USING(qgr_recordings.admin_context());
ALTER POLICY gap_ingest ON qgr_recordings.gaps WITH CHECK(qgr_recordings.ingest_context(machine,owner));
ALTER POLICY gap_owner ON qgr_recordings.gaps USING(qgr_recordings.owner_context(owner));
ALTER POLICY gap_admin ON qgr_recordings.gaps USING(qgr_recordings.admin_context());
ALTER POLICY held_admin ON qgr_recordings.held_content USING(qgr_recordings.admin_context() AND qgr_recordings.held_case_allowed(machine,session,id,nullif(btrim(current_setting('qgr.case',true)),'')));
CREATE POLICY fragment_owner ON qgr_recordings.fragments FOR SELECT TO qgr_history_owner USING(qgr_recordings.owner_context(owner));
CREATE POLICY fragment_admin ON qgr_recordings.fragments FOR SELECT TO qgr_history_admin USING(qgr_recordings.admin_context());
GRANT SELECT ON qgr_recordings.fragments TO qgr_history_owner,qgr_history_admin;
CREATE INDEX content_session ON qgr_recordings.content(machine,session,id);

ALTER FUNCTION qgr_recordings.ingest_scope(text,text,text,text,bigint) RENAME TO ingest_scope_base;
REVOKE ALL ON FUNCTION qgr_recordings.ingest_scope_base(text,text,text,text,bigint) FROM qgr_record_ingest;
CREATE FUNCTION qgr_recordings.ingest_scope(machine_id text,key_id text,session_id text,claimed_owner text,token_expiry bigint)
RETURNS TABLE(owner_id text,created_at timestamptz,ended_upper timestamptz,recovery_epoch text,durability text,lifecycle_ready boolean,recovery_pending boolean,retention_days integer,retry_not_before timestamptz,synchronous_ready boolean)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$ BEGIN
 PERFORM set_config('qgr.transaction',pg_current_xact_id()::text,true);
 RETURN QUERY SELECT * FROM qgr_recordings.ingest_scope_base(machine_id,key_id,session_id,claimed_owner,token_expiry);
END $$;
REVOKE ALL ON FUNCTION qgr_recordings.ingest_scope(text,text,text,text,bigint) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION qgr_recordings.ingest_scope(text,text,text,text,bigint) TO qgr_record_ingest;

CREATE TABLE qgr_recordings.held_gaps (
 machine text NOT NULL,session text NOT NULL,owner text NOT NULL,id text NOT NULL,
 captured_from timestamptz NOT NULL,captured_to timestamptz NOT NULL CHECK(captured_to>=captured_from),
 body jsonb NOT NULL CHECK(octet_length(body::text)<=8192),PRIMARY KEY(machine,session,id),
 FOREIGN KEY(session,machine,owner) REFERENCES qgr.sessions(id,machine,owner)
);
CREATE TABLE qgr_recordings.held_gap_cases (
 machine text NOT NULL,session text NOT NULL,id text NOT NULL,
 hold text NOT NULL CHECK(length(hold) BETWEEN 1 AND 64),case_ref text NOT NULL CHECK(length(btrim(case_ref)) BETWEEN 1 AND 256),
 PRIMARY KEY(machine,session,id,hold),FOREIGN KEY(machine,session,id) REFERENCES qgr_recordings.held_gaps(machine,session,id)
);
ALTER TABLE qgr_recordings.held_gaps ENABLE ROW LEVEL SECURITY;
ALTER TABLE qgr_recordings.held_gaps FORCE ROW LEVEL SECURITY;
ALTER TABLE qgr_recordings.held_gap_cases ENABLE ROW LEVEL SECURITY;
ALTER TABLE qgr_recordings.held_gap_cases FORCE ROW LEVEL SECURITY;
CREATE POLICY held_gap_internal ON qgr_recordings.held_gaps TO CURRENT_USER USING(true) WITH CHECK(true);
CREATE POLICY gap_coverage_internal ON qgr_recordings.held_gap_cases TO CURRENT_USER USING(true) WITH CHECK(true);
CREATE FUNCTION qgr_recordings.held_gap_case_allowed(machine_id text,session_id text,gap_id text,case_reference text) RETURNS boolean
 LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT EXISTS(SELECT 1 FROM qgr_recordings.held_gap_cases c WHERE c.machine=machine_id AND c.session=session_id AND c.id=gap_id AND c.case_ref=case_reference)
$$;
REVOKE ALL ON FUNCTION qgr_recordings.held_gap_case_allowed(text,text,text,text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION qgr_recordings.held_gap_case_allowed(text,text,text,text) TO qgr_history_admin;
CREATE POLICY held_gap_admin ON qgr_recordings.held_gaps FOR SELECT TO qgr_history_admin
 USING(qgr_recordings.admin_context() AND qgr_recordings.held_gap_case_allowed(machine,session,id,nullif(btrim(current_setting('qgr.case',true)),'')));
GRANT SELECT ON qgr_recordings.held_gaps TO qgr_history_admin;

CREATE FUNCTION qgr_recordings.owner_read_scope(actor_id text,session_id text,token_expiry bigint)
RETURNS TABLE(machine_id text,owner_id text,recovery_epoch text) LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE disabled boolean; target_machine text;
BEGIN
 SELECT a.disabled INTO STRICT disabled FROM qgr.access_state a FOR SHARE;
 IF disabled OR token_expiry IS NULL OR token_expiry<=extract(epoch FROM clock_timestamp()) THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
 PERFORM 1 FROM qgr.users u WHERE u.id=actor_id AND NOT u.disabled AND u.directory_object IS NOT NULL AND u.directory_enabled IS DISTINCT FROM false FOR SHARE;
 IF NOT FOUND THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
 SELECT s.machine INTO target_machine FROM qgr.sessions s WHERE s.id=session_id AND s.owner=actor_id FOR SHARE;
 IF NOT FOUND THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
 PERFORM set_config('qgr.transaction',pg_current_xact_id()::text,true),set_config('qgr.owner',actor_id,true),set_config('qgr.actor','',true),set_config('qgr.reason','',true),set_config('qgr.case','',true);
 RETURN QUERY SELECT target_machine,actor_id,c.recovery_epoch FROM qgr_recordings.state c WHERE c.singleton AND c.lifecycle_ready AND NOT c.recovery_pending FOR SHARE OF c;
END $$;
REVOKE ALL ON FUNCTION qgr_recordings.owner_read_scope(text,text,bigint) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION qgr_recordings.owner_read_scope(text,text,bigint) TO qgr_history_owner;

CREATE FUNCTION qgr_recordings.admin_read_scope(actor_id text,session_id text,token_expiry bigint,read_reason text,case_reference text,include_held boolean)
RETURNS TABLE(machine_id text,owner_id text,recovery_epoch text) LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE target_machine text; target_owner text;
BEGIN
 PERFORM 1 FROM qgr.access_state FOR SHARE; -- administrator plane remains available during global disable
 IF token_expiry IS NULL OR token_expiry<=extract(epoch FROM clock_timestamp()) OR coalesce(length(btrim(read_reason)),0) NOT BETWEEN 1 AND 512 OR (include_held AND coalesce(length(btrim(case_reference)),0)=0) OR coalesce(length(case_reference),0)>256 THEN RAISE EXCEPTION 'invalid read context' USING ERRCODE='22023'; END IF;
 PERFORM 1 FROM qgr.users u WHERE u.id=actor_id AND NOT u.disabled AND u.directory_object IS NOT NULL AND u.directory_enabled IS DISTINCT FROM false FOR SHARE;
 IF NOT FOUND THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
 SELECT s.machine,s.owner INTO target_machine,target_owner FROM qgr.sessions s WHERE s.id=session_id FOR SHARE;
 IF NOT FOUND THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
 IF include_held AND NOT EXISTS(SELECT 1 FROM qgr_recordings.held_cases c WHERE c.machine=target_machine AND c.session=session_id AND c.case_ref=btrim(case_reference)) AND NOT EXISTS(SELECT 1 FROM qgr_recordings.held_gap_cases c WHERE c.machine=target_machine AND c.session=session_id AND c.case_ref=btrim(case_reference)) THEN RAISE EXCEPTION 'case does not cover requested content' USING ERRCODE='42501'; END IF;
 PERFORM set_config('qgr.transaction',pg_current_xact_id()::text,true),set_config('qgr.owner','',true),set_config('qgr.actor',actor_id,true),set_config('qgr.reason',btrim(read_reason),true),set_config('qgr.case',coalesce(btrim(case_reference),''),true);
 RETURN QUERY SELECT target_machine,target_owner,c.recovery_epoch FROM qgr_recordings.state c WHERE c.singleton AND c.lifecycle_ready AND NOT c.recovery_pending FOR SHARE OF c;
END $$;
REVOKE ALL ON FUNCTION qgr_recordings.admin_read_scope(text,text,bigint,text,text,boolean) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION qgr_recordings.admin_read_scope(text,text,bigint,text,text,boolean) TO qgr_history_admin;
