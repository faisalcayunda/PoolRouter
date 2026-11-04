//! Credential and client status as the webserver reads it (`pr_api` column
//! grants, migration 20261103000004), with the shared in-process cache of at
//! most 30 s (ADR-0001 rev 15 5a.5 and 3a.4).
//!
//! The cache applies uniformly: a revoked credential or a deactivated client is
//! refused at the latest 30 s after the change. Unknown ids are cached too, so
//! a flood of random ids cannot turn into a flood of queries; the map is bounded
//! and a full map simply stops caching new keys.

use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// Maximum age of a cached status (ADR-0001 5a.5, 3a.4).
pub const CACHE_TTL: Duration = Duration::from_secs(30);
const CACHE_MAX_ENTRIES: usize = 10_000;
/// Credential rows read per client; the schema allows two active ones.
const MAX_CREDENTIALS_PER_CLIENT: i64 = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Jenis {
    ClientSecret,
    ApiKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Kredensial {
    pub credential_id: Uuid,
    pub client_id: Uuid,
    pub jenis: Jenis,
    /// argon2id PHC text (client secret) or SHA-256 (API key). Never logged.
    pub kunci_hash: Vec<u8>,
    /// `status = 'aktif'` (false means `dicabut`, which is permanent).
    pub aktif: bool,
    pub kedaluwarsa_at: Option<DateTime<Utc>>,
}

impl Kredensial {
    /// Expiry only rejects new authentication (EN-02).
    pub fn belum_kedaluwarsa(&self, now: DateTime<Utc>) -> bool {
        self.kedaluwarsa_at.is_none_or(|t| t > now)
    }
}

/// The store could not answer. The reason is deliberately not carried: driver
/// messages can name hosts and users.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("credential store unavailable")]
pub struct StoreError;

pub type Fut<'a, T> = Pin<Box<dyn Future<Output = Result<T, StoreError>> + Send + 'a>>;

pub trait CredentialStore: Send + Sync + 'static {
    /// Newest credentials of a client (any kind, any status), at most 20.
    fn kredensial_client(&self, client_id: Uuid) -> Fut<'_, Vec<Kredensial>>;
    /// One credential by id, any status.
    fn kredensial(&self, credential_id: Uuid) -> Fut<'_, Option<Kredensial>>;
    /// `Some(aktif)` for a known client, `None` for an unknown id.
    fn client_aktif(&self, client_id: Uuid) -> Fut<'_, Option<bool>>;
}

pub struct PgCredentialStore {
    pool: PgPool,
}

impl PgCredentialStore {
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

fn baris_ke_kredensial(r: &sqlx::postgres::PgRow) -> Result<Kredensial, sqlx::Error> {
    let jenis: String = r.try_get("jenis")?;
    let status: String = r.try_get("status")?;
    Ok(Kredensial {
        credential_id: r.try_get("credential_id")?,
        client_id: r.try_get("client_id")?,
        jenis: if jenis == "api_key" {
            Jenis::ApiKey
        } else {
            Jenis::ClientSecret
        },
        kunci_hash: r.try_get("kunci_hash")?,
        aktif: status == "aktif",
        kedaluwarsa_at: r.try_get("kedaluwarsa_at")?,
    })
}

fn gagal(op: &'static str) -> StoreError {
    // Static text only: the driver error may carry connection details.
    tracing::warn!(op, "credential store query failed");
    StoreError
}

impl CredentialStore for PgCredentialStore {
    fn kredensial_client(&self, client_id: Uuid) -> Fut<'_, Vec<Kredensial>> {
        Box::pin(async move {
            let rows = sqlx::query(
                "SELECT credential_id, client_id, jenis, kunci_hash, status, kedaluwarsa_at \
                 FROM client_kredensial WHERE client_id = $1 \
                 ORDER BY dibuat_at DESC LIMIT $2",
            )
            .bind(client_id)
            .bind(MAX_CREDENTIALS_PER_CLIENT)
            .fetch_all(&self.pool)
            .await
            .map_err(|_| gagal("kredensial_client"))?;
            rows.iter()
                .map(baris_ke_kredensial)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| gagal("kredensial_client"))
        })
    }

    fn kredensial(&self, credential_id: Uuid) -> Fut<'_, Option<Kredensial>> {
        Box::pin(async move {
            let row = sqlx::query(
                "SELECT credential_id, client_id, jenis, kunci_hash, status, kedaluwarsa_at \
                 FROM client_kredensial WHERE credential_id = $1",
            )
            .bind(credential_id)
            .fetch_optional(&self.pool)
            .await
            .map_err(|_| gagal("kredensial"))?;
            row.as_ref()
                .map(baris_ke_kredensial)
                .transpose()
                .map_err(|_| gagal("kredensial"))
        })
    }

    fn client_aktif(&self, client_id: Uuid) -> Fut<'_, Option<bool>> {
        Box::pin(async move {
            let row = sqlx::query("SELECT aktif FROM client WHERE client_id = $1")
                .bind(client_id)
                .fetch_optional(&self.pool)
                .await
                .map_err(|_| gagal("client_aktif"))?;
            row.map(|r| r.try_get::<bool, _>("aktif"))
                .transpose()
                .map_err(|_| gagal("client_aktif"))
        })
    }
}

/// Bounded map whose entries expire `ttl` after they were stored.
#[derive(Debug)]
pub struct TtlCache<K, V> {
    ttl: Duration,
    map: Mutex<HashMap<K, (Instant, V)>>,
}

impl<K: Eq + Hash, V: Clone> TtlCache<K, V> {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            map: Mutex::new(HashMap::new()),
        }
    }

    pub fn get(&self, key: &K, now: Instant) -> Option<V> {
        let map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        map.get(key)
            .filter(|(at, _)| now.saturating_duration_since(*at) < self.ttl)
            .map(|(_, v)| v.clone())
    }

    pub fn put(&self, key: K, value: V, now: Instant) {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() >= CACHE_MAX_ENTRIES && !map.contains_key(&key) {
            let ttl = self.ttl;
            map.retain(|_, (at, _)| now.saturating_duration_since(*at) < ttl);
            if map.len() >= CACHE_MAX_ENTRIES {
                return; // full of live entries: do not cache, do not grow
            }
        }
        map.insert(key, (now, value));
    }

    pub fn len(&self) -> usize {
        self.map.lock().unwrap_or_else(|e| e.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// [`CredentialStore`] with a 30 s cache in front of `inner`.
pub struct CachedStore {
    inner: Arc<dyn CredentialStore>,
    per_client: TtlCache<Uuid, Vec<Kredensial>>,
    per_id: TtlCache<Uuid, Option<Kredensial>>,
    client: TtlCache<Uuid, Option<bool>>,
}

impl CachedStore {
    pub fn new(inner: Arc<dyn CredentialStore>) -> Self {
        Self::with_ttl(inner, CACHE_TTL)
    }

    pub fn with_ttl(inner: Arc<dyn CredentialStore>, ttl: Duration) -> Self {
        Self {
            inner,
            per_client: TtlCache::new(ttl),
            per_id: TtlCache::new(ttl),
            client: TtlCache::new(ttl),
        }
    }
}

impl CredentialStore for CachedStore {
    fn kredensial_client(&self, client_id: Uuid) -> Fut<'_, Vec<Kredensial>> {
        Box::pin(async move {
            if let Some(v) = self.per_client.get(&client_id, Instant::now()) {
                return Ok(v);
            }
            let v = self.inner.kredensial_client(client_id).await?;
            self.per_client.put(client_id, v.clone(), Instant::now());
            Ok(v)
        })
    }

    fn kredensial(&self, credential_id: Uuid) -> Fut<'_, Option<Kredensial>> {
        Box::pin(async move {
            if let Some(v) = self.per_id.get(&credential_id, Instant::now()) {
                return Ok(v);
            }
            let v = self.inner.kredensial(credential_id).await?;
            self.per_id.put(credential_id, v.clone(), Instant::now());
            Ok(v)
        })
    }

    fn client_aktif(&self, client_id: Uuid) -> Fut<'_, Option<bool>> {
        Box::pin(async move {
            if let Some(v) = self.client.get(&client_id, Instant::now()) {
                return Ok(v);
            }
            let v = self.inner.client_aktif(client_id).await?;
            self.client.put(client_id, v, Instant::now());
            Ok(v)
        })
    }
}

#[cfg(test)]
pub(crate) mod fake {
    //! In-memory store for handler and auth tests; counts calls.
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    pub struct FakeStore {
        pub kredensial: Mutex<Vec<Kredensial>>,
        pub clients: Mutex<HashMap<Uuid, bool>>,
        pub fail: Mutex<bool>,
        pub calls: AtomicUsize,
    }

    impl FakeStore {
        pub fn calls(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
        fn hit(&self) -> Result<(), StoreError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if *self.fail.lock().unwrap() {
                Err(StoreError)
            } else {
                Ok(())
            }
        }
    }

    impl CredentialStore for FakeStore {
        fn kredensial_client(&self, client_id: Uuid) -> Fut<'_, Vec<Kredensial>> {
            Box::pin(async move {
                self.hit()?;
                Ok(self
                    .kredensial
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|k| k.client_id == client_id)
                    .cloned()
                    .collect())
            })
        }
        fn kredensial(&self, credential_id: Uuid) -> Fut<'_, Option<Kredensial>> {
            Box::pin(async move {
                self.hit()?;
                Ok(self
                    .kredensial
                    .lock()
                    .unwrap()
                    .iter()
                    .find(|k| k.credential_id == credential_id)
                    .cloned())
            })
        }
        fn client_aktif(&self, client_id: Uuid) -> Fut<'_, Option<bool>> {
            Box::pin(async move {
                self.hit()?;
                Ok(self.clients.lock().unwrap().get(&client_id).copied())
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fake::FakeStore;
    use super::*;

    #[test]
    fn ttl_cache_expires_after_ttl_and_bounds_its_size() {
        let c: TtlCache<u32, u32> = TtlCache::new(Duration::from_secs(30));
        let t0 = Instant::now();
        c.put(1, 10, t0);
        assert_eq!(c.get(&1, t0 + Duration::from_secs(29)), Some(10));
        assert_eq!(
            c.get(&1, t0 + Duration::from_secs(30)),
            None,
            "30 s is the cap"
        );
        for i in 0..(CACHE_MAX_ENTRIES as u32 + 500) {
            c.put(i + 100, i, t0);
        }
        assert_eq!(
            c.len(),
            CACHE_MAX_ENTRIES,
            "full map of live entries stops growing"
        );
        // After expiry the map makes room again.
        c.put(999_999, 1, t0 + Duration::from_secs(31));
        assert!(c.len() < CACHE_MAX_ENTRIES);
    }

    #[tokio::test]
    async fn cached_store_hits_the_database_once_within_the_window() {
        let fake = Arc::new(FakeStore::default());
        let id = Uuid::now_v7();
        fake.clients.lock().unwrap().insert(id, true);
        let store = CachedStore::new(fake.clone());
        for _ in 0..5 {
            assert_eq!(store.client_aktif(id).await, Ok(Some(true)));
        }
        assert_eq!(fake.calls(), 1);
        // Unknown ids are cached as well.
        for _ in 0..5 {
            assert_eq!(store.client_aktif(Uuid::nil()).await, Ok(None));
        }
        assert_eq!(fake.calls(), 2);
    }

    #[tokio::test]
    async fn deactivation_is_seen_after_the_cache_window() {
        let fake = Arc::new(FakeStore::default());
        let id = Uuid::now_v7();
        fake.clients.lock().unwrap().insert(id, true);
        let store = CachedStore::with_ttl(fake.clone(), Duration::from_millis(40));
        assert_eq!(store.client_aktif(id).await, Ok(Some(true)));
        fake.clients.lock().unwrap().insert(id, false);
        assert_eq!(store.client_aktif(id).await, Ok(Some(true)), "still cached");
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(store.client_aktif(id).await, Ok(Some(false)));
    }

    #[tokio::test]
    async fn store_errors_are_not_cached() {
        let fake = Arc::new(FakeStore::default());
        let id = Uuid::now_v7();
        *fake.fail.lock().unwrap() = true;
        let store = CachedStore::new(fake.clone());
        assert_eq!(store.client_aktif(id).await, Err(StoreError));
        *fake.fail.lock().unwrap() = false;
        fake.clients.lock().unwrap().insert(id, true);
        assert_eq!(store.client_aktif(id).await, Ok(Some(true)));
    }

    #[test]
    fn expiry_rejects_only_when_set_and_past() {
        let now = Utc::now();
        let mut k = Kredensial {
            credential_id: Uuid::now_v7(),
            client_id: Uuid::now_v7(),
            jenis: Jenis::ApiKey,
            kunci_hash: vec![],
            aktif: true,
            kedaluwarsa_at: None,
        };
        assert!(k.belum_kedaluwarsa(now));
        k.kedaluwarsa_at = Some(now - chrono::Duration::seconds(1));
        assert!(!k.belum_kedaluwarsa(now));
        k.kedaluwarsa_at = Some(now + chrono::Duration::seconds(1));
        assert!(k.belum_kedaluwarsa(now));
    }
}
