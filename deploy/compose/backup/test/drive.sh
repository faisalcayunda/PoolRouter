#!/bin/sh
# Uji penjadwal tanpa postgres dan tanpa Docker (SHAA-381, review Ridley). pgbackrest, pg_isready, dan psql
# diganti stub di stubs/. Jam dikendalikan lewat PR_BACKUP_CLOCK_FILE. Skenario:
#   1. start 17:59:50 tidak menjalankan slot 17;
#   2. 18:00 full berjalan 90 menit (simulasi); verify 19:00 tetap jalan setelah full selesai;
#   3. lompat 19:00 ke 23:10: slot 20, 21, 22 dicatat sebagai dilewati;
#   4. 00:05: diferensial.
# Pemakaian: sh deploy/compose/backup/test/drive.sh   (exit 0 bila skenario selesai)
set -eu
T=$(cd "$(dirname "$0")" && pwd)
PRS=$(cd "$T/.." && pwd)/pr-backup-scheduler.sh
export PATH="$T/stubs:$PATH" STUB_LOG="$T/stub.log" PGBACKREST_STANZA=poolrouter POSTGRES_USER=u POSTGRES_DB=d
export PR_BACKUP_CLOCK_FILE="$T/clock" PR_BACKUP_TICK=0.05 STUB_FULL_SECS=5400
cd "$T"; rm -f stub.log sched.log
ep() { date -u -d "$1" +%s; }
echo "$(ep "2026-11-04 17:59:50")" > clock
sh "$PRS" 2> sched.log &
PID=$!
wait_for() { i=0; until grep -q -e "$1" "$2" 2>/dev/null; do sleep 0.1; i=$((i + 1)); if [ "$i" -ge 300 ]; then echo "TIMEOUT: $1"; kill "$PID" 2>/dev/null; exit 1; fi; done; }
wait_for "mulai: slot" sched.log
echo "$(ep "2026-11-04 18:00:10")" > clock
wait_for "--type=full backup" stub.log
wait_for "pgbackrest --stanza=poolrouter verify" stub.log
echo "$(ep "2026-11-04 23:10:00")" > clock
wait_for "slot 22 UTC dilewati" sched.log
echo "$(ep "2026-11-05 00:05:00")" > clock
wait_for "--type=diff backup" stub.log
kill "$PID" 2>/dev/null || true; wait "$PID" 2>/dev/null || true
grep -q "slot 17" sched.log && { echo "GAGAL: slot 17 tidak boleh jalan"; exit 1; }
echo "=== stub.log"; cat stub.log
echo "=== sched.log"; cat sched.log
echo "SKENARIO LULUS"
