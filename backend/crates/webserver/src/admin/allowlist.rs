//! Webhook domain allowlist: `listWebhookAllowlist`,
//! `putWebhookAllowlistEntry`, `deleteWebhookAllowlistEntry` (AC-24; US-06).
//!
//! PA only. The list is global: `putWebhookConfig` checks it on every save, and
//! the webhook sender checks it again on every delivery. Delete is idempotent
//! (204 even when the row is already gone), and every write leaves one
//! `audit_konfigurasi` row.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use poolrouter_core::audit::{
    AksiKonfigurasi, Alasan, EntitasKonfigurasi, KonfigurasiBaru, Penulis, Peran,
};
use serde::Deserialize;
use serde_json::json;

use super::gerbang::{galat_validasi, masuk};
use super::sesi::{AdminState, Galat};
use crate::api::admin::{AllowlistEntry, FieldError};

const PERAN: [Peran; 1] = [Peran::PlatformAdmin];
const DOMAIN_MAX: usize = 253;

pub(super) async fn list_allowlist(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &method, &headers, peer, &PERAN, "listWebhookAllowlist").await?;
    super::gerbang::periksa_guard_mfa(&peminta)?;

    let rows: Vec<(String, bool)> =
        sqlx::query_as("SELECT domain, aktif FROM webhook_domain_allowlist ORDER BY domain")
            .fetch_all(&s.pool)
            .await
            .map_err(|e| galat_db(&e))?;
    let daftar: Vec<AllowlistEntry> = rows
        .into_iter()
        .map(|(domain, aktif)| AllowlistEntry { domain, aktif })
        .collect();
    Ok((StatusCode::OK, axum::Json(daftar)).into_response())
}

#[derive(Deserialize)]
pub(super) struct PutAllowlistBody {
    pub aktif: bool,
}

pub(super) async fn put_allowlist_entry(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(domain): Path<String>,
    axum::Json(body): axum::Json<PutAllowlistBody>,
) -> Result<Response, Galat> {
    let peminta = masuk(
        &s,
        &method,
        &headers,
        peer,
        &PERAN,
        "putWebhookAllowlistEntry",
    )
    .await?;
    super::gerbang::periksa_guard_mfa(&peminta)?;
    let domain = periksa_domain(&domain)?;

    let mut tx = s.pool.begin().await.map_err(|e| galat_db(&e))?;
    sqlx::query(
        "INSERT INTO webhook_domain_allowlist (domain, aktif, diubah_oleh) VALUES ($1, $2, $3) \
         ON CONFLICT (domain) DO UPDATE \
         SET aktif = EXCLUDED.aktif, \
             diubah_oleh = EXCLUDED.diubah_oleh, \
             diubah_at = now()",
    )
    .bind(&domain)
    .bind(body.aktif)
    .bind(peminta.pengguna_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| galat_db(&e))?;
    catat(
        &mut tx,
        &peminta,
        AksiKonfigurasi::Ubah,
        json!({ "domain": domain, "aktif": body.aktif }),
        format!("putWebhookAllowlistEntry {domain}"),
    )
    .await?;
    tx.commit().await.map_err(|e| galat_db(&e))?;

    Ok((
        StatusCode::OK,
        axum::Json(AllowlistEntry {
            domain,
            aktif: body.aktif,
        }),
    )
        .into_response())
}

pub(super) async fn delete_allowlist_entry(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(domain): Path<String>,
) -> Result<Response, Galat> {
    let peminta = masuk(
        &s,
        &method,
        &headers,
        peer,
        &PERAN,
        "deleteWebhookAllowlistEntry",
    )
    .await?;
    super::gerbang::periksa_guard_mfa(&peminta)?;
    let domain = periksa_domain(&domain)?;

    let mut tx = s.pool.begin().await.map_err(|e| galat_db(&e))?;
    sqlx::query("DELETE FROM webhook_domain_allowlist WHERE domain = $1")
        .bind(&domain)
        .execute(&mut *tx)
        .await
        .map_err(|e| galat_db(&e))?;
    catat(
        &mut tx,
        &peminta,
        AksiKonfigurasi::Cabut,
        json!({ "domain": domain }),
        format!("deleteWebhookAllowlistEntry {domain}"),
    )
    .await?;
    tx.commit().await.map_err(|e| galat_db(&e))?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

fn periksa_domain(domain: &str) -> Result<String, Galat> {
    let domain = domain.trim().to_lowercase();
    let panjang = domain.chars().count();
    let sah = !domain.is_empty()
        && panjang <= DOMAIN_MAX
        && domain
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.');
    if !sah {
        return Err(galat_validasi(vec![FieldError {
            field: "domain",
            kode: "format",
            batas: Some(DOMAIN_MAX as f64),
        }]));
    }
    Ok(domain)
}

async fn catat(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    peminta: &super::gerbang::Peminta,
    aksi: AksiKonfigurasi,
    nilai_baru: serde_json::Value,
    alasan: String,
) -> Result<(), Galat> {
    let alasan = Alasan::baru(alasan).map_err(|_| {
        galat_validasi(vec![FieldError {
            field: "domain",
            kode: "rentang",
            batas: Some(DOMAIN_MAX as f64),
        }])
    })?;
    Penulis::baru(tx)
        .catat_konfigurasi(&KonfigurasiBaru {
            pelaku: peminta.pengguna_id,
            entitas: EntitasKonfigurasi::WebhookDomainAllowlist,
            entitas_id: None,
            aksi,
            nilai_lama: None,
            nilai_baru: Some(nilai_baru),
            alasan,
        })
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, "admin: audit konfigurasi allowlist gagal");
            Galat::layanan()
        })?;
    Ok(())
}

fn galat_db(e: &sqlx::Error) -> Galat {
    tracing::error!(error = %e, "admin: kueri allowlist gagal");
    Galat::layanan()
}
