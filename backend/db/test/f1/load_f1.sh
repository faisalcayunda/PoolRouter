#!/bin/sh
# Muat fixture F1 (SHAA-335) ke PostgreSQL 17 sekali pakai di Podman.
#
#   sh backend/db/test/f1/load_f1.sh
#
# Tahap: (1) cek disk, (2) container berlabel shaal.issue=SHAA-335, (3) bootstrap peran x2,
# (4) migrasi dari kosong sebagai poolrouter_owner, (5) seed induk dan muat fixture sebagai
# poolrouter_owner (load_f1.py), (6) uji negatif per constraint utama (neg_f1.sql).
# Container dihapus di akhir run. Tidak ada secret yang dicetak.
#
# Berkas yang TIDAK dimuat dan alasannya ada di docstring load_f1.py.
# MFA (pengguna_mfa, kode_*_mfa) belum dimuat: butuh TOTP_TEST_SEED dan KEK uji dari env (lihat stage MFA).
set -eu

ENGINE=${CONTAINER_ENGINE:-podman}
ISSUE=SHAA-335
IMAGE=${PG_IMAGE:-docker.io/library/postgres:17-alpine@sha256:266d3c257594e195b309ae9d1805e54c788f1d7b375f56bf6c367aac6c917e7b}
NAME="poolrouter-f1load-$$"
here=$(cd "$(dirname "$0")" && pwd)
db_dir=$(cd "$here/../.." && pwd)
mig_dir=$(cd "$db_dir/../migrations" && pwd)
work=$(mktemp -d "$here/.work.XXXXXX")

cleanup() {
  $ENGINE rm -f -v "$NAME" >/dev/null 2>&1 || true
  rm -rf "$work"
}
trap cleanup EXIT INT TERM

echo "==> disk"
df -h / | tail -1
SU_PW=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
$ENGINE run -d --name "$NAME" --label "shaal.issue=$ISSUE" \
  -e POSTGRES_PASSWORD="$SU_PW" -e POSTGRES_DB=postgres \
  --tmpfs /var/lib/postgresql/data:rw "$IMAGE" >/dev/null

i=0
until $ENGINE exec "$NAME" pg_isready -U postgres -h 127.0.0.1 >/dev/null 2>&1; do
  i=$((i + 1)); [ "$i" -le 60 ] || { echo "postgres did not become ready" >&2; exit 1; }; sleep 1
done
sleep 2
until $ENGINE exec "$NAME" pg_isready -U postgres -h 127.0.0.1 >/dev/null 2>&1; do sleep 1; done

psql_su() { $ENGINE exec -i "$NAME" psql -U postgres -h 127.0.0.1 --no-psqlrc -q -v ON_ERROR_STOP=1 "$@"; }

echo "==> secrets and role bootstrap"
$ENGINE exec "$NAME" sh -c '
  umask 077; mkdir -p /tmp/sec
  for r in poolrouter_owner pr_api pr_admin_api pr_controller pr_worker pr_webhook pr_job_balikan pr_job_arsip pr_restore; do
    od -An -N18 -tx1 /dev/urandom | tr -d " \n" > /tmp/sec/$r
  done'
$ENGINE exec "$NAME" mkdir -p /tmp/db
$ENGINE cp "$db_dir/bootstrap.sql" "$NAME:/tmp/db/bootstrap.sql"
$ENGINE cp "$db_dir/bootstrap.sh" "$NAME:/tmp/db/bootstrap.sh"
psql_su -d postgres -c 'CREATE DATABASE poolrouter_test'
for pass in 1 2; do
  $ENGINE exec -e POOLROUTER_SECRETS_DIR=/tmp/sec -e PGUSER=postgres -e PGHOST=127.0.0.1 -e PGPASSWORD="$SU_PW" \
    "$NAME" sh /tmp/db/bootstrap.sh poolrouter_test
  echo "bootstrap pass $pass: exit 0"
done

echo "==> migrations from empty"
{ echo "SET ROLE poolrouter_owner;"; for f in "$mig_dir"/*.sql; do echo "-- $(basename "$f")"; cat "$f"; echo; done; } \
  | psql_su -d poolrouter_test -1
echo "migrations: exit 0"

echo "==> generate load and negative tests"
python3 -I "$here/load_f1.py" "$work"
echo "generate: exit 0"

echo "==> load positive fixtures (owner role)"
$ENGINE cp "$work/load_f1.sql" "$NAME:/tmp/load_f1.sql"
$ENGINE exec "$NAME" psql -U postgres -h 127.0.0.1 --no-psqlrc -q -v ON_ERROR_STOP=1 -d poolrouter_test -f /tmp/load_f1.sql
echo "load: exit 0"

echo "==> row counts per table"
for t in client client_kredensial provider upstream_pool downstream_pool downstream_pool_rute \
         downstream_endpoint request transaction_log data_balikan_batch data_balikan_item; do
  n=$(psql_su -d poolrouter_test -t -A -c "SELECT count(*) FROM $t")
  echo "$t $n"
done

if [ -s "$here/neg_f1.sql" ]; then
  echo "==> negative tests (each must be rejected)"
  $ENGINE cp "$here/neg_f1.sql" "$NAME:/tmp/neg_f1.sql"
  $ENGINE exec "$NAME" psql -U postgres -h 127.0.0.1 --no-psqlrc -q -v ON_ERROR_STOP=1 -d poolrouter_test -f /tmp/neg_f1.sql
  echo "negative tests: exit 0"
fi

echo "F1 LOAD OK"
