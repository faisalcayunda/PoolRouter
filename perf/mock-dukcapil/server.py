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
BODY_FILE = os.environ.get("MOCK_BODY_FILE", "")
# Placeholder body. Replace with the synthetic envelope from the Dukcapil
# "Verify By Element" contract (ADR-0002) before the run; do not use real data.
DEFAULT_BODY = {"response": {"status": "Sesuai", "skor": 1.0}, "synthetic": True}


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
        length = int(self.headers.get("Content-Length") or 0)
        if length:
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
                pools = {k: dict(v, kuota_efektif=STATE["kuota_efektif"].get(k)) for k, v in COUNTS.items()}
                total = sum(v["over_kuota"] for v in COUNTS.values())
                latency = STATE["latency_ms"]
            return self._send(200, {"latency_ms": latency, "pools": pools, "m02_over_kuota_total": total})
        if path == "/_reset":
            with LOCK:
                COUNTS.clear()
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
            if latency > 0:
                time.sleep(latency / 1000.0)
            return self._send(200, BODY)
        return self._send(404, {"error": "tidak_ditemukan"})


class Server(ThreadingHTTPServer):
    daemon_threads = True
    request_queue_size = 1024
    allow_reuse_address = True


if __name__ == "__main__":
    Server(("0.0.0.0", PORT), Handler).serve_forever()
