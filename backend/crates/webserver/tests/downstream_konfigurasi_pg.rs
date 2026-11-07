//! Integration test for the endpoint configuration cache (ARC-61) against
//! PostgreSQL: the cached row is used before 30 s and re-read after, and it
//! carries the `aktif` status of the six entities (AC-6). Run via
//! `make -C backend uji-pg`: the store reads as `pr_api`, fixtures use
//! `pr_admin_api`, both real grants.

use std::sync::Arc;
use std::time::{Duration, Instant};

use poolrouter_webserver::downstream::konfigurasi_endpoint::{
    CachedKonfigurasi, KonfigurasiStore, PgKonfigurasiStore,
};
use sqlx::PgPool;
use uuid::Uuid;

const TTL: Duration = Duration::from_secs(30);

fn url_uji(nama: &str) -> String {
    std::env::var(nama)
        .unwrap_or_else(|_| panic!("{nama} belum diset; jalankan lewat `make -C backend uji-pg`"))
}

struct Uji {
    /// `pr_admin_api` pool for fixtures and deactivations.
    admin: PgPool,
    konfig: Arc<CachedKonfigurasi>,
    client_id: Uuid,
    endpoint_id: Uuid,
    kredensial: Uuid,
    provider_id: Uuid,
    upstream_id: Uuid,
    pool_id: Uuid,
    admin_user: Uuid,
}

impl Uji {
    async fn baru() -> Self {
        let admin = PgPool::connect(&url_uji("POOLROUTER_TEST_DATABASE_URL"))
            .await
            .unwrap();
        let app_pg = PgPool::connect(&url_uji("POOLROUTER_TEST_DATABASE_URL_API"))
            .await
            .unwrap();
        let uid = Uuid::now_v7().simple().to_string();
        let client_id = Uuid::now_v7();
        let endpoint_id = Uuid::now_v7();
        let kredensial = Uuid::now_v7();
        let provider_id = Uuid::now_v7();
        let upstream_id = Uuid::now_v7();
        let pool_id = Uuid::now_v7();
        let admin_user = Uuid::now_v7();

        // Fixtures go through pr_admin_api: the store itself only reads as pr_api.
        sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
            "INSERT INTO admin_user (pengguna_id, username, password_hash) \
               VALUES ('{admin_user}', 'arc61-{uid}', 'x');
             INSERT INTO admin_user_peran (pengguna_id, peran, diajukan_oleh) \
               VALUES ('{admin_user}', 'platform_admin', '{admin_user}');
             INSERT INTO client (client_id, nama) VALUES ('{client_id}', 'klien-{uid}');
             INSERT INTO provider (provider_id, nama, data_balikan_aktif) \
               VALUES ('{provider_id}', 'provider-{uid}', true);
             INSERT INTO upstream_pool (upstream_pool_id, provider_id, nama, kuota_harian) \
               VALUES ('{upstream_id}', '{provider_id}', 'upstream-{uid}', 1000);
             INSERT INTO downstream_pool (downstream_pool_id, nama, prioritas) \
               VALUES ('{pool_id}', 'pool-{uid}', 1);
             INSERT INTO downstream_pool_rute (downstream_pool_id, upstream_pool_id) \
               VALUES ('{pool_id}', '{upstream_id}');
             INSERT INTO downstream_endpoint \
               (downstream_endpoint_id, nama, provider_id, downstream_pool_id, config_versi_aktif) \
               VALUES ('{endpoint_id}', 'endpoint-{uid}', '{provider_id}', '{pool_id}', 1);
             INSERT INTO endpoint_config_versi \
               (downstream_endpoint_id, versi, config_schema_version, dokumen, dibuat_oleh) \
               VALUES ('{endpoint_id}', 1, 1, '{{}}', '{admin_user}');
             INSERT INTO endpoint_metadata_field \
               (downstream_endpoint_id, nama_field, tipe, wajib, nilai_diizinkan, klasifikasi, \
                kirim_ke_data_balikan, deskripsi, versi) \
               VALUES ('{endpoint_id}', 'status_penerima_manfaat', 'string', true, \
                       '[\"sesuai\",\"tidak_sesuai\"]'::jsonb, 'Internal', false, 'Sintetis.', 1);
             INSERT INTO client_kredensial (credential_id, client_id, jenis, kunci_hash, dibuat_oleh) \
               VALUES ('{kredensial}', '{client_id}', 'api_key', '\\x00'::bytea, '{admin_user}');
             INSERT INTO client_endpoint (client_id, downstream_endpoint_id) \
               VALUES ('{client_id}', '{endpoint_id}');"
        )))
        .execute(&admin)
        .await
        .unwrap();

        let konfig = Arc::new(CachedKonfigurasi::new(Arc::new(PgKonfigurasiStore::new(
            app_pg,
        ))));
        Self {
            admin,
            konfig,
            client_id,
            endpoint_id,
            kredensial,
            provider_id,
            upstream_id,
            pool_id,
            admin_user,
        }
    }

    /// Deactivation goes through the only sanctioned path (`EN-03`).
    async fn nonaktifkan(&self, jenis: &str, id: Uuid) {
        sqlx::query("SELECT fn_nonaktifkan_entitas($1, $2, $3, 'uji ARC-61')")
            .bind(jenis)
            .bind(id)
            .bind(self.admin_user)
            .execute(&self.admin)
            .await
            .unwrap();
    }
}

/// AC-6: the cached row is used before 30 s and re-read from PostgreSQL after.
#[tokio::test]
async fn cache_dipakai_sebelum_30_detik_dan_dibaca_ulang_sesudahnya() {
    let u = Uji::baru().await;
    let t0 = Instant::now();
    let awal = u.konfig.muat(u.client_id, u.endpoint_id, t0).await.unwrap();
    assert!(awal.status.is_some(), "endpoint ada");
    assert!(awal.kontrak.is_some(), "kontrak terbaca");

    // Deactivate the endpoint: PostgreSQL already says `aktif = false`, but the
    // cached row must still answer within the window.
    u.nonaktifkan("downstream_endpoint", u.endpoint_id).await;
    let dalam_jendela = u.konfig.muat(u.client_id, u.endpoint_id, t0).await.unwrap();
    assert!(
        dalam_jendela.status.unwrap().endpoint,
        "di dalam 30 s entri cache yang dipakai"
    );

    // One tick past the window: re-read from PostgreSQL, new state visible.
    let sesudah = u
        .konfig
        .muat(u.client_id, u.endpoint_id, t0 + TTL)
        .await
        .unwrap();
    assert!(
        !sesudah.status.unwrap().endpoint,
        "sesudah 30 s status dibaca ulang dari PostgreSQL"
    );
}

/// AC-6: each of the six entities flips its own flag.
#[tokio::test]
async fn status_enam_entitas_mengikuti_aktif_dan_kredensial_aktif() {
    for jenis in [
        "provider",
        "upstream_pool",
        "downstream_pool",
        "downstream_endpoint",
        "kredensial",
        "client",
    ] {
        let u = Uji::baru().await;
        let t0 = Instant::now();
        u.konfig.muat(u.client_id, u.endpoint_id, t0).await.unwrap();
        let id = match jenis {
            "provider" => u.provider_id,
            "upstream_pool" => u.upstream_id,
            "downstream_pool" => u.pool_id,
            "downstream_endpoint" => u.endpoint_id,
            "kredensial" => u.kredensial,
            "client" => u.client_id,
            _ => unreachable!(),
        };
        u.nonaktifkan(jenis, id).await;
        let e = u
            .konfig
            .muat(u.client_id, u.endpoint_id, t0 + TTL)
            .await
            .unwrap();
        let s = e.status.expect("endpoint ada");
        assert_eq!(s.client, jenis != "client");
        assert_eq!(s.kredensial, jenis != "kredensial");
        assert_eq!(s.endpoint, jenis != "downstream_endpoint");
        assert_eq!(s.downstream_pool, jenis != "downstream_pool");
        assert_eq!(s.upstream_pool, jenis != "upstream_pool");
        assert_eq!(s.provider, jenis != "provider");
        assert!(e.kontrak.is_some(), "GW3: kontrak tetap ada ({jenis})");
    }
}
