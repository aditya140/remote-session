-- Preserve the original native nonce on cross-node dispatch. This is not a
-- freshness clock: sessiond validates the nonce against its original issue time.
ALTER TABLE qgr.machine_routes ADD COLUMN native_access jsonb
    CHECK(native_access IS NULL OR octet_length(native_access::text)<=1024);
