#!/usr/bin/env bash
set -euo pipefail
# Disposable synthetic proof. No host paths/credentials are mounted and no network is attached.
repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
cd -- "$repo_root"
fixture_binary=$(cargo test -p qgenie-remote-sessiond --lib --no-run --locked --message-format=json | python3 -c 'import sys,json; items=[json.loads(line) for line in sys.stdin]; print(next(x["executable"] for x in items if x.get("reason")=="compiler-artifact" and x.get("executable") and x["target"]["name"]=="qgenie_remote_sessiond"))')
fixture_container="qgr-recording-proof-$$"
docker_args=(--host=unix:///var/run/docker.sock)
unset DOCKER_HOST DOCKER_CONTEXT DOCKER_TLS DOCKER_TLS_VERIFY DOCKER_CERT_PATH
docker "${docker_args[@]}" info >/dev/null
docker "${docker_args[@]}" create --name "$fixture_container" --user 1000:1000 --network none --tmpfs /tmp:rw,nosuid,nodev,size=64m --tmpfs /recording:rw,nosuid,nodev,size=4m,mode=0700,uid=1000,gid=1000 debian:trixie-slim sleep infinity >/dev/null
trap 'docker "${docker_args[@]}" rm --force "$fixture_container" >/dev/null' EXIT
docker "${docker_args[@]}" cp "$fixture_binary" "$fixture_container:/sessiond-tests"
docker "${docker_args[@]}" start "$fixture_container" >/dev/null
docker "${docker_args[@]}" exec --user 0 "$fixture_container" chmod 0755 /sessiond-tests
docker "${docker_args[@]}" exec -e QGR_RECORDING_DISK_FULL_DIR=/recording "$fixture_container" /sessiond-tests --exact recording::spool::tests::actual_enospc_recovers_with_durable_uncertainty_and_live_pty_control --ignored --nocapture
