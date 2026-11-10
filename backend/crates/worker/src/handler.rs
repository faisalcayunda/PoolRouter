//! Alur satu pesan `request_id` (ADR-0001 bagian 3, 3a.4, 4, 5, 5a.1–5a.4;
//! spec `f0-controller-worker` AC-12..AC-18).
//!
//! Urutan, dan alasannya:
//!
//! 1. Baca request dan konfigurasi tanpa kunci. Request yang bukan
//!    `dispatched` (duplikat, sudah dibatalkan, terminal) di-ack tanpa
//!    panggilan dan tanpa baris log (T-01, T-03).
//! 2. Buka secret, buat `kode_transaksi` UUIDv7, render template ganda, hitung
//!    periode kuota. Semua langkah yang bisa gagal (`Err`) ada di sini, sebelum
//!    efek samping pertama.
//! 3. `slot_klaim.lua` (D1.5): perpanjang slot milik percobaan ini dan
//!    kurangi `antre`. Slot hilang atau Redis gagal → ack tanpa menulis apa
//!    pun; reconciler mengembalikan request ke `queued`.
//! 4. Satu transaksi: compare-and-set `dispatched → in_flight` dengan predikat
//!    keenam entitas aktif dibaca dari tabel (gerbang §3a.4) + history +
//!    insert `transaction_log` (`status_akses` NULL) + `attempt_count + 1`.
//!    Nol baris → gerbang: transisi K-20 bila entitas nonaktif, selain itu ack.
//! 5. `attempts_tambah.lua` dengan `requested_at` yang sama (batas 250 ms).
//!    Gagal, timeout, atau slot kedaluwarsa → upstream TIDAK dipanggil; baris
//!    log dibiarkan untuk reconciler (§5a.4).
//! 6. Panggilan upstream lewat [`PemanggilProvider`].
//! 7. Satu transaksi: lengkapi log (`WHERE status_akses IS NULL`) + status
//!    final atau `queued` (`WHERE status = 'in_flight'`) + `webhook_status`.
//! 8. Retry: `antre` naik satu lagi (klaim slot di langkah 3 sudah
//!    menguranginya, sedangkan request kembali `queued`; AC-25, CW3).
//!    `slot_lepas.lua`, lalu ack.
//!
//! Pemetaan ke port antrean: setiap hasil sesudah commit PostgreSQL adalah
//! `Ok` (ack). `Err` hanya untuk kegagalan sebelum commit apa pun (database
//! tidak terjangkau, konfigurasi rusak); adapter antrean (ARC-68) yang
//! memutuskan requeue atau DLQ.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use poolrouter_core::kuota::metrik;
use poolrouter_core::kuota::store::{
    AsalLepas, HasilKlaim, HasilTambah, KonteksKuota, KuotaStore, ModeJendela,
};
use poolrouter_core::kuota::sumber_pg::periode_dari_baris;
use poolrouter_core::queue::QueueError;
use poolrouter_core::retry::{KebijakanBackoff, SumberAcakBackoff};
use poolrouter_core::secret::{Amplop, Brankas};
use serde_json::Value;
use sqlx::postgres::PgRow;
use sqlx::types::Json;
use sqlx::{PgPool, Postgres, Row, Transaction};
use uuid::Uuid;

use crate::keputusan::{self, StatusFinal};
use crate::render;
use crate::upstream::{PemanggilProvider, PermintaanKabel};

/// Batas setiap perintah Redis (ADR-0001 bagian 5a.4). Timeout = Redis tidak
/// tersedia (fail-closed).
pub const BATAS_REDIS: Duration = Duration::from_millis(250);

/// Timeout total upstream bila `parameter_dispatch.timeout_total_upstream_detik`
/// tidak diisi (ADR-0001 bagian 5a.4: 20 s). Satu nilai dengan jalur sync dan
/// rebuild, dari `core` (ARC-241).
pub use poolrouter_core::kuota::sumber_pg::TIMEOUT_UPSTREAM_BAWAAN_DETIK;

/// TTL slot = timeout total upstream + 60 s (§5a.1 butir 5).
const TTL_SLOT_TAMBAHAN_DETIK: i64 = 60;

/// Apa yang terjadi pada satu pesan. Dipakai log dan uji; semua varian adalah
/// ack bagi antrean.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hasil {
    /// Request tidak ada atau bukan `dispatched`: tanpa panggilan, tanpa log.
    BukanDispatched,
    /// Slot percobaan tidak ada, kedaluwarsa, atau Redis gagal sebelum CAS.
    SlotTidakTersedia,
    /// Gerbang dispatch menutup request (`cancelled` atau `failed`).
    Gerbang {
        status: &'static str,
        alasan: String,
    },
    /// CAS nol baris dan tidak ada entitas nonaktif: pihak lain menang.
    KalahCas,
    /// Log sudah commit, tetapi INCR `attempts` gagal/timeout atau slot
    /// kedaluwarsa: upstream tidak dipanggil, log untuk reconciler.
    TanpaPanggilan,
    Done,
    Gagal(&'static str),
    Retry(DateTime<Utc>),
    /// Reconciler sudah menutup percobaan ini; hasil terlambat dibuang (T-06).
    HasilTerlambat,
}

/// Ekspresi `webhook_status` untuk transisi terminal (ADR-0001 §4.1, §3a.5):
/// `menunggu` bila `client_webhook.aktif`, selain itu `tidak_dikirim`. Anak
/// blend selalu `tidak_dikirim` (F3, BL-03).
macro_rules! status_webhook {
    () => {
        "CASE WHEN r.blend_id IS NULL AND coalesce((SELECT w.aktif FROM client_webhook w \
         WHERE w.client_id = r.client_id), false) THEN 'menunggu' ELSE 'tidak_dikirim' END"
    };
}

const SQL_BACA: &str = "SELECT r.status, r.mode, r.attempt_count, r.client_id, \
        r.downstream_endpoint_id, r.downstream_pool_id, r.provider_id, r.upstream_pool_id, \
        r.input_payload, r.ttl_expires_at, \
        u.batas_percobaan, u.retry_backoff_base_detik, u.retry_backoff_cap_detik, \
        floor(u.kuota_harian * (1 - u.margin_kuota))::bigint AS kuota_efektif, \
        u.jendela_kuota, u.parameter_dispatch, \
        u.reset_kuota_jam, u.reset_kuota_zona, u.reset_kuota_berlaku_mulai, \
        u.reset_kuota_jam_berikut, u.reset_kuota_zona_berikut, \
        p.wajib_log_nik, pv.versi AS provider_versi, pv.log_mode AS log_mode_provider, \
        pv.dokumen AS dokumen_provider, \
        coalesce(ev.versi, r.endpoint_config_version) AS endpoint_versi, \
        ev.log_mode AS log_mode_endpoint, ev.dokumen AS dokumen_endpoint \
   FROM request r \
   JOIN upstream_pool u ON u.upstream_pool_id = r.upstream_pool_id \
   JOIN provider p ON p.provider_id = r.provider_id \
   JOIN downstream_endpoint e ON e.downstream_endpoint_id = r.downstream_endpoint_id \
   LEFT JOIN provider_config_versi pv \
          ON pv.provider_id = p.provider_id AND pv.versi = p.config_versi_aktif \
   LEFT JOIN endpoint_config_versi ev \
          ON ev.downstream_endpoint_id = e.downstream_endpoint_id AND ev.versi = e.config_versi_aktif \
  WHERE r.request_id = $1";

/// Gerbang dispatch (§3a.4 butir 1): predikat keenam entitas aktif dibaca dari
/// tabel di transaksi yang sama, lewat fungsi yang juga dipakai penyapu.
const SQL_CAS: &str = "UPDATE request \
    SET status = 'in_flight', attempt_count = attempt_count + 1, \
        first_attempt_at = coalesce(first_attempt_at, $3) \
  WHERE request_id = $1 AND status = 'dispatched' AND attempt_count = $2 \
    AND NOT EXISTS (SELECT 1 FROM fn_entitas_nonaktif_pertama($1))";

const SQL_HISTORY: &str = "INSERT INTO request_status_history \
    (request_id, seq, status_dari, status_ke, alasan, aktor, objek_pemicu_id) \
  SELECT $1, coalesce(max(h.seq), 0) + 1, $2::text, $3::text, $4::text, 'worker', $5::uuid \
    FROM request_status_history h WHERE h.request_id = $1";

const SQL_LOG: &str = "INSERT INTO transaction_log \
    (kode_transaksi, request_id, attempt_no, provider_id, upstream_pool_id, client_id, \
     downstream_endpoint_id, provider_config_version, endpoint_config_version, nik, \
     requested_at, log_mode, request_payload, worker_id, downstream_pool_id) \
  VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)";

const SQL_GERBANG_CANCEL: &str = concat!(
    "WITH e AS (SELECT n.entitas_id, n.cancel_reason FROM fn_entitas_nonaktif_pertama($1) n) \
     UPDATE request r \
        SET status = 'cancelled', cancel_reason = e.cancel_reason, cancelled_by = 'sistem', \
            completed_at = now(), webhook_status = ",
    status_webhook!(),
    " FROM e \
      WHERE r.request_id = $1 AND r.status = 'dispatched' AND r.attempt_count = 0 \
     RETURNING e.cancel_reason AS alasan, e.entitas_id AS objek"
);

const SQL_GERBANG_GAGAL: &str = concat!(
    "WITH e AS (SELECT n.entitas_id FROM fn_entitas_nonaktif_pertama($1) n) \
     UPDATE request r \
        SET status = 'failed', failure_reason = 'objek_nonaktif', completed_at = now(), \
            final_kode_transaksi = (SELECT t.kode_transaksi FROM transaction_log t \
                                     WHERE t.request_id = $1 ORDER BY t.attempt_no DESC LIMIT 1), \
            webhook_status = ",
    status_webhook!(),
    " FROM e \
      WHERE r.request_id = $1 AND r.status = 'dispatched' AND r.attempt_count > 0 \
     RETURNING 'objek_nonaktif'::text AS alasan, e.entitas_id AS objek"
);

const SQL_LENGKAPI_LOG: &str = "UPDATE transaction_log \
    SET responded_at = $2, duration_ms = $3, status_akses = $4, provider_http_status = $5, \
        error_class = $6, response_payload = $7, response_hash = $8 \
  WHERE kode_transaksi = $1 AND status_akses IS NULL";

const SQL_DONE: &str = concat!(
    "UPDATE request r \
        SET status = 'done', completed_at = now(), final_kode_transaksi = $3, \
            output_payload = $4, sumber_hasil = 'provider', webhook_status = ",
    status_webhook!(),
    " WHERE r.request_id = $1 AND r.status = 'in_flight' AND r.attempt_count = $2"
);

const SQL_GAGAL: &str = concat!(
    "UPDATE request r \
        SET status = 'failed', completed_at = now(), final_kode_transaksi = $3, \
            failure_reason = $4, webhook_status = ",
    status_webhook!(),
    " WHERE r.request_id = $1 AND r.status = 'in_flight' AND r.attempt_count = $2"
);

/// Retry kembali `queued` (async saja, §3 dan §5); predikat `mode` di kueri.
const SQL_ANTRE: &str = "UPDATE request r SET status = 'queued', not_before = $3 \
  WHERE r.request_id = $1 AND r.status = 'in_flight' AND r.attempt_count = $2 \
    AND r.mode = 'async'";

fn galat_pg(e: sqlx::Error) -> QueueError {
    QueueError::Handler(format!("postgres: {e}"))
}

fn konfigurasi(pesan: impl Into<String>) -> QueueError {
    QueueError::Handler(pesan.into())
}

fn kol<'r, T>(baris: &'r PgRow, nama: &str) -> Result<T, QueueError>
where
    T: sqlx::Decode<'r, Postgres> + sqlx::Type<Postgres>,
{
    baris
        .try_get(nama)
        .map_err(|e| QueueError::Handler(format!("kolom {nama}: {e}")))
}

async fn tulis_history(
    tx: &mut Transaction<'_, Postgres>,
    request_id: Uuid,
    dari: &str,
    ke: &str,
    alasan: Option<String>,
    objek: Option<Uuid>,
) -> Result<(), QueueError> {
    sqlx::query(SQL_HISTORY)
        .bind(request_id)
        .bind(dari)
        .bind(ke)
        .bind(alasan)
        .bind(objek)
        .execute(&mut **tx)
        .await
        .map_err(galat_pg)?;
    Ok(())
}

/// NIK untuk `transaction_log.nik`: field input `NIK` bila 16 digit.
fn nik_dari(input: &BTreeMap<String, String>) -> Option<String> {
    input
        .get("NIK")
        .filter(|n| n.len() == 16 && n.bytes().all(|c| c.is_ascii_digit()))
        .cloned()
}

/// Handler pesan worker. Semua dependensi masuk lewat konstruktor sehingga uji
/// memakai PostgreSQL (`pr_worker`), Redis, dan mock provider sungguhan.
pub struct Handler<P, K> {
    pool: PgPool,
    kuota: K,
    pemanggil: P,
    brankas: Brankas,
    acak: Box<dyn SumberAcakBackoff>,
    worker_id: String,
}

impl<P, K> Handler<P, K>
where
    P: PemanggilProvider,
    K: KuotaStore,
{
    pub fn baru(
        pool: PgPool,
        kuota: K,
        pemanggil: P,
        brankas: Brankas,
        acak: Box<dyn SumberAcakBackoff>,
        worker_id: impl Into<String>,
    ) -> Self {
        Self {
            pool,
            kuota,
            pemanggil,
            brankas,
            acak,
            worker_id: worker_id.into(),
        }
    }

    /// Proses satu `request_id`. Lihat dokumentasi modul untuk urutannya.
    pub async fn proses(&self, request_id: Uuid) -> Result<Hasil, QueueError> {
        let Some(b) = sqlx::query(SQL_BACA)
            .bind(request_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(galat_pg)?
        else {
            return Ok(Hasil::BukanDispatched);
        };
        let status: String = kol(&b, "status")?;
        if status != "dispatched" {
            return Ok(Hasil::BukanDispatched);
        }
        let mode: String = kol(&b, "mode")?;
        if mode != "async" {
            return Err(konfigurasi("request sync tidak pernah masuk antrean"));
        }

        let attempt_count: i32 = kol(&b, "attempt_count")?;
        let pool_id: Uuid = kol(&b, "upstream_pool_id")?;
        let pool = pool_id.to_string();
        let rid = request_id.to_string();
        // Nomor slot = attempt_count sesudah CAS (`PermintaanToken::nomor_percobaan`).
        let nomor = i64::from(attempt_count) + 1;
        let parameter: Json<Value> = kol(&b, "parameter_dispatch")?;
        let timeout_detik = parameter
            .0
            .get("timeout_total_upstream_detik")
            .and_then(Value::as_i64)
            .filter(|d| *d > 0)
            .unwrap_or(TIMEOUT_UPSTREAM_BAWAAN_DETIK);
        let ttl_slot_mikro = (timeout_detik + TTL_SLOT_TAMBAHAN_DETIK) * 1_000_000;

        // Langkah 2: konfigurasi, secret, render. Semua yang bisa gagal terjadi di
        // sini, sebelum efek samping apa pun di Redis atau PostgreSQL.
        let dokumen: Option<Json<Value>> = kol(&b, "dokumen_provider")?;
        let dokumen =
            dokumen.ok_or_else(|| konfigurasi("provider tanpa versi konfigurasi aktif"))?;
        let dok =
            render::baca_dokumen_provider(&dokumen.0).map_err(|e| konfigurasi(e.to_string()))?;
        if dok.tls_trust != "system" {
            // `ca_kustom`/`pin_spki` menunggu klien ARC-221. Ditolak sebelum CAS
            // supaya tidak ada baris log dan kuota untuk panggilan yang tidak
            // pernah keluar (M-03).
            return Err(konfigurasi(format!(
                "tls_trust {} belum didukung worker",
                dok.tls_trust
            )));
        }
        let dok_endpoint: Option<Json<Value>> = kol(&b, "dokumen_endpoint")?;
        let literal = render::parameter_literal(dok_endpoint.as_ref().map(|j| &j.0));
        let input_payload: Option<Json<Value>> = kol(&b, "input_payload")?;
        let input = render::nilai_input(input_payload.as_ref().map(|j| &j.0));
        let nama = render::nama_secret(&[dok.base_url.as_str(), dok.body_template.as_str()]);
        let secret = self.buka_secret(&nama).await?;

        let kode = Uuid::now_v7();
        let rakitan = render::rakit(&dok, &input, &literal, &secret, kode)
            .map_err(|e| konfigurasi(format!("render template: {e}")))?;

        let log_mode_provider: Option<String> = kol(&b, "log_mode_provider")?;
        let log_mode_endpoint: Option<String> = kol(&b, "log_mode_endpoint")?;
        // Endpoint hanya boleh lebih ketat dari provider (ADR-0001 bagian 7).
        let full = log_mode_provider.as_deref() == Some("full_payload")
            && log_mode_endpoint.as_deref() != Some("metadata_only");
        let log_mode = if full {
            "full_payload"
        } else {
            "metadata_only"
        };
        let request_payload = full.then(|| Json(render::payload_permintaan(&dok.metode, &rakitan)));

        let ttl: Option<DateTime<Utc>> = kol(&b, "ttl_expires_at")?;
        let ttl = ttl.ok_or_else(|| konfigurasi("request async tanpa ttl_expires_at"))?;
        let kebijakan = KebijakanBackoff {
            base_detik: u32::try_from(kol::<i32>(&b, "retry_backoff_base_detik")?).unwrap_or(5),
            cap_detik: u32::try_from(kol::<i32>(&b, "retry_backoff_cap_detik")?).unwrap_or(300),
            batas_percobaan: u32::try_from(kol::<i32>(&b, "batas_percobaan")?).unwrap_or(1),
        };
        let provider_versi: Option<i32> = kol(&b, "provider_versi")?;
        let provider_versi =
            provider_versi.ok_or_else(|| konfigurasi("provider tanpa versi konfigurasi aktif"))?;
        let endpoint_versi: i32 = kol(&b, "endpoint_versi")?;

        // `requested_at` dibuat di Rust dan dipakai untuk log, periode, dan
        // ember sekaligus (§5a.1 butir 1d).
        let requested_at = Utc::now();
        let periode = periode_dari_baris(&b, requested_at)
            .map_err(|e| konfigurasi(format!("periode kuota: {e}")))?;
        let jendela: String = kol(&b, "jendela_kuota")?;
        let ctx = KonteksKuota {
            pool_id: pool.clone(),
            periode_mulai: periode.mulai,
            requested_at,
            mode: if jendela == "kalender" {
                ModeJendela::Kalender
            } else {
                ModeJendela::Bergulir
            },
            kuota_efektif: kol(&b, "kuota_efektif")?,
        };

        // Langkah 3: klaim slot (D1.5), efek samping pertama. Semua langkah yang
        // bisa gagal sudah lewat, sehingga pesan yang ditolak (`Err`) tidak pernah
        // mengurangi `antre`. `dari_antre = 1`: nilai 0 hanya untuk rekonsiliasi.
        let klaim = tokio::time::timeout(
            BATAS_REDIS,
            self.kuota
                .klaim_slot(&pool, &rid, nomor, ttl_slot_mikro, true),
        )
        .await;
        match klaim {
            Ok(Ok(HasilKlaim::Diperpanjang { .. })) => {}
            Ok(Ok(lain)) => {
                tracing::warn!(request_id = %request_id, hasil = ?lain,
                    "worker: slot percobaan tidak tersedia, ack tanpa panggilan");
                return Ok(Hasil::SlotTidakTersedia);
            }
            Ok(Err(e)) => {
                tracing::warn!(request_id = %request_id, error = %e,
                    "worker: klaim slot gagal, ack tanpa panggilan");
                return Ok(Hasil::SlotTidakTersedia);
            }
            Err(_) => {
                metrik::redis_failclosed(metrik::Komponen::Worker);
                tracing::warn!(request_id = %request_id,
                    "worker: klaim slot melewati 250 ms, ack tanpa panggilan");
                return Ok(Hasil::SlotTidakTersedia);
            }
        }

        // Langkah 4: CAS + history + log dalam satu transaksi.
        let mut tx = self.pool.begin().await.map_err(galat_pg)?;
        let menang = sqlx::query(SQL_CAS)
            .bind(request_id)
            .bind(attempt_count)
            .bind(requested_at)
            .execute(&mut *tx)
            .await
            .map_err(galat_pg)?
            .rows_affected()
            == 1;
        if !menang {
            tx.rollback().await.map_err(galat_pg)?;
            return self
                .gerbang(request_id, attempt_count, &pool, &rid, nomor)
                .await;
        }
        tulis_history(&mut tx, request_id, "dispatched", "in_flight", None, None).await?;
        sqlx::query(SQL_LOG)
            .bind(kode)
            .bind(request_id)
            .bind(attempt_count + 1)
            .bind(kol::<Uuid>(&b, "provider_id")?)
            .bind(pool_id)
            .bind(kol::<Uuid>(&b, "client_id")?)
            .bind(kol::<Uuid>(&b, "downstream_endpoint_id")?)
            .bind(provider_versi)
            .bind(endpoint_versi)
            .bind(nik_dari(&input))
            .bind(requested_at)
            .bind(log_mode)
            .bind(request_payload)
            .bind(self.worker_id.as_str())
            .bind(kol::<Option<Uuid>>(&b, "downstream_pool_id")?)
            .execute(&mut *tx)
            .await
            .map_err(galat_pg)?;
        tx.commit().await.map_err(galat_pg)?;

        // Langkah 5: INCR `attempts` sesudah log commit, sebelum panggilan.
        let tambah =
            tokio::time::timeout(BATAS_REDIS, self.kuota.tambah_attempts(&ctx, &rid, nomor)).await;
        match tambah {
            Ok(Ok(HasilTambah::Dicatat { .. })) => {}
            Ok(Ok(HasilTambah::DicatatTerlampaui {
                terpakai, kuota, ..
            })) => {
                // Jaring pengaman M-02: percobaan tetap terjadi dan tercatat.
                tracing::error!(request_id = %request_id, upstream_pool_id = %pool,
                    terpakai, kuota, "worker: terpakai melewati kuota_efektif");
            }
            Ok(Ok(HasilTambah::SlotKedaluwarsa { .. })) => {
                metrik::slot_kedaluwarsa(&pool, 1);
                tracing::warn!(request_id = %request_id, kode_transaksi = %kode,
                    "worker: slot kedaluwarsa sebelum panggilan, log untuk reconciler");
                return Ok(Hasil::TanpaPanggilan);
            }
            Ok(Err(e)) => {
                tracing::warn!(request_id = %request_id, kode_transaksi = %kode, error = %e,
                    "worker: INCR attempts gagal, upstream tidak dipanggil");
                return Ok(Hasil::TanpaPanggilan);
            }
            Err(_) => {
                metrik::redis_failclosed(metrik::Komponen::Worker);
                tracing::warn!(request_id = %request_id, kode_transaksi = %kode,
                    "worker: INCR attempts melewati 250 ms, upstream tidak dipanggil");
                return Ok(Hasil::TanpaPanggilan);
            }
        }

        // Langkah 6: panggilan upstream. Secret hanya di memori.
        let kabel = PermintaanKabel {
            metode: dok.metode.clone(),
            url: rakitan.kabel_url.clone(),
            body: rakitan.kabel_body.clone(),
            tls_trust: dok.tls_trust.clone(),
            batas_total: Duration::from_secs(u64::try_from(timeout_detik).unwrap_or(20)),
        };
        let mulai = Instant::now();
        let hasil = self.pemanggil.panggil(&kabel).await;
        let durasi = i32::try_from(mulai.elapsed().as_millis()).unwrap_or(i32::MAX);
        let sekarang = Utc::now();

        let akhir = keputusan::nilai_hasil(&hasil, &dok.respons, &input);
        let status = keputusan::status_final(
            &akhir,
            &kebijakan,
            u32::try_from(nomor).unwrap_or(u32::MAX),
            sekarang,
            ttl,
            self.acak.as_ref(),
        );
        let response_payload = if full {
            hasil
                .as_ref()
                .ok()
                .map(|r| Json(render::payload_respons(&r.body, &secret)))
        } else {
            None
        };

        // SHA-256 byte body mentah (sebelum parse dan penyamaran secret); NULL
        // tanpa respons HTTP (transport gagal), juga untuk `chk_tls`. Kode
        // respons provider (`provider_response_code`) tetap NULL: dokumen
        // provider F0 tidak menunjuk field kode.
        let response_hash = hasil.as_ref().ok().map(|r| r.sha256_hex());

        // Langkah 7: lengkapi log + status dalam satu transaksi.
        let mut tx = self.pool.begin().await.map_err(galat_pg)?;
        let log = sqlx::query(SQL_LENGKAPI_LOG)
            .bind(kode)
            .bind(akhir.ada_respons.then_some(sekarang))
            .bind(durasi)
            .bind(akhir.status_akses)
            .bind(akhir.http_status.map(i32::from))
            .bind(akhir.error_class.map(|k| k.as_str()))
            .bind(response_payload)
            .bind(response_hash)
            .execute(&mut *tx)
            .await
            .map_err(galat_pg)?
            .rows_affected();
        if log != 1 {
            tx.rollback().await.map_err(galat_pg)?;
            tracing::warn!(request_id = %request_id, kode_transaksi = %kode,
                "worker: log sudah ditutup reconciler, hasil terlambat dibuang");
            return Ok(Hasil::HasilTerlambat);
        }
        let n = attempt_count + 1;
        let (diubah, ke, alasan, hasil_akhir) = match &status {
            StatusFinal::Done(output) => {
                let d = sqlx::query(SQL_DONE)
                    .bind(request_id)
                    .bind(n)
                    .bind(kode)
                    .bind(Json(output.clone()))
                    .execute(&mut *tx)
                    .await
                    .map_err(galat_pg)?
                    .rows_affected();
                (d, "done", None, Hasil::Done)
            }
            StatusFinal::Gagal(reason) => {
                let d = sqlx::query(SQL_GAGAL)
                    .bind(request_id)
                    .bind(n)
                    .bind(kode)
                    .bind(*reason)
                    .execute(&mut *tx)
                    .await
                    .map_err(galat_pg)?
                    .rows_affected();
                (
                    d,
                    "failed",
                    Some((*reason).to_owned()),
                    Hasil::Gagal(reason),
                )
            }
            StatusFinal::Antre(not_before) => {
                let d = sqlx::query(SQL_ANTRE)
                    .bind(request_id)
                    .bind(n)
                    .bind(*not_before)
                    .execute(&mut *tx)
                    .await
                    .map_err(galat_pg)?
                    .rows_affected();
                let alasan = akhir.error_class.map(|k| k.as_str().to_owned());
                (d, "queued", alasan, Hasil::Retry(*not_before))
            }
        };
        if diubah != 1 {
            tx.rollback().await.map_err(galat_pg)?;
            tracing::warn!(request_id = %request_id, kode_transaksi = %kode,
                "worker: request bukan lagi in_flight, hasil terlambat dibuang");
            return Ok(Hasil::HasilTerlambat);
        }
        tulis_history(&mut tx, request_id, "in_flight", ke, alasan, None).await?;
        tx.commit().await.map_err(galat_pg)?;

        // Langkah 8: retry mengembalikan request ke antrean, lalu lepas slot, lalu ack.
        if matches!(hasil_akhir, Hasil::Retry(_)) {
            self.antre_naik(&pool, &rid).await;
        }
        self.lepas(&pool, &rid, nomor).await;
        tracing::info!(request_id = %request_id, kode_transaksi = %kode,
            status_akses = akhir.status_akses, status = ke, "worker: percobaan selesai");
        Ok(hasil_akhir)
    }

    /// CAS nol baris (§3a.4 butir 1): bila status masih `dispatched` dan ada
    /// entitas nonaktif, jalankan transisi K-20 sebagai compare-and-set dari
    /// `dispatched`. Selain itu pihak lain menang: ack tanpa efek samping.
    async fn gerbang(
        &self,
        request_id: Uuid,
        attempt_count: i32,
        pool: &str,
        rid: &str,
        nomor: i64,
    ) -> Result<Hasil, QueueError> {
        let (sql, ke) = if attempt_count == 0 {
            (SQL_GERBANG_CANCEL, "cancelled")
        } else {
            (SQL_GERBANG_GAGAL, "failed")
        };
        let mut tx = self.pool.begin().await.map_err(galat_pg)?;
        let baris = sqlx::query(sql)
            .bind(request_id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(galat_pg)?;
        let Some(baris) = baris else {
            tx.rollback().await.map_err(galat_pg)?;
            return Ok(Hasil::KalahCas);
        };
        let alasan: String = kol(&baris, "alasan")?;
        let objek: Uuid = kol(&baris, "objek")?;
        tulis_history(
            &mut tx,
            request_id,
            "dispatched",
            ke,
            Some(alasan.clone()),
            Some(objek),
        )
        .await?;
        if ke == "cancelled" {
            // P-02 di transaksi transisi terminal (§3, §7.2).
            sqlx::query("SELECT fn_purge_p02($1)")
                .bind(request_id)
                .execute(&mut *tx)
                .await
                .map_err(galat_pg)?;
        }
        tx.commit().await.map_err(galat_pg)?;
        // Slot dilepas saat ack (§3a.4): percobaan ini tidak pernah terjadi.
        self.lepas(pool, rid, nomor).await;
        tracing::info!(request_id = %request_id, status = ke, alasan = %alasan,
            "worker: gerbang dispatch menutup request");
        Ok(Hasil::Gerbang { status: ke, alasan })
    }

    /// Buka secret provider yang dirujuk template. Nama yang tidak ada atau
    /// gagal dibuka menolak pesan sebelum commit (tanpa panggilan).
    async fn buka_secret(&self, nama: &[String]) -> Result<BTreeMap<String, String>, QueueError> {
        let mut peta = BTreeMap::new();
        if nama.is_empty() {
            return Ok(peta);
        }
        let baris = sqlx::query(
            "SELECT nama_secret, secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id \
               FROM provider_secret WHERE nama_secret = ANY($1)",
        )
        .bind(nama.to_vec())
        .fetch_all(&self.pool)
        .await
        .map_err(galat_pg)?;
        for b in &baris {
            let nonce: Vec<u8> = kol(b, "secret_nonce")?;
            let amplop = Amplop {
                nama: kol(b, "nama_secret")?,
                ciphertext: kol(b, "secret_ciphertext")?,
                dek_terbungkus: kol(b, "secret_dek_terbungkus")?,
                nonce: <[u8; 12]>::try_from(nonce.as_slice())
                    .map_err(|_| konfigurasi("secret_nonce bukan 12 byte"))?,
                kek_id: kol(b, "kek_id")?,
            };
            let nilai = self.brankas.dekripsi(&amplop).map_err(|e| {
                konfigurasi(format!("secret {} tidak dapat dibuka: {e:?}", amplop.nama))
            })?;
            peta.insert(amplop.nama.clone(), nilai.ke_dalam());
        }
        if let Some(hilang) = nama.iter().find(|n| !peta.contains_key(*n)) {
            return Err(konfigurasi(format!("secret {hilang} tidak ada")));
        }
        Ok(peta)
    }

    /// Sesudah commit retry: klaim slot (langkah 3) sudah mengurangi `antre`,
    /// padahal request kembali `queued`, jadi `antre` naik satu (AC-25, CW3;
    /// pasangan koreksi `+1` reconciler). Best-effort: Redis gagal atau lambat
    /// tidak membatalkan ack, karena rebuild menghitung ulang `antre` dari
    /// PostgreSQL.
    async fn antre_naik(&self, pool: &str, rid: &str) {
        match tokio::time::timeout(BATAS_REDIS, self.kuota.ubah_antre(pool, 1)).await {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                tracing::warn!(request_id = %rid, error = %e, "worker: koreksi antre gagal");
            }
            Err(_) => {
                tracing::warn!(request_id = %rid, "worker: koreksi antre melewati 250 ms");
            }
        }
    }

    async fn lepas(&self, pool: &str, rid: &str, nomor: i64) {
        let hasil = tokio::time::timeout(
            BATAS_REDIS,
            self.kuota.lepas_slot(pool, rid, nomor, AsalLepas::Kas),
        )
        .await;
        match hasil {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => {
                tracing::warn!(request_id = %rid, error = %e, "worker: lepas slot gagal");
            }
            Err(_) => {
                tracing::warn!(request_id = %rid, "worker: lepas slot melewati 250 ms");
            }
        }
    }
}
