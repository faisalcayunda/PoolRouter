#!/bin/sh
# Harness skrip Lua kuota (SHAA-195) di Redis sungguhan, dalam container sekali pakai.
#
#   sh backend/lua/test/run.sh
#   CONTAINER_ENGINE=docker sh backend/lua/test/run.sh
#   LUA_REDIS_ADDR=127.0.0.1:6395 sh backend/lua/test/run.sh   # Redis yang sudah ada
#
# Tanpa LUA_REDIS_ADDR skrip ini menyalakan Redis dari image yang sama dengan
# deploy/compose/compose.yaml (pin digest, tidak pernah `latest`), memberi label
# shaal.issue=<SHAA_ISSUE>, memetakan port acak di loopback, lalu mematikannya
# kembali. Persistence dimatikan: harness butuh keadaan kosong, bukan data lama.
set -eu

ENGINE=${CONTAINER_ENGINE:-podman}
ISSUE=${SHAA_ISSUE:-SHAA-195}
PYTHON=${PYTHON:-python3}
# Sama dengan deploy/compose/compose.yaml (baris redis).
IMAGE=${REDIS_IMAGE:-docker.io/library/redis:7-alpine@sha256:8da19c7399dca900c6822a1cf30f9d2285585e2c4b457ff4095b0f4500054d3f}
NAME="poolrouter-luatest-$$"
lua_dir=$(cd "$(dirname "$0")/../../crates/core/src/kuota/lua" && pwd)
harness="$lua_dir/uji_lua.py"

[ -f "$harness" ] || { echo "harness tidak ada: $harness" >&2; exit 1; }

bersihkan() {
  if [ -n "${NAME_AKTIF:-}" ]; then
    $ENGINE rm -f -v "$NAME" >/dev/null 2>&1 || true
  fi
}
trap bersihkan EXIT INT TERM

if [ -n "${LUA_REDIS_ADDR:-}" ]; then
  host=${LUA_REDIS_ADDR%:*}
  port=${LUA_REDIS_ADDR##*:}
else
  $ENGINE run -d --name "$NAME" --label "shaal.issue=$ISSUE" \
    -p 127.0.0.1::6379 "$IMAGE" \
    redis-server --save '' --appendonly no >/dev/null
  NAME_AKTIF=1
  port=$($ENGINE port "$NAME" 6379/tcp | head -n 1 | sed 's/.*://')
  [ -n "$port" ] || { echo "port Redis tidak terbaca" >&2; exit 1; }
  host=127.0.0.1

  i=0
  until $ENGINE exec "$NAME" redis-cli ping >/dev/null 2>&1; do
    i=$((i + 1))
    [ "$i" -le 60 ] || { echo "redis tidak siap" >&2; exit 1; }
    sleep 1
  done
fi

echo "uji-lua: Redis $host:$port, harness $harness"
# Tanpa `exec`: trap EXIT harus sempat mematikan container.
status=0
"$PYTHON" "$harness" --host "$host" --port "$port" "$@" || status=$?
exit "$status"
