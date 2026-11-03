#!/bin/sh
# Harness DinD untuk bukti V3 (ADR-0001 Addendum A §5 pengecualian 1, §8).
# Menjalankan Docker Engine di container `docker:<versi>-dind` SEMENTARA yang diberi
# `--privileged` (satu-satunya tempat privileged dalam repo), lalu menaikkan stack
# Compose yang sama dengan V2 lewat klien Docker Compose v2 di container CLI.
# Semua resource diberi label shaal.issue dan dibersihkan di akhir run, termasuk saat gagal.
#
# Pemakaian (dari akar repo):
#   SHAA_ISSUE=SHAA-207 ENV_FILE=/path/env.env [PROFILES=bundled-infra] sh deploy/compose/test/dind.sh
#
# Exit code: 0 hanya jika stack sehat dan smoke test lolos dan pembersihan selesai.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd -P)
REPO=$(cd "$HERE/../../.." && pwd -P)
# shellcheck disable=SC1091
. "$HERE/images.env"

: "${SHAA_ISSUE:?isi SHAA_ISSUE, mis. SHAA-207}"
: "${ENV_FILE:?isi ENV_FILE ke file env di luar repo}"
PROFILES=${PROFILES:-bundled-infra}
case "$SHAA_ISSUE" in SHAA-[0-9]*) ;; *) echo "SHAA_ISSUE tidak valid: $SHAA_ISSUE" >&2; exit 2 ;; esac

ENV_DIR=$(cd "$(dirname "$ENV_FILE")" && pwd -P)
ENV_FILE_ABS="$ENV_DIR/$(basename "$ENV_FILE")"
SECRETS_DIR=$(sed -n 's/^POOLROUTER_SECRETS_DIR=//p' "$ENV_FILE_ABS" | head -n 1)
[ -d "$SECRETS_DIR" ] || { echo "POOLROUTER_SECRETS_DIR tidak ada: $SECRETS_DIR" >&2; exit 2; }

ISSUE_LC=$(echo "$SHAA_ISSUE" | tr 'A-Z' 'a-z')
NAME="poolrouter-dind-$ISSUE_LC"
PROJECT="poolrouter-dind-$ISSUE_LC"
GEN_DIR="$HERE/generated"
LOG_DIR="$HERE/logs"
OVERLAY="$GEN_DIR/labels-$SHAA_ISSUE.yaml"
mkdir -p "$GEN_DIR" "$LOG_DIR"
LOG="$LOG_DIR/dind-$ISSUE_LC.log"
: > "$LOG"

log() { echo "$*" | tee -a "$LOG"; }

# Pembersihan: resource berlabel milik issue ini saja (shaal-playbook §6).
cleanup() {
  rc=$?
  podman rm -f -v --filter "label=shaal.issue=$SHAA_ISSUE" >> "$LOG" 2>&1 || true
  podman volume prune -f --filter "label=shaal.issue=$SHAA_ISSUE" >> "$LOG" 2>&1 || true
  podman network prune -f --filter "label=shaal.issue=$SHAA_ISSUE" >> "$LOG" 2>&1 || true
  rm -f -- "$OVERLAY"
  left=$(podman ps -a --filter "label=shaal.issue=$SHAA_ISSUE" -q | wc -l | tr -d ' ')
  log "CLEANUP left_containers=$left"
  [ "$left" = 0 ] || rc=1
  exit "$rc"
}
trap cleanup EXIT

# Overlay label untuk resource di dalam DinD (services, volumes, network).
cat > "$OVERLAY" <<YAML
services:
  postgres: {labels: {shaal.issue: $SHAA_ISSUE}}
  redis: {labels: {shaal.issue: $SHAA_ISSUE}}
  rabbitmq: {labels: {shaal.issue: $SHAA_ISSUE}}
volumes:
  pg_data: {labels: {shaal.issue: $SHAA_ISSUE}}
  redis_data: {labels: {shaal.issue: $SHAA_ISSUE}}
  rabbitmq_data: {labels: {shaal.issue: $SHAA_ISSUE}}
networks:
  default: {labels: {shaal.issue: $SHAA_ISSUE}}
YAML

log "== DinD: start daemon $NAME ($DIND_IMAGE)"
podman rm -f -v "$NAME" >/dev/null 2>&1 || true
podman run -d --name "$NAME" --privileged \
  --label "shaal.issue=$SHAA_ISSUE" \
  -e DOCKER_TLS_CERTDIR= \
  -v "$SECRETS_DIR:$SECRETS_DIR:ro" \
  -v "$ENV_DIR:$ENV_DIR:ro" \
  "$DIND_IMAGE" >> "$LOG" 2>&1
log "run_daemon_exit=$?"

# Tunggu daemon siap (maks 90 detik).
ready=0; i=0
while [ "$i" -lt 90 ]; do
  if podman exec "$NAME" docker info >/dev/null 2>&1; then ready=1; break; fi
  i=$((i + 1)); sleep 1
done
[ "$ready" = 1 ] || { log "DAEMON_NOT_READY (lihat log dan catat kegagalan DinD privileged)"; podman logs "$NAME" >> "$LOG" 2>&1 || true; exit 1; }
log "daemon_ready_after=${i}s"

# Klien CLI berbagi netns dengan daemon: daemon ada di tcp://127.0.0.1:2375.
cli() {
  podman run --rm --network "container:$NAME" --security-opt label=disable \
    -e DOCKER_HOST=tcp://127.0.0.1:2375 \
    -v "$REPO:$REPO:ro" -w "$REPO" \
    -v "$SECRETS_DIR:$SECRETS_DIR:ro" -v "$ENV_DIR:$ENV_DIR:ro" \
    "$CLI_IMAGE" "$@"
}
COMPOSE_ARGS="-p $PROJECT -f deploy/compose/compose.yaml -f deploy/compose/compose.staging.yaml -f $OVERLAY --env-file $ENV_FILE_ABS"
PROFILE_ARGS=""
for p in $(echo "$PROFILES" | tr ',' ' '); do PROFILE_ARGS="$PROFILE_ARGS --profile $p"; done

log "== V3: docker version (daemon) =="
cli version --format 'client={{.Client.Version}} server={{.Server.Version}} os={{.Server.Os}}/{{.Server.Arch}}' >> "$LOG" 2>&1
log "== V3: compose up --wait ($PROFILES) =="
# shellcheck disable=SC2086
if cli compose $COMPOSE_ARGS $PROFILE_ARGS up -d --wait >> "$LOG" 2>&1; then
  log "compose_up_exit=0"
else
  log "compose_up_exit=$?"; cli compose $COMPOSE_ARGS $PROFILE_ARGS ps >> "$LOG" 2>&1 || true; exit 1
fi

log "== V3: smoke =="
cli docker ps --filter "label=shaal.issue=$SHAA_ISSUE" --format '{{.Names}} {{.Status}}' >> "$LOG" 2>&1
pg=$(cli docker ps --filter "label=shaal.issue=$SHAA_ISSUE" --filter "label=com.docker.compose.service=postgres" -q)
if [ -n "$pg" ]; then
  cli docker exec "$pg" sh -c 'psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" -tAc "select 1"' >> "$LOG" 2>&1
  log "smoke_postgres_exit=$?"
fi
healthy=$(cli docker ps --filter "label=shaal.issue=$SHAA_ISSUE" --filter "health=healthy" -q | wc -l | tr -d ' ')
log "healthy_containers=$healthy"

log "== V3: bersihkan stack di dalam DinD =="
# shellcheck disable=SC2086
cli compose $COMPOSE_ARGS $PROFILE_ARGS down -v >> "$LOG" 2>&1 || log "compose_down_exit=$?"

# Hitung ulang untuk bukti: compose file sha256 dan image digest (V5).
[ "$healthy" -ge 1 ] || { log "SMOKE_GAGAL: tidak ada container healthy"; exit 1; }
log "V3_OK"
