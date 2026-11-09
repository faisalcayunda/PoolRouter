//! Downstream request lifecycle, read side: the store behind `GET /v1/endpoints/
//! {endpoint_id}/kontrak` (ARC-187, AC-9 of `downstream-api.yaml`).
//!
//! Only the `pr_api` pool is reachable from here; the admin pool never enters
//! the downstream router (ADR-0001 7.2). Error text is fixed by
//! [`GalatDownstream`] and nothing carries driver or query detail to the client.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use sqlx::{PgPool, Row};
use uuid::Uuid;

/// Client identity resolved by the authentication layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SesiKlien {
    pub client_id: Uuid,
    pub credential_id: Uuid,
}

/// One input field of an endpoint contract (contract section 4).
#[derive(Debug, Clone, PartialEq)]
pub struct BidangInput {
    pub nama: String,
    pub tipe: String,
    pub wajib: bool,
    pub nilai_diizinkan: Option<serde_json::Value>,
    pub aturan_validasi: Option<serde_json::Value>,
    pub klasifikasi: String,
    pub kirim_ke_data_balikan: bool,
    pub deskripsi: String,
    pub versi: i32,
}

/// The contract of one endpoint version.
#[derive(Debug, Clone, PartialEq)]
pub struct KontrakEndpoint {
    pub endpoint_id: Uuid,
    pub config_versi_aktif: i32,
    /// Kontrak input endpoint (ADR-0002 rev 5 D6): nama `{{input.*}}` di
    /// template provider versi aktif, fungsi yang sama dengan intake.
    pub field_input: Vec<super::intake::FieldInputKontrak>,
    pub input: Vec<BidangInput>,
}

/// Why a store call did not produce a value. Maps 1:1 onto the documented
/// problem responses; the handlers add the client-facing text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GalatDownstream {
    /// The endpoint is unknown or not assigned to this client. Both collapse
    /// into one 404 on purpose (AC-9).
    EndpointTidakDikenal,
    /// The endpoint, its pool or its provider is switched off (403).
    ObjekNonaktif,
    /// A request id that does not exist or belongs to another client.
    RequestTidakDikenal,
    /// The store is down. Never carries detail.
    Layanan,
}

pub type Hasil<T> = Result<T, GalatDownstream>;

pub type Fut<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// Persistence for the downstream operations.
pub trait SessionDownstream: Send + Sync + 'static {
    /// Contract of an endpoint assigned to `klien`, or 404.
    fn kontrak(&self, klien: SesiKlien, endpoint: Uuid) -> Fut<'_, Hasil<KontrakEndpoint>>;

    /// Whether the endpoint is assigned to `klien` and every object in its chain
    /// is on. `None` means not assigned (404, never 403); `Some(false)` means
    /// assigned but switched off (403). The contract still answers for both.
    fn status_rantai(&self, klien: SesiKlien, endpoint: Uuid) -> Fut<'_, Hasil<Option<bool>>>;
}

/// PostgreSQL implementation. The `pr_api` role owns every statement here.
#[derive(Clone)]
pub struct PgSessionDownstream {
    pool: PgPool,
}

impl PgSessionDownstream {
    pub fn baru(pool: PgPool) -> Arc<Self> {
        Arc::new(Self { pool })
    }

    /// Whether the endpoint is assigned to the client and every object in its
    /// chain is on. `None` means it is not assigned at all (404, never 403).
    async fn status_rantai_dalam(&self, klien: SesiKlien, endpoint: Uuid) -> Hasil<Option<bool>> {
        let hidup = sqlx::query_scalar::<_, bool>(
            "SELECT c.aktif AND e.aktif AND p.aktif AND pr.aktif \
               FROM downstream_endpoint e \
               JOIN client_endpoint ce USING (downstream_endpoint_id) \
               JOIN client c ON c.client_id = ce.client_id \
               JOIN downstream_pool p USING (downstream_pool_id) \
               JOIN provider pr USING (provider_id) \
              WHERE e.downstream_endpoint_id = $1 AND ce.client_id = $2",
        )
        .bind(endpoint)
        .bind(klien.client_id)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| GalatDownstream::Layanan)?;
        Ok(hidup)
    }

    async fn kontrak_dalam(&self, klien: SesiKlien, endpoint: Uuid) -> Hasil<KontrakEndpoint> {
        // AC-9 / GW3: an endpoint that is assigned to the caller answers 200 even
        // while it is inactive, because the contract is what lets the client
        // prepare a request before the endpoint is switched back on. Only an
        // unknown endpoint, or one not assigned to this client, is 404 — and both
        // produce the same body.
        if self.status_rantai_dalam(klien, endpoint).await?.is_none() {
            return Err(GalatDownstream::EndpointTidakDikenal);
        }
        let baris = sqlx::query(
            "SELECT e.config_versi_aktif, pcv.dokumen->>'body_template' AS body_template \
               FROM downstream_endpoint e \
               JOIN provider p ON p.provider_id = e.provider_id \
               LEFT JOIN provider_config_versi pcv \
                      ON pcv.provider_id = p.provider_id AND pcv.versi = p.config_versi_aktif \
              WHERE e.downstream_endpoint_id = $1",
        )
        .bind(endpoint)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| GalatDownstream::Layanan)?
        .ok_or(GalatDownstream::EndpointTidakDikenal)?;
        let versi: Option<i32> = baris
            .try_get("config_versi_aktif")
            .map_err(|_| GalatDownstream::Layanan)?;
        // No active config version means the endpoint is not fully provisioned
        // yet; report it as a store problem rather than fabricating version 0.
        let versi = versi.ok_or(GalatDownstream::Layanan)?;
        let template: Option<String> = baris
            .try_get("body_template")
            .map_err(|_| GalatDownstream::Layanan)?;
        let field_input =
            super::permintaan::field_input_dari_template(template.as_deref().unwrap_or_default());
        let input = sqlx::query(
            "SELECT nama_field, tipe, wajib, nilai_diizinkan, aturan_validasi, klasifikasi, \
                    kirim_ke_data_balikan, deskripsi, versi \
               FROM endpoint_metadata_field WHERE downstream_endpoint_id = $1 ORDER BY nama_field",
        )
        .bind(endpoint)
        .fetch_all(&self.pool)
        .await
        .map_err(|_| GalatDownstream::Layanan)?
        .into_iter()
        .map(|r| {
            Ok::<_, sqlx::Error>(BidangInput {
                nama: r.try_get("nama_field")?,
                tipe: r.try_get("tipe")?,
                wajib: r.try_get("wajib")?,
                nilai_diizinkan: r
                    .try_get::<Option<sqlx::types::Json<serde_json::Value>>, _>("nilai_diizinkan")?
                    .map(|j| j.0),
                aturan_validasi: r
                    .try_get::<Option<sqlx::types::Json<serde_json::Value>>, _>("aturan_validasi")?
                    .map(|j| j.0),
                klasifikasi: r.try_get("klasifikasi")?,
                kirim_ke_data_balikan: r.try_get("kirim_ke_data_balikan")?,
                deskripsi: r.try_get("deskripsi")?,
                versi: r.try_get("versi")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(|_| GalatDownstream::Layanan)?;
        Ok(KontrakEndpoint {
            endpoint_id: endpoint,
            config_versi_aktif: versi,
            field_input,
            input,
        })
    }
}

impl SessionDownstream for PgSessionDownstream {
    fn kontrak(&self, klien: SesiKlien, endpoint: Uuid) -> Fut<'_, Hasil<KontrakEndpoint>> {
        Box::pin(async move { self.kontrak_dalam(klien, endpoint).await })
    }

    fn status_rantai(&self, klien: SesiKlien, endpoint: Uuid) -> Fut<'_, Hasil<Option<bool>>> {
        Box::pin(async move { self.status_rantai_dalam(klien, endpoint).await })
    }
}

pub mod fake {
    //! In-memory store for router-level tests, also used by `tests/`: no
    //! database, no sleeps.

    use super::*;

    #[derive(Default)]
    pub struct FakeSession {
        /// Set once, before the router is built; never mutated concurrently.
        endpoint_set: Option<Uuid>,
        bidang_set: Vec<BidangInput>,
        /// Whether `status_rantai` reports the endpoint as live.
        aktif: bool,
        /// Makes every call answer `Err(Layanan)` (the store is down).
        mati: Mutex<bool>,
    }

    use std::sync::Mutex;

    impl FakeSession {
        pub fn baru() -> Arc<Self> {
            Arc::new(Self::default())
        }

        /// Points the double at one endpoint and its metadata contract. Called
        /// through `Arc::get_mut` before the router is built, so no lock and no
        /// interior mutability are needed here.
        pub fn atur(&mut self, endpoint: Uuid, bidang: Vec<BidangInput>) {
            self.endpoint_set = Some(endpoint);
            self.bidang_set = bidang;
            self.aktif = true;
        }

        /// `Some(true)` for the configured endpoint, `None` for any other id.
        fn status(&self, endpoint: Uuid) -> Option<bool> {
            (self.endpoint_set == Some(endpoint)).then_some(self.aktif)
        }

        /// Switches the configured endpoint off; the contract still answers.
        pub fn nonaktifkan(&mut self) {
            self.aktif = false;
        }

        /// Makes every store call answer `Layanan`, for the 503 path.
        pub fn mati(&self) {
            *self.mati.lock().unwrap() = true;
        }

        fn galat(&self) -> Hasil<()> {
            if *self.mati.lock().unwrap() {
                Err(GalatDownstream::Layanan)
            } else {
                Ok(())
            }
        }
    }

    impl SessionDownstream for FakeSession {
        fn kontrak(&self, _klien: SesiKlien, endpoint: Uuid) -> Fut<'_, Hasil<KontrakEndpoint>> {
            Box::pin(async move {
                self.galat()?;
                // AC-9 / GW3: inactive still answers, only unknown is 404.
                if self.status(endpoint).is_none() {
                    return Err(GalatDownstream::EndpointTidakDikenal);
                }
                Ok(KontrakEndpoint {
                    endpoint_id: endpoint,
                    config_versi_aktif: 3,
                    field_input: Vec::new(),
                    input: self.bidang_set.clone(),
                })
            })
        }

        fn status_rantai(&self, _klien: SesiKlien, endpoint: Uuid) -> Fut<'_, Hasil<Option<bool>>> {
            Box::pin(async move {
                self.galat()?;
                Ok(self.status(endpoint))
            })
        }
    }
}
