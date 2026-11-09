//! Suite kontrak port antrean terhadap RabbitMQ sungguhan (ARC-68, spec
//! `f0-controller-worker` AC-1–AC-3, ADR-0001 bagian 6).
//!
//! Broker dijalankan `tests/uji-amqp.sh` (dipanggil `make -C backend
//! uji-amqp`, bagian dari `make test`) dan alamatnya masuk lewat
//! `POOLROUTER_TEST_AMQP`. Setiap uji memakai antrean upstream baru, jadi
//! urutan uji tidak saling memengaruhi.
//!
//! Efek ganda pesan duplikat di PostgreSQL (compare-and-set) dibuktikan uji
//! handler `duplikat_2000_tanpa_panggilan_ganda` (AC-12) di crate worker; di
//! sini yang dibuktikan sisi broker: kedua salinan sampai, keduanya di-ack.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lapin::options::{BasicGetOptions, QueueDeclareOptions};
use lapin::types::FieldTable;
use lapin::{Channel, Connection, ConnectionProperties};
use poolrouter_core::queue::{JobConsumer, JobPublisher, QueueError, nama_antrean_upstream};
use poolrouter_queue_apalis::ApalisQueue;
use poolrouter_queue_apalis::topologi::nama_dlq;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const BATAS_TUNGGU: Duration = Duration::from_secs(30);

fn uri() -> String {
    std::env::var("POOLROUTER_TEST_AMQP").expect("POOLROUTER_TEST_AMQP (make -C backend uji-amqp)")
}

fn antrean_baru() -> String {
    nama_antrean_upstream(Uuid::now_v7())
}

/// Koneksi pengamat terpisah dari adapter.
async fn pengamat() -> (Connection, Channel) {
    let koneksi = Connection::connect(&uri(), ConnectionProperties::default())
        .await
        .expect("koneksi pengamat");
    let kanal = koneksi.create_channel().await.expect("channel pengamat");
    (koneksi, kanal)
}

/// Jumlah pesan siap kirim (tanpa pesan yang sedang dipegang consumer).
async fn jumlah_siap(kanal: &Channel, antrean: &str) -> u32 {
    kanal
        .queue_declare(
            antrean.into(),
            QueueDeclareOptions {
                passive: true,
                ..QueueDeclareOptions::default()
            },
            FieldTable::default(),
        )
        .await
        .expect("queue_declare pasif")
        .message_count()
}

async fn penerbit(antrean: &str) -> ApalisQueue {
    let q = ApalisQueue::connect(&uri())
        .await
        .expect("connect penerbit");
    q.declare(antrean).await.expect("declare");
    q
}

async fn terima(rx: &mut mpsc::UnboundedReceiver<Uuid>) -> Uuid {
    tokio::time::timeout(BATAS_TUNGGU, rx.recv())
        .await
        .expect("pesan tidak sampai dalam batas waktu")
        .expect("channel uji tertutup")
}

/// Jalankan consumer di task terpisah; kembalikan token shutdown-nya.
async fn consumer<H, F>(
    antrean: &str,
    handler: H,
) -> (CancellationToken, JoinHandle<Result<(), QueueError>>)
where
    H: Fn(Uuid) -> F + Send + Sync + 'static,
    F: std::future::Future<Output = Result<(), QueueError>> + Send + 'static,
{
    let token = CancellationToken::new();
    let q = ApalisQueue::connect(&uri())
        .await
        .expect("connect consumer")
        .dengan_penghenti(token.clone());
    let antrean = antrean.to_owned();
    let tugas = tokio::spawn(async move { q.run(&antrean, handler).await });
    (token, tugas)
}

async fn hentikan(token: CancellationToken, tugas: JoinHandle<Result<(), QueueError>>) {
    token.cancel();
    tokio::time::timeout(BATAS_TUNGGU, tugas)
        .await
        .expect("consumer tidak berhenti")
        .expect("task consumer")
        .expect("run kembali Ok saat shutdown");
    // Beri broker waktu mengembalikan pesan tak-ack dari koneksi yang ditutup.
    tokio::time::sleep(Duration::from_secs(1)).await;
}

/// AC-1, S11: body hanya `request_id`, `delivery_mode = 2`, `message_id`.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "butuh RabbitMQ: make -C backend uji-amqp"]
async fn publish_persisten_dengan_body_request_id() {
    let antrean = antrean_baru();
    let q = penerbit(&antrean).await;
    let id = Uuid::now_v7();
    q.publish(&antrean, id).await.expect("publish");

    let (_k, kanal) = pengamat().await;
    let pesan = kanal
        .basic_get(antrean.as_str().into(), BasicGetOptions { no_ack: true })
        .await
        .expect("basic_get")
        .expect("pesan ada di antrean");
    assert_eq!(pesan.delivery.data, id.to_string().into_bytes());
    assert_eq!(*pesan.delivery.properties.delivery_mode(), Some(2));
    assert_eq!(
        pesan
            .delivery
            .properties
            .message_id()
            .as_ref()
            .map(|m| m.as_str().to_owned()),
        Some(id.to_string())
    );
}

/// AC-1 (W1): publish `mandatory` ke routing key tanpa binding gagal, dan
/// channel publish tetap bisa dipakai sesudahnya.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "butuh RabbitMQ: make -C backend uji-amqp"]
async fn publish_tanpa_binding_gagal() {
    let antrean = antrean_baru();
    let q = penerbit(&antrean).await;
    let galat = q.publish(&antrean_baru(), Uuid::now_v7()).await;
    assert!(
        matches!(galat, Err(QueueError::Transport(_))),
        "publish tak terarah harus gagal: {galat:?}"
    );
    q.publish(&antrean, Uuid::now_v7())
        .await
        .expect("publish terarah sesudah kegagalan");
}

/// AC-1, AC-3: pesan sampai ke consumer, di-ack setelah handler `Ok`, dan
/// deklarasi ulang (consumer) atas topologi yang sama idempoten.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "butuh RabbitMQ: make -C backend uji-amqp"]
async fn pesan_sampai_ke_consumer_lalu_diack() {
    let antrean = antrean_baru();
    let q = penerbit(&antrean).await;
    q.declare(&antrean)
        .await
        .expect("deklarasi ulang idempoten");
    let id = Uuid::now_v7();
    q.publish(&antrean, id).await.expect("publish");

    let (tx, mut rx) = mpsc::unbounded_channel();
    let (token, tugas) = consumer(&antrean, move |id| {
        let _ = tx.send(id);
        async { Ok::<(), QueueError>(()) }
    })
    .await;
    assert_eq!(terima(&mut rx).await, id);
    hentikan(token, tugas).await;

    let (_k, kanal) = pengamat().await;
    assert_eq!(jumlah_siap(&kanal, &antrean).await, 0, "pesan sudah di-ack");
    assert_eq!(jumlah_siap(&kanal, &nama_dlq(&antrean)).await, 0);
}

/// AC-1, AC-2, S3: selama handler belum selesai pesan belum di-ack; sesi
/// consumer mati (panic) sebelum ack → koneksi tertutup, consumer menyambung
/// ulang sendiri (W3), dan pesan yang sama dikirim ulang.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "butuh RabbitMQ: make -C backend uji-amqp"]
async fn consumer_mati_sebelum_ack_pesan_dikirim_ulang() {
    let antrean = antrean_baru();
    let q = penerbit(&antrean).await;
    let id = Uuid::now_v7();
    q.publish(&antrean, id).await.expect("publish");

    let panggilan = Arc::new(AtomicUsize::new(0));
    let mati = Arc::new(tokio::sync::Notify::new());
    let (p2, m2) = (Arc::clone(&panggilan), Arc::clone(&mati));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (token, tugas) = consumer(&antrean, move |id| {
        let ke = p2.fetch_add(1, Ordering::SeqCst) + 1;
        let _ = tx.send(id);
        let m3 = Arc::clone(&m2);
        async move {
            if ke == 1 {
                m3.notified().await;
                panic!("uji: consumer mati sebelum ack");
            }
            Ok::<(), QueueError>(())
        }
    })
    .await;
    assert_eq!(terima(&mut rx).await, id);

    let (_k, kanal) = pengamat().await;
    assert_eq!(
        jumlah_siap(&kanal, &antrean).await,
        0,
        "pesan sedang dipegang handler, belum di-ack dan belum kembali"
    );
    mati.notify_one();

    assert_eq!(terima(&mut rx).await, id, "pesan dikirim ulang");
    hentikan(token, tugas).await;
    assert_eq!(panggilan.load(Ordering::SeqCst), 2);
    assert_eq!(jumlah_siap(&kanal, &antrean).await, 0);
}

/// AC-12 sisi broker (S4): dua salinan `request_id` yang sama sampai dan
/// keduanya di-ack; handler idempoten hanya menjalankan efek sekali.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "butuh RabbitMQ: make -C backend uji-amqp"]
async fn duplikat_diack_tanpa_efek_ganda() {
    let antrean = antrean_baru();
    let q = penerbit(&antrean).await;
    let id = Uuid::now_v7();
    q.publish(&antrean, id).await.expect("publish 1");
    q.publish(&antrean, id).await.expect("publish 2");

    // Tiruan compare-and-set: efek hanya untuk klaim pertama.
    let klaim = Arc::new(Mutex::new(HashSet::new()));
    let efek = Arc::new(AtomicUsize::new(0));
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (k2, e2) = (Arc::clone(&klaim), Arc::clone(&efek));
    let (token, tugas) = consumer(&antrean, move |id| {
        if k2.lock().expect("kunci klaim").insert(id) {
            e2.fetch_add(1, Ordering::SeqCst);
        }
        let _ = tx.send(id);
        async { Ok::<(), QueueError>(()) }
    })
    .await;
    assert_eq!(terima(&mut rx).await, id);
    assert_eq!(terima(&mut rx).await, id);
    hentikan(token, tugas).await;

    assert_eq!(efek.load(Ordering::SeqCst), 1);
    let (_k, kanal) = pengamat().await;
    assert_eq!(
        jumlah_siap(&kanal, &antrean).await,
        0,
        "kedua salinan di-ack"
    );
}

/// AC-1: galat handler → pesan masuk DLQ, tidak di-requeue ke antrean asal.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "butuh RabbitMQ: make -C backend uji-amqp"]
async fn galat_handler_masuk_dlq_tanpa_requeue() {
    let antrean = antrean_baru();
    let q = penerbit(&antrean).await;
    let id = Uuid::now_v7();
    q.publish(&antrean, id).await.expect("publish");

    let panggilan = Arc::new(AtomicUsize::new(0));
    let p2 = Arc::clone(&panggilan);
    let (token, tugas) = consumer(&antrean, move |_| {
        p2.fetch_add(1, Ordering::SeqCst);
        async { Err::<(), QueueError>(QueueError::Handler("uji: galat permanen".into())) }
    })
    .await;

    let (_k, kanal) = pengamat().await;
    let dlq = nama_dlq(&antrean);
    let batas = tokio::time::Instant::now() + BATAS_TUNGGU;
    while jumlah_siap(&kanal, &dlq).await < 1 {
        assert!(
            tokio::time::Instant::now() < batas,
            "pesan tidak sampai DLQ"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    hentikan(token, tugas).await;

    assert_eq!(panggilan.load(Ordering::SeqCst), 1, "tanpa requeue buta");
    assert_eq!(jumlah_siap(&kanal, &dlq).await, 1);
    assert_eq!(jumlah_siap(&kanal, &antrean).await, 0);
}

/// AC-2 (S7): shutdown saat handler berjalan menunggu handler selesai, lalu
/// pesan di-ack (tidak dikirim ulang).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "butuh RabbitMQ: make -C backend uji-amqp"]
async fn shutdown_menunggu_handler_berjalan() {
    let antrean = antrean_baru();
    let q = penerbit(&antrean).await;
    let id = Uuid::now_v7();
    q.publish(&antrean, id).await.expect("publish");

    let selesai = Arc::new(AtomicUsize::new(0));
    let s2 = Arc::clone(&selesai);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (token, tugas) = consumer(&antrean, move |id| {
        let _ = tx.send(id);
        let s3 = Arc::clone(&s2);
        async move {
            tokio::time::sleep(Duration::from_millis(500)).await;
            s3.fetch_add(1, Ordering::SeqCst);
            Ok::<(), QueueError>(())
        }
    })
    .await;
    assert_eq!(terima(&mut rx).await, id);
    hentikan(token, tugas).await;

    assert_eq!(
        selesai.load(Ordering::SeqCst),
        1,
        "handler selesai sebelum berhenti"
    );
    let (_k, kanal) = pengamat().await;
    assert_eq!(
        jumlah_siap(&kanal, &antrean).await,
        0,
        "pesan di-ack, tidak kembali"
    );
}
