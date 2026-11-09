//! Client CRUD: `listClients`, `getClient`, `createClient`, `updateClient`
//! (f0-admin-api AC-19; DESIGN.md §3.2).
//!
//! Reads are open to `client_owner` for its own client only; writes are
//! platform-admin only. L-04 limiter ranges are enforced here as 422, and a
//! limiter change needs a reason that lands in `audit_konfigurasi` with the
//! effect, in one transaction.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use poolrouter_core::audit::{
    AksiKonfigurasi, Alasan, EntitasKonfigurasi, KonfigurasiBaru, Penulis, Peran,
};
use serde_json::json;
use uuid::Uuid;

use super::gerbang::{galat_validasi, masuk};
use super::k20::{self, GalatK20, JenisEntitas};
use super::sesi::{AdminState, Galat};
use crate::api::admin::{AlasanK20, Client, ClientConfig, FieldError, HasilK20, PratinjauK20};

/// Reads: platform admin, or the client owner of the addressed client.
const PERAN_BACA: [Peran; 2] = [Peran::PlatformAdmin, Peran::ClientOwner];
/// Writes: platform admin only (DESIGN.md §3.2).
const PERAN_TULIS: [Peran; 1] = [Peran::PlatformAdmin];
/// K-20 on a client: platform admin only (`fn_nonaktifkan_entitas`/`fn_aktifkan_entitas`
/// have no client_owner branch for `jenis = 'client'`).
const PERAN_K20: [Peran; 1] = [Peran::PlatformAdmin];
/// `activateClient` takes no body (section 3a.4: no confirmation dialog), but
/// the SQL function still requires a non-empty reason; this fixed, non-secret
/// string satisfies it.
const ALASAN_AKTIFKAN_CLIENT: &str =
    "Diaktifkan lewat admin console (K-20); tanpa dialog konfirmasi.";

const BURST_MIN: i64 = 1;
const BURST_MAX: i64 = 1000;
const ISI_ULANG_MIN: f64 = 0.10;
const ISI_ULANG_MAX: f64 = 100.00;
const NAMA_MAX: usize = 200;
const ALASAN_MIN: usize = 10;
const ALASAN_MAX: usize = 500;

pub(super) async fn list_clients(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &method, &headers, peer, &PERAN_BACA, "listClients").await?;
    super::gerbang::periksa_guard_mfa(&peminta)?;

    let rows: Vec<(Uuid, String, i64, f64, bool)> = match peminta.peran {
        Peran::ClientOwner => sqlx::query_as(
            "SELECT client_id, nama, limiter_penolakan_burst::bigint, \
                        limiter_penolakan_isi_ulang_per_detik::float8, aktif \
                   FROM client WHERE client_id = $1 ORDER BY nama",
        )
        .bind(peminta.client_id)
        .fetch_all(&s.pool)
        .await
        .map_err(|e| galat_db(&e))?,
        _ => sqlx::query_as(
            "SELECT client_id, nama, limiter_penolakan_burst::bigint, \
                        limiter_penolakan_isi_ulang_per_detik::float8, aktif \
                   FROM client ORDER BY nama",
        )
        .fetch_all(&s.pool)
        .await
        .map_err(|e| galat_db(&e))?,
    };

    let daftar: Vec<Client> = rows.into_iter().map(baris_client).collect();
    Ok((StatusCode::OK, axum::Json(daftar)).into_response())
}

pub(super) async fn get_client(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<Uuid>,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &method, &headers, peer, &PERAN_BACA, "getClient").await?;
    super::gerbang::periksa_guard_mfa(&peminta)?;
    if peminta.peran == Peran::ClientOwner && peminta.client_id != Some(id) {
        return Err(Galat::tidak_ditemukan());
    }

    let baris: Option<(Uuid, String, i64, f64, bool)> = sqlx::query_as(
        "SELECT client_id, nama, limiter_penolakan_burst::bigint, \
                limiter_penolakan_isi_ulang_per_detik::float8, aktif \
           FROM client WHERE client_id = $1",
    )
    .bind(id)
    .fetch_optional(&s.pool)
    .await
    .map_err(|e| galat_db(&e))?;

    let Some(baris) = baris else {
        return Err(Galat::tidak_ditemukan());
    };
    Ok((StatusCode::OK, axum::Json(baris_client(baris))).into_response())
}

pub(super) async fn create_client(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    axum::Json(body): axum::Json<ClientConfig>,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &method, &headers, peer, &PERAN_TULIS, "createClient").await?;
    super::gerbang::periksa_guard_mfa(&peminta)?;
    let burst = body.limiter_penolakan_burst.unwrap_or(20);
    let isi_ulang = body.limiter_penolakan_isi_ulang_per_detik.unwrap_or(1.0);
    periksa_nama(&body.nama)?;
    periksa_limiter(burst, isi_ulang)?;

    let id = Uuid::now_v7();
    let mut tx = s.pool.begin().await.map_err(|e| galat_db(&e))?;
    sqlx::query(
        "INSERT INTO client \
         (client_id, nama, limiter_penolakan_burst, limiter_penolakan_isi_ulang_per_detik, aktif) \
         VALUES ($1, $2, $3, $4, true)",
    )
    .bind(id)
    .bind(&body.nama)
    .bind(burst)
    .bind(isi_ulang)
    .execute(&mut *tx)
    .await
    .map_err(|e| galat_tulis(&e, "nama"))?;

    let baru = json!({
        "nama": &body.nama,
        "limiter_penolakan_burst": burst,
        "limiter_penolakan_isi_ulang_per_detik": isi_ulang,
        "aktif": true,
    });
    catat(
        &mut tx,
        &peminta,
        AksiKonfigurasi::Buat,
        id,
        None,
        Some(baru),
        format!("createClient {}", body.nama),
    )
    .await?;
    tx.commit().await.map_err(|e| galat_db(&e))?;

    Ok((
        StatusCode::CREATED,
        axum::Json(Client {
            client_id: id,
            nama: body.nama,
            limiter_penolakan_burst: burst,
            limiter_penolakan_isi_ulang_per_detik: isi_ulang,
            aktif: true,
        }),
    )
        .into_response())
}

pub(super) async fn update_client(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<Uuid>,
    axum::Json(body): axum::Json<ClientConfig>,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &method, &headers, peer, &PERAN_TULIS, "updateClient").await?;
    super::gerbang::periksa_guard_mfa(&peminta)?;
    periksa_nama(&body.nama)?;

    let lama: Option<(String, i64, f64, bool)> = sqlx::query_as(
        "SELECT nama, limiter_penolakan_burst::bigint, limiter_penolakan_isi_ulang_per_detik::float8, aktif \
           FROM client WHERE client_id = $1",
    )
    .bind(id)
    .fetch_optional(&s.pool)
    .await
    .map_err(|e| galat_db(&e))?;
    let Some((nama_lama, burst_lama, isi_ulang_lama, aktif)) = lama else {
        return Err(Galat::tidak_ditemukan());
    };

    let burst = body.limiter_penolakan_burst.unwrap_or(burst_lama);
    let isi_ulang = body
        .limiter_penolakan_isi_ulang_per_detik
        .unwrap_or(isi_ulang_lama);
    periksa_limiter(burst, isi_ulang)?;

    let limiter_berubah = burst != burst_lama || isi_ulang != isi_ulang_lama;
    let alasan = if limiter_berubah {
        Some(periksa_alasan(body.alasan.as_deref())?)
    } else {
        body.alasan
            .as_deref()
            .map(|a| periksa_alasan(Some(a)))
            .transpose()?
    };

    let mut tx = s.pool.begin().await.map_err(|e| galat_db(&e))?;
    sqlx::query(
        "UPDATE client SET nama = $2, limiter_penolakan_burst = $3, \
                limiter_penolakan_isi_ulang_per_detik = $4 \
           WHERE client_id = $1",
    )
    .bind(id)
    .bind(&body.nama)
    .bind(burst)
    .bind(isi_ulang)
    .execute(&mut *tx)
    .await
    .map_err(|e| galat_tulis(&e, "nama"))?;

    let lama = json!({
        "nama": nama_lama,
        "limiter_penolakan_burst": burst_lama,
        "limiter_penolakan_isi_ulang_per_detik": isi_ulang_lama,
    });
    let baru = json!({
        "nama": body.nama,
        "limiter_penolakan_burst": burst,
        "limiter_penolakan_isi_ulang_per_detik": isi_ulang,
    });
    catat(
        &mut tx,
        &peminta,
        AksiKonfigurasi::Ubah,
        id,
        Some(lama),
        Some(baru),
        alasan.unwrap_or_else(|| format!("updateClient {}", body.nama)),
    )
    .await?;
    tx.commit().await.map_err(|e| galat_db(&e))?;

    Ok((
        StatusCode::OK,
        axum::Json(Client {
            client_id: id,
            nama: body.nama,
            limiter_penolakan_burst: burst,
            limiter_penolakan_isi_ulang_per_detik: isi_ulang,
            aktif,
        }),
    )
        .into_response())
}

/// Preview client deactivation (K-20, AC-28): counts only, no request id, NIK
/// or payload, nothing stored.
pub(super) async fn pratinjau_nonaktifkan_client(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<Uuid>,
) -> Result<Response, Galat> {
    masuk(
        &s,
        &method,
        &headers,
        peer,
        &PERAN_K20,
        "pratinjauNonaktifkanClient",
    )
    .await?;
    ada_client(&s, id).await?;
    let hasil = s
        .k20
        .pratinjau(JenisEntitas::Client, id)
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

/// Deactivate a client (K-20, AC-29): one mass cancel/fail transaction; the
/// same numbers land in `audit_konfigurasi`.
pub(super) async fn nonaktifkan_client(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<Uuid>,
    axum::Json(body): axum::Json<AlasanK20>,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &method, &headers, peer, &PERAN_K20, "nonaktifkanClient").await?;
    ada_client(&s, id).await?;
    let hasil = s
        .k20
        .nonaktifkan(JenisEntitas::Client, id, peminta.pengguna_id, &body.alasan)
        .await
        .map_err(|e| galat_k20(&e))?;
    Ok((
        StatusCode::OK,
        axum::Json(HasilK20 {
            jumlah_cancelled: i64::from(hasil.jumlah_cancelled),
            jumlah_failed: i64::from(hasil.jumlah_failed),
            dilakukan_pada: Utc::now(),
        }),
    )
        .into_response())
}

/// Activate a client (K-20, AC-30): no K-20 dialog, applies only to requests
/// submitted afterwards.
pub(super) async fn aktifkan_client(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<Uuid>,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &method, &headers, peer, &PERAN_K20, "aktifkanClient").await?;
    let lama = ada_client(&s, id).await?;

    s.k20
        .aktifkan(
            JenisEntitas::Client,
            id,
            peminta.pengguna_id,
            ALASAN_AKTIFKAN_CLIENT,
        )
        .await
        .map_err(|e| galat_k20(&e))?;

    Ok((
        StatusCode::OK,
        axum::Json(Client {
            client_id: id,
            aktif: true,
            ..lama
        }),
    )
        .into_response())
}

/// `client_id`, or 404; the row is also the entity `aktifkan_client` returns.
async fn ada_client(s: &AdminState, client_id: Uuid) -> Result<Client, Galat> {
    let baris: Option<(Uuid, String, i64, f64, bool)> = sqlx::query_as(
        "SELECT client_id, nama, limiter_penolakan_burst::bigint, \
                limiter_penolakan_isi_ulang_per_detik::float8, aktif \
           FROM client WHERE client_id = $1",
    )
    .bind(client_id)
    .fetch_optional(&s.pool)
    .await
    .map_err(|e| galat_db(&e))?;
    baris.map(baris_client).ok_or_else(Galat::tidak_ditemukan)
}

/// `fn_nonaktifkan_entitas`/`fn_aktifkan_entitas` only ever raise
/// `check_violation` (23514, the alasan-empty guard — unreachable here, the
/// body is already validated) or `insufficient_privilege` (42501 — also
/// unreachable, `masuk` already checked the role); anything else is a
/// database failure.
fn galat_k20(e: &GalatK20) -> Galat {
    match e {
        GalatK20::AlasanTidakValid => galat_validasi(vec![FieldError {
            field: "alasan",
            kode: "rentang",
            batas: Some(k20::ALASAN_MAX as f64),
        }]),
        GalatK20::BasisData(e) => {
            tracing::error!(error = %e, "admin: operasi K-20 client gagal");
            Galat::layanan()
        }
    }
}

fn baris_client(baris: (Uuid, String, i64, f64, bool)) -> Client {
    let (client_id, nama, limiter_penolakan_burst, isi_ulang, aktif) = baris;
    Client {
        client_id,
        nama,
        limiter_penolakan_burst,
        limiter_penolakan_isi_ulang_per_detik: isi_ulang,
        aktif,
    }
}

fn periksa_nama(nama: &str) -> Result<(), Galat> {
    let panjang = nama.chars().count();
    if panjang == 0 || panjang > NAMA_MAX {
        return Err(galat_validasi(vec![FieldError {
            field: "nama",
            kode: "rentang",
            batas: Some(NAMA_MAX as f64),
        }]));
    }
    Ok(())
}

fn periksa_limiter(burst: i64, isi_ulang: f64) -> Result<(), Galat> {
    let mut errors = Vec::new();
    if !(BURST_MIN..=BURST_MAX).contains(&burst) {
        errors.push(FieldError {
            field: "limiter_penolakan_burst",
            kode: "rentang",
            batas: Some(BURST_MAX as f64),
        });
    }
    if !(ISI_ULANG_MIN..=ISI_ULANG_MAX).contains(&isi_ulang) {
        errors.push(FieldError {
            field: "limiter_penolakan_isi_ulang_per_detik",
            kode: "rentang",
            batas: Some(ISI_ULANG_MAX),
        });
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(galat_validasi(errors))
    }
}

fn periksa_alasan(alasan: Option<&str>) -> Result<String, Galat> {
    let Some(alasan) = alasan else {
        return Err(galat_validasi(vec![FieldError {
            field: "alasan",
            kode: "wajib",
            batas: None,
        }]));
    };
    let panjang = alasan.chars().count();
    if !(ALASAN_MIN..=ALASAN_MAX).contains(&panjang) {
        return Err(galat_validasi(vec![FieldError {
            field: "alasan",
            kode: "rentang",
            batas: Some(ALASAN_MAX as f64),
        }]));
    }
    Ok(alasan.to_owned())
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
            batas: Some(ALASAN_MAX as f64),
        }])
    })?;
    Penulis::baru(tx)
        .catat_konfigurasi(&KonfigurasiBaru {
            pelaku: peminta.pengguna_id,
            entitas: EntitasKonfigurasi::Client,
            entitas_id: Some(id),
            aksi,
            nilai_lama,
            nilai_baru,
            alasan,
        })
        .await
        .map_err(|e| {
            tracing::error!(error = ?e, "admin: audit konfigurasi client gagal");
            Galat::layanan()
        })?;
    Ok(())
}

/// A duplicate `nama` is the only unique constraint here; a 500 would hide a
/// fixable request error, so it becomes a 422 on the field that caused it.
fn galat_tulis(e: &sqlx::Error, field: &'static str) -> Galat {
    if e.as_database_error()
        .and_then(|d| d.code())
        .is_some_and(|c| c == "23505")
    {
        return galat_validasi(vec![FieldError {
            field,
            kode: "duplikat",
            batas: None,
        }]);
    }
    tracing::error!(error = %e, "admin: tulis client gagal");
    Galat::layanan()
}

fn galat_db(e: &sqlx::Error) -> Galat {
    tracing::error!(error = %e, "admin: kueri client gagal");
    Galat::layanan()
}
