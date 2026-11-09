//! Client credentials: `listCredentials`, `createCredential` (AC-20).
//!
//! A secret is generated here, returned once, and never stored: the row keeps
//! only the hash the downstream API checks (SHA-256 for an API key, argon2id for
//! a client secret, `core::kredensial`). `listCredentials` has no hash column in
//! its projection. The trigger on `client_kredensial` enforces two active per
//! client; its `check_violation` becomes 409 `batas_kredensial`.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Duration, Utc};
use poolrouter_core::audit::{
    AksiKonfigurasi, EntitasKonfigurasi, KonfigurasiBaru, Penulis, Peran,
};
use poolrouter_core::kredensial::{PARAMETER_SECRET, buat_api_key, hash_secret_klien, secret_acak};
use serde_json::json;
use uuid::Uuid;

use super::gerbang::{galat_validasi, masuk};
use super::k20::{self, GalatK20, JenisEntitas};
use super::sesi::{AdminState, Galat};
use crate::api::admin::{
    AlasanK20, Credential, CredentialCreate, CredentialSecret, FieldError, PratinjauK20,
};
use crate::api::enums::{JenisKredensial, StatusKredensial};
use crate::api::problem::ProblemKodeAdmin;

/// `fn_nonaktifkan_entitas`/`fn_pratinjau_nonaktif` accept a client owner for
/// its own credential (migration `20261103000003_functions.sql`); the role
/// gate here matches, and cross-client ownership is checked afterwards.
const PERAN: [Peran; 2] = [Peran::PlatformAdmin, Peran::ClientOwner];
/// `fn_aktifkan_entitas` has no client_owner branch for any entity kind.
const PERAN_AKTIFKAN: [Peran; 1] = [Peran::PlatformAdmin];
const KEDALUWARSA_MAKS_HARI: i64 = 365;

/// One credential row as sqlx returns it: id, kind, status, expiry, revocation.
type BarisKredensial = (
    Uuid,
    String,
    String,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
);
/// `BarisKredensial` plus `client_id`, for the K-20 handlers that need to check
/// ownership before acting.
type BarisKredensialDenganClient = (
    Uuid,
    Uuid,
    String,
    String,
    Option<DateTime<Utc>>,
    Option<DateTime<Utc>>,
);

pub(super) async fn list_credentials(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(client_id): Path<Uuid>,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &method, &headers, peer, &PERAN, "listCredentials").await?;
    pastikan_milik(&peminta, client_id)?;
    ada_client(&s, client_id).await?;

    let rows: Vec<BarisKredensial> = sqlx::query_as(
        "SELECT credential_id, jenis, status, kedaluwarsa_at, dicabut_at \
               FROM client_kredensial WHERE client_id = $1 \
               ORDER BY dibuat_at DESC",
    )
    .bind(client_id)
    .fetch_all(&s.pool)
    .await
    .map_err(|e| galat_db(&e))?;

    let sekarang = (s.jam)();
    let daftar: Vec<Credential> = rows
        .into_iter()
        .map(
            |(credential_id, jenis, status, kedaluwarsa_at, dicabut_at)| Credential {
                credential_id,
                jenis: jenis_dari_db(&jenis),
                status: status_dari_db(&status, kedaluwarsa_at, sekarang),
                kedaluwarsa_at,
                dicabut_at,
            },
        )
        .collect();
    Ok((StatusCode::OK, axum::Json(daftar)).into_response())
}

pub(super) async fn create_credential(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(client_id): Path<Uuid>,
    axum::Json(body): axum::Json<CredentialCreate>,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &method, &headers, peer, &PERAN, "createCredential").await?;
    pastikan_milik(&peminta, client_id)?;
    ada_client(&s, client_id).await?;

    let sekarang = (s.jam)();
    let kedaluwarsa_at = body
        .kedaluwarsa_at
        .unwrap_or(sekarang + Duration::days(KEDALUWARSA_MAKS_HARI));
    if kedaluwarsa_at > sekarang + Duration::days(KEDALUWARSA_MAKS_HARI) {
        return Err(galat_validasi(vec![FieldError {
            field: "kedaluwarsa_at",
            kode: "rentang",
            batas: Some(KEDALUWARSA_MAKS_HARI as f64),
        }]));
    }

    let credential_id = Uuid::now_v7();
    let (secret, kunci_hash) = match body.jenis {
        JenisKredensial::ApiKey => {
            let kunci = buat_api_key(credential_id).map_err(|_| Galat::layanan())?;
            (kunci.kunci, kunci.kunci_hash.to_vec())
        }
        JenisKredensial::Oauth2 => {
            let secret = secret_acak().map_err(|_| Galat::layanan())?;
            let phc = hash_secret_klien(&secret, PARAMETER_SECRET).map_err(|_| Galat::layanan())?;
            (secret, phc.as_bytes().to_vec())
        }
    };

    let mut tx = s.pool.begin().await.map_err(|e| galat_db(&e))?;
    sqlx::query(
        "INSERT INTO client_kredensial \
         (credential_id, client_id, jenis, kunci_hash, status, kedaluwarsa_at, dibuat_oleh) \
         VALUES ($1, $2, $3, $4, 'aktif', $5, $6)",
    )
    .bind(credential_id)
    .bind(client_id)
    .bind(jenis_ke_db(body.jenis))
    .bind(&kunci_hash)
    .bind(kedaluwarsa_at)
    .bind(peminta.pengguna_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| galat_simpan(&e))?;

    let baru = json!({
        "jenis": jenis_json(body.jenis),
        "status": "aktif",
        "kedaluwarsa_at": kedaluwarsa_at,
    });
    Penulis::baru(&mut tx)
        .catat_konfigurasi(&KonfigurasiBaru {
            pelaku: peminta.pengguna_id,
            entitas: EntitasKonfigurasi::Kredensial,
            entitas_id: Some(credential_id),
            aksi: AksiKonfigurasi::Buat,
            nilai_lama: None,
            nilai_baru: Some(baru),
            alasan: poolrouter_core::audit::Alasan::baru(format!(
                "createCredential {credential_id}"
            ))
            .map_err(|_| Galat::layanan())?,
        })
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, "admin: audit konfigurasi kredensial gagal");
            Galat::layanan()
        })?;
    tx.commit().await.map_err(|e| galat_db(&e))?;

    Ok((
        StatusCode::CREATED,
        axum::Json(CredentialSecret {
            credential_id,
            client_id,
            secret,
            kedaluwarsa_at: Some(kedaluwarsa_at),
        }),
    )
        .into_response())
}

/// Preview credential revocation (K-20, AC-28): counts only.
pub(super) async fn pratinjau_cabut_credential(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<Uuid>,
) -> Result<Response, Galat> {
    let peminta = masuk(
        &s,
        &method,
        &headers,
        peer,
        &PERAN,
        "pratinjauCabutCredential",
    )
    .await?;
    let baris = ada_kredensial(&s, id).await?;
    pastikan_milik(&peminta, baris.client_id)?;

    let hasil = s
        .k20
        .pratinjau(JenisEntitas::Kredensial, id)
        .await
        .map_err(|e| galat_k20(&e))?;
    Ok((
        StatusCode::OK,
        axum::Json(PratinjauK20 {
            akan_cancelled: i64::from(hasil.akan_cancelled),
            akan_failed: i64::from(hasil.akan_failed),
            dihitung_pada: hasil.dihitung_at,
            upstream_aktif_terakhir_untuk: None,
            dipakai_oleh_downstream_pool_count: None,
        }),
    )
        .into_response())
}

/// Revoke a credential (K-20, AC-29): one mass cancel/fail transaction, then
/// the credential itself turns `dicabut`.
pub(super) async fn cabut_credential(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<Uuid>,
    axum::Json(body): axum::Json<AlasanK20>,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &method, &headers, peer, &PERAN, "cabutCredential").await?;
    let baris = ada_kredensial(&s, id).await?;
    pastikan_milik(&peminta, baris.client_id)?;

    s.k20
        .nonaktifkan(
            JenisEntitas::Kredensial,
            id,
            peminta.pengguna_id,
            &body.alasan,
        )
        .await
        .map_err(|e| galat_k20(&e))?;

    let setelah = ada_kredensial(&s, id).await?;
    Ok((StatusCode::OK, axum::Json(satu_credential(setelah))).into_response())
}

/// Re-activate a credential: always 409 `kredensial_dicabut`, revoked or not
/// (U-54b; `fn_aktifkan_entitas` has no credential path other than this
/// exception).
pub(super) async fn aktifkan_credential(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<Uuid>,
) -> Result<Response, Galat> {
    let _peminta = masuk(
        &s,
        &method,
        &headers,
        peer,
        &PERAN_AKTIFKAN,
        "aktifkanCredential",
    )
    .await?;
    // `fn_aktifkan_entitas` has no credential path other than raising
    // `kredensial_dicabut_terminal` (U-54b); the outcome is known up front, so
    // reject here without running the transactional SQL (and its audit row).
    ada_kredensial(&s, id).await?;
    Err(Galat::baru(
        StatusCode::CONFLICT,
        ProblemKodeAdmin::KredensialDicabut,
    ))
}

/// One credential row, by id; 404 when it does not exist.
async fn ada_kredensial(
    s: &AdminState,
    credential_id: Uuid,
) -> Result<BarisKredensialMilik, Galat> {
    let baris: Option<BarisKredensialDenganClient> = sqlx::query_as(
        "SELECT credential_id, client_id, jenis, status, kedaluwarsa_at, dicabut_at \
               FROM client_kredensial WHERE credential_id = $1",
    )
    .bind(credential_id)
    .fetch_optional(&s.pool)
    .await
    .map_err(|e| galat_db(&e))?;
    baris
        .map(
            |(credential_id, client_id, jenis, status, kedaluwarsa_at, dicabut_at)| {
                BarisKredensialMilik {
                    credential_id,
                    client_id,
                    jenis,
                    status,
                    kedaluwarsa_at,
                    dicabut_at,
                }
            },
        )
        .ok_or_else(Galat::tidak_ditemukan)
}

struct BarisKredensialMilik {
    credential_id: Uuid,
    client_id: Uuid,
    jenis: String,
    status: String,
    kedaluwarsa_at: Option<DateTime<Utc>>,
    dicabut_at: Option<DateTime<Utc>>,
}

fn satu_credential(baris: BarisKredensialMilik) -> Credential {
    let sekarang = Utc::now();
    Credential {
        credential_id: baris.credential_id,
        jenis: jenis_dari_db(&baris.jenis),
        status: status_dari_db(&baris.status, baris.kedaluwarsa_at, sekarang),
        kedaluwarsa_at: baris.kedaluwarsa_at,
        dicabut_at: baris.dicabut_at,
    }
}

/// `fn_nonaktifkan_entitas`/`fn_pratinjau_nonaktif` only ever raise
/// `check_violation` (23514, the alasan-empty guard — unreachable, the body is
/// already validated) or `insufficient_privilege` (42501 — also unreachable,
/// `masuk`/`pastikan_milik` already checked); anything else is a database
/// failure.
fn galat_k20(e: &GalatK20) -> Galat {
    match e {
        GalatK20::AlasanTidakValid => galat_validasi(vec![FieldError {
            field: "alasan",
            kode: "rentang",
            batas: Some(k20::ALASAN_MAX as f64),
        }]),
        GalatK20::BasisData(e) => {
            tracing::error!(error = %e, "admin: operasi K-20 kredensial gagal");
            Galat::layanan()
        }
    }
}

fn pastikan_milik(peminta: &super::gerbang::Peminta, client_id: Uuid) -> Result<(), Galat> {
    if peminta.peran == Peran::ClientOwner && peminta.client_id != Some(client_id) {
        return Err(Galat::tidak_ditemukan());
    }
    Ok(())
}

async fn ada_client(s: &AdminState, client_id: Uuid) -> Result<(), Galat> {
    let ada: Option<Uuid> = sqlx::query_scalar("SELECT client_id FROM client WHERE client_id = $1")
        .bind(client_id)
        .fetch_optional(&s.pool)
        .await
        .map_err(|e| galat_db(&e))?;
    if ada.is_none() {
        return Err(Galat::tidak_ditemukan());
    }
    Ok(())
}

fn jenis_ke_db(jenis: JenisKredensial) -> &'static str {
    match jenis {
        JenisKredensial::Oauth2 => "oauth2_client_credentials",
        JenisKredensial::ApiKey => "api_key",
    }
}

fn jenis_json(jenis: JenisKredensial) -> &'static str {
    match jenis {
        JenisKredensial::Oauth2 => "oauth2",
        JenisKredensial::ApiKey => "api_key",
    }
}

fn jenis_dari_db(jenis: &str) -> JenisKredensial {
    if jenis == "api_key" {
        JenisKredensial::ApiKey
    } else {
        JenisKredensial::Oauth2
    }
}

/// An expired-but-not-revoked credential reads as `kedaluwarsa`; revocation is
/// terminal and wins (EN-03).
fn status_dari_db(
    status: &str,
    kedaluwarsa_at: Option<DateTime<Utc>>,
    sekarang: DateTime<Utc>,
) -> StatusKredensial {
    if status == "dicabut" {
        StatusKredensial::Dicabut
    } else if kedaluwarsa_at.is_some_and(|t| t <= sekarang) {
        StatusKredensial::Kedaluwarsa
    } else {
        StatusKredensial::Aktif
    }
}

/// The only check this insert can raise is the two-active-per-client trigger.
fn galat_simpan(e: &sqlx::Error) -> Galat {
    if e.as_database_error()
        .and_then(|d| d.code())
        .is_some_and(|c| c == "23514")
    {
        return Galat::baru(StatusCode::CONFLICT, ProblemKodeAdmin::BatasKredensial);
    }
    tracing::error!(error = %e, "admin: tulis kredensial gagal");
    Galat::layanan()
}

fn galat_db(e: &sqlx::Error) -> Galat {
    tracing::error!(error = %e, "admin: kueri kredensial gagal");
    Galat::layanan()
}
