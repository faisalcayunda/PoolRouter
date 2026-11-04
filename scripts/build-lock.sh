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
#   - run: exit code perintah yang dibungkus, kecuali kunci dipegang: 75 (EX_TEMPFAIL).
#     Exit 75 hanya bila podman melaporkan bentrok nama container.
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

# is_name_conflict: bentrok nama container. Podman: container name "X" is already in use;
# Docker: Conflict. The container name "/X" is already in use (SHAA-390: tanpa ini Docker keluar 125, bukan 75).
is_name_conflict() {
	grep -Eq "container name \"/?$NAME\" is already in use" "$WD/log"
}

# attempt <cmd...>: jalankan perintah, tampilkan outputnya (juga ke $WD/log), exit code ke $WD/rc.
attempt() {
	( "$@" 2>&1; echo $? >"$WD/rc" ) | tee "$WD/log"
	rc=$(cat "$WD/rc")
}

cmd_run() {
	[ "${1:-}" = "--" ] && shift
	[ $# -gt 0 ] || { echo "build-lock: perintah kosong" >&2; return 2; }
	mkdir -p .cache
	# Direktori kerja unik per run: dua run dari worktree yang sama tidak saling menimpa.
	WD=$(mktemp -d .cache/build-lock.XXXXXX) || return 1
	trap 'rm -rf "$WD"' EXIT
	attempt "$@"
	if [ "$rc" = 125 ] && is_name_conflict; then
		if remove_stale; then
			echo "build-lock: kunci basi dihapus, mencoba sekali lagi." >&2
			attempt "$@"
			if [ "$rc" = 125 ] && is_name_conflict; then
				echo "build-lock: kunci dipegang $(holder_text). Jalankan pekerjaan non-compile, atau tunggu sampai container itu selesai." >&2
				rc=$EX_TEMPFAIL
			fi
		else
			echo "build-lock: kunci dipegang $(holder_text). Jalankan pekerjaan non-compile, atau tunggu sampai container itu selesai." >&2
			rc=$EX_TEMPFAIL
		fi
	fi
	return "$rc"
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
