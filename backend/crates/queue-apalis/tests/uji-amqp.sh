#!/bin/sh
# Suite kontrak port antrean terhadap RabbitMQ sungguhan (ARC-68). Dipanggil oleh `make uji-amqp`.
#
# RabbitMQ jalan sebagai container sekali pakai berlabel arctiva.issue=<ARC_ISSUE> dengan image yang
# sama dengan deploy/compose/compose.yaml. Port dipublikasikan ke 127.0.0.1 supaya container toolchain
# cargo (jaringan host) menjangkaunya. User dan password acak hanya hidup di container dan proses ini;
# `guest` tidak dipakai karena RabbitMQ menolaknya dari luar loopback container.
#
#   sh crates/queue-apalis/tests/uji-amqp.sh <target-make-yang-menjalankan-cargo>
set -eu

ENGINE=${CONTAINER_ENGINE:-podman}
ISSUE=$ARC_ISSUE
: "${ARC_ISSUE:?isi ARC_ISSUE, mis. ARC-123}"
command -v "$ENGINE" >/dev/null 2>&1 || { echo "uji-amqp.sh: '$ENGINE' tidak ada di PATH; set CONTAINER_ENGINE=docker atau pasang engine-nya" >&2; exit 127; }
PORT=${UJI_AMQP_PORT:-$((20000 + $$ % 20000))}
MAKE_CMD=${1:?target make untuk cargo test belum diberikan}
IMAGE=${RABBITMQ_IMAGE:-docker.io/library/rabbitmq:4-management-alpine@sha256:b768d2e09a809b48458d6c3c32fc2894643a6bfafd76264c120373eff0ee6532}
NAME="poolrouter-ujiamqp-$$"

bersihkan() {
  $ENGINE rm -f -v "$NAME" >/dev/null 2>&1 || true
}
trap bersihkan EXIT INT TERM

USER_MQ="uji$(od -An -N4 -tx1 /dev/urandom | tr -d ' \n')"
PW_MQ=$(od -An -N16 -tx1 /dev/urandom | tr -d ' \n')
$ENGINE run -d --name "$NAME" --label "arctiva.issue=$ISSUE" -p "127.0.0.1:$PORT:5672" \
  -e RABBITMQ_DEFAULT_USER="$USER_MQ" -e RABBITMQ_DEFAULT_PASS="$PW_MQ" \
  --tmpfs /var/lib/rabbitmq:rw,mode=1777 "$IMAGE" >/dev/null

i=0
# `rabbitmq-diagnostics` lewat `exec` berjalan dengan cookie Erlang berbeda dari server (crash dump),
# jadi kesiapan dibaca dari log: listener AMQP sudah aktif saat "Server startup complete" tercetak.
until $ENGINE logs "$NAME" 2>&1 | grep -q "Server startup complete"; do
  i=$((i + 1))
  [ "$i" -le 120 ] || { echo "rabbitmq tidak siap" >&2; $ENGINE logs --tail 50 "$NAME" >&2 || true; exit 1; }
  sleep 1
done

export POOLROUTER_TEST_AMQP="amqp://$USER_MQ:$PW_MQ@127.0.0.1:$PORT/%2f"
echo "uji-amqp: RabbitMQ 127.0.0.1:$PORT"
$MAKE_CMD
echo "uji-amqp: lulus"
