//! Webhook config and delivery log: `getWebhookConfig`, `putWebhookConfig`,
//! `rotateWebhookSecret`, `listWebhookLog` (AC-21–23; DESIGN.md §3.2).
//!
//! A config row may exist without a secret: `secret_versi` and the envelope
//! columns stay NULL until the first rotation, which generates the Standard
//! Webhooks secret at versi 1 (AC-22). The envelope is the shared
//! `core::secret::Brankas` from T1, so `pr_admin_api` cannot SELECT it back.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use poolrouter_core::admin::NilaiAman;
use poolrouter_core::audit::{
    AksiKonfigurasi, Alasan, EntitasKonfigurasi, KonfigurasiBaru, Penulis, Peran,
};
use poolrouter_core::kredensial::buat_secret_webhook;
use serde::Deserialize;
use serde_json::json;
use uuid::Uuid;

use super::gerbang::{galat_validasi, masuk};
use super::sesi::{AdminState, Galat};
use crate::api::admin::{FieldError, WebhookConfig, WebhookLog, WebhookSecret};
use crate::api::enums::{HasilWebhook, KelasErrorWebhook};
use crate::api::problem::ProblemKodeAdmin;

const PERAN: [Peran; 2] = [Peran::PlatformAdmin, Peran::ClientOwner];
const LOG_MAKS: i64 = 100;

/// One delivery-log row as sqlx returns it: request, time, outcome, status, error class, secret version.
type BarisLog = (
    Uuid,
    DateTime<Utc>,
    String,
    Option<i32>,
    Option<String>,
    Option<i64>,
);

pub(super) async fn get_webhook_config(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(client_id): Path<Uuid>,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &method, &headers, peer, &PERAN, "getWebhookConfig").await?;
    super::gerbang::periksa_guard_mfa(&peminta)?;
    pastikan_milik(&peminta, client_id)?;

    let baris: Option<(String, bool, Option<i64>)> = sqlx::query_as(
        "SELECT webhook_url, aktif, secret_versi::bigint FROM client_webhook WHERE client_id = $1",
    )
    .bind(client_id)
    .fetch_optional(&s.pool)
    .await
    .map_err(|e| galat_db(&e))?;
    let Some((webhook_url, aktif, secret_versi)) = baris else {
        return Err(Galat::tidak_ditemukan());
    };

    Ok((
        StatusCode::OK,
        axum::Json(WebhookConfig {
            webhook_url: Some(webhook_url),
            aktif,
            secret_versi,
        }),
    )
        .into_response())
}

/// The request body is the shared contract struct from `api::admin`, so the
/// handler and the generated OpenAPI schema cannot drift (AC-1).
pub(super) type PutWebhookBody = crate::api::admin::WebhookConfigPut;

pub(super) async fn put_webhook_config(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(client_id): Path<Uuid>,
    axum::Json(body): axum::Json<PutWebhookBody>,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &method, &headers, peer, &PERAN, "putWebhookConfig").await?;
    super::gerbang::periksa_guard_mfa(&peminta)?;
    pastikan_milik(&peminta, client_id)?;
    ada_client(&s, client_id).await?;
    periksa_https(&body.webhook_url)?;
    periksa_allowlist(&s, &body.webhook_url).await?;

    let mut tx = s.pool.begin().await.map_err(|e| galat_db(&e))?;
    let secret_versi: Option<i64> = sqlx::query_scalar(
        "INSERT INTO client_webhook (client_id, webhook_url, aktif, diubah_oleh) \
         VALUES ($1, $2, $3, $4) \
         ON CONFLICT (client_id) DO UPDATE \
         SET webhook_url = EXCLUDED.webhook_url, \
             aktif = EXCLUDED.aktif, \
             diubah_oleh = EXCLUDED.diubah_oleh, \
             diubah_at = now() \
         RETURNING secret_versi::bigint",
    )
    .bind(client_id)
    .bind(&body.webhook_url)
    .bind(body.aktif)
    .bind(peminta.pengguna_id)
    .fetch_one(&mut *tx)
    .await
    .map_err(|e| galat_db(&e))?;

    catat(
        &mut tx,
        &peminta,
        AksiKonfigurasi::Ubah,
        client_id,
        None,
        Some(json!({ "webhook_url": body.webhook_url, "aktif": body.aktif })),
        format!("putWebhookConfig {client_id}"),
    )
    .await?;
    tx.commit().await.map_err(|e| galat_db(&e))?;

    Ok((
        StatusCode::OK,
        axum::Json(WebhookConfig {
            webhook_url: Some(body.webhook_url),
            aktif: body.aktif,
            secret_versi,
        }),
    )
        .into_response())
}

pub(super) async fn rotate_webhook_secret(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(client_id): Path<Uuid>,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &method, &headers, peer, &PERAN, "rotateWebhookSecret").await?;
    super::gerbang::periksa_guard_mfa(&peminta)?;
    pastikan_milik(&peminta, client_id)?;

    let mut tx = s.pool.begin().await.map_err(|e| galat_db(&e))?;
    let lama: Option<Option<i64>> = sqlx::query_scalar(
        "SELECT secret_versi::bigint FROM client_webhook WHERE client_id = $1 FOR UPDATE",
    )
    .bind(client_id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| galat_db(&e))?;
    let Some(lama) = lama else {
        return Err(Galat::tidak_ditemukan());
    };

    let versi = lama.unwrap_or(0) + 1;
    let secret = buat_secret_webhook().map_err(|_| Galat::layanan())?;
    let nama = format!("client_webhook {client_id} v{versi}");
    let amplop = s
        .brankas
        .enkripsi(&nama, &NilaiAman::baru(secret.clone()))
        .map_err(|_| Galat::layanan())?;

    sqlx::query(
        "UPDATE client_webhook \
         SET secret_versi = $2, secret_ciphertext = $3, secret_dek_terbungkus = $4, \
             secret_nonce = $5, kek_id = $6, diubah_oleh = $7, diubah_at = now() \
         WHERE client_id = $1",
    )
    .bind(client_id)
    .bind(versi)
    .bind(&amplop.ciphertext)
    .bind(&amplop.dek_terbungkus)
    .bind(amplop.nonce.as_slice())
    .bind(&amplop.kek_id)
    .bind(peminta.pengguna_id)
    .execute(&mut *tx)
    .await
    .map_err(|e| galat_db(&e))?;

    catat(
        &mut tx,
        &peminta,
        AksiKonfigurasi::Ubah,
        client_id,
        lama.map(|v| json!({ "secret_versi": v })),
        Some(json!({ "secret_versi": versi })),
        format!("rotateWebhookSecret {client_id} v{versi}"),
    )
    .await?;
    tx.commit().await.map_err(|e| galat_db(&e))?;

    Ok((
        StatusCode::OK,
        axum::Json(WebhookSecret {
            secret,
            secret_versi: versi,
        }),
    )
        .into_response())
}

#[derive(Deserialize)]
pub(super) struct ListLogQuery {
    dari: Option<DateTime<Utc>>,
    sampai: Option<DateTime<Utc>>,
    hasil: Option<String>,
    kursor: Option<String>,
}

pub(super) async fn list_webhook_log(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(client_id): Path<Uuid>,
    Query(params): Query<ListLogQuery>,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &method, &headers, peer, &PERAN, "listWebhookLog").await?;
    super::gerbang::periksa_guard_mfa(&peminta)?;
    pastikan_milik(&peminta, client_id)?;
    ada_client(&s, client_id).await?;

    let (terkirim, gagal) = match params.hasil.as_deref() {
        Some("terkirim") => (true, false),
        Some("gagal") => (false, true),
        Some(other) => {
            return Err(galat_validasi(vec![FieldError {
                field: "hasil",
                kode: "format",
                batas: Some(other.len() as f64),
            }]));
        }
        None => (true, true),
    };
    let (dari, sampai) = (params.dari, params.sampai);
    let kursor = params
        .kursor
        .as_deref()
        .map(urai_kursor)
        .transpose()?
        .unwrap_or_default();

    let rows: Vec<BarisLog> = sqlx::query_as(
        "WITH sumber AS ( \
             SELECT w.* FROM fn_webhook_log_milik_client($1) w \
             UNION ALL \
             SELECT w.* FROM webhook_log w WHERE $2::boolean AND w.client_id = $3 \
         ) \
         SELECT request_id, sent_at, hasil, http_status, error_class, secret_versi::bigint \
           FROM sumber \
          WHERE client_id = $3 \
            AND ($4::timestamptz IS NULL OR sent_at >= $4) \
            AND ($5::timestamptz IS NULL OR sent_at <= $5) \
            AND ($6::boolean = false OR hasil = 'terkirim') \
            AND ($7::boolean = false OR hasil <> 'terkirim') \
            AND ($8::timestamptz IS NULL OR (sent_at, request_id) < ($8, $9)) \
          ORDER BY sent_at DESC, request_id DESC LIMIT $10",
    )
    .bind(peminta.pengguna_id)
    .bind(peminta.peran == Peran::PlatformAdmin)
    .bind(client_id)
    .bind(dari)
    .bind(sampai)
    .bind(terkirim)
    .bind(gagal)
    .bind(kursor.0)
    .bind(kursor.1)
    .bind(LOG_MAKS)
    .fetch_all(&s.pool)
    .await
    .map_err(|e| galat_db(&e))?;

    let daftar: Vec<WebhookLog> = rows
        .into_iter()
        .map(
            |(request_id, sent_at, hasil, http_status, error_class, secret_versi)| WebhookLog {
                request_id,
                sent_at,
                hasil: hasil_dari_db(&hasil),
                http_status: http_status.map(i64::from),
                error_class: error_class.as_deref().map(kelas_dari_db),
                secret_versi,
            },
        )
        .collect();

    Ok((StatusCode::OK, axum::Json(daftar)).into_response())
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

fn periksa_https(webhook_url: &str) -> Result<(), Galat> {
    let parsed = webhook_url.parse::<axum::http::Uri>().ok();
    if parsed.as_ref().and_then(|u| u.scheme_str()) != Some("https") {
        return Err(Galat::baru(
            StatusCode::UNPROCESSABLE_ENTITY,
            ProblemKodeAdmin::WebhookTidakHttps,
        ));
    }
    Ok(())
}

async fn periksa_allowlist(s: &AdminState, webhook_url: &str) -> Result<(), Galat> {
    let host = webhook_url
        .parse::<axum::http::Uri>()
        .ok()
        .and_then(|u| u.host().map(str::to_lowercase))
        .ok_or_else(|| {
            Galat::baru(
                StatusCode::UNPROCESSABLE_ENTITY,
                ProblemKodeAdmin::WebhookTidakHttps,
            )
        })?;
    let jumlah: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM webhook_domain_allowlist WHERE domain = $1 AND aktif",
    )
    .bind(&host)
    .fetch_one(&s.pool)
    .await
    .map_err(|e| galat_db(&e))?;
    if jumlah == 0 {
        return Err(Galat::baru(
            StatusCode::UNPROCESSABLE_ENTITY,
            ProblemKodeAdmin::WebhookDomainTidakDiizinkan,
        ));
    }
    Ok(())
}

/// `kursor` = `<RFC 3339 sent_at>|<request_id>`; the unique (request_id,
/// attempt_no) pair keeps the ordering total.
fn urai_kursor(raw: &str) -> Result<(Option<DateTime<Utc>>, Option<Uuid>), Galat> {
    let (waktu, id) = raw.split_once('|').ok_or_else(|| {
        galat_validasi(vec![FieldError {
            field: "kursor",
            kode: "format",
            batas: None,
        }])
    })?;
    let waktu = DateTime::parse_from_rfc3339(waktu)
        .map_err(|_| {
            galat_validasi(vec![FieldError {
                field: "kursor",
                kode: "format",
                batas: None,
            }])
        })?
        .with_timezone(&Utc);
    let id = Uuid::parse_str(id).map_err(|_| {
        galat_validasi(vec![FieldError {
            field: "kursor",
            kode: "format",
            batas: None,
        }])
    })?;
    Ok((Some(waktu), Some(id)))
}

fn hasil_dari_db(hasil: &str) -> HasilWebhook {
    match hasil {
        "terkirim" => HasilWebhook::Terkirim,
        "gagal_http" => HasilWebhook::GagalHttp,
        "gagal_jaringan" => HasilWebhook::GagalJaringan,
        "ditolak_allowlist" => HasilWebhook::DitolakAllowlist,
        _ => HasilWebhook::GagalInternal,
    }
}

fn kelas_dari_db(kelas: &str) -> KelasErrorWebhook {
    match kelas {
        "timeout" => KelasErrorWebhook::Timeout,
        "koneksi" => KelasErrorWebhook::Koneksi,
        "rate_limit_provider" => KelasErrorWebhook::RateLimitProvider,
        "respons_tidak_valid" => KelasErrorWebhook::ResponsTidakValid,
        "tls" => KelasErrorWebhook::Tls,
        _ => KelasErrorWebhook::ProviderSementara,
    }
}

#[allow(clippy::too_many_arguments)]
async fn catat(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    peminta: &super::gerbang::Peminta,
    aksi: AksiKonfigurasi,
    id: Uuid,
    nilai_lama: Option<serde_json::Value>,
    nilai_baru: Option<serde_json::Value>,
    alasan: String,
) -> Result<(), Galat> {
    let alasan = Alasan::baru(alasan).map_err(|_| {
        galat_validasi(vec![FieldError {
            field: "alasan",
            kode: "rentang",
            batas: Some(500.0),
        }])
    })?;
    Penulis::baru(tx)
        .catat_konfigurasi(&KonfigurasiBaru {
            pelaku: peminta.pengguna_id,
            entitas: EntitasKonfigurasi::ClientWebhook,
            entitas_id: Some(id),
            aksi,
            nilai_lama,
            nilai_baru,
            alasan,
        })
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, "admin: audit konfigurasi webhook gagal");
            Galat::layanan()
        })?;
    Ok(())
}

fn galat_db(e: &sqlx::Error) -> Galat {
    tracing::error!(error = %e, "admin: kueri webhook gagal");
    Galat::layanan()
}
