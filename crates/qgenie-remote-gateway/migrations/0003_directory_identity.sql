-- Entra's oid is stable across client applications; JWT sub can be pairwise.
-- Legacy preview identities stay unmapped until a verified old subject supplies oid.
ALTER TABLE qgr.users ADD COLUMN directory_object uuid;
ALTER TABLE qgr.users ADD CONSTRAINT users_directory_identity UNIQUE(issuer,tenant,directory_object);
