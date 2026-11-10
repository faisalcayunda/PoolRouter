#!/bin/sh
# Uji image backend memuat semua biner, termasuk backend-cli (ARC-236, ADR-0003 §3.6c).
#
# Dua tingkat:
#   1. Statis (selalu jalan, tanpa engine): backend/Containerfile membangun (`--bin`), menyalin ke /out
#      (`cp`), dan memasang ke /usr/local/bin (`COPY --from=build`) setiap biner yang diharapkan, dan
#      nama biner di crates/cli/Cargo.toml sama dengan yang dibangun.
#   2. Image (hanya bila IMAGE diisi): setiap biner ada dan executable di /usr/local/bin, dan
#      `backend-cli --help` berjalan di dalam image dengan exit 0. Image dibangun pemanggil
#      (make image, atau langkah build di workflow backend-image).
#
# Jalankan dari akar repo:
#   make test-image-cli                                              (statis)
#   make test-image-cli IMAGE=localhost/poolrouter-backend:dev       (statis + image)
# Variabel: CONTAINER_ENGINE (default podman; boleh "sudo podman"), CONTAINER_LABEL (label container uji).
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
cf="$root/backend/Containerfile"
cli_toml="$root/backend/crates/cli/Cargo.toml"
BINS="webserver controller worker backend-cli"
fail=0

ok() { echo "lulus: $1"; }
bad() { echo "GAGAL: $1"; fail=1; }

# --- 1. Containerfile -------------------------------------------------------
if [ ! -f "$cf" ]; then
	bad "backend/Containerfile tidak ada"
	exit 1
fi
build_line=$(grep -E '^[[:space:]]*(RUN .*)?cargo build ' "$cf" || true)
cp_line=$(grep -E '(^|[[:space:]])cp target/release/' "$cf" || true)
copy_line=$(grep -E '^COPY --from=build ' "$cf" || true)
[ -n "$build_line" ] || bad "baris cargo build tidak ditemukan di Containerfile"
[ -n "$cp_line" ] || bad "baris cp target/release/ tidak ditemukan di Containerfile"
[ -n "$copy_line" ] || bad "baris COPY --from=build tidak ditemukan di Containerfile"

for b in $BINS; do
	if printf '%s\n' "$build_line" | grep -Eq -- "--bin $b( |\\\\|\$)"; then
		ok "cargo build --bin $b"
	else
		bad "cargo build tidak memuat --bin $b"
	fi
	if printf '%s\n' "$cp_line" | grep -Eq "target/release/$b( |\$)"; then
		ok "cp target/release/$b ke /out"
	else
		bad "cp tidak menyalin target/release/$b ke /out"
	fi
	if printf '%s\n' "$copy_line" | grep -Eq "/out/$b( |\$)"; then
		ok "COPY --from=build /out/$b ke /usr/local/bin"
	else
		bad "COPY --from=build tidak memasang /out/$b"
	fi
done

# Biner baru tidak boleh lolos tanpa uji: setiap --bin di Containerfile harus ada di daftar BINS.
for b in $(printf '%s\n' "$build_line" | grep -o -- '--bin [A-Za-z0-9_-]*' | awk '{print $2}'); do
	case " $BINS " in
	*" $b "*) ;;
	*) bad "Containerfile membangun --bin $b yang belum ada di daftar uji (scripts/test-image-cli.sh)" ;;
	esac
done

if grep -Eq '^name = "backend-cli"$' "$cli_toml"; then
	ok "crates/cli/Cargo.toml mendefinisikan [[bin]] backend-cli"
else
	bad "crates/cli/Cargo.toml tidak mendefinisikan [[bin]] name = \"backend-cli\""
fi

# --- 2. Image (opsional) ----------------------------------------------------
if [ -z "${IMAGE:-}" ]; then
	echo "lewati: IMAGE kosong, pemeriksaan di dalam image tidak dijalankan"
	exit "$fail"
fi

engine=${CONTAINER_ENGINE:-podman}
label_arg=
if [ -n "${CONTAINER_LABEL:-}" ]; then label_arg="--label $CONTAINER_LABEL"; fi
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# $engine dan $label_arg sengaja tanpa kutip: boleh berisi beberapa kata ("sudo podman", "--label k=v").
# shellcheck disable=SC2086
if $engine run --rm $label_arg --entrypoint sh "$IMAGE" -c \
	'for b in '"$BINS"'; do [ -x "/usr/local/bin/$b" ] || { echo "tidak ada: $b" >&2; exit 1; }; done' \
	>"$tmp/ada.out" 2>&1; then
	ok "image: $BINS ada dan executable di /usr/local/bin"
else
	bad "image: biner hilang"; cat "$tmp/ada.out"
fi

# --help diurai sebelum konfigurasi dibaca, jadi jalan tanpa env dan tanpa basis data.
# shellcheck disable=SC2086
if $engine run --rm $label_arg --entrypoint backend-cli "$IMAGE" --help >"$tmp/help.out" 2>"$tmp/help.err"; then
	if grep -q '^pemakaian: backend-cli' "$tmp/help.out"; then
		ok "image: backend-cli --help exit 0 dan mencetak bantuan"
	else
		bad "image: backend-cli --help exit 0 tetapi keluaran bukan bantuan"; cat "$tmp/help.out"
	fi
else
	bad "image: backend-cli --help gagal"; cat "$tmp/help.err"
fi

exit "$fail"
