#!/bin/sh
# Entrypoint image PostgreSQL + pgBackRest (ADR-0005 §4.5). Berjalan sebagai 10001:10001.
#  1. Baca secret pgBackRest dari /run/secrets lalu ekspor ke proses (pola KD-1: entrypoint tidak mendukung *_FILE).
#  2. Siapkan sertifikat TLS server (ssl = on): dari secret pada staging dan prod, self-signed hanya untuk dev.
#  3. Jalankan penjadwal backup di latar belakang untuk postgres, lalu serahkan ke entrypoint resmi.
set -eu

SECRETS_DIR=/run/secrets
TLS_DIR=/var/lib/postgresql/tls

die() { echo "pr-pg-entrypoint: $*" >&2; exit 1; }

# read_secret <nama berkas> <nama variabel> <wajib: 1|0>
read_secret() {
	f="$SECRETS_DIR/$1"
	if [ ! -e "$f" ]; then
		[ "$3" = 1 ] && die "secret $1 tidak ada di $SECRETS_DIR"
		return 0
	fi
	v=$(cat "$f")
	[ -n "$v" ] || die "secret $1 kosong"
	export "$2=$v"
}

: "${PGBACKREST_REPO1_CIPHER_TYPE:=aes-256-cbc}"
export PGBACKREST_REPO1_CIPHER_TYPE
if [ "$PGBACKREST_REPO1_CIPHER_TYPE" != none ]; then
	read_secret pgbackrest_repo_passphrase PGBACKREST_REPO1_CIPHER_PASS 1
fi

# Sertifikat TLS: secret pg_server_crt dan pg_server_key (staging dan prod) dipakai bila ada.
# Self-signed hanya untuk POOLROUTER_ENV=dev. Di lingkungan lain tanpa sertifikat, start gagal.
if [ -s /run/secrets/pg_server_crt ] && [ -s /run/secrets/pg_server_key ]; then
	cp /run/secrets/pg_server_crt "$TLS_DIR/server.crt"
	cp /run/secrets/pg_server_key "$TLS_DIR/server.key"
	chmod 0600 "$TLS_DIR/server.key"
elif [ "${POOLROUTER_ENV:-}" = dev ]; then
	if [ ! -s "$TLS_DIR/server.key" ] || [ ! -s "$TLS_DIR/server.crt" ]; then
		echo "pr-pg-entrypoint: sertifikat TLS self-signed untuk POOLROUTER_ENV=dev (jangan dipakai di staging atau prod)" >&2
		openssl req -x509 -newkey rsa:2048 -nodes -days 3650 -subj "/CN=poolrouter-postgres" \
			-keyout "$TLS_DIR/server.key" -out "$TLS_DIR/server.crt" >/dev/null 2>&1 \
			|| die "gagal membuat sertifikat TLS"
		chmod 0600 "$TLS_DIR/server.key"
	fi
else
	die "sertifikat TLS wajib untuk POOLROUTER_ENV=${POOLROUTER_ENV:-kosong}: pasang secret pg_server_crt dan pg_server_key"
fi

# pgBackRest menolak variabel yang ada tetapi kosong, jadi retensi diferensial hanya diekspor bila terisi.
if [ -n "${PR_REPO1_RETENTION_DIFF:-}" ]; then
	export PGBACKREST_REPO1_RETENTION_DIFF="$PR_REPO1_RETENTION_DIFF"
fi

if [ "${1:-}" = postgres ]; then
	/usr/local/bin/pr-backup-scheduler.sh &
fi

exec docker-entrypoint.sh "$@"
