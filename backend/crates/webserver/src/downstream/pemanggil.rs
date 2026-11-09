//! Seam panggilan provider untuk jalur sync (ARC-66, ADR-0001 rev 15 §4,
//! ADR-0002).
//!
//! Klien HTTP provider (trust TLS per provider, tanpa redirect, timeout R-04,
//! resolve secret) adalah ARC-64 dan belum ada di `main`. Handler sync hanya
//! bergantung pada trait ini. Selama tidak ada implementasi yang dipasang,
//! handler menjawab 503 `layanan_tidak_tersedia` **sebelum** menulis apa pun
//! dan sebelum menyentuh kuota, supaya tidak ada baris `in_flight` tanpa
//! panggilan.

use serde_json::Value;
use uuid::Uuid;

use super::downstream_sesi::Fut;
use super::permintaan::HasilPanggilan;

/// Satu percobaan sync. `input` hanya hidup di memori proses; implementasi
/// tidak boleh mencatatnya di log.
#[derive(Clone)]
pub struct PanggilanProvider {
    pub request_id: Uuid,
    /// `transactionId` untuk provider (K-25, ADR-0002 D2).
    pub kode_transaksi: Uuid,
    pub provider_id: Uuid,
    pub provider_config_version: i32,
    pub endpoint_config_version: i32,
    pub input: Value,
}

impl std::fmt::Debug for PanggilanProvider {
    // `input` memuat NIK; `Debug` tidak pernah mencetaknya (AC-4).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PanggilanProvider")
            .field("request_id", &self.request_id)
            .field("kode_transaksi", &self.kode_transaksi)
            .field("provider_id", &self.provider_id)
            .finish_non_exhaustive()
    }
}

pub trait PemanggilProvider: Send + Sync + 'static {
    /// Satu panggilan upstream, sudah dibatasi timeout total provider. Hasilnya
    /// selalu salah satu dari dua bentuk; tidak ada panic dan tidak ada galat
    /// di luar [`HasilPanggilan`].
    fn panggil(&self, p: PanggilanProvider) -> Fut<'_, HasilPanggilan>;
}
