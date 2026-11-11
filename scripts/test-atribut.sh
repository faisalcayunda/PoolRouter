#!/bin/sh
# Uji regresi aturan repo (AC-3, ARC-2).
#
# Menjaga hal-hal yang bisa diam-diam kembali:
#   1. `make push-main` ada, tetapi hanya sebagai penjaga: menolak dengan exit 2 dan
#      mencetak alur PR. Jalur push otomatis ke `main` (scripts/push-main.sh) tidak boleh ada.
#   2. CLAUDE.md memuat butir aturan repo: branch agent/, PR ke main, merge Operator,
#      tanpa force-push, tanpa secret, NIK sintetis, label gate arctiva.issue.
#   3. Tidak ada lagi rujukan label Shaal (`shaal.issue`, `shaal.build`) di Makefile,
#      skrip, atau workflow.
#   4. Nama lane build bersama tetap `poolrouter-build`/`poolrouter.build` (AC-5):
#      nama `arctiva` tidak boleh kembali di Makefile, skrip, runbook, atau workflow.
#   5. Teks bantuan dan pesan `push-main` tidak memaksa `RUNNER=docker` sebagai satu-satunya
#      gate resmi dan tidak menyebut pihak luar sebagai pemberi larangan (ARC-224);
#      blok "Gate resmi" di docs/runbook/docker-lane.md ikut dijaga.
#
# Tidak butuh Docker. Jalankan: make test-atribut (dari akar repo).
set -eu
root=$(cd "$(dirname "$0")/.." && pwd)
fail=0

ok() { echo "lulus: $1"; }
bad() { echo "GAGAL: $1"; fail=1; }

# --- 1. push-main penjaga ---------------------------------------------------
if [ -e "$root/scripts/push-main.sh" ]; then
	bad "scripts/push-main.sh muncul lagi; jalur push otomatis ke main harus tidak ada"
else
	ok "scripts/push-main.sh tidak ada"
fi

set +e
out=$(cd "$root" && make -s push-main RUNNER=docker FRONTEND_RUNNER=docker \
	CONTAINER_LABEL=arctiva.issue=ARC-2 2>&1)
rc=$?
set -e
if [ "$rc" -eq 2 ]; then
	ok "make push-main menolak dengan exit 2"
else
	bad "make push-main exit=$rc (harap 2)"
fi
for frasa in "PR" "agent/" "Operator"; do
	if [ -n "$out" ] && [ "$(printf '%s' "$out" | grep -c -- "$frasa")" -gt 0 ]; then
		ok "pesan push-main menyebut $frasa"
	else
		bad "pesan push-main tidak menyebut $frasa"
	fi
done
if [ -n "$out" ] && printf '%s' "$out" | grep -q "No rule to make target"; then
	bad "push-main hilang dari Makefile (No rule to make target)"
else
	ok "target push-main terdaftar di Makefile"
fi

# --- 2. CLAUDE.md -----------------------------------------------------------
rules="$root/CLAUDE.md"
if [ ! -f "$rules" ]; then
	ok "CLAUDE.md tidak ada di checkout ini (file lokal Operator, bagian ini dilewati)"
else
	cek_pola() { # <nama> <regex>
		if grep -Eq "$2" "$rules"; then ok "CLAUDE.md: $1"; else bad "CLAUDE.md tidak memuat $1"; fi
	}
	cek_pola "branch agent/ARC-<n>-<slug>"  'agent/ARC-<n>-<slug>'
	cek_pola "PR wajib ke main"          'PR.*wajib.*`main`|PR ke `main`'
	cek_pola "merge hanya Operator"      'Merge hanya oleh Operator|merge hanya (oleh )?Operator|satu-satunya yang merge'
	cek_pola "tanpa force-push"          'force-push'
	cek_pola "tanpa secret"              'secret di repo|menulis secret'
	cek_pola "NIK sintetis"              'NIK hanya sintetis'
	cek_pola "label gate arctiva.issue"  'arctiva\.issue=ARC-<n>'
	if grep -Eq 'push langsung ke `main` tanpa PR|Squad Engineering boleh commit, push' "$rules"; then
		bad "CLAUDE.md masih memuat wewenang push langsung ke main"
	else
		ok "CLAUDE.md tidak lagi mengizinkan push langsung ke main"
	fi
fi

# --- 3. label Shaal tidak kembali ------------------------------------------
sisa=$(grep -rn "shaal\.issue=\|shaal\.build=" \
	"$root"/Makefile "$root"/*/Makefile "$root"/scripts "$root"/docs "$root"/deploy \
	"$root"/.github 2>/dev/null | grep -v 'test-label.sh\|arsip-branch-shaal\|rencana-uji-beban' || true)
if [ -n "$sisa" ]; then
	bad "label Shaal masih dipakai:"; printf '%s\n' "$sisa"
else
	ok "tidak ada label shaal.issue/shaal.build di Makefile, skrip, docs, deploy, workflow"
fi

# --- 4. nama lane build bersama (AC-5) -------------------------------------
sisa=$(grep -rnE "arctiva[-.]build" \
	"$root"/backend/Makefile "$root"/scripts "$root"/.github/workflows/backend.yml \
	"$root"/docs/runbook 2>/dev/null || true)
if [ -n "$sisa" ]; then
	bad "nama lane arctiva[-.]build muncul lagi (AC-5 memakai poolrouter-build):"; printf '%s\n' "$sisa"
else
	ok "lane build bersama memakai poolrouter-build/poolrouter.build (AC-5)"
fi

# --- 5. teks gate mengikuti runner yang tersedia (ARC-224) -------------------
# Pola disusun dari potongan supaya skrip ini sendiri tidak memuatnya utuh.
pola_dilarang="RUNNER=dock""er|Arct""iva"
for cmd in "make -s help" "make -s push-main" "make -s -C backend help" "make -s -C frontend help"; do
	set +e
	out=$(cd "$root" && $cmd 2>&1)
	set -e
	if printf '%s\n' "$out" | grep -Eq "$pola_dilarang"; then
		bad "keluaran '$cmd' memaksa runner tertentu atau menyebut pemberi larangan luar:"
		printf '%s\n' "$out" | grep -En "$pola_dilarang"
	else
		ok "keluaran '$cmd' mengikuti runner yang tersedia"
	fi
done

# Blok "Gate resmi" di runbook: dari barisnya sampai penutup blok kode pertama.
runbook="$root/docs/runbook/docker-lane.md"
if [ -f "$runbook" ]; then
	blok=$(awk '/^Gate resmi/ {on=1} on {print} on && /^```$/ && ++f == 1 {exit}' "$runbook")
	if [ -z "$blok" ]; then
		bad "blok 'Gate resmi' tidak ditemukan di docs/runbook/docker-lane.md"
	elif printf '%s\n' "$blok" | grep -Eq "$pola_dilarang"; then
		bad "blok 'Gate resmi' di runbook memaksa runner tertentu:"
		printf '%s\n' "$blok" | grep -En "$pola_dilarang"
	else
		ok "blok 'Gate resmi' di runbook mengikuti runner yang tersedia"
	fi
fi

exit "$fail"
