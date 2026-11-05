#!/bin/sh
# Harness DinD untuk bukti V3 (ADR-0001 Addendum A §5 pengecualian 1, §8).
# Menjalankan Docker Engine di container `docker:<versi>-dind` SEMENTARA yang diberi
# `--privileged` (satu-satunya tempat privileged dalam repo), lalu menaikkan stack
# Compose yang sama dengan V2 lewat klien Docker Compose v2 di container CLI.
# Semua resource diberi label arctiva.issue dan dibersihkan di akhir run, termasuk saat gagal.
#
# Pemakaian (dari akar repo):
#   ARC_ISSUE=ARC-2 ENV_FILE=/path/env.env [PROFILES=bundled-infra] sh deploy/compose/test/dind.sh
#
# Exit code: 0 hanya jika stack sehat dan smoke test lolos dan pembersihan selesai.
set -eu

HERE=$(cd "$(dirname "$0")" && pwd -P)
REPO=$(cd "$HERE/../../.." && pwd -P)
# shellcheck disable=SC1091
. "$HERE/images.env"

: "${ARC_ISSUE:?isi ARC_ISSUE, mis. ARC-2}"
: "${ENV_FILE:?isi ENV_FILE ke file env di luar repo}"
PROFILES=${PROFILES:-bundled-infra}
case "$ARC_ISSUE" in ARC-[0-9]*) ;; *) echo "ARC_ISSUE tidak valid: $ARC_ISSUE" >&2; exit 2 ;; esac

ENV_DIR=$(cd "$(dirname "$ENV_FILE")" && pwd -P)
ENV_FILE_ABS="$ENV_DIR/$(basename "$ENV_FILE")"
SECRETS_DIR=$(sed -n 's/^POOLROUTER_SECRETS_DIR=//p' "$ENV_FILE_ABS" | head -n 1)
[ -d "$SECRETS_DIR" ] || { echo "POOLROUTER_SECRETS_DIR tidak ada: $SECRETS_DIR" >&2; exit 2; }

ISSUE_LC=$(echo "$ARC_ISSUE" | tr 'A-Z' 'a-z')
NAME="poolrouter-dind-$ISSUE_LC"
PROJECT="poolrouter-dind-$ISSUE_LC"
GEN_DIR="$HERE/generated"
LOG_DIR="$HERE/logs"
OVERLAY="$GEN_DIR/labels-$ARC_ISSUE.yaml"
mkdir -p "$GEN_DIR" "$LOG_DIR"
LOG="$LOG_DIR/dind-$ISSUE_LC.log"
: > "$LOG"

log() { echo "$*" | tee -a "$LOG"; }

# Pembersihan: resource berlabel milik issue ini saja (arctiva-playbook §6).
cleanup() {
  rc=$?
  if [ "$rc" != 0 ]; then
    # Diagnostik sebelum resource dihapus: status dan log daemon DinD (tanpa nilai secret).
    {
      echo "== DIAG (rc=$rc): resource berlabel"
      podman ps -a --filter "label=arctiva.issue=$ARC_ISSUE" 2>&1 || true
      echo "== DIAG: podman logs $NAME (akhir)"
      podman logs --tail 200 "$NAME" 2>&1 || true
    } >> "$LOG" 2>&1
  fi
  podman rm -f -v --filter "label=arctiva.issue=$ARC_ISSUE" >> "$LOG" 2>&1 || true
  podman volume prune -f --filter "label=arctiva.issue=$ARC_ISSUE" >> "$LOG" 2>&1 || true
  podman network prune -f --filter "label=arctiva.issue=$ARC_ISSUE" >> "$LOG" 2>&1 || true
  rm -f -- "$OVERLAY"
  left=$(podman ps -a --filter "label=arctiva.issue=$ARC_ISSUE" -q | wc -l | tr -d ' ')
  lv=$(podman volume ls --filter "label=arctiva.issue=$ARC_ISSUE" -q | wc -l | tr -d ' ')
  ln=$(podman network ls --filter "label=arctiva.issue=$ARC_ISSUE" -q | wc -l | tr -d ' ')
  log "CLEANUP left_containers=$left left_volumes=$lv left_networks=$ln"
  [ "$left" = 0 ] && [ "$lv" = 0 ] && [ "$ln" = 0 ] || rc=1
  exit "$rc"
}
trap cleanup EXIT

# Overlay label untuk resource di dalam DinD (services, volumes, network).
cat > "$OVERLAY" <<YAML
services:
  postgres: {labels: {arctiva.issue: $ARC_ISSUE}}
  redis: {labels: {arctiva.issue: $ARC_ISSUE}}
  rabbitmq: {labels: {arctiva.issue: $ARC_ISSUE}}
volumes:
  pg_data: {labels: {arctiva.issue: $ARC_ISSUE}}
  redis_data: {labels: {arctiva.issue: $ARC_ISSUE}}
  rabbitmq_data: {labels: {arctiva.issue: $ARC_ISSUE}}
networks:
  default: {labels: {arctiva.issue: $ARC_ISSUE}}
YAML

log "== DinD: start daemon $NAME ($DIND_IMAGE)"
podman rm -f -v "$NAME" >/dev/null 2>&1 || true
if podman run -d --name "$NAME" --privileged \
  --label "arctiva.issue=$ARC_ISSUE" \
  -e DOCKER_TLS_CERTDIR= \
  -v "$SECRETS_DIR:$SECRETS_DIR:ro" \
  -v "$ENV_DIR:$ENV_DIR:ro" \
  "$DIND_IMAGE" >> "$LOG" 2>&1; then
  log "run_daemon_exit=0"
else
  log "run_daemon_exit=$?"; exit 1
fi

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

# step NAMA PERINTAH...: catat perintah, exit code, dan seluruh output ke LOG serta stdout job.
# Tidak menghentikan skrip; pemanggil memutuskan dari $s_rc. Output dicetak juga ke stdout
# supaya error tampil langsung di log CI tanpa perlu artefak.
step() {
  s_name=$1; shift
  log "== step $s_name: $*"
  s_out="$LOG_DIR/step-$ISSUE_LC.out"
  if "$@" > "$s_out" 2>&1; then s_rc=0; else s_rc=$?; fi
  cat "$s_out" | tee -a "$LOG"
  rm -f -- "$s_out"
  log "step_exit name=$s_name rc=$s_rc"
}

log "== V3: docker version (daemon) =="
step docker_version cli version --format 'client={{.Client.Version}} server={{.Server.Version}} os={{.Server.Os}}/{{.Server.Arch}}'
if [ "$s_rc" != 0 ]; then
  log "V3_FAIL: docker version rc=$s_rc. Diagnostik daemon di bawah."
  # Di dalam container DinD (socket unix), di jalur TCP 2375, dan apakah dockerd benar-benar berjalan.
  step daemon_docker_version podman exec "$NAME" docker version
  step daemon_ping_tcp podman exec "$NAME" wget -qO- http://127.0.0.1:2375/_ping
  step daemon_listen_2375 podman exec "$NAME" sh -c "grep -i ':0947 ' /proc/net/tcp /proc/net/tcp6 || echo 'tidak ada listener di port 2375'"
  step daemon_dockerd_proc podman exec "$NAME" sh -c "ps 2>/dev/null | grep -i dockerd || echo 'proses dockerd tidak terlihat'"
  step cli_image podman image inspect --format '{{.Id}}' "$CLI_IMAGE"
  exit 1
fi
log "== V3: compose up --wait ($PROFILES) =="
# shellcheck disable=SC2086
if cli compose $COMPOSE_ARGS $PROFILE_ARGS up -d --wait >> "$LOG" 2>&1; then
  log "compose_up_exit=0"
else
  log "compose_up_exit=$?"; cli compose $COMPOSE_ARGS $PROFILE_ARGS ps >> "$LOG" 2>&1 || true; exit 1
fi

log "== V3: smoke (sama dengan V2: psql, redis-cli, rabbitmq-diagnostics) =="
cli docker ps --filter "label=arctiva.issue=$ARC_ISSUE" --format '{{.Names}} {{.Status}}' >> "$LOG" 2>&1
smoke() { # argumen: service, perintah
  cid=$(cli docker ps --filter "label=arctiva.issue=$ARC_ISSUE" --filter "label=com.docker.compose.service=$1" -q)
  if [ -z "$cid" ]; then log "smoke_$1_exit=missing"; return 1; fi
  shift_svc=$1; shift
  cli docker exec "$cid" sh -c "$*" >> "$LOG" 2>&1
  rc_smoke=$?
  log "smoke_${shift_svc}_exit=$rc_smoke"
  return "$rc_smoke"
}
smoke_fail=0
smoke postgres 'psql -U "$POSTGRES_USER" -d "$POSTGRES_DB" -tAc "select 1"' || smoke_fail=1
smoke redis 'REDISCLI_AUTH="$(cat /run/secrets/redis_password)" redis-cli --no-auth-warning ping | grep -q PONG' || smoke_fail=1
smoke rabbitmq 'rabbitmq-diagnostics -q ping' || smoke_fail=1
healthy=$(cli docker ps --filter "label=arctiva.issue=$ARC_ISSUE" --filter "health=healthy" -q | wc -l | tr -d ' ')
total=$(cli docker ps --filter "label=arctiva.issue=$ARC_ISSUE" -q | wc -l | tr -d ' ')
log "healthy_containers=$healthy total_containers=$total"

log "== V3: bersihkan stack di dalam DinD =="
# shellcheck disable=SC2086
cli compose $COMPOSE_ARGS $PROFILE_ARGS down -v >> "$LOG" 2>&1 || log "compose_down_exit=$?"

# Syarat lulus: tiga service bundled-infra semua healthy, smoke ketiganya lolos.
[ "$total" -ge 3 ] && [ "$healthy" = "$total" ] || { log "SMOKE_GAGAL: healthy=$healthy total=$total"; exit 1; }
[ "$smoke_fail" = 0 ] || { log "SMOKE_GAGAL: smoke tidak lolos"; exit 1; }
log "V3_OK"
