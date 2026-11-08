//! Provider secret storage: envelope encryption with AES-256-GCM (ADR-0001
//! section 7; contract R-16; f0-admin-api AD1, AC-8, AC-11).
//!
//! The database never holds a secret value in the clear and never holds the key
//! that could open it. Each value is sealed with its own data key (DEK), and the
//! DEK is sealed with the key encryption key (KEK) that arrives from the
//! environment as a file (`POOLROUTER_KEK_FILE`; production may move to KMS,
//! ADR-0004 T-5). The row keeps, per value: the ciphertext, the wrapped DEK, the
//! nonce, and the `kek_id` that says which KEK opened it.
//!
//! Two properties are enforced here rather than at the call site:
//!
//! - **A fresh DEK and a fresh nonce per write.** Re-sealing the same value twice
//!   gives different bytes; nothing is derived from the previous row.
//! - **The secret name is the associated data.** A ciphertext copied onto another
//!   name does not open, so a rename cannot silently re-point a provider at a
//!   different value.
//!
//! The plaintext only ever exists inside [`NilaiAman`], whose `Debug` and
//! `Display` are `***`, so a log line or a panic cannot print it (AC-6).

use aws_lc_rs::aead::{AES_256_GCM, Aad, NONCE_LEN, Nonce, RandomizedNonceKey};
use aws_lc_rs::rand;
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use sha2::{Digest, Sha256};

use crate::admin::NilaiAman;

/// Length of the KEK in bytes (AES-256).
pub const PANJANG_KEK: usize = 32;

/// Length of the tag AES-256-GCM appends, in bytes.
const PANJANG_TAG: usize = 16;

/// Prefix of every `kek_id` this module writes.
const PREFIX_KEK_ID: &str = "kek-";

/// One encrypted secret value, exactly as the row stores it.
///
/// `dek_terbungkus` is `nonce || ciphertext || tag` for the DEK, so the wrapped
/// key needs no second nonce column; `nonce` belongs to the value itself.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Amplop {
    /// Secret name; also the AES-GCM associated data, so the bytes only open
    /// under the name they were sealed for.
    pub nama: String,
    /// The sealed value, including the GCM tag.
    pub ciphertext: Vec<u8>,
    /// The sealed DEK: 12-byte nonce, then the wrapped 32-byte key, then the tag.
    pub dek_terbungkus: Vec<u8>,
    /// Nonce of the value's own ciphertext (12 bytes).
    pub nonce: [u8; NONCE_LEN],
    /// Which KEK sealed this envelope, e.g. `kek-1f2e3d4c5b6a7988`.
    pub kek_id: String,
}

/// Why an envelope could not be produced or opened. The messages never carry a
/// key, a nonce, or any part of the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GalatBrankas {
    /// The KEK is not 32 bytes, 64 hex characters, or standard base64 of 32 bytes.
    KekTidakValid,
    /// The envelope was sealed with a different KEK (or is corrupt beyond repair).
    KekSalah,
    /// The envelope is malformed: wrong nonce or wrapped-DEK length.
    AmplopRusak,
    /// The bytes did not authenticate: wrong name, tampered ciphertext, or a
    /// different KEK that happened to keep the same length.
    TidakTerbuka,
    /// The system random source refused.
    AcakGagal,
    /// Decrypted bytes are not UTF-8.
    BukanTeks,
}

impl std::fmt::Display for GalatBrankas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let pesan = match self {
            Self::KekTidakValid => "KEK harus 32 byte (64 karakter hex atau base64 32 byte)",
            Self::KekSalah => "KEK tidak cocok dengan kek_id amplop",
            Self::AmplopRusak => "amplop rusak: panjang nonce atau DEK terbungkus salah",
            Self::TidakTerbuka => "nilai tidak dapat dibuka dengan KEK ini",
            Self::AcakGagal => "sumber acak sistem gagal",
            Self::BukanTeks => "nilai terbuka bukan teks UTF-8",
        };
        f.write_str(pesan)
    }
}

impl std::error::Error for GalatBrankas {}

/// The KEK plus the id recorded next to every envelope it seals.
#[derive(Clone)]
pub struct Brankas {
    kek: [u8; PANJANG_KEK],
    kek_id: String,
}

impl std::fmt::Debug for Brankas {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Brankas")
            .field("kek_id", &self.kek_id)
            .finish_non_exhaustive()
    }
}

impl Brankas {
    /// Build from the text of the KEK file. Three forms are accepted because the
    /// file is written by hand in dev and by a secret manager in deploy: 64 hex
    /// characters, standard base64 of 32 bytes, or the 32 raw bytes. Surrounding
    /// whitespace (the trailing newline every secret file has) is ignored.
    pub fn dari_isi_kek(isi: &str) -> Result<Self, GalatBrankas> {
        let teks = isi.trim();
        let kek = if teks.len() == 2 * PANJANG_KEK && teks.bytes().all(|b| b.is_ascii_hexdigit()) {
            let mut buf = [0u8; PANJANG_KEK];
            for (i, pasangan) in teks.as_bytes().as_chunks::<2>().0.iter().enumerate() {
                let tinggi = (pasangan[0] as char)
                    .to_digit(16)
                    .ok_or(GalatBrankas::KekTidakValid)?;
                let rendah = (pasangan[1] as char)
                    .to_digit(16)
                    .ok_or(GalatBrankas::KekTidakValid)?;
                buf[i] =
                    u8::try_from(tinggi * 16 + rendah).map_err(|_| GalatBrankas::KekTidakValid)?;
            }
            buf
        } else if teks.len() == PANJANG_KEK {
            let mut buf = [0u8; PANJANG_KEK];
            buf.copy_from_slice(teks.as_bytes());
            buf
        } else {
            let mentah = BASE64
                .decode(teks)
                .map_err(|_| GalatBrankas::KekTidakValid)?;
            let buf: [u8; PANJANG_KEK] = mentah
                .as_slice()
                .try_into()
                .map_err(|_| GalatBrankas::KekTidakValid)?;
            buf
        };
        Ok(Self {
            kek_id: id_dari_kek(&kek),
            kek,
        })
    }

    pub fn kek_id(&self) -> &str {
        &self.kek_id
    }

    /// Seal `nilai` under `nama`: a fresh DEK, a fresh nonce, both wrapped.
    pub fn enkripsi(&self, nama: &str, nilai: &NilaiAman<String>) -> Result<Amplop, GalatBrankas> {
        // A fresh DEK per write; the value is sealed under it, not under the KEK.
        let mut dek = [0u8; PANJANG_KEK];
        rand::fill(&mut dek).map_err(|_| GalatBrankas::AcakGagal)?;
        let kunci_dek = self.kunci_nilai(&dek)?;
        let mut isi = nilai.expose().as_bytes().to_vec();
        let nonce = kunci_dek
            .seal_in_place_append_tag(Aad::from(nama.as_bytes()), &mut isi)
            .map_err(|_| GalatBrankas::AcakGagal)?;

        let kunci_kek = self.kunci_nilai(&self.kek)?;
        let mut terbungkus = dek.to_vec();
        let nonce_dek = kunci_kek
            .seal_in_place_append_tag(Aad::from(nama.as_bytes()), &mut terbungkus)
            .map_err(|_| GalatBrankas::AcakGagal)?;
        let mut dek_terbungkus = Vec::with_capacity(NONCE_LEN + terbungkus.len());
        dek_terbungkus.extend_from_slice(nonce_dek.as_ref());
        dek_terbungkus.extend_from_slice(&terbungkus);

        let mut nonce_nilai = [0u8; NONCE_LEN];
        nonce_nilai.copy_from_slice(nonce.as_ref());

        Ok(Amplop {
            nama: nama.to_owned(),
            ciphertext: isi,
            dek_terbungkus,
            nonce: nonce_nilai,
            kek_id: self.kek_id.clone(),
        })
    }

    /// Open an envelope. The returned value is the only place the plaintext
    /// exists, and it prints as `***`.
    pub fn dekripsi(&self, amplop: &Amplop) -> Result<NilaiAman<String>, GalatBrankas> {
        if amplop.dek_terbungkus.len() != NONCE_LEN + PANJANG_KEK + PANJANG_TAG {
            return Err(GalatBrankas::AmplopRusak);
        }
        if amplop.kek_id != self.kek_id {
            return Err(GalatBrankas::KekSalah);
        }
        let (nonce_dek, terbungkus) = amplop.dek_terbungkus.split_at(NONCE_LEN);
        let kunci_kek = self.kunci_nilai(&self.kek)?;
        let mut dek_buf = terbungkus.to_vec();
        let dek_buka = kunci_kek
            .open_in_place(
                Nonce::try_assume_unique_for_key(nonce_dek)
                    .map_err(|_| GalatBrankas::AmplopRusak)?,
                Aad::from(amplop.nama.as_bytes()),
                &mut dek_buf,
            )
            .map_err(|_| GalatBrankas::TidakTerbuka)?;

        let kunci_dek = self.kunci_nilai(dek_buka)?;
        let mut isi = amplop.ciphertext.clone();
        let buka = kunci_dek
            .open_in_place(
                Nonce::try_assume_unique_for_key(&amplop.nonce)
                    .map_err(|_| GalatBrankas::AmplopRusak)?,
                Aad::from(amplop.nama.as_bytes()),
                &mut isi,
            )
            .map_err(|_| GalatBrankas::TidakTerbuka)?;
        let teks = String::from_utf8(buka.to_vec()).map_err(|_| GalatBrankas::BukanTeks)?;
        Ok(NilaiAman::baru(teks))
    }

    fn kunci_nilai(&self, kunci: &[u8]) -> Result<RandomizedNonceKey, GalatBrankas> {
        RandomizedNonceKey::new(&AES_256_GCM, kunci).map_err(|_| GalatBrankas::KekTidakValid)
    }
}

/// `kek-` plus the first 8 bytes of SHA-256 over the KEK, hex. Deterministic, so
/// a KEK file swapped by mistake is named differently in the row and shows up as
/// [`GalatBrankas::KekSalah`] instead of a confusing authentication failure.
fn id_dari_kek(kek: &[u8; PANJANG_KEK]) -> String {
    let intisari = Sha256::digest(kek);
    let mut id = String::with_capacity(PREFIX_KEK_ID.len() + 16);
    id.push_str(PREFIX_KEK_ID);
    for b in &intisari[..8] {
        id.push_str(&format!("{b:02x}"));
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Synthetic KEK, never a real one (ARC-35 decision: test data is synthetic).
    fn kek_hex(bita: u8) -> String {
        std::iter::repeat_n(format!("{bita:02x}"), PANJANG_KEK).collect()
    }

    fn brankas(bita: u8) -> Brankas {
        Brankas::dari_isi_kek(&kek_hex(bita)).expect("KEK sintetis valid")
    }

    fn nilai(teks: &str) -> NilaiAman<String> {
        NilaiAman::baru(teks.to_owned())
    }

    #[test]
    fn round_trip_mengembalikan_nilai_yang_sama() {
        let b = brankas(0x11);
        let amplop = b
            .enkripsi("DUKCAPIL_PASSWORD", &nilai("sintetis-rahasia"))
            .expect("enkripsi");
        let buka = b.dekripsi(&amplop).expect("dekripsi");
        assert_eq!(buka.expose(), "sintetis-rahasia");
    }

    #[test]
    fn ciphertext_tidak_memuat_nilai_asli() {
        let b = brankas(0x11);
        let amplop = b
            .enkripsi("KUNCI", &nilai("jangan-terbaca"))
            .expect("enkripsi");
        let mentah = format!("{amplop:?} {}", String::from_utf8_lossy(&amplop.ciphertext));
        assert!(!mentah.contains("jangan-terbaca"));
    }

    #[test]
    fn nonce_dan_ciphertext_berbeda_tiap_tulis() {
        let b = brankas(0x11);
        let a = b.enkripsi("SAMA", &nilai("nilai")).expect("enkripsi");
        let c = b.enkripsi("SAMA", &nilai("nilai")).expect("enkripsi");
        assert_ne!(a.nonce, c.nonce, "nonce harus acak per tulis");
        assert_ne!(a.ciphertext, c.ciphertext);
        assert_ne!(
            a.dek_terbungkus, c.dek_terbungkus,
            "DEK harus acak per tulis"
        );
    }

    #[test]
    fn seribu_tulis_tidak_mengulang_nonce() {
        let b = brankas(0x11);
        let mut terpakai = std::collections::HashSet::new();
        for i in 0..1000 {
            let amplop = b
                .enkripsi("SAMA", &nilai("nilai"))
                .unwrap_or_else(|e| panic!("tulis ke-{i}: {e:?}"));
            assert!(terpakai.insert(amplop.nonce), "nonce terpakai ulang di {i}");
        }
    }

    #[test]
    fn kek_berbeda_menolak_amplop() {
        let amplop = brankas(0x11)
            .enkripsi("KUNCI", &nilai("nilai"))
            .expect("enkripsi");
        let galat = brankas(0x22).dekripsi(&amplop).expect_err("harus ditolak");
        assert_eq!(galat, GalatBrankas::KekSalah);
    }

    #[test]
    fn nama_lain_tidak_membuka() {
        let b = brankas(0x11);
        let mut amplop = b.enkripsi("ASLI", &nilai("nilai")).expect("enkripsi");
        amplop.nama = "PINDAHAN".to_owned();
        assert_eq!(
            b.dekripsi(&amplop).expect_err("harus ditolak"),
            GalatBrankas::TidakTerbuka
        );
    }

    #[test]
    fn ciphertext_dirusak_tidak_membuka() {
        let b = brankas(0x11);
        let mut amplop = b.enkripsi("KUNCI", &nilai("nilai")).expect("enkripsi");
        amplop.ciphertext[0] ^= 0x01;
        assert_eq!(
            b.dekripsi(&amplop).expect_err("harus ditolak"),
            GalatBrankas::TidakTerbuka
        );
    }

    #[test]
    fn amplop_terpotong_ditolak() {
        let b = brankas(0x11);
        let mut amplop = b.enkripsi("KUNCI", &nilai("nilai")).expect("enkripsi");
        amplop.dek_terbungkus.truncate(20);
        assert_eq!(
            b.dekripsi(&amplop).expect_err("harus ditolak"),
            GalatBrankas::AmplopRusak
        );
    }

    #[test]
    fn kek_id_stabil_dan_membedakan_kunci() {
        let a = brankas(0x11);
        assert_eq!(a.kek_id(), brankas(0x11).kek_id(), "kek_id deterministik");
        assert_ne!(a.kek_id(), brankas(0x22).kek_id());
        assert!(a.kek_id().starts_with(PREFIX_KEK_ID));
    }

    #[test]
    fn kek_diterima_dalam_tiga_bentuk() {
        let bita = [0x7fu8; PANJANG_KEK];
        let heks: String = bita.iter().map(|b| format!("{b:02x}")).collect();
        let b64 = BASE64.encode(bita);
        let mentah = String::from_utf8(bita.to_vec()).expect("utf8");
        for isi in [format!("{heks}\n"), format!("{b64}\n"), mentah] {
            let b = Brankas::dari_isi_kek(&isi).expect("bentuk KEK diterima");
            assert_eq!(
                b.kek_id(),
                Brankas::dari_isi_kek(&heks).expect("hex").kek_id()
            );
        }
    }

    #[test]
    fn kek_panjang_salah_ditolak() {
        for isi in ["", "abc", &kek_hex(0x11)[..62]] {
            assert_eq!(
                Brankas::dari_isi_kek(isi).expect_err("harus ditolak"),
                GalatBrankas::KekTidakValid
            );
        }
    }

    #[test]
    fn debug_brankas_tidak_memuat_kunci() {
        let teks = kek_hex(0x5a);
        let b = Brankas::dari_isi_kek(&teks).expect("KEK valid");
        assert!(!format!("{b:?}").contains("5a5a"));
    }
}
