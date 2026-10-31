#!/usr/bin/env bash
set -euo pipefail
# No host directories or credentials are mounted. Only a compiled test executable
# is copied into this disposable, unprivileged-by-default Docker container.
repo_root=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd)
cd -- "$repo_root"
fixture_binary=$(cargo test -p qgenie-remote-transport --lib --no-run --locked --message-format=json | python3 -c 'import sys,json; items=[json.loads(line) for line in sys.stdin]; print(next(x["executable"] for x in items if x.get("reason")=="compiler-artifact" and x.get("executable") and x["target"]["name"]=="qgenie_remote_transport"))')
fixture_container="qgr-ipc-proof-$$"
docker_args=(--host=unix:///var/run/docker.sock)
unset DOCKER_HOST DOCKER_CONTEXT DOCKER_TLS DOCKER_TLS_VERIFY DOCKER_CERT_PATH
docker "${docker_args[@]}" info >/dev/null
docker "${docker_args[@]}" create --name "$fixture_container" --network none --tmpfs /tmp:rw,nosuid,nodev,size=64m debian:trixie-slim sleep infinity >/dev/null
trap 'docker "${docker_args[@]}" rm --force "$fixture_container" >/dev/null' EXIT
docker "${docker_args[@]}" cp "$fixture_binary" "$fixture_container:/transport-tests"
docker "${docker_args[@]}" start "$fixture_container" >/dev/null
docker "${docker_args[@]}" exec "$fixture_container" chmod 0755 /transport-tests
docker "${docker_args[@]}" exec -e QGR_ALLOW_ROOT_IPC_PROOF=yes "$fixture_container" /transport-tests --exact ipc::tests::actual_foreign_uid_is_denied_by_filesystem_and_both_peer_checks --ignored --nocapture
