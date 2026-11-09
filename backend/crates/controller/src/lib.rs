//! Controller process of PoolRouter (ARC-69, R2 T2 of `f0-controller-worker`).
//!
//! Exposed as a library so the integration tests in `tests/` can drive the
//! leader lock, the scheduler and the jobs against a real PostgreSQL and Redis
//! (CW10 seams a, b and c). The binary in `src/main.rs` only wires the modules
//! together.

pub mod balikan;
pub mod dispatcher;
pub mod jadwal;
pub mod leader;
pub mod penyapu;
pub mod rebuild;
pub mod reconciler;
pub mod snapshot;
pub mod transisi;
pub mod webhook;

use poolrouter_core::kuota::metrik::Komponen;
use poolrouter_core::kuota::store::KuotaRedis;

/// Store kuota di atas koneksi Redis baru, atau `None` bila Redis tidak bisa
/// dihubungi. Dibuka ulang setiap siklus seperti job rebuild: koneksi
/// multiplexed tidak tersambung ulang sendiri, dan `run_id` harus dibaca dari
/// Redis yang sedang hidup (§5a.2).
pub async fn kuota_redis(
    klien: &redis::Client,
) -> Option<KuotaRedis<redis::aio::MultiplexedConnection>> {
    let koneksi = match klien.get_multiplexed_async_connection().await {
        Ok(k) => k,
        Err(galat) => {
            tracing::warn!(galat = %galat, "koneksi Redis gagal");
            return None;
        }
    };
    match KuotaRedis::baru(koneksi).await {
        Ok(store) => Some(store.dengan_komponen(Komponen::Controller)),
        Err(galat) => {
            tracing::warn!(galat = %galat, "store kuota Redis tidak bisa dibuat");
            None
        }
    }
}
