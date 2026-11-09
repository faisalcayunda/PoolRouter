//! Fixture bersama uji dispatcher, penyapu, dan reconciler (ARC-70, ARC-72).
//!
//! Fixture ditulis sebagai superuser karena `pr_controller` sengaja tidak punya
//! hak tulis ke tabel konfigurasi dan `request` di luar kolom transisinya; kode
//! yang diuji berjalan sebagai `pr_controller`. Semua env var diwajibkan
//! (`expect`), tidak ada jalan pintas "dilewati": uji yang tidak bisa jalan
//! harus gagal, bukan hijau diam-diam.
//!
//! Semua nilai sintetis. NIK berawalan `00`.

#![allow(dead_code, clippy::type_complexity)]

use chrono::{DateTime, Duration, Utc};
use poolrouter_core::kuota::pemulihan::{HasilRebuild, KUNCI_SIAP, baca_run_id, bangun_ulang};
use poolrouter_core::kuota::store::KuotaRedis;
use poolrouter_core::kuota::sumber_pg::SumberPostgres;
use poolrouter_core::queue::{JobPublisher, QueueError};
use sqlx::PgPool;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

pub type Kuota = KuotaRedis<redis::aio::MultiplexedConnection>;

pub async fn pool_su() -> PgPool {
    let url = std::env::var("POOLROUTER_TEST_DATABASE_URL_SU")
        .expect("POOLROUTER_TEST_DATABASE_URL_SU wajib (make -C backend uji-pg)");
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(4)
        .connect(&url)
        .await
        .expect("koneksi superuser")
}

pub async fn pool_controller() -> PgPool {
    let url = std::env::var("POOLROUTER_TEST_DATABASE_URL_CONTROLLER")
        .expect("POOLROUTER_TEST_DATABASE_URL_CONTROLLER wajib (make -C backend uji-pg)");
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(8)
        .connect(&url)
        .await
        .expect("koneksi pr_controller")
}

pub fn klien_redis() -> redis::Client {
    let url = std::env::var("POOLROUTER_TEST_REDIS").expect("POOLROUTER_TEST_REDIS wajib");
    redis::Client::open(url).expect("URL Redis tidak sah")
}

/// Bangun ulang Redis dari PostgreSQL (rebuild nyata, menulis `pr:siap`) dan
/// kembalikan store kuota yang siap dipakai.
pub async fn siapkan_redis(controller: &PgPool) -> Kuota {
    let klien = klien_redis();
    let mut koneksi = klien
        .get_multiplexed_async_connection()
        .await
        .expect("koneksi Redis");
    let run_id = baca_run_id(&mut koneksi).await.expect("run_id");
    redis::cmd("DEL")
        .arg(KUNCI_SIAP)
        .query_async::<()>(&mut koneksi)
        .await
        .expect("hapus pr:siap");
    let hasil = bangun_ulang(
        &mut koneksi,
        &run_id,
        &SumberPostgres::baru(controller.clone()),
        Utc::now(),
    )
    .await
    .expect("rebuild Redis");
    assert!(
        matches!(hasil, HasilRebuild::Dibangun(_) | HasilRebuild::SudahSiap),
        "rebuild harus selesai, dapat {hasil:?}"
    );
    KuotaRedis::baru(koneksi).await.expect("store kuota")
}

/// Nilai counter `antre` pool langsung dari Redis.
pub async fn antre(pool: Uuid) -> i64 {
    let mut k = klien_redis()
        .get_multiplexed_async_connection()
        .await
        .expect("koneksi Redis");
    let v: Option<i64> = redis::cmd("GET")
        .arg(format!("pr:antre:{{{pool}}}"))
        .query_async(&mut k)
        .await
        .expect("GET antre");
    v.unwrap_or(0)
}

/// Apakah member slot `<request>:<nomor>` ada di semaphore pool.
pub async fn ada_slot(pool: Uuid, request_id: Uuid, nomor: i64) -> bool {
    let mut k = klien_redis()
        .get_multiplexed_async_connection()
        .await
        .expect("koneksi Redis");
    let skor: Option<f64> = redis::cmd("ZSCORE")
        .arg(format!("pr:slot:{{{pool}}}"))
        .arg(format!("{request_id}:{nomor}"))
        .query_async(&mut k)
        .await
        .expect("ZSCORE slot");
    skor.is_some()
}

/// Publisher perekam: urutan `(queue, request_id)` yang diterbitkan.
#[derive(Clone, Default)]
pub struct Perekam(pub Arc<Mutex<Vec<(String, Uuid, std::time::Instant)>>>);

impl Perekam {
    pub fn id(&self) -> Vec<Uuid> {
        self.0
            .lock()
            .unwrap()
            .iter()
            .map(|(_, id, _)| *id)
            .collect()
    }
}

impl JobPublisher for Perekam {
    async fn publish(&self, queue: &str, request_id: Uuid) -> Result<(), QueueError> {
        self.0
            .lock()
            .unwrap()
            .push((queue.to_owned(), request_id, std::time::Instant::now()));
        Ok(())
    }
}

/// Publisher yang selalu gagal (AC-9).
pub struct PublisherGagal;

impl JobPublisher for PublisherGagal {
    async fn publish(&self, _queue: &str, _request_id: Uuid) -> Result<(), QueueError> {
        Err(QueueError::Transport("broker uji dimatikan".into()))
    }
}

pub struct OpsiGraf {
    pub kuota_harian: i32,
    pub margin: &'static str,
    pub parameter: serde_json::Value,
    pub retry_rasio: &'static str,
    pub retry_min: i32,
    pub ttl_antrean_detik: i32,
    pub webhook_aktif: bool,
}

impl Default for OpsiGraf {
    fn default() -> Self {
        Self {
            kuota_harian: 100_000,
            margin: "0.010",
            parameter: serde_json::json!({
                "rate": { "tps": { "t_mikro": 1_000, "burst": 1_000 } },
                "maks_konkurensi": 5_000,
                "timeout_total_upstream_detik": 30
            }),
            retry_rasio: "0.100",
            retry_min: 20,
            ttl_antrean_detik: 3_600,
            webhook_aktif: false,
        }
    }
}

/// Satu graf konfigurasi lengkap: client, kredensial, provider, endpoint,
/// downstream pool, upstream pool, dan rutenya.
#[derive(Debug, Clone, Copy)]
pub struct Graf {
    pub admin_id: Uuid,
    pub client_id: Uuid,
    pub kredensial_id: Uuid,
    pub provider_id: Uuid,
    pub dpool_id: Uuid,
    pub endpoint_id: Uuid,
    pub upool_id: Uuid,
}

fn nama(id: Uuid) -> String {
    format!("arc70-{}", id.simple())
}

pub async fn buat_graf(su: &PgPool, opsi: &OpsiGraf) -> Graf {
    let g = Graf {
        admin_id: Uuid::now_v7(),
        client_id: Uuid::now_v7(),
        kredensial_id: Uuid::now_v7(),
        provider_id: Uuid::now_v7(),
        dpool_id: Uuid::now_v7(),
        endpoint_id: Uuid::now_v7(),
        upool_id: Uuid::now_v7(),
    };
    sqlx::query(
        "INSERT INTO admin_user (pengguna_id, username, password_hash, bahasa_ui) VALUES ($1, $2, 'x', 'id')",
    )
    .bind(g.admin_id)
    .bind(nama(g.admin_id))
    .execute(su)
    .await
    .expect("admin_user");
    sqlx::query("INSERT INTO client (client_id, nama) VALUES ($1, $2)")
        .bind(g.client_id)
        .bind(nama(g.client_id))
        .execute(su)
        .await
        .expect("client");
    sqlx::query(
        "INSERT INTO client_kredensial (credential_id, client_id, jenis, kunci_hash, dibuat_oleh)
         VALUES ($1, $2, 'api_key', $3, $4)",
    )
    .bind(g.kredensial_id)
    .bind(g.client_id)
    .bind(vec![7u8; 32])
    .bind(g.admin_id)
    .execute(su)
    .await
    .expect("client_kredensial");
    sqlx::query("INSERT INTO provider (provider_id, nama, wajib_log_nik) VALUES ($1, $2, false)")
        .bind(g.provider_id)
        .bind(nama(g.provider_id))
        .execute(su)
        .await
        .expect("provider");
    sqlx::query(
        "INSERT INTO downstream_pool (downstream_pool_id, nama, prioritas, ttl_antrean_detik)
         VALUES ($1, $2, 1, $3)",
    )
    .bind(g.dpool_id)
    .bind(nama(g.dpool_id))
    .bind(opsi.ttl_antrean_detik)
    .execute(su)
    .await
    .expect("downstream_pool");
    buat_endpoint(su, &g, g.endpoint_id).await;
    sqlx::query(
        "INSERT INTO upstream_pool
                (upstream_pool_id, provider_id, nama, kuota_harian, margin_kuota,
                 parameter_dispatch, retry_rasio, retry_min_per_jam,
                 reset_kuota_jam, reset_kuota_zona)
         VALUES ($1, $2, $3, $4, $5::numeric, $6, $7::numeric, $8, '00:00', 'Asia/Jakarta')",
    )
    .bind(g.upool_id)
    .bind(g.provider_id)
    .bind(nama(g.upool_id))
    .bind(opsi.kuota_harian)
    .bind(opsi.margin)
    .bind(&opsi.parameter)
    .bind(opsi.retry_rasio)
    .bind(opsi.retry_min)
    .execute(su)
    .await
    .expect("upstream_pool");
    sqlx::query(
        "INSERT INTO downstream_pool_rute (downstream_pool_id, upstream_pool_id) VALUES ($1, $2)",
    )
    .bind(g.dpool_id)
    .bind(g.upool_id)
    .execute(su)
    .await
    .expect("downstream_pool_rute");
    if opsi.webhook_aktif {
        sqlx::query(
            "INSERT INTO client_webhook
                    (client_id, webhook_url, aktif, secret_versi, secret_ciphertext,
                     secret_dek_terbungkus, secret_nonce, kek_id, diubah_oleh)
             VALUES ($1, 'https://penerima.contoh.invalid/hook', true, 1, $2, $2, $2, 'uji', $3)",
        )
        .bind(g.client_id)
        .bind(vec![1u8; 16])
        .bind(g.admin_id)
        .execute(su)
        .await
        .expect("client_webhook");
    }
    g
}

pub async fn buat_endpoint(su: &PgPool, g: &Graf, endpoint_id: Uuid) {
    sqlx::query(
        "INSERT INTO downstream_endpoint (downstream_endpoint_id, nama, provider_id, downstream_pool_id)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(endpoint_id)
    .bind(nama(endpoint_id))
    .bind(g.provider_id)
    .bind(g.dpool_id)
    .execute(su)
    .await
    .expect("downstream_endpoint");
}

/// Spesifikasi satu request fixture.
#[derive(Debug, Clone)]
pub struct Req {
    pub mode: &'static str,
    /// `queued`, `dispatched`, `in_flight`, atau `done`.
    pub status: &'static str,
    /// `attempt_count` akhir.
    pub attempt: i32,
    pub prioritas: i32,
    pub ttl: Option<DateTime<Utc>>,
    pub nik: Option<String>,
    pub endpoint: Option<Uuid>,
    pub dispatched_at: Option<DateTime<Utc>>,
    /// Umur `requested_at` log percobaan terakhir (`in_flight`/`done`).
    pub umur_log: Duration,
    /// `completed_at` untuk `done`.
    pub selesai: Option<DateTime<Utc>>,
    pub webhook_status_done: &'static str,
}

impl Req {
    pub fn queued() -> Self {
        Self {
            mode: "async",
            status: "queued",
            attempt: 0,
            prioritas: 1,
            ttl: Some(Utc::now() + Duration::hours(1)),
            nik: None,
            endpoint: None,
            dispatched_at: None,
            umur_log: Duration::minutes(10),
            selesai: None,
            webhook_status_done: "terkirim",
        }
    }
}

async fn catat(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    id: Uuid,
    dari: Option<&str>,
    ke: &str,
) {
    sqlx::query(
        "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor)
         SELECT $1, COALESCE(MAX(seq), 0) + 1, $2, $3, 'webserver'
           FROM request_status_history WHERE request_id = $1",
    )
    .bind(id)
    .bind(dari)
    .bind(ke)
    .execute(&mut **tx)
    .await
    .expect("history fixture");
}

async fn log(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    g: &Graf,
    endpoint: Uuid,
    id: Uuid,
    no: i32,
    requested_at: DateTime<Utc>,
    tutup: bool,
) -> Uuid {
    let kode = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO transaction_log
                (kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id, client_id,
                 downstream_endpoint_id, provider_config_version, endpoint_config_version,
                 requested_at, log_mode, worker_id, status_akses, error_class, responded_at)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 1, 1, $8, 'metadata_only', 'arc70-worker',
                 CASE WHEN $9 THEN 'gagal_jaringan' END,
                 CASE WHEN $9 THEN 'timeout' END,
                 CASE WHEN $9 THEN $8 + interval '1 second' END)",
    )
    .bind(kode)
    .bind(id)
    .bind(no)
    .bind(g.provider_id)
    .bind(g.upool_id)
    .bind(g.client_id)
    .bind(endpoint)
    .bind(requested_at)
    .bind(tutup)
    .execute(&mut **tx)
    .await
    .expect("transaction_log fixture");
    kode
}

/// Menulis satu request lewat jalur status yang sah (T-05) dengan history
/// lengkap (T-04) dan log untuk setiap percobaan (DQ-06).
pub async fn buat_request(su: &PgPool, g: &Graf, r: &Req) -> Uuid {
    let id = Uuid::now_v7();
    let endpoint = r.endpoint.unwrap_or(g.endpoint_id);
    let sekarang = Utc::now();
    let async_ = r.mode == "async";
    let lewat_worker = async_ && matches!(r.status, "in_flight" | "done");
    let awal = if async_ { "queued" } else { "in_flight" };
    let attempt_awal = if lewat_worker {
        r.attempt - 1
    } else {
        r.attempt
    };
    let upstream = (r.attempt > 0 || r.status != "queued").then_some(g.upool_id);
    let payload = r
        .nik
        .as_ref()
        .map(|n| serde_json::json!({ "nik": n }))
        .unwrap_or_else(|| serde_json::json!({ "nik": "0012345678901234" }));

    let mut tx = su.begin().await.expect("tx fixture");
    sqlx::query(
        "INSERT INTO request
                (request_id, client_id, credential_id, downstream_endpoint_id, downstream_pool_id,
                 provider_id, upstream_pool_id, provider_config_version, endpoint_config_version,
                 mode, prioritas, prioritas_efektif, status, metadata, input_payload, received_at,
                 attempt_count, ttl_expires_at, dispatch_key)
         VALUES ($1, $2, $3, $4, $5, $6, $7, 1, 1, $8, $9, $9, $10, '{}'::jsonb, $11, now(),
                 $12, $13, CASE WHEN $8 = 'async' THEN gen_random_uuid() END)",
    )
    .bind(id)
    .bind(g.client_id)
    .bind(g.kredensial_id)
    .bind(endpoint)
    .bind(g.dpool_id)
    .bind(g.provider_id)
    .bind(upstream)
    .bind(r.mode)
    .bind(r.prioritas)
    .bind(awal)
    .bind(payload)
    .bind(if async_ { attempt_awal } else { r.attempt })
    .bind(if async_ { r.ttl } else { None })
    .execute(&mut *tx)
    .await
    .expect("request fixture");
    catat(&mut tx, id, None, awal).await;

    // Percobaan yang sudah selesai sebelumnya (menunggu retry atau sebelum
    // percobaan berjalan).
    let tertutup = if async_ { attempt_awal } else { 0 };
    for no in 1..=tertutup {
        log(
            &mut tx,
            g,
            endpoint,
            id,
            no,
            sekarang - Duration::hours(2),
            true,
        )
        .await;
    }

    if async_ && r.status != "queued" {
        sqlx::query(
            "UPDATE request SET status = 'dispatched', dispatched_at = $2 WHERE request_id = $1",
        )
        .bind(id)
        .bind(r.dispatched_at.unwrap_or(sekarang))
        .execute(&mut *tx)
        .await
        .expect("fixture dispatched");
        catat(&mut tx, id, Some("queued"), "dispatched").await;
    }
    let mut kode_terakhir = None;
    if lewat_worker {
        sqlx::query(
            "UPDATE request SET status = 'in_flight', attempt_count = $2 WHERE request_id = $1",
        )
        .bind(id)
        .bind(r.attempt)
        .execute(&mut *tx)
        .await
        .expect("fixture in_flight");
        catat(&mut tx, id, Some("dispatched"), "in_flight").await;
    }
    if lewat_worker || !async_ {
        kode_terakhir = Some(
            log(
                &mut tx,
                g,
                endpoint,
                id,
                r.attempt,
                sekarang - r.umur_log,
                false,
            )
            .await,
        );
    }
    if r.status == "done" {
        let kode = kode_terakhir.expect("log percobaan");
        sqlx::query(
            "UPDATE transaction_log SET status_akses = 'berhasil', responded_at = requested_at + interval '1 second'
              WHERE kode_transaksi = $1",
        )
        .bind(kode)
        .execute(&mut *tx)
        .await
        .expect("fixture log berhasil");
        sqlx::query(
            "UPDATE request
                SET status = 'done', output_payload = '{\"cocok\": true}'::jsonb, sumber_hasil = 'provider',
                    final_kode_transaksi = $2, completed_at = $3,
                    webhook_status = CASE WHEN mode = 'async' THEN $4 END
              WHERE request_id = $1",
        )
        .bind(id)
        .bind(kode)
        .bind(r.selesai.unwrap_or(sekarang))
        .bind(r.webhook_status_done)
        .execute(&mut *tx)
        .await
        .expect("fixture done");
        catat(&mut tx, id, Some("in_flight"), "done").await;
    }
    tx.commit().await.expect("commit fixture request");
    id
}

/// Status, alasan, dan beberapa kolom hasil satu request (dibaca superuser).
#[derive(Debug, sqlx::FromRow)]
pub struct Baris {
    pub status: String,
    pub attempt_count: i32,
    pub cancel_reason: Option<String>,
    pub cancelled_by: Option<String>,
    pub failure_reason: Option<String>,
    pub webhook_status: Option<String>,
    pub upstream_pool_id: Option<Uuid>,
    pub not_before: Option<DateTime<Utc>>,
    pub final_kode_transaksi: Option<Uuid>,
    pub input_kosong: bool,
    pub output_kosong: bool,
    pub output_purged: bool,
}

pub async fn baca(su: &PgPool, id: Uuid) -> Baris {
    sqlx::query_as(
        "SELECT status, attempt_count, cancel_reason, cancelled_by, failure_reason, webhook_status,
                upstream_pool_id, not_before, final_kode_transaksi,
                input_payload IS NULL AS input_kosong, output_payload IS NULL AS output_kosong,
                output_purged_at IS NOT NULL AS output_purged
           FROM request WHERE request_id = $1",
    )
    .bind(id)
    .fetch_one(su)
    .await
    .expect("baca request")
}

/// Baris history request, urut `seq`: `(status_dari, status_ke, alasan, pemicu)`.
pub async fn history(
    su: &PgPool,
    id: Uuid,
) -> Vec<(Option<String>, String, Option<String>, Option<Uuid>)> {
    sqlx::query_as(
        "SELECT status_dari, status_ke, alasan, objek_pemicu_id
           FROM request_status_history WHERE request_id = $1 ORDER BY seq",
    )
    .bind(id)
    .fetch_all(su)
    .await
    .expect("baca history")
}

/// `status_akses` log percobaan terakhir.
pub async fn status_log_terakhir(su: &PgPool, id: Uuid) -> Option<String> {
    sqlx::query_scalar(
        "SELECT status_akses FROM transaction_log WHERE request_id = $1 ORDER BY attempt_no DESC LIMIT 1",
    )
    .bind(id)
    .fetch_one(su)
    .await
    .expect("baca log")
}

/// Fault injection untuk uji galat di tengah siklus: setiap `UPDATE` pada
/// baris `request` itu gagal dengan galat PostgreSQL sampai [`lepas_galat`]
/// dipanggil. Baris lain tidak terpengaruh, jadi transisi yang diproses lebih
/// dulu tetap commit. Mengembalikan nama trigger.
pub async fn pasang_galat_update(su: &PgPool, request_id: Uuid) -> String {
    let nama = format!("uji_galat_{}", request_id.simple());
    let fungsi = format!(
        "CREATE FUNCTION {nama}() RETURNS trigger LANGUAGE plpgsql AS $$
         BEGIN RAISE EXCEPTION 'galat uji fault injection'; END $$"
    );
    sqlx::query(sqlx::AssertSqlSafe(fungsi.as_str()))
        .execute(su)
        .await
        .expect("fungsi galat uji");
    let trigger = format!(
        "CREATE TRIGGER {nama} BEFORE UPDATE ON request FOR EACH ROW
         WHEN (OLD.request_id = '{request_id}'::uuid) EXECUTE FUNCTION {nama}()"
    );
    sqlx::query(sqlx::AssertSqlSafe(trigger.as_str()))
        .execute(su)
        .await
        .expect("trigger galat uji");
    nama
}

/// Melepas trigger dan fungsi dari [`pasang_galat_update`].
pub async fn lepas_galat(su: &PgPool, nama: &str) {
    for sql in [
        format!("DROP TRIGGER IF EXISTS {nama} ON request"),
        format!("DROP FUNCTION IF EXISTS {nama}()"),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
            .execute(su)
            .await
            .expect("lepas galat uji");
    }
}
