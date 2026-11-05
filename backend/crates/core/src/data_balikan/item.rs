//! Item data balikan yang membawa NIK (AC-19, ADR-0006 §6, D6 "log tanpa
//! rahasia").
//!
//! [`ItemDataBalikan`] tidak punya `Debug`, `Display`, `Serialize`, atau
//! `Clone`. Itu bukan kelalaian: setiap jalur generik yang bisa mencetak nilai
//! (`{:?}` pada struct pembungkus, `tracing` field capture, pesan `.expect()`)
//! lewat `Debug`/`Display`, jadi meniadakan keduanya menutup semuanya sekaligus
//! — sama seperti [`NilaiAman`](crate::admin::NilaiAman), tetapi di sini nilai
//! hanya bisa dibaca lewat [`ItemDataBalikan::nik`] dan
//! [`ItemDataBalikan::field_input`], yang mengembalikan [`NilaiAman`].

use crate::admin::{Nik, NilaiAman};

/// Satu item yang dikirim ke provider untuk satu `request_id`.
///
/// Dibuat hanya oleh penyusun (T5), dipakai hanya oleh pengirim (T6), dan
/// tidak pernah menyeberang proses.
pub struct ItemDataBalikan {
    request_id: uuid::Uuid,
    nik: Nik,
    field_input: serde_json::Value,
}

impl ItemDataBalikan {
    /// Membuat item dari baris permintaan.
    pub fn baru(request_id: uuid::Uuid, nik: Nik, field_input: serde_json::Value) -> Self {
        Self {
            request_id,
            nik,
            field_input,
        }
    }

    /// `request_id` yang datanya diminta provider. Bukan data pribadi, jadi
    /// boleh dibaca bebas.
    pub fn request_id(&self) -> uuid::Uuid {
        self.request_id
    }

    /// NIK, selalu terbungkus [`NilaiAman`] supaya `Display`/`Debug` mencetak
    /// `***`.
    pub fn nik(&self) -> NilaiAman<&str> {
        self.nik.utuh()
    }

    /// Field input persetujuan. Dibungkus [`NilaiAman`] dengan alasan yang
    /// sama seperti NIK: isinya data pribadi pemohon.
    pub fn field_input(&self) -> NilaiAman<&serde_json::Value> {
        NilaiAman::baru(&self.field_input)
    }
}

#[cfg(test)]
mod uji {
    use super::*;

    // NIK sintetis, sesuai aturan repo: data NIK hanya sintetis.
    const NIK: &str = "3201010101010001";

    fn item() -> ItemDataBalikan {
        ItemDataBalikan::baru(
            uuid::Uuid::nil(),
            Nik::parse(NIK).unwrap(),
            serde_json::json!({ "nama": "Budi" }),
        )
    }

    #[test]
    fn request_id_boleh_dibaca() {
        assert_eq!(item().request_id(), uuid::Uuid::nil());
    }

    #[test]
    fn nik_terbungkus_nilai_aman() {
        let i = item();
        let n = i.nik();
        assert_eq!(format!("{n}"), "***");
        assert_eq!(format!("{n:?}"), "NilaiAman(***)");
        // Nilai asli tetap bisa diambil secara eksplisit oleh adapter.
        assert_eq!(*n.expose(), NIK);
    }

    #[test]
    fn field_input_terbungkus_nilai_aman() {
        let i = item();
        let f = i.field_input();
        assert_eq!(format!("{f}"), "***");
        assert_eq!(format!("{f:?}"), "NilaiAman(***)");
        assert_eq!(f.expose()["nama"], "Budi");
    }

    #[test]
    fn nilai_tidak_bocor_lewat_jalur_cetak() {
        // Akses eksplisit lalu dicetak: yang tercetak tetap `***`, bukan NIK.
        let i = item();
        let tercetak = format!("{}/{} {:?}", i.nik(), i.field_input(), i.nik());
        assert!(!tercetak.contains(NIK), "{tercetak}");
        assert!(!tercetak.contains("Budi"), "{tercetak}");
    }

    #[test]
    fn nilai_tidak_bocor_lewat_debug_struct_pembungkus() {
        // Jalur yang sama dengan `tracing` field capture dan `{:?}` pada struct
        // yang memuat item: tiap field dibungkus `NilaiAman`, jadi `Debug`
        // pembungkus tidak memuat nilai.
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Amplop<'a> {
            request_id: uuid::Uuid,
            nik: NilaiAman<&'a str>,
            field_input: NilaiAman<&'a serde_json::Value>,
        }
        let i = item();
        let a = Amplop {
            request_id: i.request_id(),
            nik: i.nik(),
            field_input: i.field_input(),
        };
        let tercetak = format!("{a:?}");
        assert!(!tercetak.contains(NIK), "{tercetak}");
        assert!(!tercetak.contains("Budi"), "{tercetak}");
        assert!(tercetak.contains("***"), "{tercetak}");
    }
}
