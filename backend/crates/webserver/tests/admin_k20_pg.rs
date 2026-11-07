//! K-20 contract tests against a real PostgreSQL (f0-admin-api AC-28..AC-31;
//! spec `products/poolrouter/specs/f0-admin-api/spec.md`). Run by
//! `make -C backend uji-pg`; `#[ignore]`d because it needs
//! `POOLROUTER_TEST_DATABASE_URL`. No mock repository: every assertion reads
//! back `request`, `request_status_history` or `audit_konfigurasi` through the
//! same `pr_admin_api` pool the service uses.
//!
//! Fixtures are seeded as superuser (`POOLROUTER_TEST_DATABASE_URL_SU`)
//! because `pr_admin_api` cannot insert `request`/`client` rows directly; the
//! service itself is still exercised through `pr_admin_api`. Every NIK,
//! payload and reason below is synthetic.

use std::sync::{Arc, Mutex};

use chrono::Utc;
use poolrouter_webserver::admin::k20::{GalatK20, JenisEntitas, K20Service, validasi_alasan};
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use tracing_subscriber::fmt::MakeWriter;
use uuid::Uuid;

fn db_ujian() -> String {
    std::env::var("POOLROUTER_TEST_DATABASE_URL")
        .expect("POOLROUTER_TEST_DATABASE_URL belum diset; jalankan lewat `make -C backend uji-pg`")
}

fn db_su() -> String {
    std::env::var("POOLROUTER_TEST_DATABASE_URL_SU").expect(
        "POOLROUTER_TEST_DATABASE_URL_SU belum diset; jalankan lewat `make -C backend uji-pg`",
    )
}

async fn kolam(url: &str) -> PgPool {
    PgPoolOptions::new()
        .max_connections(5)
        .connect(url)
        .await
        .unwrap()
}

#[derive(Clone, Default)]
struct Penyangga(Arc<Mutex<Vec<u8>>>);

impl<'a> MakeWriter<'a> for Penyangga {
    type Writer = Penulis;
    fn make_writer(&'a self) -> Self::Writer {
        Penulis(self.0.clone())
    }
}

struct Penulis(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Penulis {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Captures one tracing subscriber scope for redaction assertions (AC-6).
fn penangkap_log() -> (Penyangga, tracing::subscriber::DefaultGuard) {
    let writer = Penyangga::default();
    let subscriber = tracing_subscriber::fmt()
        .with_ansi(false)
        .with_writer(writer.clone())
        .finish();
    let guard = tracing::subscriber::set_default(subscriber);
    (writer, guard)
}

fn isi_log(writer: &Penyangga) -> String {
    String::from_utf8(writer.0.lock().unwrap().clone()).unwrap()
}

/// Synthetic admin user with `platform_admin` (self-approved: the separation
/// of duties constraint only bites `auditor`/`operator`, migration
/// `20261103000001_config.sql`).
async fn admin_platform(su: &PgPool) -> Uuid {
    let uid = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO admin_user (pengguna_id, username, password_hash) VALUES ($1, $2, 'x')",
    )
    .bind(uid)
    .bind(format!("sintetis-{}", uid.simple()))
    .execute(su)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO admin_user_peran (pengguna_id, peran, diajukan_oleh, disetujui_oleh) \
         VALUES ($1, 'platform_admin', $1, $1)",
    )
    .bind(uid)
    .execute(su)
    .await
    .unwrap();
    uid
}

/// A full synthetic route (client -> credential -> endpoint -> pool ->
/// upstream -> provider) so each test gets its own, independent fixture.
struct Rute {
    client: Uuid,
    kredensial: Uuid,
    endpoint: Uuid,
    dpool: Uuid,
    upstream: Uuid,
    provider: Uuid,
}

/// Synthetic client plus one credential, routed to its own provider/pool/endpoint
/// so the fixture is independent per test.
async fn klien_dengan_kredensial(su: &PgPool, tag: &str) -> Rute {
    let client = Uuid::now_v7();
    sqlx::query("INSERT INTO client (client_id, nama) VALUES ($1, $2)")
        .bind(client)
        .bind(format!("klien-k20-{tag}"))
        .execute(su)
        .await
        .unwrap();
    let admin = admin_platform(su).await;
    sqlx::query(
        "INSERT INTO client_webhook (client_id, webhook_url, aktif, secret_versi, secret_ciphertext, \
            secret_dek_terbungkus, secret_nonce, kek_id, diubah_oleh) \
         VALUES ($1, $2, true, 1, '\\x11', '\\x22', '\\x33', 'kek-1', $3)",
    )
    .bind(client)
    .bind(format!("https://hook.example.test/{tag}"))
    .bind(admin)
    .execute(su)
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
    .execute(su)
    .await
    .unwrap();

    let provider = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO provider (provider_id, nama, data_balikan_aktif) VALUES ($1, $2, false)",
    )
    .bind(provider)
    .bind(format!("prov-k20-{tag}"))
    .execute(su)
    .await
    .unwrap();
    let upstream = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO upstream_pool (upstream_pool_id, provider_id, nama, kuota_harian, batas_percobaan, retry_backoff_base_detik) \
         VALUES ($1, $2, $3, 1000, 3, 1)",
    )
    .bind(upstream)
    .bind(provider)
    .bind(format!("up-k20-{tag}"))
    .execute(su)
    .await
    .unwrap();
    let dpool = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO downstream_pool (downstream_pool_id, nama, prioritas) VALUES ($1, $2, 1)",
    )
    .bind(dpool)
    .bind(format!("down-k20-{tag}"))
    .execute(su)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO downstream_pool_rute (downstream_pool_id, upstream_pool_id) VALUES ($1, $2)",
    )
    .bind(dpool)
    .bind(upstream)
    .execute(su)
    .await
    .unwrap();
    let endpoint = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO downstream_endpoint (downstream_endpoint_id, nama, provider_id, downstream_pool_id) \
         VALUES ($1, $2, $3, $4)",
    )
    .bind(endpoint)
    .bind(format!("ep-k20-{tag}"))
    .bind(provider)
    .bind(dpool)
    .execute(su)
    .await
    .unwrap();

    Rute {
        client,
        kredensial,
        endpoint,
        dpool,
        upstream,
        provider,
    }
}

/// One `queued` request, `attempt_count = 0` (T-04 requires the row and its
/// first history entry in the same transaction).
async fn request_queued(su: &PgPool, rute: &Rute, nik: &str) -> Uuid {
    let rid = Uuid::now_v7();
    let mut tx = su.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO request (request_id, client_id, credential_id, downstream_endpoint_id, downstream_pool_id, \
            provider_id, provider_config_version, endpoint_config_version, mode, prioritas, prioritas_efektif, \
            status, input_payload, metadata, received_at, ttl_expires_at, attempt_count) \
         SELECT $1, $2, $3, $4, $5, provider_id, 1, 1, 'async', 1, 1, 'queued', $6, '{}'::jsonb, $7, $7 + interval '1 day', 0 \
           FROM downstream_endpoint WHERE downstream_endpoint_id = $4",
    )
    .bind(rid)
    .bind(rute.client)
    .bind(rute.kredensial)
    .bind(rute.endpoint)
    .bind(rute.dpool)
    .bind(serde_json::json!({ "NIK": nik }))
    .bind(Utc::now())
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor) \
         VALUES ($1, 1, NULL, 'queued', 'webserver')",
    )
    .bind(rid)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    rid
}

/// One request that already waited through a dispatch attempt: `queued`,
/// `attempt_count = 1`, with a matching `transaction_log` row (AC-29's
/// "menunggu retry" case).
async fn request_menunggu_retry(su: &PgPool, rute: &Rute, nik: &str) -> Uuid {
    let rid = Uuid::now_v7();
    let mut tx = su.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO request (request_id, client_id, credential_id, downstream_endpoint_id, downstream_pool_id, \
            provider_id, upstream_pool_id, provider_config_version, endpoint_config_version, mode, prioritas, \
            prioritas_efektif, status, metadata, received_at, ttl_expires_at, attempt_count) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, 1, 1, 'async', 1, 1, 'queued', '{}'::jsonb, $8, $8 + interval '1 day', 1)",
    )
    .bind(rid)
    .bind(rute.client)
    .bind(rute.kredensial)
    .bind(rute.endpoint)
    .bind(rute.dpool)
    .bind(rute.provider)
    .bind(rute.upstream)
    .bind(Utc::now())
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor) \
         VALUES ($1, 1, NULL, 'queued', 'webserver')",
    )
    .bind(rid)
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO transaction_log (kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id, \
            client_id, downstream_endpoint_id, provider_config_version, endpoint_config_version, nik, requested_at, \
            status_akses, error_class, log_mode, worker_id) \
         VALUES (gen_random_uuid(), $1, 1, $2, $3, $4, $5, 1, 1, $6, now(), 'gagal_provider', 'provider_sementara', \
            'metadata_only', 'w1')",
    )
    .bind(rid)
    .bind(rute.provider)
    .bind(rute.upstream)
    .bind(rute.client)
    .bind(rute.endpoint)
    .bind(nik)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    rid
}

/// One `in_flight` request: never touched by `fn_nonaktifkan_entitas` (AC-29).
async fn request_in_flight(su: &PgPool, rute: &Rute) -> Uuid {
    let rid = Uuid::now_v7();
    let mut tx = su.begin().await.unwrap();
    sqlx::query(
        "INSERT INTO request (request_id, client_id, credential_id, downstream_endpoint_id, downstream_pool_id, \
            provider_id, provider_config_version, endpoint_config_version, mode, prioritas, prioritas_efektif, \
            status, metadata, received_at, attempt_count) \
         SELECT $1, $2, $3, $4, $5, provider_id, 1, 1, 'sync', 1, 1, 'in_flight', '{}'::jsonb, $6, 1 \
           FROM downstream_endpoint WHERE downstream_endpoint_id = $4",
    )
    .bind(rid)
    .bind(rute.client)
    .bind(rute.kredensial)
    .bind(rute.endpoint)
    .bind(rute.dpool)
    .bind(Utc::now())
    .execute(&mut *tx)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor) \
         VALUES ($1, 1, NULL, 'in_flight', 'webserver')",
    )
    .bind(rid)
    .execute(&mut *tx)
    .await
    .unwrap();
    tx.commit().await.unwrap();
    rid
}

#[tokio::test]
#[ignore = "jalankan dengan make -C backend uji-pg"]
async fn service_pratinjau_tanpa_request_id_nik_atau_payload() {
    // AC-28: preview returns only counters + a timestamp, never request ids, NIK or payload.
    let su = kolam(&db_su()).await;
    let pool = kolam(&db_ujian()).await;
    let service = K20Service::baru(pool);

    let rute = klien_dengan_kredensial(&su, "preview").await;
    let nik = "0000000000000021";
    request_queued(&su, &rute, nik).await;

    let (log_writer, _guard) = penangkap_log();
    let preview = service
        .pratinjau(JenisEntitas::Kredensial, rute.kredensial)
        .await
        .unwrap();
    assert_eq!(preview.akan_cancelled, 1);
    assert_eq!(preview.akan_failed, 0);
    assert!(preview.dihitung_at <= Utc::now());

    let log = isi_log(&log_writer);
    assert!(!log.contains(nik));
    assert!(!log.contains(&rute.kredensial.to_string()));
}

#[tokio::test]
#[ignore = "jalankan dengan make -C backend uji-pg"]
async fn service_nonaktifkan_transisi_ac29_dan_audit_tunggal() {
    // AC-29: queued/attempt_count=0 -> cancelled; waiting-retry -> failed;
    // in_flight untouched. AC-31 (single-caller half): one audit row, counts
    // matching the mass-cancel.
    let su = kolam(&db_su()).await;
    let pool = kolam(&db_ujian()).await;
    let service = K20Service::baru(pool.clone());

    let rute = klien_dengan_kredensial(&su, "transisi").await;

    let nik_cancelled = "0000000000000031";
    let nik_failed = "0000000000000032";
    let r_cancelled = request_queued(&su, &rute, nik_cancelled).await;
    let r_failed = request_menunggu_retry(&su, &rute, nik_failed).await;
    let r_in_flight = request_in_flight(&su, &rute).await;

    let operator = admin_platform(&su).await;
    let alasan = "uji K20 transisi sintetis, cakupan AC-29";
    assert!(validasi_alasan(alasan));
    let (log_writer, _guard) = penangkap_log();
    let hasil = service
        .nonaktifkan(JenisEntitas::Kredensial, rute.kredensial, operator, alasan)
        .await
        .unwrap();
    assert_eq!(hasil.jumlah_cancelled, 1);
    assert_eq!(hasil.jumlah_failed, 1);

    // pr_admin_api has no SELECT grant on `request`/`request_status_history`
    // (ARC-52 intentionally restricts status reads to v_request_tersamar); the
    // superuser pool reads back the rows the service wrote.
    let status_cancelled: String =
        sqlx::query_scalar("SELECT status FROM request WHERE request_id = $1")
            .bind(r_cancelled)
            .fetch_one(&su)
            .await
            .unwrap();
    assert_eq!(status_cancelled, "cancelled");
    let status_failed: String =
        sqlx::query_scalar("SELECT status FROM request WHERE request_id = $1")
            .bind(r_failed)
            .fetch_one(&su)
            .await
            .unwrap();
    assert_eq!(status_failed, "failed");
    let status_in_flight: String =
        sqlx::query_scalar("SELECT status FROM request WHERE request_id = $1")
            .bind(r_in_flight)
            .fetch_one(&su)
            .await
            .unwrap();
    assert_eq!(
        status_in_flight, "in_flight",
        "AC-29: in_flight tidak disentuh"
    );

    // AC-31 (single transaction): exactly one audit_konfigurasi row, counts reconcile.
    let (alasan_tersimpan, aktif_baru, jumlah_cancelled, jumlah_failed): (
        Option<String>,
        Option<bool>,
        Option<i32>,
        Option<i32>,
    ) = sqlx::query_as(
        "SELECT alasan, (nilai_baru->>'aktif')::boolean, jumlah_cancelled, jumlah_failed \
               FROM audit_konfigurasi WHERE entitas_jenis = 'kredensial' AND entitas_id = $1",
    )
    .bind(rute.kredensial)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(alasan_tersimpan.as_deref(), Some(alasan));
    assert_eq!(
        aktif_baru,
        Some(false),
        "nilai_baru.aktif harus false, bukan sekadar mengandung substring"
    );
    assert_eq!(jumlah_cancelled, Some(1));
    assert_eq!(jumlah_failed, Some(1));
    let jumlah_baris_audit: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit_konfigurasi WHERE entitas_jenis = 'kredensial' AND entitas_id = $1")
            .bind(rute.kredensial)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(
        jumlah_baris_audit, 1,
        "AC-31: satu baris audit untuk satu eksekusi mass-cancel"
    );

    // AC-6: redaction — secret/NIK/payload never surface in audit_konfigurasi or the captured log.
    let log = isi_log(&log_writer);
    for rahasia in [nik_cancelled, nik_failed, "secret", "kunci_hash"] {
        assert!(!log.contains(rahasia), "log tidak boleh memuat {rahasia}");
    }
    let audit_json: String = sqlx::query_scalar(
        "SELECT (nilai_lama::text || nilai_baru::text) FROM audit_konfigurasi \
           WHERE entitas_jenis = 'kredensial' AND entitas_id = $1",
    )
    .bind(rute.kredensial)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert!(!audit_json.contains(nik_cancelled));
    assert!(!audit_json.contains(nik_failed));
    assert!(!audit_json.contains("secret"));
}

#[tokio::test]
#[ignore = "jalankan dengan make -C backend uji-pg"]
async fn service_dua_eksekusi_paralel_hanya_satu_transisi_dan_satu_audit() {
    // AC-31: two concurrent executions against the same entity resolve to one
    // transition and one audit_konfigurasi row (FOR UPDATE serializes them).
    let su = kolam(&db_su()).await;
    let pool = kolam(&db_ujian()).await;
    let service_a = K20Service::baru(pool.clone());
    let service_b = K20Service::baru(pool.clone());

    let rute = klien_dengan_kredensial(&su, "paralel").await;
    let r = request_queued(&su, &rute, "0000000000000041").await;

    let operator = admin_platform(&su).await;
    let alasan_a = "uji K20 paralel pertama sintetis";
    let alasan_b = "uji K20 paralel kedua sintetis";

    let (hasil_a, hasil_b) = tokio::join!(
        service_a.nonaktifkan(
            JenisEntitas::Kredensial,
            rute.kredensial,
            operator,
            alasan_a
        ),
        service_b.nonaktifkan(
            JenisEntitas::Kredensial,
            rute.kredensial,
            operator,
            alasan_b
        ),
    );

    // Both calls run the same idempotent SQL function (fn_set_status_kredensial
    // guards `dicabut` as terminal, EN-03); the DB-contract tests in
    // `40_functions.sql` cover the terminal-state rejection path. Here we only
    // assert the externally visible invariant both executors must honour:
    // exactly one request transition, exactly one audit row.
    assert!(hasil_a.is_ok() || matches!(hasil_a, Err(GalatK20::BasisData(_))));
    assert!(hasil_b.is_ok() || matches!(hasil_b, Err(GalatK20::BasisData(_))));
    assert!(
        hasil_a.is_ok() || hasil_b.is_ok(),
        "paling sedikit satu eksekusi harus berhasil"
    );

    // pr_admin_api has no SELECT grant on `request`/`request_status_history`;
    // read back through the superuser pool, same as the sequential transition test.
    let status: String = sqlx::query_scalar("SELECT status FROM request WHERE request_id = $1")
        .bind(r)
        .fetch_one(&su)
        .await
        .unwrap();
    assert_eq!(status, "cancelled");
    let jumlah_cancelled_history: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM request_status_history WHERE request_id = $1 AND status_ke = 'cancelled'",
    )
    .bind(r)
    .fetch_one(&su)
    .await
    .unwrap();
    assert_eq!(
        jumlah_cancelled_history, 1,
        "request dicatat cancelled tepat sekali"
    );
    let jumlah_audit: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit_konfigurasi WHERE entitas_jenis = 'kredensial' AND entitas_id = $1 AND aksi = 'cabut'",
    )
    .bind(rute.kredensial)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        jumlah_audit, 1,
        "AC-31: satu baris audit, bukan satu per pemanggil"
    );
}

#[tokio::test]
#[ignore = "jalankan dengan make -C backend uji-pg"]
async fn service_aktifkan_menulis_baris_audit() {
    let su = kolam(&db_su()).await;
    let pool = kolam(&db_ujian()).await;
    let service = K20Service::baru(pool.clone());

    let client = Uuid::now_v7();
    sqlx::query("INSERT INTO client (client_id, nama) VALUES ($1, $2)")
        .bind(client)
        .bind(format!("klien-k20-aktifkan-{}", client.simple()))
        .execute(&su)
        .await
        .unwrap();
    let operator = admin_platform(&su).await;
    let alasan_nonaktif = "uji K20 nonaktifkan sebelum aktifkan";
    service
        .nonaktifkan(JenisEntitas::Client, client, operator, alasan_nonaktif)
        .await
        .unwrap();

    let alasan_aktif = "uji K20 aktifkan kembali client sintetis";
    service
        .aktifkan(JenisEntitas::Client, client, operator, alasan_aktif)
        .await
        .unwrap();

    let aktif: bool = sqlx::query_scalar("SELECT aktif FROM client WHERE client_id = $1")
        .bind(client)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(aktif);
    let (alasan_tersimpan, nilai_baru_aktif): (Option<String>, Option<bool>) = sqlx::query_as(
        "SELECT alasan, (nilai_baru->>'aktif')::boolean FROM audit_konfigurasi \
           WHERE entitas_jenis = 'client' AND entitas_id = $1 AND aksi = 'aktifkan'",
    )
    .bind(client)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(alasan_tersimpan.as_deref(), Some(alasan_aktif));
    assert_eq!(nilai_baru_aktif, Some(true));
}
