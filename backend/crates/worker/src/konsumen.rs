//! Penyambung handler ke port antrean [`JobConsumer`] (ADR-0001 bagian 6).
//!
//! Worker hanya bergantung pada trait port, bukan pada tipe apalis/lapin.
//! Adapter `queue-apalis` (ARC-68) yang memutuskan arti `Err`: requeue untuk
//! kegagalan sebelum commit, DLQ untuk error permanen. Handler sendiri
//! mengembalikan `Ok` untuk setiap hasil sesudah commit PostgreSQL, termasuk
//! compare-and-set nol baris, sehingga pesan duplikat selalu di-ack (S4).

use std::sync::Arc;

use poolrouter_core::kuota::store::KuotaStore;
use poolrouter_core::queue::{JobConsumer, QueueError};

use crate::handler::Handler;
use crate::upstream::PemanggilProvider;

/// Jalankan consumer pada satu antrean sampai consumer berhenti.
pub async fn jalankan<C, P, K>(
    consumer: &C,
    antrean: &str,
    handler: Arc<Handler<P, K>>,
) -> Result<(), QueueError>
where
    C: JobConsumer,
    P: PemanggilProvider + 'static,
    K: KuotaStore + 'static,
{
    consumer
        .run(antrean, move |request_id| {
            let handler = Arc::clone(&handler);
            async move {
                let hasil = handler.proses(request_id).await?;
                tracing::debug!(request_id = %request_id, hasil = ?hasil, "worker: pesan selesai");
                Ok(())
            }
        })
        .await
}
