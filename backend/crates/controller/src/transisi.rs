//! Potongan bersama transisi status yang ditulis controller (ARC-70, ARC-72).
//!
//! Setiap transisi adalah compare-and-set di PostgreSQL ditambah satu baris
//! `request_status_history` di transaksi yang sama (ADR-0001 §3, T-04). Dua hal
//! dijaga di sini supaya dispatcher, penyapu, dan reconciler tidak berbeda:
//!
//! - `terjadi_at` memakai default database (`now()`), bukan jam yang
//!   diinjeksi: trigger T-04 menuntut `terjadi_at >= transaction_timestamp()`.
//! - Efek Redis (lepas slot, koreksi `antre`) dicatat sebagai [`EfekRedis`] dan
//!   baru dijalankan sesudah commit. Kegagalan Redis tidak pernah membatalkan
//!   transisi PostgreSQL (AC-19: penyapu tetap jalan saat Redis mati); rebuild
//!   berikutnya menghitung ulang `antre` dan semaphore dari PostgreSQL.

use poolrouter_core::kuota::store::{AsalLepas, KuotaStore};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

/// Nilai `webhook_status` untuk transisi terminal async: `menunggu` bila client
/// punya webhook aktif, selain itu `tidak_dikirim` (AC-14, §3a.5). Fragmen
/// SQL di dalam `UPDATE request`.
pub const SQL_WEBHOOK_STATUS: &str = "CASE WHEN EXISTS (
             SELECT 1 FROM client_webhook w
              WHERE w.client_id = request.client_id AND w.aktif)
           THEN 'menunggu' ELSE 'tidak_dikirim' END";

/// `kode_transaksi` percobaan terakhir request, untuk `final_kode_transaksi`
/// pada setiap transisi ke `failed` (`chk_final_log`). Fragmen SQL di dalam
/// `UPDATE request`.
pub const SQL_LOG_TERAKHIR: &str = "(SELECT l.kode_transaksi FROM transaction_log l
            WHERE l.request_id = request.request_id
            ORDER BY l.attempt_no DESC LIMIT 1)";

/// Menulis satu baris `request_status_history` dengan aktor `controller`.
///
/// `seq` diambil dari baris terakhir request itu. Dua penulis yang berebut
/// tidak bisa sampai di sini bersamaan: keduanya lebih dulu menjalankan
/// `UPDATE request`, dan yang kalah mendapat nol baris lalu berhenti.
pub async fn catat_history(
    tx: &mut Transaction<'_, Postgres>,
    request_id: Uuid,
    dari: &str,
    ke: &str,
    alasan: Option<&str>,
    pemicu: Option<Uuid>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO request_status_history
                (request_id, seq, status_dari, status_ke, alasan, aktor, objek_pemicu_id)
         SELECT $1, COALESCE(MAX(h.seq), 0) + 1, $2, $3, $4, 'controller', $5
           FROM request_status_history h
          WHERE h.request_id = $1",
    )
    .bind(request_id)
    .bind(dari)
    .bind(ke)
    .bind(alasan)
    .bind(pemicu)
    .execute(&mut **tx)
    .await?;
    Ok(())
}

/// Efek Redis satu transisi, dijalankan sesudah commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EfekRedis {
    pub request_id: Uuid,
    /// Slot yang dilepas: `(upstream_pool_id, nomor_percobaan)`.
    pub lepas_slot: Option<(Uuid, i64)>,
    /// Koreksi `antre`: `(pool, delta)` (AC-25, CW3).
    pub ubah_antre: Option<(Uuid, i64)>,
}

/// Menjalankan efek Redis sesudah commit, best-effort. Tanpa store (Redis tidak
/// terhubung) atau saat skrip menjawab `belum_siap`, efek dilewati: rebuild
/// akan membangun `antre` dan semaphore dari PostgreSQL.
pub async fn jalankan_efek<K: KuotaStore>(kuota: Option<&K>, efek: &[EfekRedis]) {
    let Some(kuota) = kuota else {
        if !efek.is_empty() {
            tracing::warn!(
                jumlah = efek.len(),
                "Redis tidak tersedia; koreksi slot/antre menunggu rebuild"
            );
        }
        return;
    };
    for e in efek {
        let request_id = e.request_id.to_string();
        if let Some((pool, nomor)) = e.lepas_slot
            && let Err(galat) = kuota
                .lepas_slot(&pool.to_string(), &request_id, nomor, AsalLepas::Reconciler)
                .await
        {
            tracing::warn!(upstream_pool_id = %pool, galat = %galat, "lepas slot gagal");
        }
        if let Some((pool, delta)) = e.ubah_antre
            && let Err(galat) = kuota.ubah_antre(&pool.to_string(), delta).await
        {
            tracing::warn!(upstream_pool_id = %pool, galat = %galat, "koreksi antre gagal");
        }
    }
}
