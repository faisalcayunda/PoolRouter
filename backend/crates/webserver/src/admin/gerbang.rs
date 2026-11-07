//! Role guard for the admin API (DESIGN.md §3; AC-3, AC-4).
//!
//! Every admin write endpoint calls [`masuk`] before it reads its body: valid
//! `Origin` for write methods, a live session, and one of the roles the
//! operation requires. A caller who is authenticated but holds the wrong role
//! gets 403 `akses_ditolak` **and** exactly one `audit_akses` row.
//!
//! The denial row commits in its own transaction. `core::audit::Penulis` shares
//! the caller's transaction so effect and trail commit together, but here the
//! effect is nothing and the row must survive the 403, so it is committed before
//! the response is built. If the row cannot be written the answer is 503: a
//! refusal we cannot record is not a refusal we may answer with 403 (AC-4).

use std::net::SocketAddr;

use axum::http::{HeaderMap, Method, StatusCode};
use chrono::{DateTime, Utc};
use poolrouter_core::admin::{GalatSesi, SesiDitolak, SesiTersimpan};
use poolrouter_core::audit::{AksesBaru, Pelaku, Penulis, Peran};
use serde_json::json;
use uuid::Uuid;

use super::login::sumber_ip;
use super::sesi::{AdminState, Galat, hash_dari_cookie, periksa_origin};
use crate::api::admin::FieldError;
use crate::api::problem::ValidasiErrorAdmin;

/// The caller of an authorized admin operation.
#[derive(Debug, Clone)]
pub(super) struct Peminta {
    pub pengguna_id: Uuid,
    /// The role that authorized this call: the first accepted role the caller
    /// holds, in the order the caller passed as `diterima`.
    pub peran: Peran,
    pub sesi_id: Uuid,
    pub sumber_ip: String,
    /// The client a `client_owner` administers (`admin_user.client_id`). Loaded
    /// only when the operation accepts `client_owner`; `None` for every other
    /// role.
    pub client_id: Option<Uuid>,
    /// MFA verification timestamp, if the session has passed the MFA step (AC-15).
    pub mfa_terverifikasi_at: Option<DateTime<Utc>>,
}

/// Check `Origin` (write methods only), the session, and the role, in that
/// order. The order matters: a foreign origin must not reach the session row
/// (ADR-0003 rev 5 §3.6f) and must not write an audit row either.
///
/// `operasi` names the attempted operation in the denial row (`putSecret`, ...).
pub(super) async fn masuk(
    s: &AdminState,
    method: &Method,
    headers: &HeaderMap,
    peer: SocketAddr,
    diterima: &[Peran],
    operasi: &'static str,
) -> Result<Peminta, Galat> {
    periksa_origin(method, headers, &s.asal)?;
    let hash = hash_dari_cookie(headers)?;
    let sekarang = (s.jam)();
    let sesi = s
        .sesi
        .autentikasi(&hash, sekarang)
        .await
        .map_err(|e| galat_sesi(&e))?
        .map_err(tidak_terautentikasi)?;

    let ditemukan = peran_aktif(s, sesi.pengguna_id, sekarang).await?;
    let ip = sumber_ip(headers, peer);
    let Some(peran) = ditemukan.iter().find(|p| diterima.contains(p)).copied() else {
        // The row keeps a role even for a caller with no active role: the column
        // is NOT NULL, and recording the role the operation needed states what
        // was attempted instead of inventing a role the caller does not have.
        let tercatat = ditemukan
            .first()
            .copied()
            .or_else(|| diterima.first().copied())
            .unwrap_or(Peran::PlatformAdmin);
        catat_penolakan(s, sesi, tercatat, operasi, &ip).await?;
        tracing::debug!(
            pengguna_id = %sesi.pengguna_id,
            ditemukan = ?ditemukan,
            diterima = ?diterima,
            "admin: peran tidak mencukupi"
        );
        return Err(Galat::baru(
            StatusCode::FORBIDDEN,
            crate::api::problem::ProblemKodeAdmin::AksesDitolak,
        ));
    };

    let peminta = Peminta {
        pengguna_id: sesi.pengguna_id,
        peran,
        sesi_id: sesi.sesi_id,
        sumber_ip: ip,
        client_id: None,
        mfa_terverifikasi_at: sesi.mfa_terverifikasi_at,
    };
    let peminta = if diterima.contains(&Peran::ClientOwner) {
        let client_id = sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT client_id FROM admin_user WHERE pengguna_id = $1",
        )
        .bind(peminta.pengguna_id)
        .fetch_one(&s.pool)
        .await
        .map_err(|e| galat_db(&e))?;
        Peminta {
            client_id,
            ..peminta
        }
    } else {
        peminta
    };
    // One line per authorized admin operation: who, as which role, from where,
    // doing what. Ids and names only — never a payload value (AC-6).
    tracing::debug!(
        pengguna_id = %peminta.pengguna_id,
        peran = %peminta.peran,
        sesi_id = %peminta.sesi_id,
        sumber_ip = %peminta.sumber_ip,
        operasi,
        "admin: operasi diizinkan"
    );
    Ok(peminta)
}

/// One `audit_akses` row per refused request, committed on its own so it
/// outlives the 403 (AC-4).
async fn catat_penolakan(
    s: &AdminState,
    sesi: SesiTersimpan,
    peran: Peran,
    operasi: &'static str,
    ip: &str,
) -> Result<(), Galat> {
    let baris = AksesBaru::ditolak(Pelaku::baru(sesi.pengguna_id, peran))
        .dengan_filter(json!({ "operasi": operasi }))
        .dengan_sumber_ip(ip)
        .dengan_sesi(sesi.sesi_id.to_string());
    let mut tx = s.pool.begin().await.map_err(|e| galat_db(&e))?;
    Penulis::baru(&mut tx)
        .catat_akses(baris)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "admin: gagal menulis audit akses ditolak");
            Galat::layanan()
        })?;
    tx.commit().await.map_err(|e| galat_db(&e))?;
    Ok(())
}

/// Roles that are granted, not revoked and not expired at `sekarang`.
pub(super) async fn peran_aktif(
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
    Ok(baris
        .iter()
        .filter_map(|(p,)| p.parse::<Peran>().ok())
        .collect())
}

/// 422 `validasi_gagal` from a pure validator's output (AC-7): every failing
/// field with its code, `batas` only when the rule has a numeric parameter, and
/// never the rejected input value.
pub(super) fn galat_validasi(errors: Vec<FieldError>) -> Galat {
    let errors = errors
        .into_iter()
        .map(|e| ValidasiErrorAdmin {
            field: e.field.to_owned(),
            kode: e.kode.to_owned(),
            batas: e.batas,
            posisi: None,
        })
        .collect();
    Galat::validasi(errors)
}

fn tidak_terautentikasi(alasan: SesiDitolak) -> Galat {
    tracing::debug!(?alasan, "admin: sesi ditolak");
    Galat::tidak_terautentikasi()
}

fn galat_sesi(e: &GalatSesi) -> Galat {
    // The driver text can carry connection details; log it, never return it.
    tracing::error!(error = %e, "admin: penyimpanan sesi gagal");
    Galat::layanan()
}

fn galat_db(e: &sqlx::Error) -> Galat {
    tracing::error!(error = %e, "admin: kueri gagal");
    Galat::layanan()
}
