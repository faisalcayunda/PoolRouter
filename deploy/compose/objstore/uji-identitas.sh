#!/bin/sh
# Uji AC identitas dan bucket arsip (SHAA-361 butir 9; SHAA-361 uji GetObject, DeleteObjectVersion, versioning).
# Prasyarat: stack objstore sudah healthy (`make compose-up ... COMPOSE_PROFILES=objstore`), dan direktori secret dev
# berisi objstore_arsip_secret, objstore_restore_secret, objstore_rotasi_secret (dibuat lokal, tidak masuk repo).
# Setiap probe adalah container Podman sementara (aws-cli, digest dipin) di jaringan compose, dihapus setelah selesai.
# Exit 0 hanya jika semua kasus lulus.
set -eu

SECRETS_DIR=${SECRETS_DIR:?isi SECRETS_DIR dengan direktori secret dev di luar repo}
NET=${NET:-poolrouter-dev_default}
BUCKET=${POOLROUTER_ARSIP_BUCKET:-poolrouter-arsip-dev}
AK_ARSIP=${OBJSTORE_ARSIP_ACCESS_KEY:-poolrouter-arsip-job}
AK_RESTORE=${OBJSTORE_RESTORE_ACCESS_KEY:-poolrouter-arsip-restore}
AK_ROTASI=${OBJSTORE_ROTASI_ACCESS_KEY:-poolrouter-arsip-rotasi}
AWS_IMAGE=docker.io/amazon/aws-cli@sha256:d557b3524475d70dd3104cf44d96568ee3188897b73a6ece2b37aa50e1af362f
RUN_ID=$(date +%s)-$$
KEY="uji-identitas/$RUN_ID.txt"
FAILS=0

# run SECRET_NAME AK aws-args...  -> menjalankan aws dengan kredensial identitas itu.
run() {
  sec=$1; ak=$2; shift 2
  podman run --rm --network "$NET" --cap-drop ALL --security-opt no-new-privileges \
    --tmpfs /tmp -e HOME=/tmp -e AK="$ak" -e SECRET="$sec" \
    -v "$SECRETS_DIR/$sec:/run/secrets/$sec:ro" \
    --entrypoint /bin/sh "$AWS_IMAGE" -c \
    'export AWS_ACCESS_KEY_ID="$AK" AWS_SECRET_ACCESS_KEY="$(cat /run/secrets/$SECRET)" AWS_DEFAULT_REGION=us-east-1; exec aws --endpoint-url http://object-storage:9000 "$@"' \
    aws "$@"
}

# expect ok|deny LABEL SECRET AK aws-args...
expect() {
  want=$1; label=$2; shift 2
  set +e
  out=$(run "$@" 2>&1); rc=$?
  set -e
  if [ "$want" = ok ]; then
    [ "$rc" -eq 0 ] && res=PASS || res=FAIL
  else
    if [ "$rc" -ne 0 ] && printf '%s' "$out" | grep -q 'AccessDenied\|Access Denied\|403'; then res=PASS; else res=FAIL; fi
  fi
  echo "$res $label (rc=$rc)"
  if [ "$res" = FAIL ]; then
    FAILS=$((FAILS + 1))
    printf '%s\n' "$out" | tail -n 3 | sed 's/^/    /'
  fi
}

# Versi objek dibuat oleh identitas arsip (satu-satunya yang boleh menulis).
set +e
PUT=$(run objstore_arsip_secret "$AK_ARSIP" s3api put-object --bucket "$BUCKET" --key "$KEY" \
  --body /etc/os-release --output json 2>&1)
rc=$?
set -e
VERSION_ID=$(printf '%s' "$PUT" | sed -n 's/.*"VersionId": *"\([^"]*\)".*/\1/p')
if [ "$rc" -eq 0 ] && [ -n "$VERSION_ID" ]; then
  echo "PASS arsip put-object (versi $VERSION_ID)"
else
  echo "FAIL arsip put-object (rc=$rc)"; FAILS=$((FAILS + 1))
  printf '%s\n' "$PUT" | tail -n 3 | sed 's/^/    /'
  echo "uji dihentikan: tanpa versi objek, kasus lain tidak bermakna"; exit 1
fi

# Identitas arsip: baca dan tulis diizinkan; hapus dan ubah versioning ditolak.
expect ok   "arsip get-object"                 objstore_arsip_secret "$AK_ARSIP" s3api get-object --bucket "$BUCKET" --key "$KEY" /dev/null
expect deny "arsip delete-object (tanpa Delete)" objstore_arsip_secret "$AK_ARSIP" s3api delete-object --bucket "$BUCKET" --key "$KEY"
expect deny "arsip delete-object-version"      objstore_arsip_secret "$AK_ARSIP" s3api delete-object --bucket "$BUCKET" --key "$KEY" --version-id "$VERSION_ID"
expect deny "arsip put-bucket-versioning Suspended" objstore_arsip_secret "$AK_ARSIP" s3api put-bucket-versioning --bucket "$BUCKET" --versioning-configuration Status=Suspended

# Identitas restore: baca per versi diizinkan; tulis, hapus versi, dan ubah versioning ditolak.
expect ok   "restore get-object"               objstore_restore_secret "$AK_RESTORE" s3api get-object --bucket "$BUCKET" --key "$KEY" /dev/null
expect ok   "restore get-object per versi"     objstore_restore_secret "$AK_RESTORE" s3api get-object --bucket "$BUCKET" --key "$KEY" --version-id "$VERSION_ID" /dev/null
expect deny "restore put-object"               objstore_restore_secret "$AK_RESTORE" s3api put-object --bucket "$BUCKET" --key "uji-identitas/tulis-$RUN_ID.txt" --body /etc/os-release
expect deny "restore delete-object-version"    objstore_restore_secret "$AK_RESTORE" s3api delete-object --bucket "$BUCKET" --key "$KEY" --version-id "$VERSION_ID"
expect deny "restore put-bucket-versioning Suspended" objstore_restore_secret "$AK_RESTORE" s3api put-bucket-versioning --bucket "$BUCKET" --versioning-configuration Status=Suspended

# Identitas rotasi: tanpa akses bucket sama sekali (ADR-0006 rev 4 §8.1 butir 13; tanpa GetObject).
expect deny "rotasi get-object (tanpa GetObject)" objstore_rotasi_secret "$AK_ROTASI" s3api get-object --bucket "$BUCKET" --key "$KEY" /dev/null
expect deny "rotasi list-objects"              objstore_rotasi_secret "$AK_ROTASI" s3api list-objects-v2 --bucket "$BUCKET"
expect deny "rotasi put-object"                objstore_rotasi_secret "$AK_ROTASI" s3api put-object --bucket "$BUCKET" --key "uji-identitas/rotasi-$RUN_ID.txt" --body /etc/os-release

if [ "$FAILS" -gt 0 ]; then
  echo "uji-identitas: GAGAL ($FAILS kasus)"; exit 1
fi
echo "uji-identitas: LULUS"
