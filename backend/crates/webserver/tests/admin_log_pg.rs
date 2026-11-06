//! Log, NIK search, payload opening, export and audit reading over HTTP against
//! a real PostgreSQL (ARC-52; AC-32..AC-38). Run by `make -C backend uji-pg`;
//! `#[ignore]`d because it needs `POOLROUTER_TEST_DATABASE_URL`.
//!
//! The audit assertions read `audit_akses` as `pr_admin_api`, which is the same
//! role the handlers use: if a row is missing there, the real trail is missing
//! too. `POOLROUTER_TEST_DATABASE_URL_SU` is only used to seed `transaction_log`,
//! whose insert the admin role deliberately cannot do. Every NIK, payload and
//! reason below is synthetic.

use std::io::Write as _;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use chrono::{DateTime, TimeZone, Utc};
use poolrouter_core::admin::AsalUi;
use poolrouter_core::admin::sesi_store::SesiStore;
use poolrouter_core::admin::token::TokenSesi;
use poolrouter_core::secret::Brankas;
use poolrouter_webserver::admin::pembatas::PembatasLogin;
use poolrouter_webserver::admin::sesi::{AdminState, COOKIE_SESI, router};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const ASAL: &str = "https://admin.poolrouter.test";
const KEK_HEX: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
const KUNCI_HMAC: &[u8] = b"kunci-hmac-sintetis-untuk-uji";

/// A syntactically valid, entirely synthetic NIK, unique per call so tests that
/// share one database cannot see each other's `transaction_log` rows (the seed
/// prefix `00` keeps it clearly fake).
fn nik_sintetis() -> String {
    let n = Uuid::now_v7().as_u128() % 100_000_000_000_000;
    format!("00{n:014}")
}

fn redis_ujian() -> String {
    std::env::var("POOLROUTER_TEST_REDIS")
        .expect("POOLROUTER_TEST_REDIS belum diset; jalankan lewat `make -C backend uji-pg`")
}

fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 5, 9, 0, 0).unwrap()
}

struct Uji {
    app: Router,
    pool: PgPool,
    su: PgPool,
}

impl Uji {
    async fn kirim(
        &self,
        method: Method,
        path: &str,
        cookie: Option<&str>,
        origin: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, HeaderMap, Vec<u8>) {
        let mut req = Request::builder().method(method).uri(path);
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, c);
        }
        if let Some(o) = origin {
            req = req.header(header::ORIGIN, o);
        }
        let req = match body {
            Some(b) => req
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(b.to_string()))
                .unwrap(),
            None => req.body(Body::empty()).unwrap(),
        };
        let res = self.app.clone().oneshot(req).await.unwrap();
        let status = res.status();
        let headers = res.headers().clone();
        let bytes = to_bytes(res.into_body(), 16 << 20).await.unwrap().to_vec();
        (status, headers, bytes)
    }

    /// JSON body, for the endpoints that answer JSON.
    async fn json(
        &self,
        method: Method,
        path: &str,
        cookie: Option<&str>,
        body: Option<Value>,
    ) -> (StatusCode, HeaderMap, Value) {
        let (st, h, b) = self.kirim(method, path, cookie, Some(ASAL), body).await;
        (st, h, serde_json::from_slice(&b).unwrap_or(Value::Null))
    }

    async fn user(&self, uid: Uuid) {
        sqlx::query(
            "INSERT INTO admin_user (pengguna_id, username, password_hash) VALUES ($1, $2, 'x')",
        )
        .bind(uid)
        .bind(format!("sintetis-{}", uid.simple()))
        .execute(&self.pool)
        .await
        .unwrap();
    }

    /// A user with the given roles and one live session; returns the cookie.
    /// `auditor`/`operator` need the separation-of-duties triple the schema
    /// enforces, so those roles get two extra synthetic users.
    async fn pengguna(&self, peran: &[&str]) -> (Uuid, String) {
        let uid = Uuid::now_v7();
        self.user(uid).await;
        for p in peran {
            let (pengaju, penyetuju) = if matches!(*p, "auditor" | "operator") {
                let a = Uuid::now_v7();
                let b = Uuid::now_v7();
                self.user(a).await;
                self.user(b).await;
                (Some(a), Some(b))
            } else {
                (None, None)
            };
            sqlx::query(
                "INSERT INTO admin_user_peran (pengguna_id, peran, diajukan_oleh, disetujui_oleh) \
                 VALUES ($1, $2, coalesce($3, $1), $4)",
            )
            .bind(uid)
            .bind(p)
            .bind(pengaju)
            .bind(penyetuju)
            .execute(&self.pool)
            .await
            .unwrap();
        }
        let token = TokenSesi::baru();
        SesiStore::baru(self.pool.clone())
            .terbitkan(uid, &token.hash(), Some("uji"), t0())
            .await
            .unwrap();
        (uid, format!("{COOKIE_SESI}={}", token.sebagai_b64()))
    }

    /// Seeds a self-contained synthetic log fixture: client, credential, pool,
    /// provider, endpoint and two `transaction_log` rows for the synthetic NIK —
    /// one with a payload, one without. Returns `(kode_berpayload,
    /// request_id_berpayload, nik)`.
    ///
    /// It writes as superuser because the admin role deliberately cannot insert
    /// configuration or log rows; the handlers are still exercised as
    /// `pr_admin_api`. Every identifier and value is synthetic.
    async fn siapkan_log(&self) -> (Uuid, Uuid, String) {
        let nik = nik_sintetis();
        let admin = Uuid::now_v7();
        self.user(admin).await;

        let client = Uuid::now_v7();
        sqlx::query("INSERT INTO client (client_id, nama) VALUES ($1, $2)")
            .bind(client)
            .bind(format!("klien-{}", client.simple()))
            .execute(&self.su)
            .await
            .unwrap();

        let kredensial = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO client_kredensial (credential_id, client_id, jenis, kunci_hash, dibuat_oleh) \
             VALUES ($1, $2, 'api_key', '\\x01', $3)",
        )
        .bind(kredensial)
        .bind(client)
        .bind(admin)
        .execute(&self.su)
        .await
        .unwrap();

        let provider = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO provider (provider_id, nama, data_balikan_aktif) VALUES ($1, $2, false)",
        )
        .bind(provider)
        .bind(format!("prov-{}", provider.simple()))
        .execute(&self.su)
        .await
        .unwrap();

        let pool_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO upstream_pool (upstream_pool_id, provider_id, nama, kuota_harian) VALUES ($1, $2, $3, 100)",
        )
        .bind(pool_id)
        .bind(provider)
        .bind(format!("pool-{}", pool_id.simple()))
        .execute(&self.su)
        .await
        .unwrap();

        let dpool = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO downstream_pool (downstream_pool_id, nama, prioritas) VALUES ($1, $2, 1)",
        )
        .bind(dpool)
        .bind(format!("dpool-{}", dpool.simple()))
        .execute(&self.su)
        .await
        .unwrap();
        sqlx::query("INSERT INTO downstream_pool_rute VALUES ($1, $2)")
            .bind(dpool)
            .bind(pool_id)
            .execute(&self.su)
            .await
            .unwrap();

        let endpoint = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO downstream_endpoint (downstream_endpoint_id, nama, provider_id, downstream_pool_id) \
             VALUES ($1, $2, $3, $4)",
        )
        .bind(endpoint)
        .bind(format!("ep-{}", endpoint.simple()))
        .bind(provider)
        .bind(dpool)
        .execute(&self.su)
        .await
        .unwrap();

        let hasil_nik = nik.clone();
        let mut hasil = (Uuid::nil(), Uuid::nil(), hasil_nik);
        for berpayload in [true, false] {
            let rid = Uuid::now_v7();
            let kt = Uuid::now_v7();
            // T-04 is a DEFERRABLE trigger: the request row and its first history
            // row have to be in one transaction, or autocommit fires the check
            // before the history row exists.
            let mut tx = self.su.begin().await.unwrap();
            sqlx::query(
                "INSERT INTO request (request_id, client_id, credential_id, downstream_endpoint_id, downstream_pool_id, \
                    provider_id, upstream_pool_id, provider_config_version, endpoint_config_version, mode, prioritas, \
                    prioritas_efektif, status, metadata, received_at, attempt_count, dispatch_key, \
                    final_kode_transaksi) \
                 VALUES ($1, $2, $3, $4, $5, $6, $7, 1, 1, 'sync', 1, 1, 'in_flight', '{}', $8, 1, NULL, $9)",
            )
            .bind(rid)
            .bind(client)
            .bind(kredensial)
            .bind(endpoint)
            .bind(dpool)
            .bind(provider)
            .bind(pool_id)
            .bind(t0())
            // No kode yet: `transaction_log` references `request_id`, so the kode
            // can only be filled in after that row exists. `chk_final_log_only`
            // wants it to stay NULL while the status is not terminal, so the
            // request starts `in_flight` and is closed in a second statement.
            .bind(Option::<Uuid>::None)
            .execute(&mut *tx)
            .await
            .unwrap();
            // seq = 1, `status_dari` NULL, transaction-time timestamp.
            sqlx::query(
                "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor, terjadi_at) \
                 VALUES ($1, 1, NULL, 'in_flight', 'webserver', now())",
            )
            .bind(rid)
            .execute(&mut *tx)
            .await
            .unwrap();
            tx.commit().await.unwrap();
            sqlx::query(
                "INSERT INTO transaction_log (kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id, \
                    client_id, downstream_endpoint_id, provider_config_version, endpoint_config_version, nik, \
                    requested_at, responded_at, duration_ms, status_akses, log_mode, request_payload, response_payload, worker_id) \
                 VALUES ($1, $2, 1, $3, $4, $5, $6, 1, 1, $7, $8, $8, 120, 'berhasil', $9, $10, $11, 'worker-uji')",
            )
            .bind(kt)
            .bind(rid)
            .bind(provider)
            .bind(pool_id)
            .bind(client)
            .bind(endpoint)
            .bind(nik.as_str())
            .bind(t0())
            .bind(if berpayload { "full_payload" } else { "metadata_only" })
            .bind(if berpayload {
                Some(json!({"input": "sintetis"}))
            } else {
                None
            })
            .bind(if berpayload {
                Some(json!({"hasil": "ok"}))
            } else {
                None
            })
            .execute(&self.su)
            .await
            .unwrap();
            // Close the request: `done` + `final_kode_transaksi` is what the
            // request_id -> kode lookup reads from `v_request_tersamar`. The
            // transition trigger wants the status change plus its history row
            // in the same transaction.
            let mut tx = self.su.begin().await.unwrap();
            sqlx::query(
                "UPDATE request SET status = 'done', completed_at = $2, final_kode_transaksi = $3, \
                        output_purged_at = $2 \
                  WHERE request_id = $1",
            )
            .bind(rid)
            .bind(t0())
            .bind(kt)
            .execute(&mut *tx)
            .await
            .unwrap();
            sqlx::query(
                "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor, terjadi_at) \
                 VALUES ($1, 2, 'in_flight', 'done', 'worker', now())",
            )
            .bind(rid)
            .execute(&mut *tx)
            .await
            .unwrap();
            tx.commit().await.unwrap();
            if berpayload {
                hasil.0 = kt;
                hasil.1 = rid;
            }
        }
        hasil
    }

    /// `audit_akses` is INSERT-only for `pr_admin_api` (E-06: reading it is itself
    /// audited, so there is deliberately no SELECT grant). The trail is therefore
    /// verified as superuser; the rows are the same ones the handlers wrote.
    ///
    /// Scoped to `pengguna_id`: every test seeds its own user and they share one
    /// database, so an unscoped count would pick up a sibling test's rows.
    async fn jumlah_audit(&self, pengguna: Uuid, aksi: &str, hasil: &str) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM audit_akses WHERE pengguna_id = $1 AND aksi = $2 AND hasil = $3",
        )
        .bind(pengguna)
        .bind(aksi)
        .bind(hasil)
        .fetch_one(&self.su)
        .await
        .unwrap()
    }

    async fn checksum_ekspor_terakhir(&self) -> String {
        sqlx::query_scalar(
            "SELECT ekspor_checksum FROM audit_akses WHERE aksi = 'ekspor' AND hasil = 'diizinkan' \
              ORDER BY terjadi_at DESC LIMIT 1",
        )
        .fetch_one(&self.su)
        .await
        .unwrap()
    }
}

async fn siapkan() -> Uji {
    let url = std::env::var("POOLROUTER_TEST_DATABASE_URL").expect(
        "POOLROUTER_TEST_DATABASE_URL belum diset; jalankan lewat `make -C backend uji-pg`",
    );
    let su_url = std::env::var("POOLROUTER_TEST_DATABASE_URL_SU").expect(
        "POOLROUTER_TEST_DATABASE_URL_SU belum diset; jalankan lewat `make -C backend uji-pg`",
    );
    let pool = PgPool::connect(&url).await.expect("database");
    let su = PgPool::connect(&su_url).await.expect("database su");
    let state = AdminState::baru(
        pool.clone(),
        AsalUi::baru(ASAL).unwrap(),
        PembatasLogin::baru(redis::Client::open(redis_ujian()).unwrap(), KUNCI_HMAC),
        Brankas::dari_isi_kek(KEK_HEX).unwrap(),
    )
    .dengan_jam(Arc::new(t0));
    let app = router(state).layer(MockConnectInfo(SocketAddr::from(([203, 0, 113, 9], 4000))));
    Uji { app, pool, su }
}

fn no_store(h: &HeaderMap) {
    assert_eq!(h[header::CACHE_CONTROL], "no-store");
}

// ---------------------------------------------------------------------------
// AC-32: searchLog
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn cari_log_menyaring_nik_dan_mencatat_satu_cari_log() {
    let u = siapkan().await;
    let (uid, op) = u.pengguna(&["operator"]).await;
    let (_, _, nik) = u.siapkan_log().await;

    let sebelum = u.jumlah_audit(uid, "cari_log", "diizinkan").await;
    let (st, h, j) = u
        .json(
            Method::POST,
            "/admin/v1/log/cari",
            Some(&op),
            Some(json!({
                "dari": t0() - chrono::Duration::hours(1),
                "sampai": t0() + chrono::Duration::hours(1),
            })),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    no_store(&h);
    let baris = j["baris"].as_array().expect("baris");
    assert!(!baris.is_empty(), "baris log sintetis terlihat");
    // AC-32: OP sees the masked form, never the NIK it was stored from.
    assert!(
        baris
            .iter()
            .all(|b| b["nik"].as_str().unwrap_or("").contains("****")),
        "NIK tersamar untuk OP"
    );
    assert!(
        !j.to_string().contains(&nik),
        "NIK utuh tidak pernah keluar ke OP"
    );
    assert_eq!(
        u.jumlah_audit(uid, "cari_log", "diizinkan").await,
        sebelum + 1,
        "tepat satu baris cari_log"
    );
}

#[tokio::test]
#[ignore]
async fn pencarian_log_menolak_kode_transaksi_bukan_uuid() {
    let u = siapkan().await;
    let (_uid, op) = u.pengguna(&["operator"]).await;
    let (st, _, j) = u
        .json(
            Method::POST,
            "/admin/v1/log/cari",
            Some(&op),
            Some(json!({
                "dari": t0() - chrono::Duration::hours(1),
                "sampai": t0() + chrono::Duration::hours(1),
                "kode_transaksi": "bukan-uuid",
            })),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(j["kode"], "validasi_gagal");
    assert_eq!(j["errors"][0]["field"], "kode_transaksi");
    assert_eq!(j["errors"][0]["kode"], "format");
}

#[tokio::test]
#[ignore]
async fn nik_dan_payload_sintetis_tidak_muncul_di_log() {
    use tracing_subscriber::fmt::MakeWriter;

    #[derive(Clone, Default)]
    struct Penyangga(Arc<Mutex<Vec<u8>>>);
    struct Penulis(Arc<Mutex<Vec<u8>>>);
    impl std::io::Write for Penulis {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    impl<'a> MakeWriter<'a> for Penyangga {
        type Writer = Penulis;
        fn make_writer(&'a self) -> Self::Writer {
            Penulis(self.0.clone())
        }
    }

    let penyangga = Penyangga::default();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .with_writer(penyangga.clone())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    let u = siapkan().await;
    let (_, au) = u.pengguna(&["auditor"]).await;
    let (kode, req_id, nik) = u.siapkan_log().await;
    let payload = "payload-sintetis-rahasia";
    sqlx::query("UPDATE transaction_log SET request_payload = $1 WHERE request_id = $2")
        .bind(serde_json::json!({"isi": payload}))
        .bind(req_id)
        .execute(&u.su)
        .await
        .unwrap();
    let (st, _, response) = u
        .json(
            Method::POST,
            "/admin/v1/auditor/payload",
            Some(&au),
            Some(json!({"kode_transaksi": kode.to_string(), "alasan": "uji payload sintetis"})),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    assert!(
        response.to_string().contains(payload),
        "uji memastikan payload fixture nyata"
    );
    let log = String::from_utf8(penyangga.0.lock().unwrap().clone()).unwrap();
    assert!(!log.is_empty(), "penangkap tracing dipasang");
    assert!(!log.contains(&nik), "NIK tidak muncul di tracing");
    assert!(
        !log.contains(payload),
        "payload sintetis tidak muncul di tracing"
    );
}

#[tokio::test]
#[ignore]
async fn cari_log_dengan_potongan_nik_403_dan_akses_ditolak() {
    let u = siapkan().await;
    let (uid, op) = u.pengguna(&["operator"]).await;

    let sebelum = u.jumlah_audit(uid, "akses_ditolak", "ditolak").await;
    // Six consecutive digits anywhere in any free-text parameter.
    for badan in [
        json!({"dari": t0() - chrono::Duration::hours(1), "sampai": t0(), "kode_transaksi": "3201234567890001"}),
        json!({"dari": t0() - chrono::Duration::hours(1), "sampai": t0(), "status_akses": "klien 3201234567890001"}),
        json!({"dari": t0() - chrono::Duration::hours(1), "sampai": t0(), "kursor": "000123"}),
    ] {
        let (st, h, j) = u
            .json(Method::POST, "/admin/v1/log/cari", Some(&op), Some(badan))
            .await;
        assert_eq!(st, StatusCode::FORBIDDEN);
        no_store(&h);
        assert_eq!(j["kode"], "pencarian_nik_dilarang");
    }
    assert_eq!(
        u.jumlah_audit(uid, "akses_ditolak", "ditolak").await,
        sebelum + 3,
        "satu akses_ditolak per pencarian NIK"
    );
}

#[tokio::test]
#[ignore]
async fn origin_asing_ditolak_pada_pencarian_log() {
    let u = siapkan().await;
    let (_uid, op) = u.pengguna(&["operator"]).await;
    let (st, _, j) = u
        .json(
            Method::POST,
            "/admin/v1/log/cari",
            Some(&op),
            Some(json!({
                "dari": t0() - chrono::Duration::hours(1),
                "sampai": t0() + chrono::Duration::hours(1),
            })),
        )
        .await;
    assert_eq!(st, StatusCode::FORBIDDEN);
    assert_eq!(j["kode"], "origin_tidak_valid");
}

#[tokio::test]
#[ignore]
async fn cari_log_tanpa_rentang_422() {
    let u = siapkan().await;
    let (_uid, op) = u.pengguna(&["operator"]).await;
    let (st, _, j) = u
        .json(
            Method::POST,
            "/admin/v1/log/cari",
            Some(&op),
            Some(json!({"dari": t0(), "sampai": t0()})),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(j["kode"], "validasi_gagal");
}

// ---------------------------------------------------------------------------
// AC-33: getLogEntry
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn get_log_tersamar_dan_404() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_test_writer()
        .try_init();
    let u = siapkan().await;
    let (uid, op) = u.pengguna(&["operator"]).await;
    let (kode, _, nik) = u.siapkan_log().await;

    let sebelum = u.jumlah_audit(uid, "lihat_log", "diizinkan").await;
    let (st, h, j) = u
        .json(
            Method::GET,
            &format!("/admin/v1/log/{kode}"),
            Some(&op),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    no_store(&h);
    assert!(j["nik"].as_str().unwrap().contains("****"));
    assert!(!j.to_string().contains(&nik));
    assert_eq!(
        u.jumlah_audit(uid, "lihat_log", "diizinkan").await,
        sebelum + 1
    );

    let (st, _, _) = u
        .json(
            Method::GET,
            &format!("/admin/v1/log/{}", Uuid::now_v7()),
            Some(&op),
            None,
        )
        .await;
    assert_eq!(st, StatusCode::NOT_FOUND);
}

// ---------------------------------------------------------------------------
// AC-34: searchLogByNik
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn cari_nik_ok_dan_audit_per_panggilan() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_test_writer()
        .try_init();
    let u = siapkan().await;
    let (uid, au) = u.pengguna(&["auditor"]).await;
    let (_, _, nik) = u.siapkan_log().await;

    let sebelum = u.jumlah_audit(uid, "cari_nik", "diizinkan").await;
    for _ in 0..2 {
        let (st, h, j) = u
            .json(
                Method::POST,
                "/admin/v1/auditor/log/cari-nik",
                Some(&au),
                Some(json!({
                    "nik": nik,
                    "dari": t0() - chrono::Duration::hours(1),
                    "sampai": t0() + chrono::Duration::hours(1),
                    "alasan": "uji audit sintetis",
                })),
            )
            .await;
        assert_eq!(st, StatusCode::OK);
        no_store(&h);
        assert_eq!(
            j["baris"].as_array().unwrap().len(),
            2,
            "kedua baris log untuk NIK ini"
        );
        let baris = j["baris"].as_array().unwrap();
        assert_eq!(baris[0]["nik"], nik);
        assert!(!baris[0]["client_nama"].as_str().unwrap_or("").is_empty());
    }
    assert_eq!(
        u.jumlah_audit(uid, "cari_nik", "diizinkan").await,
        sebelum + 2,
        "satu baris cari_nik per panggilan, termasuk pengulangan"
    );
}

#[tokio::test]
#[ignore]
async fn cari_nik_bukan_16_digit_atau_alasan_pendek_422() {
    let u = siapkan().await;
    let (_uid, au) = u.pengguna(&["auditor"]).await;
    let nik = nik_sintetis();

    let (st, _, j) = u
        .json(
            Method::POST,
            "/admin/v1/auditor/log/cari-nik",
            Some(&au),
            Some(json!({"nik": "31750000", "dari": t0() - chrono::Duration::hours(1), "sampai": t0(), "alasan": "uji audit sintetis"})),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(j["errors"][0]["field"], "nik");

    let (st, _, j) = u
        .json(
            Method::POST,
            "/admin/v1/auditor/log/cari-nik",
            Some(&au),
            Some(json!({"nik": nik, "dari": t0() - chrono::Duration::hours(1), "sampai": t0(), "alasan": "pendek"})),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(j["errors"][0]["field"], "alasan");
}

// ---------------------------------------------------------------------------
// AC-35: openPayload
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn buka_payload_satu_baris_per_pembukaan() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_test_writer()
        .try_init();
    let u = siapkan().await;
    let (uid, au) = u.pengguna(&["auditor"]).await;
    let (kode, req_id, _nik) = u.siapkan_log().await;

    let sebelum = u.jumlah_audit(uid, "lihat_payload", "diizinkan").await;
    for badan in [
        json!({"kode_transaksi": kode.to_string(), "alasan": "uji buka payload"}),
        json!({"request_id": req_id, "alasan": "uji buka payload"}),
    ] {
        let (st, _, _) = u
            .json(
                Method::POST,
                "/admin/v1/auditor/payload",
                Some(&au),
                Some(badan),
            )
            .await;
        assert_eq!(st, StatusCode::OK);
    }
    assert_eq!(
        u.jumlah_audit(uid, "lihat_payload", "diizinkan").await,
        sebelum + 2
    );
}

#[tokio::test]
#[ignore]
async fn buka_payload_dua_atau_nol_penanda_422() {
    let u = siapkan().await;
    let (_uid, au) = u.pengguna(&["auditor"]).await;
    let (kode, req_id, _nik) = u.siapkan_log().await;

    for badan in [
        json!({"alasan": "uji buka payload"}),
        json!({"kode_transaksi": kode.to_string(), "request_id": req_id, "alasan": "uji buka payload"}),
    ] {
        let (st, _, j) = u
            .json(
                Method::POST,
                "/admin/v1/auditor/payload",
                Some(&au),
                Some(badan),
            )
            .await;
        assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(j["kode"], "validasi_gagal");
    }
}

// ---------------------------------------------------------------------------
// AC-36: exportLogAuditor
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn ekspor_checksum_cocok_dan_header_aman() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_test_writer()
        .try_init();
    let u = siapkan().await;
    let (uid, au) = u.pengguna(&["auditor"]).await;
    let (_, _, nik) = u.siapkan_log().await;

    let sebelum = u.jumlah_audit(uid, "ekspor", "diizinkan").await;
    let (st, h, badan) = u
        .kirim(
            Method::POST,
            "/admin/v1/auditor/log/ekspor",
            Some(&au),
            Some(ASAL),
            Some(json!({
                "nik": nik,
                "dari": t0() - chrono::Duration::hours(1),
                "sampai": t0() + chrono::Duration::hours(1),
                "alasan": "uji ekspor sintetis",
            })),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    no_store(&h);
    assert_eq!(h[header::ACCEPT_RANGES], "none");
    assert_eq!(h[header::CONTENT_TYPE], "text/csv; charset=utf-8");
    assert!(
        !h[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .contains(&nik),
        "nama berkas tanpa NIK"
    );
    let digest = h["repr-digest"].to_str().unwrap().to_owned();
    let tercatat = u.checksum_ekspor_terakhir().await;
    assert_eq!(digest, format!("sha-256=:{tercatat}:"));
    // The header must describe the bytes the client actually received.
    let sebenarnya = aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, &badan);
    let hex: String = sebenarnya
        .as_ref()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    assert_eq!(
        hex, tercatat,
        "checksum audit = SHA-256 berkas yang diterima"
    );
    assert_eq!(
        u.jumlah_audit(uid, "ekspor", "diizinkan").await,
        sebelum + 1
    );
}

// ---------------------------------------------------------------------------
// AC-37: searchAuditAkses
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn cari_audit_menolak_rentang_panjang_dan_tanpa_nik_hmac() {
    let _ = tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .with_test_writer()
        .try_init();
    let u = siapkan().await;
    let (uid, au) = u.pengguna(&["auditor"]).await;

    let (st, _, j) = u
        .json(
            Method::POST,
            "/admin/v1/auditor/audit-akses/cari",
            Some(&au),
            Some(json!({"dari": t0() - chrono::Duration::days(40), "sampai": t0()})),
        )
        .await;
    assert_eq!(st, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(j["errors"][0]["field"], "sampai");

    let sebelum = u.jumlah_audit(uid, "lihat_audit", "diizinkan").await;
    let (st, h, j) = u
        .json(
            Method::POST,
            "/admin/v1/auditor/audit-akses/cari",
            Some(&au),
            Some(json!({"dari": t0() - chrono::Duration::days(1), "sampai": t0() + chrono::Duration::days(1)})),
        )
        .await;
    assert_eq!(st, StatusCode::OK);
    no_store(&h);
    assert!(
        !j.to_string().contains("nik_hmac"),
        "nik_hmac tidak pernah keluar"
    );
    assert_eq!(
        u.jumlah_audit(uid, "lihat_audit", "diizinkan").await,
        sebelum + 1
    );
}

// ---------------------------------------------------------------------------
// AC-38: role separation
// ---------------------------------------------------------------------------

#[tokio::test]
#[ignore]
async fn peran_bukan_auditor_403_di_keempat_operasi() {
    let u = siapkan().await;
    let (uid, pa) = u.pengguna(&["platform_admin"]).await;
    let (kode, _, nik) = u.siapkan_log().await;

    let kasus = [
        (
            "/admin/v1/auditor/log/cari-nik",
            json!({"nik": nik, "dari": t0() - chrono::Duration::hours(1), "sampai": t0(), "alasan": "uji audit sintetis"}),
        ),
        (
            "/admin/v1/auditor/payload",
            json!({"kode_transaksi": kode.to_string(), "alasan": "uji buka payload"}),
        ),
        (
            "/admin/v1/auditor/log/ekspor",
            json!({"nik": nik, "dari": t0() - chrono::Duration::hours(1), "sampai": t0(), "alasan": "uji ekspor sintetis"}),
        ),
        (
            "/admin/v1/auditor/audit-akses/cari",
            json!({"dari": t0() - chrono::Duration::hours(1), "sampai": t0()}),
        ),
    ];
    let sebelum = u.jumlah_audit(uid, "akses_ditolak", "ditolak").await;
    for (jalur, badan) in kasus {
        let (st, _, j) = u.json(Method::POST, jalur, Some(&pa), Some(badan)).await;
        assert_eq!(st, StatusCode::FORBIDDEN, "{jalur}");
        assert_eq!(j["kode"], "akses_ditolak", "{jalur}");
    }
    assert_eq!(
        u.jumlah_audit(uid, "akses_ditolak", "ditolak").await,
        sebelum + 4,
        "satu akses_ditolak per operasi auditor"
    );
}

#[tokio::test]
#[ignore]
async fn tanpa_sesi_401_di_enam_operasi() {
    let u = siapkan().await;
    let (kode, _, _nik) = u.siapkan_log().await;
    for (method, jalur) in [
        (Method::POST, "/admin/v1/log/cari".to_owned()),
        (Method::GET, format!("/admin/v1/log/{kode}")),
        (Method::POST, "/admin/v1/auditor/log/cari-nik".to_owned()),
        (Method::POST, "/admin/v1/auditor/payload".to_owned()),
        (Method::POST, "/admin/v1/auditor/log/ekspor".to_owned()),
        (
            Method::POST,
            "/admin/v1/auditor/audit-akses/cari".to_owned(),
        ),
    ] {
        let badan = if method == Method::GET {
            None
        } else {
            Some(json!({}))
        };
        let (st, _, _) = u
            .kirim(method.clone(), &jalur, None, Some(ASAL), badan)
            .await;
        assert_eq!(st, StatusCode::UNAUTHORIZED, "{method} {jalur}");
    }
}
