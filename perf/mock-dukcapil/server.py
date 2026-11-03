"""Mock upstream Dukcapil for SHAA-205 load tests (synthetic only, stdlib only).

Purpose:
- Latency is adjustable at runtime (POST /_config) so one container serves the
  0.5 s / 2 s / 5 s scenarios without a restart.
- Every call that reaches the mock is an upstream "percobaan" (attempt). Attempts
  are counted per upstream pool, keyed by the first path segment after /p/.
  Pool p is reached through the base URL http://<host>:8080/p/<pool>/...
- M-02 = number of attempts whose index is above kuota_efektif for the pool.
  The mock still answers those calls, so the violation is observed, not hidden.

Endpoints:
- ANY /p/<pool>/...   counted upstream call (after latency), JSON body
- GET /healthz        liveness
- GET /_stats         counters per pool + m02_over_kuota_total
- POST /_reset        zero counters (config is kept)
- POST /_config       JSON {"latency_ms": int, "kuota_efektif": {"pool": int}}

Env:
- MOCK_PORT (default 8080), MOCK_LATENCY_MS (default 500),
  MOCK_KUOTA_EFEKTIF (JSON object, default {}), MOCK_BODY_FILE (optional file
  with the response body; default is a placeholder, see README note).

Nothing here is a secret. Responses carry no real NIK or personal data.
"""

import json
import math
import os
import threading
import time
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.parse import urlsplit

PORT = int(os.environ.get("MOCK_PORT", "8080"))
LOCK = threading.Lock()
STATE = {
    "latency_ms": int(os.environ.get("MOCK_LATENCY_MS", "500")),
    "kuota_efektif": json.loads(os.environ.get("MOCK_KUOTA_EFEKTIF", "{}") or "{}"),
}
COUNTS = {}  # pool -> {"attempts": int, "over_kuota": int}
# pool -> list of actual handler latencies in ms (receive to response write). The
# nominal MOCK_LATENCY_MS is not the same as this: time.sleep adds jitter. Overhead
# in the report is computed against the mean of this list from a control run.
LAT_ACTUAL = {}
BODY_FILE = os.environ.get("MOCK_BODY_FILE", "")
# Default body follows ADR-0002 rev 5 D3 (contract "Verify By Element"): a
# pagination envelope whose `content` holds exactly one element, with values
# "Sesuai" / "Tidak Sesuai" optionally followed by " (skor)". Only the parts the
# adapter parses are shaped here. The pagination keys outside `content` are not
# specified in ADR-0002 yet (open question P-3 / Dukcapil docs), so they are left
# out. Replace with the synthetic envelope from the contract via MOCK_BODY_FILE
# when it is confirmed; never use real data.
DEFAULT_BODY = {"content": [{"NAMA_LGKP": "Sesuai (100)"}]}


def load_body():
    if BODY_FILE and os.path.isfile(BODY_FILE):
        with open(BODY_FILE, "rb") as fh:
            return fh.read()
    return json.dumps(DEFAULT_BODY).encode("utf-8")


BODY = load_body()


def record_attempt(pool):
    with LOCK:
        c = COUNTS.setdefault(pool, {"attempts": 0, "over_kuota": 0})
        c["attempts"] += 1
        quota = STATE["kuota_efektif"].get(pool)
        if quota is not None and c["attempts"] > int(quota):
            c["over_kuota"] += 1


def record_actual_latency(pool, ms):
    with LOCK:
        LAT_ACTUAL.setdefault(pool, []).append(ms)


def _summary(values):
    if not values:
        return {"count": 0}
    s = sorted(values)

    def pct(p):
        # nearest-rank percentile
        return s[min(len(s) - 1, max(0, math.ceil(p / 100.0 * len(s)) - 1))]

    return {
        "count": len(s),
        "mean": round(sum(s) / len(s), 3),
        "min": round(s[0], 3),
        "p50": round(pct(50), 3),
        "p95": round(pct(95), 3),
        "p99": round(pct(99), 3),
        "max": round(s[-1], 3),
    }


class Handler(BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"
    server_version = "mock-dukcapil/1"

    def log_message(self, fmt, *args):  # keep container logs small
        return

    def _send(self, status, payload, ctype="application/json"):
        body = payload if isinstance(payload, bytes) else json.dumps(payload).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", ctype)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _drain(self):
        try:
            length = int(self.headers.get("Content-Length") or 0)
        except ValueError:
            # Content-Length tidak valid: body tidak bisa dibaca, jadi koneksi ditutup.
            self.close_connection = True
            return b""
        if length > 0:
            return self.rfile.read(length)
        return b""

    def _route(self):
        return urlsplit(self.path).path

    def do_GET(self):
        self._dispatch()

    def do_POST(self):
        self._dispatch()

    def do_PUT(self):
        self._dispatch()

    def do_DELETE(self):
        self._dispatch()

    def _dispatch(self):
        path = self._route()
        raw = self._drain()
        if path == "/healthz":
            return self._send(200, {"ok": True})
        if path == "/_stats":
            with LOCK:
                pools = {
                    k: dict(v, kuota_efektif=STATE["kuota_efektif"].get(k), latency_actual_ms=_summary(LAT_ACTUAL.get(k, [])))
                    for k, v in COUNTS.items()
                }
                total = sum(v["over_kuota"] for v in COUNTS.values())
                latency = STATE["latency_ms"]
                all_lat = [x for vals in LAT_ACTUAL.values() for x in vals]
            return self._send(
                200,
                {
                    "latency_ms": latency,
                    "latency_actual_ms": _summary(all_lat),
                    "pools": pools,
                    "m02_over_kuota_total": total,
                },
            )
        if path == "/_reset":
            with LOCK:
                COUNTS.clear()
                LAT_ACTUAL.clear()
            return self._send(200, {"reset": True})
        if path == "/_config":
            try:
                cfg = json.loads(raw or b"{}")
                with LOCK:
                    if "latency_ms" in cfg:
                        STATE["latency_ms"] = int(cfg["latency_ms"])
                    if "kuota_efektif" in cfg:
                        STATE["kuota_efektif"] = {str(k): int(v) for k, v in cfg["kuota_efektif"].items()}
                    latency = STATE["latency_ms"]
                    quotas = dict(STATE["kuota_efektif"])
            except (ValueError, TypeError, AttributeError):
                return self._send(400, {"error": "config_tidak_valid"})
            return self._send(200, {"latency_ms": latency, "kuota_efektif": quotas})

        parts = path.split("/", 3)  # ["", "p", "<pool>", rest]
        if len(parts) >= 3 and parts[1] == "p" and parts[2]:
            pool = parts[2]
            record_attempt(pool)
            with LOCK:
                latency = STATE["latency_ms"]
            t0 = time.perf_counter()
            if latency > 0:
                time.sleep(latency / 1000.0)
            actual_ms = (time.perf_counter() - t0) * 1000.0
            record_actual_latency(pool, actual_ms)
            return self._send(200, BODY)
        return self._send(404, {"error": "tidak_ditemukan"})


class Server(ThreadingHTTPServer):
    daemon_threads = True
    request_queue_size = 1024
    allow_reuse_address = True


if __name__ == "__main__":
    Server(("0.0.0.0", PORT), Handler).serve_forever()
