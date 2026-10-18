BEGIN IMMEDIATE;
ALTER TABLE sessions ADD COLUMN remote_scope TEXT CHECK(remote_scope IS NULL OR length(remote_scope)<=4096);
CREATE TABLE remote_bindings (
    origin TEXT NOT NULL CHECK(length(origin)<=2048),
    machine TEXT NOT NULL,
    owner TEXT NOT NULL,
    PRIMARY KEY(origin,machine)
);
PRAGMA user_version=3;
COMMIT;
