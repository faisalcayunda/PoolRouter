//! `getMe` and `updatePreferensi` (ADR-0003 rev 5 §3.6; Addendum A §7.2).
//!
//! - `getMe` is an ordinary authenticated request: it pushes the idle deadline
//!   like any other call, and returns identity, roles and `bahasa_ui`.
//! - `PATCH auth/me/preferensi` changes only the caller's own language. There
//!   is no user id in the path or body (the body denies unknown fields), so
//!   there is nothing to point at another user. It does **not** touch the
//!   session (read-only identity lookup), writes no `audit_akses` row, and is
//!   guarded by the `Origin`/`Sec-Fetch-Site` check like every write.

use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use poolrouter_core::admin::sesi_store::SesiDitolak;
use uuid::Uuid;

use super::sesi::{AdminState, Galat, hash_dari_cookie, periksa_origin};
use crate::api::admin::{Me, MeMfa, PreferensiUpdate};
use crate::api::enums::{BahasaUi, Peran};
use crate::api::problem::ProblemKodeAdmin;

/// The `Me.mfa` object for `getMe` (AC-14).
///
/// `wajib` is the policy answer, `aktif` the enrollment answer, and
/// `sisa_kode_pemulihan` the count of unused recovery codes. Read-only: it
/// writes no `audit_akses` row and does not touch the session.
async fn mfa_untuk(s: &AdminState, pengguna_id: Uuid, peran: &[Peran]) -> Result<MeMfa, Galat> {
    // Same rule as the login step: `mfa_wajib` plus a role ADR-0006 §9 covers.
    let wajib = s.mfa_wajib
        && peran
            .iter()
            .any(|p| matches!(p, Peran::Auditor | Peran::PlatformAdmin));
    let aktif = s.mfa.aktif(pengguna_id).await.map_err(|e| {
        tracing::error!(error = %e, "admin: pembacaan status MFA gagal");
        Galat::layanan()
    })?;
    let sisa = s.mfa.sisa_kode_pemulihan(pengguna_id).await.map_err(|e| {
        tracing::error!(error = %e, "admin: penghitungan kode pemulihan gagal");
        Galat::layanan()
    })?;
    Ok(MeMfa {
        wajib,
        aktif,
        sisa_kode_pemulihan: sisa,
    })
}

fn peran_dari_teks(t: &str) -> Option<Peran> {
    match t {
        "auditor" => Some(Peran::Auditor),
        "operator" => Some(Peran::Operator),
        "platform_admin" => Some(Peran::PlatformAdmin),
        "client_owner" => Some(Peran::ClientOwner),
        _ => None,
    }
}

fn bahasa_dari_teks(t: &str) -> Option<BahasaUi> {
    match t {
        "id" => Some(BahasaUi::Id),
        "en" => Some(BahasaUi::En),
        _ => None,
    }
}

fn galat_db(e: &sqlx::Error) -> Galat {
    // The driver text can carry connection details; log it, never return it.
    tracing::error!(error = %e, "admin: kueri gagal");
    Galat::layanan()
}

fn tidak_terautentikasi(alasan: SesiDitolak) -> Galat {
    tracing::debug!(?alasan, "admin: sesi ditolak");
    Galat::baru(
        StatusCode::UNAUTHORIZED,
        ProblemKodeAdmin::TidakTerautentikasi,
    )
}

/// Roles that are granted, not revoked and not expired at `sekarang`.
async fn peran_aktif(
    s: &AdminState,
    pengguna_id: Uuid,
    sekarang: DateTime<Utc>,
) -> Result<Vec<Peran>, Galat> {
    let baris: Vec<(String,)> = sqlx::query_as(
        "SELECT peran FROM admin_user_peran \
         WHERE pengguna_id = $1 AND dicabut_at IS NULL \
           AND (berlaku_sampai IS NULL OR berlaku_sampai > $2) \
         ORDER BY peran",
    )
    .bind(pengguna_id)
    .bind(sekarang)
    .fetch_all(&s.pool)
    .await
    .map_err(|e| galat_db(&e))?;
    Ok(baris.iter().filter_map(|(p,)| peran_dari_teks(p)).collect())
}

pub(super) async fn get_me(
    State(s): State<AdminState>,
    headers: HeaderMap,
) -> Result<Response, Galat> {
    let hash = hash_dari_cookie(&headers)?;
    let sekarang = (s.jam)();
    let sesi = s
        .sesi
        .autentikasi(&hash, sekarang)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "admin: penyimpanan sesi gagal");
            Galat::layanan()
        })?
        .map_err(tidak_terautentikasi)?;

    let baris: (String, Option<Uuid>, Option<String>) = sqlx::query_as(
        "SELECT username, client_id, bahasa_ui FROM admin_user WHERE pengguna_id = $1",
    )
    .bind(sesi.pengguna_id)
    .fetch_one(&s.pool)
    .await
    .map_err(|e| galat_db(&e))?;
    let peran = peran_aktif(&s, sesi.pengguna_id, sekarang).await?;
    let mfa = mfa_untuk(&s, sesi.pengguna_id, &peran).await?;
    let me = Me {
        pengguna_id: sesi.pengguna_id,
        username: baris.0,
        peran,
        client_id: baris.1,
        bahasa_ui: baris.2.as_deref().and_then(bahasa_dari_teks),
        mfa,
    };
    Ok((StatusCode::OK, axum::Json(me)).into_response())
}

pub(super) async fn update_preferensi(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    body: Result<axum::Json<PreferensiUpdate>, JsonRejection>,
) -> Result<Response, Galat> {
    periksa_origin(&method, &headers, &s.asal)?;
    let hash = hash_dari_cookie(&headers)?;
    let sesi = s
        .sesi
        .identitas(&hash, (s.jam)())
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "admin: penyimpanan sesi gagal");
            Galat::layanan()
        })?
        .map_err(tidak_terautentikasi)?;
    // After the session check on purpose: an unauthenticated caller learns
    // nothing about the body schema.
    let axum::Json(req) = body.map_err(|_| {
        Galat::baru(
            StatusCode::UNPROCESSABLE_ENTITY,
            ProblemKodeAdmin::ValidasiGagal,
        )
    })?;
    let nilai = match req.bahasa_ui {
        BahasaUi::Id => "id",
        BahasaUi::En => "en",
    };
    sqlx::query("UPDATE admin_user SET bahasa_ui = $2, diubah_at = now() WHERE pengguna_id = $1")
        .bind(sesi.pengguna_id)
        .bind(nilai)
        .execute(&s.pool)
        .await
        .map_err(|e| galat_db(&e))?;
    Ok((StatusCode::OK, axum::Json(req)).into_response())
}
