#!/bin/sh
# Uji image backend memuat semua biner, termasuk backend-cli dan migrate (ARC-236, ARC-254, ADR-0003 §3.6c).
#
# Dua tingkat:
#   1. Statis (selalu jalan, tanpa engine): backend/Containerfile membangun (`--bin`), menyalin ke /out
#      (`cp`), dan memasang ke /usr/local/bin (`COPY --from=build`) setiap biner yang diharapkan, dan
#      nama biner di crates/cli/Cargo.toml sama dengan yang dibangun. Tahap runtime memasang
#      ca-certificates (reqwest 0.13 panik tanpa CA sistem, ARC-254). Perintah service `migrate` di
#      deploy/compose/compose.yaml menunjuk biner yang ada di image, dan setiap bind mount relatif di
#      compose.yaml menunjuk jalur yang ada di repo.
#   2. Image (hanya bila IMAGE diisi): setiap biner ada dan executable di /usr/local/bin,
#      `backend-cli --help` berjalan di dalam image dengan exit 0, `migrate` mulai dan menolak konfigurasi
#      kosong dengan exit 2 (bukan "executable not found"), dan healthcheck webserver, controller, serta
#      liveness worker tanpa jaringan gagal bersih dengan exit 1 (bukan panik 101). Image dibangun
#      pemanggil (make image, atau langkah build di workflow backend-image).
#
# Jalankan dari akar repo:
#   make test-image-cli                                              (statis)
#   make test-image-cli IMAGE=localhost/poolrouter-backend:dev       (statis + image)
# Variabel: CONTAINER_ENGINE (default podman; boleh "sudo podman"), CONTAINER_LABEL (label container uji).
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
cf="$root/backend/Containerfile"
cli_toml="$root/backend/crates/cli/Cargo.toml"
compose="$root/deploy/compose/compose.yaml"
BINS="webserver controller worker backend-cli migrate"
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

for b in backend-cli migrate; do
	if grep -Eq "^name = \"$b\"\$" "$cli_toml"; then
		ok "crates/cli/Cargo.toml mendefinisikan [[bin]] $b"
	else
		bad "crates/cli/Cargo.toml tidak mendefinisikan [[bin]] name = \"$b\""
	fi
done

# Tahap runtime = dari baris FROM terakhir. ca-certificates wajib terpasang di sana, bukan di build stage.
runtime=$(awk '/^FROM /{buf=""} {buf=buf $0 "\n"} END{printf "%s", buf}' "$cf")
if printf '%s' "$runtime" | grep -Eq 'apt-get install .*ca-certificates'; then
	ok "tahap runtime memasang ca-certificates"
else
	bad "tahap runtime tidak memasang ca-certificates (healthcheck reqwest panik tanpa CA sistem)"
fi

# --- 1b. Compose --------------------------------------------------------------
if [ ! -f "$compose" ]; then
	bad "deploy/compose/compose.yaml tidak ada"
else
	migrate_cmd=$(awk '/^  [A-Za-z0-9_-]+:[[:space:]]*$/ {svc=$1} svc=="migrate:" && /^    command:/ {print; exit}' "$compose" \
		| sed -E 's/^[^[]*\[[[:space:]]*"([^"]+)".*/\1/')
	case " $BINS " in
	*" $migrate_cmd "*) ok "compose service migrate menjalankan biner image: $migrate_cmd" ;;
	*) bad "compose service migrate menjalankan '$migrate_cmd', bukan biner image ($BINS)" ;;
	esac
	# Bind mount relatif (./x:/y atau ../x:/y) dihitung dari direktori compose.yaml.
	mounts=$(grep -E '^[[:space:]]+- \.{1,2}/[^:]+:/' "$compose" | sed -E 's/^[[:space:]]+- (\.{1,2}\/[^:]+):.*/\1/' || true)
	for m in $mounts; do
		if [ -e "$(dirname "$compose")/$m" ]; then
			ok "compose bind mount $m ada"
		else
			bad "compose bind mount $m tidak ada (relatif terhadap deploy/compose)"
		fi
	done
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

# migrate tanpa env: konfigurasi ditolak (exit 2) sebelum menyentuh basis data. Pada image lama, "executable not found" (127/126).
# shellcheck disable=SC2086
rc=0
$engine run --rm $label_arg --network none --entrypoint migrate "$IMAGE" >"$tmp/mig.out" 2>"$tmp/mig.err" || rc=$?
if [ "$rc" -eq 2 ]; then
	ok "image: migrate mulai dan menolak konfigurasi kosong (exit 2)"
else
	bad "image: migrate exit $rc, diharapkan 2"; cat "$tmp/mig.err"
fi

# Healthcheck tanpa listener dan tanpa jaringan: gagal bersih (exit 1). Tanpa ca-certificates reqwest panik (exit 101).
for probe in "webserver healthcheck" "controller healthcheck" "worker liveness"; do
	rc=0
	# shellcheck disable=SC2086
	$engine run --rm $label_arg --network none --entrypoint "${probe%% *}" "$IMAGE" ${probe#* } >"$tmp/probe.out" 2>"$tmp/probe.err" || rc=$?
	if [ "$rc" -eq 1 ] && ! grep -q 'panicked' "$tmp/probe.err"; then
		ok "image: $probe tanpa listener gagal bersih (exit 1, tanpa panik)"
	else
		bad "image: $probe exit $rc, diharapkan 1 tanpa panik"; cat "$tmp/probe.err"
	fi
done

exit "$fail"
