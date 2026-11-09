//! K-20 untuk endpoint, downstream pool, upstream pool, dan provider:
//! pratinjau, nonaktifkan, aktifkan (f0-admin-api AC-28..AC-31, ARC-168).
//!
//! Semua operasi hanya untuk platform admin (DESIGN.md §3.3). Hitungan dan
//! transisi request dikerjakan fungsi basis data lewat `k20::K20Service`;
//! modul ini hanya memasang HTTP: guard, 404 untuk id tak dikenal, 422 untuk
//! alasan, dan bentuk respons yang dijanjikan kontrak. Client dan kredensial
//! ada di `klien` dan `kredensial`.

use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::Utc;
use poolrouter_core::audit::Peran;
use uuid::Uuid;

use super::gerbang::{galat_validasi, masuk, periksa_guard_mfa};
use super::k20::{self, GalatK20, JenisEntitas};
use super::sesi::{AdminState, Galat};
use crate::api::admin::{AlasanK20, FieldError, HasilK20, PratinjauK20, UpstreamAktifTerakhir};

const PERAN_PA: [Peran; 1] = [Peran::PlatformAdmin];

/// `activate*` tanpa body (3a.4: tanpa dialog), tetapi fungsi SQL tetap
/// mewajibkan alasan; string tetap ini bukan secret dan memenuhinya.
const ALASAN_AKTIFKAN: &str = "Diaktifkan lewat admin console (K-20); tanpa dialog konfirmasi.";

/// Jenis entitas beserta kueri keberadaannya (id tak dikenal menjadi 404) dan
/// nama operasi untuk baris `audit_akses` saat peran ditolak.
struct Jenis {
    jenis: JenisEntitas,
    ada: &'static str,
    op_pratinjau: &'static str,
    op_nonaktifkan: &'static str,
    op_aktifkan: &'static str,
}

const ENDPOINT: Jenis = Jenis {
    jenis: JenisEntitas::DownstreamEndpoint,
    ada: "SELECT EXISTS (SELECT 1 FROM downstream_endpoint WHERE downstream_endpoint_id = $1)",
    op_pratinjau: "pratinjauNonaktifkanEndpoint",
    op_nonaktifkan: "nonaktifkanEndpoint",
    op_aktifkan: "aktifkanEndpoint",
};
const DOWNSTREAM: Jenis = Jenis {
    jenis: JenisEntitas::DownstreamPool,
    ada: "SELECT EXISTS (SELECT 1 FROM downstream_pool WHERE downstream_pool_id = $1)",
    op_pratinjau: "pratinjauNonaktifkanDownstreamPool",
    op_nonaktifkan: "nonaktifkanDownstreamPool",
    op_aktifkan: "aktifkanDownstreamPool",
};
const UPSTREAM: Jenis = Jenis {
    jenis: JenisEntitas::UpstreamPool,
    ada: "SELECT EXISTS (SELECT 1 FROM upstream_pool WHERE upstream_pool_id = $1)",
    op_pratinjau: "pratinjauNonaktifkanUpstreamPool",
    op_nonaktifkan: "nonaktifkanUpstreamPool",
    op_aktifkan: "aktifkanUpstreamPool",
};
const PROVIDER: Jenis = Jenis {
    jenis: JenisEntitas::Provider,
    ada: "SELECT EXISTS (SELECT 1 FROM provider WHERE provider_id = $1)",
    op_pratinjau: "pratinjauNonaktifkanProvider",
    op_nonaktifkan: "nonaktifkanProvider",
    op_aktifkan: "aktifkanProvider",
};

/// Pratinjau (AC-28): hanya angka, tidak ada request_id, NIK, atau payload.
async fn pratinjau(
    s: &AdminState,
    method: &Method,
    headers: &HeaderMap,
    peer: SocketAddr,
    j: &Jenis,
    id: Uuid,
) -> Result<PratinjauK20, Galat> {
    let peminta = masuk(s, method, headers, peer, &PERAN_PA, j.op_pratinjau).await?;
    periksa_guard_mfa(&peminta)?;
    ada(s, j, id).await?;
    let hasil = s
        .k20
        .pratinjau(j.jenis, id)
        .await
        .map_err(|e| galat_k20(&e))?;
    Ok(PratinjauK20 {
        akan_cancelled: i64::from(hasil.akan_cancelled),
        akan_failed: i64::from(hasil.akan_failed),
        dihitung_pada: hasil.dihitung_at,
        upstream_aktif_terakhir_untuk: None,
        dipakai_oleh_downstream_pool_count: None,
    })
}

/// Nonaktifkan (AC-29): satu transaksi di SQL; angka yang sama masuk
/// `audit_konfigurasi`.
async fn nonaktifkan(
    s: &AdminState,
    method: &Method,
    headers: &HeaderMap,
    peer: SocketAddr,
    j: &Jenis,
    id: Uuid,
    body: &AlasanK20,
) -> Result<Response, Galat> {
    let peminta = masuk(s, method, headers, peer, &PERAN_PA, j.op_nonaktifkan).await?;
    periksa_guard_mfa(&peminta)?;
    ada(s, j, id).await?;
    let hasil = s
        .k20
        .nonaktifkan(j.jenis, id, peminta.pengguna_id, &body.alasan)
        .await
        .map_err(|e| galat_k20(&e))?;
    Ok((
        StatusCode::OK,
        Json(HasilK20 {
            jumlah_cancelled: i64::from(hasil.jumlah_cancelled),
            jumlah_failed: i64::from(hasil.jumlah_failed),
            dilakukan_pada: Utc::now(),
        }),
    )
        .into_response())
}

/// Aktifkan (AC-30): hanya berlaku untuk request sesudahnya; request lama tidak
/// disentuh. Pemanggil membaca ulang entitasnya untuk respons.
async fn aktifkan(
    s: &AdminState,
    method: &Method,
    headers: &HeaderMap,
    peer: SocketAddr,
    j: &Jenis,
    id: Uuid,
) -> Result<(), Galat> {
    let peminta = masuk(s, method, headers, peer, &PERAN_PA, j.op_aktifkan).await?;
    periksa_guard_mfa(&peminta)?;
    ada(s, j, id).await?;
    s.k20
        .aktifkan(j.jenis, id, peminta.pengguna_id, ALASAN_AKTIFKAN)
        .await
        .map_err(|e| galat_k20(&e))
}

async fn ada(s: &AdminState, j: &Jenis, id: Uuid) -> Result<(), Galat> {
    let ada: bool = sqlx::query_scalar(j.ada)
        .bind(id)
        .fetch_one(&s.pool)
        .await
        .map_err(|e| galat_db(&e))?;
    if ada {
        Ok(())
    } else {
        Err(Galat::tidak_ditemukan())
    }
}

/// Fungsi SQL K-20 hanya melempar `check_violation` (alasan kosong, sudah
/// dicegah di service) atau `insufficient_privilege` (sudah dicegah `masuk`);
/// selain alasan, kegagalan adalah kegagalan basis data.
fn galat_k20(e: &GalatK20) -> Galat {
    match e {
        GalatK20::AlasanTidakValid => galat_validasi(vec![FieldError {
            field: "alasan",
            kode: "rentang",
            batas: Some(k20::ALASAN_MAX as f64),
        }]),
        GalatK20::BasisData(e) => galat_db(e),
    }
}

fn galat_db(e: &sqlx::Error) -> Galat {
    tracing::error!(error = %e, "admin: operasi K-20 entitas gagal");
    Galat::layanan()
}

/// Downstream pool yang upstream aktif terakhirnya adalah pool ini, dihitung
/// dari konfigurasi (ADR-0001 §3a.10): pool ini aktif, downstream pool aktif,
/// dan tidak ada upstream pool aktif lain di rutenya.
async fn upstream_aktif_terakhir(
    s: &AdminState,
    id: Uuid,
) -> Result<Vec<UpstreamAktifTerakhir>, Galat> {
    let baris: Vec<(Uuid, String)> = sqlx::query_as(
        "SELECT d.downstream_pool_id, d.nama \
           FROM downstream_pool d \
           JOIN downstream_pool_rute r ON r.downstream_pool_id = d.downstream_pool_id \
           JOIN upstream_pool u ON u.upstream_pool_id = r.upstream_pool_id \
          WHERE r.upstream_pool_id = $1 AND u.aktif AND d.aktif \
            AND NOT EXISTS ( \
                SELECT 1 FROM downstream_pool_rute r2 \
                  JOIN upstream_pool u2 ON u2.upstream_pool_id = r2.upstream_pool_id \
                 WHERE r2.downstream_pool_id = d.downstream_pool_id \
                   AND r2.upstream_pool_id <> $1 AND u2.aktif) \
          ORDER BY d.nama, d.downstream_pool_id",
    )
    .bind(id)
    .fetch_all(&s.pool)
    .await
    .map_err(|e| galat_db(&e))?;
    Ok(baris
        .into_iter()
        .map(|(downstream_pool_id, nama)| UpstreamAktifTerakhir {
            downstream_pool_id,
            nama,
        })
        .collect())
}

async fn jumlah_downstream_pool(s: &AdminState, id: Uuid) -> Result<i64, Galat> {
    sqlx::query_scalar("SELECT count(*) FROM downstream_pool_rute WHERE upstream_pool_id = $1")
        .bind(id)
        .fetch_one(&s.pool)
        .await
        .map_err(|e| galat_db(&e))
}

// ---------------------------------------------------------------------------
// Endpoint
// ---------------------------------------------------------------------------

pub(super) async fn pratinjau_nonaktifkan_endpoint(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<Uuid>,
) -> Result<Response, Galat> {
    let p = pratinjau(&s, &method, &headers, peer, &ENDPOINT, id).await?;
    Ok((StatusCode::OK, Json(p)).into_response())
}

pub(super) async fn nonaktifkan_endpoint(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<Uuid>,
    Json(body): Json<AlasanK20>,
) -> Result<Response, Galat> {
    nonaktifkan(&s, &method, &headers, peer, &ENDPOINT, id, &body).await
}

pub(super) async fn aktifkan_endpoint(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<Uuid>,
) -> Result<Response, Galat> {
    aktifkan(&s, &method, &headers, peer, &ENDPOINT, id).await?;
    let mut sambungan = s.pool.acquire().await.map_err(|e| galat_db(&e))?;
    let entitas = super::endpoint::baca_endpoint(&mut sambungan, id)
        .await?
        .ok_or_else(Galat::tidak_ditemukan)?;
    Ok((StatusCode::OK, Json(entitas)).into_response())
}

// ---------------------------------------------------------------------------
// Downstream pool
// ---------------------------------------------------------------------------

pub(super) async fn pratinjau_nonaktifkan_downstream_pool(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<Uuid>,
) -> Result<Response, Galat> {
    let p = pratinjau(&s, &method, &headers, peer, &DOWNSTREAM, id).await?;
    Ok((StatusCode::OK, Json(p)).into_response())
}

pub(super) async fn nonaktifkan_downstream_pool(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<Uuid>,
    Json(body): Json<AlasanK20>,
) -> Result<Response, Galat> {
    nonaktifkan(&s, &method, &headers, peer, &DOWNSTREAM, id, &body).await
}

pub(super) async fn aktifkan_downstream_pool(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<Uuid>,
) -> Result<Response, Galat> {
    aktifkan(&s, &method, &headers, peer, &DOWNSTREAM, id).await?;
    let entitas = super::pool::baca_downstream(&s.pool, id)
        .await?
        .ok_or_else(Galat::tidak_ditemukan)?;
    Ok((StatusCode::OK, Json(entitas)).into_response())
}

// ---------------------------------------------------------------------------
// Upstream pool
// ---------------------------------------------------------------------------

/// Pratinjau upstream pool menambah dua keterangan dari konfigurasi; dua angka
/// utamanya tidak berubah (§3a.10).
pub(super) async fn pratinjau_nonaktifkan_upstream_pool(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<Uuid>,
) -> Result<Response, Galat> {
    let mut p = pratinjau(&s, &method, &headers, peer, &UPSTREAM, id).await?;
    p.upstream_aktif_terakhir_untuk = Some(upstream_aktif_terakhir(&s, id).await?);
    p.dipakai_oleh_downstream_pool_count = Some(jumlah_downstream_pool(&s, id).await?);
    Ok((StatusCode::OK, Json(p)).into_response())
}

pub(super) async fn nonaktifkan_upstream_pool(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<Uuid>,
    Json(body): Json<AlasanK20>,
) -> Result<Response, Galat> {
    nonaktifkan(&s, &method, &headers, peer, &UPSTREAM, id, &body).await
}

pub(super) async fn aktifkan_upstream_pool(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<Uuid>,
) -> Result<Response, Galat> {
    aktifkan(&s, &method, &headers, peer, &UPSTREAM, id).await?;
    let entitas = super::pool::baca_upstream(&s.pool, id)
        .await?
        .ok_or_else(Galat::tidak_ditemukan)?;
    Ok((StatusCode::OK, Json(entitas)).into_response())
}

// ---------------------------------------------------------------------------
// Provider
// ---------------------------------------------------------------------------

pub(super) async fn pratinjau_nonaktifkan_provider(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<Uuid>,
) -> Result<Response, Galat> {
    let p = pratinjau(&s, &method, &headers, peer, &PROVIDER, id).await?;
    Ok((StatusCode::OK, Json(p)).into_response())
}

pub(super) async fn nonaktifkan_provider(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<Uuid>,
    Json(body): Json<AlasanK20>,
) -> Result<Response, Galat> {
    nonaktifkan(&s, &method, &headers, peer, &PROVIDER, id, &body).await
}

pub(super) async fn aktifkan_provider(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(id): Path<Uuid>,
) -> Result<Response, Galat> {
    aktifkan(&s, &method, &headers, peer, &PROVIDER, id).await?;
    let entitas = super::provider::baca_provider(&s, id).await?;
    Ok((StatusCode::OK, Json(entitas)).into_response())
}
