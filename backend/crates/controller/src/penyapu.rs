//! Penyapu controller (ARC-72, R2 T5 spec `f0-controller-worker`, AC-19–21
//! dan AC-25; ADR-0001 §3, §3a.4 butir 2, §4).
//!
//! Satu siklus menjalankan empat langkah berurutan, semuanya hanya bergantung
//! pada PostgreSQL sehingga tetap berjalan saat Redis mati (U-20a/b):
//!
//! 1. **Entitas nonaktif** (T-02e): request `queued`/`dispatched` yang salah
//!    satu entitasnya nonaktif ditutup K-20 dengan urutan alasan tetap
//!    `fn_entitas_nonaktif_pertama`: `cancelled` bila `attempt_count = 0`
//!    (plus purge P-02 di transaksi yang sama), `failed`/`objek_nonaktif` bila
//!    menunggu retry. Langkah ini mendahului TTL, jadi request di jendela cache
//!    yang TTL-nya juga lewat ditutup K-20, bukan `expired` (U-50).
//! 2. **TTL** (T-02d): `attempt_count = 0` lewat `ttl_expires_at` → `expired` +
//!    P-02; menunggu retry lewat TTL → `failed`/`retry_habis`.
//! 3. **Log `tidak_diketahui`** (R-02): baris `transaction_log` yang masih
//!    `status_akses IS NULL` lebih tua dari timeout total upstream + 60 s
//!    ditutup lewat `fn_tutup_log_tidak_diketahui`. Predikat `IS NULL` di
//!    fungsi itu dan di worker membuat respons terlambat tidak menimpanya.
//! 4. **Purge P-03**: `fn_purge_p03`. Predikat `completed_at + TTL downstream
//!    pool` (S-05) ada di fungsi itu, bukan di sini.
//!
//! Setiap transisi adalah compare-and-set dengan status dan `attempt_count`
//! yang dibaca, jadi race dengan gerbang worker atau penulis lain berakhir
//! dengan tepat satu transisi (U-27, U-52). Koreksi `antre` dan pelepasan slot
//! dijalankan sesudah commit lewat [`crate::transisi::jalankan_efek`].

use chrono::{DateTime, Utc};
use poolrouter_core::kuota::store::KuotaStore;
use poolrouter_core::kuota::sumber_pg::{ParameterDispatch, sql_pool_antre};
use sqlx::PgPool;
use uuid::Uuid;

use crate::transisi::{
    EfekRedis, SQL_LOG_TERAKHIR, SQL_WEBHOOK_STATUS, catat_history, jalankan_efek,
};

/// Batas baris per langkah per siklus, supaya satu siklus tetap pendek.
#[derive(Debug, Clone)]
pub struct KonfigurasiPenyapu {
    pub batas_baris: i64,
    pub batas_purge: i32,
}

impl Default for KonfigurasiPenyapu {
    fn default() -> Self {
        Self {
            batas_baris: 500,
            batas_purge: 1000,
        }
    }
}

/// Jumlah transisi per jenis dalam satu siklus.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RingkasanPenyapu {
    pub k20_cancelled: usize,
    pub k20_failed: usize,
    pub expired: usize,
    pub retry_habis: usize,
    pub log_ditutup: usize,
    pub dipurge: i64,
}

impl RingkasanPenyapu {
    fn ada(&self) -> bool {
        self.k20_cancelled + self.k20_failed + self.expired + self.retry_habis + self.log_ditutup
            > 0
            || self.dipurge > 0
    }
}

/// Request menunggu yang menjadi kandidat langkah 1 dan 2.
struct Menunggu {
    request_id: Uuid,
    status: String,
    attempt_count: i32,
    upstream_pool_id: Option<Uuid>,
    pool_antre: Option<Uuid>,
}

impl Menunggu {
    /// Efek Redis sesudah request ini keluar dari `queued`/`dispatched` tanpa
    /// klaim slot: `antre` turun satu (AC-25), dan slot `dispatched` dilepas.
    fn efek_keluar(&self) -> EfekRedis {
        EfekRedis {
            request_id: self.request_id,
            lepas_slot: match (self.status.as_str(), self.upstream_pool_id) {
                ("dispatched", Some(pool)) => Some((pool, i64::from(self.attempt_count) + 1)),
                _ => None,
            },
            ubah_antre: self.pool_antre.map(|p| (p, -1)),
        }
    }
}

type BarisMenunggu = (Uuid, String, i32, Option<Uuid>, Option<Uuid>);
type BarisLog = (Uuid, DateTime<Utc>, sqlx::types::Json<serde_json::Value>);

fn menunggu(b: BarisMenunggu) -> Menunggu {
    Menunggu {
        request_id: b.0,
        status: b.1,
        attempt_count: b.2,
        upstream_pool_id: b.3,
        pool_antre: b.4,
    }
}

/// Menjalankan satu siklus penyapu. `kuota` `None` berarti Redis tidak
/// terhubung: langkah PostgreSQL tetap jalan, koreksi Redis menunggu rebuild.
pub async fn jalankan_siklus_penyapu<K: KuotaStore>(
    db: &PgPool,
    kuota: Option<&K>,
    konf: &KonfigurasiPenyapu,
    sekarang: DateTime<Utc>,
) -> Result<RingkasanPenyapu, String> {
    let mut ringkasan = RingkasanPenyapu::default();
    let mut efek = Vec::new();

    // `efek` hanya diisi sesudah commit per baris. Galat PostgreSQL di baris
    // berikutnya tidak boleh membuang efek baris yang sudah commit: tanpa itu
    // `antre` tidak turun dan slot tidak dilepas sampai rebuild (AC-25). Jadi
    // efek dijalankan dulu, galat dikembalikan sesudahnya.
    let hasil = async {
        langkah_entitas_nonaktif(db, konf, sekarang, &mut ringkasan, &mut efek).await?;
        langkah_ttl(db, konf, sekarang, &mut ringkasan, &mut efek).await
    }
    .await;
    jalankan_efek(kuota, &efek).await;
    hasil?;
    ringkasan.log_ditutup = tutup_log_tidak_diketahui(db, konf, sekarang).await?;
    ringkasan.dipurge = purge_p03(db, konf).await?;
    if ringkasan.ada() {
        tracing::info!(?ringkasan, "penyapu menutup request");
    }
    Ok(ringkasan)
}

async fn langkah_entitas_nonaktif(
    db: &PgPool,
    konf: &KonfigurasiPenyapu,
    sekarang: DateTime<Utc>,
    ringkasan: &mut RingkasanPenyapu,
    efek: &mut Vec<EfekRedis>,
) -> Result<(), String> {
    let sql = format!(
        "SELECT r.request_id, r.status, r.attempt_count, r.upstream_pool_id, {} AS pool_antre
           FROM request r
          WHERE r.status IN ('queued', 'dispatched') AND r.mode = 'async'
            AND EXISTS (SELECT 1 FROM fn_entitas_nonaktif_pertama(r.request_id))
          ORDER BY r.received_at
          LIMIT $1",
        sql_pool_antre()
    );
    let calon: Vec<BarisMenunggu> = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(konf.batas_baris)
        .fetch_all(db)
        .await
        .map_err(|e| format!("pilih request entitas nonaktif: {e}"))?;

    for baris in calon {
        let m = menunggu(baris);
        let mut tx = db.begin().await.map_err(|e| e.to_string())?;
        // Dibaca ulang di transaksi ini: entitas yang diaktifkan lagi sejak
        // kandidat dipilih membuat request diproses normal (U-53).
        let entitas: Option<(Uuid, String)> =
            sqlx::query_as("SELECT entitas_id, cancel_reason FROM fn_entitas_nonaktif_pertama($1)")
                .bind(m.request_id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e| format!("baca entitas nonaktif: {e}"))?;
        let Some((entitas_id, cancel_reason)) = entitas else {
            continue;
        };

        let menang = if m.attempt_count == 0 {
            let sql = format!(
                "UPDATE request
                    SET status = 'cancelled', cancel_reason = $4, cancelled_by = 'sistem',
                        completed_at = $5, webhook_status = {SQL_WEBHOOK_STATUS}
                  WHERE request_id = $1 AND status = $2 AND attempt_count = $3"
            );
            let n = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(m.request_id)
                .bind(&m.status)
                .bind(m.attempt_count)
                .bind(&cancel_reason)
                .bind(sekarang)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("transisi cancelled K-20: {e}"))?
                .rows_affected();
            if n == 1 {
                catat_history(
                    &mut tx,
                    m.request_id,
                    &m.status,
                    "cancelled",
                    Some(&cancel_reason),
                    Some(entitas_id),
                )
                .await
                .map_err(|e| format!("history cancelled: {e}"))?;
                purge_p02(&mut tx, m.request_id).await?;
                ringkasan.k20_cancelled += 1;
            }
            n == 1
        } else {
            let n = gagalkan(&mut tx, &m, "objek_nonaktif", sekarang, None).await?;
            if n {
                catat_history(
                    &mut tx,
                    m.request_id,
                    &m.status,
                    "failed",
                    Some("objek_nonaktif"),
                    Some(entitas_id),
                )
                .await
                .map_err(|e| format!("history failed objek_nonaktif: {e}"))?;
                ringkasan.k20_failed += 1;
            }
            n
        };
        if menang {
            tx.commit().await.map_err(|e| format!("commit K-20: {e}"))?;
            efek.push(m.efek_keluar());
        }
    }
    Ok(())
}

async fn langkah_ttl(
    db: &PgPool,
    konf: &KonfigurasiPenyapu,
    sekarang: DateTime<Utc>,
    ringkasan: &mut RingkasanPenyapu,
    efek: &mut Vec<EfekRedis>,
) -> Result<(), String> {
    let sql = format!(
        "SELECT r.request_id, r.status, r.attempt_count, r.upstream_pool_id, {} AS pool_antre
           FROM request r
          WHERE r.status IN ('queued', 'dispatched') AND r.mode = 'async'
            AND r.ttl_expires_at <= $1
          ORDER BY r.ttl_expires_at
          LIMIT $2",
        sql_pool_antre()
    );
    let calon: Vec<BarisMenunggu> = sqlx::query_as(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(sekarang)
        .bind(konf.batas_baris)
        .fetch_all(db)
        .await
        .map_err(|e| format!("pilih request lewat TTL: {e}"))?;

    for baris in calon {
        let m = menunggu(baris);
        let mut tx = db.begin().await.map_err(|e| e.to_string())?;
        let menang = if m.attempt_count == 0 {
            let sql = format!(
                "UPDATE request
                    SET status = 'expired', completed_at = $4,
                        webhook_status = {SQL_WEBHOOK_STATUS}
                  WHERE request_id = $1 AND status = $2 AND attempt_count = $3
                    AND ttl_expires_at <= $4"
            );
            let n = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
                .bind(m.request_id)
                .bind(&m.status)
                .bind(m.attempt_count)
                .bind(sekarang)
                .execute(&mut *tx)
                .await
                .map_err(|e| format!("transisi expired: {e}"))?
                .rows_affected();
            if n == 1 {
                catat_history(&mut tx, m.request_id, &m.status, "expired", None, None)
                    .await
                    .map_err(|e| format!("history expired: {e}"))?;
                purge_p02(&mut tx, m.request_id).await?;
                ringkasan.expired += 1;
            }
            n == 1
        } else {
            let n = gagalkan(&mut tx, &m, "retry_habis", sekarang, Some(sekarang)).await?;
            if n {
                catat_history(
                    &mut tx,
                    m.request_id,
                    &m.status,
                    "failed",
                    Some("retry_habis"),
                    None,
                )
                .await
                .map_err(|e| format!("history retry_habis: {e}"))?;
                ringkasan.retry_habis += 1;
            }
            n
        };
        if menang {
            tx.commit().await.map_err(|e| format!("commit TTL: {e}"))?;
            efek.push(m.efek_keluar());
        }
    }
    Ok(())
}

/// Compare-and-set request menunggu retry ke `failed` dengan
/// `final_kode_transaksi` percobaan terakhir. `ttl_lewat` menambah predikat
/// `ttl_expires_at <= ttl_lewat` untuk langkah TTL.
async fn gagalkan(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    m: &Menunggu,
    alasan: &str,
    sekarang: DateTime<Utc>,
    ttl_lewat: Option<DateTime<Utc>>,
) -> Result<bool, String> {
    let sql = format!(
        "UPDATE request
            SET status = 'failed', failure_reason = $4, completed_at = $5,
                final_kode_transaksi = {SQL_LOG_TERAKHIR},
                webhook_status = {SQL_WEBHOOK_STATUS}
          WHERE request_id = $1 AND status = $2 AND attempt_count = $3
            AND ($6::timestamptz IS NULL OR ttl_expires_at <= $6)"
    );
    let n = sqlx::query(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(m.request_id)
        .bind(&m.status)
        .bind(m.attempt_count)
        .bind(alasan)
        .bind(sekarang)
        .bind(ttl_lewat)
        .execute(&mut **tx)
        .await
        .map_err(|e| format!("transisi failed/{alasan}: {e}"))?
        .rows_affected();
    Ok(n == 1)
}

/// P-02 di transaksi transisi terminal (§3).
async fn purge_p02(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    request_id: Uuid,
) -> Result<(), String> {
    sqlx::query_scalar::<_, bool>("SELECT fn_purge_p02($1)")
        .bind(request_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| format!("purge P-02: {e}"))?;
    Ok(())
}

/// Langkah 3: tutup log percobaan yang menggantung (R-02, AC-20).
async fn tutup_log_tidak_diketahui(
    db: &PgPool,
    konf: &KonfigurasiPenyapu,
    sekarang: DateTime<Utc>,
) -> Result<usize, String> {
    // Ambang minimum 60 s menyaring di SQL; ambang per pool dihitung di Rust
    // dari `parameter_dispatch` yang sama dengan TTL slot.
    let calon: Vec<BarisLog> = sqlx::query_as(
        "SELECT l.kode_transaksi, l.requested_at, u.parameter_dispatch
           FROM transaction_log l
           JOIN upstream_pool u ON u.upstream_pool_id = l.upstream_pool_id
          WHERE l.status_akses IS NULL
            AND l.requested_at <= $1 - interval '60 seconds'
          ORDER BY l.requested_at
          LIMIT $2",
    )
    .bind(sekarang)
    .bind(konf.batas_baris)
    .fetch_all(db)
    .await
    .map_err(|e| format!("pilih log menggantung: {e}"))?;

    let mut ditutup = 0;
    for (kode, requested_at, parameter) in calon {
        let ambang = match ParameterDispatch::dari_json(&parameter.0) {
            Ok(p) => p.ambang_reconciler_detik(),
            Err(galat) => {
                tracing::warn!(galat = %galat, "parameter_dispatch tidak sah; log dilewati");
                continue;
            }
        };
        if requested_at > sekarang - chrono::Duration::seconds(ambang) {
            continue;
        }
        let ok: bool = sqlx::query_scalar("SELECT fn_tutup_log_tidak_diketahui($1, $2)")
            .bind(kode)
            .bind(i32::try_from(ambang).unwrap_or(i32::MAX))
            .fetch_one(db)
            .await
            .map_err(|e| format!("tutup log tidak_diketahui: {e}"))?;
        if ok {
            ditutup += 1;
        }
    }
    Ok(ditutup)
}

/// Langkah 4: purge P-03 (AC-21).
async fn purge_p03(db: &PgPool, konf: &KonfigurasiPenyapu) -> Result<i64, String> {
    let n: i32 = sqlx::query_scalar("SELECT fn_purge_p03($1)")
        .bind(konf.batas_purge)
        .fetch_one(db)
        .await
        .map_err(|e| format!("purge P-03: {e}"))?;
    Ok(i64::from(n))
}

/// [`crate::jadwal::Pekerjaan`] penyapu. Redis dihubungkan ulang setiap siklus
/// (seperti job rebuild); kegagalan koneksi tidak menghentikan langkah
/// PostgreSQL.
pub struct Penyapu {
    db: PgPool,
    redis: redis::Client,
    konf: KonfigurasiPenyapu,
}

impl Penyapu {
    pub fn baru(db: PgPool, redis: redis::Client) -> Self {
        Self {
            db,
            redis,
            konf: KonfigurasiPenyapu::default(),
        }
    }
}

impl crate::jadwal::Pekerjaan for Penyapu {
    fn nama(&self) -> &'static str {
        "penyapu"
    }

    fn jalankan(
        &self,
        sekarang: DateTime<Utc>,
    ) -> crate::jadwal::KotakFutur<'_, Result<bool, String>> {
        Box::pin(async move {
            let kuota = crate::kuota_redis(&self.redis).await;
            let r = jalankan_siklus_penyapu(&self.db, kuota.as_ref(), &self.konf, sekarang).await?;
            Ok(r.ada())
        })
    }
}
