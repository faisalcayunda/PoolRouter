//! Object keys and the DEK wrapper (DA10, AC-16).
//!
//! The object is sealed with a random data key (DEK) per object, and that DEK is
//! wrapped by a [`PembungkusDek`] — in production the KEK from secret storage
//! (ARC-48), in tests a fixed key. Two rules are enforced by construction:
//!
//! - **The raw DEK never leaves this module.** [`Dek`] has no accessor; the
//!   writer and reader live in this crate and use it, and everything else only
//!   sees [`DekTerbungkus`], the wrapped bytes.
//! - **No DEK is ever formatted.** `Debug` prints a fixed marker, so a
//!   `{:?}` on the writer parameters cannot put key material in a log.
//!
//! The DEK is generated through `aws_lc_rs::rand::SystemRandom` to match
//! [`crate::secret`], which ADR-0004 §4.1 requires instead of the `rand` crate.

use std::fmt;

use aws_lc_rs::aead::{AES_256_GCM, Aad, Nonce, RandomizedNonceKey};
use aws_lc_rs::rand::{SecureRandom, SystemRandom};
use zeroize::Zeroize;

use crate::arsip::galat::GalatArsip;

/// Length of a DEK in bytes (AES-256).
pub const PANJANG_DEK: usize = 32;

/// Length of the nonce AES-256-GCM uses, in bytes.
pub const PANJANG_NONCE: usize = 12;

/// Length of the authentication tag, in bytes.
pub const PANJANG_TAG: usize = 16;

/// A data key. Opaque on purpose: no accessor, and `Drop` wipes it.
pub struct Dek([u8; PANJANG_DEK]);

impl Dek {
    /// A fresh random DEK from the system CSPRNG.
    pub fn acak() -> Result<Self, GalatArsip> {
        let mut k = [0u8; PANJANG_DEK];
        SystemRandom::new()
            .fill(&mut k)
            .map_err(|_| GalatArsip::EnkripsiTidakUtuh("dek"))?;
        Ok(Self(k))
    }

    /// Rebuild a DEK from the bytes a [`PembungkusDek`] just unwrapped.
    ///
    /// Crate-visible so an implementor of the trait outside this module cannot
    /// mint a `Dek` from arbitrary bytes.
    pub(crate) fn dari_bytes(bytes: [u8; PANJANG_DEK]) -> Self {
        Self(bytes)
    }

    /// The raw key, for the AEAD call in this module only.
    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    fn kunci(&self) -> Result<RandomizedNonceKey, GalatArsip> {
        RandomizedNonceKey::new(&AES_256_GCM, &self.0)
            .map_err(|_| GalatArsip::EnkripsiTidakUtuh("dek"))
    }
}

impl fmt::Debug for Dek {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Dek(***)")
    }
}

impl Drop for Dek {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

/// A DEK sealed by the key encryption key, exactly as the object header stores
/// it.
///
/// `bytes` is `nonce || ciphertext || tag` for the DEK, so no second nonce column
/// is needed; the shape matches [`crate::secret::Amplop::dek_terbungkus`].
#[derive(Clone, PartialEq, Eq)]
pub struct DekTerbungkus(Vec<u8>);

impl DekTerbungkus {
    /// Wrap already-produced wrapping bytes. Only a [`PembungkusDek`] produces
    /// these, but the reader has to rebuild the type from the object, so the
    /// constructor is public and cheap.
    pub fn dari_bytes(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    /// The wrapped bytes, for storing in the object header.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// `true` when the value is too short to be `nonce || ciphertext || tag`,
    /// which means the header was truncated or never written.
    pub fn kosong(&self) -> bool {
        self.0.len() < PANJANG_NONCE + PANJANG_DEK + PANJANG_TAG
    }
}

impl fmt::Debug for DekTerbungkus {
    /// Prints the length only: the wrapped DEK is still key material for anyone
    /// holding the KEK.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DekTerbungkus({} byte)", self.0.len())
    }
}

/// Wraps and unwraps the per-object DEK with the KEK.
///
/// The trait exists so the archive module does not depend on how the KEK is
/// stored. Production uses the secret storage KEK (ARC-48); tests use a fixed
/// key. A wrong wrapper answers [`GalatArsip::Pembungkus`] with no key material
/// in the message.
pub trait PembungkusDek: Send + Sync {
    /// Seal the DEK for storage in the object header.
    fn bungkus(&self, dek: &Dek) -> Result<DekTerbungkus, GalatArsip>;

    /// Open a wrapped DEK. A wrong KEK or a changed byte answers
    /// [`GalatArsip::Pembungkus`].
    fn buka(&self, terbungkus: &DekTerbungkus) -> Result<Dek, GalatArsip>;

    /// Identifier of the KEK, recorded so a rotation can tell which key opened
    /// an object. Same shape as [`crate::secret::Amplop::kek_id`].
    fn kek_id(&self) -> &str;
}

/// A [`PembungkusDek`] over a fixed 32-byte KEK.
///
/// Used by the job in T4 when the KEK file has already been read, and by tests.
/// It is not itself a source of the KEK: it never reads the environment.
pub struct PembungkusKekAwal {
    kunci: RandomizedNonceKey,
    kek_id: String,
}

impl PembungkusKekAwal {
    /// Build from a 32-byte KEK. `kek_id` is carried through unchanged; pass
    /// [`crate::secret::kek_id`] to match the rest of the product.
    pub fn baru(kek: &[u8; PANJANG_DEK], kek_id: impl Into<String>) -> Result<Self, GalatArsip> {
        let kunci = RandomizedNonceKey::new(&AES_256_GCM, kek)
            .map_err(|_| GalatArsip::Pembungkus("KEK tidak valid".to_owned()))?;
        Ok(Self {
            kunci,
            kek_id: kek_id.into(),
        })
    }

    /// A deterministic test KEK. Not for production: the bytes are public.
    #[cfg(test)]
    pub fn uji(label: &str) -> Self {
        use sha2::Digest as _;
        let mut kek = [0u8; PANJANG_DEK];
        let d = sha2::Sha256::digest(label.as_bytes());
        kek.copy_from_slice(&d);
        Self::baru(&kek, format!("kek-uji-{label}")).unwrap()
    }
}

impl fmt::Debug for PembungkusKekAwal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "PembungkusKekAwal({})", self.kek_id)
    }
}

impl PembungkusDek for PembungkusKekAwal {
    fn bungkus(&self, dek: &Dek) -> Result<DekTerbungkus, GalatArsip> {
        // AAD binds the wrapped DEK to this context, so bytes lifted out of one
        // object cannot be replayed into another header by accident.
        let aad = Aad::from(b"poolrouter-arsip-dek".as_slice());
        let mut cipher = dek.0.to_vec();
        let (nonce, tag) = self
            .kunci
            .seal_in_place_separate_tag(aad, &mut cipher)
            .map_err(|_| GalatArsip::Pembungkus("gagal membungkus DEK".to_owned()))?;
        let mut out = Vec::with_capacity(PANJANG_NONCE + PANJANG_DEK + PANJANG_TAG);
        out.extend_from_slice(nonce.as_ref());
        out.extend_from_slice(&cipher);
        out.extend_from_slice(tag.as_ref());
        Ok(DekTerbungkus(out))
    }

    fn buka(&self, terbungkus: &DekTerbungkus) -> Result<Dek, GalatArsip> {
        if terbungkus.kosong() {
            return Err(GalatArsip::Pembungkus(
                "DEK terbungkus terpotong".to_owned(),
            ));
        }
        let bytes = terbungkus.as_bytes();
        let (nonce, ciphertext_tag) = bytes.split_at(PANJANG_NONCE);
        let nonce = Nonce::try_assume_unique_for_key(nonce)
            .map_err(|_| GalatArsip::Pembungkus("nonce DEK tidak valid".to_owned()))?;
        // `open_in_place` wants ciphertext||tag in one buffer.
        let mut buf = ciphertext_tag.to_vec();
        let aad = Aad::from(b"poolrouter-arsip-dek".as_slice());
        let plain = self
            .kunci
            .open_in_place(nonce, aad, &mut buf)
            .map_err(|_| GalatArsip::Pembungkus("KEK tidak membuka DEK".to_owned()))?;
        let mut k = [0u8; PANJANG_DEK];
        if plain.len() != PANJANG_DEK {
            buf.zeroize();
            return Err(GalatArsip::Pembungkus(
                "DEK terbungkus salah panjang".to_owned(),
            ));
        }
        k.copy_from_slice(plain);
        buf.zeroize();
        Ok(Dek::dari_bytes(k))
    }

    fn kek_id(&self) -> &str {
        &self.kek_id
    }
}

/// Where one archive object lives.
///
/// The key is deterministic per unit (DA9: table, provider, window) so a
/// retried job overwrites its own object instead of leaving a second copy
/// (AC-19). The shape is fixed here so T4 and T7 agree without a second
/// definition.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct KunciObjek(String);

impl KunciObjek {
    /// Build the key for one archive unit:
    /// `arsip/<tabel>/provider=<provider_id>/tanggal=<YYYY-MM-DD>/<rentang>.parquet`.
    ///
    /// `provider_id` and `tanggal` are caller-formatted parts, never raw input:
    /// the type does not parse them, it only refuses an empty part so a missing
    /// provider cannot silently produce `provider=/`.
    pub fn baru(
        tabel: &str,
        provider_id: &str,
        tanggal: &str,
        rentang: &str,
    ) -> Result<Self, GalatArsip> {
        for (nama, v) in [
            ("tabel", tabel),
            ("provider_id", provider_id),
            ("tanggal", tanggal),
            ("rentang", rentang),
        ] {
            if v.is_empty() {
                return Err(GalatArsip::SkemaKosong);
            }
            // A `/` or `..` in a part would escape the unit's own prefix and let
            // one unit overwrite another's object.
            if v.contains('/') || v.contains("..") {
                return Err(GalatArsip::Penyimpanan(format!(
                    "kunci objek: bagian {nama} tidak boleh memuat '/' atau '..'"
                )));
            }
        }
        Ok(Self(format!(
            "arsip/{tabel}/provider={provider_id}/tanggal={tanggal}/{rentang}.parquet"
        )))
    }

    /// The key as the port and the SDK see it.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// A key that names no object, used as the placeholder in a
    /// [`super::RingkasanObjek`] returned before the caller knows where it puts
    /// the bytes. It never reaches the port: [`super::TulisArsip::tulis_ke`]
    /// replaces it, and a caller that stores the placeholder would fail loudly
    /// on a key with a leading `/`.
    pub fn placeholder() -> Self {
        Self("/belum-ditempatkan".to_owned())
    }
}

impl fmt::Display for KunciObjek {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dek_tidak_tampil_di_debug() {
        let dek = Dek::acak().unwrap();
        assert_eq!(format!("{dek:?}"), "Dek(***)");
    }

    #[test]
    fn bungkus_buka_bolak_balik() {
        let p = PembungkusKekAwal::uji("satu");
        let dek = Dek::acak().unwrap();
        let asli = dek.0;
        let t = p.bungkus(&dek).unwrap();
        assert!(!t.kosong());
        // The wrapped form is not the raw DEK.
        assert_ne!(t.as_bytes(), &asli);
        assert!(!t.as_bytes().windows(32).any(|w| w == asli));
        let kembali = p.buka(&t).unwrap();
        assert_eq!(kembali.0, asli);
    }

    #[test]
    fn kek_salah_tidak_membuka() {
        let a = PembungkusKekAwal::uji("a");
        let b = PembungkusKekAwal::uji("b");
        let t = a.bungkus(&Dek::acak().unwrap()).unwrap();
        let e = b.buka(&t).unwrap_err();
        assert!(matches!(e, GalatArsip::Pembungkus(_)));
    }

    #[test]
    fn dek_terbungkus_terpotong_ditolak() {
        let p = PembungkusKekAwal::uji("a");
        let t = DekTerbungkus::dari_bytes(vec![0u8; 10]);
        assert!(t.kosong());
        assert!(p.buka(&t).is_err());
    }

    #[test]
    fn kunci_objek_deterministik_dan_menolak_pelarian() {
        let a = KunciObjek::baru("transaction_log", "p1", "2026-11-05", "00-06").unwrap();
        let b = KunciObjek::baru("transaction_log", "p1", "2026-11-05", "00-06").unwrap();
        assert_eq!(a, b);
        assert!(a.as_str().starts_with("arsip/transaction_log/provider=p1/"));
        assert!(KunciObjek::baru("t", "", "d", "r").is_err());
        assert!(KunciObjek::baru("t", "../x", "d", "r").is_err());
        assert!(KunciObjek::baru("t", "p", "2026/11/05", "r").is_err());
    }
}
