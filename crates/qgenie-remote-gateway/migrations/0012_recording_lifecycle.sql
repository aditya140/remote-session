-- The same database-authoritative lock fences ingest, hold changes and final partition DROP.
-- None of the service roles can mutate these tables directly.
ALTER TABLE qgr_recordings.state ADD COLUMN database_identity uuid NOT NULL DEFAULT gen_random_uuid();
CREATE FUNCTION qgr_recordings.database_identity() RETURNS text LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT s.database_identity::text FROM qgr_recordings.state s WHERE s.singleton
$$;
REVOKE ALL ON FUNCTION qgr_recordings.database_identity() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION qgr_recordings.database_identity() TO qgr_app,qgr_record_ingest,qgr_history_owner,qgr_history_admin,qgr_record_lifecycle;
GRANT USAGE ON SCHEMA qgr_recordings TO qgr_app;
CREATE FUNCTION qgr_recordings.audit_durability_ready() RETURNS boolean
LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT coalesce(qgr_recordings.admin_context(),false) AND s.lifecycle_ready AND NOT s.recovery_pending
 AND s.durability='synchronous_standby' AND current_setting('synchronous_standby_names')<>''
 AND EXISTS(SELECT 1 FROM pg_stat_replication r WHERE r.state='streaming' AND r.sync_state IN ('sync','quorum'))
 FROM qgr_recordings.state s WHERE s.singleton
$$;
REVOKE ALL ON FUNCTION qgr_recordings.audit_durability_ready() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION qgr_recordings.audit_durability_ready() TO qgr_history_admin;
CREATE TABLE qgr_recordings.holds (
 id text PRIMARY KEY CHECK(id ~ '^hold_[0-9a-f-]{36}$'),
 owner text NOT NULL REFERENCES qgr.users(id),
 revision numeric(20,0) NOT NULL CHECK(revision BETWEEN 1 AND 18446744073709551615),
 active boolean NOT NULL,
 captured_from timestamptz, captured_to timestamptz,
 case_ref text CHECK(length(btrim(case_ref)) BETWEEN 1 AND 256),
 reason text NOT NULL CHECK(length(btrim(reason)) BETWEEN 1 AND 512),
 actor text NOT NULL REFERENCES qgr.users(id),
 changed_at timestamptz NOT NULL DEFAULT clock_timestamp(),
 CHECK(captured_from IS NULL OR captured_to IS NULL OR captured_from<=captured_to)
);
ALTER TABLE qgr_recordings.holds ENABLE ROW LEVEL SECURITY;
ALTER TABLE qgr_recordings.holds FORCE ROW LEVEL SECURITY;
CREATE POLICY hold_internal ON qgr_recordings.holds TO CURRENT_USER USING(true) WITH CHECK(true);
CREATE INDEX holds_active_owner ON qgr_recordings.holds(owner) WHERE active;
ALTER TABLE qgr_recordings.held_cases ALTER COLUMN case_ref DROP NOT NULL;
ALTER TABLE qgr_recordings.held_gap_cases ALTER COLUMN case_ref DROP NOT NULL;
ALTER TABLE qgr_recordings.held_cases ADD FOREIGN KEY(hold) REFERENCES qgr_recordings.holds(id);
ALTER TABLE qgr_recordings.held_gap_cases ADD FOREIGN KEY(hold) REFERENCES qgr_recordings.holds(id);
ALTER TABLE qgr_recordings.held_content ADD COLUMN kind text NOT NULL DEFAULT 'content' CHECK(kind='content');
ALTER TABLE qgr_recordings.held_content ADD FOREIGN KEY(machine,session,kind,id) REFERENCES qgr_recordings.receipts(machine,session,kind,id);
ALTER TABLE qgr_recordings.held_gaps ADD COLUMN kind text NOT NULL DEFAULT 'gap' CHECK(kind='gap');
ALTER TABLE qgr_recordings.held_gaps ADD FOREIGN KEY(machine,session,kind,id) REFERENCES qgr_recordings.receipts(machine,session,kind,id);

CREATE FUNCTION qgr_recordings.hold_matches(h qgr_recordings.holds,row_owner text,first_at timestamptz,last_at timestamptz)
RETURNS boolean LANGUAGE sql IMMUTABLE SET search_path=pg_catalog AS $$
 SELECT h.active AND h.owner=row_owner AND (h.captured_from IS NULL OR h.captured_from<=last_at) AND (h.captured_to IS NULL OR h.captured_to>=first_at)
$$;
REVOKE ALL ON FUNCTION qgr_recordings.hold_matches(qgr_recordings.holds,text,timestamptz,timestamptz) FROM PUBLIC;

-- A changed/released hold stops authorizing reads immediately, including during bounded cleanup.
CREATE OR REPLACE FUNCTION qgr_recordings.held_case_allowed(machine_id text,session_id text,chunk_id text,case_reference text)
RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT EXISTS(SELECT 1 FROM qgr_recordings.held_cases c JOIN qgr_recordings.holds h ON h.id=c.hold
 JOIN qgr_recordings.held_content d ON d.machine=c.machine AND d.session=c.session AND d.id=c.id
 WHERE c.machine=machine_id AND c.session=session_id AND c.id=chunk_id AND h.case_ref=case_reference
 AND qgr_recordings.hold_matches(h,d.owner,d.captured_from,d.captured_to))
$$;
CREATE OR REPLACE FUNCTION qgr_recordings.held_gap_case_allowed(machine_id text,session_id text,gap_id text,case_reference text)
RETURNS boolean LANGUAGE sql STABLE SECURITY DEFINER SET search_path=pg_catalog AS $$
 SELECT EXISTS(SELECT 1 FROM qgr_recordings.held_gap_cases c JOIN qgr_recordings.holds h ON h.id=c.hold
 JOIN qgr_recordings.held_gaps d ON d.machine=c.machine AND d.session=c.session AND d.id=c.id
 WHERE c.machine=machine_id AND c.session=session_id AND c.id=gap_id AND h.case_ref=case_reference
 AND qgr_recordings.hold_matches(h,d.owner,d.captured_from,d.captured_to))
$$;

CREATE FUNCTION qgr_recordings.lifecycle_scope(actor_id text,token_expiry bigint)
RETURNS TABLE(recovery_epoch text,durability text,synchronous_ready boolean)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$ BEGIN
 PERFORM 1 FROM qgr.access_state FOR SHARE; -- administrator plane remains available during global disable
 IF token_expiry IS NULL OR token_expiry<=extract(epoch FROM clock_timestamp()) THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
 PERFORM 1 FROM qgr.users u WHERE u.id=actor_id AND NOT u.disabled AND u.directory_object IS NOT NULL AND u.directory_enabled IS DISTINCT FROM false FOR SHARE;
 IF NOT FOUND THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
 -- Never acquire user/machine/session locks after this lock: ingest holds them while waiting here.
 PERFORM pg_advisory_xact_lock(5856751401703146820);
 PERFORM set_config('qgr.transaction',pg_current_xact_id()::text,true),set_config('qgr.actor',actor_id,true),set_config('qgr.lifecycle','active',true);
 UPDATE qgr_recordings.commit_barrier SET touched_at=clock_timestamp() WHERE singleton;
 RETURN QUERY SELECT s.recovery_epoch,s.durability,
 current_setting('synchronous_standby_names')<>'' AND EXISTS(SELECT 1 FROM pg_stat_replication r WHERE r.state='streaming' AND r.sync_state IN ('sync','quorum'))
 FROM qgr_recordings.state s WHERE s.singleton AND s.lifecycle_ready AND NOT s.recovery_pending FOR SHARE OF s;
END $$;
REVOKE ALL ON FUNCTION qgr_recordings.lifecycle_scope(text,bigint) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION qgr_recordings.lifecycle_scope(text,bigint) TO qgr_record_lifecycle;

-- Nightly maintenance uses the isolated service credential, not a fabricated human/admin
-- token. It can only run the fixed retention policy. Human hold mutation still uses the
-- verified administrator path. The nil UUID is a reserved system audit identity.
CREATE FUNCTION qgr_recordings.maintenance_scope()
RETURNS TABLE(actor_id text,durability text,synchronous_ready boolean)
LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$ BEGIN
 PERFORM 1 FROM qgr.access_state FOR SHARE;
 PERFORM pg_advisory_xact_lock(5856751401703146820);
 PERFORM set_config('qgr.transaction',pg_current_xact_id()::text,true),set_config('qgr.actor','u_00000000-0000-0000-0000-000000000000',true),set_config('qgr.lifecycle','active',true);
 UPDATE qgr_recordings.commit_barrier SET touched_at=clock_timestamp() WHERE singleton;
 RETURN QUERY SELECT 'u_00000000-0000-0000-0000-000000000000'::text,s.durability,
 current_setting('synchronous_standby_names')<>'' AND EXISTS(SELECT 1 FROM pg_stat_replication r WHERE r.state='streaming' AND r.sync_state IN ('sync','quorum'))
 FROM qgr_recordings.state s WHERE s.singleton AND s.lifecycle_ready AND NOT s.recovery_pending FOR SHARE OF s;
END $$;
REVOKE ALL ON FUNCTION qgr_recordings.maintenance_scope() FROM PUBLIC;
GRANT EXECUTE ON FUNCTION qgr_recordings.maintenance_scope() TO qgr_record_lifecycle;

CREATE FUNCTION qgr_recordings.require_lifecycle() RETURNS void LANGUAGE plpgsql SET search_path=pg_catalog AS $$ BEGIN
 IF NOT coalesce(qgr_recordings.context_current(),false) OR current_setting('qgr.lifecycle',true) IS DISTINCT FROM 'active' OR nullif(current_setting('qgr.actor',true),'') IS NULL
 THEN RAISE EXCEPTION 'missing lifecycle context' USING ERRCODE='42501'; END IF;
 PERFORM pg_advisory_xact_lock(5856751401703146820);
END $$;
REVOKE ALL ON FUNCTION qgr_recordings.require_lifecycle() FROM PUBLIC;

CREATE FUNCTION qgr_recordings.change_hold(hold_id text,target_owner text,next_revision numeric,is_active boolean,first_at timestamptz,last_at timestamptz,case_reference text,change_reason text)
RETURNS boolean LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE old qgr_recordings.holds; BEGIN
 PERFORM qgr_recordings.require_lifecycle();
 IF current_setting('qgr.actor',true)='u_00000000-0000-0000-0000-000000000000' THEN RAISE EXCEPTION 'hold changes require an administrator' USING ERRCODE='42501'; END IF;
 IF coalesce(length(btrim(change_reason)),0) NOT BETWEEN 1 AND 512 OR coalesce(length(btrim(case_reference)),0)>256 OR (case_reference IS NOT NULL AND length(btrim(case_reference))=0) OR first_at>last_at OR next_revision IS NULL OR next_revision<1 OR next_revision>18446744073709551615 OR next_revision<>trunc(next_revision)
 THEN RAISE EXCEPTION 'invalid hold' USING ERRCODE='22023'; END IF;
 SELECT * INTO old FROM qgr_recordings.holds WHERE id=hold_id;
 IF FOUND THEN
  IF old.owner<>target_owner THEN RAISE EXCEPTION 'hold owner is immutable' USING ERRCODE='23505'; END IF;
  IF old.revision=next_revision AND old.active=is_active AND old.captured_from IS NOT DISTINCT FROM first_at AND old.captured_to IS NOT DISTINCT FROM last_at AND old.case_ref IS NOT DISTINCT FROM btrim(case_reference) AND old.reason=btrim(change_reason) THEN RETURN false; END IF;
  IF old.revision+1<>next_revision THEN RAISE EXCEPTION 'hold revision conflict' USING ERRCODE='23505'; END IF;
  UPDATE qgr_recordings.holds SET revision=next_revision,active=is_active,captured_from=first_at,captured_to=last_at,case_ref=btrim(case_reference),reason=btrim(change_reason),actor=current_setting('qgr.actor'),changed_at=clock_timestamp() WHERE id=hold_id;
 ELSE
  IF next_revision<>1 OR NOT is_active OR NOT EXISTS(SELECT 1 FROM qgr.users WHERE id=target_owner) THEN RAISE EXCEPTION 'invalid initial hold' USING ERRCODE='22023'; END IF;
  INSERT INTO qgr_recordings.holds(id,owner,revision,active,captured_from,captured_to,case_ref,reason,actor) VALUES(hold_id,target_owner,next_revision,is_active,first_at,last_at,btrim(case_reference),btrim(change_reason),current_setting('qgr.actor'));
 END IF;
 RETURN true;
END $$;
REVOKE ALL ON FUNCTION qgr_recordings.change_hold(text,text,numeric,boolean,timestamptz,timestamptz,text,text) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION qgr_recordings.change_hold(text,text,numeric,boolean,timestamptz,timestamptz,text,text) TO qgr_record_lifecycle;

-- Internal copy primitives also establish all overlapping coverage. A conflicting existing
-- held copy fails closed rather than letting the final preservation check drop its source.
CREATE FUNCTION qgr_recordings.preserve_content(machine_id text,session_id text,row_owner text,chunk_id text,first_at timestamptz,last_at timestamptz,bytes bytea,digest text)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$ BEGIN
 PERFORM pg_advisory_xact_lock(5856751401703146820);
 IF NOT EXISTS(SELECT 1 FROM qgr_recordings.holds h WHERE qgr_recordings.hold_matches(h,row_owner,first_at,last_at)) THEN RETURN; END IF;
 INSERT INTO qgr_recordings.held_content(machine,session,owner,id,captured_from,captured_to,payload,stored_digest) VALUES(machine_id,session_id,row_owner,chunk_id,first_at,last_at,bytes,digest) ON CONFLICT DO NOTHING;
 IF NOT EXISTS(SELECT 1 FROM qgr_recordings.held_content c WHERE c.machine=machine_id AND c.session=session_id AND c.owner=row_owner AND c.id=chunk_id AND c.captured_from=first_at AND c.captured_to=last_at AND c.payload=bytes AND c.stored_digest=digest) THEN RAISE EXCEPTION 'held content conflict' USING ERRCODE='23505'; END IF;
 INSERT INTO qgr_recordings.held_cases(machine,session,id,hold,case_ref) SELECT machine_id,session_id,chunk_id,h.id,h.case_ref FROM qgr_recordings.holds h WHERE qgr_recordings.hold_matches(h,row_owner,first_at,last_at) ON CONFLICT(machine,session,id,hold) DO UPDATE SET case_ref=excluded.case_ref;
END $$;
CREATE FUNCTION qgr_recordings.preserve_gap(machine_id text,session_id text,row_owner text,gap_id text,first_at timestamptz,last_at timestamptz,gap_body jsonb)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$ BEGIN
 PERFORM pg_advisory_xact_lock(5856751401703146820);
 IF NOT EXISTS(SELECT 1 FROM qgr_recordings.holds h WHERE qgr_recordings.hold_matches(h,row_owner,first_at,last_at)) THEN RETURN; END IF;
 INSERT INTO qgr_recordings.held_gaps(machine,session,owner,id,captured_from,captured_to,body) VALUES(machine_id,session_id,row_owner,gap_id,first_at,last_at,gap_body) ON CONFLICT DO NOTHING;
 IF NOT EXISTS(SELECT 1 FROM qgr_recordings.held_gaps c WHERE c.machine=machine_id AND c.session=session_id AND c.owner=row_owner AND c.id=gap_id AND c.captured_from=first_at AND c.captured_to=last_at AND c.body=gap_body) THEN RAISE EXCEPTION 'held gap conflict' USING ERRCODE='23505'; END IF;
 INSERT INTO qgr_recordings.held_gap_cases(machine,session,id,hold,case_ref) SELECT machine_id,session_id,gap_id,h.id,h.case_ref FROM qgr_recordings.holds h WHERE qgr_recordings.hold_matches(h,row_owner,first_at,last_at) ON CONFLICT(machine,session,id,hold) DO UPDATE SET case_ref=excluded.case_ref;
END $$;
REVOKE ALL ON FUNCTION qgr_recordings.preserve_content(text,text,text,text,timestamptz,timestamptz,bytea,text),qgr_recordings.preserve_gap(text,text,text,text,timestamptz,timestamptz,jsonb) FROM PUBLIC;

-- The ingest role cannot select held rows or set coverage. This wrapper checks its original
-- transaction scope/receipt before invoking the internal primitive. No hold details are returned.
CREATE FUNCTION qgr_recordings.preserve_upload_content(machine_id text,session_id text,row_owner text,chunk_id text,first_at timestamptz,last_at timestamptz,bytes bytea,digest text)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$ BEGIN
 IF NOT coalesce(qgr_recordings.ingest_context(machine_id,row_owner),false) OR NOT EXISTS(SELECT 1 FROM qgr_recordings.receipts r WHERE r.machine=machine_id AND r.session=session_id AND r.owner=row_owner AND r.kind='content' AND r.id=chunk_id AND r.captured_from=first_at AND r.captured_to=last_at AND r.immutable->'stored'->>'digest'=digest) THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
 PERFORM qgr_recordings.preserve_content(machine_id,session_id,row_owner,chunk_id,first_at,last_at,bytes,digest);
END $$;
CREATE FUNCTION qgr_recordings.preserve_upload_gap(machine_id text,session_id text,row_owner text,gap_id text,first_at timestamptz,last_at timestamptz,gap_body jsonb)
RETURNS void LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$ BEGIN
 IF NOT coalesce(qgr_recordings.ingest_context(machine_id,row_owner),false) OR NOT EXISTS(SELECT 1 FROM qgr_recordings.receipts r WHERE r.machine=machine_id AND r.session=session_id AND r.owner=row_owner AND r.kind='gap' AND r.id=gap_id AND r.captured_from=first_at AND r.captured_to=last_at AND r.immutable=gap_body) THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
 PERFORM qgr_recordings.preserve_gap(machine_id,session_id,row_owner,gap_id,first_at,last_at,gap_body);
END $$;
REVOKE ALL ON FUNCTION qgr_recordings.preserve_upload_content(text,text,text,text,timestamptz,timestamptz,bytea,text),qgr_recordings.preserve_upload_gap(text,text,text,text,timestamptz,timestamptz,jsonb) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION qgr_recordings.preserve_upload_content(text,text,text,text,timestamptz,timestamptz,bytea,text),qgr_recordings.preserve_upload_gap(text,text,text,text,timestamptz,timestamptz,jsonb) TO qgr_record_ingest;

CREATE FUNCTION qgr_recordings.lifecycle_step(batch_size integer)
RETURNS TABLE(stage text,content_month date,affected integer) LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE r record; copied integer:=0; cutoff timestamptz; target qgr_recordings.partitions; BEGIN
 PERFORM qgr_recordings.require_lifecycle();
 IF batch_size IS NULL OR batch_size NOT BETWEEN 1 AND 32 THEN RAISE EXCEPTION 'invalid batch size' USING ERRCODE='22023'; END IF;
 SELECT clock_timestamp()-s.retention_days*interval '1 day' INTO STRICT cutoff FROM qgr_recordings.state s WHERE s.singleton AND s.lifecycle_ready AND NOT s.recovery_pending;
 -- Include already held rows: newly overlapping holds must acquire their own coverage even
 -- after the ordinary partition has gone. One copy covers every currently matching hold.
 FOR r IN SELECT d.* FROM (SELECT machine,session,owner,id,captured_from,captured_to,payload,stored_digest FROM qgr_recordings.content UNION ALL SELECT machine,session,owner,id,captured_from,captured_to,payload,stored_digest FROM qgr_recordings.held_content) d
 WHERE EXISTS(SELECT 1 FROM qgr_recordings.holds h WHERE qgr_recordings.hold_matches(h,d.owner,d.captured_from,d.captured_to) AND NOT EXISTS(SELECT 1 FROM qgr_recordings.held_cases c WHERE c.machine=d.machine AND c.session=d.session AND c.id=d.id AND c.hold=h.id))
 LIMIT batch_size LOOP
  PERFORM qgr_recordings.preserve_content(r.machine,r.session,r.owner,r.id,r.captured_from,r.captured_to,r.payload,r.stored_digest); copied:=copied+1;
 END LOOP;
 IF copied>0 THEN RETURN QUERY SELECT 'preserved_content'::text,NULL::date,copied; RETURN; END IF;
 FOR r IN SELECT d.* FROM (SELECT machine,session,owner,id,captured_from,captured_to,body FROM qgr_recordings.gaps UNION ALL SELECT machine,session,owner,id,captured_from,captured_to,body FROM qgr_recordings.held_gaps) d
 WHERE EXISTS(SELECT 1 FROM qgr_recordings.holds h WHERE qgr_recordings.hold_matches(h,d.owner,d.captured_from,d.captured_to) AND NOT EXISTS(SELECT 1 FROM qgr_recordings.held_gap_cases c WHERE c.machine=d.machine AND c.session=d.session AND c.id=d.id AND c.hold=h.id))
 LIMIT batch_size LOOP
  PERFORM qgr_recordings.preserve_gap(r.machine,r.session,r.owner,r.id,r.captured_from,r.captured_to,r.body); copied:=copied+1;
 END LOOP;
 IF copied>0 THEN RETURN QUERY SELECT 'preserved_gaps'::text,NULL::date,copied; RETURN; END IF;
 -- Remove obsolete membership in bounded batches. Read authorization already joins the live
 -- hold and its current window, so pending cleanup cannot keep released access alive.
 WITH obsolete AS (SELECT c.ctid FROM qgr_recordings.held_cases c JOIN qgr_recordings.holds h ON h.id=c.hold JOIN qgr_recordings.held_content d ON d.machine=c.machine AND d.session=c.session AND d.id=c.id WHERE NOT qgr_recordings.hold_matches(h,d.owner,d.captured_from,d.captured_to) LIMIT batch_size)
 DELETE FROM qgr_recordings.held_cases c USING obsolete o WHERE c.ctid=o.ctid;
 GET DIAGNOSTICS copied=ROW_COUNT;
 IF copied>0 THEN RETURN QUERY SELECT 'released_content_coverage'::text,NULL::date,copied; RETURN; END IF;
 WITH obsolete AS (SELECT c.ctid FROM qgr_recordings.held_gap_cases c JOIN qgr_recordings.holds h ON h.id=c.hold JOIN qgr_recordings.held_gaps d ON d.machine=c.machine AND d.session=c.session AND d.id=c.id WHERE NOT qgr_recordings.hold_matches(h,d.owner,d.captured_from,d.captured_to) LIMIT batch_size)
 DELETE FROM qgr_recordings.held_gap_cases c USING obsolete o WHERE c.ctid=o.ctid;
 GET DIAGNOSTICS copied=ROW_COUNT;
 IF copied>0 THEN RETURN QUERY SELECT 'released_gap_coverage'::text,NULL::date,copied; RETURN; END IF;
 SELECT p.* INTO target FROM qgr_recordings.partitions p WHERE p.state<>'retired' AND ((p.month+interval '1 month')::date::timestamp AT TIME ZONE 'UTC')<=cutoff ORDER BY p.month LIMIT 1;
 IF FOUND THEN
  IF target.state='active' THEN
   UPDATE qgr_recordings.partitions SET state='retiring' WHERE month=target.month;
   RETURN QUERY SELECT 'retiring'::text,target.month,0; RETURN;
  END IF;
  -- Final atomic check, under the same lock as hold changes. Check exact bytes and every
  -- currently active overlapping membership. A committed new hold sends work back to copy.
  IF EXISTS(SELECT 1 FROM qgr_recordings.content d JOIN qgr_recordings.holds h ON qgr_recordings.hold_matches(h,d.owner,d.captured_from,d.captured_to)
   WHERE d.captured_from>=target.month::timestamp AT TIME ZONE 'UTC' AND d.captured_from<(target.month+interval '1 month')::timestamp AT TIME ZONE 'UTC'
   AND NOT EXISTS(SELECT 1 FROM qgr_recordings.held_content x JOIN qgr_recordings.held_cases c USING(machine,session,id) WHERE x.machine=d.machine AND x.session=d.session AND x.id=d.id AND x.owner=d.owner AND x.captured_from=d.captured_from AND x.captured_to=d.captured_to AND x.payload=d.payload AND x.stored_digest=d.stored_digest AND c.hold=h.id))
  THEN RAISE EXCEPTION 'preservation incomplete' USING ERRCODE='23514'; END IF;
  EXECUTE format('DROP TABLE qgr_recordings.%I',target.name);
  UPDATE qgr_recordings.partitions SET state='retired' WHERE month=target.month;
  RETURN QUERY SELECT 'retired'::text,target.month,1; RETURN;
 END IF;
 -- Gaps spanning months expire by the end month, never the start month.
 FOR r IN SELECT g.* FROM qgr_recordings.gaps g WHERE ((date_trunc('month',g.captured_to AT TIME ZONE 'UTC')+interval '1 month') AT TIME ZONE 'UTC')<=cutoff LIMIT batch_size LOOP
  PERFORM qgr_recordings.preserve_gap(r.machine,r.session,r.owner,r.id,r.captured_from,r.captured_to,r.body);
  DELETE FROM qgr_recordings.gaps WHERE machine=r.machine AND session=r.session AND id=r.id; copied:=copied+1;
 END LOOP;
 IF copied>0 THEN RETURN QUERY SELECT 'expired_gaps'::text,NULL::date,copied; RETURN; END IF;
 WITH expired AS (SELECT d.machine,d.session,d.id FROM qgr_recordings.held_content d WHERE ((date_trunc('month',d.captured_to AT TIME ZONE 'UTC')+interval '1 month') AT TIME ZONE 'UTC')<=cutoff AND NOT EXISTS(SELECT 1 FROM qgr_recordings.held_cases c WHERE c.machine=d.machine AND c.session=d.session AND c.id=d.id) AND NOT EXISTS(SELECT 1 FROM qgr_recordings.holds h WHERE qgr_recordings.hold_matches(h,d.owner,d.captured_from,d.captured_to)) LIMIT batch_size)
 DELETE FROM qgr_recordings.held_content d USING expired e WHERE d.machine=e.machine AND d.session=e.session AND d.id=e.id;
 GET DIAGNOSTICS copied=ROW_COUNT;
 IF copied>0 THEN RETURN QUERY SELECT 'expired_held_content'::text,NULL::date,copied; RETURN; END IF;
 WITH expired AS (SELECT d.machine,d.session,d.id FROM qgr_recordings.held_gaps d WHERE ((date_trunc('month',d.captured_to AT TIME ZONE 'UTC')+interval '1 month') AT TIME ZONE 'UTC')<=cutoff AND NOT EXISTS(SELECT 1 FROM qgr_recordings.held_gap_cases c WHERE c.machine=d.machine AND c.session=d.session AND c.id=d.id) AND NOT EXISTS(SELECT 1 FROM qgr_recordings.holds h WHERE qgr_recordings.hold_matches(h,d.owner,d.captured_from,d.captured_to)) LIMIT batch_size)
 DELETE FROM qgr_recordings.held_gaps d USING expired e WHERE d.machine=e.machine AND d.session=e.session AND d.id=e.id;
 GET DIAGNOSTICS copied=ROW_COUNT;
 RETURN QUERY SELECT CASE WHEN copied>0 THEN 'expired_held_gaps' ELSE 'idle' END,NULL::date,copied;
END $$;
REVOKE ALL ON FUNCTION qgr_recordings.lifecycle_step(integer) FROM PUBLIC;
GRANT EXECUTE ON FUNCTION qgr_recordings.lifecycle_step(integer) TO qgr_record_lifecycle;

-- Case authorization follows the current hold, including case/window changes and release.
CREATE OR REPLACE FUNCTION qgr_recordings.admin_read_scope(actor_id text,session_id text,token_expiry bigint,read_reason text,case_reference text,include_held boolean)
RETURNS TABLE(machine_id text,owner_id text,recovery_epoch text) LANGUAGE plpgsql SECURITY DEFINER SET search_path=pg_catalog AS $$
DECLARE target_machine text; target_owner text;
BEGIN
 PERFORM 1 FROM qgr.access_state FOR SHARE; -- administrator plane remains available during global disable
 IF token_expiry IS NULL OR token_expiry<=extract(epoch FROM clock_timestamp()) OR coalesce(length(btrim(read_reason)),0) NOT BETWEEN 1 AND 512 OR (include_held AND coalesce(length(btrim(case_reference)),0)=0) OR coalesce(length(case_reference),0)>256 THEN RAISE EXCEPTION 'invalid read context' USING ERRCODE='22023'; END IF;
 PERFORM 1 FROM qgr.users u WHERE u.id=actor_id AND NOT u.disabled AND u.directory_object IS NOT NULL AND u.directory_enabled IS DISTINCT FROM false FOR SHARE;
 IF NOT FOUND THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
 SELECT s.machine,s.owner INTO target_machine,target_owner FROM qgr.sessions s WHERE s.id=session_id FOR SHARE;
 IF NOT FOUND THEN RAISE EXCEPTION 'access denied' USING ERRCODE='42501'; END IF;
 IF include_held THEN PERFORM pg_advisory_xact_lock(5856751401703146820); END IF;
 IF include_held AND NOT EXISTS(SELECT 1 FROM qgr_recordings.held_cases c WHERE c.machine=target_machine AND c.session=session_id AND qgr_recordings.held_case_allowed(c.machine,c.session,c.id,btrim(case_reference))) AND NOT EXISTS(SELECT 1 FROM qgr_recordings.held_gap_cases c WHERE c.machine=target_machine AND c.session=session_id AND qgr_recordings.held_gap_case_allowed(c.machine,c.session,c.id,btrim(case_reference))) THEN RAISE EXCEPTION 'case does not cover requested content' USING ERRCODE='42501'; END IF;
 PERFORM set_config('qgr.transaction',pg_current_xact_id()::text,true),set_config('qgr.owner','',true),set_config('qgr.actor',actor_id,true),set_config('qgr.reason',btrim(read_reason),true),set_config('qgr.case',coalesce(btrim(case_reference),''),true);
 RETURN QUERY SELECT target_machine,target_owner,c.recovery_epoch FROM qgr_recordings.state c WHERE c.singleton AND c.lifecycle_ready AND NOT c.recovery_pending FOR SHARE OF c;
END $$;
