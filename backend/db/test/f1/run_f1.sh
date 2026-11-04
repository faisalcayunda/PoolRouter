#!/bin/sh
# Pembungkus run F1 (SHAA-335): membuat seed TOTP uji dan KEK uji 32 byte acak untuk SATU run,
# hanya di memori, lalu menjalankan load_f1.sh. Nilai tidak dicetak, tidak ditulis ke repo atau log,
# dan hilang bersama proses ini.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
TOTP_TEST_SEED=$(python3 -I -c 'import base64,secrets; print(base64.b32encode(secrets.token_bytes(20)).decode().rstrip("="))')
F1_KEK_HEX=$(od -An -N32 -tx1 /dev/urandom | tr -d ' \n')
export TOTP_TEST_SEED F1_KEK_HEX
exec sh "$here/load_f1.sh"
