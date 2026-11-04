#!/bin/sh
# Dijalankan entrypoint nginx sebelum render template (20-envsubst-on-templates.sh).
# Menyiapkan direktori keluaran dan include allowlist IP (ADR-0003 §3.9, R-12).
set -eu

out=${NGINX_ENVSUBST_OUTPUT_DIR:-/tmp/nginx-conf}
mkdir -p "$out"
allow="$out/allowlist.inc"
: > "$allow"

# HSTS hanya di poc dan prod.
hsts="$out/hsts.inc"
: > "$hsts"
case "${POOLROUTER_ENV:-}" in
  poc|prod) echo 'add_header Strict-Transport-Security "max-age=31536000; includeSubDomains" always;' > "$hsts" ;;
esac

# Allowlist IP admin hanya ditegakkan di lingkungan poc. Lingkungan lain: include kosong.
if [ "${POOLROUTER_ENV:-}" = "poc" ]; then
  list=$(printf '%s' "${POOLROUTER_ADMIN_IP_ALLOWLIST:-}" | tr ',' ' ')
  n=0
  # Healthcheck container (wget ke loopback) tidak boleh terkunci oleh allowlist.
  { echo "allow 127.0.0.1;"; echo "allow ::1;"; } >> "$allow"
  for entry in $list; do
    # Hanya IPv4/IPv6/CIDR; token lain ditolak supaya tidak menyuntik direktif nginx.
    case "$entry" in
      *[!0-9a-fA-F:./]*) echo "POOLROUTER_ADMIN_IP_ALLOWLIST: entri tidak valid" >&2; exit 1 ;;
      "") continue ;;
    esac
    echo "allow $entry;" >> "$allow"
    n=$((n + 1))
  done
  if [ "$n" -eq 0 ]; then
    echo "POOLROUTER_ENV=poc mewajibkan POOLROUTER_ADMIN_IP_ALLOWLIST (daftar IP/CIDR dipisah koma)" >&2
    exit 1
  fi
  echo "deny all;" >> "$allow"
fi
