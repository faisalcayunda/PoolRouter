#!/bin/sh
# Penjadwal backup pgBackRest (ADR-0005 §4.5, SHAA-206 §5). Dijalankan di latar belakang oleh pr-pg-entrypoint.sh.
#   - stanza-create dan check saat start (diulang sampai berhasil);
#   - full harian 01.00 WIB (18.00 UTC), diferensial tiap 6 jam (00, 06, 12 UTC);
#   - pgbackrest verify harian 02.00 WIB (19.00 UTC).
# Slot dihitung per jam UTC, tidak per menit: bila loop tertunda (mis. full berjalan lama) slot jam
# berikutnya tetap dijalankan, dan setiap jam yang terlewat dicatat. Verify yang terlewat dikejar.
# Arsip WAL dilakukan oleh archive_command di postgres, bukan di sini.
# Semua keluaran ke stderr agar terlihat di `compose logs`.
#
# Variabel uji (tidak di-set di Compose): PR_BACKUP_TICK (detik antar pemeriksaan, bawaan 30),
# PR_BACKUP_CLOCK_FILE (berisi epoch UTC, menggantikan jam sistem).
set -u

STANZA="${PGBACKREST_STANZA:-poolrouter}"
FULL_HOUR_UTC="${PR_BACKUP_FULL_HOUR_UTC:-18}"
DIFF_EVERY_HOURS="${PR_BACKUP_DIFF_EVERY_HOURS:-6}"
VERIFY_HOUR_UTC="${PR_BACKUP_VERIFY_HOUR_UTC:-19}"
# daily (jadwal SHAA-206) atau hari ISO 1-7 untuk full mingguan (jadwal hemat, ADR-0005 §5.2).
FULL_WEEKDAY="${PR_BACKUP_FULL_WEEKDAY:-daily}"
TICK="${PR_BACKUP_TICK:-30}"
CLOCK_FILE="${PR_BACKUP_CLOCK_FILE:-}"
SOCKET_DIR=/var/run/postgresql

log() { echo "pr-backup $(date -u +%Y-%m-%dT%H:%M:%SZ) $*" >&2; }

now_epoch() {
	if [ -n "$CLOCK_FILE" ]; then cat "$CLOCK_FILE"; else date -u +%s; fi
}

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

verify_now() {
	if pgbackrest --stanza="$STANZA" verify; then log "verify sukses"; else log "verify GAGAL rc=$?"; fi
}

# run_slot <jam UTC> <hari ISO>: tugas untuk satu jam UTC.
run_slot() {
	h=$1; wd=$2
	if [ "$h" -eq "$FULL_HOUR_UTC" ] && { [ "$FULL_WEEKDAY" = daily ] || [ "$wd" -eq "$FULL_WEEKDAY" ]; }; then
		run_backup full
	elif [ $((h % DIFF_EVERY_HOURS)) -eq 0 ]; then
		run_backup diff
	fi
	if [ "$h" -eq "$VERIFY_HOUR_UTC" ]; then verify_now; fi
}

wait_db
until pgbackrest --stanza="$STANZA" stanza-create; do
	log "stanza-create gagal, coba lagi 30 detik"
	sleep 30
done
pgbackrest --stanza="$STANZA" check || log "check gagal (archive belum berjalan?), penjadwal tetap jalan"

# Jam saat start tidak dijalankan ulang: start di tengah jam tidak mengulang slot jam itu. Dicatat.
cur=$(now_epoch)
last_start=$((cur - cur % 3600))
log "mulai: slot jam $(date -u -d "@$cur" +%Y-%m-%dT%H)Z tidak dijalankan ulang saat start"

while :; do
	cur=$(now_epoch)
	start=$((cur - cur % 3600))
	if [ "$start" -gt "$last_start" ]; then
		s=$((last_start + 3600))
		while [ "$s" -lt "$start" ]; do
			sh_=$(date -u -d "@$s" +%-H)
			log "slot $sh_ UTC dilewati (loop tertunda)"
			if [ "$sh_" -eq "$VERIFY_HOUR_UTC" ]; then verify_now; fi
			s=$((s + 3600))
		done
		run_slot "$(date -u -d "@$cur" +%-H)" "$(date -u -d "@$cur" +%u)"
		last_start=$start
	fi
	sleep "$TICK"
done
