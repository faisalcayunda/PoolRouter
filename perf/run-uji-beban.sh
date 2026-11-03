#!/bin/sh
# Satu run uji beban SHAA-205 (lokal, Podman, data sintetis).
#
# Pemakaian (contoh, dari akar repo):
#   RUN_NAME=sync-lat2s-r1 MODE=sync LAT_MS=2000 RATE=11.6 DURATION=10m \
#   KUOTA_EFEKTIF='{"pool-uji":6960}' BASE_URL=http://poolrouter-gateway:8080 \
#   ENDPOINT_ID=<uuid> POOLROUTER_API_KEY=... GATEWAY_NET=<nama-network-gateway> \
#     sh perf/run-uji-beban.sh
#
# Yang dibuat dan dihapus di akhir run (semua berlabel shaal.issue=SHAA-205):
#   - network sementara, container mock Dukcapil, container k6
# Yang tidak disentuh: resource tanpa label ini, termasuk gateway dan layanan lain.
#
# Keluaran di <workdir>/logs/perf-shaa205/<RUN_NAME>/: summary k6 (JSON) dan
# statistik mock (JSON). Exit code 1 jika M-02 (m02_over_kuota_total) > 0;
# exit code 2 jika run tidak valid (KUOTA_EFEKTIF kosong, mock tidak dipanggil, atau nama pool tidak cocok).
# Secret hanya dibaca dari environment; tidak ditulis ke file atau log.

set -eu

LABEL=shaal.issue=SHAA-205
REPO=$(cd "$(dirname "$0")/.." && pwd -P)
WORKDIR=$(dirname "$REPO")
RUN_NAME=${RUN_NAME:?isi RUN_NAME, mis. sync-lat2s-r1}
MODE=${MODE:-sync}
LAT_MS=${LAT_MS:-500}
RATE=${RATE:-11.6}
DURATION=${DURATION:-10m}
KUOTA_EFEKTIF=${KUOTA_EFEKTIF:?isi KUOTA_EFEKTIF dengan kuota per pool}
case "$KUOTA_EFEKTIF" in '{}'|'') echo "KUOTA_EFEKTIF tidak boleh kosong: M-02 tidak bisa dibuktikan" >&2; exit 2 ;; esac
MOCK_IMAGE=localhost/perf-mock-dukcapil:shaa205
MOCK_NAME=perf-mock-shaa205
NET=perf-shaa205-net
OUT="$WORKDIR/logs/perf-shaa205/$RUN_NAME"
# Dipin ke tag dan digest, sama dengan Containerfile.
K6_IMAGE=docker.io/grafana/k6@sha256:76f864063eb66a2d2b5c5a0be00cb9effa3ea9dc3bd695f5d12f6c1e7cf9e3fd

: "${BASE_URL:?isi BASE_URL gateway}"
: "${ENDPOINT_ID:?isi ENDPOINT_ID}"
: "${POOLROUTER_API_KEY:?isi POOLROUTER_API_KEY dari environment}"

cleanup() {
  # Pola bersih di shaal-playbook bagian 6: hanya resource berlabel shaal.issue=SHAA-205.
  podman rm -f -v --filter label="$LABEL" >/dev/null 2>&1 || true
  podman volume prune -f --filter label="$LABEL" >/dev/null 2>&1 || true
  podman network prune -f --filter label="$LABEL" >/dev/null 2>&1 || true
  # Image mock dibangun ulang setiap run; hapus di akhir run (nama image tepat, bukan pola).
  podman rmi -f "$MOCK_IMAGE" >/dev/null 2>&1 || true
}
trap cleanup EXIT

mkdir -p -- "$OUT"

podman build --label "$LABEL" -t "$MOCK_IMAGE" -f "$REPO/perf/mock-dukcapil/Containerfile" "$REPO/perf/mock-dukcapil" >"$OUT/build-mock.log" 2>&1

podman network create --label "$LABEL" "$NET" >/dev/null

JOIN_ARGS="--network $NET"
if [ -n "${GATEWAY_NET:-}" ]; then
  JOIN_ARGS="$JOIN_ARGS --network $GATEWAY_NET"
fi

# shellcheck disable=SC2086
podman run -d --name "$MOCK_NAME" --label "$LABEL" $JOIN_ARGS --network-alias perf-mock \
  -e MOCK_LATENCY_MS="$LAT_MS" -e MOCK_KUOTA_EFEKTIF="$KUOTA_EFEKTIF" \
  "$MOCK_IMAGE" >/dev/null

# Tunggu mock siap (maks 30 detik).
i=0
until podman exec "$MOCK_NAME" python -I -c "import urllib.request as u; u.urlopen('http://127.0.0.1:8080/healthz', timeout=2)" >/dev/null 2>&1; do
  i=$((i + 1))
  [ "$i" -lt 30 ] || { echo "mock tidak siap" >&2; exit 2; }
  sleep 1
done

# Uji beban. Secret diteruskan sebagai nama variabel saja (-e NAMA), nilainya dari environment host.
set +e
# shellcheck disable=SC2086
podman run --rm --label "$LABEL" $JOIN_ARGS \
  -v "$REPO/perf/k6:/scripts:ro" -v "$OUT:/out" \
  -e BASE_URL -e ENDPOINT_ID -e ENDPOINT_ID_ASYNC -e POOLROUTER_API_KEY \
  -e MODE="$MODE" -e RATE="$RATE" -e SYNC_SHARE="${SYNC_SHARE:-0.2}" -e DURATION="$DURATION" \
  -e POLL_SAMPLE="${POLL_SAMPLE:-0.1}" -e POLL_TIMEOUT_S="${POLL_TIMEOUT_S:-90}" \
  "$K6_IMAGE" run --summary-export=/out/k6-summary.json /scripts/uji-beban.js >"$OUT/k6-stdout.log" 2>&1
K6_RC=$?
set -e

# Statistik mock dan validasi run. Parsing dilakukan di dalam container mock (tanpa python host).
# Run tidak valid (exit 2), sehingga tidak bisa lolos palsu, bila:
#   - statistik mock tidak bisa diambil;
#   - total percobaan 0 (mock tidak pernah dipanggil);
#   - ada pool tanpa kuota_efektif (nama pool salah ketik atau KUOTA_EFEKTIF tidak lengkap);
#   - pool di KUOTA_EFEKTIF tidak pernah dipanggil.
if ! podman exec "$MOCK_NAME" python -I -c "import urllib.request as u; print(u.urlopen('http://127.0.0.1:8080/_stats', timeout=5).read().decode())" >"$OUT/mock-stats.json"; then
  echo "RUN TIDAK VALID: statistik mock tidak bisa diambil" >&2
  exit 2
fi
if ! OVER=$(podman exec -i -e KUOTA_EFEKTIF="$KUOTA_EFEKTIF" "$MOCK_NAME" python -I -c '
import json, os, sys
s = json.loads(sys.stdin.read())
q = json.loads(os.environ["KUOTA_EFEKTIF"])
pools = s.get("pools", {})
err = []
if not q:
    err.append("KUOTA_EFEKTIF kosong")
if sum(p.get("attempts", 0) for p in pools.values()) == 0:
    err.append("total percobaan 0: mock tidak pernah dipanggil")
for k, p in pools.items():
    if p.get("kuota_efektif") is None:
        err.append("pool tanpa kuota_efektif: " + k)
for k in q:
    if pools.get(k, {}).get("attempts", 0) == 0:
        err.append("pool di KUOTA_EFEKTIF tidak pernah dipanggil: " + k)
if err:
    sys.stderr.write("; ".join(err) + "\n")
    sys.exit(2)
print(s["m02_over_kuota_total"])
' <"$OUT/mock-stats.json"); then
  echo "RUN TIDAK VALID: lihat pesan di atas (exit 2)" >&2
  exit 2
fi
echo "RUN=$RUN_NAME K6_RC=$K6_RC M02_OVER_KUOTA=$OVER OUT=$OUT"

if [ "$OVER" -ne 0 ]; then
  echo "M-02 GAGAL: $OVER percobaan melewati kuota_efektif" >&2
  exit 1
fi
exit "$K6_RC"
