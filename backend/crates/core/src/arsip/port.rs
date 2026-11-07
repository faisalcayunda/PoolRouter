//! The storage port and its two implementations (AC-19, DA21).
//!
//! [`PenyimpanArsip`] has exactly three operations — `put`, `get_range`, `head`.
//! There is deliberately **no delete**: the retention requirement this module
//! exists for is undone by one `delete`, so the port cannot express it. Erasing
//! an object at end of retention is T7's job with a bucket-side lifecycle rule,
//! not something a job in this process can do by accident.
//!
//! `get_range` is the third operation rather than a whole-object `get` because
//! AC-17 measures reads: a NIK lookup must be footer + one row group, and the
//! only way to make that observable is to count the ranges the reader asks for.
//!
//! Two implementations:
//!
//! - [`Memori`] — an in-process store for tests. It records every range it
//!   serves, which is what AC-17 asserts on.
//! - [`S3`] — `object_store` with the `aws` feature, covering S3 and the
//!   OBS Huawei S3-compatible endpoint (ADR-0006 §8 butir 3).

use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::{Arc, Mutex};

use futures::FutureExt;
use futures::future::BoxFuture;

use crate::arsip::galat::GalatArsip;
use crate::arsip::kunci::KunciObjek;

/// What the job records about one written object, and what [`super::Verifikasi`]
/// checks it against (AC-15, AC-18).
///
/// Every field is recomputed from the bytes on verification, so the summary is
/// the only thing that has to be trusted between the write and the audit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RingkasanObjek {
    /// The object this summary describes.
    pub kunci: KunciObjek,
    /// SHA-256 of the whole object, lowercase hex.
    pub sha256: String,
    /// Number of rows in the object.
    pub jumlah_baris: i64,
    /// Size of the object in bytes.
    pub ukuran: u64,
    /// Range of `kode_transaksi`; `None` for an empty object.
    pub rentang_kode_transaksi: Option<super::RentangKodeTransaksi>,
    /// HMAC key version the rows were hashed with (DA11).
    pub kunci_hmac_versi: super::VersiKunciHmac,
}

/// The three operations the archive job needs from storage.
///
/// Returns boxed futures rather than `async fn` because the port is used as
/// `&dyn PenyimpanArsip`: the format code must not know the storage type, and an
/// `async fn` in a trait is not dyn-compatible.
pub trait PenyimpanArsip: Send + Sync {
    /// Write the object. Writing the same key twice leaves the **last** object
    /// (AC-19): a retried job must not need a delete first.
    fn put<'a>(
        &'a self,
        kunci: &'a KunciObjek,
        isi: Vec<u8>,
    ) -> BoxFuture<'a, Result<(), GalatArsip>>;

    /// Read `range` bytes. The reader uses this for the footer and for each row
    /// group's column chunks, so the ranges it asks for are the reads the job
    /// actually performs.
    fn get_range<'a>(
        &'a self,
        kunci: &'a KunciObjek,
        range: Range<u64>,
    ) -> BoxFuture<'a, Result<Vec<u8>, GalatArsip>>;

    /// Size and existence of the object, without reading it. `None` when the
    /// object is not there.
    fn head<'a>(&'a self, kunci: &'a KunciObjek) -> BoxFuture<'a, Result<Option<u64>, GalatArsip>>;
}

/// One `get_range` call the in-memory store served: object name and byte range.
pub type CatatanBaca = (String, Range<u64>);

/// In-process [`PenyimpanArsip`] that records every range it serves.
///
/// The recording is not debug output: AC-17 asserts on it to prove a NIK lookup
/// did not read the whole object.
#[derive(Debug, Clone, Default)]
pub struct Memori {
    isi: Arc<Mutex<BTreeMap<String, Vec<u8>>>>,
    baca: Arc<Mutex<Vec<CatatanBaca>>>,
}

impl Memori {
    /// An empty store.
    pub fn baru() -> Self {
        Self::default()
    }

    /// Every `get_range` served so far, in order.
    pub fn catatan_baca(&self) -> Vec<CatatanBaca> {
        self.baca.lock().expect("mutex memori").clone()
    }

    /// Ranges served for one object.
    pub fn catatan_baca_kunci(&self, kunci: &KunciObjek) -> Vec<Range<u64>> {
        self.baca
            .lock()
            .expect("mutex memori")
            .iter()
            .filter(|(k, _)| k == kunci.as_str())
            .map(|(_, r)| r.clone())
            .collect()
    }

    /// Drop the recording, keeping the objects. Tests that write before they
    /// measure call this.
    pub fn lupakan_baca(&self) {
        self.baca.lock().expect("mutex memori").clear();
    }

    /// The full object, for tests that scan raw bytes for a leaked value
    /// (AC-16).
    pub fn bytes_mentah(&self, kunci: &KunciObjek) -> Option<Vec<u8>> {
        self.isi
            .lock()
            .expect("mutex memori")
            .get(kunci.as_str())
            .cloned()
    }

    /// How many objects the store holds.
    pub fn jumlah_objek(&self) -> usize {
        self.isi.lock().expect("mutex memori").len()
    }
}

impl PenyimpanArsip for Memori {
    fn put<'a>(
        &'a self,
        kunci: &'a KunciObjek,
        isi: Vec<u8>,
    ) -> BoxFuture<'a, Result<(), GalatArsip>> {
        async move {
            // Insert overwrites, which is the idempotent-last-writer rule of AC-19.
            self.isi
                .lock()
                .expect("mutex memori")
                .insert(kunci.as_str().to_owned(), isi);
            Ok(())
        }
        .boxed()
    }

    fn get_range<'a>(
        &'a self,
        kunci: &'a KunciObjek,
        range: Range<u64>,
    ) -> BoxFuture<'a, Result<Vec<u8>, GalatArsip>> {
        async move {
            self.baca
                .lock()
                .expect("mutex memori")
                .push((kunci.as_str().to_owned(), range.clone()));
            let isi = self.isi.lock().expect("mutex memori");
            let objek = isi
                .get(kunci.as_str())
                .ok_or_else(|| GalatArsip::Penyimpanan(format!("objek {kunci} tidak ada")))?;
            let len = objek.len() as u64;
            if range.start > range.end || range.end > len {
                return Err(GalatArsip::Penyimpanan(format!(
                    "rentang {}..{} di luar objek {len} byte",
                    range.start, range.end
                )));
            }
            Ok(objek[range.start as usize..range.end as usize].to_vec())
        }
        .boxed()
    }

    fn head<'a>(&'a self, kunci: &'a KunciObjek) -> BoxFuture<'a, Result<Option<u64>, GalatArsip>> {
        async move {
            Ok(self
                .isi
                .lock()
                .expect("mutex memori")
                .get(kunci.as_str())
                .map(|v| v.len() as u64))
        }
        .boxed()
    }
}

/// [`PenyimpanArsip`] over `object_store`, for S3 and S3-compatible storage
/// (OBS Huawei included).
///
/// The port is the boundary: the rest of the module never names a type from
/// this crate, so a future storage swap does not reach the format code.
pub struct S3 {
    toko: Arc<dyn object_store::ObjectStore>,
    // Kept so `Debug` can name the store without printing credentials.
    label: String,
}

impl std::fmt::Debug for S3 {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "S3({})", self.label)
    }
}

impl S3 {
    /// Wrap an already-configured `object_store` store. Configuration (endpoint,
    /// bucket, credentials, `with_allow_http`) is the caller's job: T7 owns the
    /// lane, and keeping it out of here means the port does not read the
    /// environment.
    pub fn baru(toko: Arc<dyn object_store::ObjectStore>, label: impl Into<String>) -> Self {
        Self {
            toko,
            label: label.into(),
        }
    }

    /// Build the S3 store from a bucket and an optional endpoint, which is the
    /// shape `AmazonS3Builder` needs for both AWS and OBS Huawei.
    ///
    /// `allow_http` is a parameter, not an environment read, because only the
    /// Compose lane and MinIO in tests may use it.
    pub fn dari_bucket(
        bucket: &str,
        endpoint: Option<&str>,
        allow_http: bool,
        label: impl Into<String>,
    ) -> Result<Self, GalatArsip> {
        use object_store::aws::AmazonS3Builder;
        let mut b = AmazonS3Builder::from_env().with_bucket_name(bucket);
        if let Some(e) = endpoint {
            b = b.with_endpoint(e).with_allow_http(allow_http);
        }
        let toko = b
            .build()
            .map_err(|e| GalatArsip::Penyimpanan(format!("S3 tidak siap: {e}")))?;
        Ok(Self::baru(Arc::new(toko), label))
    }

    fn path(kunci: &KunciObjek) -> object_store::path::Path {
        object_store::path::Path::from(kunci.as_str())
    }
}

impl PenyimpanArsip for S3 {
    fn put<'a>(
        &'a self,
        kunci: &'a KunciObjek,
        isi: Vec<u8>,
    ) -> BoxFuture<'a, Result<(), GalatArsip>> {
        use object_store::ObjectStoreExt;
        async move {
            self.toko
                .put(&Self::path(kunci), isi.into())
                .await
                .map(|_| ())
                .map_err(|e| GalatArsip::Penyimpanan(e.to_string()))
        }
        .boxed()
    }

    fn get_range<'a>(
        &'a self,
        kunci: &'a KunciObjek,
        range: Range<u64>,
    ) -> BoxFuture<'a, Result<Vec<u8>, GalatArsip>> {
        use object_store::ObjectStoreExt;
        async move {
            self.toko
                .get_range(&Self::path(kunci), range)
                .await
                .map(|b| b.to_vec())
                .map_err(|e| GalatArsip::Penyimpanan(e.to_string()))
        }
        .boxed()
    }

    fn head<'a>(&'a self, kunci: &'a KunciObjek) -> BoxFuture<'a, Result<Option<u64>, GalatArsip>> {
        use object_store::ObjectStoreExt;
        async move {
            match self.toko.head(&Self::path(kunci)).await {
                Ok(m) => Ok(Some(m.size)),
                Err(object_store::Error::NotFound { .. }) => Ok(None),
                Err(e) => Err(GalatArsip::Penyimpanan(e.to_string())),
            }
        }
        .boxed()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn memori_idempoten_penulis_terakhir_menang() {
        let m = Memori::baru();
        let k = KunciObjek::baru("t", "p", "2026-11-05", "00-06").unwrap();
        m.put(&k, b"satu".to_vec()).await.unwrap();
        m.put(&k, b"dua".to_vec()).await.unwrap();
        assert_eq!(m.jumlah_objek(), 1);
        assert_eq!(m.bytes_mentah(&k).unwrap(), b"dua");
        assert_eq!(m.head(&k).await.unwrap(), Some(3));
    }

    #[tokio::test]
    async fn memori_head_hilang_none() {
        let m = Memori::baru();
        let k = KunciObjek::baru("t", "p", "2026-11-05", "00-06").unwrap();
        assert_eq!(m.head(&k).await.unwrap(), None);
    }

    #[tokio::test]
    async fn memori_mencatat_rentang() {
        let m = Memori::baru();
        let k = KunciObjek::baru("t", "p", "2026-11-05", "00-06").unwrap();
        m.put(&k, (0u8..10).collect()).await.unwrap();
        m.get_range(&k, 0..4).await.unwrap();
        m.get_range(&k, 4..10).await.unwrap();
        assert_eq!(m.catatan_baca_kunci(&k), vec![0..4, 4..10]);
        m.lupakan_baca();
        assert!(m.catatan_baca().is_empty());
    }

    #[tokio::test]
    async fn memori_rentang_di_luar_objek_ditolak() {
        let m = Memori::baru();
        let k = KunciObjek::baru("t", "p", "2026-11-05", "00-06").unwrap();
        m.put(&k, b"abc".to_vec()).await.unwrap();
        assert!(m.get_range(&k, 0..4).await.is_err());
    }
}
