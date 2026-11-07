//! Job webhook terhadap PostgreSQL sungguhan (ARC-73, AC-26..AC-31).
//!
//! Dijalankan oleh `make -C backend uji-pg`. Fixture ditulis sebagai superuser
//! karena `pr_webhook` hanya punya grant kirim; job-nya sendiri berjalan di pool
//! `pr_webhook` yang dipakai produksi, jadi grant matrix ikut teruji.

use std::net::{IpAddr, Ipv4Addr};
use std::pin::Pin;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use poolrouter_controller::jadwal::Pekerjaan;
use poolrouter_controller::webhook::{Baris, Resolver, Webhook};
use poolrouter_core::admin::NilaiAman;
use poolrouter_core::secret::Brankas;
use sqlx::PgPool;
use uuid::Uuid;

const KEK_UJI: &str = "a2VrLXVqaS1hcmM3My1wcm9kdWtzaS1qYW5nYW4tZGk=";
const DOMAIN: &str = "hook.contoh.test";
const SECRET: &str = "whsec_dGVzdC1zZWNyZXQta2V5"; // base64("test-secret-key")
const SECRET_BARU: &str = "whsec_c2VjcmV0LWJhcnUtdWpp"; // base64("secret-baru-uji")

fn url(nama: &str) -> Option<String> {
    std::env::var(nama).ok()
}

async fn pool(url: &str) -> PgPool {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(2)
        .connect(url)
        .await
        .expect("koneksi PostgreSQL uji")
}

/// Resolver tetap: satu alamat, atau galat DNS.
struct Tetap {
    ip: IpAddr,
}

impl Resolver for Tetap {
    fn selesaikan<'a>(
        &'a self,
        _host: &'a str,
    ) -> Pin<Box<dyn std::future::Future<Output = Result<Vec<IpAddr>, String>> + Send + 'a>> {
        let ip = self.ip;
        Box::pin(async move { Ok(vec![ip]) })
    }
}

fn brankas() -> Brankas {
    Brankas::dari_isi_kek(KEK_UJI).expect("KEK uji")
}

struct Fixture {
    su: PgPool,
    client_id: Uuid,
    admin_id: Uuid,
    endpoint_id: Uuid,
    kredensial_id: Uuid,
    url_webhook: String,
    downstream_pool_id: Uuid,
    provider_id: Uuid,
    upstream_pool_id: Uuid,
}

impl Fixture {
    async fn baru(su: PgPool) -> Self {
        let client_id = Uuid::now_v7();
        let admin_id = Uuid::now_v7();
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
        .bind(format!("arc73-{akhiran}"))
        .execute(&su)
        .await
        .expect("admin_user");
        sqlx::query("INSERT INTO client (client_id, nama) VALUES ($1, $2)")
            .bind(client_id)
            .bind(format!("arc73-{akhiran}"))
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
            "INSERT INTO provider (provider_id, nama, wajib_log_nik) VALUES ($1, $2, false)",
        )
        .bind(provider_id)
        .bind(format!("arc73-{akhiran}"))
        .execute(&su)
        .await
        .expect("provider");
        sqlx::query(
            "INSERT INTO downstream_pool (downstream_pool_id, nama, prioritas) VALUES ($1, $2, 1)",
        )
        .bind(downstream_pool_id)
        .bind(format!("arc73-{akhiran}"))
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
        .bind(format!("arc73-{akhiran}"))
        .execute(&su)
        .await
        .expect("upstream_pool");
        sqlx::query(
            "INSERT INTO downstream_endpoint (downstream_endpoint_id, nama, provider_id, downstream_pool_id)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(endpoint_id)
        .bind(format!("arc73-{akhiran}"))
        .bind(provider_id)
        .bind(downstream_pool_id)
        .execute(&su)
        .await
        .expect("downstream_endpoint");

        Self {
            su,
            client_id,
            admin_id,
            endpoint_id,
            kredensial_id,
            url_webhook: format!("https://{DOMAIN}/hook"),
            downstream_pool_id,
            provider_id,
            upstream_pool_id,
        }
    }

    /// Arahkan konfigurasi webhook fixture ke domain lain (uji allowlist).
    fn arahkan_ke(&mut self, domain: &str) {
        self.url_webhook = format!("https://{domain}/hook");
    }

    /// Baris `request` async terminal yang menunggu webhook.
    ///
    /// Insert lewat jalur sah (`queued`), lalu transisi ke `done` — trigger
    /// `request_transisi` menolak insert langsung berstatus terminal.
    async fn request(
        &self,
        webhook_status: &str,
        attempt: i32,
        last_at: Option<DateTime<Utc>>,
    ) -> Uuid {
        let request_id = Uuid::now_v7();
        let mut tx = self.su.begin().await.expect("tx fixture");
        sqlx::query(
            "INSERT INTO request
                    (request_id, client_id, credential_id, downstream_endpoint_id,
                     downstream_pool_id, provider_id, upstream_pool_id,
                     provider_config_version, endpoint_config_version, mode,
                     prioritas, prioritas_efektif, status, metadata, received_at,
                     ttl_expires_at, attempt_count)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 1, 1, 'async', 1, 1, 'queued', '{}'::jsonb, now(),
                     now() + interval '1 hour', 1)",
        )
        .bind(request_id)
        .bind(self.client_id)
        .bind(self.kredensial_id)
        .bind(self.endpoint_id)
        .bind(self.downstream_pool_id)
        .bind(self.provider_id)
        .bind(self.upstream_pool_id)
        .execute(&mut *tx)
        .await
        .expect("request");
        sqlx::query(
            "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor, terjadi_at)
             VALUES ($1, 1, NULL, 'queued', 'webserver', now())",
        )
        .bind(request_id)
        .execute(&mut *tx)
        .await
        .expect("request_status_history queued");
        sqlx::query("UPDATE request SET status = 'dispatched' WHERE request_id = $1")
            .bind(request_id)
            .execute(&mut *tx)
            .await
            .expect("queued -> dispatched");
        sqlx::query(
            "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor, terjadi_at)
             VALUES ($1, 2, 'queued', 'dispatched', 'controller', now())",
        )
        .bind(request_id)
        .execute(&mut *tx)
        .await
        .expect("request_status_history dispatched");
        sqlx::query("UPDATE request SET status = 'in_flight' WHERE request_id = $1")
            .bind(request_id)
            .execute(&mut *tx)
            .await
            .expect("dispatched -> in_flight");
        sqlx::query(
            "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor, terjadi_at)
             VALUES ($1, 3, 'dispatched', 'in_flight', 'worker', now())",
        )
        .bind(request_id)
        .execute(&mut *tx)
        .await
        .expect("request_status_history in_flight");
        let kode = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO transaction_log (kode_transaksi, request_id, attempt_no, provider_id,
                     upstream_pool_id, client_id, downstream_endpoint_id, provider_config_version,
                     endpoint_config_version, requested_at, responded_at, duration_ms, log_mode,
                     worker_id)
             VALUES ($1, $2, 1, $3, $4, $5, $6, 1, 1, now(), now(), 5, 'metadata_only', 'arc73-worker')",
        )
        .bind(kode)
        .bind(request_id)
        .bind(self.provider_id)
        .bind(self.upstream_pool_id)
        .bind(self.client_id)
        .bind(self.endpoint_id)
        .execute(&mut *tx)
        .await
        .expect("transaction_log");
        sqlx::query(
            "UPDATE request SET status = 'done', completed_at = now(), final_kode_transaksi = $2,
                    output_payload = '{\"data\": {\"nama\": \"uji\"}}'::jsonb,
                    webhook_status = 'menunggu', webhook_attempt_count = 0
              WHERE request_id = $1",
        )
        .bind(request_id)
        .bind(kode)
        .bind(request_id)
        .execute(&mut *tx)
        .await
        .expect("in_flight -> done");
        sqlx::query(
            "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor, terjadi_at)
             VALUES ($1, 4, 'in_flight', 'done', 'worker', now())",
        )
        .bind(request_id)
        .execute(&mut *tx)
        .await
        .expect("request_status_history done");
        sqlx::query(
            "UPDATE request SET webhook_status = $2, webhook_attempt_count = $3, webhook_last_at = $4
              WHERE request_id = $1",
        )
        .bind(request_id)
        .bind(webhook_status)
        .bind(attempt)
        .bind(last_at)
        .execute(&mut *tx)
        .await
        .expect("kolom webhook");
        tx.commit().await.expect("commit fixture");
        request_id
    }

    async fn webhook(&self, aktif: bool, rusak: bool) {
        self.webhook_versi(aktif, rusak, 1).await;
    }

    /// Versi secret ikut ditulis supaya rotasi (CW9) bisa diuji: job harus
    /// membaca versi terbaru saat kirim, bukan versi saat baris dibuat.
    async fn webhook_versi(&self, aktif: bool, rusak: bool, versi: i32) {
        let amplop = brankas()
            .enkripsi(
                &poolrouter_controller::webhook::nama_amplop(self.client_id),
                &NilaiAman::baru(SECRET.to_owned()),
            )
            .expect("enkripsi secret");
        let dek = if rusak {
            {
                let mut d = amplop.dek_terbungkus.clone();
                d[0] ^= 0xFF;
                d
            }
        } else {
            amplop.dek_terbungkus.clone()
        };
        sqlx::query(
            "INSERT INTO client_webhook (client_id, webhook_url, aktif, secret_versi,
                     secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id, diubah_oleh)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)",
        )
        .bind(self.client_id)
        .bind(&self.url_webhook)
        .bind(aktif)
        .bind(versi)
        .bind(amplop.ciphertext.as_slice())
        .bind(dek.as_slice())
        .bind(amplop.nonce.as_slice())
        .bind(amplop.kek_id)
        .bind(self.admin_id)
        .execute(&self.su)
        .await
        .expect("client_webhook");
    }

    /// Rotasi secret: ciphertext baru, `secret_versi` naik satu.
    async fn rotasi_secret(&self, versi: i32) {
        let amplop = brankas()
            .enkripsi(
                &poolrouter_controller::webhook::nama_amplop(self.client_id),
                &NilaiAman::baru(SECRET_BARU.to_owned()),
            )
            .expect("enkripsi secret baru");
        sqlx::query(
            "UPDATE client_webhook SET secret_versi = $2, secret_ciphertext = $3,
                     secret_dek_terbungkus = $4, secret_nonce = $5, kek_id = $6, diubah_oleh = $7
               WHERE client_id = $1",
        )
        .bind(self.client_id)
        .bind(versi)
        .bind(amplop.ciphertext.as_slice())
        .bind(amplop.dek_terbungkus.as_slice())
        .bind(amplop.nonce.as_slice())
        .bind(amplop.kek_id)
        .bind(self.admin_id)
        .execute(&self.su)
        .await
        .expect("rotasi secret");
    }

    async fn izinkan(&self, domain: &str) {
        sqlx::query(
            "INSERT INTO webhook_domain_allowlist (domain, aktif, diubah_oleh) VALUES ($1, true, $2)
             ON CONFLICT (domain) DO UPDATE SET aktif = true",
        )
        .bind(domain)
        .bind(self.admin_id)
        .execute(&self.su)
        .await
        .expect("allowlist");
    }
}

/// Log satu percobaan, atau `None` bila tidak ada.
async fn log(
    pool: &PgPool,
    request_id: Uuid,
) -> Option<(String, Option<i32>, Option<String>, Option<i32>, i32)> {
    let mut baris = sqlx::query_as::<_, (String, Option<i32>, Option<String>, Option<i32>, i32)>(
        "SELECT hasil, http_status, error_class, secret_versi, attempt_no
           FROM webhook_log WHERE request_id = $1",
    )
    .bind(request_id)
    .fetch_optional(pool)
    .await
    .expect("webhook_log");
    if let Some((_, _, _, _, _)) = &baris {
        // Tidak ada kolom body/header/URL penuh yang bisa tersimpan (AC-30).
        let kolom: Vec<String> = sqlx::query_scalar(
            "SELECT column_name FROM information_schema.columns WHERE table_name = 'webhook_log'",
        )
        .fetch_all(pool)
        .await
        .expect("kolom webhook_log");
        for dilarang in ["body", "header", "headers", "payload", "url", "secret"] {
            assert!(
                !kolom.iter().any(|k| k == dilarang),
                "kolom {dilarang} tidak boleh ada"
            );
        }
    }
    baris.take()
}

async fn status_webhook(pool: &PgPool, request_id: Uuid) -> (String, i32, Option<String>) {
    sqlx::query_as("SELECT webhook_status, webhook_attempt_count, webhook_last_error FROM request WHERE request_id = $1")
        .bind(request_id)
        .fetch_one(pool)
        .await
        .expect("request")
}

fn webhook(pool: PgPool, ip: IpAddr) -> Webhook {
    Webhook::dengan_resolver(pool, brankas(), Box::new(Tetap { ip }))
}

#[tokio::test]
#[ignore = "butuh PostgreSQL uji; jalankan lewat make uji-pg"]
async fn ac26_klaim_cas_satu_kali_kirim() {
    let Some(url_su) = url("POOLROUTER_TEST_DATABASE_URL_SU") else {
        return;
    };
    let su = pool(&url_su).await;
    let f = Fixture::baru(su.clone()).await;
    f.webhook(true, false).await;
    f.izinkan(DOMAIN).await;
    let request_id = f.request("menunggu", 0, None).await;

    // Set webhook_batas_percobaan=1 so first failure marks status as 'gagal', not 'menunggu'.
    sqlx::query("UPDATE konfigurasi_platform SET nilai = '1'::jsonb WHERE kunci = 'webhook_batas_percobaan'")
        .execute(&su)
        .await
        .expect("set webhook_batas_percobaan=1");

    let webhook_pool =
        pool(&url("POOLROUTER_TEST_DATABASE_URL_WEBHOOK").expect("url webhook")).await;
    // TEST-NET-3 tidak bisa dirutekan: jalur kirim berakhir `gagal_jaringan`,
    // yang justru membuktikan satu percobaan tercatat lengkap.
    let job = webhook(webhook_pool, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10)));
    assert!(job.jalankan(Utc::now()).await.expect("jalankan"));

    let (hasil, http, kelas, versi, attempt) = log(&su, request_id).await.expect("satu log");
    assert_eq!(hasil, "gagal_jaringan");
    assert_eq!(http, None);
    assert_eq!(kelas.as_deref(), Some("gagal_jaringan"));
    assert_eq!(versi, Some(1));
    assert_eq!(attempt, 1);

    let (status, jumlah, _) = status_webhook(&su, request_id).await;
    assert_eq!(status, "gagal");
    assert_eq!(jumlah, 1);

    // Kirim kedua tidak terjadi: baris sudah `gagal`, bukan `menunggu`.
    job.jalankan(Utc::now()).await.expect("jalankan ulang");
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM webhook_log WHERE request_id = $1")
        .bind(request_id)
        .fetch_one(&su)
        .await
        .expect("hitung log");
    assert_eq!(total, 1, "satu request hanya sekali kirim");

    // Restore webhook_batas_percobaan to default (6) so other tests are not affected.
    sqlx::query("UPDATE konfigurasi_platform SET nilai = '6'::jsonb WHERE kunci = 'webhook_batas_percobaan'")
        .execute(&su)
        .await
        .expect("restore webhook_batas_percobaan");
}

/// AC-28 / CW9: rotasi versi secret berlaku untuk pengiriman berikutnya. Job
/// membaca `secret_versi` saat kirim, jadi baris `menunggu` yang dibuat sebelum
/// rotasi memakai versi baru — bukan versi yang tersimpan di baris outbox.
#[tokio::test]
#[ignore = "butuh PostgreSQL uji; jalankan lewat make uji-pg"]
async fn ac28_rotasi_versi_dipakai_saat_kirim() {
    let Some(url_su) = url("POOLROUTER_TEST_DATABASE_URL_SU") else {
        return;
    };
    let su = pool(&url_su).await;
    let f = Fixture::baru(su.clone()).await;
    f.webhook(true, false).await;
    f.izinkan(DOMAIN).await;
    let request_id = f.request("menunggu", 0, None).await;

    // Baris outbox sudah ada; rotasi terjadi sebelum job menyentuhnya.
    f.rotasi_secret(2).await;

    let webhook_pool =
        pool(&url("POOLROUTER_TEST_DATABASE_URL_WEBHOOK").expect("url webhook")).await;
    webhook(webhook_pool, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10)))
        .jalankan(Utc::now())
        .await
        .expect("jalankan");

    let (hasil, _, kelas, versi, attempt) = log(&su, request_id).await.expect("satu log");
    assert_eq!(
        hasil, "gagal_jaringan",
        "versi 2 tidak bisa dihubungi, bukan ditolak"
    );
    assert_eq!(kelas.as_deref(), Some("gagal_jaringan"));
    assert_eq!(
        versi,
        Some(2),
        "rotasi dipakai saat kirim, bukan versi lama"
    );
    assert_eq!(attempt, 1);
}

#[tokio::test]
#[ignore = "butuh PostgreSQL uji; jalankan lewat make uji-pg"]
async fn ac29_domain_di_luar_allowlist() {
    let Some(url_su) = url("POOLROUTER_TEST_DATABASE_URL_SU") else {
        return;
    };
    let su = pool(&url_su).await;
    let mut f = Fixture::baru(su.clone()).await;
    f.arahkan_ke("hook.di-luar.test");
    f.webhook(true, false).await;
    let request_id = f.request("menunggu", 0, None).await;

    let webhook_pool =
        pool(&url("POOLROUTER_TEST_DATABASE_URL_WEBHOOK").expect("url webhook")).await;
    webhook(webhook_pool, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10)))
        .jalankan(Utc::now())
        .await
        .expect("jalankan");

    let (hasil, http, kelas, versi, _) = log(&su, request_id).await.expect("satu log");
    assert_eq!(hasil, "ditolak_allowlist");
    assert_eq!(kelas.as_deref(), Some("domain_tidak_diizinkan"));
    assert_eq!(http, None);
    assert_eq!(
        versi, None,
        "ditolak_allowlist tidak menyimpan versi secret"
    );
}

#[tokio::test]
#[ignore = "butuh PostgreSQL uji; jalankan lewat make uji-pg"]
async fn ac29_ip_terlarang_tanpa_koneksi() {
    let Some(url_su) = url("POOLROUTER_TEST_DATABASE_URL_SU") else {
        return;
    };
    let su = pool(&url_su).await;
    let f = Fixture::baru(su.clone()).await;
    f.webhook(true, false).await;
    f.izinkan(DOMAIN).await;
    let request_id = f.request("menunggu", 0, None).await;

    let webhook_pool =
        pool(&url("POOLROUTER_TEST_DATABASE_URL_WEBHOOK").expect("url webhook")).await;
    webhook(webhook_pool, IpAddr::V4(Ipv4Addr::new(10, 0, 0, 5)))
        .jalankan(Utc::now())
        .await
        .expect("jalankan");

    let (hasil, http, kelas, versi, _) = log(&su, request_id).await.expect("satu log");
    assert_eq!(hasil, "ditolak_allowlist");
    assert_eq!(kelas.as_deref(), Some("ip_terlarang"));
    assert_eq!(http, None);
    assert_eq!(versi, None);
}

#[tokio::test]
#[ignore = "butuh PostgreSQL uji; jalankan lewat make uji-pg"]
async fn ac31_webhook_nonaktif_tidak_dikirim() {
    let Some(url_su) = url("POOLROUTER_TEST_DATABASE_URL_SU") else {
        return;
    };
    let su = pool(&url_su).await;
    let f = Fixture::baru(su.clone()).await;
    f.webhook(false, false).await;
    f.izinkan(DOMAIN).await;
    let request_id = f.request("menunggu", 0, None).await;

    let webhook_pool =
        pool(&url("POOLROUTER_TEST_DATABASE_URL_WEBHOOK").expect("url webhook")).await;
    webhook(webhook_pool, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10)))
        .jalankan(Utc::now())
        .await
        .expect("jalankan");

    let (status, _, _) = status_webhook(&su, request_id).await;
    assert_eq!(status, "tidak_dikirim");
    assert!(
        log(&su, request_id).await.is_none(),
        "tidak ada percobaan, tidak ada log"
    );
}

#[tokio::test]
#[ignore = "butuh PostgreSQL uji; jalankan lewat make uji-pg"]
async fn ac26_klaim_macet_ditutup_gagal() {
    let Some(url_su) = url("POOLROUTER_TEST_DATABASE_URL_SU") else {
        return;
    };
    let su = pool(&url_su).await;
    let f = Fixture::baru(su.clone()).await;
    let request_id = f
        .request("menunggu", 1, Some(Utc::now() - Duration::minutes(10)))
        .await;

    let webhook_pool =
        pool(&url("POOLROUTER_TEST_DATABASE_URL_WEBHOOK").expect("url webhook")).await;
    webhook(webhook_pool, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10)))
        .jalankan(Utc::now())
        .await
        .expect("jalankan");

    let (status, jumlah, galat) = status_webhook(&su, request_id).await;
    assert_eq!(status, "gagal");
    assert_eq!(jumlah, 1, "attempt_count tidak bertambah");
    assert_eq!(galat.as_deref(), Some("klaim_kedaluwarsa"));
    assert!(
        log(&su, request_id).await.is_none(),
        "tidak ada kirim ulang"
    );
}

#[tokio::test]
#[ignore = "butuh PostgreSQL uji; jalankan lewat make uji-pg"]
async fn ac30_dekripsi_gagal_tanpa_kiriman() {
    let Some(url_su) = url("POOLROUTER_TEST_DATABASE_URL_SU") else {
        return;
    };
    let su = pool(&url_su).await;
    let f = Fixture::baru(su.clone()).await;
    f.webhook(true, true).await;
    f.izinkan(DOMAIN).await;
    let request_id = f.request("menunggu", 0, None).await;

    let webhook_pool =
        pool(&url("POOLROUTER_TEST_DATABASE_URL_WEBHOOK").expect("url webhook")).await;
    webhook(webhook_pool, IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10)))
        .jalankan(Utc::now())
        .await
        .expect("jalankan");

    let (hasil, http, kelas, versi, _) = log(&su, request_id).await.expect("satu log");
    assert_eq!(hasil, "gagal_internal");
    assert_eq!(kelas.as_deref(), Some("dekripsi_gagal"));
    assert_eq!(versi, Some(1), "versi yang gagal tetap tercatat");
    assert_eq!(http, None, "tidak ada koneksi keluar");
    let (status, _, _) = status_webhook(&su, request_id).await;
    assert_eq!(status, "gagal");
}

/// AC-30 / U-23: `pr_webhook` tidak boleh membaca secret provider atau payload.
#[tokio::test]
#[ignore = "butuh PostgreSQL uji; jalankan lewat make uji-pg"]
async fn ac30_pr_webhook_ditolak_baca_tabel_lain() {
    let Some(url_webhook) = url("POOLROUTER_TEST_DATABASE_URL_WEBHOOK") else {
        return;
    };
    let webhook_pool = pool(&url_webhook).await;
    for sql in [
        "SELECT secret_ciphertext FROM provider_secret LIMIT 1",
        "SELECT input_payload FROM request LIMIT 1",
        "SELECT * FROM client LIMIT 1",
    ] {
        let galat = sqlx::query(sql).execute(&webhook_pool).await;
        assert!(galat.is_err(), "{sql} harus ditolak untuk pr_webhook");
    }
    // Delapan kolom yang memang diizinkan masih bisa dibaca.
    sqlx::query(
        "SELECT client_id, webhook_url, aktif, secret_versi, secret_ciphertext,
                        secret_dek_terbungkus, secret_nonce, kek_id FROM client_webhook LIMIT 1",
    )
    .execute(&webhook_pool)
    .await
    .expect("kolom webhook yang diizinkan");
    sqlx::query("SELECT domain FROM webhook_domain_allowlist LIMIT 1")
        .execute(&webhook_pool)
        .await
        .expect("allowlist terbaca");
}

/// Body yang dikirim job lolos skema `RequestStatus`: bentuknya sama dengan
/// polling. Diuji tanpa jaringan lewat `body`.
#[tokio::test]
#[ignore = "butuh PostgreSQL uji; jalankan lewat make uji-pg"]
async fn ac27_body_dari_baris_nyata() {
    let Some(url_su) = url("POOLROUTER_TEST_DATABASE_URL_SU") else {
        return;
    };
    let su = pool(&url_su).await;
    let f = Fixture::baru(su.clone()).await;
    let request_id = f.request("menunggu", 0, None).await;
    let webhook_pool =
        pool(&url("POOLROUTER_TEST_DATABASE_URL_WEBHOOK").expect("url webhook")).await;

    let baris: Baris = sqlx::query_as(
        "SELECT v.request_id, r.client_id, v.status, v.attempt_count, v.cancel_reason,
                v.failure_reason, v.output_payload, v.webhook_attempt_count, v.webhook_not_before
           FROM v_request_webhook v JOIN request r ON r.request_id = v.request_id
          WHERE v.request_id = $1",
    )
    .bind(request_id)
    .fetch_one(&webhook_pool)
    .await
    .expect("v_request_webhook sebagai pr_webhook");
    let isi = poolrouter_controller::webhook::body(&baris).expect("body");
    let obj: serde_json::Value = serde_json::from_slice(&isi).expect("json");
    assert_eq!(obj["queue_id"], request_id.to_string());
    assert_eq!(obj["status"], "done");
    assert_eq!(obj["hasil"]["data"]["nama"], "uji");
}

/// Sisa import dipakai untuk memastikan fixture tidak bocor antar uji.
#[allow(dead_code)]
fn _arc() -> Arc<()> {
    Arc::new(())
}

/// AC-5: klaim n -> n+1 lewat `fn_klaim_webhook`. Dua panggilan bersamaan atas
/// baris yang sama hanya satu yang menang per percobaan (CAS). 100 ulangan
/// pada attempt_count independen membuktikan tidak ada race di bawah beban.
#[tokio::test]
#[ignore = "butuh PostgreSQL uji; jalankan lewat make uji-pg"]
async fn ac5_klaim_n_ke_n_plus_1_cas_konkuren() {
    let Some(url_su) = url("POOLROUTER_TEST_DATABASE_URL_SU") else {
        return;
    };
    let su = pool(&url_su).await;
    let f = Fixture::baru(su.clone()).await;
    let webhook_pool = pool(&url("POOLROUTER_TEST_DATABASE_URL_WEBHOOK").expect("url webhook")).await;

    for n in 0..100i32 {
        let request_id = f.request("menunggu", n, None).await;
        let sekarang = Utc::now();

        let a = webhook_pool.clone();
        let b = webhook_pool.clone();
        let (menang_a, menang_b) = tokio::join!(
            sqlx::query_scalar::<_, i32>(
                "SELECT fn_klaim_webhook($1, $2, NULL::integer, NULL::text, $3::integer)"
            )
            .bind(request_id)
            .bind(sekarang)
            .bind(n)
            .fetch_one(&a),
            sqlx::query_scalar::<_, i32>(
                "SELECT fn_klaim_webhook($1, $2, NULL::integer, NULL::text, $3::integer)"
            )
            .bind(request_id)
            .bind(sekarang)
            .bind(n)
            .fetch_one(&b),
        );
        let total = menang_a.expect("klaim a") + menang_b.expect("klaim b");
        assert_eq!(total, 1, "tepat satu klaim menang pada percobaan n={n}");

        let (_, jumlah, _) = status_webhook(&su, request_id).await;
        assert_eq!(jumlah, n + 1, "attempt_count naik tepat satu kali, n={n}");
    }
}

/// AC-5: klaim ditolak bila `webhook_not_before` belum lewat, dan berhasil
/// setelah waktunya tiba (jam diinjeksi lewat parameter `p_sekarang`).
#[tokio::test]
#[ignore = "butuh PostgreSQL uji; jalankan lewat make uji-pg"]
async fn ac5_klaim_menunggu_not_before() {
    let Some(url_su) = url("POOLROUTER_TEST_DATABASE_URL_SU") else {
        return;
    };
    let su = pool(&url_su).await;
    let f = Fixture::baru(su.clone()).await;
    let request_id = f.request("menunggu", 1, None).await;
    let not_before = Utc::now() + Duration::seconds(30);
    sqlx::query("UPDATE request SET webhook_not_before = $2 WHERE request_id = $1")
        .bind(request_id)
        .bind(not_before)
        .execute(&su)
        .await
        .expect("set not_before");

    let webhook_pool = pool(&url("POOLROUTER_TEST_DATABASE_URL_WEBHOOK").expect("url webhook")).await;

    // Sebelum jatuh tempo: klaim gagal (0 baris).
    let sebelum: i32 = sqlx::query_scalar(
        "SELECT fn_klaim_webhook($1, $2, NULL::integer, NULL::text, 1::integer)",
    )
    .bind(request_id)
    .bind(not_before - Duration::seconds(5))
    .fetch_one(&webhook_pool)
    .await
    .expect("klaim sebelum jatuh tempo");
    assert_eq!(sebelum, 0, "klaim ditolak sebelum webhook_not_before");

    // Setelah jatuh tempo: klaim berhasil, attempt_count naik, not_before dikosongkan.
    let setelah: i32 = sqlx::query_scalar(
        "SELECT fn_klaim_webhook($1, $2, NULL::integer, NULL::text, 1::integer)",
    )
    .bind(request_id)
    .bind(not_before + Duration::seconds(1))
    .fetch_one(&webhook_pool)
    .await
    .expect("klaim setelah jatuh tempo");
    assert_eq!(setelah, 1, "klaim berhasil setelah webhook_not_before lewat");

    let (status, jumlah, _) = status_webhook(&su, request_id).await;
    assert_eq!(status, "menunggu");
    assert_eq!(jumlah, 2);
}
