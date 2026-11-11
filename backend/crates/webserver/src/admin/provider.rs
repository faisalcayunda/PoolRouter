//! Provider CRUD with versioning: `listProviders`, `getProvider`,
//! `createProvider`, `updateProvider` (US-01/US-04; AC-12..AC-15).
//!
//! The shape of this module is set by V-03: `provider_config_versi` is
//! immutable, so an edit never touches a stored document. It inserts version
//! `n+1`, then moves `provider.config_versi_aktif` to it, in one transaction.
//! A reader can therefore never see a half-switched provider, and a request that
//! was already dispatched keeps the version it started with.
//!
//! Validation runs before the first write and inside the same transaction: the
//! template is checked against the parameter table in the body and against the
//! names in the global secret store (AD1). Any failure rolls the whole thing
//! back, so a 422 never leaves a provider row behind.
//!
//! The audit row shares the transaction through `core::audit::Penulis`, which
//! makes "config and trail commit together" a property of the type, not a rule
//! the handler is trusted to follow.

use std::collections::BTreeSet;
use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use poolrouter_core::audit::{
    AksiKonfigurasi, Alasan, EntitasKonfigurasi, KonfigurasiBaru, Penulis, Peran,
};
use poolrouter_core::data_balikan::GalatDataBalikan;
use poolrouter_core::template::{
    CONFIG_SCHEMA_VERSION_F1, ParameterTemplate, TemplateError, validasi_config_schema_version,
    validasi_template,
};
use serde_json::json;
use uuid::Uuid;

use super::gerbang::{Peminta, masuk};
use super::sesi::{AdminState, Galat};
use crate::api::admin::{Provider, ProviderConfig};
use crate::api::problem::{Posisi, ProblemKodeAdmin, ValidasiErrorAdmin};

/// Every provider operation is platform-admin only (DESIGN.md §3).
const PERAN_PA: [Peran; 1] = [Peran::PlatformAdmin];

/// Body member the template pointer is reported under (AC-13).
const FIELD_TEMPLATE: &str = "/body_template";
const FIELD_VERSI: &str = "/config_schema_version";

// ---------------------------------------------------------------------------
// Reads
// ---------------------------------------------------------------------------

/// `listProviders` (PA).
pub(super) async fn list_providers(
    State(s): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Response, Galat> {
    masuk(&s, &Method::GET, &headers, peer, &PERAN_PA, "listProviders").await?;

    let baris: Vec<BarisProvider> = sqlx::query_as(
        "SELECT p.provider_id, p.nama, p.aktif, \
         v.versi, v.config_schema_version, v.dokumen \
         FROM provider p \
         LEFT JOIN provider_config_versi v \
           ON v.provider_id = p.provider_id AND v.versi = p.config_versi_aktif \
         ORDER BY p.nama",
    )
    .fetch_all(&s.pool)
    .await
    .map_err(|e| galat_db(&e))?;

    let mut daftar: Vec<Provider> = Vec::with_capacity(baris.len());
    for b in &baris {
        // A provider with no active version is not a state F0 produces; skip it
        // rather than fail the whole list.
        if let Some(p) = b.ke_provider()? {
            daftar.push(p);
        }
    }
    Ok((StatusCode::OK, Json(daftar)).into_response())
}

/// `getProvider` (PA).
pub(super) async fn get_provider(
    State(s): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, Galat> {
    masuk(&s, &Method::GET, &headers, peer, &PERAN_PA, "getProvider").await?;

    let baris: Option<BarisProvider> = sqlx::query_as(
        "SELECT p.provider_id, p.nama, p.aktif, \
         v.versi, v.config_schema_version, v.dokumen \
         FROM provider p \
         LEFT JOIN provider_config_versi v \
           ON v.provider_id = p.provider_id AND v.versi = p.config_versi_aktif \
         WHERE p.provider_id = $1",
    )
    .bind(id)
    .fetch_optional(&s.pool)
    .await
    .map_err(|e| galat_db(&e))?;

    let Some(baris) = baris else {
        return Err(Galat::baru(
            StatusCode::NOT_FOUND,
            ProblemKodeAdmin::TidakDitemukan,
        ));
    };
    let provider = baris.ke_provider()?.ok_or_else(|| {
        tracing::error!(provider_id = %id, "admin: provider tanpa dokumen konfigurasi");
        Galat::layanan()
    })?;
    Ok((StatusCode::OK, Json(provider)).into_response())
}

// ---------------------------------------------------------------------------
// Writes
// ---------------------------------------------------------------------------

/// `createProvider` (PA). One transaction: `provider` row, version 1, the
/// pointer to it, and the audit row (AC-12).
pub(super) async fn create_provider(
    State(s): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<ProviderConfig>,
) -> Result<Response, Galat> {
    let peminta = masuk(
        &s,
        &Method::POST,
        &headers,
        peer,
        &PERAN_PA,
        "createProvider",
    )
    .await?;
    super::gerbang::periksa_guard_mfa(&peminta)?;

    let mut tx = s.pool.begin().await.map_err(|e| galat_db(&e))?;

    let parameter = parameter_template(&body);
    let rahasia = nama_secret(&mut tx).await?;
    periksa_body(&body, &parameter, &rahasia)?;

    let provider_id = Uuid::now_v7();
    let dokumen = dokumen_provider(&body);

    // `nama` is UNIQUE; a duplicate is the operator's mistake, not a 500.
    sqlx::query(
        "INSERT INTO provider (provider_id, nama, aktif, config_versi_aktif) \
         VALUES ($1, $2, true, NULL)",
    )
    .bind(provider_id)
    .bind(&body.nama)
    .execute(&mut *tx)
    .await
    .map_err(|e| galat_unik_atau_db(&e, &body.nama))?;

    let versi = simpan_versi(
        &mut tx,
        provider_id,
        1,
        &body,
        &dokumen,
        peminta.pengguna_id,
    )
    .await?;

    sqlx::query(
        "UPDATE provider SET config_versi_aktif = $2, diubah_at = now() WHERE provider_id = $1",
    )
    .bind(provider_id)
    .bind(versi)
    .execute(&mut *tx)
    .await
    .map_err(|e| galat_db(&e))?;

    catat(
        &mut tx,
        &peminta,
        AksiKonfigurasi::Buat,
        provider_id,
        None,
        Some(json!({ "nama": body.nama, "versi": versi })),
        alasan_create(&body),
    )
    .await?;

    tx.commit().await.map_err(|e| galat_db(&e))?;

    let provider = baca_provider(&s, provider_id).await?;
    Ok((StatusCode::CREATED, Json(provider)).into_response())
}

/// `updateProvider` (PA). Inserts version `n+1` and moves the pointer; version
/// `n` keeps its bytes (V-03). `alasan` is required and goes to the audit row.
pub(super) async fn update_provider(
    State(s): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    Json(body): Json<ProviderConfig>,
) -> Result<Response, Galat> {
    let peminta = masuk(
        &s,
        &Method::PUT,
        &headers,
        peer,
        &PERAN_PA,
        "updateProvider",
    )
    .await?;
    super::gerbang::periksa_guard_mfa(&peminta)?;

    let mut tx = s.pool.begin().await.map_err(|e| galat_db(&e))?;

    // Lock the provider row so two concurrent updates cannot pick the same
    // next version.
    let lama: Option<(i32, String)> = sqlx::query_as(
        "SELECT config_versi_aktif, nama FROM provider WHERE provider_id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| galat_db(&e))?;
    let Some((versi_lama, nama_lama)) = lama else {
        return Err(Galat::baru(
            StatusCode::NOT_FOUND,
            ProblemKodeAdmin::TidakDitemukan,
        ));
    };

    let parameter = parameter_template(&body);
    let rahasia = nama_secret(&mut tx).await?;
    periksa_body(&body, &parameter, &rahasia)?;

    let versi_baru = versi_lama.max(0) + 1;
    let dokumen = dokumen_provider(&body);

    if body.nama != nama_lama {
        sqlx::query("UPDATE provider SET nama = $2, diubah_at = now() WHERE provider_id = $1")
            .bind(id)
            .bind(&body.nama)
            .execute(&mut *tx)
            .await
            .map_err(|e| galat_unik_atau_db(&e, &body.nama))?;
    }

    simpan_versi(
        &mut tx,
        id,
        versi_baru,
        &body,
        &dokumen,
        peminta.pengguna_id,
    )
    .await?;

    sqlx::query(
        "UPDATE provider SET config_versi_aktif = $2, diubah_at = now() WHERE provider_id = $1",
    )
    .bind(id)
    .bind(versi_baru)
    .execute(&mut *tx)
    .await
    .map_err(|e| galat_db(&e))?;

    catat(
        &mut tx,
        &peminta,
        AksiKonfigurasi::Ubah,
        id,
        Some(json!({ "nama": nama_lama, "versi": versi_lama })),
        Some(json!({ "nama": body.nama, "versi": versi_baru })),
        alasan_wajib(&body)?,
    )
    .await?;

    tx.commit().await.map_err(|e| galat_db(&e))?;

    let provider = baca_provider(&s, id).await?;
    Ok((StatusCode::OK, Json(provider)).into_response())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// One joined row of `provider` and its active `provider_config_versi`.
#[derive(sqlx::FromRow)]
struct BarisProvider {
    provider_id: Uuid,
    nama: String,
    aktif: bool,
    versi: Option<i32>,
    config_schema_version: Option<i32>,
    dokumen: Option<serde_json::Value>,
}

impl BarisProvider {
    /// Rebuild the read entity. A provider with no active version is not a state
    /// F0 can produce (create writes version 1 in the same transaction), so it
    /// answers `None` and the caller decides.
    fn ke_provider(&self) -> Result<Option<Provider>, Galat> {
        let (Some(versi), Some(dokumen)) = (self.versi, self.dokumen.as_ref()) else {
            return Ok(None);
        };
        let doc: DokumenProvider = serde_json::from_value(dokumen.clone()).map_err(|e| {
            tracing::error!(provider_id = %self.provider_id, error = %e,
                "admin: dokumen provider tidak bisa dibaca");
            Galat::layanan()
        })?;
        Ok(Some(Provider {
            provider_id: self.provider_id,
            nama: self.nama.clone(),
            metode: doc.metode,
            base_url: doc.base_url,
            body_template: doc.body_template,
            parameters: doc.parameters,
            respons_skema: doc.respons_skema,
            tls_trust: doc.tls_trust,
            tls_ca_pem: doc.tls_ca_pem,
            tls_pin_spki: doc.tls_pin_spki,
            log_mode: doc.log_mode,
            data_balikan: doc.data_balikan,
            config_schema_version: i64::from(self.config_schema_version.unwrap_or(1)),
            provider_config_version: i64::from(versi),
            aktif: self.aktif,
        }))
    }
}

/// The stored document: everything in `ProviderConfig` except the fields that
/// live on columns of their own (`nama` on `provider`, `log_mode` and
/// `config_schema_version` on the version row) and except `alasan`, which is an
/// audit field and never part of the configuration. For v2, also includes `data_balikan`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct DokumenProvider {
    metode: crate::api::enums::MetodeProvider,
    base_url: String,
    body_template: String,
    parameters: Vec<crate::api::admin::ParameterRule>,
    respons_skema: serde_json::Value,
    tls_trust: crate::api::enums::TlsTrust,
    /// TLS material (ARC-239). Absent in documents written before it existed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tls_ca_pem: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tls_pin_spki: Option<Vec<String>>,
    log_mode: crate::api::enums::LogMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    data_balikan: Option<serde_json::Value>,
}

fn dokumen_provider(body: &ProviderConfig) -> DokumenProvider {
    DokumenProvider {
        metode: body.metode,
        base_url: body.base_url.clone(),
        body_template: body.body_template.clone(),
        parameters: body.parameters.clone(),
        respons_skema: body.respons_skema.clone(),
        tls_trust: body.tls_trust,
        tls_ca_pem: body.tls_ca_pem.clone(),
        tls_pin_spki: body.tls_pin_spki.clone(),
        log_mode: body.log_mode,
        data_balikan: body.data_balikan.clone(),
    }
}

fn parameter_template(body: &ProviderConfig) -> Vec<ParameterTemplate> {
    body.parameters
        .iter()
        .map(|p| ParameterTemplate {
            nama: p.nama.clone(),
        })
        .collect()
}

/// Names of the secrets that exist right now (AD1: the store is global per name,
/// so this is answerable before the provider has an id).
async fn nama_secret(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
) -> Result<BTreeSet<String>, Galat> {
    let baris: Vec<(String,)> = sqlx::query_as("SELECT nama_secret FROM provider_secret")
        .fetch_all(&mut **tx)
        .await
        .map_err(|e| galat_db(&e))?;
    Ok(baris.into_iter().map(|(n,)| n).collect())
}

/// Every check that can answer 422, in the order AC-13 lists them, all collected
/// so the UI can show them at once (AC-20: data_balikan validation for v2).
fn periksa_body(
    body: &ProviderConfig,
    parameter: &[ParameterTemplate],
    rahasia: &BTreeSet<String>,
) -> Result<(), Galat> {
    let mut galat: Vec<TemplateError> = Vec::new();

    if let Err(e) = validasi_config_schema_version(FIELD_VERSI, body.config_schema_version) {
        galat.push(e);
    }
    if let Err(mut e) = validasi_template(FIELD_TEMPLATE, &body.body_template, parameter, rahasia) {
        galat.append(&mut e);
    }
    galat.extend(validasi_rule(&body.parameters));
    galat.extend(validasi_base_url(&body.base_url));
    galat.extend(validasi_tls(
        body.tls_trust,
        body.tls_ca_pem.as_deref(),
        body.tls_pin_spki.as_deref(),
    ));

    // v2: data_balikan required and validated; v1: ignored
    if body.config_schema_version == CONFIG_SCHEMA_VERSION_F1 {
        if let Some(blok) = &body.data_balikan {
            if let Err(e) = validasi_data_balikan(blok) {
                galat.push(e);
            }
        } else {
            galat.push(TemplateError {
                field: "/data_balikan".to_owned(),
                kode: "required_untuk_versi_2",
                posisi: None,
                nama: None,
            });
        }
    }

    if galat.is_empty() {
        Ok(())
    } else {
        Err(Galat::validasi(
            galat.into_iter().map(ke_validasi_admin).collect(),
        ))
    }
}

/// Parse and validate data_balikan block (AC-20, AC-15). Convert GalatDataBalikan to TemplateError.
fn validasi_data_balikan(blok: &serde_json::Value) -> Result<(), TemplateError> {
    use poolrouter_core::data_balikan::KonfigurasiDataBalikan;
    match KonfigurasiDataBalikan::parse(2, blok) {
        Ok(_) => Ok(()),
        Err(GalatDataBalikan::TidakSah(field, pesan)) => Err(TemplateError {
            field: format!("/data_balikan/{}", field),
            kode: "data_balikan_tidak_sah",
            posisi: None,
            nama: Some(pesan.to_owned()),
        }),
        Err(GalatDataBalikan::SkemaTerlaluTua(_)) => Err(TemplateError {
            field: "/data_balikan".to_owned(),
            kode: "data_balikan_skema_terlalu_tua",
            posisi: None,
            nama: None,
        }),
        Err(GalatDataBalikan::SkemaTerlaluBaru(_)) => Err(TemplateError {
            field: "/data_balikan".to_owned(),
            kode: "data_balikan_skema_terlalu_baru",
            posisi: None,
            nama: None,
        }),
    }
}

/// `base_url` must be https: Dukcapil carries NIK and credentials, so a plain
/// http base URL is refused the same way a non-https webhook is.
fn validasi_base_url(base_url: &str) -> Vec<TemplateError> {
    if base_url.starts_with("https://") && base_url.len() > "https://".len() {
        Vec::new()
    } else {
        vec![TemplateError {
            field: "/base_url".to_owned(),
            kode: "format",
            posisi: None,
            nama: None,
        }]
    }
}

/// TLS material must match `tls_trust` (ARC-239): `ca_kustom` needs a PEM and
/// no pins, `pin_spki` needs pins and no PEM, `system` takes neither. Material
/// for the wrong mode is a 422 rather than ignored, so a PUT (a full
/// replacement) that forgets or misplaces it never creates a version the sync
/// path and the worker would reject. Once the fields are sound, the client is
/// assembled as a dry run, so what passes here can be built at runtime.
fn validasi_tls(
    mode: crate::api::enums::TlsTrust,
    pem: Option<&str>,
    pins: Option<&[String]>,
) -> Vec<TemplateError> {
    use crate::api::enums::TlsTrust as Mode;
    use poolrouter_adapter_provider::client::{TlsTrust, build_provider_client};
    use poolrouter_adapter_provider::dokumen::{PIN_MAKS, pin_dari_base64, validasi_ca_pem};

    let mut galat: Vec<TemplateError> = Vec::new();
    let te = |field: String, kode: &'static str| TemplateError {
        field,
        kode,
        posisi: None,
        nama: None,
    };
    let pem_field = || "/tls_ca_pem".to_owned();
    let pin_field = || "/tls_pin_spki".to_owned();

    if mode != Mode::CaKustom && pem.is_some() {
        galat.push(te(pem_field(), "tidak_untuk_mode"));
    }
    if mode != Mode::PinSpki && pins.is_some() {
        galat.push(te(pin_field(), "tidak_untuk_mode"));
    }
    let mut trust = None;
    match mode {
        Mode::System => {}
        Mode::CaKustom => match pem {
            None => galat.push(te(pem_field(), "wajib")),
            Some(p) => match validasi_ca_pem(p) {
                Ok(()) => {
                    trust = Some(TlsTrust::CaKustom {
                        ca_pem: p.as_bytes().to_vec(),
                    });
                }
                Err(kode) => galat.push(te(pem_field(), kode)),
            },
        },
        Mode::PinSpki => match pins {
            None | Some([]) => galat.push(te(pin_field(), "wajib")),
            Some(daftar) => {
                if daftar.len() > PIN_MAKS {
                    galat.push(te(pin_field(), "jumlah"));
                }
                let mut sah: Vec<[u8; 32]> = Vec::new();
                for (i, p) in daftar.iter().enumerate() {
                    match pin_dari_base64(p) {
                        Err(_) => galat.push(te(format!("/tls_pin_spki/{i}"), "pin_tidak_sah")),
                        Ok(h) if sah.contains(&h) => {
                            galat.push(te(format!("/tls_pin_spki/{i}"), "ganda"))
                        }
                        Ok(h) => sah.push(h),
                    }
                }
                if sah.len() == daftar.len() {
                    trust = Some(TlsTrust::PinSpki { pins: sah });
                }
            }
        },
    }
    if galat.is_empty()
        && let Some(trust) = trust
        && let Err(e) = build_provider_client(trust)
    {
        tracing::debug!(error = %e, "admin: bahan TLS provider tidak bisa dirakit");
        let field = if mode == Mode::CaKustom {
            pem_field()
        } else {
            pin_field()
        };
        galat.push(te(field, "tidak_sah"));
    }
    galat
}

/// Parameter rules (AC-15): a `regex` that does not compile, an `enum` with no
/// values, a `panjang` with no bounds, or a rule whose parameters do not match
/// its `jenis` is a 422 now rather than a surprise at request time.
fn validasi_rule(parameters: &[crate::api::admin::ParameterRule]) -> Vec<TemplateError> {
    use crate::api::enums::RuleJenis;

    let mut galat = Vec::new();
    let mut push = |field: String, kode: &'static str| {
        galat.push(TemplateError {
            field,
            kode,
            posisi: None,
            nama: None,
        });
    };

    for (i, p) in parameters.iter().enumerate() {
        if p.nama.trim().is_empty() {
            push(format!("/parameters/{i}/nama"), "wajib");
        }
        let Some(rule) = &p.rule else { continue };
        let dasar = format!("/parameters/{i}/rule");
        match rule.jenis {
            RuleJenis::Regex => match rule.pola.as_deref() {
                Some(pola) => {
                    if let Err(e) = regex::Regex::new(pola) {
                        tracing::debug!(error = %e, "admin: pola regex tidak bisa dikompilasi");
                        push(format!("{dasar}/pola"), "format");
                    }
                }
                None => push(format!("{dasar}/pola"), "wajib"),
            },
            RuleJenis::Enum => {
                if rule.nilai_diizinkan.as_ref().is_none_or(Vec::is_empty) {
                    push(format!("{dasar}/nilai_diizinkan"), "wajib");
                }
            }
            RuleJenis::Panjang => {
                if rule.panjang_min.is_none() && rule.panjang_maks.is_none() {
                    push(format!("{dasar}/panjang_min"), "wajib");
                }
                if let (Some(min), Some(maks)) = (rule.panjang_min, rule.panjang_maks)
                    && maks < min
                {
                    push(format!("{dasar}/panjang_maks"), "rentang");
                }
            }
            RuleJenis::FormatTanggal => {}
        }
    }
    galat
}

fn ke_validasi_admin(e: TemplateError) -> ValidasiErrorAdmin {
    ValidasiErrorAdmin {
        field: e.field,
        kode: e.kode.to_owned(),
        batas: None,
        posisi: e.posisi.map(|p| Posisi {
            baris: p.baris,
            kolom: p.kolom,
        }),
        nama: e.nama,
    }
}

/// Insert one version row and return its number.
async fn simpan_versi(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    provider_id: Uuid,
    versi: i32,
    body: &ProviderConfig,
    dokumen: &DokumenProvider,
    pengguna: Uuid,
) -> Result<i32, Galat> {
    let dokumen = serde_json::to_value(dokumen).map_err(|e| {
        tracing::error!(error = %e, "admin: dokumen provider tidak bisa diserialkan");
        Galat::layanan()
    })?;
    sqlx::query(
        "INSERT INTO provider_config_versi \
         (provider_id, versi, config_schema_version, log_mode, dokumen, dibuat_oleh) \
         VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(provider_id)
    .bind(versi)
    .bind(i32::try_from(body.config_schema_version).unwrap_or(1))
    .bind(body.log_mode.as_str())
    .bind(&dokumen)
    .bind(pengguna)
    .execute(&mut **tx)
    .await
    .map_err(|e| galat_db(&e))?;
    Ok(versi)
}

pub(super) async fn baca_provider(s: &AdminState, id: Uuid) -> Result<Provider, Galat> {
    let baris: BarisProvider = sqlx::query_as(
        "SELECT p.provider_id, p.nama, p.aktif, \
         v.versi, v.config_schema_version, v.dokumen \
         FROM provider p \
         JOIN provider_config_versi v \
           ON v.provider_id = p.provider_id AND v.versi = p.config_versi_aktif \
         WHERE p.provider_id = $1",
    )
    .bind(id)
    .fetch_one(&s.pool)
    .await
    .map_err(|e| galat_db(&e))?;
    baris.ke_provider()?.ok_or_else(|| {
        tracing::error!(provider_id = %id, "admin: provider tanpa dokumen setelah tulis");
        Galat::layanan()
    })
}

/// `createProvider` carries no `alasan` in the contract; the audit row still
/// needs a reason, so it states the operation.
fn alasan_create(_body: &ProviderConfig) -> String {
    "createProvider".to_owned()
}

/// `updateProvider` requires `alasan` (DESIGN.md §3). The contract marks it
/// optional because create does not use it, so a missing one on update is a 422.
const ALASAN_MIN: usize = 10;
const ALASAN_MAX: usize = 500;

fn alasan_wajib(body: &ProviderConfig) -> Result<String, Galat> {
    let Some(a) = body.alasan.as_deref() else {
        return Err(Galat::validasi(vec![ValidasiErrorAdmin {
            field: "/alasan".to_owned(),
            kode: "wajib".to_owned(),
            batas: None,
            posisi: None,
            nama: None,
        }]));
    };
    let panjang = a.trim().len();
    if !(ALASAN_MIN..=ALASAN_MAX).contains(&panjang) {
        return Err(Galat::validasi(vec![ValidasiErrorAdmin {
            field: "/alasan".to_owned(),
            kode: "rentang".to_owned(),
            batas: Some(ALASAN_MAX as f64),
            posisi: None,
            nama: None,
        }]));
    }
    Ok(a.to_owned())
}

/// One `audit_konfigurasi` row in the caller's transaction.
async fn catat(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    peminta: &Peminta,
    aksi: AksiKonfigurasi,
    entitas_id: Uuid,
    nilai_lama: Option<serde_json::Value>,
    nilai_baru: Option<serde_json::Value>,
    alasan: String,
) -> Result<(), Galat> {
    let baris = KonfigurasiBaru {
        pelaku: peminta.pengguna_id,
        entitas: EntitasKonfigurasi::Provider,
        entitas_id: Some(entitas_id),
        aksi,
        nilai_lama,
        nilai_baru,
        alasan: Alasan::baru(alasan).map_err(|e| {
            tracing::error!(error = %e, "admin: alasan audit tidak valid");
            Galat::layanan()
        })?,
    };
    Penulis::baru(tx)
        .catat_konfigurasi(&baris)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "admin: gagal menulis audit konfigurasi");
            Galat::layanan()
        })?;
    Ok(())
}

/// A duplicate `nama` is the operator's mistake, not a 500. The contract has no
/// 409 for providers, so it is reported as 422 on the `nama` field, where the UI
/// can show it next to the input.
fn galat_unik_atau_db(e: &sqlx::Error, nama: &str) -> Galat {
    if let sqlx::Error::Database(db) = e
        && db.is_unique_violation()
    {
        tracing::debug!(nama, "admin: nama provider sudah dipakai");
        return Galat::validasi(vec![ValidasiErrorAdmin {
            field: "/nama".to_owned(),
            kode: "sudah_dipakai".to_owned(),
            batas: None,
            posisi: None,
            nama: None,
        }]);
    }
    galat_db(e)
}

fn galat_db(e: &sqlx::Error) -> Galat {
    tracing::error!(error = %e, "admin: kueri provider gagal");
    Galat::layanan()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::enums::TlsTrust as Mode;
    use base64::Engine as _;

    fn pem_ca() -> String {
        let key = rcgen::KeyPair::generate().unwrap();
        let mut p = rcgen::CertificateParams::new(Vec::new()).unwrap();
        p.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        p.self_signed(&key).unwrap().pem()
    }

    fn pin(n: u8) -> String {
        base64::engine::general_purpose::STANDARD.encode([n; 32])
    }

    /// `(field, kode)` dari semua galat.
    fn galat(
        mode: Mode,
        pem: Option<&str>,
        pins: Option<&[String]>,
    ) -> Vec<(String, &'static str)> {
        validasi_tls(mode, pem, pins)
            .into_iter()
            .map(|e| (e.field, e.kode))
            .collect()
    }

    #[test]
    fn bahan_yang_cocok_dengan_mode_lolos() {
        assert!(galat(Mode::System, None, None).is_empty());
        assert!(galat(Mode::CaKustom, Some(&pem_ca()), None).is_empty());
        // Satu pin cukup: peringatan dua pin adalah urusan UI.
        assert!(galat(Mode::PinSpki, None, Some(&[pin(1)])).is_empty());
        let sepuluh: Vec<String> = (1..=10).map(pin).collect();
        assert!(galat(Mode::PinSpki, None, Some(&sepuluh)).is_empty());
    }

    #[test]
    fn bahan_tidak_cocok_dengan_mode_adalah_422() {
        let pem = pem_ca();
        let pins = [pin(1)];
        assert_eq!(
            galat(Mode::System, Some(&pem), Some(&pins)),
            vec![
                ("/tls_ca_pem".to_owned(), "tidak_untuk_mode"),
                ("/tls_pin_spki".to_owned(), "tidak_untuk_mode")
            ]
        );
        // String kosong tetap "diisi": tidak diabaikan diam-diam.
        assert_eq!(
            galat(Mode::System, Some(""), None),
            vec![("/tls_ca_pem".to_owned(), "tidak_untuk_mode")]
        );
        assert_eq!(
            galat(Mode::CaKustom, None, None),
            vec![("/tls_ca_pem".to_owned(), "wajib")]
        );
        assert_eq!(
            galat(Mode::CaKustom, Some(&pem), Some(&pins)),
            vec![("/tls_pin_spki".to_owned(), "tidak_untuk_mode")]
        );
        assert_eq!(
            galat(Mode::PinSpki, None, None),
            vec![("/tls_pin_spki".to_owned(), "wajib")]
        );
        assert_eq!(
            galat(Mode::PinSpki, None, Some(&[])),
            vec![("/tls_pin_spki".to_owned(), "wajib")]
        );
        assert_eq!(
            galat(Mode::PinSpki, Some(&pem), Some(&pins)),
            vec![("/tls_ca_pem".to_owned(), "tidak_untuk_mode")]
        );
    }

    #[test]
    fn pem_yang_salah_ditolak_dengan_kode_yang_membedakan() {
        let pem = pem_ca();
        let kunci = "-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----\n";
        assert_eq!(
            galat(Mode::CaKustom, Some(&format!("{pem}{kunci}")), None),
            vec![("/tls_ca_pem".to_owned(), "pem_tidak_sah")]
        );
        assert_eq!(
            galat(Mode::CaKustom, Some("bukan pem"), None),
            vec![("/tls_ca_pem".to_owned(), "pem_tidak_sah")]
        );
        let besar = format!("{pem}{}", " ".repeat(64 * 1024));
        assert_eq!(
            galat(Mode::CaKustom, Some(&besar), None),
            vec![("/tls_ca_pem".to_owned(), "terlalu_besar")]
        );
    }

    #[test]
    fn pin_salah_ditunjuk_per_indeks() {
        let tiga_puluh_satu = base64::engine::general_purpose::STANDARD.encode([1u8; 31]);
        let pins = [
            pin(1),
            "bukan base64!".to_owned(),
            tiga_puluh_satu,
            pin(1),
            pin(2),
        ];
        assert_eq!(
            galat(Mode::PinSpki, None, Some(&pins)),
            vec![
                ("/tls_pin_spki/1".to_owned(), "pin_tidak_sah"),
                ("/tls_pin_spki/2".to_owned(), "pin_tidak_sah"),
                ("/tls_pin_spki/3".to_owned(), "ganda"),
            ]
        );
    }

    #[test]
    fn lebih_dari_sepuluh_pin_ditolak() {
        let banyak: Vec<String> = (1..=11).map(pin).collect();
        assert_eq!(
            galat(Mode::PinSpki, None, Some(&banyak)),
            vec![("/tls_pin_spki".to_owned(), "jumlah")]
        );
    }
}
