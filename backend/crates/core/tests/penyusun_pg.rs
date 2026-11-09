//! T-24a..T-28f: uji seam penyusun batch data balikan (AC-24–AC-28, ARC-181/T5b, ARC-211).
//!
//! Dijalankan oleh `make -C backend uji-pg`. Fixture ditulis sebagai superuser
//! (`POOLROUTER_TEST_DATABASE_URL_SU`); handler berjalan sebagai `pr_job_balikan`
//! (`POOLROUTER_TEST_DATABASE_URL_JOB_BALIKAN`), role yang dipakai produksi, jadi grant ikut teruji.
//! Setiap uji membuat provider sendiri dengan nama unik, sehingga data uji lain tidak ikut terbaca.
//! NIK di fixture sintetis (berawalan 00).

use chrono::{DateTime, Duration, SubsecRound, TimeZone, Utc};
use poolrouter_core::data_balikan::{HasilPenyusun, jalankan_job_balikan};
use serde_json::json;
use sqlx::{Connection, PgConnection, PgPool};
use uuid::Uuid;

/// NIK sintetis, bukan data pribadi.
const NIK_UJI: &str = "0012345678901234";
const URL_JOB_BALIKAN: &str = "POOLROUTER_TEST_DATABASE_URL_JOB_BALIKAN";
const URL_SU: &str = "POOLROUTER_TEST_DATABASE_URL_SU";

fn url(nama: &str) -> String {
    std::env::var(nama).unwrap_or_else(|_| panic!("{nama} wajib di-set (make -C backend uji-pg)"))
}

async fn pool(nama: &str, maks: u32) -> PgPool {
    sqlx::postgres::PgPoolOptions::new()
        .max_connections(maks)
        .connect(&url(nama))
        .await
        .expect("koneksi PostgreSQL uji")
}

async fn pool_worker() -> PgPool {
    pool(URL_JOB_BALIKAN, 10).await
}

async fn pool_su() -> PgPool {
    pool(URL_SU, 4).await
}

/// Waktu uji dibulatkan ke detik: timestamptz PostgreSQL hanya menyimpan mikrodetik.
fn sekarang() -> DateTime<Utc> {
    Utc::now().trunc_subsecs(0)
}

fn kunci(provider_id: Uuid) -> String {
    format!("poolrouter.balikan.{provider_id}")
}

/// Blok `data_balikan` minimal yang lolos `KonfigurasiDataBalikan::parse` (skema 2).
fn blok(pemicu_jumlah_n: u32, ukuran_batch_maks: u32) -> serde_json::Value {
    json!({
        "endpoint": "https://balikan.contoh.test/terima",
        "rujukan_secret": "{{secret.BALIKAN_UJI}}",
        "pemicu_jumlah_n": pemicu_jumlah_n,
        "ukuran_batch_maks": ukuran_batch_maks,
        "format_version": "v1",
    })
}

/// Pemegang kunci sesi di luar pool (penyusun lain atau pengirim, AC-24). Koneksi sendiri, bukan dari pool,
/// supaya kunci tidak ikut kembali ke pool dan tertinggal untuk panggilan berikutnya.
async fn pegang_kunci_sesi(provider_id: Uuid) -> PgConnection {
    let mut conn = PgConnection::connect(&url(URL_JOB_BALIKAN))
        .await
        .expect("koneksi pemegang kunci");
    sqlx::query("SELECT pg_advisory_lock(hashtextextended($1, 0))")
        .bind(kunci(provider_id))
        .execute(&mut conn)
        .await
        .expect("pegang kunci sesi");
    conn
}

/// Lepas kunci secara eksplisit lalu tutup koneksi.
async fn lepas_kunci_sesi(mut conn: PgConnection, provider_id: Uuid) {
    let lepas: bool = sqlx::query_scalar("SELECT pg_advisory_unlock(hashtextextended($1, 0))")
        .bind(kunci(provider_id))
        .fetch_one(&mut conn)
        .await
        .expect("lepas kunci sesi");
    assert!(lepas, "kunci sesi harus dipegang koneksi ini");
    conn.close().await.expect("tutup koneksi pemegang kunci");
}

/// Satu rangkaian request untuk [`Fixture::request`].
struct Rangkaian {
    jumlah: usize,
    /// `completed_at` request pertama; request berikutnya +1 detik.
    selesai_mulai: DateTime<Utc>,
    /// `done` atau `failed`.
    status_akhir: &'static str,
    sumber_hasil: Option<&'static str>,
    input_payload: Option<serde_json::Value>,
    metadata: serde_json::Value,
    nik: Option<&'static str>,
    /// Jumlah attempt; attempt terakhir menjadi `final_kode_transaksi`.
    attempt: i32,
}

impl Rangkaian {
    /// Request yang memenuhi syarat Q4: done, sumber provider, input dan NIK ada.
    fn eligible(jumlah: usize, selesai_mulai: DateTime<Utc>) -> Self {
        Self {
            jumlah,
            selesai_mulai,
            status_akhir: "done",
            sumber_hasil: Some("provider"),
            input_payload: Some(json!({"nama_lengkap": "UJI SINTETIS"})),
            metadata: json!({}),
            nik: Some(NIK_UJI),
            attempt: 1,
        }
    }
}

/// Request yang dibuat fixture, dengan attempt final-nya.
struct RequestUji {
    request_id: Uuid,
    kode_final: Uuid,
    requested_at_final: DateTime<Utc>,
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
    /// Provider dengan konfigurasi skema 2 yang sah.
    async fn standar(su: &PgPool, pemicu_jumlah_n: u32, ukuran_batch_maks: u32) -> Self {
        let dokumen = json!({"data_balikan": blok(pemicu_jumlah_n, ukuran_batch_maks)});
        Self::baru(su, 2, dokumen).await
    }

    /// Objek konfigurasi lengkap untuk satu provider. `wajib_log_nik = false` supaya uji NIK kosong
    /// (Q5) bisa menulis `transaction_log` tanpa NIK.
    async fn baru(su: &PgPool, config_schema_version: i32, dokumen: serde_json::Value) -> Self {
        let f = Self {
            su: su.clone(),
            admin_id: Uuid::now_v7(),
            client_id: Uuid::now_v7(),
            kredensial_id: Uuid::now_v7(),
            provider_id: Uuid::now_v7(),
            downstream_pool_id: Uuid::now_v7(),
            upstream_pool_id: Uuid::now_v7(),
            endpoint_id: Uuid::now_v7(),
        };
        let nama = format!("arc211-{}", f.provider_id.simple());
        // Dokumen harus tetap terbaca oleh admin API (`DokumenProvider`): uji lain di database yang sama
        // (admin_provider_pg) membaca semua provider lewat GET /admin/v1/providers dan menjawab 503 bila
        // satu dokumen tidak lengkap. Field dasar ditambahkan bila belum ada.
        let mut dokumen = dokumen;
        let obj = dokumen.as_object_mut().expect("dokumen harus objek");
        for (kunci_field, nilai) in [
            ("metode", json!("POST")),
            ("base_url", json!("https://provider.contoh.test")),
            ("body_template", json!("{}")),
            ("parameters", json!([])),
            ("respons_skema", json!({})),
            ("tls_trust", json!("system")),
            ("log_mode", json!("metadata_only")),
        ] {
            obj.entry(kunci_field).or_insert(nilai);
        }

        sqlx::query(
            "INSERT INTO admin_user (pengguna_id, username, password_hash, bahasa_ui)
             VALUES ($1, $2, 'x', 'id')",
        )
        .bind(f.admin_id)
        .bind(&nama)
        .execute(su)
        .await
        .expect("admin_user");
        sqlx::query("INSERT INTO client (client_id, nama) VALUES ($1, $2)")
            .bind(f.client_id)
            .bind(&nama)
            .execute(su)
            .await
            .expect("client");
        sqlx::query(
            "INSERT INTO client_kredensial (credential_id, client_id, jenis, kunci_hash, dibuat_oleh)
             VALUES ($1, $2, 'api_key', $3, $4)",
        )
        .bind(f.kredensial_id)
        .bind(f.client_id)
        .bind(vec![0u8; 32])
        .bind(f.admin_id)
        .execute(su)
        .await
        .expect("client_kredensial");
        sqlx::query(
            "INSERT INTO provider (provider_id, nama, wajib_log_nik, data_balikan_aktif, config_versi_aktif)
             VALUES ($1, $2, false, true, 1)",
        )
        .bind(f.provider_id)
        .bind(&nama)
        .execute(su)
        .await
        .expect("provider");
        sqlx::query(
            "INSERT INTO provider_config_versi
                    (provider_id, versi, config_schema_version, log_mode, dokumen, dibuat_oleh)
             VALUES ($1, 1, $2, 'metadata_only', $3, $4)",
        )
        .bind(f.provider_id)
        .bind(config_schema_version)
        .bind(dokumen)
        .bind(f.admin_id)
        .execute(su)
        .await
        .expect("provider_config_versi");
        sqlx::query(
            "INSERT INTO downstream_pool (downstream_pool_id, nama, prioritas) VALUES ($1, $2, 1)",
        )
        .bind(f.downstream_pool_id)
        .bind(&nama)
        .execute(su)
        .await
        .expect("downstream_pool");
        sqlx::query(
            "INSERT INTO upstream_pool (upstream_pool_id, provider_id, nama, kuota_harian,
                     reset_kuota_jam, reset_kuota_zona)
             VALUES ($1, $2, $3, 1000, '00:00', 'Asia/Jakarta')",
        )
        .bind(f.upstream_pool_id)
        .bind(f.provider_id)
        .bind(&nama)
        .execute(su)
        .await
        .expect("upstream_pool");
        sqlx::query(
            "INSERT INTO downstream_endpoint (downstream_endpoint_id, nama, provider_id, downstream_pool_id)
             VALUES ($1, $2, $3, $4)",
        )
        .bind(f.endpoint_id)
        .bind(&nama)
        .bind(f.provider_id)
        .bind(f.downstream_pool_id)
        .execute(su)
        .await
        .expect("downstream_endpoint");
        f
    }

    /// Request terminal lewat jalur status sah (`queued` → `dispatched` → `in_flight` → akhir), masing-masing
    /// dengan baris `request_status_history` di transaksi yang sama (T-04). Set-based supaya 10.001 request
    /// (T-25b) tetap cepat. `requested_at` attempt final = `completed_at` − 2 detik; attempt sebelumnya
    /// mundur satu jam per langkah.
    async fn request(&self, r: &Rangkaian) -> Vec<RequestUji> {
        let mut hasil = Vec::with_capacity(r.jumlah);
        let (mut log_kode, mut log_request, mut log_attempt, mut log_waktu) =
            (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        let mut selesai = Vec::with_capacity(r.jumlah);
        for i in 0..r.jumlah {
            let request_id = Uuid::now_v7();
            let completed_at = r.selesai_mulai + Duration::seconds(i as i64);
            let requested_at_final = completed_at - Duration::seconds(2);
            let mut kode_final = Uuid::nil();
            for a in 1..=r.attempt {
                let kode = Uuid::now_v7();
                log_kode.push(kode);
                log_request.push(request_id);
                log_attempt.push(a);
                log_waktu.push(requested_at_final - Duration::hours(i64::from(r.attempt - a)));
                kode_final = kode;
            }
            selesai.push(completed_at);
            hasil.push(RequestUji {
                request_id,
                kode_final,
                requested_at_final,
            });
        }
        let ids: Vec<Uuid> = hasil.iter().map(|h| h.request_id).collect();
        let finals: Vec<Uuid> = hasil.iter().map(|h| h.kode_final).collect();

        let mut tx = self.su.begin().await.expect("tx fixture");
        sqlx::query(
            "INSERT INTO request
                    (request_id, client_id, credential_id, downstream_endpoint_id, downstream_pool_id,
                     provider_id, upstream_pool_id, provider_config_version, endpoint_config_version, mode,
                     prioritas, prioritas_efektif, status, input_payload, metadata, received_at,
                     ttl_expires_at, attempt_count)
             SELECT u.rid, $2, $3, $4, $5, $6, $7, 1, 1, 'async', 1, 1, 'queued', $8, $9, now(),
                    now() + interval '1 hour', $10
               FROM unnest($1::uuid[]) AS u(rid)",
        )
        .bind(&ids)
        .bind(self.client_id)
        .bind(self.kredensial_id)
        .bind(self.endpoint_id)
        .bind(self.downstream_pool_id)
        .bind(self.provider_id)
        .bind(self.upstream_pool_id)
        .bind(r.input_payload.clone())
        .bind(r.metadata.clone())
        .bind(r.attempt)
        .execute(&mut *tx)
        .await
        .expect("request queued");
        sqlx::query(
            "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor)
             SELECT u.rid, 1, NULL, 'queued', 'webserver' FROM unnest($1::uuid[]) AS u(rid)",
        )
        .bind(&ids)
        .execute(&mut *tx)
        .await
        .expect("history queued");

        for (seq, dari, ke, aktor) in [
            (2, "queued", "dispatched", "controller"),
            (3, "dispatched", "in_flight", "worker"),
        ] {
            sqlx::query("UPDATE request SET status = $2 WHERE request_id = ANY ($1)")
                .bind(&ids)
                .bind(ke)
                .execute(&mut *tx)
                .await
                .expect("transisi request");
            sqlx::query(
                "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor)
                 SELECT u.rid, $2, $3, $4, $5 FROM unnest($1::uuid[]) AS u(rid)",
            )
            .bind(&ids)
            .bind(seq)
            .bind(dari)
            .bind(ke)
            .bind(aktor)
            .execute(&mut *tx)
            .await
            .expect("history transisi");
        }

        sqlx::query(
            "INSERT INTO transaction_log
                    (kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id, client_id,
                     downstream_endpoint_id, provider_config_version, endpoint_config_version, nik,
                     requested_at, responded_at, duration_ms, status_akses, log_mode, worker_id)
             SELECT u.kode, u.rid, u.attempt, $5, $6, $7, $8, 1, 1, $9, u.waktu, u.waktu, 5, 'berhasil',
                    'metadata_only', 'arc211-worker'
               FROM unnest($1::uuid[], $2::uuid[], $3::int4[], $4::timestamptz[]) AS u(kode, rid, attempt, waktu)",
        )
        .bind(&log_kode)
        .bind(&log_request)
        .bind(&log_attempt)
        .bind(&log_waktu)
        .bind(self.provider_id)
        .bind(self.upstream_pool_id)
        .bind(self.client_id)
        .bind(self.endpoint_id)
        .bind(r.nik)
        .execute(&mut *tx)
        .await
        .expect("transaction_log");

        sqlx::query(
            "UPDATE request r
                SET status = $4, completed_at = u.selesai, final_kode_transaksi = u.kode, sumber_hasil = $5,
                    output_payload = CASE WHEN $4 = 'done' THEN '{\"data\": {}}'::jsonb END,
                    failure_reason = CASE WHEN $4 = 'failed' THEN 'retry_habis' END,
                    webhook_status = 'menunggu'
               FROM unnest($1::uuid[], $2::uuid[], $3::timestamptz[]) AS u(rid, kode, selesai)
              WHERE r.request_id = u.rid",
        )
        .bind(&ids)
        .bind(&finals)
        .bind(&selesai)
        .bind(r.status_akhir)
        .bind(r.sumber_hasil)
        .execute(&mut *tx)
        .await
        .expect("request terminal");
        sqlx::query(
            "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor)
             SELECT u.rid, 4, 'in_flight', $2, 'worker' FROM unnest($1::uuid[]) AS u(rid)",
        )
        .bind(&ids)
        .bind(r.status_akhir)
        .execute(&mut *tx)
        .await
        .expect("history terminal");
        tx.commit().await.expect("commit fixture");
        hasil
    }

    async fn jumlah_batch(&self) -> i64 {
        sqlx::query_scalar("SELECT count(*) FROM data_balikan_batch WHERE provider_id = $1")
            .bind(self.provider_id)
            .fetch_one(&self.su)
            .await
            .expect("hitung batch")
    }

    async fn tersusun(&self, request_id: Uuid) -> bool {
        sqlx::query_scalar("SELECT data_balikan_tersusun FROM request WHERE request_id = $1")
            .bind(request_id)
            .fetch_one(&self.su)
            .await
            .expect("baca penanda")
    }

    async fn item_batch(&self, batch_id: Uuid) -> Vec<Uuid> {
        sqlx::query_scalar(
            "SELECT request_id FROM data_balikan_item WHERE batch_id = $1 ORDER BY request_id",
        )
        .bind(batch_id)
        .fetch_all(&self.su)
        .await
        .expect("item batch")
    }
}

fn disusun(hasil: HasilPenyusun) -> (Uuid, u32) {
    match hasil {
        HasilPenyusun::Disusun {
            batch_id,
            jumlah_item,
        } => (batch_id, jumlah_item),
        lain => panic!("diharapkan Disusun, dapat {lain:?}"),
    }
}

// T-24a: kunci sesi dipegang pihak lain → TidakDapatKunci tanpa menulis apa pun; setelah dilepas → Disusun.
#[tokio::test]
#[ignore]
async fn t24a_kunci_sesi_memblok_penyusun() {
    let su = pool_su().await;
    let w = pool_worker().await;
    let f = Fixture::standar(&su, 1, 100).await;
    let r = f
        .request(&Rangkaian::eligible(1, sekarang() - Duration::minutes(5)))
        .await;

    let a = pegang_kunci_sesi(f.provider_id).await;
    let b = jalankan_job_balikan(&w, f.provider_id, sekarang())
        .await
        .expect("B");
    assert_eq!(b, HasilPenyusun::TidakDapatKunci);
    assert_eq!(
        f.jumlah_batch().await,
        0,
        "tanpa kunci tidak ada yang ditulis"
    );
    assert!(!f.tersusun(r[0].request_id).await);
    lepas_kunci_sesi(a, f.provider_id).await;

    let b2 = jalankan_job_balikan(&w, f.provider_id, sekarang())
        .await
        .expect("B2");
    let (_, jumlah) = disusun(b2);
    assert_eq!(jumlah, 1);
}

// T-24b: 50 panggilan serentak → tepat satu Disusun, sisanya TidakDapatKunci atau BatchAktif.
#[tokio::test]
#[ignore]
async fn t24b_lima_puluh_serentak_satu_batch() {
    let su = pool_su().await;
    let w = pool_worker().await;
    let f = Fixture::standar(&su, 5, 100).await;
    f.request(&Rangkaian::eligible(5, sekarang() - Duration::minutes(5)))
        .await;

    let t = sekarang();
    let hasil =
        futures::future::join_all((0..50).map(|_| jalankan_job_balikan(&w, f.provider_id, t)))
            .await;

    let (mut jumlah_disusun, mut jumlah_lain) = (0, 0);
    for h in hasil {
        match h.expect("tanpa galat") {
            HasilPenyusun::Disusun { jumlah_item, .. } => {
                assert_eq!(jumlah_item, 5);
                jumlah_disusun += 1;
            }
            HasilPenyusun::TidakDapatKunci | HasilPenyusun::BatchAktif => jumlah_lain += 1,
            lain => panic!("hasil tak terduga: {lain:?}"),
        }
    }
    assert_eq!(jumlah_disusun, 1);
    assert_eq!(jumlah_lain, 49);
    assert_eq!(f.jumlah_batch().await, 1);
}

// T-24c: kunci sesi pengirim diperiksa sebelum apa pun dibaca: TidakDapatKunci walau ada batch aktif;
// setelah pengirim melepas kunci, batch aktif baru terlihat (BatchAktif).
#[tokio::test]
#[ignore]
async fn t24c_kunci_sesi_pengirim() {
    let su = pool_su().await;
    let w = pool_worker().await;
    let f = Fixture::standar(&su, 1, 100).await;
    f.request(&Rangkaian::eligible(1, sekarang() - Duration::minutes(5)))
        .await;
    let pertama = jalankan_job_balikan(&w, f.provider_id, sekarang())
        .await
        .expect("pertama");
    disusun(pertama);

    let pengirim = pegang_kunci_sesi(f.provider_id).await;
    let saat_kirim = jalankan_job_balikan(&w, f.provider_id, sekarang())
        .await
        .expect("saat kirim");
    assert_eq!(saat_kirim, HasilPenyusun::TidakDapatKunci);
    lepas_kunci_sesi(pengirim, f.provider_id).await;

    let sesudah = jalankan_job_balikan(&w, f.provider_id, sekarang())
        .await
        .expect("sesudah");
    assert_eq!(sesudah, HasilPenyusun::BatchAktif);
}

// T-25a: hanya done + sumber provider + belum tersusun milik provider ini; penanda disetel di transaksi Q8.
#[tokio::test]
#[ignore]
async fn t25a_filter_eligible_dan_penanda() {
    let su = pool_su().await;
    let w = pool_worker().await;
    let f = Fixture::standar(&su, 1, 100).await;
    let lain = Fixture::standar(&su, 1, 100).await;
    let mulai = sekarang() - Duration::minutes(10);

    let eligible = f.request(&Rangkaian::eligible(1, mulai)).await;
    let gagal = f
        .request(&Rangkaian {
            status_akhir: "failed",
            sumber_hasil: None,
            ..Rangkaian::eligible(1, mulai)
        })
        .await;
    let cache = f
        .request(&Rangkaian {
            sumber_hasil: Some("cache"),
            ..Rangkaian::eligible(1, mulai)
        })
        .await;
    let provider_lain = lain.request(&Rangkaian::eligible(1, mulai)).await;

    let hasil = jalankan_job_balikan(&w, f.provider_id, sekarang())
        .await
        .expect("handler");
    let (batch_id, jumlah) = disusun(hasil);
    assert_eq!(jumlah, 1);
    assert_eq!(f.item_batch(batch_id).await, vec![eligible[0].request_id]);

    assert!(f.tersusun(eligible[0].request_id).await, "penanda ARC-199");
    assert!(!f.tersusun(gagal[0].request_id).await);
    assert!(!f.tersusun(cache[0].request_id).await);
    assert!(!f.tersusun(provider_lain[0].request_id).await);
    assert_eq!(lain.jumlah_batch().await, 0);
}

// T-25b: 10.001 eligible, ukuran_batch_maks 10.000 → 10.000 terlama; yang terbaru tetap tertunda.
#[tokio::test]
#[ignore]
async fn t25b_batas_ukuran_batch() {
    let su = pool_su().await;
    let w = pool_worker().await;
    let f = Fixture::standar(&su, 10_000, 10_000).await;
    let r = f
        .request(&Rangkaian::eligible(10_001, sekarang() - Duration::days(1)))
        .await;

    let hasil = jalankan_job_balikan(&w, f.provider_id, sekarang())
        .await
        .expect("handler");
    let (batch_id, jumlah) = disusun(hasil);
    assert_eq!(jumlah, 10_000);
    assert_eq!(f.item_batch(batch_id).await.len(), 10_000);

    let terbaru = r.last().expect("ada request").request_id;
    assert!(
        !f.tersusun(terbaru).await,
        "request ke-10.001 tidak masuk batch"
    );
    let tersusun: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM request WHERE provider_id = $1 AND data_balikan_tersusun",
    )
    .bind(f.provider_id)
    .fetch_one(&su)
    .await
    .expect("hitung penanda");
    assert_eq!(tersusun, 10_000);
}

// T-25c + T-28c: request multi-attempt → tepat satu item, kode_transaksi dan requested_at dari attempt final.
#[tokio::test]
#[ignore]
async fn t25c_t28c_multi_attempt_pakai_attempt_final() {
    let su = pool_su().await;
    let w = pool_worker().await;
    let f = Fixture::standar(&su, 1, 100).await;
    let r = f
        .request(&Rangkaian {
            attempt: 3,
            ..Rangkaian::eligible(1, sekarang() - Duration::minutes(5))
        })
        .await;

    let hasil = jalankan_job_balikan(&w, f.provider_id, sekarang())
        .await
        .expect("handler");
    let (batch_id, jumlah) = disusun(hasil);
    assert_eq!(jumlah, 1);

    let (kode, requested_at): (Uuid, DateTime<Utc>) = sqlx::query_as(
        "SELECT kode_transaksi, requested_at FROM data_balikan_item WHERE batch_id = $1 AND request_id = $2",
    )
    .bind(batch_id)
    .bind(r[0].request_id)
    .fetch_one(&su)
    .await
    .expect("item");
    assert_eq!(kode, r[0].kode_final);
    assert_eq!(requested_at, r[0].requested_at_final);
}

// T-26: ada batch aktif → tidak menyusun; request baru tetap tertunda.
#[tokio::test]
#[ignore]
async fn t26_batch_aktif() {
    let su = pool_su().await;
    let w = pool_worker().await;
    let f = Fixture::standar(&su, 1, 100).await;
    f.request(&Rangkaian::eligible(1, sekarang() - Duration::minutes(5)))
        .await;
    let pertama = jalankan_job_balikan(&w, f.provider_id, sekarang())
        .await
        .expect("pertama");
    disusun(pertama);

    let baru = f
        .request(&Rangkaian::eligible(1, sekarang() - Duration::minutes(1)))
        .await;
    let kedua = jalankan_job_balikan(&w, f.provider_id, sekarang())
        .await
        .expect("kedua");
    assert_eq!(kedua, HasilPenyusun::BatchAktif);
    assert_eq!(f.jumlah_batch().await, 1);
    assert!(!f.tersusun(baru[0].request_id).await);
}

// T-27: pemicu periode dengan waktu tetap (tidak bergantung jam uji). Jam kirim default 00:00 Asia/Jakarta.
#[tokio::test]
#[ignore]
async fn t27_pemicu_periode() {
    let su = pool_su().await;
    let w = pool_worker().await;
    let f = Fixture::standar(&su, 5, 100).await;
    // 00:00 WIB 9 November 2026.
    let batas = Utc
        .with_ymd_and_hms(2026, 11, 8, 17, 0, 0)
        .single()
        .expect("waktu sah");
    f.request(&Rangkaian::eligible(1, batas + Duration::minutes(1)))
        .await;

    // 23:59 WIB 9 November: batas terakhir 00:00 WIB 9 November, item lebih baru dari batas → tidak memicu.
    let t1 = batas + Duration::hours(23) + Duration::minutes(59);
    let hasil1 = jalankan_job_balikan(&w, f.provider_id, t1)
        .await
        .expect("t1");
    assert_eq!(hasil1, HasilPenyusun::TidakAdaPemicu);

    // 00:05 WIB 10 November: batas baru 00:00 WIB 10 November → batch periode, jadwal_kirim_at = batas.
    let t2 = batas + Duration::hours(24) + Duration::minutes(5);
    let hasil2 = jalankan_job_balikan(&w, f.provider_id, t2)
        .await
        .expect("t2");
    let (batch_id, jumlah) = disusun(hasil2);
    assert_eq!(jumlah, 1);
    let (pemicu, jadwal): (String, DateTime<Utc>) = sqlx::query_as(
        "SELECT pemicu, jadwal_kirim_at FROM data_balikan_batch WHERE batch_id = $1",
    )
    .bind(batch_id)
    .fetch_one(&su)
    .await
    .expect("batch");
    assert_eq!(pemicu, "periode");
    assert_eq!(jadwal, batas + Duration::hours(24));
}

// T-28a: field_metadata hanya field kirim_ke_data_balikan, bukan Rahasia, dan ada di metadata request.
#[tokio::test]
#[ignore]
async fn t28a_metadata_disaring() {
    let su = pool_su().await;
    let w = pool_worker().await;
    let f = Fixture::standar(&su, 1, 100).await;
    sqlx::query(
        "INSERT INTO endpoint_metadata_field
                (downstream_endpoint_id, nama_field, tipe, wajib, klasifikasi, kirim_ke_data_balikan,
                 deskripsi, versi)
         VALUES ($1, 'kode_wilayah', 'string', false, 'Internal', true, 'uji', 1),
                ($1, 'kode_pos', 'string', false, 'Internal', true, 'uji', 1),
                ($1, 'catatan_rahasia', 'string', false, 'Rahasia', true, 'uji', 1),
                ($1, 'referensi', 'string', false, 'Terbatas', false, 'uji', 1)",
    )
    .bind(f.endpoint_id)
    .execute(&su)
    .await
    .expect("endpoint_metadata_field");
    let r = f
        .request(&Rangkaian {
            metadata: json!({
                "kode_wilayah": "3201",
                "catatan_rahasia": "rahasia",
                "referensi": "ref-1",
            }),
            ..Rangkaian::eligible(1, sekarang() - Duration::minutes(5))
        })
        .await;

    let hasil = jalankan_job_balikan(&w, f.provider_id, sekarang())
        .await
        .expect("handler");
    let (batch_id, _) = disusun(hasil);
    let metadata: serde_json::Value = sqlx::query_scalar(
        "SELECT field_metadata FROM data_balikan_item WHERE batch_id = $1 AND request_id = $2",
    )
    .bind(batch_id)
    .bind(r[0].request_id)
    .fetch_one(&su)
    .await
    .expect("field_metadata");
    assert_eq!(metadata, json!({"kode_wilayah": "3201"}));
}

// T-28b: kolom batch terisi sesuai WB18.
#[tokio::test]
#[ignore]
async fn t28b_kolom_batch() {
    let su = pool_su().await;
    let w = pool_worker().await;
    let f = Fixture::standar(&su, 1, 100).await;
    let r = f
        .request(&Rangkaian::eligible(1, sekarang() - Duration::minutes(5)))
        .await;
    let t = sekarang();

    let hasil = jalankan_job_balikan(&w, f.provider_id, t)
        .await
        .expect("handler");
    let (batch_id, _) = disusun(hasil);
    #[allow(clippy::type_complexity)]
    let b: (
        String,
        String,
        i32,
        i32,
        String,
        i32,
        DateTime<Utc>,
        DateTime<Utc>,
        DateTime<Utc>,
        Option<DateTime<Utc>>,
    ) = sqlx::query_as(
        "SELECT pemicu, status, jumlah_item, attempt_count, format_version, config_ref_version,
                periode_mulai_at, periode_selesai_at, jadwal_kirim_at, disusun_at
           FROM data_balikan_batch WHERE batch_id = $1",
    )
    .bind(batch_id)
    .fetch_one(&su)
    .await
    .expect("batch");
    assert_eq!(b.0, "jumlah_n");
    assert_eq!(b.1, "disusun");
    assert_eq!(b.2, 1);
    assert_eq!(b.3, 0);
    assert_eq!(b.4, "v1");
    assert_eq!(b.5, 1);
    assert_eq!(b.6, r[0].requested_at_final);
    assert_eq!(b.7, r[0].requested_at_final);
    assert_eq!(b.8, t);
    assert_eq!(b.9, Some(t));
}

// T-28d: gauge tertunda diset saat tidak memicu (2 tertunda, ambang 5).
#[tokio::test]
#[ignore]
async fn t28d_gauge_tertunda() {
    let recorder = metrics_exporter_prometheus::PrometheusBuilder::new().build_recorder();
    let handle = recorder.handle();
    // Recorder lokal thread: `#[tokio::test]` berjalan di thread saat ini.
    let _penjaga = metrics::set_default_local_recorder(&recorder);

    let su = pool_su().await;
    let w = pool_worker().await;
    let f = Fixture::standar(&su, 5, 100).await;
    f.request(&Rangkaian::eligible(2, sekarang() - Duration::minutes(5)))
        .await;

    let hasil = jalankan_job_balikan(&w, f.provider_id, sekarang())
        .await
        .expect("handler");
    assert_eq!(hasil, HasilPenyusun::TidakAdaPemicu);

    let teks = handle.render();
    let nilai = teks
        .lines()
        .find_map(|l| l.strip_prefix("poolrouter_data_balikan_tertunda "))
        .unwrap_or_else(|| panic!("gauge tidak ada di keluaran:\n{teks}"));
    assert_eq!(nilai.trim().parse::<f64>().expect("angka"), 2.0);
}

// T-28e: request tanpa input_payload atau tanpa NIK dilewati (Q5), tidak masuk batch dan tetap tertunda.
#[tokio::test]
#[ignore]
async fn t28e_dilewati() {
    let su = pool_su().await;
    let w = pool_worker().await;
    let f = Fixture::standar(&su, 1, 100).await;
    let mulai = sekarang() - Duration::minutes(10);
    let eligible = f.request(&Rangkaian::eligible(1, mulai)).await;
    let tanpa_input = f
        .request(&Rangkaian {
            input_payload: None,
            ..Rangkaian::eligible(1, mulai)
        })
        .await;
    let tanpa_nik = f
        .request(&Rangkaian {
            nik: None,
            ..Rangkaian::eligible(1, mulai)
        })
        .await;

    let hasil = jalankan_job_balikan(&w, f.provider_id, sekarang())
        .await
        .expect("handler");
    let (batch_id, jumlah) = disusun(hasil);
    assert_eq!(jumlah, 1);
    assert_eq!(f.item_batch(batch_id).await, vec![eligible[0].request_id]);
    assert!(!f.tersusun(tanpa_input[0].request_id).await);
    assert!(!f.tersusun(tanpa_nik[0].request_id).await);
}

// T-28f: konfigurasi skema 1 tanpa blok → KonfigTidakAda; ukuran_batch_maks 0 → KonfigTidakValid.
#[tokio::test]
#[ignore]
async fn t28f_konfigurasi_tidak_ada_atau_tidak_sah() {
    let su = pool_su().await;
    let w = pool_worker().await;

    let skema1 = Fixture::baru(&su, 1, json!({})).await;
    skema1
        .request(&Rangkaian::eligible(1, sekarang() - Duration::minutes(5)))
        .await;
    let hasil = jalankan_job_balikan(&w, skema1.provider_id, sekarang())
        .await
        .expect("skema 1");
    assert_eq!(hasil, HasilPenyusun::KonfigTidakAda);
    assert_eq!(skema1.jumlah_batch().await, 0);

    let ukuran0 = Fixture::baru(&su, 2, json!({"data_balikan": blok(1, 0)})).await;
    ukuran0
        .request(&Rangkaian::eligible(1, sekarang() - Duration::minutes(5)))
        .await;
    let hasil = jalankan_job_balikan(&w, ukuran0.provider_id, sekarang())
        .await
        .expect("ukuran 0");
    assert_eq!(hasil, HasilPenyusun::KonfigTidakValid);
    assert_eq!(ukuran0.jumlah_batch().await, 0);
}
