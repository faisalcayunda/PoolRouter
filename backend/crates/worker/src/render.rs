//! Dokumen konfigurasi provider dan render template ganda (ADR-0001 bagian 7,
//! ADR-0002 D1–D3).
//!
//! Isinya dipindah tanpa perubahan ke `poolrouter_adapter_provider::dokumen`
//! (ARC-66) supaya jalur sync webserver memakai kode yang sama. Modul ini
//! tinggal re-export agar path `crate::render::*` di worker tetap berlaku.

pub use poolrouter_adapter_provider::dokumen::*;
