//! Downstream request lifecycle: the store behind the five operations of
//! `downstream-api.yaml` (ARC-158, AC-1, AC-2, AC-3, AC-9).
//!
//! Only the `pr_api` pool is reachable from here; the admin pool never enters
//! the downstream router (ADR-0001 7.2). Error text is fixed by
//! [`GalatDownstream`] and nothing carries driver or query detail to the client.

use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;

use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::api::enums::ApiStatus;
use poolrouter_core::status_api::{StatusApi, StatusInternal, StatusTerpetakan, status_api};

/// The wire enum of `downstream-api.yaml`; `poolrouter_core` owns the mapping
/// and this crate owns the schema, so the two are joined here once.
fn ke_api(s: StatusApi) -> crate::api::enums::ApiStatus {
    match s {
        StatusApi::Queued => crate::api::enums::ApiStatus::Queued,
        StatusApi::Processing => crate::api::enums::ApiStatus::Processing,
        StatusApi::Done => crate::api::enums::ApiStatus::Done,
        StatusApi::Failed => crate::api::enums::ApiStatus::Failed,
        StatusApi::Expired => crate::api::enums::ApiStatus::Expired,
        StatusApi::Cancelled => crate::api::enums::ApiStatus::Cancelled,
    }
}

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
    pub nama: String,
    pub config_versi_aktif: i32,
    pub input: Vec<BidangInput>,
}

/// A request row reduced to what the API shows.
#[derive(Debug, Clone, PartialEq)]
pub struct BarisRequest {
    pub request_id: Uuid,
    /// Already mapped to the six API statuses (ADR-0001 4.1); `None` means the
    /// request was rejected and is not pollable (S-02).
    pub status: Option<ApiStatus>,
    pub mode: String,
    pub ttl_detik: Option<i64>,
    pub hasil: Option<serde_json::Value>,
    pub failure_reason: Option<String>,
    pub cancel_reason: Option<String>,
}

/// Why a store call did not produce a value. Maps 1:1 onto the documented
/// problem responses; the handlers add the client-facing text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GalatDownstream {
    /// The endpoint is unknown or not assigned to this client. Both collapse
    /// into one 404 on purpose (AC-3).
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
    /// Contract of an active endpoint assigned to `klien`, or 404.
    fn kontrak(&self, klien: SesiKlien, endpoint: Uuid) -> Fut<'_, Hasil<KontrakEndpoint>>;

    /// Queues an async request. Returns `(row, created)`: `created` is false on
    /// an idempotency hit, and the row is the one already stored.
    fn antre(&self, permintaan: AntrePermintaan) -> Fut<'_, Hasil<(BarisRequest, bool)>>;

    /// Reads one request owned by `klien`.
    fn request(&self, klien: SesiKlien, request: Uuid) -> Fut<'_, Hasil<BarisRequest>>;

    /// Pulls back a queued async request owned by `klien`.
    ///
    /// Returns `Ok((row, true))` when the cancel went through and
    /// `Ok((row, false))` when the request is `processing` or terminal; the
    /// caller turns the second case into 409 with `row.status`.
    fn batalkan(&self, klien: SesiKlien, request: Uuid) -> Fut<'_, Hasil<(BarisRequest, bool)>>;
}

/// Everything the store needs to queue a request.
#[derive(Debug, Clone)]
pub struct AntrePermintaan {
    pub klien: SesiKlien,
    pub endpoint_id: Uuid,
    /// Validated metadata, keyed by input field name.
    pub metadata: serde_json::Value,
    pub referensi_client: Option<String>,
    pub idempotency_key: Option<String>,
    /// SHA-256 hex of the canonical request; the schema keeps 64 hex chars.
    pub idempotency_fingerprint: Option<String>,
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

    /// Reads one request through any executor (pool or open transaction).
    async fn baca(
        conn: &mut sqlx::PgConnection,
        klien: SesiKlien,
        request: Uuid,
    ) -> Hasil<BarisRequest> {
        let r = sqlx::query(
            "SELECT request_id, status, attempt_count, mode, output_payload, failure_reason, \
                    cancel_reason, \
                    greatest(0, extract(epoch FROM (ttl_expires_at - now()))::bigint)::bigint AS ttl_detik \
               FROM request WHERE request_id = $1 AND client_id = $2",
        )
        .bind(request)
        .bind(klien.client_id)
        .fetch_optional(conn)
        .await
        .map_err(|_| GalatDownstream::Layanan)?
        .ok_or(GalatDownstream::RequestTidakDikenal)?;
        let internal: String = r.try_get("status").map_err(|_| GalatDownstream::Layanan)?;
        let attempt: i32 = r
            .try_get("attempt_count")
            .map_err(|_| GalatDownstream::Layanan)?;
        let status =
            internal
                .parse::<StatusInternal>()
                .ok()
                .and_then(|s| match status_api(s, attempt) {
                    StatusTerpetakan::Status(api) => Some(ke_api(api)),
                    StatusTerpetakan::TidakBisaDipolling => None,
                });
        // A rejected request has no `queue_id` to poll (S-02).
        if status.is_none() {
            return Err(GalatDownstream::RequestTidakDikenal);
        }
        Ok(BarisRequest {
            request_id: r
                .try_get("request_id")
                .map_err(|_| GalatDownstream::Layanan)?,
            status,
            mode: r.try_get("mode").map_err(|_| GalatDownstream::Layanan)?,
            ttl_detik: r.try_get("ttl_detik").ok(),
            hasil: r
                .try_get("output_payload")
                .map_err(|_| GalatDownstream::Layanan)?,
            failure_reason: r
                .try_get("failure_reason")
                .map_err(|_| GalatDownstream::Layanan)?,
            cancel_reason: r
                .try_get("cancel_reason")
                .map_err(|_| GalatDownstream::Layanan)?,
        })
    }

    /// Whether the endpoint is assigned to the client and every object in its
    /// chain is on. `None` means it is not assigned at all (404, never 403).
    async fn status_rantai(&self, klien: SesiKlien, endpoint: Uuid) -> Hasil<Option<bool>> {
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
}

impl SessionDownstream for PgSessionDownstream {
    fn kontrak(&self, klien: SesiKlien, endpoint: Uuid) -> Fut<'_, Hasil<KontrakEndpoint>> {
        Box::pin(async move { self.kontrak_dalam(klien, endpoint).await })
    }

    fn antre(&self, permintaan: AntrePermintaan) -> Fut<'_, Hasil<(BarisRequest, bool)>> {
        Box::pin(async move { self.antre_dalam(permintaan).await })
    }

    fn request(&self, klien: SesiKlien, request: Uuid) -> Fut<'_, Hasil<BarisRequest>> {
        Box::pin(async move {
            let mut conn = self
                .pool
                .acquire()
                .await
                .map_err(|_| GalatDownstream::Layanan)?;
            Self::baca(&mut conn, klien, request).await
        })
    }

    fn batalkan(&self, klien: SesiKlien, request: Uuid) -> Fut<'_, Hasil<(BarisRequest, bool)>> {
        Box::pin(async move { self.batalkan_dalam(klien, request).await })
    }
}

impl PgSessionDownstream {
    async fn kontrak_dalam(&self, klien: SesiKlien, endpoint: Uuid) -> Hasil<KontrakEndpoint> {
        // AC-9 / GW3: an endpoint that is assigned to the caller answers 200 even
        // while it is inactive, because the contract is what lets the client
        // prepare a request before the endpoint is switched back on. Only an
        // unknown endpoint, or one not assigned to this client, is 404 — and both
        // produce the same body.
        if self.status_rantai(klien, endpoint).await?.is_none() {
            return Err(GalatDownstream::EndpointTidakDikenal);
        }
        let baris = sqlx::query(
            "SELECT nama, config_versi_aktif FROM downstream_endpoint \
              WHERE downstream_endpoint_id = $1",
        )
        .bind(endpoint)
        .fetch_optional(&self.pool)
        .await
        .map_err(|_| GalatDownstream::Layanan)?
        .ok_or(GalatDownstream::EndpointTidakDikenal)?;
        let nama: String = baris
            .try_get("nama")
            .map_err(|_| GalatDownstream::Layanan)?;
        let versi: Option<i32> = baris
            .try_get("config_versi_aktif")
            .map_err(|_| GalatDownstream::Layanan)?;
        let versi = versi.ok_or(GalatDownstream::ObjekNonaktif)?;
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
                nilai_diizinkan: r.try_get("nilai_diizinkan")?,
                aturan_validasi: r.try_get("aturan_validasi")?,
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
            nama,
            config_versi_aktif: versi,
            input,
        })
    }

    async fn antre_dalam(&self, permintaan: AntrePermintaan) -> Hasil<(BarisRequest, bool)> {
        match self
            .status_rantai(permintaan.klien, permintaan.endpoint_id)
            .await?
        {
            None => return Err(GalatDownstream::EndpointTidakDikenal),
            Some(false) => return Err(GalatDownstream::ObjekNonaktif),
            Some(true) => {}
        }
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| GalatDownstream::Layanan)?;
        // Endpoint configuration, pool and provider version all come from the
        // configuration tables; never from the request body.
        let inserted = sqlx::query(
            "INSERT INTO request (request_id, client_id, credential_id, downstream_endpoint_id, \
                                  downstream_pool_id, provider_id, provider_config_version, \
                                  endpoint_config_version, mode, prioritas, prioritas_efektif, status, \
                                  idempotency_key, idempotency_fingerprint, referensi_client, \
                                  metadata, received_at, ttl_expires_at, dispatch_key) \
             SELECT $1, $2, $3, e.downstream_endpoint_id, e.downstream_pool_id, e.provider_id, \
                    coalesce(pr.config_versi_aktif, 1), e.config_versi_aktif, 'async', p.prioritas, \
                    p.prioritas, 'queued', $5, $6, $7, $4, now(), \
                    now() + make_interval(secs => p.ttl_antrean_detik), gen_random_uuid() \
               FROM downstream_endpoint e \
               JOIN downstream_pool p USING (downstream_pool_id) \
               JOIN provider pr USING (provider_id) \
              WHERE e.downstream_endpoint_id = $8 AND e.config_versi_aktif IS NOT NULL \
             ON CONFLICT (client_id, idempotency_key) WHERE idempotency_key IS NOT NULL DO NOTHING \
             RETURNING request_id",
        )
        .bind(Uuid::now_v7())
        .bind(permintaan.klien.client_id)
        .bind(permintaan.klien.credential_id)
        .bind(&permintaan.metadata)
        .bind(&permintaan.idempotency_key)
        .bind(&permintaan.idempotency_fingerprint)
        .bind(&permintaan.referensi_client)
        .bind(permintaan.endpoint_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| GalatDownstream::Layanan)?;
        let dibuat = inserted.is_some();
        let request_id: Uuid = match inserted {
            Some(r) => r
                .try_get("request_id")
                .map_err(|_| GalatDownstream::Layanan)?,
            None => {
                // Idempotency hit: replay the row already stored for that key.
                let kunci = permintaan.idempotency_key.clone().unwrap_or_default();
                sqlx::query_scalar::<_, Uuid>(
                    "SELECT request_id FROM request WHERE client_id = $1 AND idempotency_key = $2",
                )
                .bind(permintaan.klien.client_id)
                .bind(&kunci)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|_| GalatDownstream::Layanan)?
                .ok_or(GalatDownstream::Layanan)?
            }
        };
        // T-04: `request_history_wajib` is a deferred constraint trigger, so the
        // `queued` row is only legal together with the first history row. Both
        // land in the same transaction or neither does.
        if dibuat {
            sqlx::query(
                "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, aktor) \
                 VALUES ($1, 1, NULL, 'queued', 'webserver')",
            )
            .bind(request_id)
            .execute(&mut *tx)
            .await
            .map_err(|_| GalatDownstream::Layanan)?;
        }
        let baris = Self::baca(&mut tx, permintaan.klien, request_id).await?;
        tx.commit().await.map_err(|_| GalatDownstream::Layanan)?;
        Ok((baris, dibuat))
    }

    async fn batalkan_dalam(&self, klien: SesiKlien, request: Uuid) -> Hasil<(BarisRequest, bool)> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|_| GalatDownstream::Layanan)?;
        let row = sqlx::query(
            "SELECT status, attempt_count FROM request \
              WHERE request_id = $1 AND client_id = $2 FOR UPDATE",
        )
        .bind(request)
        .bind(klien.client_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|_| GalatDownstream::Layanan)?
        .ok_or(GalatDownstream::RequestTidakDikenal)?;
        let internal: String = row
            .try_get("status")
            .map_err(|_| GalatDownstream::Layanan)?;
        let attempt: i32 = row
            .try_get("attempt_count")
            .map_err(|_| GalatDownstream::Layanan)?;
        // Only `queued` (never sent to a provider) can still be pulled back.
        let bisa = matches!(
            internal
                .parse::<StatusInternal>()
                .ok()
                .map(|s| status_api(s, attempt)),
            Some(StatusTerpetakan::Status(StatusApi::Queued))
        );
        if !bisa {
            let baris = Self::baca(&mut tx, klien, request).await?;
            tx.rollback().await.ok();
            return Ok((baris, false));
        }
        // An async row that reaches a terminal status must also record where its
        // webhook stands (`chk_webhook`), or have one pending; a client without a
        // webhook reads `tidak_dikirim`, the same rule `fn_putusan_*` applies.
        sqlx::query(
            "UPDATE request SET status = 'cancelled', cancel_reason = 'dibatalkan_client', \
                    cancelled_by = 'client', completed_at = now(), \
                    webhook_status = CASE WHEN mode = 'async' THEN \
                        (CASE WHEN EXISTS (SELECT 1 FROM client_webhook w \
                                            WHERE w.client_id = $2 AND w.aktif) \
                              THEN 'menunggu' ELSE 'tidak_dikirim' END) \
                        ELSE NULL END \
              WHERE request_id = $1",
        )
        .bind(request)
        .bind(klien.client_id)
        .execute(&mut *tx)
        .await
        .map_err(|_| GalatDownstream::Layanan)?;
        // T-04: every status change needs its history row, in the same
        // transaction, with the next `seq`.
        sqlx::query(
            "INSERT INTO request_status_history (request_id, seq, status_dari, status_ke, alasan, aktor) \
             SELECT $1, coalesce(max(seq), 0) + 1, 'queued', 'cancelled', 'dibatalkan_client', 'client' \
               FROM request_status_history WHERE request_id = $1",
        )
        .bind(request)
        .execute(&mut *tx)
        .await
        .map_err(|_| GalatDownstream::Layanan)?;
        let baris = Self::baca(&mut tx, klien, request).await?;
        tx.commit().await.map_err(|_| GalatDownstream::Layanan)?;
        Ok((baris, true))
    }
}

pub mod fake {
    //! In-memory store for router-level tests, also used by `tests/`: no
    //! database, no sleeps.

    use std::collections::HashMap;
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    pub struct FakeSession {
        /// Set once, before the router is built; never mutated concurrently.
        endpoint_set: Option<Uuid>,
        bidang_set: Vec<BidangInput>,
        /// Whether `status_rantai` reports the endpoint as live.
        aktif: bool,
        baris: Mutex<HashMap<Uuid, BarisRequest>>,
        kunci: Mutex<HashMap<String, Uuid>>,
        /// Makes every call answer `Err(Layanan)` (the store is down).
        mati: Mutex<bool>,
    }

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

        /// Switches the endpoint off, as `deactivateEndpoint` would.
        pub fn nonaktifkan(&mut self) {
            self.aktif = false;
        }

        /// `Some(true)` for the configured endpoint, `None` for any other id.
        fn status(&self, endpoint: Uuid) -> Option<bool> {
            (self.endpoint_set == Some(endpoint)).then_some(self.aktif)
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

    fn contoh(id: Uuid, status: Option<ApiStatus>) -> BarisRequest {
        BarisRequest {
            request_id: id,
            status,
            mode: "async".into(),
            ttl_detik: Some(86_400),
            hasil: None,
            failure_reason: None,
            cancel_reason: None,
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
                    nama: "contoh".into(),
                    config_versi_aktif: 3,
                    input: self.bidang_set.clone(),
                })
            })
        }

        fn antre(&self, p: AntrePermintaan) -> Fut<'_, Hasil<(BarisRequest, bool)>> {
            Box::pin(async move {
                self.galat()?;
                match self.status(p.endpoint_id) {
                    None => return Err(GalatDownstream::EndpointTidakDikenal),
                    Some(false) => return Err(GalatDownstream::ObjekNonaktif),
                    Some(true) => {}
                }
                if let Some(k) = &p.idempotency_key
                    && let Some(id) = self.kunci.lock().unwrap().get(k).copied()
                    && let Some(b) = self.baris.lock().unwrap().get(&id).cloned()
                {
                    return Ok((b, false));
                }
                let id = Uuid::now_v7();
                let baris = contoh(id, Some(ApiStatus::Queued));
                self.baris.lock().unwrap().insert(id, baris.clone());
                if let Some(k) = &p.idempotency_key {
                    self.kunci.lock().unwrap().insert(k.clone(), id);
                }
                Ok((baris, true))
            })
        }

        fn request(&self, _klien: SesiKlien, request: Uuid) -> Fut<'_, Hasil<BarisRequest>> {
            Box::pin(async move {
                self.galat()?;
                self.baris
                    .lock()
                    .unwrap()
                    .get(&request)
                    .cloned()
                    .ok_or(GalatDownstream::RequestTidakDikenal)
            })
        }

        fn batalkan(
            &self,
            _klien: SesiKlien,
            request: Uuid,
        ) -> Fut<'_, Hasil<(BarisRequest, bool)>> {
            Box::pin(async move {
                self.galat()?;
                let mut map = self.baris.lock().unwrap();
                let b = map
                    .get_mut(&request)
                    .ok_or(GalatDownstream::RequestTidakDikenal)?;
                if b.status != Some(ApiStatus::Queued) {
                    return Ok((b.clone(), false));
                }
                b.status = Some(ApiStatus::Cancelled);
                b.cancel_reason = Some("dibatalkan_client".into());
                Ok((b.clone(), true))
            })
        }
    }
}
