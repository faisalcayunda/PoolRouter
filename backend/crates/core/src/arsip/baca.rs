//! Reading one archive object (AC-15, AC-16, AC-17).
//!
//! Two read paths, deliberately separate:
//!
//! - [`BacaArsip::baca_semua`] decodes every row. Used by the round-trip test and
//!   by restore when it needs the whole window.
//! - [`BacaArsip::cari`] answers "which rows have these `nik_hmac`" **without
//!   decoding the object**. It reads the footer, asks each row group's bloom
//!   filter, and decodes only the groups that might contain a key (AC-17). This
//!   is the reason the format has a bloom filter at all.
//!
//! Both go through [`PenyimpanArsip::get_range`] via an [`AsyncFileReader`]
//! adapter, so the ranges counter in [`super::Memori`] sees exactly the reads
//! the object costs. Streaming through `object_store` directly instead would
//! hide that from the only test that can prove the claim.
//!
//! The DEK is opened from the footer metadata through the caller's
//! [`PembungkusDek`]; a wrong wrapper fails at [`GalatArsip::Pembungkus`] before
//! any cell is touched.

use std::ops::Range;
use std::sync::Arc;

use arrow::array::{Array, BinaryArray, Int32Array, StringArray, TimestampMicrosecondArray};
use arrow::record_batch::RecordBatch;
use bytes::Bytes;
use futures::FutureExt;
use futures::future::BoxFuture;
use parquet::arrow::ProjectionMask;
use parquet::arrow::arrow_reader::ArrowReaderMetadata;
use parquet::arrow::arrow_reader::ArrowReaderOptions;
use parquet::arrow::async_reader::{AsyncFileReader, ParquetRecordBatchStreamBuilder};
use parquet::file::metadata::ParquetMetaData;

use crate::admin::aman::{Nik, NilaiAman};
use crate::arsip::galat::GalatArsip;
use crate::arsip::kunci::{Dek, PembungkusDek};
use crate::arsip::port::PenyimpanArsip;
use crate::arsip::tulis::{
    KOLOM_KODE_TRANSAKSI, KOLOM_KUNCI_HMAC_VERSI, KOLOM_NIK, KOLOM_NIK_HMAC, KOLOM_PAYLOAD,
    KOLOM_TERJADI_AT, META_DEK, META_FORMAT, META_KEK_ID, META_KUNCI_HMAC_VERSI, META_TABEL,
    META_VERSI, VERSI_FORMAT, base64_decode, buka_cell,
};
use crate::arsip::{BarisArsip, KunciObjek, VersiKunciHmac};

/// Which NIK digests a search is looking for.
///
/// A newtype rather than `&[NikHmac]` so the empty case is a named variant and
/// "search for nothing" cannot be confused with "read everything".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaringanNik(Vec<String>);

impl SaringanNik {
    /// Search for these digests. An empty slice means "match nothing", which
    /// [`BacaArsip::cari`] answers with zero rows rather than the whole object.
    pub fn baru(hmac: impl IntoIterator<Item = impl AsRef<str>>) -> Self {
        Self(hmac.into_iter().map(|h| h.as_ref().to_owned()).collect())
    }

    /// Nothing matches.
    pub fn kosong() -> Self {
        Self(Vec::new())
    }

    fn berisi(&self, h: &str) -> bool {
        self.0.iter().any(|x| x == h)
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Reads archive objects.
pub struct BacaArsip;

impl BacaArsip {
    /// Decode every row of the object, in stored order.
    pub async fn baca_semua(
        penyimpan: Arc<dyn PenyimpanArsip>,
        kunci: Arc<KunciObjek>,
        pembungkus: &dyn PembungkusDek,
    ) -> Result<Vec<BarisArsip>, GalatArsip> {
        let (pembaca, meta) =
            baca_metadata(PembacaPort::baru(penyimpan.clone(), kunci.clone())).await?;
        let dek = dek_dari_meta(&meta, pembungkus)?;
        let versi = versi_dari_meta(&meta)?;
        let batch = baca_batch(pembaca, meta, None).await?;
        baris_dari_batch(&batch, &dek, versi)
    }

    /// Rows whose `nik_hmac` is in `saring`, reading only the row groups whose
    /// bloom filter could contain one of the keys (AC-17).
    ///
    /// Returns `(baris, jumlah_row_group_dibaca)`. The count is what a test can
    /// compare against the total, and what the job can log so an operator sees
    /// a search that suddenly reads every group.
    pub async fn cari(
        penyimpan: Arc<dyn PenyimpanArsip>,
        kunci: Arc<KunciObjek>,
        saring: &SaringanNik,
        pembungkus: &dyn PembungkusDek,
    ) -> Result<(Vec<BarisArsip>, usize), GalatArsip> {
        if saring.is_empty() {
            return Ok((Vec::new(), 0));
        }
        let (pembaca, meta) =
            baca_metadata(PembacaPort::baru(penyimpan.clone(), kunci.clone())).await?;
        let dek = dek_dari_meta(&meta, pembungkus)?;
        let versi = versi_dari_meta(&meta)?;

        let kandidat = row_group_kandidat(&meta, pembaca, saring).await?;
        if kandidat.is_empty() {
            return Ok((Vec::new(), 0));
        }
        let dibaca = kandidat.len();
        let (pembaca, meta) =
            baca_metadata(PembacaPort::baru(penyimpan.clone(), kunci.clone())).await?;
        let batch = baca_batch(pembaca, meta, Some(kandidat)).await?;
        let semua = baris_dari_batch(&batch, &dek, versi)?;
        // The bloom filter is probabilistic: it can say "maybe" for a key that is
        // absent. Filter exactly so a caller never sees a false positive.
        Ok((
            semua
                .into_iter()
                .filter(|b| saring.berisi(b.nik_hmac.as_str()))
                .collect(),
            dibaca,
        ))
    }

    /// The footer metadata, for a caller that needs the summary fields without
    /// decoding rows.
    pub async fn metadata(
        penyimpan: Arc<dyn PenyimpanArsip>,
        kunci: Arc<KunciObjek>,
    ) -> Result<ParquetMetaData, GalatArsip> {
        let (_, meta) = baca_metadata(PembacaPort::baru(penyimpan.clone(), kunci.clone())).await?;
        Ok(Arc::unwrap_or_clone(meta))
    }

    /// The wrapped-DEK metadata without opening it, so a caller can report
    /// `kek_id` before it has the KEK.
    pub fn kek_id(meta: &ParquetMetaData) -> Option<String> {
        meta.file_metadata()
            .key_value_metadata()?
            .iter()
            .find(|kv| kv.key == META_KEK_ID)
            .map(|kv| kv.value.clone().unwrap_or_default())
    }

    /// The source table recorded when the object was written.
    pub fn tabel(meta: &ParquetMetaData) -> Option<String> {
        meta.file_metadata()
            .key_value_metadata()?
            .iter()
            .find(|kv| kv.key == META_TABEL)
            .map(|kv| kv.value.clone().unwrap_or_default())
    }
}

/// An [`AsyncFileReader`] over [`PenyimpanArsip`], so every byte the Parquet
/// code asks for goes through the port's `get_range` and is counted there.
struct PembacaPort {
    penyimpan: Arc<dyn PenyimpanArsip>,
    kunci: Arc<KunciObjek>,
}

impl PembacaPort {
    fn baru(penyimpan: Arc<dyn PenyimpanArsip>, kunci: Arc<KunciObjek>) -> Self {
        Self { penyimpan, kunci }
    }
}

impl AsyncFileReader for PembacaPort {
    fn get_bytes(&mut self, range: Range<u64>) -> BoxFuture<'_, parquet::errors::Result<Bytes>> {
        async move {
            let v = self
                .penyimpan
                .get_range(self.kunci.as_ref(), range)
                .await
                .map_err(|e| parquet::errors::ParquetError::General(e.to_string()))?;
            Ok(Bytes::from(v))
        }
        .boxed()
    }

    fn get_metadata<'a>(
        &'a mut self,
        _options: Option<&'a ArrowReaderOptions>,
    ) -> BoxFuture<'a, parquet::errors::Result<Arc<ParquetMetaData>>> {
        async move {
            // `ParquetMetaDataReader` needs the total length to read the footer,
            // which is why `head` is part of the port.
            let size = self
                .penyimpan
                .head(self.kunci.as_ref())
                .await
                .map_err(|e| parquet::errors::ParquetError::General(e.to_string()))?
                .ok_or_else(|| {
                    parquet::errors::ParquetError::General("objek arsip tidak ditemukan".to_owned())
                })?;
            let fetch = FetchPort {
                penyimpan: self.penyimpan.clone(),
                kunci: self.kunci.clone(),
            };
            let meta = parquet::file::metadata::ParquetMetaDataReader::new()
                .load_and_finish(fetch, size)
                .await?;
            Ok(Arc::new(meta))
        }
        .boxed()
    }
}

/// Adapts the port to the footer reader's fetch interface.
struct FetchPort {
    penyimpan: Arc<dyn PenyimpanArsip>,
    kunci: Arc<KunciObjek>,
}

impl parquet::arrow::async_reader::MetadataFetch for FetchPort {
    fn fetch(&mut self, range: Range<u64>) -> BoxFuture<'_, parquet::errors::Result<Bytes>> {
        async move {
            let v = self
                .penyimpan
                .get_range(self.kunci.as_ref(), range)
                .await
                .map_err(|e| parquet::errors::ParquetError::General(e.to_string()))?;
            Ok(Bytes::from(v))
        }
        .boxed()
    }
}

impl parquet::arrow::async_reader::MetadataSuffixFetch for FetchPort {
    fn fetch_suffix(&mut self, suffix: usize) -> BoxFuture<'_, parquet::errors::Result<Bytes>> {
        async move {
            let size = self
                .penyimpan
                .head(self.kunci.as_ref())
                .await
                .map_err(|e| parquet::errors::ParquetError::General(e.to_string()))?
                .ok_or_else(|| {
                    parquet::errors::ParquetError::General("objek arsip tidak ditemukan".to_owned())
                })?;
            let start = size.saturating_sub(suffix as u64);
            let v = self
                .penyimpan
                .get_range(self.kunci.as_ref(), start..size)
                .await
                .map_err(|e| parquet::errors::ParquetError::General(e.to_string()))?;
            Ok(Bytes::from(v))
        }
        .boxed()
    }
}

/// Read the footer, and hand back a fresh reader for the data that follows.
///
/// The footer read consumes the reader, so the caller gets a new one: the same
/// `PenyimpanArsip` and key, wrapped again.
async fn baca_metadata(p: PembacaPort) -> Result<(PembacaPort, Arc<ParquetMetaData>), GalatArsip> {
    let segar = PembacaPort::baru(p.penyimpan.clone(), p.kunci.clone());
    let mut p = p;
    let meta = p
        .get_metadata(None)
        .await
        .map_err(|e| GalatArsip::Parquet(e.to_string()))?;
    cek_format(&meta)?;
    Ok((segar, meta))
}

fn cek_format(meta: &ParquetMetaData) -> Result<(), GalatArsip> {
    let kv = meta
        .file_metadata()
        .key_value_metadata()
        .ok_or_else(|| GalatArsip::Format("objek arsip tanpa metadata format".to_owned()))?;
    let cari = |k: &str| kv.iter().find(|x| x.key == k).and_then(|x| x.value.clone());
    let format = cari(META_FORMAT)
        .ok_or_else(|| GalatArsip::Format("objek bukan objek arsip".to_owned()))?;
    if format != "parquet" {
        return Err(GalatArsip::Format(format!("format {format} tidak dikenal")));
    }
    let versi =
        cari(META_VERSI).ok_or_else(|| GalatArsip::Format("versi arsip hilang".to_owned()))?;
    if versi != VERSI_FORMAT {
        return Err(GalatArsip::Format(format!(
            "versi arsip {versi} tidak didukung"
        )));
    }
    Ok(())
}

fn dek_dari_meta(
    meta: &ParquetMetaData,
    pembungkus: &dyn PembungkusDek,
) -> Result<Dek, GalatArsip> {
    let kv = meta
        .file_metadata()
        .key_value_metadata()
        .ok_or_else(|| GalatArsip::Format("objek tanpa metadata".to_owned()))?;
    let b64 = kv
        .iter()
        .find(|x| x.key == META_DEK)
        .and_then(|x| x.value.clone())
        .ok_or_else(|| GalatArsip::Format("dek_terbungkus hilang".to_owned()))?;
    let bytes = base64_decode(&b64)?;
    pembungkus.buka(&crate::arsip::kunci::DekTerbungkus::dari_bytes(bytes))
}

fn versi_dari_meta(meta: &ParquetMetaData) -> Result<VersiKunciHmac, GalatArsip> {
    let kv = meta
        .file_metadata()
        .key_value_metadata()
        .ok_or_else(|| GalatArsip::Format("objek tanpa metadata".to_owned()))?;
    let s = kv
        .iter()
        .find(|x| x.key == META_KUNCI_HMAC_VERSI)
        .and_then(|x| x.value.clone())
        .ok_or_else(|| GalatArsip::Format("kunci_hmac_versi hilang".to_owned()))?;
    let n: i32 = s
        .parse()
        .map_err(|_| GalatArsip::Format("kunci_hmac_versi bukan angka".to_owned()))?;
    // The object's column values must agree with this too; the reader checks the
    // footer value here and the per-row value on decode.
    VersiKunciHmac::baru(n).map_err(|_| GalatArsip::KunciHmacTidakTersedia(n))
}

/// Which row groups could hold one of the search keys, by bloom filter.
///
/// Uses the Parquet reader's own bloom-filter fetch, which goes through the
/// same [`AsyncFileReader`] and therefore through [`PenyimpanArsip::get_range`].
/// A group with no filter cannot be ruled out and is read.
async fn row_group_kandidat(
    meta: &ParquetMetaData,
    pembaca: PembacaPort,
    saring: &SaringanNik,
) -> Result<Vec<usize>, GalatArsip> {
    let kolom_hmac = indeks_kolom_hmac(meta)?;
    let arrow_meta =
        ArrowReaderMetadata::try_new(Arc::new(meta.clone()), ArrowReaderOptions::new())?;
    let mut builder = ParquetRecordBatchStreamBuilder::new_with_metadata(pembaca, arrow_meta);

    let mut kandidat = Vec::new();
    for g in 0..meta.num_row_groups() {
        let filter = builder
            .get_row_group_column_bloom_filter(g, kolom_hmac)
            .await?;
        let cocok = match filter {
            Some(f) => saring.0.iter().any(|h| f.check(h.as_bytes())),
            None => true,
        };
        if cocok {
            kandidat.push(g);
        }
    }
    Ok(kandidat)
}

fn indeks_kolom_hmac(meta: &ParquetMetaData) -> Result<usize, GalatArsip> {
    meta.row_group(0)
        .columns()
        .iter()
        .position(|c| c.column_path().string() == KOLOM_NIK_HMAC)
        .ok_or_else(|| GalatArsip::Format("kolom nik_hmac tidak ada".to_owned()))
}

async fn baca_batch(
    pembaca: PembacaPort,
    meta: Arc<ParquetMetaData>,
    row_group: Option<Vec<usize>>,
) -> Result<RecordBatch, GalatArsip> {
    let arrow_meta = ArrowReaderMetadata::try_new(meta, ArrowReaderOptions::new())?;
    let mut builder = ParquetRecordBatchStreamBuilder::new_with_metadata(pembaca, arrow_meta);
    if let Some(g) = row_group {
        builder = builder.with_row_groups(g);
    }
    let reader = builder.build()?;
    let schema = reader.schema().clone();
    let mut batches = Vec::new();
    let mut stream = reader;
    while let Some(b) = futures::StreamExt::next(&mut stream).await {
        batches.push(b?);
    }
    let batch = arrow::compute::concat_batches(&schema, &batches)?;
    Ok(batch)
}

fn baris_dari_batch(
    batch: &RecordBatch,
    dek: &Dek,
    versi_footer: VersiKunciHmac,
) -> Result<Vec<BarisArsip>, GalatArsip> {
    let kode = kolom::<StringArray>(batch, KOLOM_KODE_TRANSAKSI)?;
    let hmac = kolom::<StringArray>(batch, KOLOM_NIK_HMAC)?;
    let versi = kolom::<Int32Array>(batch, KOLOM_KUNCI_HMAC_VERSI)?;
    let nik = kolom::<BinaryArray>(batch, KOLOM_NIK)?;
    let payload = kolom::<BinaryArray>(batch, KOLOM_PAYLOAD)?;
    let terjadi = kolom_terjadi(batch)?;

    let mut out = Vec::with_capacity(batch.num_rows());
    for i in 0..batch.num_rows() {
        let h = hmac.value(i).to_owned();
        let v = VersiKunciHmac::baru(versi.value(i))
            .map_err(|_| GalatArsip::KunciHmacTidakTersedia(versi.value(i)))?;
        if v != versi_footer {
            return Err(GalatArsip::KunciHmacTidakTersedia(v.nilai()));
        }
        let nik = if nik.is_null(i) {
            None
        } else {
            let pt = buka_cell(dek, nik.value(i), &h, KOLOM_NIK)?;
            let s = String::from_utf8(pt).map_err(|_| GalatArsip::EnkripsiTidakUtuh(KOLOM_NIK))?;
            // The column is a NIK by contract, so a value that does not parse
            // means the cell opened under the wrong AAD rather than "no NIK".
            let n = Nik::parse(&s).map_err(|_| GalatArsip::EnkripsiTidakUtuh(KOLOM_NIK))?;
            Some(NilaiAman::baru(n))
        };
        let payload = if payload.is_null(i) {
            None
        } else {
            let pt = buka_cell(dek, payload.value(i), &h, KOLOM_PAYLOAD)?;
            Some(NilaiAman::baru(
                String::from_utf8(pt).map_err(|_| GalatArsip::EnkripsiTidakUtuh(KOLOM_PAYLOAD))?,
            ))
        };
        let nik_hmac = crate::audit::NikHmac::parse(h)
            .map_err(|_| GalatArsip::Format("nik_hmac bukan heksadesimal".to_owned()))?;
        out.push(BarisArsip {
            kode_transaksi: kode.value(i).to_owned(),
            nik_hmac,
            kunci_hmac_versi: v,
            nik,
            payload,
            terjadi_at: micros_ke_datetime(terjadi.value(i))?,
        });
    }
    Ok(out)
}

/// Timestamp column values are microseconds since the epoch.
fn micros_ke_datetime(micros: i64) -> Result<chrono::DateTime<chrono::Utc>, GalatArsip> {
    chrono::DateTime::from_timestamp_micros(micros)
        .ok_or_else(|| GalatArsip::Format("terjadi_at di luar rentang waktu".to_owned()))
}

fn kolom<'a, T: 'static>(batch: &'a RecordBatch, nama: &str) -> Result<&'a T, GalatArsip> {
    batch
        .column_by_name(nama)
        .and_then(|c| c.as_any().downcast_ref::<T>())
        .ok_or_else(|| GalatArsip::Format(format!("kolom {nama} hilang atau salah tipe")))
}

fn kolom_terjadi(batch: &RecordBatch) -> Result<&TimestampMicrosecondArray, GalatArsip> {
    let c = batch
        .column_by_name(KOLOM_TERJADI_AT)
        .ok_or_else(|| GalatArsip::Format("kolom terjadi_at hilang".to_owned()))?;
    if let Some(a) = c.as_any().downcast_ref::<TimestampMicrosecondArray>() {
        return Ok(a);
    }
    Err(GalatArsip::Format(format!(
        "kolom terjadi_at bertipe {:?}, bukan Timestamp(Microsecond)",
        c.data_type()
    )))
}

/// `ProjectionMask` helper for callers that want only the sort columns, for
/// example a listing that must not touch sealed cells at all. Wired in by T6/T7.
pub fn proyeksi_tanpa_pii(meta: &ParquetMetaData) -> Result<ProjectionMask, GalatArsip> {
    let skema = meta.file_metadata().schema_descr();
    let idx: Vec<usize> = (0..skema.num_columns())
        .filter(|i| {
            let n = skema.column(*i).name().to_owned();
            n == KOLOM_KODE_TRANSAKSI || n == KOLOM_NIK_HMAC || n == KOLOM_KUNCI_HMAC_VERSI
        })
        .collect();
    Ok(ProjectionMask::leaves(skema, idx))
}
