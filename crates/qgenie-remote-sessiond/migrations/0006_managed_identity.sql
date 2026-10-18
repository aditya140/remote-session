-- Per-native-instance correlation survives owner restart without adopting its process.
ALTER TABLE sessions ADD COLUMN agent TEXT CHECK(agent IS NULL OR (json_valid(agent) AND length(CAST(agent AS BLOB))<=4096));
PRAGMA user_version=6;
