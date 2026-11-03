#!/bin/sh
# Guard dan penunjuk path untuk build cache Cargo di SSD eksternal (SHAA-274).
#
# Pemakaian:
#   CARGO_TARGET_DIR="$(sh scripts/cargo-target-ssd.sh)" cargo build ...
#
# Env wajib: POOLROUTER_CARGO_CACHE = folder akar cache di SSD, diisi owner di env agent
# (bukan di repo). Contoh nilai: "/Volumes/<nama SSD>/shaal-cache/poolrouter".
# Env opsional: POOLROUTER_CARGO_MIN_FREE_KIB (default 10 GiB) = sisa ruang minimum.
#
# Script ini TIDAK membuat folder akar. Kalau SSD tidak ter-mount, folder akar tidak ada,
# dan membuatnya di disk utama akan mengisi disk utama lagi. Owner yang membuat folder akar.
# Script juga tidak membuat folder target; cargo yang membuatnya saat build.
#
# Output: path target di stdout. Exit code:
#   0  siap dipakai
#   2  POOLROUTER_CARGO_CACHE kosong atau bukan path absolut
#   3  folder akar tidak ada (SSD tidak ter-mount, atau folder belum dibuat owner)
#   4  folder akar berada di disk boot (bukan SSD)
#   5  sisa ruang di bawah batas minimum
set -u

ROOT="${POOLROUTER_CARGO_CACHE:-}"
MIN_FREE_KIB="${POOLROUTER_CARGO_MIN_FREE_KIB:-10485760}"

fail() { echo "cargo-target-ssd: $2" >&2; exit "$1"; }

# disk_of <path>: identifier disk fisik (misal disk3 dari /dev/disk3s1s1). Kosong bila tidak terbaca.
disk_of() {
	df -P -k "$1" 2>/dev/null | awk 'NR==2 {print $1}' \
		| sed -e 's#^/dev/##' -e 's/s[0-9][0-9]*$//' -e 's/s[0-9][0-9]*$//'
}

[ -n "$ROOT" ] || fail 2 "POOLROUTER_CARGO_CACHE belum di-set. Lihat docs/runbook/cargo-target-ssd.md."
case "$ROOT" in
	/*) ;;
	*) fail 2 "POOLROUTER_CARGO_CACHE harus path absolut." ;;
esac
if [ ! -d "$ROOT" ]; then
	fail 3 "$ROOT tidak ada. SSD belum ter-mount, atau folder akar belum dibuat owner. Script tidak membuat folder ini."
fi

REAL=$(realpath "$ROOT") || fail 2 "realpath gagal untuk $ROOT. Laporkan sebagai blocker."

# Folder akar harus di disk fisik lain selain disk boot. Di macOS, / dan Data volume
# (/System/Volumes/Data, tempat /tmp dan /Users berada) adalah partisi berbeda pada disk fisik
# yang sama, jadi yang dibandingkan adalah identifier disk, bukan device partisinya.
DISK=$(disk_of "$REAL")
BOOT_DISK=$(disk_of /)
DATA_DISK=$(disk_of /System/Volumes/Data)
if [ -z "$DISK" ] || [ "$DISK" = "$BOOT_DISK" ] || [ "$DISK" = "$DATA_DISK" ]; then
	fail 4 "$REAL ada di disk boot (${BOOT_DISK:-?}). SSD belum ter-mount; build dibatalkan agar disk utama tidak terisi."
fi

FREE_KIB=$(df -P -k "$REAL" | awk 'NR==2 {print $4}')
case "$FREE_KIB" in
	''|*[!0-9]*) fail 5 "tidak bisa membaca sisa ruang di $REAL." ;;
esac
if [ "$FREE_KIB" -lt "$MIN_FREE_KIB" ]; then
	fail 5 "sisa ruang $FREE_KIB KiB di $REAL di bawah batas $MIN_FREE_KIB KiB. Jalankan pembersihan di runbook dulu."
fi

printf '%s/target\n' "$REAL"
