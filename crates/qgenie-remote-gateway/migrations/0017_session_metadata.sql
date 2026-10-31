-- Gateway-owned metadata is independent of native session projections.
ALTER TABLE qgr.sessions ADD COLUMN metadata jsonb NOT NULL DEFAULT '{"name":null,"labels":{}}'
CHECK(jsonb_typeof(metadata)='object' AND octet_length(metadata::text)<=8192);
CREATE INDEX sessions_owner_name ON qgr.sessions(owner,machine,(metadata->>'name'))
WHERE metadata->>'name' IS NOT NULL;
