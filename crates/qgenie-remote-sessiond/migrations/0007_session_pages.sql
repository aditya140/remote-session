BEGIN IMMEDIATE;
ALTER TABLE sessions ADD COLUMN recording_warning TEXT;
CREATE INDEX sessions_scope_page ON sessions(remote_scope,id);
CREATE INDEX sessions_active_page ON sessions(id) WHERE state IN ('"accepted"','"launching"','"running"','"idle"');
CREATE INDEX sessions_scope_active_page ON sessions(remote_scope,id) WHERE state IN ('"accepted"','"launching"','"running"','"idle"');
CREATE INDEX sessions_native_history ON sessions(remote_scope,json_extract(agent,'$.agent'),json_extract(agent,'$.native_thread')) WHERE agent IS NOT NULL;
CREATE INDEX sessions_native_uncertain ON sessions(coalesce(json_extract(agent,'$.native_home'),id)) WHERE agent IS NOT NULL AND state NOT IN ('"exited"','"failed"');
PRAGMA user_version=7;
COMMIT;
