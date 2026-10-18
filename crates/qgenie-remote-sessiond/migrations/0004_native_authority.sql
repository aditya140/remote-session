BEGIN IMMEDIATE;
ALTER TABLE remote_bindings ADD COLUMN host_floor BLOB NOT NULL DEFAULT X'0000000000000000' CHECK(length(host_floor)=8);
ALTER TABLE remote_bindings ADD COLUMN connection TEXT;
PRAGMA user_version=4;
COMMIT;
