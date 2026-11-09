//! Kontrak data balikan otomatis (ADR-0006 §6, AC-15–AC-19).
//!
//! Mesin batch data balikan dibangun **di balik port** [`PengirimDataBalikan`]
//! (ADR-0006 §6 opsi B): penjadwalan, penyusunan, dan pengiriman bisa selesai
//! dan diuji dengan adapter tiruan selama format kabel, transport,
//! autentikasi, dan ack Dukcapil masih terbuka (D-08). Adapter asli menjadi
//! T8 (ARC-125) dan hanya perlu lulus suite kontrak yang sama.
//!
//! - [`KonfigurasiDataBalikan::parse`] — blok `data_balikan` pada
//!   `config_schema_version` = 2 (AC-15).
//! - [`evaluasi_pemicu`] — pemicu `JumlahN` atau `Periode` (AC-16).
//! - [`jeda_batch`] — full jitter batch gagal, berbagi
//!   [`SumberAcakBackoff`](crate::retry::SumberAcakBackoff) dengan retry
//!   webhook (AC-17).
//! - [`PengirimDataBalikan`] dan [`PengirimTiruan`] — port + adapter tiruan
//!   (AC-18).
//!
//! NIK dan `field_input` hanya ada di dalam [`ItemDataBalikan`], yang tidak
//! punya `Debug`/`Display` yang menampilkan nilainya (AC-19).

mod item;
mod konfigurasi;
mod pemicu;
mod pengirim;
mod penyusun;

pub use item::ItemDataBalikan;
pub use konfigurasi::{
    BACKOFF_BASE_DETIK_DEFAULT, BACKOFF_CAP_DETIK_DEFAULT, FORMAT_VERSION_DEFAULT,
    GalatDataBalikan, KIRIM_BATAS_PERCOBAAN_DEFAULT, KonfigurasiDataBalikan,
    PEMICU_JUMLAH_N_DEFAULT, Periode, TOLERANSI_DETIK_DEFAULT, UKURAN_BATCH_MAKS_DEFAULT,
};
pub use pemicu::{PemicuBatch, evaluasi_pemicu, jeda_batch, kebijakan_batch, zona_dari_nama};
pub use pengirim::{
    BatchKeluar, HasilAck, HasilDiprogam, HasilKirimBatch, KelasGagal, PengirimDataBalikan,
    PengirimTiruan, RekamanKirim,
};
pub use penyusun::{GalatPenyusun, jalankan_job_balikan};
