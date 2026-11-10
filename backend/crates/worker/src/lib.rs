//! Worker PoolRouter (ARC-71, R2 T4 spec `f0-controller-worker`).
//!
//! Worker mengonsumsi `request_id` dari antrean lewat port
//! [`poolrouter_core::queue::JobConsumer`] lalu memproses satu percobaan
//! panggilan upstream sesuai ADR-0001 bagian 3, 3a.4, 4, 5 dan 5a, serta
//! ADR-0002 D1–D3:
//!
//! - [`handler`]: alur satu pesan, dari gerbang dispatch sampai status final.
//! - [`render`]: dokumen konfigurasi provider, render template ganda (kabel
//!   dan log) dan penyamaran secret.
//! - [`keputusan`]: pemetaan hasil panggilan ke `status_akses`/`error_class`
//!   dan keputusan retry; murni, tanpa I/O.
//! - [`upstream`]: port panggilan HTTP ke provider beserta implementasi
//!   `reqwest`.
//! - [`konsumen`]: penyambung handler ke port `JobConsumer`.
//! - [`liveness`]: pengawas yang mendeteksi proses tidak hidup lagi.

pub mod handler;
pub mod keputusan;
pub mod konsumen;
pub mod liveness;
pub mod render;
pub mod upstream;
