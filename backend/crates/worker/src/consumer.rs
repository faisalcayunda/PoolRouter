//! Consumer handler untuk antrian `pr.jobs.balikan`.
//!
//! Satu pesan membawa `provider_id` (UUID). Handler:
//! 1. Baca `provider_id` dari payload.
//! 2. Jalankan `jalankan_job_pengirim` untuk status lama (ack).
//! 3. Jalankan `jalankan_job_balikan` untuk menyusun & kirim batch baru.
//! 4. Jika provider_id tidak valid: reject tanpa requeue.
//! 5. Jika kedua job selesai: ack pesan.
//! Jika provider tidak ada atau config tidak aktif: success (ack) supaya pesan tidak ulang.

use std::str::FromStr;
use uuid::Uuid;

use poolrouter_core::data_balikan::{
    jalankan_job_balikan, jalankan_job_pengirim, PengirimDataBalikan,
};
use poolrouter_core::queue::QueueError;
use sqlx::PgPool;

/// Jalankan kedua job data balikan per provider_id.
pub async fn proses_balikan<P>(
    pool: &PgPool,
    payload: &str,
    pengirim: &P,
) -> Result<(), QueueError>
where
    P: PengirimDataBalikan,
{
    let provider_id = Uuid::from_str(payload)
        .map_err(|e| QueueError::Handler(format!("invalid provider_id: {e}")))?;

    let sekarang = chrono::Utc::now();

    // Jalankan pengirim terlebih dahulu (AC-23–AC-24 dependency).
    if let Err(e) = jalankan_job_pengirim(pool, provider_id, pengirim, sekarang).await {
        tracing::warn!(provider_id = %provider_id, error = %e, "pengirim gagal");
    }

    // Jalankan penyusun batch baru.
    if let Err(e) = jalankan_job_balikan(pool, provider_id, pengirim, sekarang).await {
        tracing::warn!(provider_id = %provider_id, error = %e, "penyusun gagal");
    }

    Ok(())
}
