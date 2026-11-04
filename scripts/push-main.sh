#!/bin/sh
# Push ke main hanya bila gate penuh hijau pada hash persis yang di-push (SHAA-328, keputusan SHAA-204).
#
# Pemakaian (dari akar repo):
#   make push-main CONTAINER_LABEL=shaal.issue=SHAA-<n>
#
# Urutan: working tree bersih, origin/main ancestor HEAD, semua commit baru memuat id SHAA-<n>,
# gate `make setup`, `make test`, `make lint` (berurutan, berhenti di kegagalan pertama),
# lalu `git push origin HEAD:main` tanpa force. Blok bukti dicetak di akhir.
#
# Exit code:
#   0   push masuk
#   75  kunci shaal-build-heavy dipegang (diteruskan dari gate); tidak push, coba lagi nanti
#   1   ditolak (tree kotor, di belakang main, commit tanpa id issue, gate merah, push ditolak)
#   2   salah pakai (CONTAINER_LABEL tidak valid, bukan di akar repo)
#
# Variabel: MAKE (default make, bisa dioverride untuk uji), REMOTE (default origin), BRANCH (default main).
set -u

MAKE="${MAKE:-make}"
REMOTE="${REMOTE:-origin}"
BRANCH="${BRANCH:-main}"
LABEL="${CONTAINER_LABEL:-}"
GATE_TARGETS="setup test lint"

START=$(date +%s)
BEFORE=""
RC_LINES=""
HEAD_HASH=""

die() {
	code=$1
	shift
	echo "push-main: $*" >&2
	echo "push-main: TIDAK push." >&2
	exit "$code"
}

# print_evidence <hasil push>: satu blok bukti, dicetak juga saat gate gagal supaya bisa dikutip.
print_evidence() {
	after=$(git ls-remote "$REMOTE" "refs/heads/$BRANCH" 2>/dev/null | cut -f1)
	echo "==== push-main: bukti ===="
	echo "hash HEAD       : $HEAD_HASH"
	echo "$REMOTE/$BRANCH sebelum: ${BEFORE:-?}"
	echo "$REMOTE/$BRANCH sesudah: ${after:-?}"
	for t in $GATE_TARGETS; do
		r=$(printf '%s\n' "$RC_LINES" | sed -n "s/^$t=//p")
		echo "exit make $t    : ${r:-tidak dijalankan}"
	done
	echo "hasil push      : $1"
	echo "waktu           : $(($(date +%s) - START)) detik ($(date '+%Y-%m-%d %H:%M:%S %z'))"
	echo "==== akhir bukti ===="
}

[ -f Makefile ] && [ -d .git ] || [ -f .git ] || die 2 "jalankan dari akar repo."
printf '%s' "$LABEL" | grep -Eq '^shaal\.issue=SHAA-[0-9]+$' ||
	die 2 "CONTAINER_LABEL wajib shaal.issue=SHAA-<n> (contoh: make push-main CONTAINER_LABEL=shaal.issue=SHAA-328)."

if [ -n "$(git status --porcelain)" ]; then
	git status --short >&2
	die 1 "working tree tidak bersih. Commit atau buang perubahan dulu."
fi

git fetch "$REMOTE" "$BRANCH" || die 1 "git fetch $REMOTE $BRANCH gagal."
BEFORE=$(git rev-parse FETCH_HEAD)
HEAD_HASH=$(git rev-parse HEAD)
git merge-base --is-ancestor "$BEFORE" HEAD ||
	die 1 "$REMOTE/$BRANCH ($BEFORE) bukan ancestor HEAD. Merge $BRANCH terbaru dulu, lalu ulangi."

bad=$(git log --format='%h %s' "$BEFORE..HEAD" | while IFS= read -r line; do
	hash=${line%% *}
	git log -1 --format=%B "$hash" | grep -Eq 'SHAA-[0-9]+' || echo "$line"
done)
if [ -n "$bad" ]; then
	echo "$bad" >&2
	die 1 "commit di atas tidak memuat id issue SHAA-<n> di pesannya."
fi

rc_gate=0
for t in $GATE_TARGETS; do
	echo "==> push-main: $MAKE $t"
	$MAKE "$t" "RUNNER=${RUNNER:-podman}" "FRONTEND_RUNNER=${FRONTEND_RUNNER:-podman}" "CONTAINER_LABEL=$LABEL"
	rc=$?
	RC_LINES="$RC_LINES$t=$rc
"
	if [ "$rc" -ne 0 ]; then
		rc_gate=$rc
		break
	fi
done
if [ "$rc_gate" -ne 0 ]; then
	print_evidence "tidak push (gate merah)"
	if [ "$rc_gate" -eq 75 ]; then
		echo "push-main: kunci build dipegang (exit 75). Tidak push; ulangi saat kunci bebas." >&2
		exit 75
	fi
	echo "push-main: gate gagal (exit $rc_gate). Tidak push." >&2
	exit 1
fi

if ! git push "$REMOTE" "HEAD:$BRANCH"; then
	print_evidence "ditolak remote"
	echo "push-main: push ditolak remote; $BRANCH kemungkinan bergeser selama gate. Merge $BRANCH terbaru, lalu jalankan make push-main lagi (gate diulang)." >&2
	exit 1
fi
print_evidence "berhasil"
