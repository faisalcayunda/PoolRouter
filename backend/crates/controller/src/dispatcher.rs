//! Dispatcher controller (ARC-70, R2 T3 spec `f0-controller-worker`,
//! ADR-0001 §4, §5, §5a.1, §5a.3).
//!
//! Satu putaran, per upstream pool aktif:
//!
//! 1. Dalam satu transaksi pendek, pilih satu request `queued` yang
//!    `not_before`-nya NULL atau sudah lewat, urut `(prioritas_efektif,
//!    dispatch_key)` dengan `FOR UPDATE SKIP LOCKED` (AC-7). `dispatch_key`
//!    acak CSPRNG dari intake, jadi daftar NIK berurutan tidak dikirim
//!    berurutan (K-05).
//! 2. Ambil token lewat `kuota_ambil_token.lua` (`sync = false`, AC-8). Skrip
//!    yang sama menegakkan rate, kuota efektif, cadangan porsi sync, slot
//!    konkurensi, dan retry budget.
//! 3. Compare-and-set `queued → dispatched` (mengisi `upstream_pool_id`) plus
//!    baris history, commit, **baru** publish `request_id` (AC-9). Publish yang
//!    gagal tidak membatalkan status: reconciler mengembalikan request
//!    `dispatched` yang macet ke `queued` (AC-22).
//! 4. Tidur jeda acak di dalam satu slot token, 0 sampai `1/rate` (AC-10).
//!
//! Satu putaran dibatasi jumlah dan waktu ([`KonfigurasiPutaran`]) supaya
//! pemanggil bisa memeriksa leader lock di antara putaran.

use std::time::{Duration, Instant};

use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use chrono::{DateTime, Utc};
use poolrouter_core::kuota::pemulihan::GalatPemulihan;
use poolrouter_core::kuota::store::{
    AsalLepas, GalatStore, HasilToken, KonteksKuota, KuotaStore, ModeJendela, PermintaanToken,
};
use poolrouter_core::kuota::sumber_pg::{ParameterDispatch, SQL_RUTE_PERTAMA, periode_dari_baris};
use poolrouter_core::queue::{JobPublisher, nama_antrean_upstream};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::transisi::catat_history;

/// Sumber jeda acak K-05. Produksi memakai [`JedaAwsLc`]; uji memakai
/// [`JedaTetap`] supaya waktu putaran bisa diperkirakan.
pub trait SumberJeda: Send + Sync {
    /// Nilai di `0..=batas_mikro`.
    fn jeda_mikro(&self, batas_mikro: i64) -> i64;
}

/// Jeda acak seragam dari `aws-lc-rs` (ADR-0004 §4.4).
#[derive(Default)]
pub struct JedaAwsLc(SystemRandom);

impl JedaAwsLc {
    pub fn baru() -> Self {
        Self(SystemRandom::new())
    }
}

impl SumberJeda for JedaAwsLc {
    fn jeda_mikro(&self, batas_mikro: i64) -> i64 {
        if batas_mikro <= 0 {
            return 0;
        }
        let mut buf = [0u8; 8];
        // RNG sistem gagal: tanpa jeda. Jeda hanya pengacak waktu; rate tetap
        // dijaga skrip token, jadi ini tidak pernah menambah panggilan.
        if self.0.fill(&mut buf).is_err() {
            return 0;
        }
        let v = u64::from_le_bytes(buf);
        // Bias modulo diabaikan: rentang <= 1e6 jauh di bawah 2^64.
        (v % (batas_mikro as u64 + 1)) as i64
    }
}

/// Jeda tetap untuk uji, dipotong ke batas slot.
pub struct JedaTetap(pub i64);

impl SumberJeda for JedaTetap {
    fn jeda_mikro(&self, batas_mikro: i64) -> i64 {
        self.0.clamp(0, batas_mikro.max(0))
    }
}

/// Batas satu putaran.
#[derive(Debug, Clone)]
pub struct KonfigurasiPutaran {
    /// Paling banyak request yang di-dispatch per pool per putaran.
    pub batas_per_pool: usize,
    /// Batas waktu dinding satu putaran.
    pub batas_waktu: Duration,
    /// Hanya pool ini (uji dan operasi terarah). `None` = semua pool aktif.
    pub saring_pool: Option<Vec<Uuid>>,
}

impl Default for KonfigurasiPutaran {
    fn default() -> Self {
        Self {
            batas_per_pool: 500,
            batas_waktu: Duration::from_secs(2),
            saring_pool: None,
        }
    }
}

/// Hasil satu putaran, untuk log dan uji.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RingkasanPutaran {
    /// Request yang berpindah ke `dispatched`, urut waktu.
    pub dikirim: Vec<Uuid>,
    /// Publish yang gagal sesudah commit (dipulihkan reconciler).
    pub publish_gagal: usize,
    /// Request menunggu retry yang dilewati karena retry budget habis.
    pub dilewati_retry: usize,
    /// Redis tidak tersedia atau belum siap: putaran berhenti (fail-closed).
    pub redis_tidak_siap: bool,
    /// Tunggu terpendek yang dilaporkan skrip token (rate), mikrodetik.
    pub tunggu_mikro: Option<i64>,
}

/// Apa yang dilakukan dispatcher dengan jawaban skrip token.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeputusanToken {
    /// Token diberikan: transisi lalu publish.
    Kirim,
    /// Retry budget habis untuk request menunggu retry: lewati request ini,
    /// request baru tetap jalan (§5a.3, AC-8).
    Lewati,
    /// Penolakan lain (rate, kuota, porsi sync, konkurensi, slot ganda,
    /// anggaran): berhenti untuk pool ini di putaran ini.
    Berhenti,
}

/// Keputusan murni atas jawaban token; `attempt_count` dibaca dari baris.
pub fn putuskan_token(hasil: &HasilToken, attempt_count: i32) -> KeputusanToken {
    match hasil {
        HasilToken::Diberikan(_) => KeputusanToken::Kirim,
        HasilToken::RetryBudgetHabis(_) if attempt_count > 0 => KeputusanToken::Lewati,
        _ => KeputusanToken::Berhenti,
    }
}

/// `floor(kuota_harian * (1 - margin))` dengan margin tiga desimal
/// (`numeric(4,3)`), dihitung dalam bilangan bulat supaya tidak ada galat
/// pembulatan float di batas kuota (M-02).
pub fn kuota_efektif(kuota_harian: i64, margin_teks: &str) -> Option<i64> {
    let margin: f64 = margin_teks.trim().parse().ok()?;
    if !(0.0..1.0).contains(&margin) {
        return None;
    }
    let margin_mili = (margin * 1000.0).round() as i64;
    let hasil = kuota_harian.checked_mul(1000 - margin_mili)? / 1000;
    (hasil > 0).then_some(hasil)
}

/// Konfigurasi satu upstream pool, dibaca ulang setiap putaran.
struct PoolDispatch {
    id: Uuid,
    konteks: KonteksKuota,
    parameter: ParameterDispatch,
    rasio_retry: f64,
    retry_min: i64,
    porsi_sync: f64,
}

const SQL_POOL: &str = "SELECT upstream_pool_id, kuota_harian, margin_kuota::text AS margin_kuota,
        jendela_kuota, parameter_dispatch, porsi_sync_maks::text AS porsi_sync_maks,
        retry_rasio::text AS retry_rasio, retry_min_per_jam,
        reset_kuota_jam, reset_kuota_zona, reset_kuota_berlaku_mulai,
        reset_kuota_jam_berikut, reset_kuota_zona_berikut
   FROM upstream_pool
  WHERE aktif
  ORDER BY upstream_pool_id";

fn pool_dari_baris(baris: &PgRow, sekarang: DateTime<Utc>) -> Result<PoolDispatch, String> {
    let ambil = |e: sqlx::Error| e.to_string();
    let id: Uuid = baris.try_get("upstream_pool_id").map_err(ambil)?;
    let kuota_harian: i32 = baris.try_get("kuota_harian").map_err(ambil)?;
    let margin: String = baris.try_get("margin_kuota").map_err(ambil)?;
    let jendela: String = baris.try_get("jendela_kuota").map_err(ambil)?;
    let parameter: sqlx::types::Json<serde_json::Value> =
        baris.try_get("parameter_dispatch").map_err(ambil)?;
    let angka = |kolom: &str| -> Result<f64, String> {
        baris
            .try_get::<String, _>(kolom)
            .map_err(|e| e.to_string())?
            .parse::<f64>()
            .map_err(|e| format!("{kolom}: {e}"))
    };
    let retry_min: i32 = baris.try_get("retry_min_per_jam").map_err(ambil)?;
    let periode = periode_dari_baris(baris, sekarang).map_err(|e: GalatPemulihan| e.to_string())?;
    let kuota_efektif = kuota_efektif(i64::from(kuota_harian), &margin)
        .ok_or_else(|| format!("kuota efektif pool {id} tidak sah"))?;
    Ok(PoolDispatch {
        id,
        konteks: KonteksKuota {
            pool_id: id.to_string(),
            periode_mulai: periode.mulai,
            requested_at: sekarang,
            mode: if jendela == "kalender" {
                ModeJendela::Kalender
            } else {
                ModeJendela::Bergulir
            },
            kuota_efektif,
        },
        parameter: ParameterDispatch::dari_json(&parameter.0).map_err(|e| e.to_string())?,
        rasio_retry: angka("retry_rasio")?,
        retry_min: i64::from(retry_min),
        porsi_sync: angka("porsi_sync_maks")?,
    })
}

/// Berapa kandidat teratas yang dibaca dari tiap sumber (satu downstream pool,
/// atau request menunggu retry) sebelum dikunci. Harus lebih besar dari jumlah
/// dispatcher yang boleh memegang kunci baris bersamaan: `SKIP LOCKED` hanya
/// melewati baris yang sudah dikunci dispatcher lain, jadi kandidat yang
/// dikunci semua tidak menyisakan pilihan di putaran itu. Hanya satu leader
/// yang berjalan di produksi; uji AC-11 memakai dua.
const KANDIDAT_PER_SUMBER: i64 = 32;

/// Kandidat berikutnya untuk satu upstream pool, urut `(prioritas_efektif,
/// dispatch_key)`, `FOR UPDATE SKIP LOCKED`. Request yang belum pernah
/// di-dispatch belum punya `upstream_pool_id` (EN-04), jadi pool-nya diturunkan
/// dari rute pertama yang aktif (asumsi CW4; `SQL_RUTE_PERTAMA`, fragmen yang
/// sama dengan rebuild dan penyapu); request menunggu retry tetap di pool
/// percobaan sebelumnya.
///
/// Bentuknya dipilih supaya `request_dispatch_idx` (`downstream_pool_id,
/// prioritas_efektif, dispatch_key`) terpakai berurutan, bukan seq scan plus
/// sort seluruh antrean (ARC-229):
///
/// 1. `hulu`: downstream pool yang rute pertamanya `$1`, dihitung sekali per
///    kueri dari tabel rute yang kecil, bukan per baris `request`.
/// 2. `teratas`: per downstream pool, `KANDIDAT_PER_SUMBER` request baru
///    teratas lewat indeks (`LATERAL ... LIMIT`), digabung dengan request
///    menunggu retry `upstream_pool_id = $1`. Tanpa kunci: `FOR UPDATE` tidak
///    boleh di `UNION` atau subquery ber-`LIMIT` yang menyebar kunci ke baris
///    yang tidak dipilih.
/// 3. Kueri luar mengunci kandidat terbaik yang tidak sedang dikunci dispatcher
///    lain. Yang disortir paling banyak `KANDIDAT_PER_SUMBER` kali jumlah
///    sumber, berapa pun panjang antreannya.
///
/// Parameter: `$1` upstream pool, `$2` waktu sekarang, `$3` request yang
/// dilewati putaran ini.
pub fn sql_calon() -> String {
    format!(
        "WITH hulu AS (
            SELECT r.downstream_pool_id
              FROM (SELECT DISTINCT downstream_pool_id FROM downstream_pool_rute) r
             WHERE {SQL_RUTE_PERTAMA} = $1
         ), teratas AS (
            (SELECT t.request_id
               FROM hulu h
              CROSS JOIN LATERAL (
                    SELECT q.request_id
                      FROM request q
                     WHERE q.downstream_pool_id = h.downstream_pool_id
                       AND q.status = 'queued'
                       AND q.mode = 'async'
                       AND q.upstream_pool_id IS NULL
                       AND (q.not_before IS NULL OR q.not_before <= $2)
                       AND q.request_id <> ALL($3)
                     ORDER BY q.prioritas_efektif, q.dispatch_key
                     LIMIT {KANDIDAT_PER_SUMBER}) t)
            UNION ALL
            (SELECT q.request_id
               FROM request q
              WHERE q.upstream_pool_id = $1
                AND q.status = 'queued'
                AND q.mode = 'async'
                AND (q.not_before IS NULL OR q.not_before <= $2)
                AND q.request_id <> ALL($3)
              ORDER BY q.prioritas_efektif, q.dispatch_key
              LIMIT {KANDIDAT_PER_SUMBER})
         )
         SELECT r.request_id, r.attempt_count
           FROM request r
          WHERE r.request_id IN (SELECT request_id FROM teratas)
            AND r.status = 'queued'
          ORDER BY r.prioritas_efektif, r.dispatch_key
          LIMIT 1
          FOR UPDATE OF r SKIP LOCKED"
    )
}

/// Menjalankan satu putaran dispatcher atas semua upstream pool aktif.
///
/// `Err` hanya untuk kegagalan PostgreSQL yang membuat putaran tidak bisa
/// dilanjutkan. Redis yang tidak siap bukan error: putaran berhenti dan
/// [`RingkasanPutaran::redis_tidak_siap`] diisi (fail-closed, §5).
pub async fn jalankan_putaran<K, P>(
    db: &PgPool,
    kuota: &K,
    publisher: &P,
    jeda: &dyn SumberJeda,
    konf: &KonfigurasiPutaran,
    sekarang: DateTime<Utc>,
) -> Result<RingkasanPutaran, String>
where
    K: KuotaStore,
    P: JobPublisher,
{
    let mulai = Instant::now();
    let mut ringkasan = RingkasanPutaran::default();
    let baris_pool = sqlx::query(SQL_POOL)
        .fetch_all(db)
        .await
        .map_err(|e| format!("baca upstream_pool: {e}"))?;
    let sql = sql_calon();

    for baris in baris_pool {
        let pool = match pool_dari_baris(&baris, sekarang) {
            Ok(p) => p,
            Err(galat) => {
                tracing::warn!(galat = %galat, "konfigurasi upstream pool tidak sah; pool dilewati");
                continue;
            }
        };
        if let Some(saring) = &konf.saring_pool
            && !saring.contains(&pool.id)
        {
            continue;
        }
        let antrean = nama_antrean_upstream(pool.id);
        let mut lewati: Vec<Uuid> = Vec::new();
        let mut dikirim_pool = 0usize;

        while dikirim_pool < konf.batas_per_pool && mulai.elapsed() < konf.batas_waktu {
            let mut tx = db.begin().await.map_err(|e| e.to_string())?;
            let calon: Option<(Uuid, i32)> = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(pool.id)
                .bind(sekarang)
                .bind(lewati.as_slice())
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| format!("pilih request queued: {e}"))?;
            let Some((request_id, attempt_count)) = calon else {
                break;
            };
            let nomor = i64::from(attempt_count) + 1;
            let permintaan = PermintaanToken {
                request_id: request_id.to_string(),
                nomor_percobaan: nomor,
                sync: false,
                maks_konkurensi: pool.parameter.maks_konkurensi,
                ttl_slot_mikro: pool.parameter.ttl_slot_mikro(),
                rasio_retry: pool.rasio_retry,
                retry_min: pool.retry_min,
                porsi_sync: pool.porsi_sync,
                jendela_rate: pool.parameter.jendela_rate.clone(),
            };
            let hasil = match kuota.ambil_token(&pool.konteks, &permintaan).await {
                Ok(h) => h,
                Err(GalatStore::BelumSiap | GalatStore::Redis(_)) => {
                    // Fail-closed: tidak ada token tanpa Redis yang siap.
                    ringkasan.redis_tidak_siap = true;
                    return Ok(ringkasan);
                }
                Err(galat) => {
                    tracing::warn!(upstream_pool_id = %pool.id, galat = %galat, "skrip token menolak argumen; pool dilewati");
                    break;
                }
            };
            match putuskan_token(&hasil, attempt_count) {
                KeputusanToken::Lewati => {
                    lewati.push(request_id);
                    ringkasan.dilewati_retry += 1;
                    continue;
                }
                KeputusanToken::Berhenti => {
                    if let HasilToken::RateLimit(s) = &hasil {
                        let t = s.tunggu_mikro.max(0);
                        ringkasan.tunggu_mikro =
                            Some(ringkasan.tunggu_mikro.map_or(t, |lama| lama.min(t)));
                    }
                    break;
                }
                KeputusanToken::Kirim => {}
            }

            if let Err(galat) = tandai_dispatched(tx, request_id, pool.id, sekarang).await {
                // Token sudah memegang slot; lepas supaya kapasitas tidak
                // tertahan sampai TTL (D1.5).
                let _ = kuota
                    .lepas_slot(
                        &pool.id.to_string(),
                        &request_id.to_string(),
                        nomor,
                        AsalLepas::Dispatcher,
                    )
                    .await;
                return Err(galat);
            }

            if let Err(galat) = publisher.publish(&antrean, request_id).await {
                ringkasan.publish_gagal += 1;
                tracing::warn!(upstream_pool_id = %pool.id, galat = %galat, "publish gagal sesudah commit; reconciler akan memulihkan");
            }
            ringkasan.dikirim.push(request_id);
            dikirim_pool += 1;

            let jeda_mikro = jeda.jeda_mikro(pool.parameter.batas_jeda_mikro());
            if jeda_mikro > 0 {
                tokio::time::sleep(Duration::from_micros(jeda_mikro as u64)).await;
            }
        }
    }
    Ok(ringkasan)
}

/// Compare-and-set `queued → dispatched` + history, lalu commit.
async fn tandai_dispatched(
    mut tx: sqlx::Transaction<'_, sqlx::Postgres>,
    request_id: Uuid,
    pool_id: Uuid,
    sekarang: DateTime<Utc>,
) -> Result<(), String> {
    let n = sqlx::query(
        "UPDATE request
            SET status = 'dispatched', upstream_pool_id = $2, dispatched_at = $3
          WHERE request_id = $1 AND status = 'queued'",
    )
    .bind(request_id)
    .bind(pool_id)
    .bind(sekarang)
    .execute(&mut *tx)
    .await
    .map_err(|e| format!("transisi dispatched: {e}"))?
    .rows_affected();
    if n != 1 {
        return Err(format!(
            "compare-and-set dispatched nol baris untuk {request_id}"
        ));
    }
    catat_history(&mut tx, request_id, "queued", "dispatched", None, None)
        .await
        .map_err(|e| format!("history dispatched: {e}"))?;
    tx.commit()
        .await
        .map_err(|e| format!("commit dispatched: {e}"))
}

/// Loop dispatcher selama proses ini leader (dipanggil dari `main.rs`).
///
/// Berjalan di task sendiri, terpisah dari penjadwal 10 s, supaya dispatch
/// mengikuti rate pool, bukan kadens siklus. Leader lock diperiksa sebelum
/// setiap putaran; putaran dibatasi [`KonfigurasiPutaran::batas_waktu`], jadi
/// leader yang kehilangan lock berhenti mengirim paling lambat satu putaran
/// kemudian (AC-4).
pub async fn jalankan_loop_dispatcher<P: JobPublisher>(
    leader: std::sync::Arc<dyn crate::leader::Leader>,
    db: PgPool,
    redis: redis::Client,
    publisher: P,
    berhenti: tokio_util::sync::CancellationToken,
) {
    let jeda = JedaAwsLc::baru();
    let konf = KonfigurasiPutaran::default();
    loop {
        if berhenti.is_cancelled() || !leader.masih_leader().await {
            tracing::warn!("dispatcher berhenti: bukan leader atau proses berhenti");
            return;
        }
        let tidur = match crate::kuota_redis(&redis).await {
            None => Duration::from_secs(1),
            Some(kuota) => {
                match jalankan_putaran(&db, &kuota, &publisher, &jeda, &konf, Utc::now()).await {
                    Ok(r) if r.redis_tidak_siap => Duration::from_secs(1),
                    Ok(r) if !r.dikirim.is_empty() => Duration::ZERO,
                    Ok(r) => r
                        .tunggu_mikro
                        .map(|t| Duration::from_micros(t.clamp(10_000, 1_000_000) as u64))
                        .unwrap_or(Duration::from_secs(1)),
                    Err(galat) => {
                        tracing::error!(galat = %galat, "putaran dispatcher gagal");
                        Duration::from_secs(1)
                    }
                }
            }
        };
        tokio::select! {
            () = berhenti.cancelled() => return,
            () = tokio::time::sleep(tidur) => {}
        }
    }
}

#[cfg(test)]
mod uji {
    use super::*;
    use poolrouter_core::kuota::store::StatusToken;

    fn s() -> StatusToken {
        StatusToken {
            terpakai: 0,
            slot_aktif: 0,
            kedaluwarsa: 0,
            tunggu_mikro: 0,
            isi: 0,
            isi_porsi: 0,
        }
    }

    #[test]
    fn retry_budget_habis_hanya_melewati_request_menunggu_retry() {
        assert_eq!(
            putuskan_token(&HasilToken::RetryBudgetHabis(s()), 1),
            KeputusanToken::Lewati
        );
        assert_eq!(
            putuskan_token(&HasilToken::RetryBudgetHabis(s()), 0),
            KeputusanToken::Berhenti
        );
        assert_eq!(
            putuskan_token(&HasilToken::Diberikan(s()), 2),
            KeputusanToken::Kirim
        );
        for h in [
            HasilToken::KuotaHabis(s()),
            HasilToken::KonkurensiPenuh(s()),
            HasilToken::PorsiSyncHabis(s()),
            HasilToken::RateLimit(s()),
            HasilToken::SlotGanda(s()),
            HasilToken::AnggaranHabis(s()),
        ] {
            assert_eq!(putuskan_token(&h, 0), KeputusanToken::Berhenti, "{h:?}");
        }
    }

    #[test]
    fn kuota_efektif_dibulatkan_ke_bawah_tanpa_galat_float() {
        assert_eq!(kuota_efektif(10_000, "0.010"), Some(9_900));
        assert_eq!(kuota_efektif(1_000, "0.001"), Some(999));
        assert_eq!(kuota_efektif(7, "0.200"), Some(5));
        assert_eq!(kuota_efektif(1, "0.200"), None, "kuota efektif nol ditolak");
        assert_eq!(kuota_efektif(100, "x"), None);
    }

    #[test]
    fn jeda_selalu_di_dalam_slot() {
        let acak = JedaAwsLc::baru();
        for _ in 0..1_000 {
            let j = acak.jeda_mikro(100_000);
            assert!((0..=100_000).contains(&j));
        }
        assert_eq!(acak.jeda_mikro(0), 0);
        assert_eq!(JedaTetap(500).jeda_mikro(100), 100);
        assert_eq!(JedaTetap(50).jeda_mikro(100), 50);
    }
}
