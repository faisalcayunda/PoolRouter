#!/bin/sh
# Uji reproduksi SHAA-401: target cargo bersama antar-lane Docker tidak boleh membuat satu lane memakai
# artefak atau vonis lint lane lain. Dua "checkout" A dan B berisi crate workspace yang sama (path relatif sama)
# dengan isi berbeda, satu CARGO_TARGET_DIR bersama, mtime sumber B dibuat lebih tua dari build A.
# Pola opsi sama dengan backend/Makefile: RUSTC_WORKSPACE_WRAPPER=/lane/<kunci>/w.sh dan, untuk clippy,
# `--cfg shaal_lane="<kunci>"` (clippy menimpa wrapper dengan clippy-driver).
#
# Pemakaian: sh scripts/uji-isolasi-target.sh      (butuh docker dan image RUST_IMAGE lokal; tanpa jaringan)
# Wajib dijalankan ulang saat RUST_IMAGE (versi cargo) diganti: pengisolasian bergantung pada perilaku cargo.
# Exit 0 bila semua pemeriksaan "sesudah" lulus. Kasus "sebelum" hanya memperlihatkan celahnya (tidak memengaruhi exit).
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
IMG=${RUST_IMAGE:-$(sed -n 's/^RUST_IMAGE ?= \(.*\)$/\1/p' "$root/backend/Makefile" | head -n 1)}
# Toolchain dengan komponen clippy ada di rustup bersama lane (komponen dipasang `make setup` lewat rust-toolchain.toml); image dasar tidak punya clippy.
SHARED_CARGO_HOME=${SHARED_CARGO_HOME:-/data/workspace/shaal-build/cargo-home}
[ -d "$SHARED_CARGO_HOME/rustup/toolchains" ] || { echo "rustup bersama $SHARED_CARGO_HOME/rustup belum ada: jalankan make -C backend setup RUNNER=docker dulu" >&2; exit 2; }
tmp=$(mktemp -d "${TMPDIR:-/tmp}/uji-isolasi.XXXXXX")
# Isi tmp dimiliki uid host (container jalan --user), jadi rm biasa cukup.
trap 'rm -rf "$tmp"' EXIT
fail=0

bikin() { # dir nilai [bad]
	mkdir -p "$1/crates/inti/src" "$1/crates/app/src"
	printf '[workspace]\nmembers=["crates/inti","crates/app"]\nresolver="2"\n' > "$1/Cargo.toml"
	printf '[package]\nname="inti"\nversion="0.1.0"\nedition="2021"\n' > "$1/crates/inti/Cargo.toml"
	if [ "${3:-}" = bad ]; then # pelanggaran clippy::needless_return
		printf 'pub fn nilai() -> u32 { return %s; }\n' "$2" > "$1/crates/inti/src/lib.rs"
	else
		printf 'pub fn nilai() -> u32 { %s }\n' "$2" > "$1/crates/inti/src/lib.rs"
	fi
	printf '[package]\nname="app"\nversion="0.1.0"\nedition="2021"\n[dependencies]\ninti={path="../inti"}\n' > "$1/crates/app/Cargo.toml"
	printf 'fn main(){ println!("{}", inti::nilai()); }\n' > "$1/crates/app/src/main.rs"
	touch -d '2001-01-01 00:00:00' "$1/crates/inti/src/lib.rs" "$1/crates/app/src/main.rs"
}
# cr <target> <dir> <kunci|-> <perintah...>: jalankan di container seperti lane Docker.
cr() {
	t=$1; d=$2; k=$3; shift 3
	if [ "$k" = - ]; then w=""; else
		w="-e RUSTC_WORKSPACE_WRAPPER=/lane/$k/w.sh -v $root/scripts/lane-wrapper.sh:/lane/$k/w.sh:ro"
	fi
	# shellcheck disable=SC2086
	docker run --rm --user "$(id -u):$(id -g)" -v "$t":/cache/target -e CARGO_TARGET_DIR=/cache/target/build \
		-e CARGO_HOME=/tmp/ch -e RUSTUP_HOME=/cache/rustup -v "$SHARED_CARGO_HOME/rustup":/cache/rustup:ro -e CARGO_INCREMENTAL=0 $w -v "$d":/work -w /work "$IMG" "$@"
}
# Dua build berurutan di target bersama; B harus keluar 222.
uji_build() { # label kunciA kunciB dirA dirB
	T="$tmp/t-$1"; mkdir -p "$T"
	bikin "$4" 111; bikin "$5" 222
	touch "$4/crates/app/src/main.rs" # A lebih baru dari B
	a=$(cr "$T" "$4" "$2" cargo run -q --offline -p app); b=$(cr "$T" "$5" "$3" cargo run -q --offline -p app)
	echo "[$1] build: A => $a (harus 111), B => $b (harus 222), du target $(du -sk "$T" | cut -f1) KiB"
	[ "$a" = 111 ] && [ "$b" = 222 ]
}
# A bersih lint dulu, lalu B (pelanggaran, mtime lama); lint B harus gagal. $4 = argumen tambahan clippy.
uji_lint() { # label kunciA kunciB extraA extraB dirA dirB
	T="$tmp/t-$1"; mkdir -p "$T"
	bikin "$6" 111; bikin "$7" 222 bad
	touch "$6/crates/inti/src/lib.rs" "$6/crates/app/src/main.rs"
	# shellcheck disable=SC2086
	cr "$T" "$6" "$2" cargo clippy --offline --workspace -q -- -D warnings $4 >/dev/null 2>&1 || { echo "[$1] lint A seharusnya bersih"; return 2; }
	# shellcheck disable=SC2086
	if cr "$T" "$7" "$3" cargo clippy --offline --workspace -q -- -D warnings $5 >/dev/null 2>&1; then
		echo "[$1] lint: B (ada pelanggaran) LULUS = hijau palsu"; return 1
	fi
	echo "[$1] lint: B gagal seperti seharusnya"; return 0
}
cek() { # label harapan(0|1) perintah...
	l=$1; h=$2; shift 2
	rc=0; "$@" || rc=$?
	if [ "$rc" -eq "$h" ]; then echo "  -> $l: OK"; else echo "  -> $l: GAGAL (rc=$rc, harapan $h)"; fail=1; fi
}

mkdir -p "$tmp/x/A" "$tmp/x/B"
mkdir -p "$tmp/x/A" "$tmp/x/B"
kA=$("$root/scripts/lane-key.sh" shaal.issue=SHAA-901 "$tmp/x/A"); kB=$("$root/scripts/lane-key.sh" shaal.issue=SHAA-901 "$tmp/x/B")
echo "kunci A=$kA B=$kB (label sama, path host beda)"
[ "$kA" != "$kB" ] || { echo "GAGAL: kunci sama untuk path host berbeda"; fail=1; }

echo "== build, tanpa perbaikan (celah): harus tereproduksi =="
cek "sebelum (B memakai artefak A)" 1 uji_build sebelum - - "$tmp/s/A" "$tmp/s/B"
echo "== build, wrapper per lane (label sama, path host beda) =="
cek "sesudah" 0 uji_build sesudah "$kA" "$kB" "$tmp/b/A" "$tmp/b/B"
echo "== lint, hanya wrapper: celah clippy (tereproduksi bila clippy menimpa wrapper) =="
cek "lint tanpa --cfg (celah)" 1 uji_lint lintbocor "$kA" "$kB" "" "" "$tmp/l1/A" "$tmp/l1/B"
echo "== lint, wrapper + --cfg shaal_lane =="
cek "lint dengan --cfg" 0 uji_lint lintcfg "$kA" "$kB" "--cfg shaal_lane=\"$kA\"" "--cfg shaal_lane=\"$kB\"" "$tmp/l2/A" "$tmp/l2/B"
[ "$fail" -eq 0 ] && echo "SEMUA LULUS" || echo "ADA YANG GAGAL"
exit "$fail"
