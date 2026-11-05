#!/bin/sh
# Uji integrasi Rust terhadap PostgreSQL sungguhan (SHAA-196). Dipanggil oleh `make uji-pg`.
#
# PostgreSQL jalan sebagai container sekali pakai berlabel arctiva.issue=<ARC_ISSUE>, skema dari
# migrasi, peran dari bootstrap. Port dipublikasikan ke 127.0.0.1 supaya container toolchain cargo
# (jaringan host) menjangkaunya. Uji berjalan sebagai `pr_admin_api`, bukan superuser, jadi
# grant yang dipakai kode sungguhan ikut teruji. Password acak hanya hidup di container dan proses ini.
#
#   sh db/test/uji-pg.sh <target-make-yang-menjalankan-cargo>
set -eu

ENGINE=${CONTAINER_ENGINE:-podman}
ISSUE=$ARC_ISSUE
: "${ARC_ISSUE:?isi ARC_ISSUE, mis. ARC-123}"
# Gagal di awal dengan pesan jelas, bukan nanti saat `podman`/`docker` dipanggil.
command -v "$ENGINE" >/dev/null 2>&1 || { echo "uji-pg.sh: '$ENGINE' tidak ada di PATH; set CONTAINER_ENGINE=docker atau pasang engine-nya" >&2; exit 127; }
PORT=${UJI_PG_PORT:-$((20000 + $$ % 20000))}
MAKE_CMD=${1:?target make untuk cargo test belum diberikan}
IMAGE=${PG_IMAGE:-docker.io/library/postgres:17-alpine@sha256:266d3c257594e195b309ae9d1805e54c788f1d7b375f56bf6c367aac6c917e7b}
NAME="poolrouter-ujipg-$$"
here=$(cd "$(dirname "$0")" && pwd)
db_dir=$(cd "$here/.." && pwd)
mig_dir=$(cd "$db_dir/../migrations" && pwd)

RPORT=${UJI_PG_REDIS_PORT:-$((40000 + $$ % 20000))}
RIMAGE=${REDIS_IMAGE:-docker.io/library/redis:7-alpine@sha256:8da19c7399dca900c6822a1cf30f9d2285585e2c4b457ff4095b0f4500054d3f}
RNAME="poolrouter-ujipg-redis-$$"

bersihkan() {
  $ENGINE rm -f -v "$NAME" >/dev/null 2>&1 || true
  $ENGINE rm -f -v "$RNAME" >/dev/null 2>&1 || true
}
trap bersihkan EXIT INT TERM

# Redis for the login limiter (SHAA-196): throwaway, labelled, no persistence.
$ENGINE run -d --name "$RNAME" --label "arctiva.issue=$ISSUE" -p "127.0.0.1:$RPORT:6379" "$RIMAGE" \
  redis-server --save '' --appendonly no >/dev/null

SU_PW=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
$ENGINE run -d --name "$NAME" --label "arctiva.issue=$ISSUE" -p "127.0.0.1:$PORT:5432" \
  -e POSTGRES_PASSWORD="$SU_PW" -e POSTGRES_DB=postgres \
  --tmpfs /var/lib/postgresql/data:rw "$IMAGE" >/dev/null

i=0
until $ENGINE exec "$NAME" pg_isready -U postgres -h 127.0.0.1 >/dev/null 2>&1; do
  i=$((i + 1))
  [ "$i" -le 60 ] || { echo "postgres tidak siap" >&2; exit 1; }
  sleep 1
done
sleep 2
until $ENGINE exec "$NAME" pg_isready -U postgres -h 127.0.0.1 >/dev/null 2>&1; do sleep 1; done

i=0
until $ENGINE exec "$RNAME" redis-cli ping >/dev/null 2>&1; do
  i=$((i + 1))
  [ "$i" -le 60 ] || { echo "redis tidak siap" >&2; exit 1; }
  sleep 1
done

psql_su() { $ENGINE exec -i "$NAME" psql -U postgres -h 127.0.0.1 --no-psqlrc -q -v ON_ERROR_STOP=1 "$@"; }

$ENGINE exec "$NAME" sh -c '
  umask 077; mkdir -p /tmp/sec
  for r in poolrouter_owner pr_api pr_admin_api pr_controller pr_worker pr_webhook pr_job_balikan pr_job_arsip pr_restore pr_blend; do
    od -An -N18 -tx1 /dev/urandom | tr -d " \n" > /tmp/sec/$r
  done'
$ENGINE exec "$NAME" mkdir -p /tmp/db
$ENGINE cp "$db_dir/bootstrap.sql" "$NAME:/tmp/db/bootstrap.sql"
$ENGINE cp "$db_dir/bootstrap.sh" "$NAME:/tmp/db/bootstrap.sh"
psql_su -d postgres -c 'CREATE DATABASE poolrouter_test'
$ENGINE exec -e POOLROUTER_SECRETS_DIR=/tmp/sec -e PGUSER=postgres -e PGHOST=127.0.0.1 -e PGPASSWORD="$SU_PW" \
  "$NAME" sh /tmp/db/bootstrap.sh poolrouter_test
{ echo "SET ROLE poolrouter_owner;"; for f in "$mig_dir"/*.sql; do echo "-- $(basename "$f")"; cat "$f"; echo; done; } \
  | psql_su -d poolrouter_test -1

PW=$($ENGINE exec "$NAME" cat /tmp/sec/pr_admin_api)
export POOLROUTER_TEST_DATABASE_URL="postgres://pr_admin_api:$PW@127.0.0.1:$PORT/poolrouter_test"
export POOLROUTER_TEST_REDIS="redis://127.0.0.1:$RPORT"
export POOLROUTER_TEST_DATABASE_URL_SU="postgres://postgres:$SU_PW@127.0.0.1:$PORT/poolrouter_test"
# The controller tests run as the role the controller itself uses (ARC-69): the
# snapshot and rebuild must work under pr_controller's column grants, not the
# stricter pr_admin_api ones, and not as superuser.
PW_C=$($ENGINE exec "$NAME" cat /tmp/sec/pr_controller)
export POOLROUTER_TEST_DATABASE_URL_CONTROLLER="postgres://pr_controller:$PW_C@127.0.0.1:$PORT/poolrouter_test"
echo "uji-pg: PostgreSQL 127.0.0.1:$PORT sebagai pr_admin_api"
$MAKE_CMD
echo "uji-pg: lulus"
