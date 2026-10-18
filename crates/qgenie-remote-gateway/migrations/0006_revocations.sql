CREATE TABLE qgr.access_revocations (
    actor text NOT NULL REFERENCES qgr.users(id),
    request text NOT NULL CHECK(length(request)<=64),
    target jsonb NOT NULL CHECK(octet_length(target::text)<=256),
    revision numeric(20,0) NOT NULL CHECK(revision BETWEEN 1 AND 18446744073709551615),
    committed_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    PRIMARY KEY(actor,request)
);
GRANT SELECT,INSERT ON qgr.access_revocations TO qgr_app;
