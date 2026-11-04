#!/bin/sh
# Penjadwal backup pgBackRest (ADR-0005 §4.5, SHAA-206 §5). Dijalankan di latar belakang oleh pr-pg-entrypoint.sh.
#   - stanza-create dan check saat start (diulang sampai berhasil);
#   - full harian 01.00 WIB (18.00 UTC), diferensial tiap 6 jam (00, 06, 12 UTC);
#   - pgbackrest verify harian 02.00 WIB (19.00 UTC).
# Arsip WAL dilakukan oleh archive_command di postgres, bukan di sini.
# Semua keluaran ke stderr agar terlihat di `compose logs`.
set -u

STANZA="${PGBACKREST_STANZA:-poolrouter}"
FULL_HOUR_UTC="${PR_BACKUP_FULL_HOUR_UTC:-18}"
DIFF_EVERY_HOURS="${PR_BACKUP_DIFF_EVERY_HOURS:-6}"
VERIFY_HOUR_UTC="${PR_BACKUP_VERIFY_HOUR_UTC:-19}"
# daily (jadwal SHAA-206) atau hari ISO 1-7 untuk full mingguan (jadwal hemat, ADR-0005 §5.2).
FULL_WEEKDAY="${PR_BACKUP_FULL_WEEKDAY:-daily}"
SOCKET_DIR=/var/run/postgresql

log() { echo "pr-backup $(date -u +%Y-%m-%dT%H:%M:%SZ) $*" >&2; }

wait_db() {
	# pg_isready saja tidak cukup: selama initdb ada server sementara di socket yang sama.
	until pg_isready -q -h "$SOCKET_DIR" -U "$POSTGRES_USER" -d "$POSTGRES_DB" \
		&& psql -q -h "$SOCKET_DIR" -U "$POSTGRES_USER" -d "$POSTGRES_DB" -c 'SELECT 1' >/dev/null 2>&1; do
		sleep 5
	done
}

run_backup() {
	log "backup $1 mulai"
	if pgbackrest --stanza="$STANZA" --type="$1" backup; then
		log "backup $1 sukses"
	else
		log "backup $1 GAGAL rc=$?"
	fi
}

wait_db
until pgbackrest --stanza="$STANZA" stanza-create; do
	log "stanza-create gagal, coba lagi 30 detik"
	sleep 30
done
pgbackrest --stanza="$STANZA" check || log "check gagal (archive belum berjalan?), penjadwal tetap jalan"

last=""
while :; do
	key=$(date -u +%Y%m%d%H)
	minute=$(date -u +%M)
	if [ "$minute" = 00 ] && [ "$key" != "$last" ]; then
		last=$key
		h=$(date -u +%-H)
		if [ "$h" -eq "$FULL_HOUR_UTC" ] && { [ "$FULL_WEEKDAY" = daily ] || [ "$(date -u +%u)" -eq "$FULL_WEEKDAY" ]; }; then
			run_backup full
		elif [ $((h % DIFF_EVERY_HOURS)) -eq 0 ]; then
			run_backup diff
		fi
		if [ "$h" -eq "$VERIFY_HOUR_UTC" ]; then
			if pgbackrest --stanza="$STANZA" verify; then log "verify sukses"; else log "verify GAGAL rc=$?"; fi
		fi
	fi
	sleep 30
done
