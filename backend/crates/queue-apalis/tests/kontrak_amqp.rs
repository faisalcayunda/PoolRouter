//! Suite kontrak port antrean terhadap RabbitMQ sungguhan (ARC-68, spec
//! `f0-controller-worker` AC-1–AC-3, ADR-0001 bagian 6).
//!
//! Broker dijalankan `tests/uji-amqp.sh` (dipanggil `make -C backend
//! uji-amqp`, bagian dari `make test`) dan alamatnya masuk lewat
//! `POOLROUTER_TEST_AMQP`. Setiap uji memakai antrean upstream baru, jadi
//! urutan uji tidak saling memengaruhi.
//!
//! Uji AC-2 memutus koneksi dari sisi broker lewat management API RabbitMQ
//! (`POOLROUTER_TEST_AMQP_MGMT`, `POOLROUTER_TEST_AMQP_MGMT_AUTH`) memakai
//! HTTP/1.1 mentah di atas `tokio::net::TcpStream`, supaya tidak menambah
//! dependensi (lock `--locked`).
//!
//! Efek ganda pesan duplikat di PostgreSQL (compare-and-set) dibuktikan uji
//! handler `duplikat_2000_tanpa_panggilan_ganda` (AC-12) di crate worker; di
//! sini yang dibuktikan sisi broker: kedua salinan sampai, keduanya di-ack.

use std::collections::{HashMap, HashSet};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use lapin::options::{BasicGetOptions, QueueDeclareOptions};
use lapin::types::{AMQPValue, FieldTable, ShortString};
use lapin::{Channel, Connection, ConnectionProperties};
use poolrouter_core::queue::{
    JobConsumer, JobPublisher, QueueError, StatusKonsumen, nama_antrean_upstream,
};
use poolrouter_queue_apalis::topologi::nama_dlq;
use poolrouter_queue_apalis::{ApalisQueue, PREFETCH_BAWAAN, TOLERANSI_LIVENESS};
use tokio::io::{AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::tcp::OwnedReadHalf;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
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
    consumer_dengan(antrean, PREFETCH_BAWAAN, None, handler).await
}

/// [`consumer`] dengan prefetch dan status liveness eksplisit.
async fn consumer_dengan<H, F>(
    antrean: &str,
    prefetch: u16,
    status: Option<Arc<StatusKonsumen>>,
    handler: H,
) -> (CancellationToken, JoinHandle<Result<(), QueueError>>)
where
    H: Fn(Uuid) -> F + Send + Sync + 'static,
    F: std::future::Future<Output = Result<(), QueueError>> + Send + 'static,
{
    let token = CancellationToken::new();
    let mut q = ApalisQueue::connect(&uri())
        .await
        .expect("connect consumer")
        .dengan_penghenti(token.clone())
        .dengan_prefetch(prefetch);
    if let Some(status) = status {
        q = q.dengan_status(status);
    }
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
    // ARC-227, S11: header AMQP hanya `x-trace-id`. Belum ada skema trace di
    // kode (OpenTelemetry belum dipasang), jadi nilainya id korelasi log
    // aplikasi, yaitu `request_id`.
    let headers = pesan
        .delivery
        .properties
        .headers()
        .clone()
        .expect("header AMQP ada");
    assert_eq!(headers.inner().len(), 1, "header hanya x-trace-id");
    match headers.inner().get(&ShortString::from("x-trace-id")) {
        Some(AMQPValue::LongString(nilai)) => assert_eq!(nilai.to_string(), id.to_string()),
        lain => panic!("x-trace-id harus string request_id: {lain:?}"),
    }
}

/// ARC-227: koneksi penerbit diputus broker (pengganti restart broker, karena
/// container toolchain tidak bisa me-restart broker). `publish` berikutnya
/// pada `ApalisQueue` yang sama harus pulih sendiri tanpa proses baru. Satu
/// publish yang kebetulan jatuh tepat saat putus boleh gagal (reconciler
/// memulihkannya), jadi uji mencoba ulang sampai batas tunggu; tanpa sambung
/// ulang semua percobaan gagal selamanya.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "butuh RabbitMQ: make -C backend uji-amqp"]
async fn publish_pulih_setelah_koneksi_diputus_broker() {
    let antrean = antrean_baru();
    let q = penerbit(&antrean).await;
    q.publish(&antrean, Uuid::now_v7())
        .await
        .expect("publish sebelum putus");

    putuskan_dari_broker().await;

    let id = Uuid::now_v7();
    let batas = tokio::time::Instant::now() + BATAS_TUNGGU;
    loop {
        match q.publish(&antrean, id).await {
            Ok(()) => break,
            Err(galat) => {
                assert!(
                    tokio::time::Instant::now() < batas,
                    "publish tidak pulih setelah koneksi diputus: {galat}"
                );
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        }
    }

    let (_k, kanal) = pengamat().await;
    let mut isi = Vec::new();
    while let Some(pesan) = kanal
        .basic_get(antrean.as_str().into(), BasicGetOptions { no_ack: true })
        .await
        .expect("basic_get")
    {
        isi.push(pesan.delivery.data);
    }
    assert!(
        isi.contains(&id.to_string().into_bytes()),
        "pesan sesudah sambung ulang ada di antrean"
    );
}

/// Salin byte satu arah; saat `beku`, byte dibaca lalu dibuang (soket tetap
/// terbuka, tanpa jawaban).
async fn salin(mut dari: OwnedReadHalf, mut ke: impl AsyncWrite + Unpin, beku: Arc<AtomicBool>) {
    let mut buf = [0u8; 4096];
    while let Ok(n) = dari.read(&mut buf).await {
        if n == 0 || (!beku.load(Ordering::SeqCst) && ke.write_all(&buf[..n]).await.is_err()) {
            return;
        }
    }
}

/// Proxy TCP di depan broker yang bisa dibekukan: meniru broker yang tidak
/// menjawab (flow control menahan baca socket, atau host tidak merespons).
/// Saat beku, koneksi lama tidak lagi diteruskan dan koneksi baru diterima
/// tanpa pernah disambung ke broker. Mengembalikan URI lewat proxy dan
/// sakelar bekunya.
async fn proxy_beku() -> (String, Arc<AtomicBool>) {
    let asli = uri();
    let (awal, sisa) = asli.split_once('@').expect("URI uji memuat userinfo");
    let (alamat_broker, jalur) = sisa.split_once('/').expect("URI uji memuat vhost");
    let alamat_broker = alamat_broker.to_owned();
    let pendengar = TcpListener::bind("127.0.0.1:0").await.expect("bind proxy");
    let uri_proxy = format!(
        "{awal}@{}/{jalur}",
        pendengar.local_addr().expect("alamat proxy")
    );
    let beku = Arc::new(AtomicBool::new(false));
    let sakelar = beku.clone();
    tokio::spawn(async move {
        while let Ok((klien, _)) = pendengar.accept().await {
            let alamat_broker = alamat_broker.clone();
            let beku = beku.clone();
            tokio::spawn(async move {
                let (klien_r, klien_w) = klien.into_split();
                if beku.load(Ordering::SeqCst) {
                    salin(klien_r, tokio::io::sink(), beku).await;
                    return;
                }
                let Ok(broker) = TcpStream::connect(&alamat_broker).await else {
                    return;
                };
                let (broker_r, broker_w) = broker.into_split();
                tokio::select! {
                    () = salin(klien_r, broker_w, beku.clone()) => {}
                    () = salin(broker_r, klien_w, beku) => {}
                }
            });
        }
    });
    (uri_proxy, sakelar)
}

/// Batas luar uji: BATAS_CONFIRM (5 detik) ditambah selisih penjadwalan.
const BATAS_LUAR_DECLARE: Duration = Duration::from_secs(10);

/// ARC-227 AC-1: `declare` (jalur `PublisherDeklarasi::publish` di controller,
/// yang memanggilnya sebelum publish sambil memegang mutex) ikut dibatasi
/// `BATAS_CONFIRM` saat broker tidak menjawab RPC deklarasi pada koneksi yang
/// masih tampak tersambung. Tanpa batas, dispatcher macet selama masih leader.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "butuh RabbitMQ: make -C backend uji-amqp"]
async fn declare_dibatasi_saat_broker_tidak_menjawab() {
    let (uri_proxy, beku) = proxy_beku().await;
    let q = ApalisQueue::connect(&uri_proxy)
        .await
        .expect("connect lewat proxy");
    beku.store(true, Ordering::SeqCst);

    let hasil = tokio::time::timeout(BATAS_LUAR_DECLARE, q.declare(&antrean_baru()))
        .await
        .expect("declare menggantung melewati batas confirm");
    assert!(
        matches!(hasil, Err(QueueError::Transport(_))),
        "declare yang lewat batas harus galat transport: {hasil:?}"
    );
}

/// ARC-227 AC-1: sambung ulang di dalam `declare` (koneksi publish diputus
/// broker, lalu host baru tidak merespons) juga dibatasi, bukan menunggu
/// timeout TCP OS dengan mutex publish terpegang.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "butuh RabbitMQ: make -C backend uji-amqp"]
async fn declare_dengan_sambung_ulang_dibatasi_saat_broker_tidak_menjawab() {
    let (uri_proxy, beku) = proxy_beku().await;
    let q = ApalisQueue::connect(&uri_proxy)
        .await
        .expect("connect lewat proxy");
    putuskan_dari_broker().await;
    beku.store(true, Ordering::SeqCst);

    let hasil = tokio::time::timeout(BATAS_LUAR_DECLARE, q.declare(&antrean_baru()))
        .await
        .expect("declare dengan sambung ulang menggantung melewati batas confirm");
    assert!(
        matches!(hasil, Err(QueueError::Transport(_))),
        "declare yang lewat batas harus galat transport: {hasil:?}"
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

/// Tunggu sampai `kondisi` benar, paling lama [`BATAS_TUNGGU`].
async fn tunggu(pesan: &str, kondisi: impl Fn() -> bool) {
    let batas = tokio::time::Instant::now() + BATAS_TUNGGU;
    while !kondisi() {
        assert!(tokio::time::Instant::now() < batas, "{pesan}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Tutup semua koneksi milik user uji dari sisi broker
/// (`DELETE /api/connections/username/<user>`). Ikut menutup koneksi
/// penerbit dan pengamat; keduanya dibuka ulang oleh pemanggil bila perlu.
async fn putuskan_dari_broker() {
    let alamat = std::env::var("POOLROUTER_TEST_AMQP_MGMT")
        .expect("POOLROUTER_TEST_AMQP_MGMT (make -C backend uji-amqp)");
    let auth = std::env::var("POOLROUTER_TEST_AMQP_MGMT_AUTH")
        .expect("POOLROUTER_TEST_AMQP_MGMT_AUTH (make -C backend uji-amqp)");
    let user = lapin::uri::AMQPUri::from_str(&uri())
        .expect("URI uji")
        .authority
        .userinfo
        .username;
    let mut tcp = tokio::net::TcpStream::connect(alamat.as_str())
        .await
        .expect("koneksi management API");
    let permintaan = format!(
        "DELETE /api/connections/username/{user} HTTP/1.1\r\nHost: {alamat}\r\n\
         Authorization: Basic {auth}\r\nX-Reason: uji AC-2\r\n\
         Content-Length: 0\r\nConnection: close\r\n\r\n"
    );
    tcp.write_all(permintaan.as_bytes())
        .await
        .expect("kirim permintaan management");
    let mut jawaban = Vec::new();
    tokio::time::timeout(BATAS_TUNGGU, tcp.read_to_end(&mut jawaban))
        .await
        .expect("management API tidak menjawab")
        .expect("baca jawaban management");
    let baris_status = String::from_utf8_lossy(&jawaban)
        .lines()
        .next()
        .unwrap_or_default()
        .to_owned();
    assert!(
        baris_status.starts_with("HTTP/1.1 2"),
        "management API menolak penutupan koneksi: {baris_status}"
    );
}

/// `GET <jalur>` ke management API; badan JSON sebagai teks.
async fn get_management(jalur: &str) -> String {
    let alamat = std::env::var("POOLROUTER_TEST_AMQP_MGMT")
        .expect("POOLROUTER_TEST_AMQP_MGMT (make -C backend uji-amqp)");
    let auth = std::env::var("POOLROUTER_TEST_AMQP_MGMT_AUTH")
        .expect("POOLROUTER_TEST_AMQP_MGMT_AUTH (make -C backend uji-amqp)");
    let mut tcp = tokio::net::TcpStream::connect(alamat.as_str())
        .await
        .expect("koneksi management API");
    let permintaan = format!(
        "GET {jalur} HTTP/1.1\r\nHost: {alamat}\r\nAuthorization: Basic {auth}\r\n\
         Connection: close\r\n\r\n"
    );
    tcp.write_all(permintaan.as_bytes())
        .await
        .expect("kirim permintaan management");
    let mut jawaban = Vec::new();
    tokio::time::timeout(BATAS_TUNGGU, tcp.read_to_end(&mut jawaban))
        .await
        .expect("management API tidak menjawab")
        .expect("baca jawaban management");
    String::from_utf8_lossy(&jawaban).into_owned()
}

/// ARC-228: DLQ dideklarasikan dengan retensi (TTL 7 hari, 10.000 pesan,
/// `drop-head`), antrean job tidak, dan deklarasi ulang idempoten (tanpa 406).
#[tokio::test(flavor = "multi_thread")]
#[ignore = "butuh RabbitMQ: make -C backend uji-amqp"]
async fn dlq_dideklarasikan_dengan_retensi_dan_antrean_job_tidak() {
    let antrean = antrean_baru();
    let q = penerbit(&antrean).await;
    q.declare(&antrean)
        .await
        .expect("deklarasi ulang idempoten");

    let dlq = get_management(&format!("/api/queues/%2f/{}", nama_dlq(&antrean))).await;
    assert!(dlq.contains("\"x-message-ttl\":604800000"), "{dlq}");
    assert!(dlq.contains("\"x-max-length\":10000"), "{dlq}");
    assert!(dlq.contains("\"x-overflow\":\"drop-head\""), "{dlq}");
    let job = get_management(&format!("/api/queues/%2f/{antrean}")).await;
    assert!(!job.contains("x-message-ttl"), "{job}");
    assert!(!job.contains("x-max-length"), "{job}");
}

/// ARC-228: `jumlah_dlq` membaca panjang DLQ; antrean yang belum pernah
/// dideklarasikan dihitung 0 dan 404-nya tidak menjatuhkan channel publish.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "butuh RabbitMQ: make -C backend uji-amqp"]
async fn jumlah_dlq_membaca_panjang_dan_antrean_tak_ada_nol() {
    let antrean = antrean_baru();
    let q = penerbit(&antrean).await;
    assert_eq!(q.jumlah_dlq(&antrean_baru()).await.expect("tak ada"), 0);
    assert_eq!(q.jumlah_dlq(&antrean).await.expect("kosong"), 0);

    let id = Uuid::now_v7();
    q.publish(&antrean, id)
        .await
        .expect("publish masih jalan setelah 404");
    let (token, tugas) = consumer(&antrean, |_| async {
        Err::<(), QueueError>(QueueError::Handler("uji: galat permanen".into()))
    })
    .await;
    let batas = tokio::time::Instant::now() + BATAS_TUNGGU;
    while q.jumlah_dlq(&antrean).await.expect("hitung") < 1 {
        assert!(
            tokio::time::Instant::now() < batas,
            "pesan tidak sampai DLQ"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    hentikan(token, tugas).await;
    assert_eq!(q.jumlah_dlq(&antrean).await.expect("hitung"), 1);
}

/// AC-2 (a), W3: koneksi consumer diputus broker saat pesan sedang diproses.
/// Consumer menyambung ulang sendiri dengan backoff di proses yang sama (task
/// `run` tidak pernah selesai), pesan yang belum di-ack dikirim ulang tepat
/// sekali, dan efek akhirnya (tiruan compare-and-set) terjadi satu kali.
/// Liveness tetap hidup selama reconnect normal.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "butuh RabbitMQ: make -C backend uji-amqp"]
async fn koneksi_diputus_broker_consumer_sambung_ulang_dan_pesan_dikirim_ulang() {
    let antrean = antrean_baru();
    let q = penerbit(&antrean).await;
    let id = Uuid::now_v7();
    q.publish(&antrean, id).await.expect("publish");

    let status = StatusKonsumen::baru(TOLERANSI_LIVENESS);
    let panggilan = Arc::new(AtomicUsize::new(0));
    let klaim = Arc::new(Mutex::new(HashSet::new()));
    let efek = Arc::new(AtomicUsize::new(0));
    let (lepas_tx, lepas_rx) = watch::channel(false);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (p2, k2, e2) = (
        Arc::clone(&panggilan),
        Arc::clone(&klaim),
        Arc::clone(&efek),
    );
    let (token, tugas) = consumer_dengan(
        &antrean,
        PREFETCH_BAWAAN,
        Some(Arc::clone(&status)),
        move |id| {
            let ke = p2.fetch_add(1, Ordering::SeqCst) + 1;
            let _ = tx.send(id);
            let (k3, e3) = (Arc::clone(&k2), Arc::clone(&e2));
            let mut lepas = lepas_rx.clone();
            async move {
                if ke == 1 {
                    // Handler pertama masih berjalan saat koneksi diputus.
                    let _ = lepas.wait_for(|v| *v).await;
                }
                // "Commit": efek hanya untuk klaim pertama (compare-and-set).
                if k3.lock().expect("kunci klaim").insert(id) {
                    e3.fetch_add(1, Ordering::SeqCst);
                }
                Ok::<(), QueueError>(())
            }
        },
    )
    .await;
    assert_eq!(terima(&mut rx).await, id);
    tunggu("sesi consumer tidak tercatat tersambung", || {
        status.jumlah_tersambung() == 1
    })
    .await;

    putuskan_dari_broker().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    // Handler pertama selesai sesudah koneksinya mati: ack-nya tidak sampai.
    lepas_tx.send_replace(true);
    tunggu("sesi lama tidak tercatat terputus", || {
        status.jumlah_tersambung() == 0
    })
    .await;
    assert!(
        status.hidup(),
        "reconnect normal tidak menggagalkan liveness"
    );
    assert!(!tugas.is_finished(), "consumer tidak boleh berhenti");

    assert_eq!(terima(&mut rx).await, id, "pesan tak-ack dikirim ulang");
    tunggu("consumer tidak tersambung ulang", || {
        status.jumlah_tersambung() == 1
    })
    .await;
    assert!(status.hidup());
    assert!(!tugas.is_finished(), "sambung ulang tanpa restart");
    hentikan(token, tugas).await;

    assert_eq!(
        panggilan.load(Ordering::SeqCst),
        2,
        "dikirim ulang tepat sekali"
    );
    assert_eq!(efek.load(Ordering::SeqCst), 1, "efek akhir tepat sekali");
    let (_k, kanal) = pengamat().await;
    assert_eq!(
        jumlah_siap(&kanal, &antrean).await,
        0,
        "salinan ulang di-ack"
    );
    assert_eq!(jumlah_siap(&kanal, &nama_dlq(&antrean)).await, 0);
}

/// AC-2 (b), S7: SIGTERM saat 10 handler berjalan (prefetch 10). Consumer
/// berhenti mengambil pesan, menunggu kesepuluh handler, meng-ack semuanya,
/// lalu `run` kembali `Ok`: 0 pesan hilang, 0 eksekusi ganda.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "butuh RabbitMQ: make -C backend uji-amqp"]
async fn sigterm_dengan_10_handler_berjalan_tanpa_hilang_tanpa_ganda() {
    const JUMLAH: usize = 10;
    let antrean = antrean_baru();
    let q = penerbit(&antrean).await;
    let ids: Vec<Uuid> = (0..JUMLAH).map(|_| Uuid::now_v7()).collect();
    for id in &ids {
        q.publish(&antrean, *id).await.expect("publish");
    }

    let eksekusi: Arc<Mutex<HashMap<Uuid, usize>>> = Arc::default();
    let (lepas_tx, lepas_rx) = watch::channel(false);
    let (tx, mut rx) = mpsc::unbounded_channel();
    let e2 = Arc::clone(&eksekusi);
    let (token, tugas) = consumer_dengan(&antrean, JUMLAH as u16, None, move |id| {
        let _ = tx.send(id);
        let e3 = Arc::clone(&e2);
        let mut lepas = lepas_rx.clone();
        async move {
            let _ = lepas.wait_for(|v| *v).await;
            *e3.lock().expect("kunci eksekusi").entry(id).or_default() += 1;
            Ok::<(), QueueError>(())
        }
    })
    .await;
    // Jalur produksi: SIGTERM → `cancel_on_signal` → token penghenti.
    tokio::spawn(poolrouter_core::runtime::cancel_on_signal(token.clone()));

    let mut mulai = HashSet::new();
    for _ in 0..JUMLAH {
        mulai.insert(terima(&mut rx).await);
    }
    assert_eq!(mulai, ids.iter().copied().collect::<HashSet<_>>());
    let (_k, kanal) = pengamat().await;
    assert_eq!(
        jumlah_siap(&kanal, &antrean).await,
        0,
        "kesepuluh pesan dipegang handler, belum di-ack"
    );

    tokio::time::sleep(Duration::from_millis(500)).await;
    let pid = std::process::id().to_string();
    let kirim = std::process::Command::new("sh")
        .args(["-c", "kill -TERM \"$1\"", "sh", &pid])
        .status()
        .expect("jalankan kill");
    assert!(kirim.success(), "SIGTERM terkirim");
    tokio::time::timeout(BATAS_TUNGGU, token.cancelled())
        .await
        .expect("SIGTERM tidak membatalkan token");
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert!(
        !tugas.is_finished(),
        "consumer harus menunggu handler yang berjalan"
    );

    lepas_tx.send_replace(true);
    tokio::time::timeout(BATAS_TUNGGU, tugas)
        .await
        .expect("consumer tidak berhenti")
        .expect("task consumer")
        .expect("run kembali Ok saat shutdown");
    // Beri broker waktu mengembalikan pesan tak-ack (bila ada) ke antrean.
    tokio::time::sleep(Duration::from_secs(1)).await;

    let eksekusi = eksekusi.lock().expect("kunci eksekusi").clone();
    assert_eq!(eksekusi.len(), JUMLAH, "0 pesan hilang");
    assert!(
        eksekusi.values().all(|n| *n == 1),
        "0 eksekusi ganda: {eksekusi:?}"
    );
    assert!(rx.try_recv().is_err(), "tidak ada pengiriman tambahan");
    assert_eq!(jumlah_siap(&kanal, &antrean).await, 0, "semua di-ack");
    assert_eq!(jumlah_siap(&kanal, &nama_dlq(&antrean)).await, 0);
}

/// W3 liveness: antrean lama dengan argumen lain membuat setiap sesi gagal
/// 406 PRECONDITION_FAILED. Loop terus mencoba (tidak keluar), tetapi status
/// liveness gagal setelah toleransi habis.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "butuh RabbitMQ: make -C backend uji-amqp"]
async fn sesi_gagal_terus_menggagalkan_liveness_setelah_toleransi() {
    let antrean = antrean_baru();
    let (_k, kanal) = pengamat().await;
    kanal
        .queue_declare(
            antrean.as_str().into(),
            QueueDeclareOptions {
                durable: true,
                ..QueueDeclareOptions::default()
            },
            FieldTable::default(),
        )
        .await
        .expect("antrean lama tanpa argumen DLX");

    let toleransi = Duration::from_secs(3);
    let status = StatusKonsumen::baru(toleransi);
    let panggilan = Arc::new(AtomicUsize::new(0));
    let p2 = Arc::clone(&panggilan);
    let (token, tugas) = consumer_dengan(
        &antrean,
        PREFETCH_BAWAAN,
        Some(Arc::clone(&status)),
        move |_| {
            p2.fetch_add(1, Ordering::SeqCst);
            async { Ok::<(), QueueError>(()) }
        },
    )
    .await;
    assert!(status.hidup(), "grace saat start");
    tokio::time::sleep(toleransi + Duration::from_secs(3)).await;
    assert_eq!(status.jumlah_tersambung(), 0);
    assert!(
        !status.hidup(),
        "loop yang gagal terus harus menggagalkan liveness"
    );
    assert!(!tugas.is_finished(), "loop tetap mencoba sambung ulang");
    hentikan(token, tugas).await;
    assert_eq!(panggilan.load(Ordering::SeqCst), 0);
}
