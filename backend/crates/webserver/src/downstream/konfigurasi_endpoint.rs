//! Endpoint configuration as the downstream API reads it (`pr_api` column
//! grants, migration 20261103000004), behind the same in-process cache as the
//! credential store: at most 30 s old (ADR-0001 rev 15 §3a.4).
//!
//! One cache row per `(client_id, endpoint_id)`. It carries the contract and
//! the `aktif` status of the six entities of §3a.4 (client, credential,
//! endpoint, downstream pool, upstream pool, provider), so a deactivation
//! reaches this process at the latest 30 s later. The contract is only served
//! when the endpoint is assigned to the calling client, which is why the client
//! is part of the key.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use sqlx::PgPool;
use uuid::Uuid;

use super::credential::{CACHE_TTL, StoreError, TtlCache};
use crate::api::downstream::{EndpointKontrak, FieldMetadata};

/// Errors are not distinguishable to the caller; see [`StoreError`].
pub type Fut<'a, T> = Pin<Box<dyn Future<Output = Result<T, StoreError>> + Send + 'a>>;

/// `aktif` of the six entities of ADR-0001 rev 15 §3a.4.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusEntitas {
    pub client: bool,
    /// Whether the client still holds at least one active credential.
    pub kredensial: bool,
    pub endpoint: bool,
    pub downstream_pool: bool,
    /// Whether the endpoint's pool still routes to at least one active
    /// upstream pool.
    pub upstream_pool: bool,
    pub provider: bool,
}

/// One cached answer. `kontrak` is `None` when the endpoint is unknown or not
/// assigned to the client; `status` is `None` only when the endpoint itself
/// does not exist.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EntriKonfig {
    pub kontrak: Option<EndpointKontrak>,
    pub status: Option<StatusEntitas>,
}

pub trait KonfigurasiStore: Send + Sync + 'static {
    /// Reads the entry for `client_id` and `endpoint_id`. `sekarang` is the
    /// clock the cache measures the 30 s window with, injected so tests advance
    /// it instead of sleeping.
    fn muat(&self, client_id: Uuid, endpoint_id: Uuid, sekarang: Instant) -> Fut<'_, EntriKonfig>;
}

/// PostgreSQL-backed reader: two round trips on a cache miss, none on a hit.
pub struct PgKonfigurasiStore {
    pool: PgPool,
}

impl PgKonfigurasiStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl KonfigurasiStore for PgKonfigurasiStore {
    fn muat(&self, client_id: Uuid, endpoint_id: Uuid, _sekarang: Instant) -> Fut<'_, EntriKonfig> {
        Box::pin(async move {
            let status = sqlx::query_as::<_, BarisStatus>(
                "SELECT \
                   (SELECT c.aktif FROM client c WHERE c.client_id = $1) AS client_aktif, \
                   EXISTS (SELECT 1 FROM client_kredensial k \
                            WHERE k.client_id = $1 AND k.status = 'aktif' \
                              AND (k.kedaluwarsa_at IS NULL OR k.kedaluwarsa_at > now())) \
                     AS kredensial_aktif, \
                   e.aktif AS endpoint_aktif, \
                   p.aktif AS provider_aktif, \
                   d.aktif AS downstream_pool_aktif, \
                   EXISTS (SELECT 1 FROM downstream_pool_rute r \
                            JOIN upstream_pool u ON u.upstream_pool_id = r.upstream_pool_id \
                            WHERE r.downstream_pool_id = e.downstream_pool_id AND u.aktif) \
                     AS upstream_pool_aktif \
                 FROM downstream_endpoint e \
                 JOIN provider p ON p.provider_id = e.provider_id \
                 JOIN downstream_pool d ON d.downstream_pool_id = e.downstream_pool_id \
                 WHERE e.downstream_endpoint_id = $2",
            )
            .bind(client_id)
            .bind(endpoint_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|_| StoreError)?
            .map(|b| StatusEntitas {
                client: b.client_aktif.unwrap_or(false),
                kredensial: b.kredensial_aktif,
                endpoint: b.endpoint_aktif,
                downstream_pool: b.downstream_pool_aktif,
                upstream_pool: b.upstream_pool_aktif,
                provider: b.provider_aktif,
            });
            if status.is_none() {
                return Ok(EntriKonfig {
                    kontrak: None,
                    status: None,
                });
            }
            let rows = sqlx::query_as::<_, BarisKontrak>(
                "SELECT e.downstream_endpoint_id, c.versi, m.nama_field, m.tipe, m.wajib, \
                 m.nilai_diizinkan, m.deskripsi, m.versi AS versi_field \
                 FROM client_endpoint ce \
                 JOIN downstream_endpoint e \
                      ON e.downstream_endpoint_id = ce.downstream_endpoint_id \
                 LEFT JOIN endpoint_config_versi c \
                      ON c.downstream_endpoint_id = e.downstream_endpoint_id \
                     AND c.versi = e.config_versi_aktif \
                 LEFT JOIN endpoint_metadata_field m \
                      ON m.downstream_endpoint_id = e.downstream_endpoint_id \
                 WHERE ce.client_id = $1 AND ce.downstream_endpoint_id = $2 \
                 ORDER BY m.nama_field",
            )
            .bind(client_id)
            .bind(endpoint_id)
            .fetch_all(&self.pool)
            .await
            .map_err(|_| StoreError)?;
            Ok(EntriKonfig {
                kontrak: susun_kontrak(rows)?,
                status,
            })
        })
    }
}

#[derive(sqlx::FromRow)]
struct BarisStatus {
    client_aktif: Option<bool>,
    kredensial_aktif: bool,
    endpoint_aktif: bool,
    provider_aktif: bool,
    downstream_pool_aktif: bool,
    upstream_pool_aktif: bool,
}

/// One row of the contract join. The metadata columns are `None` when the
/// endpoint has no metadata field at all; the left join still yields one row.
#[derive(sqlx::FromRow)]
struct BarisKontrak {
    downstream_endpoint_id: Uuid,
    versi: Option<i32>,
    nama_field: Option<String>,
    tipe: Option<String>,
    wajib: Option<bool>,
    nilai_diizinkan: Option<serde_json::Value>,
    deskripsi: Option<String>,
    versi_field: Option<i32>,
}

fn susun_kontrak(rows: Vec<BarisKontrak>) -> Result<Option<EndpointKontrak>, StoreError> {
    let Some(first) = rows.first() else {
        return Ok(None);
    };
    // No assigned active version means no contract to serve, not an empty one.
    let Some(versi) = first.versi else {
        return Ok(None);
    };
    let mut field_metadata = Vec::new();
    for r in &rows {
        let (Some(nama_field), Some(tipe), Some(wajib), Some(deskripsi), Some(versi_field)) = (
            r.nama_field.as_ref(),
            r.tipe.as_ref(),
            r.wajib,
            r.deskripsi.as_ref(),
            r.versi_field,
        ) else {
            continue;
        };
        let nilai_diizinkan = match &r.nilai_diizinkan {
            None | Some(serde_json::Value::Null) => None,
            Some(v) => {
                Some(serde_json::from_value::<Vec<String>>(v.clone()).map_err(|_| StoreError)?)
            }
        };
        field_metadata.push(FieldMetadata {
            nama_field: nama_field.clone(),
            tipe: tipe.clone(),
            wajib,
            nilai_diizinkan,
            deskripsi: deskripsi.clone(),
            versi: i64::from(versi_field),
        });
    }
    Ok(Some(EndpointKontrak {
        endpoint_id: first.downstream_endpoint_id,
        endpoint_config_version: i64::from(versi),
        // Empty in F0: no column holds the input contract yet; the admin API
        // fills it once it stores the provider parameters (ARC-54).
        field_input: Vec::new(),
        field_metadata,
    }))
}

/// [`KonfigurasiStore`] with a 30 s cache in front of `inner`. Errors are not
/// cached, so an outage does not pin a failure for 30 s.
pub struct CachedKonfigurasi {
    inner: Arc<dyn KonfigurasiStore>,
    entri: TtlCache<(Uuid, Uuid), EntriKonfig>,
}

impl CachedKonfigurasi {
    pub fn new(inner: Arc<dyn KonfigurasiStore>) -> Self {
        Self::with_ttl(inner, CACHE_TTL)
    }

    pub fn with_ttl(inner: Arc<dyn KonfigurasiStore>, ttl: Duration) -> Self {
        Self {
            inner,
            entri: TtlCache::new(ttl),
        }
    }
}

impl KonfigurasiStore for CachedKonfigurasi {
    fn muat(&self, client_id: Uuid, endpoint_id: Uuid, sekarang: Instant) -> Fut<'_, EntriKonfig> {
        Box::pin(async move {
            let kunci = (client_id, endpoint_id);
            if let Some(v) = self.entri.get(&kunci, sekarang) {
                return Ok(v);
            }
            let v = self.inner.muat(client_id, endpoint_id, sekarang).await?;
            self.entri.put(kunci, v.clone(), sekarang);
            Ok(v)
        })
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! In-memory store for handler tests; counts queries.
    use super::*;
    use std::collections::HashMap;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    pub struct FakeKonfigurasi {
        /// `(client_id, endpoint_id)` → entry. Absent means 404 and no status.
        pub entri: Mutex<HashMap<(Uuid, Uuid), EntriKonfig>>,
        pub fail: Mutex<bool>,
        pub calls: AtomicUsize,
    }

    impl FakeKonfigurasi {
        pub fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
        pub fn taruh(&self, client_id: Uuid, kontrak: EndpointKontrak) {
            self.entri.lock().unwrap().insert(
                (client_id, kontrak.endpoint_id),
                EntriKonfig {
                    kontrak: Some(kontrak),
                    status: Some(aktif_semua()),
                },
            );
        }
    }

    pub fn aktif_semua() -> StatusEntitas {
        StatusEntitas {
            client: true,
            kredensial: true,
            endpoint: true,
            downstream_pool: true,
            upstream_pool: true,
            provider: true,
        }
    }

    impl KonfigurasiStore for FakeKonfigurasi {
        fn muat(
            &self,
            client_id: Uuid,
            endpoint_id: Uuid,
            _sekarang: Instant,
        ) -> Fut<'_, EntriKonfig> {
            Box::pin(async move {
                self.calls.fetch_add(1, Ordering::SeqCst);
                if *self.fail.lock().unwrap() {
                    return Err(StoreError);
                }
                Ok(self
                    .entri
                    .lock()
                    .unwrap()
                    .get(&(client_id, endpoint_id))
                    .cloned()
                    .unwrap_or(EntriKonfig {
                        kontrak: None,
                        status: None,
                    }))
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::{FakeKonfigurasi, aktif_semua};
    use super::*;

    fn kontrak(versi: i64) -> EndpointKontrak {
        EndpointKontrak {
            endpoint_id: Uuid::now_v7(),
            endpoint_config_version: versi,
            field_input: Vec::new(),
            field_metadata: vec![FieldMetadata {
                nama_field: "status_penerima_manfaat".to_owned(),
                tipe: "string".to_owned(),
                wajib: true,
                nilai_diizinkan: Some(vec!["sesuai".to_owned(), "tidak_sesuai".to_owned()]),
                deskripsi: "Sintetis.".to_owned(),
                versi: 1,
            }],
        }
    }

    #[tokio::test]
    async fn cache_hits_within_the_window_and_refetches_after_it() {
        let fake = Arc::new(FakeKonfigurasi::default());
        let k = kontrak(1);
        let (klien, ep) = (Uuid::now_v7(), k.endpoint_id);
        fake.taruh(klien, k.clone());
        let c = CachedKonfigurasi::with_ttl(fake.clone(), Duration::from_secs(30));
        let t0 = Instant::now();
        for detik in [0, 29] {
            let t = t0 + Duration::from_secs(detik);
            assert_eq!(c.muat(klien, ep, t).await.unwrap().kontrak, Some(k.clone()));
        }
        assert_eq!(fake.calls(), 1, "one query within the window");
        let t = t0 + Duration::from_secs(30);
        assert_eq!(c.muat(klien, ep, t).await.unwrap().kontrak, Some(k));
        assert_eq!(fake.calls(), 2, "refetched at the 30 s boundary");
    }

    #[test]
    fn default_ttl_is_at_most_thirty_seconds() {
        // The endpoint config shares the 30 s cap of ADR-0001 3a.4.
        assert!(CACHE_TTL <= Duration::from_secs(30));
    }

    #[tokio::test]
    async fn unassigned_is_cached_too_and_expires() {
        let fake = Arc::new(FakeKonfigurasi::default());
        let (klien, ep) = (Uuid::now_v7(), Uuid::now_v7());
        let c = CachedKonfigurasi::with_ttl(fake.clone(), Duration::from_secs(30));
        let t0 = Instant::now();
        for detik in [0, 10] {
            assert!(
                c.muat(klien, ep, t0 + Duration::from_secs(detik))
                    .await
                    .unwrap()
                    .kontrak
                    .is_none()
            );
        }
        assert_eq!(fake.calls(), 1, "404s are cached as well");
        fake.taruh(
            klien,
            EndpointKontrak {
                endpoint_id: ep,
                ..kontrak(1)
            },
        );
        assert!(
            c.muat(klien, ep, t0 + Duration::from_secs(29))
                .await
                .unwrap()
                .kontrak
                .is_none(),
            "still the cached 404"
        );
        assert!(
            c.muat(klien, ep, t0 + Duration::from_secs(31))
                .await
                .unwrap()
                .kontrak
                .is_some()
        );
    }

    #[tokio::test]
    async fn other_clients_do_not_share_the_cache_row() {
        let fake = Arc::new(FakeKonfigurasi::default());
        let k = kontrak(1);
        let (pemilik, lain) = (Uuid::now_v7(), Uuid::now_v7());
        fake.taruh(pemilik, k.clone());
        let c = CachedKonfigurasi::new(fake.clone());
        let t = Instant::now();
        assert!(
            c.muat(pemilik, k.endpoint_id, t)
                .await
                .unwrap()
                .kontrak
                .is_some()
        );
        assert!(
            c.muat(lain, k.endpoint_id, t)
                .await
                .unwrap()
                .kontrak
                .is_none()
        );
    }

    #[tokio::test]
    async fn store_errors_are_not_cached() {
        let fake = Arc::new(FakeKonfigurasi::default());
        let k = kontrak(1);
        let (klien, ep) = (Uuid::now_v7(), k.endpoint_id);
        *fake.fail.lock().unwrap() = true;
        let c = CachedKonfigurasi::new(fake.clone());
        let t = Instant::now();
        assert_eq!(c.muat(klien, ep, t).await, Err(StoreError));
        *fake.fail.lock().unwrap() = false;
        fake.taruh(klien, k.clone());
        assert_eq!(c.muat(klien, ep, t).await.unwrap().kontrak, Some(k));
    }

    #[tokio::test]
    async fn status_and_contract_share_one_cached_entry() {
        let fake = Arc::new(FakeKonfigurasi::default());
        let k = kontrak(1);
        let (klien, ep) = (Uuid::now_v7(), k.endpoint_id);
        fake.taruh(klien, k);
        let c = CachedKonfigurasi::new(fake.clone());
        let t = Instant::now();
        let e = c.muat(klien, ep, t).await.unwrap();
        assert_eq!(e.status, Some(aktif_semua()));
        assert!(e.kontrak.is_some());
        assert_eq!(fake.calls(), 1);
    }

    #[test]
    fn susun_kontrak_returns_none_without_rows() {
        assert_eq!(susun_kontrak(Vec::new()), Ok(None));
    }

    #[test]
    fn susun_kontrak_returns_none_without_an_active_version() {
        let rows = vec![BarisKontrak {
            downstream_endpoint_id: Uuid::now_v7(),
            versi: None,
            nama_field: Some("status_penerima_manfaat".to_owned()),
            tipe: Some("string".to_owned()),
            wajib: Some(true),
            nilai_diizinkan: None,
            deskripsi: Some("Sintetis.".to_owned()),
            versi_field: Some(1),
        }];
        assert_eq!(susun_kontrak(rows), Ok(None));
    }

    #[test]
    fn susun_kontrak_keeps_an_endpoint_without_metadata_fields() {
        let ep = Uuid::now_v7();
        let rows = vec![BarisKontrak {
            downstream_endpoint_id: ep,
            versi: Some(2),
            nama_field: None,
            tipe: None,
            wajib: None,
            nilai_diizinkan: None,
            deskripsi: None,
            versi_field: None,
        }];
        let k = susun_kontrak(rows).unwrap().unwrap();
        assert_eq!(k.endpoint_id, ep);
        assert_eq!(k.endpoint_config_version, 2);
        assert!(k.field_input.is_empty() && k.field_metadata.is_empty());
    }

    #[test]
    fn susun_kontrak_reads_nilai_diizinkan_and_null() {
        let ep = Uuid::now_v7();
        let baris = |nama: &str, nilai: Option<serde_json::Value>| BarisKontrak {
            downstream_endpoint_id: ep,
            versi: Some(1),
            nama_field: Some(nama.to_owned()),
            tipe: Some("string".to_owned()),
            wajib: Some(false),
            nilai_diizinkan: nilai,
            deskripsi: Some("Sintetis.".to_owned()),
            versi_field: Some(1),
        };
        let rows = vec![
            baris(
                "status_penerima_manfaat",
                Some(serde_json::json!(["sesuai", "tidak_sesuai"])),
            ),
            baris("referensi_client", Some(serde_json::Value::Null)),
        ];
        let k = susun_kontrak(rows).unwrap().unwrap();
        assert_eq!(
            k.field_metadata[0].nilai_diizinkan,
            Some(vec!["sesuai".to_owned(), "tidak_sesuai".to_owned()])
        );
        assert_eq!(k.field_metadata[1].nilai_diizinkan, None);
    }
}
