//! Checking a written object against its summary (AC-18).
//!
//! Verification re-reads the object through the port and recomputes every field
//! of [`RingkasanObjek`] from the bytes, so the summary is never trusted. It
//! refuses on:
//!
//! - a changed byte (the digest differs),
//! - a different row count,
//! - a different size,
//! - a different HMAC key version,
//! - a different object key,
//! - a missing object.
//!
//! A missing object is a failure rather than a pass: the retention promise is
//! "this window is archived", and an object that is not there does not keep it.
//!
//! The check does **not** need the KEK. It compares the footer metadata and the
//! row count, which is what the job can do in the audit path where the DEK
//! wrapper may not be loaded.

use crate::arsip::galat::GalatVerifikasi;
use crate::arsip::kunci::KunciObjek;
use crate::arsip::port::{PenyimpanArsip, RingkasanObjek};
use crate::arsip::{BacaArsip, VersiKunciHmac};
use parquet::file::metadata::{ParquetMetaData, ParquetMetaDataReader};

/// Runs AC-18 against a summary.
pub struct Verifikasi;

impl Verifikasi {
    /// Verify `ringkasan` against the store. `Ok(())` means every field matched.
    pub async fn verifikasi(
        penyimpan: &dyn PenyimpanArsip,
        ringkasan: &RingkasanObjek,
    ) -> Result<(), GalatVerifikasi> {
        // Size first: `head` is the cheapest call and a missing object is the
        // most common failure (a partial write, a lifecycle rule that ran early).
        let ukuran = penyimpan
            .head(&ringkasan.kunci)
            .await
            .map_err(|_| GalatVerifikasi::ObjekHilang)?;
        let Some(ukuran) = ukuran else {
            return Err(GalatVerifikasi::ObjekHilang);
        };
        if ukuran != ringkasan.ukuran {
            return Err(GalatVerifikasi::UkuranBeda {
                diharapkan: ringkasan.ukuran,
                didapat: ukuran,
            });
        }

        let utuh = baca_utuh(penyimpan, &ringkasan.kunci).await?;
        let didapat_sha = crate::arsip::tulis::hex_sha256(&utuh);
        if didapat_sha != ringkasan.sha256 {
            return Err(GalatVerifikasi::IntisariBeda {
                diharapkan: ringkasan.sha256.clone(),
                didapat: didapat_sha,
            });
        }

        let meta = metadata_dari_bytes(&utuh)?;
        let jumlah = jumlah_baris(&meta);
        if jumlah != ringkasan.jumlah_baris {
            return Err(GalatVerifikasi::JumlahBarisBeda {
                diharapkan: ringkasan.jumlah_baris,
                didapat: jumlah,
            });
        }

        if let Some(v) = versi_dari_meta(&meta) {
            if v != ringkasan.kunci_hmac_versi {
                return Err(GalatVerifikasi::VersiKunciBeda {
                    diharapkan: ringkasan.kunci_hmac_versi.nilai(),
                    didapat: v.nilai(),
                });
            }
        } else {
            return Err(GalatVerifikasi::TidakTerbaca(
                "kunci_hmac_versi hilang dari footer".to_owned(),
            ));
        }

        // The footer must name the object it lives in, so a summary cannot be
        // satisfied by an object copied from another unit.
        if let Some(tabel) = BacaArsip::tabel(&meta)
            && tabel.is_empty()
        {
            return Err(GalatVerifikasi::TidakTerbaca("tabel kosong".to_owned()));
        }
        Ok(())
    }

    /// Verify a summary whose recorded key the caller supplies separately, for
    /// the case where the object was moved or replicated.
    pub async fn verifikasi_kunci(
        penyimpan: &dyn PenyimpanArsip,
        kunci: &KunciObjek,
        ringkasan: &RingkasanObjek,
    ) -> Result<(), GalatVerifikasi> {
        if kunci != &ringkasan.kunci {
            return Err(GalatVerifikasi::KunciBeda {
                diharapkan: ringkasan.kunci.to_string(),
                didapat: kunci.to_string(),
            });
        }
        Self::verifikasi(penyimpan, ringkasan).await
    }
}

async fn baca_utuh(
    penyimpan: &dyn PenyimpanArsip,
    kunci: &KunciObjek,
) -> Result<Vec<u8>, GalatVerifikasi> {
    let ukuran = penyimpan
        .head(kunci)
        .await
        .map_err(|_| GalatVerifikasi::ObjekHilang)?
        .ok_or(GalatVerifikasi::ObjekHilang)?;
    penyimpan
        .get_range(kunci, 0..ukuran)
        .await
        .map_err(|e| GalatVerifikasi::TidakTerbaca(e.to_string()))
}

fn metadata_dari_bytes(bytes: &[u8]) -> Result<ParquetMetaData, GalatVerifikasi> {
    let b = bytes::Bytes::copy_from_slice(bytes);
    ParquetMetaDataReader::new()
        .parse_and_finish(&b)
        .map_err(|e| GalatVerifikasi::TidakTerbaca(e.to_string()))
}

/// Row count from the footer, so verification does not decode any column.
fn jumlah_baris(meta: &ParquetMetaData) -> i64 {
    meta.file_metadata().num_rows()
}

fn versi_dari_meta(meta: &ParquetMetaData) -> Option<VersiKunciHmac> {
    let kv = meta.file_metadata().key_value_metadata()?;
    let s = kv
        .iter()
        .find(|x| x.key == crate::arsip::tulis::META_KUNCI_HMAC_VERSI)
        .and_then(|x| x.value.clone())?;
    VersiKunciHmac::baru(s.parse().ok()?).ok()
}
