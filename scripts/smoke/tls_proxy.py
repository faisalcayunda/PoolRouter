"""Sidecar TLS `mock-tls` untuk smoke (ARC-378, spec smoke-test Q1).

Masalah: `createProvider` mewajibkan `base_url` berawalan `https://`, tetapi
mock-dukcapil (perf/mock-dukcapil/server.py) cuma HTTP biasa. Proxy ini
menerima TLS di MOCK_TLS_PORT (default 8443) dengan sertifikat buatan run
(CA sendiri, 1 hari, lihat scripts/smoke.sh) dan meneruskan tiap request
apa adanya ke MOCK_UPSTREAM (default http://mock-dukcapil:8080), lalu
meneruskan balik respons apa adanya. Tanpa dependensi luar stdlib, sama
seperti perf/mock-dukcapil/server.py; dijalankan di image yang sama
(`python -I /smoke/tls_proxy.py`).

Env:
- MOCK_TLS_PORT (default 8443)
- MOCK_TLS_CERT_FILE, MOCK_TLS_KEY_FILE (wajib; PEM, dari CA run)
- MOCK_UPSTREAM (default http://mock-dukcapil:8080)

Tidak ada secret atau data pribadi: sertifikat dibuat per run, tanpa SAN atau
subjek yang memuat data sungguhan; body request/response diteruskan transparan
tanpa dicatat.
"""

import os
import ssl
import sys
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from urllib.error import HTTPError, URLError
from urllib.request import Request, urlopen

PORT = int(os.environ.get("MOCK_TLS_PORT", "8443"))
UPSTREAM = os.environ.get("MOCK_UPSTREAM", "http://mock-dukcapil:8080").rstrip("/")
CERT_FILE = os.environ["MOCK_TLS_CERT_FILE"]
KEY_FILE = os.environ["MOCK_TLS_KEY_FILE"]

# Header yang tidak boleh diteruskan apa adanya (hop-by-hop / panjang transfer
# yang sudah dihitung ulang oleh urllib saat membaca body).
_SKIP_REQUEST_HEADERS = {"host", "connection", "content-length"}
_SKIP_RESPONSE_HEADERS = {"connection", "transfer-encoding"}


class ProxyHandler(BaseHTTPRequestHandler):
    def _proxy(self):
        length = int(self.headers.get("Content-Length", 0) or 0)
        body = self.rfile.read(length) if length else None
        headers = {
            k: v for k, v in self.headers.items() if k.lower() not in _SKIP_REQUEST_HEADERS
        }
        req = Request(UPSTREAM + self.path, data=body, headers=headers, method=self.command)
        try:
            with urlopen(req, timeout=30) as resp:
                self._relay(resp.status, resp.headers.items(), resp.read())
        except HTTPError as exc:
            self._relay(exc.code, exc.headers.items() if exc.headers else [], exc.read())
        except URLError as exc:
            self.send_response(502)
            self.send_header("Content-Type", "text/plain")
            self.end_headers()
            self.wfile.write(f"mock-tls: upstream tidak terjangkau: {exc}".encode())

    def _relay(self, status, header_items, body):
        self.send_response(status)
        for k, v in header_items:
            if k.lower() not in _SKIP_RESPONSE_HEADERS:
                self.send_header(k, v)
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        if body:
            self.wfile.write(body)

    def do_GET(self):
        self._proxy()

    def do_POST(self):
        self._proxy()

    def do_PUT(self):
        self._proxy()

    def do_PATCH(self):
        self._proxy()

    def do_DELETE(self):
        self._proxy()

    def log_message(self, fmt, *args):  # noqa: D401 - stdlib override, tanpa data pribadi
        sys.stderr.write("mock-tls: " + (fmt % args) + "\n")


def main():
    ctx = ssl.SSLContext(ssl.PROTOCOL_TLS_SERVER)
    ctx.load_cert_chain(certfile=CERT_FILE, keyfile=KEY_FILE)
    server = ThreadingHTTPServer(("0.0.0.0", PORT), ProxyHandler)  # noqa: S104 - sidecar efemeral di network compose
    server.socket = ctx.wrap_socket(server.socket, server_side=True)
    sys.stderr.write(f"mock-tls: TLS di :{PORT} -> {UPSTREAM}\n")
    server.serve_forever()


if __name__ == "__main__":
    main()
