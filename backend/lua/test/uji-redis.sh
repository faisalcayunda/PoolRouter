#!/bin/sh
# Uji fail-closed, deteksi failover, dan rebuild terhadap Redis sungguhan
# (SHAA-195, ADR-0001 rev 15 §5 dan §5a.2). Dipanggil oleh `make uji-redis`.
#
# Redis jalan sebagai container sekali pakai berlabel arctiva.issue=<ARC_ISSUE>
# dengan jaringan host dan port tetap, supaya container toolchain cargo bisa
# menjangkaunya. Container di-restart di tengah (persistence mati, jadi state
# hilang dan `run_id` baru) untuk membuktikan dispatch berhenti lalu rebuild
# memulihkan counter.
#
#   sh lua/test/uji-redis.sh <target-make-yang-menjalankan-cargo>
set -eu

ENGINE=${CONTAINER_ENGINE:-podman}
ISSUE=$ARC_ISSUE
: "${ARC_ISSUE:?isi ARC_ISSUE, mis. ARC-123}"
# Gagal di awal dengan pesan jelas, bukan nanti saat `podman`/`docker` dipanggil.
command -v "$ENGINE" >/dev/null 2>&1 || { echo "uji-redis.sh: '$ENGINE' tidak ada di PATH; set CONTAINER_ENGINE=docker atau pasang engine-nya" >&2; exit 127; }
# Port acak per proses supaya dua lane paralel di VM yang sama tidak bertabrakan
# (review 225be8d, non-blocking 4); UJI_REDIS_PORT menimpanya.
PORT=${UJI_REDIS_PORT:-$((20000 + $$ % 20000))}
MAKE_CMD=${1:?target make untuk cargo test belum diberikan}
IMAGE=${REDIS_IMAGE:-docker.io/library/redis:7-alpine@sha256:8da19c7399dca900c6822a1cf30f9d2285585e2c4b457ff4095b0f4500054d3f}
NAME="poolrouter-redistest-$$"

bersihkan() {
  $ENGINE rm -f -v "$NAME" >/dev/null 2>&1 || true
}
trap bersihkan EXIT INT TERM

tunggu_siap() {
  i=0
  until $ENGINE exec "$NAME" redis-cli -p "$PORT" ping >/dev/null 2>&1; do
    i=$((i + 1))
    [ "$i" -le 60 ] || { echo "redis tidak siap" >&2; exit 1; }
    sleep 1
  done
}

$ENGINE run -d --name "$NAME" --label "arctiva.issue=$ISSUE" --network host "$IMAGE" \
  redis-server --port "$PORT" --bind 127.0.0.1 --save '' --appendonly no >/dev/null
tunggu_siap

export POOLROUTER_TEST_REDIS="redis://127.0.0.1:$PORT"
echo "uji-redis: Redis $POOLROUTER_TEST_REDIS"

# Tanpa restart.
POOLROUTER_TEST_FASE= $MAKE_CMD

# Fase sebelum restart: rebuild, lalu catat run_id.
log=$(mktemp)
trap 'rm -f "$log"; bersihkan' EXIT INT TERM
POOLROUTER_TEST_FASE=sebelum $MAKE_CMD | tee "$log"
lama=$(sed -n 's/.*RUN_ID_LAMA=\([0-9a-f]*\).*/\1/p' "$log" | head -n 1)
[ -n "$lama" ] || { echo "run_id lama tidak tercatat" >&2; exit 1; }

# Restart: state hilang, run_id baru.
$ENGINE restart "$NAME" >/dev/null
tunggu_siap

POOLROUTER_TEST_FASE=sesudah POOLROUTER_TEST_RUN_ID_LAMA="$lama" $MAKE_CMD
echo "uji-redis: lulus"
