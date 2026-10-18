-- Upstream state is a projection, separate from permanent manual disablement.
-- Process-local suspend-aware evidence, never this wall timestamp, grants access.
ALTER TABLE qgr.users ADD COLUMN directory_enabled boolean;
ALTER TABLE qgr.users ADD COLUMN directory_verified_at timestamptz;
