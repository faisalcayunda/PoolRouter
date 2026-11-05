//! Pembacaan respons provider (ADR-0002 rev 5 D3).
//!
//! Parser dipilih lewat konfigurasi provider. F0 baru punya satu parser,
//! [`status_skor`]: respons Web Service Kesesuaian Dukcapil.

pub mod status_skor;

pub use status_skor::{
    HasilElemen, KonfigurasiStatusSkor, ResponsTidakValid, StatusKesesuaian, parse_respons,
};
