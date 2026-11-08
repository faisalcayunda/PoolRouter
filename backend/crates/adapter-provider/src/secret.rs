//! Resolve a provider secret from `provider_secret` (ARC-48 AD1/AD2, AC-23).
//!
//! The row holds the envelope only; the plaintext exists after
//! [`poolrouter_core::secret::Brankas::dekripsi`] and stays inside
//! [`NilaiAman`], whose `Debug`/`Display` are `***`. Nothing here logs the
//! value, puts it in an error, or returns it outside the guard.
//!
//! The pool must be the `pr_api` role: the grants let it read the envelope
//! columns, and only it and `pr_worker` decrypt in process.

use poolrouter_core::admin::NilaiAman;
use poolrouter_core::secret::{Amplop, Brankas, GalatBrankas};
use sqlx::PgPool;

/// Why a secret could not be resolved. Messages carry the name, never a value.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GalatSecret {
    #[error("secret {0} tidak ada di provider_secret")]
    TidakAda(String),
    #[error("amplop secret rusak: {0}")]
    AmplopRusak(String),
    #[error("secret tidak dapat dibuka: {0}")]
    Brankas(GalatBrankas),
    #[error("gagal membaca provider_secret: {0}")]
    Db(String),
}

impl From<sqlx::Error> for GalatSecret {
    fn from(e: sqlx::Error) -> Self {
        Self::Db(e.to_string())
    }
}

impl From<GalatBrankas> for GalatSecret {
    fn from(e: GalatBrankas) -> Self {
        Self::Brankas(e)
    }
}

/// Read and open one secret by name.
pub async fn resolusi_secret(
    pool: &PgPool,
    brankas: &Brankas,
    nama: &str,
) -> Result<NilaiAman<String>, GalatSecret> {
    type Baris = (Vec<u8>, Vec<u8>, Vec<u8>, String);
    let baris: Option<Baris> = sqlx::query_as(
        "SELECT secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id \
         FROM provider_secret WHERE nama_secret = $1",
    )
    .bind(nama)
    .fetch_optional(pool)
    .await?;

    let (ciphertext, dek_terbungkus, nonce, kek_id) =
        baris.ok_or_else(|| GalatSecret::TidakAda(nama.to_owned()))?;
    let nonce: [u8; 12] = nonce
        .try_into()
        .map_err(|_| GalatSecret::AmplopRusak("nonce bukan 12 byte".to_owned()))?;

    let amplop = Amplop {
        nama: nama.to_owned(),
        ciphertext,
        dek_terbungkus,
        nonce,
        kek_id,
    };
    Ok(brankas.dekripsi(&amplop)?)
}
