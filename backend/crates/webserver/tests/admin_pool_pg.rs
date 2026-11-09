//! Upstream pool, downstream pool, dan quota tracker lewat HTTP terhadap
//! PostgreSQL asli (ARC-53; AC-1..AC-5, AC-7, AC-16..AC-18). Dijalankan oleh
//! `make -C backend uji-pg`; `#[ignore]` karena butuh
//! `POOLROUTER_TEST_DATABASE_URL`.
//!
//! Router yang diuji adalah router admin produksi (`admin::sesi::router`) dengan
//! cookie sesi dan `Origin` asli, terhubung sebagai `pr_admin_api`. Fixture dan
//! pemeriksaan tabel memakai URL superuser. Semua nilai sintetis.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::Router;
use axum::body::{Body, to_bytes};
use axum::extract::connect_info::MockConnectInfo;
use axum::http::{HeaderMap, Method, Request, StatusCode, header};
use chrono::{DateTime, TimeZone, Utc};
use poolrouter_core::admin::sesi_store::SesiStore;
use poolrouter_core::admin::{AsalUi, TokenSesi};
use poolrouter_core::kuota::sumber_pg::ParameterDispatch;
use poolrouter_core::secret::Brankas;
use poolrouter_webserver::admin::pembatas::PembatasLogin;
use poolrouter_webserver::admin::sesi::{AdminState, COOKIE_SESI, router};
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use uuid::Uuid;

const ASAL: &str = "https://ui.example.test";
const KEK_HEX: &str = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
const KUNCI_HMAC: &[u8] = b"kunci-hmac-sintetis-untuk-uji";
const UPSTREAM: &str = "/admin/v1/upstream-pools";
const DOWNSTREAM: &str = "/admin/v1/downstream-pools";
const ALASAN: &str = "Penyesuaian kuota uji sintetis";

fn env(nama: &str) -> String {
    std::env::var(nama)
        .unwrap_or_else(|_| panic!("{nama} belum diset; jalankan lewat `make -C backend uji-pg`"))
}

/// 2026-11-05 09:00 UTC = 16:00 WIB.
fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 11, 5, 9, 0, 0).unwrap()
}

struct Uji {
    app: Router,
    pool: PgPool,
    su: PgPool,
}

async fn siapkan(mfa_wajib: bool) -> Uji {
    let pool = PgPool::connect(&env("POOLROUTER_TEST_DATABASE_URL"))
        .await
        .expect("database");
    let su = PgPool::connect(&env("POOLROUTER_TEST_DATABASE_URL_SU"))
        .await
        .expect("database su");
    let redis = env("POOLROUTER_TEST_REDIS");
    let state = AdminState::baru(
        pool.clone(),
        AsalUi::baru(ASAL).unwrap(),
        PembatasLogin::baru(redis::Client::open(redis.clone()).unwrap(), KUNCI_HMAC),
        Brankas::dari_isi_kek(KEK_HEX).unwrap(),
        redis::Client::open(redis).unwrap(),
        mfa_wajib,
    )
    .dengan_jam(Arc::new(t0));
    let app = router(state).layer(MockConnectInfo(SocketAddr::from(([203, 0, 113, 9], 4000))));
    Uji { app, pool, su }
}

fn nama_unik(dasar: &str) -> String {
    format!("{dasar}-{}", Uuid::now_v7().simple())
}

impl Uji {
    async fn kirim_dari(
        &self,
        method: Method,
        path: &str,
        cookie: Option<&str>,
        asal: &str,
        body: Option<Value>,
    ) -> (StatusCode, HeaderMap, Value) {
        let mut req = Request::builder().method(method.clone()).uri(path);
        if let Some(c) = cookie {
            req = req.header(header::COOKIE, c);
        }
        if method != Method::GET {
            req = req.header(header::ORIGIN, asal);
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
        let bytes = to_bytes(res.into_body(), 1 << 20).await.unwrap();
        (
            status,
            headers,
            serde_json::from_slice(&bytes).unwrap_or(Value::Null),
        )
    }

    async fn kirim(
        &self,
        method: Method,
        path: &str,
        cookie: &str,
        body: Option<Value>,
    ) -> (StatusCode, Value) {
        let (s, h, b) = self
            .kirim_dari(method, path, Some(cookie), ASAL, body)
            .await;
        assert_eq!(
            h.get(header::CACHE_CONTROL).map(|v| v.to_str().unwrap()),
            Some("no-store"),
            "AC-2: setiap respons no-store"
        );
        (s, b)
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

    async fn pengguna(&self, peran: &str) -> (Uuid, String) {
        let uid = Uuid::now_v7();
        self.user(uid).await;
        let (pengaju, penyetuju) = if matches!(peran, "auditor" | "operator") {
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
        .bind(peran)
        .bind(pengaju)
        .bind(penyetuju)
        .execute(&self.pool)
        .await
        .unwrap();
        let token = TokenSesi::baru();
        SesiStore::baru(self.pool.clone())
            .terbitkan(uid, &token.hash(), Some("uji"), t0())
            .await
            .unwrap();
        (uid, format!("{COOKIE_SESI}={}", token.sebagai_b64()))
    }

    /// Provider berversi tanpa NIK wajib (supaya fixture log tidak butuh NIK).
    async fn provider(&self, pembuat: Uuid) -> Uuid {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO provider (provider_id, nama, wajib_log_nik, config_versi_aktif) \
             VALUES ($1, $2, false, 1)",
        )
        .bind(id)
        .bind(nama_unik("prov"))
        .execute(&self.su)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO provider_config_versi (provider_id, versi, config_schema_version, \
             log_mode, dokumen, dibuat_oleh) VALUES ($1, 1, 1, 'metadata_only', $2, $3)",
        )
        .bind(id)
        // Dokumen lengkap: `listProviders` membaca setiap provider di database
        // bersama, jadi fixture tidak boleh memakai dokumen yang tidak terbaca.
        .bind(json!({
            "metode": "POST", "base_url": "https://prov.example.test",
            "body_template": "{}", "parameters": [], "respons_skema": {},
            "tls_trust": "system", "log_mode": "metadata_only"
        }))
        .bind(pembuat)
        .execute(&self.su)
        .await
        .unwrap();
        id
    }

    async fn audit_konfigurasi(&self, jenis: &str, id: Uuid) -> Vec<(String, Option<String>)> {
        sqlx::query_as(
            "SELECT aksi, alasan FROM audit_konfigurasi \
             WHERE entitas_jenis = $1 AND entitas_id = $2 ORDER BY terjadi_at",
        )
        .bind(jenis)
        .bind(id)
        .fetch_all(&self.su)
        .await
        .unwrap()
    }

    async fn jumlah_ditolak(&self, uid: Uuid) -> i64 {
        sqlx::query_scalar(
            "SELECT count(*) FROM audit_akses \
             WHERE pengguna_id = $1 AND aksi = 'akses_ditolak' AND hasil = 'ditolak'",
        )
        .bind(uid)
        .fetch_one(&self.su)
        .await
        .unwrap()
    }
}

fn body_upstream(provider: Uuid) -> Value {
    json!({
        "provider_id": provider,
        "kuota_harian": 1000,
        "rate_limit_per_jam": 3600,
        "porsi_sync": 0.2,
        "margin_kuota": 0.01,
        "batas_percobaan": 3,
        "retry_backoff_base_detik": 5,
        "retry_backoff_cap_detik": 300,
        "retry_rasio": 0.1,
        "retry_min_per_jam": 20,
        "jendela_kuota": "bergulir_24j",
        "reset_kuota_jam": 0,
        "reset_kuota_zona": "Asia/Jakarta",
        "jeda_min": 100,
        "jeda_maks": 500,
        "ambang_berdampingan": 4
    })
}

fn kode_error(b: &Value) -> Vec<(String, String)> {
    b["errors"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|e| {
                    (
                        e["field"].as_str().unwrap_or_default().to_owned(),
                        e["kode"].as_str().unwrap_or_default().to_owned(),
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

async fn buat_upstream(u: &Uji, cookie: &str, provider: Uuid) -> Uuid {
    let (s, b) = u
        .kirim(
            Method::POST,
            UPSTREAM,
            cookie,
            Some(body_upstream(provider)),
        )
        .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    b["upstream_pool_id"].as_str().unwrap().parse().unwrap()
}

#[tokio::test]
#[ignore = "butuh PostgreSQL; jalankan lewat `make -C backend uji-pg`"]
async fn upstream_crud_menyimpan_semua_field_dan_satu_audit_per_tulis() {
    let u = siapkan(false).await;
    let (pa, c) = u.pengguna("platform_admin").await;
    let provider = u.provider(pa).await;

    let (s, b) = u
        .kirim(Method::POST, UPSTREAM, &c, Some(body_upstream(provider)))
        .await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let id: Uuid = b["upstream_pool_id"].as_str().unwrap().parse().unwrap();
    for (k, v) in body_upstream(provider).as_object().unwrap() {
        assert_eq!(&b[k], v, "field {k} kembali persis");
    }
    assert_eq!(b["aktif"], true);

    // Kunci dispatcher (ARC-70) ikut terisi, jadi pool bisa langsung dipakai.
    let parameter: Value = sqlx::query_scalar(
        "SELECT parameter_dispatch FROM upstream_pool WHERE upstream_pool_id = $1",
    )
    .bind(id)
    .fetch_one(&u.su)
    .await
    .unwrap();
    let d = ParameterDispatch::dari_json(&parameter).unwrap();
    assert_eq!(d.maks_konkurensi, 4);
    assert!(
        d.jendela_rate
            .iter()
            .any(|j| j.jenis == "tph" && j.t_mikro == 1_000_000)
    );

    let (s, g) = u
        .kirim(Method::GET, &format!("{UPSTREAM}/{id}"), &c, None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(g, b, "GET sama dengan respons create");
    let (s, daftar) = u.kirim(Method::GET, UPSTREAM, &c, None).await;
    assert_eq!(s, StatusCode::OK);
    assert!(
        daftar
            .as_array()
            .unwrap()
            .iter()
            .any(|p| p["upstream_pool_id"] == b["upstream_pool_id"])
    );

    let mut ubah = body_upstream(provider);
    ubah["kuota_harian"] = json!(2000);
    ubah["alasan"] = json!(ALASAN);
    let (s, b2) = u
        .kirim(Method::PUT, &format!("{UPSTREAM}/{id}"), &c, Some(ubah))
        .await;
    assert_eq!(s, StatusCode::OK, "{b2}");
    assert_eq!(b2["kuota_harian"], 2000);

    let audit = u.audit_konfigurasi("upstream_pool", id).await;
    assert_eq!(audit.len(), 2, "AC-5: satu baris per tulis");
    assert_eq!(audit[0].0, "buat");
    assert_eq!(audit[1], ("ubah".to_owned(), Some(ALASAN.to_owned())));

    let (s, _) = u
        .kirim(
            Method::GET,
            &format!("{UPSTREAM}/{}", Uuid::now_v7()),
            &c,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
#[ignore = "butuh PostgreSQL; jalankan lewat `make -C backend uji-pg`"]
async fn upstream_422_mengumpulkan_semua_galat_tanpa_menulis() {
    let u = siapkan(false).await;
    let (pa, c) = u.pengguna("platform_admin").await;
    let provider = u.provider(pa).await;
    let id = buat_upstream(&u, &c, provider).await;

    // PUT penggantian penuh: field yang hilang 422, bukan nilai bawaan (AC-16).
    let mut kurang = body_upstream(provider);
    kurang.as_object_mut().unwrap().remove("jeda_min");
    kurang.as_object_mut().unwrap().remove("margin_kuota");
    kurang["alasan"] = json!(ALASAN);
    let (s, b) = u
        .kirim(Method::PUT, &format!("{UPSTREAM}/{id}"), &c, Some(kurang))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    let e = kode_error(&b);
    assert!(e.contains(&("jeda_min".into(), "wajib".into())), "{e:?}");
    assert!(
        e.contains(&("margin_kuota".into(), "wajib".into())),
        "{e:?}"
    );

    // Batas KP-01, batas silang, zona, kalender tanpa alasan, provider tak ada.
    let mut salah = body_upstream(Uuid::now_v7());
    salah["porsi_sync"] = json!(1.5);
    salah["jeda_maks"] = json!(50);
    salah["jendela_kuota"] = json!("kalender");
    salah["reset_kuota_zona"] = json!("Europe/Berlin");
    let (s, b) = u.kirim(Method::POST, UPSTREAM, &c, Some(salah)).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert_eq!(b["kode"], "validasi_gagal");
    let e = kode_error(&b);
    for harap in [
        ("porsi_sync", "rentang"),
        ("jeda_maks", "rentang"),
        ("reset_kuota_zona", "format"),
        ("alasan", "wajib"),
        ("provider_id", "tidak_ada"),
    ] {
        assert!(
            e.contains(&(harap.0.into(), harap.1.into())),
            "{harap:?} {e:?}"
        );
    }
    assert!(
        !b.to_string().contains("Europe/Berlin"),
        "AC-7: nilai input tidak dikembalikan"
    );

    // Field tak dikenal.
    let mut asing = body_upstream(provider);
    asing["mode"] = json!("sync");
    let (s, b) = u.kirim(Method::POST, UPSTREAM, &c, Some(asing)).await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(kode_error(&b).contains(&("mode".into(), "tidak_dikenal".into())));

    assert_eq!(
        u.audit_konfigurasi("upstream_pool", id).await.len(),
        1,
        "422 tidak menulis audit"
    );
}

#[tokio::test]
#[ignore = "butuh PostgreSQL; jalankan lewat `make -C backend uji-pg`"]
async fn reset_kuota_dijadwalkan_ke_akhir_periode_berjalan() {
    let u = siapkan(false).await;
    let (pa, c) = u.pengguna("platform_admin").await;
    let provider = u.provider(pa).await;
    let id = buat_upstream(&u, &c, provider).await;

    let mut ubah = body_upstream(provider);
    ubah["reset_kuota_jam"] = json!(6);
    ubah["alasan"] = json!(ALASAN);
    let (s, b) = u
        .kirim(
            Method::PUT,
            &format!("{UPSTREAM}/{id}"),
            &c,
            Some(ubah.clone()),
        )
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(
        b["reset_kuota_jam"], 6,
        "GET/PUT melaporkan aturan terjadwal"
    );
    // t0 = 16:00 WIB; periode reset 00:00 WIB berakhir 2026-11-06 00:00 WIB.
    let akhir = Utc.with_ymd_and_hms(2026, 11, 5, 17, 0, 0).unwrap();
    assert_eq!(
        kolom_reset(&u.su, id).await,
        (
            "00:00:00".to_owned(),
            Some("06:00:00".to_owned()),
            Some(akhir)
        ),
        "KP-02: aturan aktif tetap sampai akhir periode"
    );

    // PUT ulang body yang sama tidak menggeser jadwal.
    let (s, _) = u
        .kirim(Method::PUT, &format!("{UPSTREAM}/{id}"), &c, Some(ubah))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(kolom_reset(&u.su, id).await.2, Some(akhir));

    // Kembali ke aturan aktif membatalkan jadwal.
    let mut balik = body_upstream(provider);
    balik["alasan"] = json!(ALASAN);
    let (s, _) = u
        .kirim(Method::PUT, &format!("{UPSTREAM}/{id}"), &c, Some(balik))
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(
        kolom_reset(&u.su, id).await,
        ("00:00:00".to_owned(), None, None)
    );
}

#[tokio::test]
#[ignore = "butuh PostgreSQL; jalankan lewat `make -C backend uji-pg`"]
async fn sesi_origin_peran_dan_mfa_dijaga() {
    let u = siapkan(false).await;
    let (pa, c) = u.pengguna("platform_admin").await;
    let provider = u.provider(pa).await;

    let (s, h, b) = u.kirim_dari(Method::GET, UPSTREAM, None, ASAL, None).await;
    assert_eq!(s, StatusCode::UNAUTHORIZED);
    assert_eq!(b["kode"], "tidak_terautentikasi");
    assert_eq!(h.get(header::CACHE_CONTROL).unwrap(), "no-store");

    let (s, _, b) = u
        .kirim_dari(
            Method::POST,
            UPSTREAM,
            Some(&c),
            "https://asing.example.test",
            Some(body_upstream(provider)),
        )
        .await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert_eq!(b["kode"], "origin_tidak_valid");

    let (op, c_op) = u.pengguna("operator").await;
    let sebelum = u.jumlah_ditolak(op).await;
    for (method, path, body) in [
        (Method::POST, UPSTREAM, Some(body_upstream(provider))),
        (Method::GET, DOWNSTREAM, None),
    ] {
        let (s, b) = u.kirim(method, path, &c_op, body).await;
        assert_eq!(s, StatusCode::FORBIDDEN, "{path}");
        assert_eq!(b["kode"], "akses_ditolak");
    }
    assert_eq!(
        u.jumlah_ditolak(op).await,
        sebelum + 2,
        "AC-4: satu baris per penolakan"
    );

    // MFA wajib: platform admin tanpa verifikasi MFA ditolak.
    let u2 = siapkan(true).await;
    let (_, c2) = u2.pengguna("platform_admin").await;
    let (s, b) = u2.kirim(Method::GET, UPSTREAM, &c2, None).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{b}");
}

/// Aturan reset aktif, aturan terjadwal, dan saat berlakunya.
async fn kolom_reset(su: &PgPool, id: Uuid) -> (String, Option<String>, Option<DateTime<Utc>>) {
    sqlx::query_as(
        "SELECT reset_kuota_jam::text, reset_kuota_jam_berikut::text, reset_kuota_berlaku_mulai \
         FROM upstream_pool WHERE upstream_pool_id = $1",
    )
    .bind(id)
    .fetch_one(su)
    .await
    .unwrap()
}

fn body_downstream(nama: &str, upstream: Uuid) -> Value {
    json!({ "nama": nama, "upstream_pool_ids": [upstream] })
}

#[tokio::test]
#[ignore = "butuh PostgreSQL; jalankan lewat `make -C backend uji-pg`"]
async fn downstream_crud_rute_dan_alokasi_dalam_kuota() {
    let u = siapkan(false).await;
    let (pa, c) = u.pengguna("platform_admin").await;
    let provider = u.provider(pa).await;
    // kuota 1000, margin 0,01: kuota efektif 990.
    let upstream = buat_upstream(&u, &c, provider).await;

    let nama_a = nama_unik("dp-a");
    let mut a = body_downstream(&nama_a, upstream);
    a["alokasi_jenis"] = json!("exact");
    a["alokasi_nilai"] = json!(600);
    a["sifat"] = json!("reserved");
    let (s, b) = u.kirim(Method::POST, DOWNSTREAM, &c, Some(a)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");
    let id_a: Uuid = b["downstream_pool_id"].as_str().unwrap().parse().unwrap();
    assert_eq!(b["prioritas"], 1);
    assert_eq!(b["ttl_antrean_detik"], 86_400);
    assert_eq!(b["upstream_pool_ids"], json!([upstream]));
    assert_eq!(b["alokasi_nilai"], 600.0);
    assert_eq!(b["boleh_pinjam"], false);

    // 600 + 400 > 990: ditolak 422 dengan batas = kelebihan, tanpa baris.
    let nama_b = nama_unik("dp-b");
    let mut lebih = body_downstream(&nama_b, upstream);
    lebih["alokasi_jenis"] = json!("exact");
    lebih["alokasi_nilai"] = json!(400);
    lebih["sifat"] = json!("shareable");
    let (s, b) = u
        .kirim(Method::POST, DOWNSTREAM, &c, Some(lebih.clone()))
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert_eq!(b["errors"][0]["kode"], "alokasi_kelebihan");
    assert_eq!(b["errors"][0]["batas"], 10.0);
    let ada: i64 = sqlx::query_scalar("SELECT count(*) FROM downstream_pool WHERE nama = $1")
        .bind(&nama_b)
        .fetch_one(&u.su)
        .await
        .unwrap();
    assert_eq!(ada, 0, "422 menggulung balik seluruh transaksi");

    lebih["alokasi_nilai"] = json!(390);
    let (s, b) = u.kirim(Method::POST, DOWNSTREAM, &c, Some(lebih)).await;
    assert_eq!(s, StatusCode::CREATED, "{b}");

    // Nama ganda dan lebih dari satu upstream pool.
    let (s, b) = u
        .kirim(
            Method::POST,
            DOWNSTREAM,
            &c,
            Some(body_downstream(&nama_a, upstream)),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(
        kode_error(&b),
        vec![("nama".to_owned(), "sudah_dipakai".to_owned())]
    );
    let (s, b) = u
        .kirim(
            Method::POST,
            DOWNSTREAM,
            &c,
            Some(json!({ "nama": nama_unik("dp-c"), "upstream_pool_ids": [upstream, upstream] })),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(kode_error(&b).contains(&("upstream_pool_ids".into(), "jumlah".into())));
    let (s, b) = u
        .kirim(
            Method::POST,
            DOWNSTREAM,
            &c,
            Some(json!({ "nama": nama_unik("dp-d"), "upstream_pool_ids": [upstream], "prioritas": 2 })),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "prioritas hanya baca");
    assert!(kode_error(&b).contains(&("prioritas".into(), "tidak_dikenal".into())));

    // Update: alasan wajib, lalu ganti ke persen.
    let mut ubah = body_downstream(&nama_a, upstream);
    ubah["alokasi_jenis"] = json!("persen");
    ubah["alokasi_nilai"] = json!(10);
    ubah["sifat"] = json!("shareable");
    ubah["boleh_pinjam"] = json!(true);
    let (s, b) = u
        .kirim(
            Method::PUT,
            &format!("{DOWNSTREAM}/{id_a}"),
            &c,
            Some(ubah.clone()),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY);
    assert!(kode_error(&b).contains(&("alasan".into(), "wajib".into())));
    ubah["alasan"] = json!(ALASAN);
    let (s, b) = u
        .kirim(Method::PUT, &format!("{DOWNSTREAM}/{id_a}"), &c, Some(ubah))
        .await;
    assert_eq!(s, StatusCode::OK, "{b}");
    assert_eq!(b["alokasi_jenis"], "persen");
    assert_eq!(b["boleh_pinjam"], true);
    let (s, g) = u
        .kirim(Method::GET, &format!("{DOWNSTREAM}/{id_a}"), &c, None)
        .await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(g, b);

    // Kuota upstream yang turun di bawah alokasi (390 exact) ditolak.
    let mut turun = body_upstream(provider);
    turun["kuota_harian"] = json!(300);
    turun["alasan"] = json!(ALASAN);
    let (s, b) = u
        .kirim(
            Method::PUT,
            &format!("{UPSTREAM}/{upstream}"),
            &c,
            Some(turun),
        )
        .await;
    assert_eq!(s, StatusCode::UNPROCESSABLE_ENTITY, "{b}");
    assert_eq!(b["errors"][0]["kode"], "alokasi_kelebihan");

    let audit = u.audit_konfigurasi("downstream_pool", id_a).await;
    assert_eq!(
        audit.iter().map(|a| a.0.as_str()).collect::<Vec<_>>(),
        vec!["buat", "ubah"]
    );
}

/// Request dan baris log sintetis untuk pool `upstream` pada `requested_at`.
async fn baris_log(u: &Uji, pa: Uuid, provider: Uuid, upstream: Uuid, requested_at: DateTime<Utc>) {
    let client = Uuid::now_v7();
    let kredensial = Uuid::now_v7();
    let dpool = Uuid::now_v7();
    let endpoint = Uuid::now_v7();
    let rid = Uuid::now_v7();
    let su = &u.su;
    sqlx::query("INSERT INTO client (client_id, nama) VALUES ($1, $2)")
        .bind(client)
        .bind(nama_unik("klien"))
        .execute(su)
        .await
        .unwrap();
    sqlx::query(
        "INSERT INTO client_kredensial (credential_id, client_id, jenis, kunci_hash, dibuat_oleh) \
         VALUES ($1, $2, 'api_key', '\\x01', $3)",
    )
    .bind(kredensial)
    .bind(client)
    .bind(pa)
    .execute(su)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO downstream_pool (downstream_pool_id, nama, prioritas) VALUES ($1, $2, 1)",
    )
    .bind(dpool)
    .bind(nama_unik("dp-log"))
    .execute(su)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO downstream_endpoint (downstream_endpoint_id, nama, provider_id, \
         downstream_pool_id, config_versi_aktif) VALUES ($1, $2, $3, $4, 1)",
    )
    .bind(endpoint)
    .bind(nama_unik("ep-log"))
    .bind(provider)
    .bind(dpool)
    .execute(su)
    .await
    .unwrap();
    // Request dan riwayat status pertamanya satu transaksi: trigger T-04
    // ditunda sampai commit.
    let mut tx = su.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO request (request_id, client_id, credential_id, downstream_endpoint_id, \
         downstream_pool_id, provider_id, upstream_pool_id, provider_config_version, \
         endpoint_config_version, mode, prioritas, prioritas_efektif, status, metadata, \
         received_at, attempt_count, dispatch_key, final_kode_transaksi) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, 1, 1, 'sync', 1, 1, 'in_flight', '{}', $8, 1, NULL, NULL)",
    )
    .bind(rid)
    .bind(client)
    .bind(kredensial)
    .bind(endpoint)
    .bind(dpool)
    .bind(provider)
    .bind(upstream)
    .bind(requested_at)
    .execute(&mut *tx)
    .await
    .unwrap();
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
        "INSERT INTO transaction_log (kode_transaksi, request_id, attempt_no, provider_id, \
         upstream_pool_id, client_id, downstream_endpoint_id, provider_config_version, \
         endpoint_config_version, requested_at, log_mode, worker_id) \
         VALUES ($1, $2, 1, $3, $4, $5, $6, 1, 1, $7, 'metadata_only', 'worker-uji')",
    )
    .bind(Uuid::now_v7())
    .bind(rid)
    .bind(provider)
    .bind(upstream)
    .bind(client)
    .bind(endpoint)
    .bind(requested_at)
    .execute(su)
    .await
    .unwrap();
}

#[tokio::test]
#[ignore = "butuh PostgreSQL; jalankan lewat `make -C backend uji-pg`"]
async fn quota_tracker_membaca_database() {
    let u = siapkan(false).await;
    let (pa, c) = u.pengguna("platform_admin").await;
    let provider = u.provider(pa).await;
    let upstream = buat_upstream(&u, &c, provider).await;
    let path = format!("/admin/v1/kuota?upstream_pool_id={upstream}");

    let (s, b) = u.kirim(Method::GET, &path, &c, None).await;
    assert_eq!(s, StatusCode::OK, "{b}");
    let baris = &b[0];
    assert_eq!(b.as_array().unwrap().len(), 1);
    assert_eq!(baris["kuota_harian"], 1000);
    assert_eq!(baris["kuota_efektif"], 990);
    assert_eq!(baris["terpakai_periode"], 0);
    assert_eq!(baris["sisa_periode"], 990);
    assert_eq!(baris["rate_limit_per_jam"], 3600);
    assert_eq!(baris["porsi_sync_maks"], 720);
    assert_eq!(baris["periode_mulai"], "2026-11-04T17:00:00Z");
    assert_eq!(baris["periode_selesai"], "2026-11-05T17:00:00Z");

    // Satu baris log baru menaikkan terpakai tepat 1 (AC-18).
    baris_log(&u, pa, provider, upstream, t0()).await;
    let (_, b) = u.kirim(Method::GET, &path, &c, None).await;
    assert_eq!(b[0]["terpakai_periode"], 1);
    assert_eq!(b[0]["sisa_periode"], 989);
    assert_eq!(b[0]["terpakai_jam_berjalan"], 1);
    assert_eq!(b[0]["terpakai_sync_jam_berjalan"], 1);

    // Operator boleh membaca; id yang tidak ada 404.
    let (_, c_op) = u.pengguna("operator").await;
    let (s, _) = u.kirim(Method::GET, &path, &c_op, None).await;
    assert_eq!(s, StatusCode::OK);
    let (s, b) = u
        .kirim(
            Method::GET,
            &format!("/admin/v1/kuota?upstream_pool_id={}", Uuid::now_v7()),
            &c,
            None,
        )
        .await;
    assert_eq!(s, StatusCode::NOT_FOUND);
    assert_eq!(b["kode"], "tidak_ditemukan");
}
