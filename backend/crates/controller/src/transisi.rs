//! Potongan bersama transisi status yang ditulis controller (ARC-70, ARC-72).
//!
//! Setiap transisi adalah compare-and-set di PostgreSQL ditambah satu baris
//! `request_status_history` di transaksi yang sama (ADR-0001 §3, T-04). Dua hal
//! dijaga di sini supaya dispatcher, penyapu, dan reconciler tidak berbeda:
//!
//! - `terjadi_at` memakai default database (`now()`), bukan jam yang
//!   diinjeksi: trigger T-04 menuntut `terjadi_at >= transaction_timestamp()`.

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
