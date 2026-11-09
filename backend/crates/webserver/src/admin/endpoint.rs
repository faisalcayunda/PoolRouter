//! CRUD endpoint berversi dengan kontrak metadata, plus `setClientEndpoints`
//! (ARC-54; AC-25..AC-27).
//!
//! Bentuknya sama dengan provider (V-03): `endpoint_config_versi` tidak pernah
//! diubah. Edit menyisipkan versi `n+1` lalu memindahkan
//! `downstream_endpoint.config_versi_aktif`, dalam satu transaksi dengan baris
//! audit. Definisi field metadata (`endpoint_metadata_field`) tidak berversi per
//! baris tabel, jadi edit menggantinya utuh; field yang definisinya tidak
//! berubah mempertahankan `versi`-nya, field yang berubah atau baru mendapat
//! versi endpoint yang baru (nilai `versi` itu dipakai kontrak downstream).
//!
//! Aturan 422 yang diuji lewat router (AC-25):
//! - rujukan `{{endpoint.X}}` di template provider tanpa `parameter_literal.X`;
//! - provider berdata balikan (`data_balikan_aktif`) tanpa field
//!   `status_penerima_manfaat` → kode problem `metadata_wajib_hilang` (M-R1);
//! - `log_mode` endpoint lebih longgar dari provider.
//!
//! Kolom `aktif` tidak disentuh; penonaktifan endpoint lewat jalur K-20.

use std::collections::{BTreeMap, BTreeSet};
use std::net::SocketAddr;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use poolrouter_core::audit::{AksiKonfigurasi, EntitasKonfigurasi, Peran};
use poolrouter_core::template::rujukan_namespace;
use serde_json::{Value, json};
use uuid::Uuid;

use super::gerbang::{masuk, periksa_guard_mfa};
use super::pool::{catat, galat_db, galat_field, galat_tulis, periksa_alasan, snapshot, urai_body};
use super::sesi::{AdminState, Galat};
use crate::api::admin::{Client, ClientEndpoints, Endpoint, EndpointConfig, MetadataFieldDef};
use crate::api::enums::{KlasifikasiMetadata, LogMode, TipeMetadata};
use crate::api::problem::{ProblemKodeAdmin, ValidasiErrorAdmin};

const PERAN_PA: [Peran; 1] = [Peran::PlatformAdmin];

const ENDPOINT_WAJIB: [&str; 6] = [
    "provider_id",
    "downstream_pool_id",
    "parameter_literal",
    "log_mode",
    "metadata_fields",
    "config_schema_version",
];
const ENDPOINT_OPSIONAL: [&str; 1] = ["alasan"];

/// Field metadata wajib endpoint provider berdata balikan (M-R1).
const FIELD_STATUS_PENERIMA: &str = "status_penerima_manfaat";
const NAMA_FIELD_MAX: usize = 64;

// ---------------------------------------------------------------------------
// Baca
// ---------------------------------------------------------------------------

/// Endpoint beserta versi aktifnya. `log_mode` versi yang NULL (warisan
/// provider, mungkin ada dari fixture/seed) dilaporkan sebagai `log_mode`
/// provider, nilai yang memang berlaku.
macro_rules! select_endpoint {
    ($ekor:literal) => {
        concat!(
            "SELECT e.downstream_endpoint_id, e.provider_id, e.downstream_pool_id, e.aktif, \
             v.versi, coalesce(v.log_mode, pv.log_mode) AS log_mode, v.dokumen \
             FROM downstream_endpoint e \
             LEFT JOIN endpoint_config_versi v \
               ON v.downstream_endpoint_id = e.downstream_endpoint_id AND v.versi = e.config_versi_aktif \
             LEFT JOIN provider p ON p.provider_id = e.provider_id \
             LEFT JOIN provider_config_versi pv \
               ON pv.provider_id = p.provider_id AND pv.versi = p.config_versi_aktif",
            $ekor
        )
    };
}

#[derive(sqlx::FromRow)]
struct BarisEndpoint {
    downstream_endpoint_id: Uuid,
    provider_id: Uuid,
    downstream_pool_id: Uuid,
    aktif: bool,
    versi: Option<i32>,
    log_mode: Option<String>,
    dokumen: Option<sqlx::types::Json<Value>>,
}

#[derive(sqlx::FromRow)]
struct BarisMetadata {
    downstream_endpoint_id: Uuid,
    nama_field: String,
    tipe: String,
    wajib: bool,
    nilai_diizinkan: Option<sqlx::types::Json<Value>>,
    aturan_validasi: Option<sqlx::types::Json<Value>>,
    klasifikasi: String,
    kirim_ke_data_balikan: bool,
    deskripsi: String,
    versi: i32,
}

impl BarisMetadata {
    fn ke_def(&self) -> Option<MetadataFieldDef> {
        let tipe: TipeMetadata = serde_json::from_value(Value::String(self.tipe.clone())).ok()?;
        let klasifikasi: KlasifikasiMetadata =
            serde_json::from_value(Value::String(self.klasifikasi.clone())).ok()?;
        let nilai_diizinkan = match self.nilai_diizinkan.as_ref().map(|j| &j.0) {
            None | Some(Value::Null) => None,
            Some(v) => serde_json::from_value::<Vec<String>>(v.clone()).ok(),
        };
        let aturan_validasi = match self.aturan_validasi.as_ref().map(|j| &j.0) {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.clone()),
            Some(v) => Some(v.to_string()),
        };
        Some(MetadataFieldDef {
            nama_field: self.nama_field.clone(),
            tipe,
            wajib: self.wajib,
            nilai_diizinkan,
            aturan_validasi,
            klasifikasi,
            kirim_ke_data_balikan: self.kirim_ke_data_balikan,
            deskripsi: self.deskripsi.clone(),
        })
    }
}

fn log_mode_dari_kolom(k: &str) -> LogMode {
    if k == "full_payload" {
        LogMode::FullPayload
    } else {
        LogMode::MetadataOnly
    }
}

/// Susun entitas dari baris endpoint dan baris metadatanya. Endpoint tanpa
/// versi aktif bukan keadaan yang dibuat admin API; dilewati.
fn susun(baris: Vec<BarisEndpoint>, meta: &[BarisMetadata]) -> Vec<Endpoint> {
    let mut per_endpoint: BTreeMap<Uuid, Vec<MetadataFieldDef>> = BTreeMap::new();
    for m in meta {
        if let Some(d) = m.ke_def() {
            per_endpoint
                .entry(m.downstream_endpoint_id)
                .or_default()
                .push(d);
        }
    }
    baris
        .into_iter()
        .filter_map(|b| {
            let versi = b.versi?;
            let literal: BTreeMap<String, String> = b
                .dokumen
                .as_ref()
                .and_then(|d| d.0.get("parameter_literal"))
                .and_then(Value::as_object)
                .map(|o| {
                    o.iter()
                        .map(|(k, v)| {
                            let teks = v.as_str().map_or_else(|| v.to_string(), str::to_owned);
                            (k.clone(), teks)
                        })
                        .collect()
                })
                .unwrap_or_default();
            Some(Endpoint {
                endpoint_id: b.downstream_endpoint_id,
                provider_id: b.provider_id,
                downstream_pool_id: b.downstream_pool_id,
                parameter_literal: literal,
                log_mode: log_mode_dari_kolom(b.log_mode.as_deref().unwrap_or("metadata_only")),
                metadata_fields: per_endpoint
                    .remove(&b.downstream_endpoint_id)
                    .unwrap_or_default(),
                config_schema_version: 1,
                endpoint_config_version: i64::from(versi),
                aktif: b.aktif,
            })
        })
        .collect()
}

async fn baca_metadata(
    conn: &mut sqlx::PgConnection,
    id: &[Uuid],
) -> Result<Vec<BarisMetadata>, Galat> {
    sqlx::query_as(
        "SELECT downstream_endpoint_id, nama_field::text AS nama_field, tipe, wajib, \
         nilai_diizinkan, aturan_validasi, klasifikasi, kirim_ke_data_balikan, deskripsi, versi \
         FROM endpoint_metadata_field WHERE downstream_endpoint_id = ANY($1) \
         ORDER BY downstream_endpoint_id, nama_field",
    )
    .bind(id)
    .fetch_all(conn)
    .await
    .map_err(|e| galat_db(&e))
}

pub(super) async fn baca_endpoint(
    conn: &mut sqlx::PgConnection,
    id: Uuid,
) -> Result<Option<Endpoint>, Galat> {
    let baris: Vec<BarisEndpoint> =
        sqlx::query_as(select_endpoint!(" WHERE e.downstream_endpoint_id = $1"))
            .bind(id)
            .fetch_all(&mut *conn)
            .await
            .map_err(|e| galat_db(&e))?;
    let meta = baca_metadata(conn, &[id]).await?;
    Ok(susun(baris, &meta).into_iter().next())
}

/// `listEndpoints` (PA).
pub(super) async fn list_endpoints(
    State(s): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &Method::GET, &headers, peer, &PERAN_PA, "listEndpoints").await?;
    periksa_guard_mfa(&peminta)?;
    let mut conn = s.pool.acquire().await.map_err(|e| galat_db(&e))?;
    let baris: Vec<BarisEndpoint> = sqlx::query_as(select_endpoint!(
        " ORDER BY e.dibuat_at, e.downstream_endpoint_id"
    ))
    .fetch_all(&mut *conn)
    .await
    .map_err(|e| galat_db(&e))?;
    let id: Vec<Uuid> = baris.iter().map(|b| b.downstream_endpoint_id).collect();
    let meta = baca_metadata(&mut conn, &id).await?;
    Ok((StatusCode::OK, Json(susun(baris, &meta))).into_response())
}

/// `getEndpoint` (PA).
pub(super) async fn get_endpoint(
    State(s): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &Method::GET, &headers, peer, &PERAN_PA, "getEndpoint").await?;
    periksa_guard_mfa(&peminta)?;
    let mut conn = s.pool.acquire().await.map_err(|e| galat_db(&e))?;
    let entitas = baca_endpoint(&mut conn, id)
        .await?
        .ok_or_else(Galat::tidak_ditemukan)?;
    Ok((StatusCode::OK, Json(entitas)).into_response())
}

// ---------------------------------------------------------------------------
// Validasi
// ---------------------------------------------------------------------------

/// Bagian provider aktif yang dibutuhkan validasi endpoint.
#[derive(Debug, Clone)]
struct InfoProvider {
    data_balikan_aktif: bool,
    log_mode: LogMode,
    /// Nama `{{endpoint.X}}` yang dirujuk template provider.
    rujukan_endpoint: BTreeSet<String>,
}

async fn info_provider(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    id: Uuid,
) -> Result<Option<InfoProvider>, Galat> {
    let baris: Option<(bool, String, sqlx::types::Json<Value>)> = sqlx::query_as(
        "SELECT p.data_balikan_aktif, v.log_mode, v.dokumen FROM provider p \
         JOIN provider_config_versi v \
           ON v.provider_id = p.provider_id AND v.versi = p.config_versi_aktif \
         WHERE p.provider_id = $1",
    )
    .bind(id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| galat_db(&e))?;
    Ok(baris.map(|(data_balikan_aktif, log_mode, dokumen)| {
        let mut rujukan = BTreeSet::new();
        for kunci in ["body_template", "base_url"] {
            if let Some(t) = dokumen.0.get(kunci).and_then(Value::as_str) {
                rujukan.extend(rujukan_namespace(t, "endpoint"));
            }
        }
        InfoProvider {
            data_balikan_aktif,
            log_mode: log_mode_dari_kolom(&log_mode),
            rujukan_endpoint: rujukan,
        }
    }))
}

/// Semua pemeriksaan body endpoint, dikumpulkan sekaligus (AC-7, AC-25).
/// Mengembalikan galat dan apakah M-R1 dilanggar (kode problem
/// `metadata_wajib_hilang`).
fn periksa_endpoint(
    body: &EndpointConfig,
    provider: Option<&InfoProvider>,
    update: bool,
) -> (Vec<ValidasiErrorAdmin>, bool) {
    let mut galat = Vec::new();
    if body.config_schema_version != 1 {
        galat.push(galat_field("config_schema_version", "rentang", Some(1.0)));
    }
    if body.parameter_literal.keys().any(|k| k.trim().is_empty()) {
        galat.push(galat_field("parameter_literal", "format", None));
    }

    let mut dilihat = BTreeSet::new();
    for (i, m) in body.metadata_fields.iter().enumerate() {
        let dasar = format!("metadata_fields/{i}");
        if !nama_field_sah(&m.nama_field) {
            galat.push(galat_field(format!("{dasar}/nama_field"), "format", None));
        } else if !dilihat.insert(m.nama_field.as_str()) {
            galat.push(galat_field(
                format!("{dasar}/nama_field"),
                "sudah_dipakai",
                None,
            ));
        }
        if m.tipe == TipeMetadata::Enum && m.nilai_diizinkan.as_ref().is_none_or(Vec::is_empty) {
            galat.push(galat_field(
                format!("{dasar}/nilai_diizinkan"),
                "wajib",
                None,
            ));
        }
        if m.deskripsi.trim().is_empty() {
            galat.push(galat_field(format!("{dasar}/deskripsi"), "wajib", None));
        }
    }

    let mut melanggar_mr1 = false;
    match provider {
        None => galat.push(galat_field("provider_id", "tidak_ada", None)),
        Some(p) => {
            for nama in p
                .rujukan_endpoint
                .iter()
                .filter(|n| !body.parameter_literal.contains_key(*n))
            {
                galat.push(ValidasiErrorAdmin {
                    nama: Some(nama.clone()),
                    ..galat_field("parameter_literal", "literal_tidak_ada", None)
                });
            }
            if p.log_mode == LogMode::MetadataOnly && body.log_mode == LogMode::FullPayload {
                galat.push(galat_field("log_mode", "lebih_longgar_dari_provider", None));
            }
            if p.data_balikan_aktif
                && !body
                    .metadata_fields
                    .iter()
                    .any(|m| m.nama_field == FIELD_STATUS_PENERIMA)
            {
                melanggar_mr1 = true;
                galat.push(ValidasiErrorAdmin {
                    nama: Some(FIELD_STATUS_PENERIMA.to_owned()),
                    ..galat_field("metadata_fields", "metadata_wajib_hilang", None)
                });
            }
        }
    }
    if let Some(e) = periksa_alasan(body.alasan.as_deref(), update) {
        galat.push(e);
    }
    (galat, melanggar_mr1)
}

/// `^[a-z][a-z0-9_]*$`, paling panjang 64 (CHECK kolom `nama_field`).
fn nama_field_sah(nama: &str) -> bool {
    let mut c = nama.chars();
    nama.len() <= NAMA_FIELD_MAX
        && c.next().is_some_and(|a| a.is_ascii_lowercase())
        && c.all(|x| x.is_ascii_lowercase() || x.is_ascii_digit() || x == '_')
}

fn galat_endpoint(galat: Vec<ValidasiErrorAdmin>, mr1: bool) -> Galat {
    if mr1 {
        Galat::validasi_dengan_kode(ProblemKodeAdmin::MetadataWajibHilang, galat)
    } else {
        Galat::validasi(galat)
    }
}

async fn pool_ada(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    id: Uuid,
) -> Result<bool, Galat> {
    sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM downstream_pool WHERE downstream_pool_id = $1)",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| galat_db(&e))
}

// ---------------------------------------------------------------------------
// Tulis
// ---------------------------------------------------------------------------

/// Validasi bersama create dan update; mengembalikan body yang sudah diurai.
async fn siapkan(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    body: &EndpointConfig,
    update: bool,
) -> Result<(), Galat> {
    let provider = info_provider(tx, body.provider_id).await?;
    let (mut galat, mr1) = periksa_endpoint(body, provider.as_ref(), update);
    if !pool_ada(tx, body.downstream_pool_id).await? {
        galat.push(galat_field("downstream_pool_id", "tidak_ada", None));
    }
    if galat.is_empty() {
        Ok(())
    } else {
        Err(galat_endpoint(galat, mr1))
    }
}

async fn simpan_versi(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    id: Uuid,
    versi: i32,
    body: &EndpointConfig,
    pengguna: Uuid,
) -> Result<(), Galat> {
    sqlx::query(
        "INSERT INTO endpoint_config_versi \
         (downstream_endpoint_id, versi, config_schema_version, log_mode, dokumen, dibuat_oleh) \
         VALUES ($1, $2, 1, $3, $4, $5)",
    )
    .bind(id)
    .bind(versi)
    .bind(body.log_mode.as_str())
    .bind(sqlx::types::Json(
        json!({ "parameter_literal": body.parameter_literal }),
    ))
    .bind(pengguna)
    .execute(&mut **tx)
    .await
    .map_err(|e| galat_tulis(&e, "provider_id"))?;
    sqlx::query(
        "UPDATE downstream_endpoint SET config_versi_aktif = $2, diubah_at = now() \
         WHERE downstream_endpoint_id = $1",
    )
    .bind(id)
    .bind(versi)
    .execute(&mut **tx)
    .await
    .map_err(|e| galat_db(&e))?;
    Ok(())
}

/// Ganti definisi metadata. `versi_lama` memetakan nama field ke definisi dan
/// versinya sebelum edit; definisi yang sama persis mempertahankan versinya.
async fn tulis_metadata(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    id: Uuid,
    fields: &[MetadataFieldDef],
    versi_baru: i32,
    versi_lama: &BTreeMap<String, (MetadataFieldDef, i32)>,
) -> Result<(), Galat> {
    sqlx::query("DELETE FROM endpoint_metadata_field WHERE downstream_endpoint_id = $1")
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(|e| galat_db(&e))?;
    for (i, m) in fields.iter().enumerate() {
        let versi = match versi_lama.get(&m.nama_field) {
            Some((lama, v)) if lama == m => *v,
            _ => versi_baru,
        };
        sqlx::query(
            "INSERT INTO endpoint_metadata_field (downstream_endpoint_id, nama_field, tipe, wajib, \
             nilai_diizinkan, aturan_validasi, klasifikasi, kirim_ke_data_balikan, deskripsi, versi) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)",
        )
        .bind(id)
        .bind(&m.nama_field)
        .bind(m.tipe.as_str())
        .bind(m.wajib)
        .bind(m.nilai_diizinkan.as_ref().map(sqlx::types::Json))
        .bind(m.aturan_validasi.as_ref().map(sqlx::types::Json))
        .bind(m.klasifikasi.as_str())
        .bind(m.kirim_ke_data_balikan)
        .bind(m.deskripsi.trim())
        .bind(versi)
        .execute(&mut **tx)
        .await
        .map_err(|e| {
            let field = format!("metadata_fields/{i}");
            if let sqlx::Error::Database(db) = &e
                && db.is_check_violation()
            {
                return Galat::validasi(vec![galat_field(field, "format", None)]);
            }
            galat_db(&e)
        })?;
    }
    Ok(())
}

fn ringkas(e: &Endpoint) -> Value {
    // Nilai literal (mis. `TRESHOLD`) bukan rahasia, tetapi audit cukup memuat
    // kuncinya; definisi field metadata disimpan utuh.
    json!({
        "provider_id": e.provider_id,
        "downstream_pool_id": e.downstream_pool_id,
        "endpoint_config_version": e.endpoint_config_version,
        "log_mode": e.log_mode,
        "parameter_literal": e.parameter_literal.keys().collect::<Vec<_>>(),
        "metadata_fields": snapshot(&e.metadata_fields),
    })
}

/// `createEndpoint` (PA): endpoint, versi 1, field metadata, audit; satu
/// transaksi.
pub(super) async fn create_endpoint(
    State(s): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Result<Response, Galat> {
    let peminta = masuk(
        &s,
        &Method::POST,
        &headers,
        peer,
        &PERAN_PA,
        "createEndpoint",
    )
    .await?;
    periksa_guard_mfa(&peminta)?;
    let body: EndpointConfig = urai_body(body, &ENDPOINT_WAJIB, &ENDPOINT_OPSIONAL, &[])?;

    let mut tx = s.pool.begin().await.map_err(|e| galat_db(&e))?;
    siapkan(&mut tx, &body, false).await?;

    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO downstream_endpoint \
         (downstream_endpoint_id, nama, provider_id, downstream_pool_id, aktif, config_versi_aktif) \
         VALUES ($1, $2, $3, $4, true, NULL)",
    )
    .bind(id)
    // Kontrak tidak punya `nama` endpoint; kolomnya NOT NULL UNIQUE.
    .bind(format!("endpoint-{id}"))
    .bind(body.provider_id)
    .bind(body.downstream_pool_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| galat_tulis(&e, "provider_id"))?;
    simpan_versi(&mut tx, id, 1, &body, peminta.pengguna_id).await?;
    tulis_metadata(&mut tx, id, &body.metadata_fields, 1, &BTreeMap::new()).await?;

    let entitas = baca_endpoint(&mut tx, id)
        .await?
        .ok_or_else(Galat::layanan)?;
    catat(
        &mut tx,
        &peminta,
        EntitasKonfigurasi::DownstreamEndpoint,
        AksiKonfigurasi::Buat,
        id,
        None,
        Some(ringkas(&entitas)),
        body.alasan
            .clone()
            .unwrap_or_else(|| "createEndpoint".to_owned()),
    )
    .await?;
    tx.commit().await.map_err(|e| galat_db(&e))?;
    Ok((StatusCode::CREATED, Json(entitas)).into_response())
}

/// `updateEndpoint` (PA): versi `n+1`, versi `n` tetap utuh (V-03).
pub(super) async fn update_endpoint(
    State(s): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    body: Result<Json<Value>, JsonRejection>,
) -> Result<Response, Galat> {
    let peminta = masuk(
        &s,
        &Method::PUT,
        &headers,
        peer,
        &PERAN_PA,
        "updateEndpoint",
    )
    .await?;
    periksa_guard_mfa(&peminta)?;
    let body: EndpointConfig = urai_body(body, &ENDPOINT_WAJIB, &ENDPOINT_OPSIONAL, &[])?;

    let mut tx = s.pool.begin().await.map_err(|e| galat_db(&e))?;
    // Kunci baris supaya dua update bersamaan tidak memilih versi yang sama.
    let terkunci: Option<(Option<i32>,)> = sqlx::query_as(
        "SELECT config_versi_aktif FROM downstream_endpoint \
         WHERE downstream_endpoint_id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| galat_db(&e))?;
    let Some((versi_lama,)) = terkunci else {
        return Err(Galat::tidak_ditemukan());
    };
    siapkan(&mut tx, &body, true).await?;

    let lama = baca_endpoint(&mut tx, id).await?;
    let meta_lama: BTreeMap<String, (MetadataFieldDef, i32)> = baca_metadata(&mut tx, &[id])
        .await?
        .iter()
        .filter_map(|m| Some((m.nama_field.clone(), (m.ke_def()?, m.versi))))
        .collect();
    let versi_baru = versi_lama.unwrap_or(0).max(0) + 1;

    sqlx::query(
        "UPDATE downstream_endpoint SET provider_id = $2, downstream_pool_id = $3, \
         diubah_at = now() WHERE downstream_endpoint_id = $1",
    )
    .bind(id)
    .bind(body.provider_id)
    .bind(body.downstream_pool_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| galat_tulis(&e, "provider_id"))?;
    simpan_versi(&mut tx, id, versi_baru, &body, peminta.pengguna_id).await?;
    tulis_metadata(&mut tx, id, &body.metadata_fields, versi_baru, &meta_lama).await?;

    let entitas = baca_endpoint(&mut tx, id)
        .await?
        .ok_or_else(Galat::layanan)?;
    catat(
        &mut tx,
        &peminta,
        EntitasKonfigurasi::DownstreamEndpoint,
        AksiKonfigurasi::Ubah,
        id,
        lama.as_ref().map(ringkas),
        Some(ringkas(&entitas)),
        body.alasan.clone().unwrap_or_default(),
    )
    .await?;
    tx.commit().await.map_err(|e| galat_db(&e))?;
    Ok((StatusCode::OK, Json(entitas)).into_response())
}

// ---------------------------------------------------------------------------
// setClientEndpoints (AC-26)
// ---------------------------------------------------------------------------

/// `setClientEndpoints` (PA). Mengganti seluruh penugasan dalam satu transaksi;
/// himpunan kosong mencabut semuanya (U-7). Id endpoint yang tidak ada → 422
/// dengan `field` = indeks array; client tidak ada → 404.
pub(super) async fn set_client_endpoints(
    State(s): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(client_id): Path<Uuid>,
    body: Result<Json<Value>, JsonRejection>,
) -> Result<Response, Galat> {
    let peminta = masuk(
        &s,
        &Method::PUT,
        &headers,
        peer,
        &PERAN_PA,
        "setClientEndpoints",
    )
    .await?;
    periksa_guard_mfa(&peminta)?;
    let body: ClientEndpoints = urai_body(body, &["endpoint_ids"], &[], &[])?;

    let mut tx = s.pool.begin().await.map_err(|e| galat_db(&e))?;
    let client: Option<(Uuid, String, i64, f64, bool)> = sqlx::query_as(
        "SELECT client_id, nama, limiter_penolakan_burst::bigint, \
         limiter_penolakan_isi_ulang_per_detik::float8, aktif \
         FROM client WHERE client_id = $1 FOR UPDATE",
    )
    .bind(client_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| galat_db(&e))?;
    let Some((client_id, nama, burst, isi_ulang, aktif)) = client else {
        return Err(Galat::tidak_ditemukan());
    };

    let ada: BTreeSet<Uuid> = sqlx::query_scalar::<_, Uuid>(
        "SELECT downstream_endpoint_id FROM downstream_endpoint \
         WHERE downstream_endpoint_id = ANY($1)",
    )
    .bind(&body.endpoint_ids)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| galat_db(&e))?
    .into_iter()
    .collect();
    let galat: Vec<ValidasiErrorAdmin> = body
        .endpoint_ids
        .iter()
        .enumerate()
        .filter(|(_, id)| !ada.contains(id))
        .map(|(i, _)| galat_field(format!("endpoint_ids/{i}"), "tidak_ada", None))
        .collect();
    if !galat.is_empty() {
        return Err(Galat::validasi(galat));
    }
    let baru: BTreeSet<Uuid> = body.endpoint_ids.iter().copied().collect();

    let lama: Vec<Uuid> = sqlx::query_scalar(
        "SELECT downstream_endpoint_id FROM client_endpoint WHERE client_id = $1 \
         ORDER BY downstream_endpoint_id",
    )
    .bind(client_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| galat_db(&e))?;
    sqlx::query("DELETE FROM client_endpoint WHERE client_id = $1")
        .bind(client_id)
        .execute(&mut *tx)
        .await
        .map_err(|e| galat_db(&e))?;
    for endpoint in &baru {
        sqlx::query(
            "INSERT INTO client_endpoint (client_id, downstream_endpoint_id) VALUES ($1, $2)",
        )
        .bind(client_id)
        .bind(endpoint)
        .execute(&mut *tx)
        .await
        .map_err(|e| galat_tulis(&e, "endpoint_ids"))?;
    }
    catat(
        &mut tx,
        &peminta,
        EntitasKonfigurasi::Client,
        AksiKonfigurasi::Ubah,
        client_id,
        Some(json!({ "endpoint_ids": lama })),
        Some(json!({ "endpoint_ids": baru })),
        "setClientEndpoints".to_owned(),
    )
    .await?;
    tx.commit().await.map_err(|e| galat_db(&e))?;

    let entitas = Client {
        client_id,
        nama,
        limiter_penolakan_burst: burst,
        limiter_penolakan_isi_ulang_per_detik: isi_ulang,
        aktif,
    };
    Ok((StatusCode::OK, Json(entitas)).into_response())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body(v: Value) -> EndpointConfig {
        serde_json::from_value(v).unwrap()
    }

    fn field_status() -> Value {
        json!({ "nama_field": "status_penerima_manfaat", "tipe": "enum", "wajib": true,
                "nilai_diizinkan": ["aktif", "tidak_aktif"], "klasifikasi": "Internal",
                "kirim_ke_data_balikan": true, "deskripsi": "Status penerima manfaat" })
    }

    fn dukcapil() -> InfoProvider {
        InfoProvider {
            data_balikan_aktif: true,
            log_mode: LogMode::MetadataOnly,
            rujukan_endpoint: ["TRESHOLD".to_owned()].into_iter().collect(),
        }
    }

    fn kode(g: &[ValidasiErrorAdmin]) -> Vec<(&str, &str)> {
        g.iter()
            .map(|e| (e.field.as_str(), e.kode.as_str()))
            .collect()
    }

    #[test]
    fn endpoint_dukcapil_valid_lolos() {
        let b = body(json!({
            "provider_id": Uuid::nil(), "downstream_pool_id": Uuid::nil(),
            "parameter_literal": { "TRESHOLD": "90" }, "log_mode": "metadata_only",
            "metadata_fields": [field_status()], "config_schema_version": 1
        }));
        let (g, mr1) = periksa_endpoint(&b, Some(&dukcapil()), false);
        assert!(g.is_empty(), "{g:?}");
        assert!(!mr1);
    }

    #[test]
    fn literal_hilang_mr1_dan_log_mode_dilaporkan_sekaligus() {
        let b = body(json!({
            "provider_id": Uuid::nil(), "downstream_pool_id": Uuid::nil(),
            "parameter_literal": {}, "log_mode": "full_payload",
            "metadata_fields": [], "config_schema_version": 2
        }));
        let (g, mr1) = periksa_endpoint(&b, Some(&dukcapil()), true);
        assert!(mr1);
        assert_eq!(
            kode(&g),
            vec![
                ("config_schema_version", "rentang"),
                ("parameter_literal", "literal_tidak_ada"),
                ("log_mode", "lebih_longgar_dari_provider"),
                ("metadata_fields", "metadata_wajib_hilang"),
                ("alasan", "wajib"),
            ]
        );
        assert_eq!(g[1].nama.as_deref(), Some("TRESHOLD"));
    }

    #[test]
    fn metadata_diperiksa_terhadap_check_tabel() {
        let b = body(json!({
            "provider_id": Uuid::nil(), "downstream_pool_id": Uuid::nil(),
            "parameter_literal": { "TRESHOLD": "90" }, "log_mode": "metadata_only",
            "metadata_fields": [
                field_status(),
                { "nama_field": "Bukan-Sah", "tipe": "string", "wajib": false,
                  "klasifikasi": "Terbatas", "kirim_ke_data_balikan": false, "deskripsi": "x" },
                { "nama_field": "pilihan", "tipe": "enum", "wajib": false,
                  "klasifikasi": "Rahasia", "kirim_ke_data_balikan": false, "deskripsi": " " },
                field_status()
            ],
            "config_schema_version": 1
        }));
        let (g, _) = periksa_endpoint(&b, Some(&dukcapil()), false);
        assert_eq!(
            kode(&g),
            vec![
                ("metadata_fields/1/nama_field", "format"),
                ("metadata_fields/2/nilai_diizinkan", "wajib"),
                ("metadata_fields/2/deskripsi", "wajib"),
                ("metadata_fields/3/nama_field", "sudah_dipakai"),
            ]
        );
    }

    #[test]
    fn klasifikasi_huruf_kecil_ditolak_saat_urai() {
        let r: Result<MetadataFieldDef, _> = serde_json::from_value(json!({
            "nama_field": "a", "tipe": "string", "wajib": true, "klasifikasi": "internal",
            "kirim_ke_data_balikan": false, "deskripsi": "x"
        }));
        assert!(r.is_err());
    }

    #[test]
    fn provider_tidak_ada_dilaporkan() {
        let b = body(json!({
            "provider_id": Uuid::nil(), "downstream_pool_id": Uuid::nil(),
            "parameter_literal": {}, "log_mode": "metadata_only",
            "metadata_fields": [], "config_schema_version": 1
        }));
        let (g, mr1) = periksa_endpoint(&b, None, false);
        assert_eq!(kode(&g), vec![("provider_id", "tidak_ada")]);
        assert!(!mr1);
    }
}
