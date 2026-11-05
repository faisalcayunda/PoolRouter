//! Writing one archive object (DA9, DA10, AC-15, AC-16).
//!
//! The writer takes rows in the clear, sorts them by `nik_hmac` then
//! `kode_transaksi` (the format's order, DA9), seals `nik` and the payload with
//! AES-256-GCM under one random DEK, and writes a Parquet object with zstd
//! compression, 128.000-row groups, and a bloom filter on `nik_hmac` only.
//!
//! Two things about the sealing constrain the column layout:
//!
//! - **Sealing is per cell, not per file.** DA10 says application-level sealing
//!   with AES-256-GCM and the `parquet` `encryption` feature is off, so a sealed
//!   cell is a `BYTE_ARRAY` of `nonce || ciphertext || tag`. That keeps the
//!   object readable by any Parquet reader that only needs `nik_hmac`, which is
//!   what makes the range read of AC-17 possible.
//! - **The AAD is the row's own identity.** `nik_hmac` (hex) plus the column name
//!   is the associated data, so a sealed cell cannot be moved to another row or
//!   to the other sealed column without the tag failing.
//!
//! The DEK never leaves the module: it is wrapped through the caller's
//! [`PembungkusDek`] and only the wrapped form goes into the footer metadata.

use std::collections::BTreeMap;
use std::sync::Arc;

use arrow::array::{ArrayRef, BinaryBuilder, Int32Array, StringArray, TimestampMicrosecondArray};
use arrow::datatypes::{DataType, Field, Schema, TimeUnit};
use arrow::record_batch::RecordBatch;
use aws_lc_rs::aead::{Aad, RandomizedNonceKey};
use base64::Engine as _;
use parquet::arrow::ArrowWriter;
use parquet::basic::Compression;
use parquet::file::metadata::{KeyValue, SortingColumn};
use parquet::file::properties::{BloomFilterProperties, EnabledStatistics, WriterProperties};
use parquet::schema::types::ColumnPath;
use sha2::{Digest, Sha256};

use crate::arsip::galat::GalatArsip;
use crate::arsip::kunci::{Dek, PANJANG_NONCE, PANJANG_TAG, PembungkusDek};
use crate::arsip::port::RingkasanObjek;
use crate::arsip::{
    BARIS_PER_ROW_GROUP, BarisArsip, KOLOM_BLOOM, KunciObjek, RentangKodeTransaksi, VersiKunciHmac,
};

/// `kode_transaksi`: the sort tiebreaker.
pub const KOLOM_KODE_TRANSAKSI: &str = "kode_transaksi";
/// `HMAC-SHA256(nik)`, lowercase hex; the bloom filter column.
pub const KOLOM_NIK_HMAC: &str = KOLOM_BLOOM;
/// Key version of the HMAC above.
pub const KOLOM_KUNCI_HMAC_VERSI: &str = "kunci_hmac_versi";
/// Sealed NIK: `nonce || ciphertext || tag`.
pub const KOLOM_NIK: &str = "nik";
/// Sealed payload: `nonce || ciphertext || tag`.
pub const KOLOM_PAYLOAD: &str = "payload";
/// Request time, in the clear.
pub const KOLOM_TERJADI_AT: &str = "terjadi_at";

/// Footer marker: this object was written by this module.
pub const META_FORMAT: &str = "poolrouter.arsip.format";
/// Footer: version of the format.
pub const META_VERSI: &str = "poolrouter.arsip.versi";
/// Footer: the wrapped DEK, base64.
pub const META_DEK: &str = "poolrouter.arsip.dek_terbungkus";
/// Footer: `kek_id` of the wrapper.
pub const META_KEK_ID: &str = "poolrouter.arsip.kek_id";
/// Footer: HMAC key version.
pub const META_KUNCI_HMAC_VERSI: &str = "poolrouter.arsip.kunci_hmac_versi";
/// Footer: row count.
pub const META_JUMLAH_BARIS: &str = "poolrouter.arsip.jumlah_baris";
/// Footer: source table.
pub const META_TABEL: &str = "poolrouter.arsip.tabel";

/// Current value of [`META_VERSI`].
pub const VERSI_FORMAT: &str = "1";

/// What to write besides the fixed bookkeeping columns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParameterTulis {
    /// Source table, recorded in the footer.
    pub tabel: String,
}

impl ParameterTulis {
    /// The `transaction_log` shape.
    pub fn transaction_log() -> Self {
        Self {
            tabel: "transaction_log".to_owned(),
        }
    }

    /// Any other table.
    pub fn baru(tabel: impl Into<String>) -> Self {
        Self {
            tabel: tabel.into(),
        }
    }
}

/// Seals rows into one Parquet object.
///
/// A stateless marker type: keeping no state means a retried job cannot reuse a
/// DEK from a previous attempt.
pub struct TulisArsip;

impl TulisArsip {
    /// Seal `baris` into the bytes of one object and describe it.
    ///
    /// The returned [`RingkasanObjek`] has a placeholder key; the caller sets it
    /// with [`TulisArsip::tulis_ke`] or by assigning `ringkasan.kunci`, because
    /// the writer does not decide where the object goes.
    pub async fn tulis(
        baris: impl IntoIterator<Item = BarisArsip>,
        par: &ParameterTulis,
        pembungkus: &dyn PembungkusDek,
    ) -> Result<(Vec<u8>, RingkasanObjek), GalatArsip> {
        let mut baris: Vec<BarisArsip> = baris.into_iter().collect();
        // Sorted here rather than trusted from the caller: a caller streaming
        // from PostgreSQL without ORDER BY would otherwise write an object the
        // format says cannot exist, and the bloom filter's value depends on the
        // row groups being ordered.
        baris.sort_by(|a, b| a.kunci_urut().cmp(&b.kunci_urut()));

        let versi = match baris.first() {
            Some(p) => {
                let versi = p.kunci_hmac_versi;
                if baris.iter().any(|b| b.kunci_hmac_versi != versi) {
                    // One object carries one key version: restore picks the key
                    // from the summary, and a mixed object would make that pick
                    // wrong for half the rows.
                    return Err(GalatArsip::KunciHmacTidakTersedia(versi.nilai()));
                }
                versi
            }
            None => VersiKunciHmac::F1,
        };

        let dek = Dek::acak()?;
        let dek_terbungkus = pembungkus.bungkus(&dek)?;
        let skema = skema();
        let batch = bangun_batch(&baris, &dek, &skema)?;

        let mut meta = BTreeMap::new();
        meta.insert(META_FORMAT.to_owned(), "parquet".to_owned());
        meta.insert(META_VERSI.to_owned(), VERSI_FORMAT.to_owned());
        meta.insert(META_KEK_ID.to_owned(), pembungkus.kek_id().to_owned());
        meta.insert(META_KUNCI_HMAC_VERSI.to_owned(), versi.nilai().to_string());
        meta.insert(META_JUMLAH_BARIS.to_owned(), baris.len().to_string());
        meta.insert(META_TABEL.to_owned(), par.tabel.clone());
        // Base64 so the footer stays printable ASCII: a tool that trims or
        // escapes metadata cannot corrupt raw key bytes.
        meta.insert(
            META_DEK.to_owned(),
            base64_encode(dek_terbungkus.as_bytes()),
        );

        let props = properti_tulis()
            .set_key_value_metadata(Some(
                meta.into_iter().map(|(k, v)| KeyValue::new(k, v)).collect(),
            ))
            .set_created_by(format!("poolrouter-core arsip v{VERSI_FORMAT}"))
            .build();

        let mut buf: Vec<u8> = Vec::new();
        {
            let mut w = ArrowWriter::try_new(&mut buf, Arc::new(skema), Some(props))?;
            if batch.num_rows() > 0 {
                w.write(&batch)?;
            }
            w.close()?;
        }

        let ringkasan = RingkasanObjek {
            kunci: KunciObjek::placeholder(),
            sha256: hex_sha256(&buf),
            jumlah_baris: baris.len() as i64,
            ukuran: buf.len() as u64,
            rentang_kode_transaksi: RentangKodeTransaksi::dari_baris(&baris),
            kunci_hmac_versi: versi,
        };
        Ok((buf, ringkasan))
    }

    /// [`Self::tulis`], then `put` through the port. Returns the summary with the
    /// key filled in.
    pub async fn tulis_ke(
        penyimpan: &dyn crate::arsip::PenyimpanArsip,
        kunci: &KunciObjek,
        baris: impl IntoIterator<Item = BarisArsip>,
        par: &ParameterTulis,
        pembungkus: &dyn PembungkusDek,
    ) -> Result<RingkasanObjek, GalatArsip> {
        let (isi, mut ringkasan) = Self::tulis(baris, par, pembungkus).await?;
        penyimpan.put(kunci, isi).await?;
        ringkasan.kunci = kunci.clone();
        Ok(ringkasan)
    }
}

/// The fixed Parquet schema. `nik` and `payload` are optional so a table
/// without them (T5) writes nulls rather than a second schema.
pub fn skema() -> Schema {
    Schema::new(vec![
        Field::new(KOLOM_KODE_TRANSAKSI, DataType::Utf8, false),
        Field::new(KOLOM_NIK_HMAC, DataType::Utf8, false),
        Field::new(KOLOM_KUNCI_HMAC_VERSI, DataType::Int32, false),
        Field::new(KOLOM_NIK, DataType::Binary, true),
        Field::new(KOLOM_PAYLOAD, DataType::Binary, true),
        Field::new(
            KOLOM_TERJADI_AT,
            DataType::Timestamp(TimeUnit::Microsecond, Some("UTC".into())),
            false,
        ),
    ])
}

/// zstd, 128.000 rows per group, bloom filter on `nik_hmac` only, sorted by
/// `nik_hmac` then `kode_transaksi` (DA9).
fn properti_tulis() -> parquet::file::properties::WriterPropertiesBuilder {
    let kolom_bloom = ColumnPath::new(vec![KOLOM_NIK_HMAC.to_owned()]);
    WriterProperties::builder()
        .set_compression(Compression::ZSTD(Default::default()))
        .set_max_row_group_row_count(Some(BARIS_PER_ROW_GROUP))
        // Chunk statistics are what let a reader skip a row group by
        // `nik_hmac` range as well as by bloom filter.
        .set_statistics_enabled(EnabledStatistics::Chunk)
        // Bloom filter: off globally, on for `nik_hmac` only (DA9).
        .set_bloom_filter_enabled(false)
        .set_column_bloom_filter_enabled(kolom_bloom.clone(), true)
        .set_column_bloom_filter_properties(kolom_bloom, BloomFilterProperties::default())
        // The leaf indexes are: 0 = kode_transaksi, 1 = nik_hmac. Both are
        // required, so the indexes are stable against an optional field being
        // added later.
        .set_sorting_columns(Some(vec![
            SortingColumn {
                column_idx: 1,
                descending: false,
                nulls_first: false,
            },
            SortingColumn {
                column_idx: 0,
                descending: false,
                nulls_first: false,
            },
        ]))
}

fn bangun_batch(
    baris: &[BarisArsip],
    dek: &Dek,
    skema: &Schema,
) -> Result<RecordBatch, GalatArsip> {
    let kode: StringArray = baris
        .iter()
        .map(|b| Some(b.kode_transaksi.as_str()))
        .collect();
    let hmac: StringArray = baris.iter().map(|b| Some(b.nik_hmac.as_str())).collect();
    let versi: Int32Array = baris
        .iter()
        .map(|b| Some(b.kunci_hmac_versi.nilai()))
        .collect();

    let mut nik = BinaryBuilder::new();
    let mut payload = BinaryBuilder::new();
    for b in baris {
        match &b.nik {
            Some(v) => nik.append_value(segel(
                dek,
                v.expose().utuh().expose().as_bytes(),
                b.nik_hmac.as_str(),
                KOLOM_NIK,
            )?),
            None => nik.append_null(),
        }
        match &b.payload {
            Some(v) => payload.append_value(segel(
                dek,
                v.expose().as_bytes(),
                b.nik_hmac.as_str(),
                KOLOM_PAYLOAD,
            )?),
            None => payload.append_null(),
        }
    }

    let terjadi: TimestampMicrosecondArray = baris
        .iter()
        .map(|b| Some(b.terjadi_at.timestamp_micros()))
        .collect();

    let kolom: Vec<ArrayRef> = vec![
        Arc::new(kode),
        Arc::new(hmac),
        Arc::new(versi),
        Arc::new(nik.finish()),
        Arc::new(payload.finish()),
        Arc::new(terjadi.with_timezone("UTC")),
    ];
    Ok(RecordBatch::try_new(Arc::new(skema.clone()), kolom)?)
}

/// Seal one cell into `nonce || ciphertext || tag`.
///
/// `nik_hmac` and the column name are the associated data, so a cell cannot be
/// moved to another row or the other sealed column without the tag failing.
pub(crate) fn segel(
    dek: &Dek,
    plaintext: &[u8],
    nik_hmac: &str,
    kolom: &'static str,
) -> Result<Vec<u8>, GalatArsip> {
    let kunci = RandomizedNonceKey::new(&aws_lc_rs::aead::AES_256_GCM, dek.as_bytes())
        .map_err(|_| GalatArsip::EnkripsiTidakUtuh(kolom))?;
    let aad = aad_cell(nik_hmac, kolom);
    let mut cipher: Vec<u8> = plaintext.to_vec();
    let (nonce, tag) = kunci
        .seal_in_place_separate_tag(Aad::from(aad.as_slice()), &mut cipher)
        .map_err(|_| GalatArsip::EnkripsiTidakUtuh(kolom))?;

    let mut out = Vec::with_capacity(PANJANG_NONCE + cipher.len() + PANJANG_TAG);
    out.extend_from_slice(nonce.as_ref());
    out.extend_from_slice(&cipher);
    out.extend_from_slice(tag.as_ref());
    Ok(out)
}

/// Open a cell sealed by [`segel`]. A wrong DEK or a changed byte fails.
pub(crate) fn buka_cell(
    dek: &Dek,
    cell: &[u8],
    nik_hmac: &str,
    kolom: &'static str,
) -> Result<Vec<u8>, GalatArsip> {
    if cell.len() < PANJANG_NONCE + PANJANG_TAG {
        return Err(GalatArsip::EnkripsiTidakUtuh(kolom));
    }
    let kunci = RandomizedNonceKey::new(&aws_lc_rs::aead::AES_256_GCM, dek.as_bytes())
        .map_err(|_| GalatArsip::EnkripsiTidakUtuh(kolom))?;
    let (nonce, cipher_tag) = cell.split_at(PANJANG_NONCE);
    let nonce = aws_lc_rs::aead::Nonce::try_assume_unique_for_key(nonce)
        .map_err(|_| GalatArsip::DekSalah)?;
    let aad = aad_cell(nik_hmac, kolom);
    // `open_in_place` wants ciphertext||tag in one buffer.
    let mut buf = cipher_tag.to_vec();
    let plain = kunci
        .open_in_place(nonce, Aad::from(aad.as_slice()), &mut buf)
        .map_err(|_| GalatArsip::DekSalah)?;
    Ok(plain.to_vec())
}

fn aad_cell(nik_hmac: &str, kolom: &str) -> Vec<u8> {
    let mut a = Vec::with_capacity(kolom.len() + nik_hmac.len() + 1);
    a.extend_from_slice(kolom.as_bytes());
    a.push(b':');
    a.extend_from_slice(nik_hmac.as_bytes());
    a
}

/// Standard base64, as stored in [`META_DEK`].
pub(crate) fn base64_encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// Standard base64 decoding for [`META_DEK`].
pub(crate) fn base64_decode(s: &str) -> Result<Vec<u8>, GalatArsip> {
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .map_err(|_| GalatArsip::Format("dek_terbungkus bukan base64".to_owned()))
}

/// Lowercase hex SHA-256 of `bytes`.
pub fn hex_sha256(bytes: &[u8]) -> String {
    let d = Sha256::digest(bytes);
    let mut s = String::with_capacity(64);
    for b in d {
        s.push_str(&format!("{b:02x}"));
    }
    s
}
