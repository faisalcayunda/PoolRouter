#!/bin/sh
# Kunci build berat per host untuk toolchain Podman (SHAA-194, K-41).
#
# Kunci = nama container tetap (BUILD_LOCK_NAME, default shaal-build-heavy). Podman menolak nama
# container ganda secara atomik, jadi hanya satu build berat yang jalan di VM. Container dibuat
# dengan --rm, sehingga kunci lepas sendiri saat container selesai. Tidak ada file kunci.
#
# Pemakaian (dipanggil dari Makefile, lihat scripts/build-lock.mk):
#   sh scripts/build-lock.sh run -- podman run ... --name shaal-build-heavy --label shaal.build=heavy ...
#   sh scripts/build-lock.sh status
#   sh scripts/build-lock.sh clean    # hapus kunci basi saja (container berhenti, belum terhapus)
#
# Exit code:
#   - run: exit code perintah yang dibungkus. Bila kunci dipegang, run mengantre (FIFO) dan menunggu
#     sampai BUILD_LOCK_WAIT_SECS (default 1800 = 30 menit); habis waktu: exit 75 (EX_TEMPFAIL)
#     dengan pesan yang menyebut shaal.issue pemegang. Env: BUILD_LOCK_WAIT_SECS, BUILD_LOCK_POLL_SECS,
#     BUILD_LOCK_QUEUE_DIR.
#   - clean: 0 bila tidak ada kunci basi atau sudah dihapus; 1 bila kunci masih dipegang container jalan.
set -u

NAME="${BUILD_LOCK_NAME:-shaal-build-heavy}"
ENGINE="${BUILD_LOCK_ENGINE:-podman}"
EX_TEMPFAIL=75

# inspect_field <format>: nilai field dari container kunci, kosong bila tidak ada.
inspect_field() {
	"$ENGINE" inspect --format "$1" "$NAME" 2>/dev/null | head -n 1
}

# holder_text: ringkasan pemegang kunci untuk pesan ke pengguna.
holder_text() {
	issue=$(inspect_field '{{index .Config.Labels "shaal.issue"}}')
	state=$(inspect_field '{{.State.Status}}')
	case "$issue" in ""|"<no value>") issue="tidak berlabel" ;; esac
	printf 'container %s (shaal.issue=%s, status=%s)' "$NAME" "$issue" "${state:-?}"
}

# remove_stale: hapus kunci basi. Hanya container berlabel shaal.build=heavy yang sudah berhenti.
# Tanpa -f, jadi container yang masih jalan (termasuk build yang baru mulai) tidak pernah disentuh.
remove_stale() {
	label=$(inspect_field '{{index .Config.Labels "shaal.build"}}')
	state=$(inspect_field '{{.State.Status}}')
	[ "$label" = "heavy" ] || return 1
	case "$state" in
		exited|stopped|dead) ;;
		*) return 1 ;;
	esac
	"$ENGINE" rm "$NAME" >/dev/null 2>&1
}

# attempt <cmd...>: jalankan perintah, tampilkan outputnya (juga ke $WD/log), exit code ke $WD/rc.
attempt() {
	( "$@" 2>&1; echo $? >"$WD/rc" ) | tee "$WD/log"
	rc=$(cat "$WD/rc")
}

# --- Antrean FIFO (SHAA-395) ---
# Run yang menemui kunci terpegang tidak gagal seketika: ia mengambil tiket di antrean bersama host
# (BUILD_LOCK_QUEUE_DIR), lalu menunggu sampai tiketnya paling depan dan kunci container bebas.
# Nama tiket = <nanodetik>-<pid>, jadi urutan nama = urutan datang. Tiket milik proses yang sudah mati
# dibuang otomatis, jadi run yang di-kill tidak menyumbat antrean. Kunci container tetap dipegang
# hanya oleh container build (--rm), seperti sebelumnya; antrean hanya mengatur siapa yang boleh
# mencoba berikutnya. Kunci basi tetap dibersihkan oleh remove_stale (tidak diubah, lihat SHAA-366).
QUEUE="${BUILD_LOCK_QUEUE_DIR:-${TMPDIR:-/tmp}/shaal-build-lock-queue}"
WAIT_SECS="${BUILD_LOCK_WAIT_SECS:-1800}"
POLL_SECS="${BUILD_LOCK_POLL_SECS:-2}"

now_secs() { date +%s; }

# queue_head: nama tiket paling depan yang prosesnya masih hidup (tiket mati dibuang).
queue_head() {
	for t in $(ls "$QUEUE" 2>/dev/null | sort); do
		pid=${t##*-}
		if kill -0 "$pid" 2>/dev/null; then
			echo "$t"
			return 0
		fi
		rm -f "$QUEUE/$t"
	done
	return 1
}

queue_len() { ls "$QUEUE" 2>/dev/null | wc -l | tr -d ' '; }

is_conflict() {
	[ "$rc" = 125 ] && grep -Eq "(container name \"/?$NAME\" is already in use|The container name \"/?$NAME\" is already in use)" "$WD/log"
}

# try_once <cmd...>: 0 bila perintah sempat jalan (exit code di $rc), 1 bila kunci terpegang.
try_once() {
	if "$ENGINE" container inspect "$NAME" >/dev/null 2>&1; then
		remove_stale || return 1
		echo "build-lock: kunci basi dihapus." >&2
	fi
	attempt "$@"
	if is_conflict; then
		return 1
	fi
	return 0
}

cmd_run() {
	[ "${1:-}" = "--" ] && shift
	[ $# -gt 0 ] || { echo "build-lock: perintah kosong" >&2; return 2; }
	mkdir -p .cache
	mkdir -p "$QUEUE" && chmod 1777 "$QUEUE" 2>/dev/null
	# Direktori kerja unik per run: dua run dari worktree yang sama tidak saling menimpa.
	WD=$(mktemp -d .cache/build-lock.XXXXXX) || return 1
	TICKET="$QUEUE/$(date +%s%N)-$$"
	: >"$TICKET"
	trap 'rm -rf "$WD"; rm -f "$TICKET"' EXIT
	trap 'exit 130' INT TERM
	deadline=$(( $(now_secs) + WAIT_SECS ))
	announced=0
	while :; do
		head=$(queue_head || true)
		if [ "$head" = "${TICKET##*/}" ]; then
			if try_once "$@"; then
				return "$rc"
			fi
		fi
		if [ "$(now_secs)" -ge "$deadline" ]; then
			echo "build-lock: batas tunggu ${WAIT_SECS} detik habis; kunci dipegang $(holder_text), antrean $(queue_len) run. Tidak ada build yang dijalankan." >&2
			return "$EX_TEMPFAIL"
		fi
		if [ "$announced" -le "$(now_secs)" ]; then
			echo "build-lock: menunggu giliran (kunci dipegang $(holder_text); antrean $(queue_len) run; batas ${WAIT_SECS} detik)." >&2
			announced=$(( $(now_secs) + 60 ))
		fi
		sleep "$POLL_SECS"
	done
}

cmd_status() {
	if "$ENGINE" container inspect "$NAME" >/dev/null 2>&1; then
		echo "dipegang: $(holder_text)"
	else
		echo "bebas: tidak ada container $NAME"
	fi
}

cmd_clean() {
	if ! "$ENGINE" container inspect "$NAME" >/dev/null 2>&1; then
		echo "build-lock: tidak ada kunci"
		return 0
	fi
	if remove_stale; then
		echo "build-lock: kunci basi dihapus ($NAME)"
		return 0
	fi
	echo "build-lock: kunci masih dipegang $(holder_text); tidak dihapus" >&2
	return 1
}

case "${1:-}" in
	run) shift; cmd_run "$@" ;;
	status) cmd_status ;;
	clean) cmd_clean ;;
	*) echo "pemakaian: build-lock.sh run -- <perintah> | status | clean" >&2; exit 2 ;;
esac
