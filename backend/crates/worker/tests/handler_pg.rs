//! Handler worker terhadap PostgreSQL, Redis, dan mock Dukcapil sungguhan
//! (ARC-71, AC-12..AC-18; seam CW10).
//!
//! Dijalankan oleh `make -C backend uji-pg` (dan `uji-pg-worker`). Fixture
//! ditulis sebagai superuser; handler berjalan di pool `pr_worker` sehingga
//! grant kolom ikut teruji. `KuotaStore` dan skrip Lua tidak di-mock: slot
//! dipesan lewat `ambil_token` seperti dispatcher. Provider diganti wiremock.
//! Data sintetis saja: NIK berawalan `00`, secret sintetis.
//!
//! Variabel lingkungan wajib ada; uji gagal (bukan dilewati) bila tidak diset,
//! supaya gate tidak hijau tanpa uji ini berjalan.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration as StdDuration;

use chrono::{DateTime, Duration, Utc};
use poolrouter_core::admin::NilaiAman;
use poolrouter_core::kuota::SumberDana;
use poolrouter_core::kuota::pemulihan::{
    GalatPemulihan, JendelaRate, KeadaanPool, SumberRebuild, baca_run_id, bangun_ulang, kunci_slot,
    member_slot,
};
use poolrouter_core::kuota::store::{
    AsalLepas, Bacaan, GalatStore, HasilIntake, HasilKlaim, HasilPenolakan, HasilTambah,
    HasilToken, IntakeAlokasi, KonteksKuota, KuotaRedis, KuotaStore, ModeJendela, PermintaanToken,
};
use poolrouter_core::queue::{JobConsumer, QueueError};
use poolrouter_core::retry::AcakBatasAtas;
use poolrouter_core::secret::Brankas;
use poolrouter_worker::handler::{Handler, Hasil};
use poolrouter_worker::konsumen;
use poolrouter_worker::upstream::PemanggilReqwest;
use redis::aio::MultiplexedConnection;
use serde_json::{Value, json};
use sqlx::{PgPool, Row};
use uuid::Uuid;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const KEK_UJI: &str = "a2VrLXVqaS1hcmM3My1wcm9kdWtzaS1qYW5nYW4tZGk=";
const NIK: &str = "0012345678901234";
const SECRET: &str = "rahasia-sintetis-arc71";

fn env(nama: &str) -> String {
    std::env::var(nama)
        .unwrap_or_else(|_| panic!("{nama} belum diset; jalankan lewat `make -C backend uji-pg`"))
}

async fn pool(url: &str) -> PgPool {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(12)
        .connect(url)
        .await
        .expect("koneksi PostgreSQL uji")
}

fn brankas() -> Brankas {
    Brankas::dari_isi_kek(KEK_UJI).expect("KEK uji")
}

struct Kosong;

impl SumberRebuild for Kosong {
    async fn keadaan(&self, _: DateTime<Utc>) -> Result<Vec<KeadaanPool>, GalatPemulihan> {
        Ok(vec![])
    }
}

/// Redis dikosongkan lalu ditandai siap untuk `run_id`-nya (`pr:siap`).
async fn redis_siap() -> (MultiplexedConnection, KuotaRedis<MultiplexedConnection>) {
    let mut c = redis::Client::open(env("POOLROUTER_TEST_REDIS"))
        .expect("URL Redis")
        .get_multiplexed_async_connection()
        .await
        .expect("Redis uji");
    redis::cmd("FLUSHALL")
        .query_async::<()>(&mut c)
        .await
        .unwrap();
    let store = KuotaRedis::baru(c.clone()).await.unwrap();
    let run_id = baca_run_id(&mut c).await.unwrap();
    bangun_ulang(&mut c, &run_id, &Kosong, Utc::now())
        .await
        .unwrap();
    (c, store)
}

struct Lingkungan {
    su: PgPool,
    worker: PgPool,
    redis: MultiplexedConnection,
    store: KuotaRedis<MultiplexedConnection>,
    mock: MockServer,
}

async fn lingkungan() -> Lingkungan {
    let su = pool(&env("POOLROUTER_TEST_DATABASE_URL_SU")).await;
    let worker = pool(&env("POOLROUTER_TEST_DATABASE_URL_WORKER")).await;
    let (redis, store) = redis_siap().await;
    let mock = MockServer::start().await;
    Lingkungan {
        su,
        worker,
        redis,
        store,
        mock,
    }
}

impl Lingkungan {
    fn handler<K: KuotaStore>(&self, kuota: K) -> Handler<PemanggilReqwest, K> {
        Handler::baru(
            self.worker.clone(),
            kuota,
            PemanggilReqwest::baru().expect("klien reqwest"),
            brankas(),
            Box::new(AcakBatasAtas),
            "worker-uji-arc71",
        )
    }

    async fn panggilan(&self) -> Vec<wiremock::Request> {
        self.mock.received_requests().await.unwrap_or_default()
    }

    async fn pasang(&self, kode: u16, body: Value, prioritas: u8, kali: Option<u64>) {
        let mut m = Mock::given(method("POST"))
            .and(path("/kesesuaian"))
            .respond_with(ResponseTemplate::new(kode).set_body_json(body))
            .with_priority(prioritas);
        if let Some(n) = kali {
            m = m.up_to_n_times(n);
        }
        m.mount(&self.mock).await;
    }
}

fn respons_sukses() -> Value {
    json!({"content": [{
        "NAMA_LGKP": "Sesuai (90)",
        "TMPT_LHR": "Tidak Sesuai",
        "NIK": "Sesuai",
        "UID": SECRET
    }]})
}

struct Fixture {
    su: PgPool,
    admin_id: Uuid,
    client_id: Uuid,
    kredensial_id: Uuid,
    provider_id: Uuid,
    downstream_pool_id: Uuid,
    upstream_pool_id: Uuid,
    endpoint_id: Uuid,
}

impl Fixture {
    async fn baru(l: &Lingkungan, timeout_detik: i64) -> Self {
        let su = l.su.clone();
        let admin_id = Uuid::now_v7();
        let client_id = Uuid::now_v7();
        let kredensial_id = Uuid::now_v7();
        let provider_id = Uuid::now_v7();
        let downstream_pool_id = Uuid::now_v7();
        let upstream_pool_id = Uuid::now_v7();
        let endpoint_id = Uuid::now_v7();
        let akhiran = client_id.simple().to_string();
        let nama = format!("arc71-{akhiran}");
        let nama_secret = format!("U{}", &akhiran[..24]);

        sqlx::query(
            "INSERT INTO admin_user (pengguna_id, username, password_hash, bahasa_ui)
             VALUES ($1, $2, 'x', 'id')",
        )
        .bind(admin_id)
        .bind(&nama)
        .execute(&su)
        .await
        .expect("admin_user");
        sqlx::query("INSERT INTO client (client_id, nama) VALUES ($1, $2)")
            .bind(client_id)
            .bind(&nama)
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
        sqlx::query("INSERT INTO provider (provider_id, nama) VALUES ($1, $2)")
            .bind(provider_id)
            .bind(&nama)
            .execute(&su)
            .await
            .expect("provider");
        let body_template = format!(
            "{{\"NIK\":\"{{{{input.NIK}}}}\",\"NAMA_LGKP\":\"{{{{input.NAMA_LGKP}}}}\",\
             \"USER_ID\":\"{{{{secret.{nama_secret}}}}}\",\"TRESHOLD\":\"{{{{endpoint.TRESHOLD}}}}\",\
             \"transactionId\":\"{{{{system.kode_transaksi}}}}\"}}"
        );
        let dokumen = json!({
            "metode": "POST",
            "base_url": format!("{}/kesesuaian", l.mock.uri()),
            "body_template": body_template,
            "parameters": [],
            "respons_skema": {
                "parser": "status_skor",
                "literal": [
                    {"teks": "Sesuai", "status": "sesuai"},
                    {"teks": "Tidak Sesuai", "status": "tidak_sesuai"}
                ],
                "parameter_output": [{"nama": "NAMA_LGKP"}, {"nama": "TMPT_LHR"}]
            },
            "tls_trust": "system",
            "log_mode": "full_payload"
        });
        sqlx::query(
            "INSERT INTO provider_config_versi
                    (provider_id, versi, config_schema_version, log_mode, dokumen, dibuat_oleh)
             VALUES ($1, 1, 1, 'full_payload', $2, $3)",
        )
        .bind(provider_id)
        .bind(sqlx::types::Json(dokumen))
        .bind(admin_id)
        .execute(&su)
        .await
        .expect("provider_config_versi");
        sqlx::query("UPDATE provider SET config_versi_aktif = 1 WHERE provider_id = $1")
            .bind(provider_id)
            .execute(&su)
            .await
            .expect("provider versi aktif");

        let amplop = brankas()
            .enkripsi(&nama_secret, &NilaiAman::baru(SECRET.to_owned()))
            .expect("enkripsi secret");
        sqlx::query(
            "INSERT INTO provider_secret (nama_secret, secret_ciphertext, secret_dek_terbungkus,
                     secret_nonce, kek_id, diubah_oleh)
             VALUES ($1, $2, $3, $4, $5, $6)",
        )
        .bind(&nama_secret)
        .bind(amplop.ciphertext.clone())
        .bind(amplop.dek_terbungkus.clone())
        .bind(amplop.nonce.to_vec())
        .bind(amplop.kek_id.clone())
        .bind(admin_id)
        .execute(&su)
        .await
        .expect("provider_secret");

        sqlx::query(
            "INSERT INTO downstream_pool (downstream_pool_id, nama, prioritas) VALUES ($1, $2, 1)",
        )
        .bind(downstream_pool_id)
        .bind(&nama)
        .execute(&su)
        .await
        .expect("downstream_pool");
        sqlx::query(
            "INSERT INTO upstream_pool (upstream_pool_id, provider_id, nama, kuota_harian,
                     parameter_dispatch, reset_kuota_jam, reset_kuota_zona)
             VALUES ($1, $2, $3, 1000, $4, '00:00', 'Asia/Jakarta')",
        )
        .bind(upstream_pool_id)
        .bind(provider_id)
        .bind(&nama)
        .bind(sqlx::types::Json(
            json!({"timeout_total_upstream_detik": timeout_detik}),
        ))
        .execute(&su)
        .await
        .expect("upstream_pool");
        sqlx::query(
            "INSERT INTO downstream_pool_rute (downstream_pool_id, upstream_pool_id) VALUES ($1, $2)",
        )
        .bind(downstream_pool_id)
        .bind(upstream_pool_id)
        .execute(&su)
        .await
        .expect("downstream_pool_rute");
        sqlx::query(
            "INSERT INTO downstream_endpoint (downstream_endpoint_id, nama, provider_id, downstream_pool_id)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(endpoint_id)
        .bind(&nama)
        .bind(provider_id)
        .bind(downstream_pool_id)
        .execute(&su)
        .await
        .expect("downstream_endpoint");
        sqlx::query(
            "INSERT INTO endpoint_config_versi
                    (downstream_endpoint_id, versi, config_schema_version, dokumen, dibuat_oleh)
             VALUES ($1, 1, 1, $2, $3)",
        )
        .bind(endpoint_id)
        .bind(sqlx::types::Json(
            json!({"parameter_literal": {"TRESHOLD": "80"}}),
        ))
        .bind(admin_id)
        .execute(&su)
        .await
        .expect("endpoint_config_versi");
        sqlx::query(
            "UPDATE downstream_endpoint SET config_versi_aktif = 1 WHERE downstream_endpoint_id = $1",
        )
        .bind(endpoint_id)
        .execute(&su)
        .await
        .expect("endpoint versi aktif");

        Self {
            su,
            admin_id,
            client_id,
            kredensial_id,
            provider_id,
            downstream_pool_id,
            upstream_pool_id,
            endpoint_id,
        }
    }

    fn pool_redis(&self) -> String {
        self.upstream_pool_id.to_string()
    }

    /// Request async `dispatched` dengan `attempt_awal` percobaan sebelumnya
    /// (baris log lengkap `timeout`), seperti hasil dispatcher.
    async fn request(&self, attempt_awal: i32, ttl: StdDuration) -> Uuid {
        let request_id = Uuid::now_v7();
        let mut tx = self.su.begin().await.expect("tx fixture");
        sqlx::query(
            "INSERT INTO request
                    (request_id, client_id, credential_id, downstream_endpoint_id,
                     downstream_pool_id, provider_id, upstream_pool_id,
                     provider_config_version, endpoint_config_version, mode,
                     prioritas, prioritas_efektif, status, input_payload, metadata, received_at,
                     ttl_expires_at, attempt_count, dispatch_key)
             VALUES ($1, $2, $3, $4, $5, $6, $7, 1, 1, 'async', 1, 1, 'queued', $8, '{}'::jsonb,
                     now(), $9, $10, $11)",
        )
        .bind(request_id)
        .bind(self.client_id)
        .bind(self.kredensial_id)
        .bind(self.endpoint_id)
        .bind(self.downstream_pool_id)
        .bind(self.provider_id)
        .bind(self.upstream_pool_id)
        .bind(sqlx::types::Json(json!({"NIK": NIK, "NAMA_LGKP": "BUDI"})))
        .bind(Utc::now() + Duration::from_std(ttl).unwrap())
        .bind(attempt_awal)
        .bind(Uuid::now_v7())
        .execute(&mut *tx)
        .await
        .expect("request");
        sqlx::query(
            "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor)
             VALUES ($1, 1, NULL, 'queued', 'webserver')",
        )
        .bind(request_id)
        .execute(&mut *tx)
        .await
        .expect("history queued");
        for no in 1..=attempt_awal {
            sqlx::query(
                "INSERT INTO transaction_log (kode_transaksi, request_id, attempt_no, provider_id,
                         upstream_pool_id, client_id, downstream_endpoint_id, provider_config_version,
                         endpoint_config_version, nik, requested_at, responded_at, status_akses,
                         error_class, log_mode, worker_id)
                 VALUES ($1, $2, $3, $4, $5, $6, $7, 1, 1, $8, now() - interval '1 minute', NULL,
                         'gagal_jaringan', 'timeout', 'metadata_only', 'uji')",
            )
            .bind(Uuid::now_v7())
            .bind(request_id)
            .bind(no)
            .bind(self.provider_id)
            .bind(self.upstream_pool_id)
            .bind(self.client_id)
            .bind(self.endpoint_id)
            .bind(NIK)
            .execute(&mut *tx)
            .await
            .expect("log percobaan sebelumnya");
        }
        tx.commit().await.expect("commit fixture");
        self.dispatch(request_id).await;
        request_id
    }

    /// `queued → dispatched` seperti dispatcher (tanpa slot).
    async fn dispatch(&self, request_id: Uuid) {
        let mut tx = self.su.begin().await.expect("tx dispatch");
        sqlx::query(
            "UPDATE request SET status = 'dispatched', not_before = NULL WHERE request_id = $1",
        )
        .bind(request_id)
        .execute(&mut *tx)
        .await
        .expect("queued -> dispatched");
        sqlx::query(
            "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor)
             SELECT $1, max(seq) + 1, 'queued', 'dispatched', 'controller'
               FROM request_status_history WHERE request_id = $1",
        )
        .bind(request_id)
        .execute(&mut *tx)
        .await
        .expect("history dispatched");
        tx.commit().await.expect("commit dispatch");
    }

    async fn nonaktifkan(&self, jenis: &str) {
        let sql = match jenis {
            "client" => "UPDATE client SET aktif = false WHERE client_id = $1",
            "kredensial" => {
                "UPDATE client_kredensial SET status = 'dicabut', dicabut_at = now(), dicabut_oleh = $2
                  WHERE credential_id = $1"
            }
            "endpoint" => "UPDATE downstream_endpoint SET aktif = false WHERE downstream_endpoint_id = $1",
            "downstream_pool" => "UPDATE downstream_pool SET aktif = false WHERE downstream_pool_id = $1",
            "upstream_pool" => "UPDATE upstream_pool SET aktif = false WHERE upstream_pool_id = $1",
            "provider" => "UPDATE provider SET aktif = false WHERE provider_id = $1",
            lain => panic!("jenis tidak dikenal {lain}"),
        };
        let id = match jenis {
            "client" => self.client_id,
            "kredensial" => self.kredensial_id,
            "endpoint" => self.endpoint_id,
            "downstream_pool" => self.downstream_pool_id,
            "upstream_pool" => self.upstream_pool_id,
            _ => self.provider_id,
        };
        let q = sqlx::query(sql).bind(id);
        let q = if jenis == "kredensial" {
            q.bind(self.admin_id)
        } else {
            q
        };
        q.execute(&self.su).await.expect("nonaktifkan entitas");
    }

    async fn webhook_aktif(&self) {
        sqlx::query(
            "INSERT INTO client_webhook (client_id, webhook_url, aktif, diubah_oleh)
             VALUES ($1, 'https://hook.contoh.test/h', true, $2)",
        )
        .bind(self.client_id)
        .bind(self.admin_id)
        .execute(&self.su)
        .await
        .expect("client_webhook");
    }
}

/// Pesan slot percobaan `nomor` lewat skrip token sungguhan (seperti dispatcher).
async fn pesan_slot(store: &KuotaRedis<MultiplexedConnection>, f: &Fixture, id: Uuid, nomor: i64) {
    let sekarang = Utc::now();
    let ctx = KonteksKuota {
        pool_id: f.pool_redis(),
        periode_mulai: sekarang - Duration::hours(1),
        requested_at: sekarang,
        mode: ModeJendela::Bergulir,
        kuota_efektif: 990,
    };
    let token = store
        .ambil_token(
            &ctx,
            &PermintaanToken {
                request_id: id.to_string(),
                nomor_percobaan: nomor,
                sync: false,
                maks_konkurensi: 50,
                ttl_slot_mikro: 120_000_000,
                rasio_retry: 0.1,
                retry_min: 20,
                porsi_sync: 0.2,
                jendela_rate: vec![JendelaRate {
                    jenis: "tps".into(),
                    t_mikro: 1_000,
                    burst: 10_000,
                }],
            },
        )
        .await
        .expect("ambil_token");
    assert!(token.diberikan(), "{token:?}");
}

struct Baris {
    status: String,
    attempt_count: i32,
    final_kode: Option<Uuid>,
    output: Option<Value>,
    webhook_status: Option<String>,
    failure_reason: Option<String>,
    cancel_reason: Option<String>,
    cancelled_by: Option<String>,
    not_before: Option<DateTime<Utc>>,
    purged: bool,
}

async fn baris_request(su: &PgPool, id: Uuid) -> Baris {
    let b = sqlx::query(
        "SELECT status, attempt_count, final_kode_transaksi, output_payload, webhook_status,
                failure_reason, cancel_reason, cancelled_by, not_before,
                payload_purged_at IS NOT NULL AS purged
           FROM request WHERE request_id = $1",
    )
    .bind(id)
    .fetch_one(su)
    .await
    .expect("baca request");
    Baris {
        status: b.get("status"),
        attempt_count: b.get("attempt_count"),
        final_kode: b.get("final_kode_transaksi"),
        output: b
            .get::<Option<sqlx::types::Json<Value>>, _>("output_payload")
            .map(|j| j.0),
        webhook_status: b.get("webhook_status"),
        failure_reason: b.get("failure_reason"),
        cancel_reason: b.get("cancel_reason"),
        cancelled_by: b.get("cancelled_by"),
        not_before: b.get("not_before"),
        purged: b.get("purged"),
    }
}

struct BarisLog {
    kode: Uuid,
    status_akses: Option<String>,
    error_class: Option<String>,
    http: Option<i32>,
    responded: bool,
    request_payload: Option<String>,
    response_payload: Option<String>,
    nik: Option<String>,
}

async fn log_request(su: &PgPool, id: Uuid) -> Vec<BarisLog> {
    sqlx::query(
        "SELECT kode_transaksi, status_akses, error_class, provider_http_status,
                responded_at IS NOT NULL AS responded, request_payload::text AS rq,
                response_payload::text AS rs, nik::text AS nik
           FROM transaction_log WHERE request_id = $1 ORDER BY attempt_no",
    )
    .bind(id)
    .fetch_all(su)
    .await
    .expect("baca log")
    .into_iter()
    .map(|b| BarisLog {
        kode: b.get("kode_transaksi"),
        status_akses: b.get("status_akses"),
        error_class: b.get("error_class"),
        http: b.get("provider_http_status"),
        responded: b.get("responded"),
        request_payload: b.get("rq"),
        response_payload: b.get("rs"),
        nik: b.get("nik"),
    })
    .collect()
}

async fn slot_ada(c: &MultiplexedConnection, f: &Fixture, id: Uuid, nomor: i64) -> bool {
    let mut c = c.clone();
    let skor: Option<f64> = redis::cmd("ZSCORE")
        .arg(kunci_slot(&f.pool_redis()))
        .arg(member_slot(&id.to_string(), nomor))
        .query_async(&mut c)
        .await
        .expect("ZSCORE");
    skor.is_some()
}

/// AC-14 + AC-16: jalur sukses lengkap, urutan dibuktikan dari baris DB,
/// counter Redis, dan panggilan mock; secret tidak pernah sampai ke log.
#[tokio::test]
#[ignore = "butuh PostgreSQL + Redis: make -C backend uji-pg"]
async fn sukses_log_sebelum_panggilan_lalu_done() {
    let l = lingkungan().await;
    let f = Fixture::baru(&l, 5).await;
    l.pasang(200, respons_sukses(), 1, None).await;
    let id = f.request(0, StdDuration::from_secs(3600)).await;
    pesan_slot(&l.store, &f, id, 1).await;

    let hasil = l.handler(l.store.clone()).proses(id).await.expect("proses");
    assert_eq!(hasil, Hasil::Done);

    let r = baris_request(&l.su, id).await;
    assert_eq!(r.status, "done");
    assert_eq!(r.attempt_count, 1);
    assert_eq!(
        r.output,
        Some(json!({"NAMA_LGKP": {"status": "sesuai", "skor": 90}})),
        "TMPT_LHR tidak dikirim client sehingga dibuang (P-1); NIK bukan parameter output"
    );
    assert_eq!(r.webhook_status.as_deref(), Some("tidak_dikirim"));

    let log = log_request(&l.su, id).await;
    assert_eq!(log.len(), 1);
    assert_eq!(r.final_kode, Some(log[0].kode));
    assert_eq!(log[0].kode.get_version_num(), 7, "kode_transaksi UUIDv7");
    assert_eq!(log[0].status_akses.as_deref(), Some("berhasil"));
    assert_eq!(log[0].http, Some(200));
    assert_eq!(log[0].nik.as_deref(), Some(NIK));
    let rq = log[0].request_payload.clone().expect("full_payload");
    let rs = log[0].response_payload.clone().expect("full_payload");
    assert!(
        !rq.contains(SECRET) && !rs.contains(SECRET),
        "secret bocor ke log"
    );
    assert!(rq.contains("{{secret."));
    assert!(rs.contains("{{secret."));
    assert!(rq.contains(&log[0].kode.to_string()));

    let panggilan = l.panggilan().await;
    assert_eq!(panggilan.len(), 1);
    let body = String::from_utf8(panggilan[0].body.clone()).unwrap();
    assert!(body.contains(SECRET), "kabel memakai nilai secret asli");
    assert!(body.contains(&format!("\"transactionId\":\"{}\"", log[0].kode)));
    assert!(body.contains("\"TRESHOLD\":\"80\""));

    // INCR attempts sekali, lalu slot dilepas.
    let mut c = l.redis.clone();
    let kunci: Vec<String> = redis::cmd("KEYS")
        .arg(format!("pr:attempts:{{{}}}:*", f.pool_redis()))
        .query_async(&mut c)
        .await
        .unwrap();
    assert_eq!(kunci.len(), 1);
    let attempts: i64 = redis::cmd("GET")
        .arg(&kunci[0])
        .query_async(&mut c)
        .await
        .unwrap();
    assert_eq!(attempts, 1);
    assert!(!slot_ada(&l.redis, &f, id, 1).await);
}

/// Consumer palsu: mengirim daftar `request_id` berurutan ke handler.
struct Daftar(Vec<Uuid>);

impl JobConsumer for Daftar {
    fn run<H, F>(
        &self,
        _queue: &str,
        handler: H,
    ) -> impl Future<Output = Result<(), QueueError>> + Send
    where
        H: Fn(Uuid) -> F + Send + Sync + 'static,
        F: Future<Output = Result<(), QueueError>> + Send + 'static,
    {
        let daftar = self.0.clone();
        async move {
            for id in daftar {
                handler(id).await?;
            }
            Ok(())
        }
    }
}

/// AC-12: 2.000 pesan duplikat → 0 panggilan ganda, 0 baris log baru; pesan
/// untuk request `queued` di-ack tanpa panggilan.
#[tokio::test]
#[ignore = "butuh PostgreSQL + Redis: make -C backend uji-pg"]
async fn duplikat_2000_tanpa_panggilan_ganda() {
    let l = lingkungan().await;
    let f = Fixture::baru(&l, 5).await;
    l.pasang(200, respons_sukses(), 1, None).await;
    let id = f.request(0, StdDuration::from_secs(3600)).await;
    pesan_slot(&l.store, &f, id, 1).await;

    let handler = Arc::new(l.handler(l.store.clone()));
    konsumen::jalankan(&Daftar(vec![id; 2000]), "uji", Arc::clone(&handler))
        .await
        .expect("consumer");
    assert_eq!(l.panggilan().await.len(), 1);
    assert_eq!(log_request(&l.su, id).await.len(), 1);
    assert_eq!(baris_request(&l.su, id).await.status, "done");

    // Request yang masih `queued`: CAS nol baris, ack, tanpa log.
    let antre = f.request(0, StdDuration::from_secs(3600)).await;
    let mut tx = l.su.begin().await.expect("tx");
    sqlx::query("UPDATE request SET status = 'queued' WHERE request_id = $1")
        .bind(antre)
        .execute(&mut *tx)
        .await
        .expect("dispatched -> queued");
    sqlx::query(
        "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor)
         SELECT $1, max(seq) + 1, 'dispatched', 'queued', 'controller'
           FROM request_status_history WHERE request_id = $1",
    )
    .bind(antre)
    .execute(&mut *tx)
    .await
    .expect("history queued");
    tx.commit().await.expect("commit");
    let h = handler.proses(antre).await.expect("proses");
    assert_eq!(h, Hasil::BukanDispatched);
    assert_eq!(log_request(&l.su, antre).await.len(), 0);
    assert_eq!(l.panggilan().await.len(), 1);
}

/// AC-13 (U-49, U-51): gerbang dispatch untuk keenam entitas, urutan alasan
/// tetap, dan request menunggu retry → `failed`/`objek_nonaktif`.
#[tokio::test]
#[ignore = "butuh PostgreSQL + Redis: make -C backend uji-pg"]
async fn gerbang_dispatch_enam_entitas() {
    let l = lingkungan().await;
    l.pasang(200, respons_sukses(), 1, None).await;
    let kasus = [
        ("client", "client_nonaktif"),
        ("kredensial", "kredensial_nonaktif"),
        ("endpoint", "endpoint_nonaktif"),
        ("downstream_pool", "pool_nonaktif"),
        ("upstream_pool", "pool_nonaktif"),
        ("provider", "provider_nonaktif"),
    ];
    for (jenis, alasan) in kasus {
        let f = Fixture::baru(&l, 5).await;
        let id = f.request(0, StdDuration::from_secs(3600)).await;
        pesan_slot(&l.store, &f, id, 1).await;
        f.nonaktifkan(jenis).await;

        let h = l.handler(l.store.clone()).proses(id).await.expect("proses");
        assert_eq!(
            h,
            Hasil::Gerbang {
                status: "cancelled",
                alasan: alasan.to_owned()
            },
            "{jenis}"
        );
        let r = baris_request(&l.su, id).await;
        assert_eq!(r.status, "cancelled", "{jenis}");
        assert_eq!(r.cancel_reason.as_deref(), Some(alasan), "{jenis}");
        assert_eq!(r.cancelled_by.as_deref(), Some("sistem"));
        assert_eq!(r.attempt_count, 0);
        assert!(r.purged, "P-02 di transaksi yang sama ({jenis})");
        assert_eq!(log_request(&l.su, id).await.len(), 0, "{jenis}");
        let objek: Option<Uuid> = sqlx::query_scalar(
            "SELECT objek_pemicu_id FROM request_status_history
              WHERE request_id = $1 AND status_ke = 'cancelled'",
        )
        .bind(id)
        .fetch_one(&l.su)
        .await
        .expect("history gerbang");
        assert!(objek.is_some(), "{jenis}");
        assert!(
            !slot_ada(&l.redis, &f, id, 1).await,
            "slot dilepas ({jenis})"
        );
    }

    // U-51: client dan provider nonaktif → alasan client lebih dulu.
    let f = Fixture::baru(&l, 5).await;
    let id = f.request(0, StdDuration::from_secs(3600)).await;
    pesan_slot(&l.store, &f, id, 1).await;
    f.nonaktifkan("provider").await;
    f.nonaktifkan("client").await;
    l.handler(l.store.clone()).proses(id).await.expect("proses");
    assert_eq!(
        baris_request(&l.su, id).await.cancel_reason.as_deref(),
        Some("client_nonaktif")
    );

    // U-49 (b): menunggu retry → failed/objek_nonaktif, log tidak bertambah.
    let f = Fixture::baru(&l, 5).await;
    let id = f.request(1, StdDuration::from_secs(3600)).await;
    pesan_slot(&l.store, &f, id, 2).await;
    f.nonaktifkan("endpoint").await;
    let h = l.handler(l.store.clone()).proses(id).await.expect("proses");
    assert_eq!(
        h,
        Hasil::Gerbang {
            status: "failed",
            alasan: "objek_nonaktif".to_owned()
        }
    );
    let r = baris_request(&l.su, id).await;
    assert_eq!(r.failure_reason.as_deref(), Some("objek_nonaktif"));
    let log = log_request(&l.su, id).await;
    assert_eq!(log.len(), 1);
    assert_eq!(r.final_kode, Some(log[0].kode));

    assert_eq!(
        l.panggilan().await.len(),
        0,
        "gerbang tidak pernah memanggil provider"
    );
}

/// AC-17 (K-25): kelas yang di-retry kembali `queued` dengan `not_before` batas
/// atas; percobaan ulang memakai slot dan `kode_transaksi` baru.
#[tokio::test]
#[ignore = "butuh PostgreSQL + Redis: make -C backend uji-pg"]
async fn retry_lalu_percobaan_baru_dengan_kode_baru() {
    let l = lingkungan().await;
    let f = Fixture::baru(&l, 5).await;
    l.pasang(503, json!({"error": "sibuk"}), 1, Some(1)).await;
    l.pasang(200, respons_sukses(), 2, None).await;
    let id = f.request(0, StdDuration::from_secs(3600)).await;
    pesan_slot(&l.store, &f, id, 1).await;

    let sebelum = Utc::now();
    let h = l.handler(l.store.clone()).proses(id).await.expect("proses");
    let sesudah = Utc::now();
    let Hasil::Retry(not_before) = h else {
        panic!("harus retry: {h:?}");
    };
    // n = 1 → batas atas min(300, 5 × 2^0) = 5 s.
    assert!(not_before >= sebelum + Duration::seconds(5));
    assert!(not_before <= sesudah + Duration::seconds(5));
    let r = baris_request(&l.su, id).await;
    assert_eq!(r.status, "queued");
    let selisih = r.not_before.expect("not_before") - not_before;
    assert!(
        selisih.num_milliseconds().abs() < 1,
        "presisi mikrodetik PostgreSQL"
    );
    assert_eq!(r.attempt_count, 1);
    assert_eq!(r.final_kode, None);
    assert_eq!(r.webhook_status, None);
    let log = log_request(&l.su, id).await;
    assert_eq!(log[0].status_akses.as_deref(), Some("gagal_provider"));
    assert_eq!(log[0].error_class.as_deref(), Some("provider_sementara"));
    assert_eq!(log[0].http, Some(503));
    assert!(!slot_ada(&l.redis, &f, id, 1).await);

    f.dispatch(id).await;
    pesan_slot(&l.store, &f, id, 2).await;
    let h = l
        .handler(l.store.clone())
        .proses(id)
        .await
        .expect("proses ulang");
    assert_eq!(h, Hasil::Done);
    let log = log_request(&l.su, id).await;
    assert_eq!(log.len(), 2);
    assert_ne!(log[0].kode, log[1].kode);
    assert_eq!(baris_request(&l.su, id).await.final_kode, Some(log[1].kode));
    assert_eq!(l.panggilan().await.len(), 2);
}

/// AC-17: timeout upstream → `gagal_jaringan`/`timeout`, tanpa `responded_at`,
/// lalu retry.
#[tokio::test]
#[ignore = "butuh PostgreSQL + Redis: make -C backend uji-pg"]
async fn timeout_upstream_gagal_jaringan_lalu_retry() {
    let l = lingkungan().await;
    let f = Fixture::baru(&l, 1).await;
    Mock::given(method("POST"))
        .and(path("/kesesuaian"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(respons_sukses())
                .set_delay(StdDuration::from_secs(3)),
        )
        .mount(&l.mock)
        .await;
    let id = f.request(0, StdDuration::from_secs(3600)).await;
    pesan_slot(&l.store, &f, id, 1).await;

    let h = l.handler(l.store.clone()).proses(id).await.expect("proses");
    assert!(matches!(h, Hasil::Retry(_)), "{h:?}");
    let log = log_request(&l.su, id).await;
    assert_eq!(log[0].status_akses.as_deref(), Some("gagal_jaringan"));
    assert_eq!(log[0].error_class.as_deref(), Some("timeout"));
    assert!(!log[0].responded);
    assert_eq!(log[0].response_payload, None);
}

/// AC-18 (U-26): batas percobaan pool tercapai, atau `not_before` ≥ TTL →
/// langsung `failed`/`retry_habis`.
#[tokio::test]
#[ignore = "butuh PostgreSQL + Redis: make -C backend uji-pg"]
async fn retry_habis_batas_percobaan_dan_ttl() {
    let l = lingkungan().await;
    l.pasang(503, json!({}), 1, None).await;

    // Batas 3: dua percobaan sebelumnya + percobaan ini.
    let f = Fixture::baru(&l, 5).await;
    let id = f.request(2, StdDuration::from_secs(3600)).await;
    pesan_slot(&l.store, &f, id, 3).await;
    let h = l.handler(l.store.clone()).proses(id).await.expect("proses");
    assert_eq!(h, Hasil::Gagal("retry_habis"));
    let r = baris_request(&l.su, id).await;
    assert_eq!(r.status, "failed");
    assert_eq!(r.attempt_count, 3);
    assert_eq!(r.failure_reason.as_deref(), Some("retry_habis"));
    assert_eq!(r.webhook_status.as_deref(), Some("tidak_dikirim"));
    let log = log_request(&l.su, id).await;
    assert_eq!(log.len(), 3);
    assert_eq!(r.final_kode, Some(log[2].kode));

    // TTL tinggal 2 s, backoff 5 s → not_before melewati TTL.
    let f = Fixture::baru(&l, 5).await;
    let id = f.request(0, StdDuration::from_secs(2)).await;
    pesan_slot(&l.store, &f, id, 1).await;
    let h = l.handler(l.store.clone()).proses(id).await.expect("proses");
    assert_eq!(h, Hasil::Gagal("retry_habis"));
    assert_eq!(baris_request(&l.su, id).await.not_before, None);
}

/// AC-16: kelas yang tidak di-retry → `failed`/`provider_error_permanen`;
/// `webhook_status` `menunggu` bila `client_webhook.aktif`.
#[tokio::test]
#[ignore = "butuh PostgreSQL + Redis: make -C backend uji-pg"]
async fn kelas_tidak_diretry_failed_permanen() {
    let l = lingkungan().await;
    l.pasang(401, json!({"error": "auth"}), 1, Some(1)).await;
    l.pasang(200, json!({"content": []}), 2, None).await;

    let f = Fixture::baru(&l, 5).await;
    f.webhook_aktif().await;
    let id = f.request(0, StdDuration::from_secs(3600)).await;
    pesan_slot(&l.store, &f, id, 1).await;
    let h = l.handler(l.store.clone()).proses(id).await.expect("proses");
    assert_eq!(h, Hasil::Gagal("provider_error_permanen"));
    let r = baris_request(&l.su, id).await;
    assert_eq!(r.webhook_status.as_deref(), Some("menunggu"));
    let log = log_request(&l.su, id).await;
    assert_eq!(log[0].error_class.as_deref(), Some("auth_ditolak"));

    let id = f.request(0, StdDuration::from_secs(3600)).await;
    pesan_slot(&l.store, &f, id, 1).await;
    let h = l.handler(l.store.clone()).proses(id).await.expect("proses");
    assert_eq!(h, Hasil::Gagal("provider_error_permanen"));
    let log = log_request(&l.su, id).await;
    assert_eq!(log[0].error_class.as_deref(), Some("respons_tidak_valid"));
    assert_eq!(baris_request(&l.su, id).await.output, None);
}

/// `KuotaStore` sungguhan dengan `attempts_tambah` yang melambat melewati
/// 250 ms; seluruh skrip lain tetap ke Redis (AC-15).
struct Lambat<K>(K);

impl<K: KuotaStore> KuotaStore for Lambat<K> {
    async fn intake(
        &self,
        ctx: &KonteksKuota,
        sync: bool,
        client_id: &str,
    ) -> Result<HasilIntake, GalatStore> {
        self.0.intake(ctx, sync, client_id).await
    }

    async fn intake_alokasi(
        &self,
        ctx: &KonteksKuota,
        sync: bool,
        client_id: &str,
        alokasi: &IntakeAlokasi,
    ) -> Result<HasilIntake, GalatStore> {
        self.0.intake_alokasi(ctx, sync, client_id, alokasi).await
    }

    async fn ambil_token(
        &self,
        ctx: &KonteksKuota,
        permintaan: &PermintaanToken,
    ) -> Result<HasilToken, GalatStore> {
        self.0.ambil_token(ctx, permintaan).await
    }

    async fn ambil_token_alokasi(
        &self,
        ctx: &KonteksKuota,
        permintaan: &PermintaanToken,
        urutan: &[SumberDana],
    ) -> Result<HasilToken, GalatStore> {
        self.0.ambil_token_alokasi(ctx, permintaan, urutan).await
    }

    async fn klaim_slot(
        &self,
        pool_id: &str,
        request_id: &str,
        nomor_percobaan: i64,
        ttl_slot_mikro: i64,
        dari_antre: bool,
    ) -> Result<HasilKlaim, GalatStore> {
        self.0
            .klaim_slot(
                pool_id,
                request_id,
                nomor_percobaan,
                ttl_slot_mikro,
                dari_antre,
            )
            .await
    }

    async fn tambah_attempts(
        &self,
        ctx: &KonteksKuota,
        request_id: &str,
        nomor_percobaan: i64,
    ) -> Result<HasilTambah, GalatStore> {
        tokio::time::sleep(StdDuration::from_millis(600)).await;
        self.0
            .tambah_attempts(ctx, request_id, nomor_percobaan)
            .await
    }

    async fn tambah_attempts_alokasi(
        &self,
        ctx: &KonteksKuota,
        request_id: &str,
        nomor_percobaan: i64,
    ) -> Result<HasilTambah, GalatStore> {
        self.0
            .tambah_attempts_alokasi(ctx, request_id, nomor_percobaan)
            .await
    }

    async fn lepas_slot(
        &self,
        pool_id: &str,
        request_id: &str,
        nomor_percobaan: i64,
        asal: AsalLepas,
    ) -> Result<bool, GalatStore> {
        self.0
            .lepas_slot(pool_id, request_id, nomor_percobaan, asal)
            .await
    }

    async fn baca(&self, ctx: &KonteksKuota) -> Result<Bacaan, GalatStore> {
        self.0.baca(ctx).await
    }

    async fn ambil_token_penolakan(
        &self,
        client_id: &str,
        isi_ulang_mikro: i64,
        burst: i64,
    ) -> Result<HasilPenolakan, GalatStore> {
        self.0
            .ambil_token_penolakan(client_id, isi_ulang_mikro, burst)
            .await
    }
}

/// AC-15: INCR `attempts` melewati 250 ms setelah log commit → upstream tidak
/// dipanggil, pesan di-ack, baris log dibiarkan untuk reconciler.
#[tokio::test]
#[ignore = "butuh PostgreSQL + Redis: make -C backend uji-pg"]
async fn redis_lambat_setelah_log_tanpa_panggilan() {
    let l = lingkungan().await;
    let f = Fixture::baru(&l, 5).await;
    l.pasang(200, respons_sukses(), 1, None).await;
    let id = f.request(0, StdDuration::from_secs(3600)).await;
    pesan_slot(&l.store, &f, id, 1).await;

    let h = l
        .handler(Lambat(l.store.clone()))
        .proses(id)
        .await
        .expect("ack, bukan error");
    assert_eq!(h, Hasil::TanpaPanggilan);
    assert_eq!(l.panggilan().await.len(), 0);
    let r = baris_request(&l.su, id).await;
    assert_eq!(r.status, "in_flight");
    assert_eq!(r.attempt_count, 1);
    let log = log_request(&l.su, id).await;
    assert_eq!(log.len(), 1);
    assert_eq!(log[0].status_akses, None, "log dibiarkan untuk reconciler");
}

/// AC-12 (S4): pesan duplikat yang diproses bersamaan. Penengahnya CAS
/// PostgreSQL: tepat satu `Done`, sisanya ack tanpa panggilan dan tanpa log.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "butuh PostgreSQL + Redis: make -C backend uji-pg"]
async fn duplikat_bersamaan_tepat_satu_panggilan() {
    let l = lingkungan().await;
    let f = Fixture::baru(&l, 5).await;
    l.pasang(200, respons_sukses(), 1, None).await;
    let id = f.request(0, StdDuration::from_secs(3600)).await;
    pesan_slot(&l.store, &f, id, 1).await;

    let handler = Arc::new(l.handler(l.store.clone()));
    let mut kerja = tokio::task::JoinSet::new();
    for _ in 0..50 {
        let h = Arc::clone(&handler);
        kerja.spawn(async move { h.proses(id).await });
    }
    let mut done = 0;
    while let Some(hasil) = kerja.join_next().await {
        let hasil = hasil.expect("task").expect("ack, bukan error");
        match hasil {
            Hasil::Done => done += 1,
            Hasil::BukanDispatched | Hasil::KalahCas | Hasil::SlotTidakTersedia => {}
            lain => panic!("hasil tak terduga: {lain:?}"),
        }
    }
    assert_eq!(done, 1);
    assert_eq!(l.panggilan().await.len(), 1);
    assert_eq!(log_request(&l.su, id).await.len(), 1);
}

/// Penulis log bersama untuk uji tangkap `tracing`.
#[derive(Clone, Default)]
struct Tangkap(Arc<std::sync::Mutex<Vec<u8>>>);

impl std::io::Write for Tangkap {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("kunci buffer").extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Tangkap {
    fn teks(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().expect("kunci buffer")).into_owned()
    }
}

/// AC-16 (R1 AC-4, AC-23): secret provider dan NIK tidak muncul di log
/// aplikasi pada jalur sukses, kelas yang tidak di-retry, dan secret hilang.
/// Secret hilang juga ditolak sebelum efek samping: slot dan status utuh.
#[tokio::test]
#[ignore = "butuh PostgreSQL + Redis: make -C backend uji-pg"]
async fn log_aplikasi_tanpa_secret_dan_nik() {
    let buffer = Tangkap::default();
    let penulis = buffer.clone();
    let subscriber = tracing_subscriber::fmt()
        // Log aplikasi worker; log pustaka (sqlx, hyper, wiremock) di luar cakupan uji ini.
        .with_env_filter(tracing_subscriber::EnvFilter::new(
            "poolrouter_worker=debug",
        ))
        .with_ansi(false)
        .with_writer(move || penulis.clone())
        .finish();
    let _jaga = tracing::subscriber::set_default(subscriber);

    let l = lingkungan().await;
    l.pasang(401, json!({"error": SECRET}), 1, Some(1)).await;
    l.pasang(200, respons_sukses(), 2, None).await;
    let f = Fixture::baru(&l, 5).await;
    let handler = l.handler(l.store.clone());

    let id = f.request(0, StdDuration::from_secs(3600)).await;
    pesan_slot(&l.store, &f, id, 1).await;
    assert_eq!(
        handler.proses(id).await.expect("proses"),
        Hasil::Gagal("provider_error_permanen")
    );
    let id = f.request(0, StdDuration::from_secs(3600)).await;
    pesan_slot(&l.store, &f, id, 1).await;
    assert_eq!(handler.proses(id).await.expect("proses"), Hasil::Done);

    // Secret dihapus: pesan ditolak sebelum klaim slot dan sebelum CAS.
    sqlx::query(
        "DELETE FROM provider_secret WHERE nama_secret IN
           (SELECT substring(dokumen->>'body_template' FROM 'secret\\.([A-Za-z0-9_]+)')
              FROM provider_config_versi WHERE provider_id = $1)",
    )
    .bind(f.provider_id)
    .execute(&l.su)
    .await
    .expect("hapus secret");
    let id = f.request(0, StdDuration::from_secs(3600)).await;
    pesan_slot(&l.store, &f, id, 1).await;
    let galat = handler.proses(id).await.expect_err("secret hilang ditolak");
    assert!(!galat.to_string().contains(SECRET));
    assert_eq!(baris_request(&l.su, id).await.status, "dispatched");
    assert!(slot_ada(&l.redis, &f, id, 1).await);
    assert_eq!(log_request(&l.su, id).await.len(), 0);

    let log = buffer.teks();
    assert!(log.contains("worker:"), "log worker tertangkap");
    assert!(!log.contains(SECRET), "secret muncul di log aplikasi");
    assert!(!log.contains(NIK), "NIK muncul di log aplikasi");
}
