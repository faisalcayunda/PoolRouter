//! Handler penyusun + pengirim data balikan terhadap PostgreSQL sungguhan
//! (ARC-123, AC-24–AC-35).
//!
//! Dijalankan oleh `make -C backend uji-pg`. Fixture ditulis sebagai
//! superuser; handler berjalan di pool `pr_job_balikan` yang dipakai produksi,
//! jadi matriks grant ikut teruji. NIK di fixture sintetis (berawalan 00).

use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::Duration as DurasiTunggu;

use chrono::{DateTime, Duration, Utc};
use poolrouter_core::data_balikan::{
    HasilAck, HasilDiprogam, PengirimTiruan, jalankan_job_balikan, jalankan_job_pengirim,
};
use poolrouter_core::retry::AcakBatasAtas;
use sqlx::{Connection, PgConnection, PgPool};
use uuid::Uuid;

const URL_SU: &str = "POOLROUTER_TEST_DATABASE_URL_SU";
const URL_JOB_BALIKAN: &str = "POOLROUTER_TEST_DATABASE_URL_JOB_BALIKAN";

fn env(nama: &str) -> String {
    std::env::var(nama).unwrap_or_else(|_| panic!("{nama} tidak diset"))
}

async fn pool(url: &str) -> PgPool {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(url)
        .await
        .expect("koneksi PostgreSQL uji")
}

const NIK_A: &str = "0012345678900001";
const NIK_B: &str = "0012345678900002";

struct Fixture {
    su: PgPool,
    worker: PgPool,
    provider_id: Uuid,
    endpoint_id: Uuid,
    client_id: Uuid,
    kredensial_id: Uuid,
    downstream_pool_id: Uuid,
    upstream_pool_id: Uuid,
}

impl Fixture {
    async fn baru(su: PgPool, worker: PgPool) -> Self {
        let admin_id = Uuid::now_v7();
        let client_id = Uuid::now_v7();
        let provider_id = Uuid::now_v7();
        let downstream_pool_id = Uuid::now_v7();
        let upstream_pool_id = Uuid::now_v7();
        let endpoint_id = Uuid::now_v7();
        let kredensial_id = Uuid::now_v7();
        let akhiran = client_id.simple().to_string();

        sqlx::query(
            "INSERT INTO admin_user (pengguna_id, username, password_hash, bahasa_ui)
             VALUES ($1, $2, 'x', 'id')",
        )
        .bind(admin_id)
        .bind(format!("arc123-{akhiran}"))
        .execute(&su)
        .await
        .expect("admin_user");
        sqlx::query("INSERT INTO client (client_id, nama) VALUES ($1, $2)")
            .bind(client_id)
            .bind(format!("arc123-{akhiran}"))
            .execute(&su)
            .await
            .expect("client");
        sqlx::query(
            "INSERT INTO client_kredensial (credential_id, client_id, jenis, kunci_hash, dibuat_oleh)
             VALUES ($1, $2, 'api_key', $3, $4)",
        )
        .bind(kredensial_id)
        .bind(client_id)
        .bind(vec![0u8; 32])
        .bind(admin_id)
        .execute(&su)
        .await
        .expect("client_kredensial");
        sqlx::query(
            "INSERT INTO provider (provider_id, nama, wajib_log_nik, data_balikan_aktif, config_versi_aktif)
             VALUES ($1, $2, false, true, 1)",
        )
        .bind(provider_id)
        .bind(format!("arc123-{akhiran}"))
        .execute(&su)
        .await
        .expect("provider");
        // Schema 2 membawa blok `data_balikan` (F1-5.3). Dokumen harus tetap terbaca oleh admin API
        // (`DokumenProvider`): admin_provider_pg di database yang sama membaca semua provider dan menjawab
        // 503 bila satu dokumen tidak lengkap, jadi field dasar ikut ditulis (sama dengan penyusun_pg).
        sqlx::query(
            "INSERT INTO provider_config_versi
                    (provider_id, versi, config_schema_version, log_mode, dokumen, dibuat_oleh)
             VALUES ($1, 1, 2, 'metadata_only', $2, $3)",
        )
        .bind(provider_id)
        .bind(serde_json::json!({
            "metode": "POST",
            "base_url": "https://provider.contoh.test",
            "body_template": "{}",
            "parameters": [],
            "respons_skema": {},
            "tls_trust": "system",
            "log_mode": "metadata_only",
            "data_balikan": {
                "pemicu_jumlah_n": 2,
                "ukuran_batch_maks": 10,
                "kirim_batas_percobaan": 2,
                "backoff_base_detik": 1,
                "backoff_cap_detik": 2,
                "toleransi_detik": 0,
                "endpoint": "https://contoh.test/balikan",
                "rujukan_secret": "{{secret.DUKCAPIL}}",
                "format_version": "uji-1"
            }
        }))
        .bind(admin_id)
        .execute(&su)
        .await
        .expect("provider_config_versi");
        sqlx::query(
            "INSERT INTO downstream_pool (downstream_pool_id, nama, prioritas) VALUES ($1, $2, 1)",
        )
        .bind(downstream_pool_id)
        .bind(format!("arc123-{akhiran}"))
        .execute(&su)
        .await
        .expect("downstream_pool");
        sqlx::query(
            "INSERT INTO upstream_pool (upstream_pool_id, provider_id, nama, kuota_harian,
                     reset_kuota_jam, reset_kuota_zona)
             VALUES ($1, $2, $3, 1000, '00:00', 'Asia/Jakarta')",
        )
        .bind(upstream_pool_id)
        .bind(provider_id)
        .bind(format!("arc123-{akhiran}"))
        .execute(&su)
        .await
        .expect("upstream_pool");
        sqlx::query(
            "INSERT INTO downstream_endpoint (downstream_endpoint_id, nama, provider_id, downstream_pool_id)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(endpoint_id)
        .bind(format!("arc123-{akhiran}"))
        .bind(provider_id)
        .bind(downstream_pool_id)
        .execute(&su)
        .await
        .expect("downstream_endpoint");
        Self {
            su,
            worker,
            provider_id,
            endpoint_id,
            client_id,
            kredensial_id,
            downstream_pool_id,
            upstream_pool_id,
        }
    }

    /// Request `done` milik provider lewat jalur status yang sah (`queued` -> `dispatched` -> `in_flight`
    /// -> `done`, masing-masing dengan baris `request_status_history`), lengkap dengan log NIK. Sama
    /// dengan fixture penyusun_pg. `webhook_status = 'tidak_dikirim'` supaya baris ini inert bagi job
    /// webhook (baris fixture tidak bisa dihapus: histori dan log append-only).
    async fn request_selesai(&self, nik: &str, metadata: serde_json::Value) -> Uuid {
        let request_id = Uuid::now_v7();
        let kode = Uuid::now_v7();
        let selesai = Utc::now() - Duration::hours(1);
        let diminta = selesai - Duration::seconds(2);
        let mut tx = self.su.begin().await.expect("tx fixture");
        sqlx::query(
            "INSERT INTO request
                    (request_id, client_id, credential_id, downstream_endpoint_id, downstream_pool_id,
                     provider_id, upstream_pool_id, provider_config_version, endpoint_config_version, mode,
                     prioritas, prioritas_efektif, status, input_payload, metadata, received_at,
                     ttl_expires_at, attempt_count)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 1, 1, 'async', 1, 1, 'queued', $8, $9, now(),
                     now() + interval '1 hour', 1)",
        )
        .bind(request_id)
        .bind(self.client_id)
        .bind(self.kredensial_id)
        .bind(self.endpoint_id)
        .bind(self.downstream_pool_id)
        .bind(self.provider_id)
        .bind(self.upstream_pool_id)
        .bind(serde_json::json!({"nama_lengkap": "UJI SINTETIS"}))
        .bind(&metadata)
        .execute(&mut *tx)
        .await
        .expect("request queued");
        sqlx::query(
            "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor)
             VALUES ($1, 1, NULL, 'queued', 'webserver')",
        )
        .bind(request_id)
        .execute(&mut *tx)
        .await
        .expect("history queued");
        for (seq, dari, ke, aktor) in [
            (2, "queued", "dispatched", "controller"),
            (3, "dispatched", "in_flight", "worker"),
        ] {
            sqlx::query("UPDATE request SET status = $2 WHERE request_id = $1")
                .bind(request_id)
                .bind(ke)
                .execute(&mut *tx)
                .await
                .expect("transisi request");
            sqlx::query(
                "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor)
                 VALUES ($1, $2, $3, $4, $5)",
            )
            .bind(request_id)
            .bind(seq)
            .bind(dari)
            .bind(ke)
            .bind(aktor)
            .execute(&mut *tx)
            .await
            .expect("history transisi");
        }
        sqlx::query(
            "INSERT INTO transaction_log (kode_transaksi, request_id, attempt_no, provider_id,
                     upstream_pool_id, client_id, downstream_endpoint_id, provider_config_version,
                     endpoint_config_version, nik, requested_at, responded_at, duration_ms,
                     status_akses, log_mode, worker_id)
             VALUES ($1, $2, 1, $3, $4, $5, $6, 1, 1, $7, $8, $8, 5,
                     'berhasil', 'metadata_only', 'arc214-worker')",
        )
        .bind(kode)
        .bind(request_id)
        .bind(self.provider_id)
        .bind(self.upstream_pool_id)
        .bind(self.client_id)
        .bind(self.endpoint_id)
        .bind(nik)
        .bind(diminta)
        .execute(&mut *tx)
        .await
        .expect("transaction_log");
        sqlx::query(
            "UPDATE request
                SET status = 'done', completed_at = $2, final_kode_transaksi = $3,
                    sumber_hasil = 'provider', output_payload = '{\"data\": {}}'::jsonb,
                    webhook_status = 'tidak_dikirim'
              WHERE request_id = $1",
        )
        .bind(request_id)
        .bind(selesai)
        .bind(kode)
        .execute(&mut *tx)
        .await
        .expect("in_flight -> done");
        sqlx::query(
            "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor)
             VALUES ($1, 4, 'in_flight', 'done', 'worker')",
        )
        .bind(request_id)
        .execute(&mut *tx)
        .await
        .expect("history done");
        tx.commit().await.expect("commit fixture");
        request_id
    }

    async fn batch(&self) -> Option<(Uuid, String, i32, Option<String>)> {
        sqlx::query_as(
            "SELECT batch_id, status, attempt_count, payload_hash FROM data_balikan_batch
              WHERE provider_id = $1 ORDER BY disusun_at DESC LIMIT 1",
        )
        .bind(self.provider_id)
        .fetch_optional(&self.su)
        .await
        .expect("batch terakhir")
    }
}

fn sekarang() -> DateTime<Utc> {
    Utc::now()
}

/// Penampung log `tracing` untuk AC-34: menangkap keluaran log handler yang
/// sungguhan, bukan event tiruan.
#[derive(Clone, Default)]
struct Penampung(Arc<Mutex<Vec<u8>>>);

impl Write for Penampung {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Penampung {
    type Writer = Penampung;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

// AC-29/30: klaim CAS, kirim, status + hash + ack ref tercatat.
// AC-34: log handler yang sungguhan tidak memuat NIK maupun body.
#[tokio::test]
#[ignore]
async fn ac29_30_kirim_diterima() {
    let su = pool(&env(URL_SU)).await;
    let f = Fixture::baru(su.clone(), pool(&env(URL_JOB_BALIKAN)).await).await;
    f.request_selesai(NIK_A, serde_json::json!({})).await;
    f.request_selesai(NIK_B, serde_json::json!({})).await;
    jalankan_job_balikan(&f.worker, f.provider_id, sekarang())
        .await
        .expect("susun");
    let (batch_id, _, _, _) = f.batch().await.expect("batch");

    let pengirim = Arc::new(PengirimTiruan::baru());
    // `set_default` bersifat per-thread; `#[tokio::test]` memakai runtime
    // current-thread, jadi seluruh run di bawah tetap tertangkap selama guard hidup.
    let log = Penampung::default();
    let guard = tracing::subscriber::set_default(
        tracing_subscriber::fmt()
            .with_writer(log.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::INFO)
            .finish(),
    );
    jalankan_job_pengirim(
        &f.worker,
        f.provider_id,
        &pengirim,
        &AcakBatasAtas,
        sekarang(),
    )
    .await
    .expect("kirim");
    drop(guard);

    let isi = String::from_utf8(log.0.lock().unwrap().clone()).expect("log utf8");
    assert!(
        isi.contains("batch data balikan selesai"),
        "log kosong: {isi}"
    );
    for nik in [NIK_A, NIK_B] {
        assert!(!isi.contains(nik), "NIK bocor ke log: {isi}");
    }
    assert!(!isi.contains("nama_lengkap"), "body bocor ke log: {isi}");

    let (status, attempt, hash, ack): (String, i32, Option<String>, Option<String>) =
        sqlx::query_as(
            "SELECT status, attempt_count, payload_hash, provider_ack_ref
               FROM data_balikan_batch WHERE batch_id = $1",
        )
        .bind(batch_id)
        .fetch_one(&su)
        .await
        .unwrap();
    assert_eq!(status, "diterima");
    assert_eq!(attempt, 1);
    assert_eq!(ack.as_deref(), Some(format!("ack-{batch_id}").as_str()));
    assert_eq!(
        hash.as_deref(),
        Some(pengirim.rekaman()[0].hash_body.as_str()),
        "payload_hash = hash body yang benar-benar dikirim"
    );
    assert_eq!(pengirim.jumlah_kirim(batch_id), 1);
}

// AC-30/32: sementara lalu permanen saat batas percobaan tercapai.
#[tokio::test]
#[ignore]
async fn ac30_32_gagal_sementara_lalu_permanen() {
    let su = pool(&env(URL_SU)).await;
    let f = Fixture::baru(su.clone(), pool(&env(URL_JOB_BALIKAN)).await).await;
    f.request_selesai(NIK_A, serde_json::json!({})).await;
    f.request_selesai(NIK_B, serde_json::json!({})).await;
    jalankan_job_balikan(&f.worker, f.provider_id, sekarang())
        .await
        .expect("susun");
    let (batch_id, _, _, _) = f.batch().await.expect("batch");

    let pengirim = Arc::new(PengirimTiruan::baru().dengan_bawaan(HasilDiprogam::GagalSementara));
    let acak = AcakBatasAtas;
    jalankan_job_pengirim(&f.worker, f.provider_id, &pengirim, &acak, sekarang())
        .await
        .expect("kirim 1");

    let (status, attempt, jadwal, error): (String, i32, Option<DateTime<Utc>>, Option<String>) =
        sqlx::query_as(
            "SELECT status, attempt_count, kirim_berikutnya_at, error_terakhir
               FROM data_balikan_batch WHERE batch_id = $1",
        )
        .bind(batch_id)
        .fetch_one(&su)
        .await
        .unwrap();
    assert_eq!(status, "gagal");
    assert_eq!(attempt, 1);
    assert!(jadwal.is_some(), "gagal wajib punya jadwal kirim ulang");
    assert_eq!(error.as_deref(), Some("sementara"));

    // Percobaan kedua = batas (2) → permanen, item dinonaktifkan trigger.
    jalankan_job_pengirim(
        &f.worker,
        f.provider_id,
        &pengirim,
        &acak,
        sekarang() + Duration::hours(1),
    )
    .await
    .expect("kirim 2");

    let (status, attempt, jadwal, error): (String, i32, Option<DateTime<Utc>>, Option<String>) =
        sqlx::query_as(
            "SELECT status, attempt_count, kirim_berikutnya_at, error_terakhir
               FROM data_balikan_batch WHERE batch_id = $1",
        )
        .bind(batch_id)
        .fetch_one(&su)
        .await
        .unwrap();
    assert_eq!(status, "gagal_permanen");
    assert_eq!(attempt, 2);
    assert!(jadwal.is_none());
    assert_eq!(error.as_deref(), Some("batas_percobaan"));
    let aktif: i64 =
        sqlx::query_scalar("SELECT count(*) FROM data_balikan_item WHERE batch_id = $1 AND aktif")
            .bind(batch_id)
            .fetch_one(&su)
            .await
            .unwrap();
    assert_eq!(aktif, 0, "item nonaktif setelah gagal_permanen");
    // AC-35: tidak ada jejak audit_akses dari jalur data balikan.
    let diaudit: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_akses
          WHERE request_id IN (SELECT request_id FROM data_balikan_item WHERE batch_id = $1)",
    )
    .bind(batch_id)
    .fetch_one(&su)
    .await
    .unwrap_or(0);
    assert_eq!(diaudit, 0);
}

// AC-29/31: klaim yang ditinggal proses mati dipulihkan dan dikirim ulang.
#[tokio::test]
#[ignore]
async fn ac31_sisa_crash_dikirim_ulang() {
    let su = pool(&env(URL_SU)).await;
    let f = Fixture::baru(su.clone(), pool(&env(URL_JOB_BALIKAN)).await).await;
    f.request_selesai(NIK_A, serde_json::json!({})).await;
    f.request_selesai(NIK_B, serde_json::json!({})).await;
    jalankan_job_balikan(&f.worker, f.provider_id, sekarang())
        .await
        .expect("susun");
    let (batch_id, _, _, _) = f.batch().await.expect("batch");

    // Sisa crash: klaim tercatat, tapi tidak ada ack ref dan sudah kedaluwarsa.
    sqlx::query(
        "UPDATE data_balikan_batch
            SET status = 'dikirim', attempt_count = 1, dikirim_at = $2,
                payload_hash = repeat('a', 64), provider_ack_ref = NULL
          WHERE batch_id = $1",
    )
    .bind(batch_id)
    .bind(sekarang() - Duration::minutes(20))
    .execute(&su)
    .await
    .expect("sisa crash");

    let pengirim = Arc::new(PengirimTiruan::baru());
    jalankan_job_pengirim(
        &f.worker,
        f.provider_id,
        &pengirim,
        &AcakBatasAtas,
        sekarang(),
    )
    .await
    .expect("pulihkan");

    let (status, attempt): (String, i32) =
        sqlx::query_as("SELECT status, attempt_count FROM data_balikan_batch WHERE batch_id = $1")
            .bind(batch_id)
            .fetch_one(&su)
            .await
            .unwrap();
    assert_eq!(status, "diterima", "sisa crash dikirim ulang dan diterima");
    assert_eq!(attempt, 2);
    assert_eq!(pengirim.jumlah_kirim(batch_id), 1, "hanya satu kirim baru");
}

// AC-31/32: klaim segar tidak direbut; batas percobaan tidak dilewati.
#[tokio::test]
#[ignore]
async fn ac29_32_klaim_segar_dan_batas() {
    let su = pool(&env(URL_SU)).await;
    let f = Fixture::baru(su.clone(), pool(&env(URL_JOB_BALIKAN)).await).await;
    f.request_selesai(NIK_A, serde_json::json!({})).await;
    f.request_selesai(NIK_B, serde_json::json!({})).await;
    jalankan_job_balikan(&f.worker, f.provider_id, sekarang())
        .await
        .expect("susun");
    let (batch_id, _, _, _) = f.batch().await.expect("batch");

    // Klaim segar milik proses lain: tidak boleh disentuh.
    sqlx::query(
        "UPDATE data_balikan_batch
            SET status = 'dikirim', attempt_count = 1, dikirim_at = $2,
                payload_hash = repeat('b', 64)
          WHERE batch_id = $1",
    )
    .bind(batch_id)
    .bind(sekarang())
    .execute(&su)
    .await
    .expect("klaim segar");

    let pengirim = Arc::new(PengirimTiruan::baru());
    jalankan_job_pengirim(
        &f.worker,
        f.provider_id,
        &pengirim,
        &AcakBatasAtas,
        sekarang(),
    )
    .await
    .expect("putaran");
    assert!(pengirim.rekaman().is_empty(), "klaim segar tidak direbut");

    // Batch gagal yang sudah memakai seluruh jatah tidak dikirim lagi.
    sqlx::query(
        "UPDATE data_balikan_batch
            SET status = 'gagal', attempt_count = 2, kirim_berikutnya_at = $2,
                provider_ack_ref = NULL
          WHERE batch_id = $1",
    )
    .bind(batch_id)
    .bind(sekarang() - Duration::minutes(1))
    .execute(&su)
    .await
    .expect("gagal habis");
    jalankan_job_pengirim(
        &f.worker,
        f.provider_id,
        &pengirim,
        &AcakBatasAtas,
        sekarang(),
    )
    .await
    .expect("tutup permanen");
    let (status, attempt): (String, i32) =
        sqlx::query_as("SELECT status, attempt_count FROM data_balikan_batch WHERE batch_id = $1")
            .bind(batch_id)
            .fetch_one(&su)
            .await
            .unwrap();
    assert_eq!((status.as_str(), attempt), ("gagal_permanen", 2));
    assert_eq!(pengirim.jumlah_kirim(batch_id), 0);
}

// AC-33: ack yang belum ada bukan kegagalan; ditolak menjadi permanen.
#[tokio::test]
#[ignore]
async fn ac33_ack_belum_ada_dan_ditolak() {
    let su = pool(&env(URL_SU)).await;
    let f = Fixture::baru(su.clone(), pool(&env(URL_JOB_BALIKAN)).await).await;
    f.request_selesai(NIK_A, serde_json::json!({})).await;
    f.request_selesai(NIK_B, serde_json::json!({})).await;
    jalankan_job_balikan(&f.worker, f.provider_id, sekarang())
        .await
        .expect("susun");
    let (batch_id, _, _, _) = f.batch().await.expect("batch");

    let pengirim = Arc::new(PengirimTiruan::baru().dengan_bawaan(HasilDiprogam::Dikirim));
    jalankan_job_pengirim(
        &f.worker,
        f.provider_id,
        &pengirim,
        &AcakBatasAtas,
        sekarang(),
    )
    .await
    .expect("kirim");
    let (status, referensi): (String, Option<String>) = sqlx::query_as(
        "SELECT status, provider_ack_ref FROM data_balikan_batch WHERE batch_id = $1",
    )
    .bind(batch_id)
    .fetch_one(&su)
    .await
    .unwrap();
    assert_eq!(status, "dikirim");
    let referensi = referensi.expect("ref tersimpan");

    // Belum ada ack: tetap `dikirim`.
    jalankan_job_pengirim(
        &f.worker,
        f.provider_id,
        &pengirim,
        &AcakBatasAtas,
        sekarang() + Duration::minutes(5),
    )
    .await
    .expect("cek ack kosong");
    let status: String =
        sqlx::query_scalar("SELECT status FROM data_balikan_batch WHERE batch_id = $1")
            .bind(batch_id)
            .fetch_one(&su)
            .await
            .unwrap();
    assert_eq!(status, "dikirim");

    // Ditolak saat ack: permanen.
    pengirim.program_ack(
        batch_id,
        &referensi,
        HasilAck::Ditolak {
            alasan: "format tidak dikenal".to_owned(),
        },
    );
    jalankan_job_pengirim(
        &f.worker,
        f.provider_id,
        &pengirim,
        &AcakBatasAtas,
        sekarang() + Duration::minutes(10),
    )
    .await
    .expect("cek ack ditolak");
    let (status, error): (String, Option<String>) =
        sqlx::query_as("SELECT status, error_terakhir FROM data_balikan_batch WHERE batch_id = $1")
            .bind(batch_id)
            .fetch_one(&su)
            .await
            .unwrap();
    assert_eq!(status, "gagal_permanen");
    assert_eq!(error.as_deref(), Some("ditolak_ack"));
}

/// Kunci sesi pengirim bisa diambil koneksi segar (di luar pool). Menunggu paling lama 1 detik, karena
/// koneksi yang ditutup pool baru dilepas server sesaat setelah klien menutupnya.
async fn kunci_pengirim_bebas(provider_id: Uuid) -> bool {
    let mut koneksi = PgConnection::connect(&env(URL_JOB_BALIKAN))
        .await
        .expect("koneksi segar");
    let kunci = format!("poolrouter.balikan.{provider_id}");
    let mut bebas = false;
    for _ in 0..20 {
        bebas = sqlx::query_scalar("SELECT pg_try_advisory_lock(hashtextextended($1, 0))")
            .bind(&kunci)
            .fetch_one(&mut koneksi)
            .await
            .expect("pg_try_advisory_lock");
        if bebas {
            break;
        }
        tokio::time::sleep(DurasiTunggu::from_millis(50)).await;
    }
    koneksi.close().await.expect("tutup koneksi segar");
    bebas
}

// AC-31: panik tengah kirim. Proses worker tetap hidup (panic hanya mengakhiri sesi consumer), jadi
// koneksi pool dikembalikan. Kunci sesi pengirim tidak boleh ikut bocor di koneksi itu, dan batch harus
// bisa dipulihkan tanpa kirim ganda.
#[tokio::test]
#[ignore]
async fn ac31_panik_tengah_kirim_dipulihkan() {
    let su = pool(&env(URL_SU)).await;
    let f = Fixture::baru(su.clone(), pool(&env(URL_JOB_BALIKAN)).await).await;
    f.request_selesai(NIK_A, serde_json::json!({})).await;
    f.request_selesai(NIK_B, serde_json::json!({})).await;
    jalankan_job_balikan(&f.worker, f.provider_id, sekarang())
        .await
        .expect("susun");
    let (batch_id, _, _, _) = f.batch().await.expect("batch");

    // Pengiriman pertama: panik setelah body terkirim, sebelum hasil ditulis.
    let pengirim_crash = Arc::new(PengirimTiruan::dengan_program(vec![
        HasilDiprogam::PanikSetelahKirim,
    ]));
    let pool_worker = f.worker.clone();
    let provider_id = f.provider_id;
    let gabung = tokio::spawn(async move {
        jalankan_job_pengirim(
            &pool_worker,
            provider_id,
            &pengirim_crash,
            &AcakBatasAtas,
            sekarang(),
        )
        .await
    })
    .await;
    assert!(
        gabung.expect_err("panik harus terpicu").is_panic(),
        "tugas harus berakhir karena panik, bukan dibatalkan"
    );

    // (a) Sisa crash: klaim tercatat, tapi tanpa ack ref.
    let (status, attempt, ack_ref): (String, i32, Option<String>) = sqlx::query_as(
        "SELECT status, attempt_count, provider_ack_ref FROM data_balikan_batch WHERE batch_id = $1",
    )
    .bind(batch_id)
    .fetch_one(&su)
    .await
    .unwrap();
    assert_eq!(status, "dikirim", "status tetap dikirim setelah crash");
    assert_eq!(attempt, 1, "attempt_count tercatat");
    assert_eq!(
        ack_ref, None,
        "provider_ack_ref kosong (crash sebelum catat)"
    );

    // (b) Kunci sesi tidak bocor di koneksi yang kembali ke pool.
    assert!(
        kunci_pengirim_bebas(f.provider_id).await,
        "kunci sesi pengirim bocor setelah panik"
    );

    // (c) Pemulihan: sisa crash dikirim ulang sekali, tanpa duplikat.
    let pengirim_pulih = Arc::new(PengirimTiruan::baru());
    jalankan_job_pengirim(
        &f.worker,
        f.provider_id,
        &pengirim_pulih,
        &AcakBatasAtas,
        sekarang() + Duration::minutes(20),
    )
    .await
    .expect("pulihkan setelah crash");
    let (status, attempt): (String, i32) =
        sqlx::query_as("SELECT status, attempt_count FROM data_balikan_batch WHERE batch_id = $1")
            .bind(batch_id)
            .fetch_one(&su)
            .await
            .unwrap();
    assert_eq!(status, "diterima", "sisa crash dipulihkan dan diterima");
    assert_eq!(attempt, 2, "attempt_count naik");
    assert_eq!(
        pengirim_pulih.jumlah_kirim(batch_id),
        1,
        "hanya satu kirim baru, tidak ada dupe"
    );
}

// AC-33 juga memakai provider nonaktif: tidak ada konfigurasi → putaran diam.
#[tokio::test]
#[ignore]
async fn konfigurasi_nonaktif_tidak_mengirim() {
    let su = pool(&env(URL_SU)).await;
    let f = Fixture::baru(su.clone(), pool(&env(URL_JOB_BALIKAN)).await).await;
    f.request_selesai(NIK_A, serde_json::json!({})).await;
    f.request_selesai(NIK_B, serde_json::json!({})).await;
    jalankan_job_balikan(&f.worker, f.provider_id, sekarang())
        .await
        .expect("susun");
    sqlx::query("UPDATE provider SET data_balikan_aktif = false WHERE provider_id = $1")
        .bind(f.provider_id)
        .execute(&su)
        .await
        .expect("matikan data balikan");

    let pengirim = Arc::new(PengirimTiruan::baru());
    let hasil = jalankan_job_pengirim(
        &f.worker,
        f.provider_id,
        &pengirim,
        &AcakBatasAtas,
        sekarang(),
    )
    .await;
    assert_eq!(
        hasil,
        Err(poolrouter_core::data_balikan::GalatPengirim::KonfigTidakAda)
    );
    assert!(pengirim.rekaman().is_empty());
}
