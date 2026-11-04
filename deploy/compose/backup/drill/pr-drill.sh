#!/bin/sh
# Alat drill backup dan PITR (ADR-0005 §4.5, SHAA-206 §7). Shell + psql + pgbackrest di image PostgreSQL;
# tanpa runtime bahasa baru. Data hanya sintetis (tabel drill_sentinel berisi md5 acak).
#
# Pemakaian (dari akar repo, lihat docs/runbook/backup-restore.md):
#   sh deploy/compose/backup/drill/pr-drill.sh <perintah> [argumen]
#
# Perintah:
#   sentinel-init  <kontainer>                      buat tabel drill_sentinel (idempoten)
#   sentinel-write <kontainer> <detik> <log>        tulis 1 baris tiap ~1 detik; catat "id ts_server" per commit yang sukses
#   checksum       <kontainer> [<ts_target>]        cetak jumlah baris, max(ts), dan md5 baris dengan ts <= target
#   restore        <nama-volume-data> <ts_target|-> restore dari repo ke volume baru (PITR bila ts diisi, default bila "-")
#   serve          <nama-volume-data> <kontainer>   nyalakan postgres dari volume hasil restore (tanpa scheduler, tanpa archive)
#   rpo            <log> <max_ts_pulih>             selisih commit terakhir yang di-ACK di log dengan max_ts yang pulih (detik)
#
# Variabel (wajib dari env file di luar repo): ENGINE (docker|podman), DRILL_IMAGE, DRILL_LABEL (mis. shaal.issue=SHAA-381),
# DRILL_NETWORK (jaringan kosong, mis. none untuk restore), REPO_VOLUME (volume repo pgBackRest),
# SECRETS_DIR (dir secret), POSTGRES_USER, POSTGRES_DB, PGBACKREST_STANZA.
set -eu

ENGINE=${ENGINE:-docker}
DRILL_IMAGE=${DRILL_IMAGE:-localhost/poolrouter-postgres:dev}
: "${DRILL_LABEL:?isi DRILL_LABEL, mis. shaal.issue=SHAA-381}"
: "${REPO_VOLUME:?isi REPO_VOLUME (volume repo pgBackRest)}"
: "${SECRETS_DIR:?isi SECRETS_DIR}"
POSTGRES_USER=${POSTGRES_USER:-poolrouter_admin}
POSTGRES_DB=${POSTGRES_DB:-poolrouter}
PGBACKREST_STANZA=${PGBACKREST_STANZA:-poolrouter}
PSQL="psql -X -q -v ON_ERROR_STOP=1 -U $POSTGRES_USER -d $POSTGRES_DB"

die() { echo "pr-drill: $*" >&2; exit 1; }

# Tabel dan query dijalankan lewat psql di dalam kontainer (socket unix, pengguna lokal).
sql() { # sql <kontainer> <query>
	$ENGINE exec -i "$1" sh -c "$PSQL -At" <<EOQ
$2
EOQ
}

sentinel_init() {
	sql "$1" 'CREATE TABLE IF NOT EXISTS drill_sentinel (
  id bigserial PRIMARY KEY,
  ts timestamptz NOT NULL DEFAULT clock_timestamp(),
  payload text NOT NULL
);' >/dev/null
	echo "drill_sentinel siap di $1"
}

sentinel_write() { # sentinel_write <kontainer> <detik> <log>
	n=$2; log=$3; i=0
	: > "$log"
	while [ "$i" -lt "$n" ]; do
		# RETURNING clock_timestamp(): waktu commit di server, bukan jam klien. Baris log hanya setelah psql sukses.
		out=$(sql "$1" "INSERT INTO drill_sentinel (payload) VALUES (md5(random()::text)) RETURNING id || ' ' || ts;" 2>/dev/null) \
			&& echo "$out" >> "$log" || echo "GAGAL-INSERT $(date -u +%FT%T.%3NZ)" >> "$log"
		i=$((i + 1))
		sleep 1
	done
}

checksum() { # checksum <kontainer> [<ts_target>]
	if [ -n "${2:-}" ]; then where="WHERE ts <= '$2'"; else where=""; fi
	sql "$1" "SELECT 'rows=' || count(*) || ' max_ts=' || coalesce(max(ts)::text,'-') || ' md5=' || coalesce(md5(string_agg(id || ':' || payload, ',' ORDER BY id)),'-') FROM drill_sentinel $where;"
}

# Kredensial repo hanya di proses, dibaca dari secret (pola KD-1). Dipakai oleh restore dan serve.
repo_env() {
	cat <<EOR
-e PGBACKREST_STANZA=$PGBACKREST_STANZA -e PGBACKREST_PG1_PATH=/var/lib/postgresql/data -e PGBACKREST_REPO1_TYPE=posix -e PGBACKREST_REPO1_PATH=/var/lib/pgbackrest -e PGBACKREST_REPO1_CIPHER_TYPE=aes-256-cbc -e PGBACKREST_SPOOL_PATH=/var/spool/pgbackrest -e PGBACKREST_LOG_PATH=/var/log/pgbackrest -e PGBACKREST_CONFIG=/etc/pgbackrest/pgbackrest.conf
EOR
}

restore() { # restore <volume-data> <ts_target|->
	vol=$1; tgt=$2
	[ -n "$tgt" ] || die "isi ts_target atau '-'"
	# Opsi dibangun sebagai daftar argumen, bukan string, agar nilai waktu dengan spasi tetap utuh.
	if [ "$tgt" = "-" ]; then
		set -- --type=default
	else
		set -- --type=time "--target=$tgt" --target-action=promote
	fi
	$ENGINE volume create --label "$DRILL_LABEL" "$vol" >/dev/null
	$ENGINE run --rm --name "shaa381-restore-$(echo "$vol" | tr -c 'a-zA-Z0-9' '-' | cut -c1-40)" \
		--label "$DRILL_LABEL" --network "${DRILL_NETWORK:-none}" \
		-v "$vol:/var/lib/postgresql/data" \
		-v "$REPO_VOLUME:/var/lib/pgbackrest" \
		-v "$SECRETS_DIR/pgbackrest_repo1_cipher_pass:/run/secrets/pgbackrest_repo1_cipher_pass:ro" \
		$(repo_env) \
		--entrypoint /bin/sh "$DRILL_IMAGE" -c \
		'set -eu; export PGBACKREST_REPO1_CIPHER_PASS="$(cat /run/secrets/pgbackrest_repo1_cipher_pass)"; pgbackrest --stanza="$PGBACKREST_STANZA" "$@" restore' \
		pr-drill "$@"
	echo "restore $* selesai ke volume $vol"
}

serve() { # serve <volume-data> <kontainer>
	vol=$1; name=$2
	# Tanpa scheduler dan tanpa archive_mode: restore_command dari recovery tetap membaca repo lewat archive-get.
	$ENGINE run -d --name "$name" --label "$DRILL_LABEL" --network "${DRILL_NETWORK:-none}" \
		-v "$vol:/var/lib/postgresql/data" \
		-v "$REPO_VOLUME:/var/lib/pgbackrest" \
		-v "$SECRETS_DIR/pgbackrest_repo1_cipher_pass:/run/secrets/pgbackrest_repo1_cipher_pass:ro" \
		$(repo_env) \
		--entrypoint /bin/sh "$DRILL_IMAGE" -c \
		'set -eu; export PGBACKREST_REPO1_CIPHER_PASS="$(cat /run/secrets/pgbackrest_repo1_cipher_pass)"; exec /usr/lib/postgresql/17/bin/postgres -D /var/lib/postgresql/data -c archive_mode=off -c listen_addresses= -k /var/run/postgresql' >/dev/null
	i=0
	until $ENGINE exec "$name" pg_isready -q -h /var/run/postgresql -U "$POSTGRES_USER" -d "$POSTGRES_DB" 2>/dev/null; do
		i=$((i + 1)); [ "$i" -lt 300 ] || die "postgres hasil restore tidak siap dalam 300 detik"; sleep 1
	done
	date -u +"siap %FT%T.%3NZ"
}

rpo() { # rpo <log> <max_ts_pulih>
	last=$(grep -v '^GAGAL' "$1" | tail -n 1 | cut -d' ' -f2-)
	[ -n "$last" ] || die "log kosong: $1"
	a=$(date -d "$last" +%s.%N); b=$(date -d "$2" +%s.%N)
	echo "commit terakhir di-ACK: $last"
	echo "max_ts pulih          : $2"
	echo "RPO (detik)           : $(awk -v a="$a" -v b="$b" 'BEGIN { printf "%.3f", a - b }')"
}

cmd=${1:-}; [ $# -gt 0 ] && shift
case "$cmd" in
	sentinel-init) sentinel_init "$@" ;;
	sentinel-write) sentinel_write "$@" ;;
	checksum) checksum "$@" ;;
	restore) restore "$@" ;;
	serve) serve "$@" ;;
	rpo) rpo "$@" ;;
	*) sed -n '2,20p' "$0" >&2; exit 2 ;;
esac
