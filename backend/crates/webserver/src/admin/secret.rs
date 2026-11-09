//! The global provider secret store: `listSecrets`, `putSecret`, `deleteSecret`
//! (US-02; AC-8..AC-11).
//!
//! The value is write-only end to end: `putSecret` seals it with
//! `core::secret::Brankas` and the plaintext never leaves the request handling;
//! `listSecrets` answers name, version and a usage count, and nothing reads the
//! ciphertext back over HTTP. Each write bumps `secret_versi`, records `kek_id`,
//! and leaves an `audit_konfigurasi` row in the same transaction (AC-5).
//!
//! `dipakai_oleh_provider_count` and the 409 from `deleteSecret` look at the
//! active configuration of every active provider and count the ones whose
//! `dokumen` mentions `{{secret.<nama>}}`. That is the placeholder
//! `createProvider` validates (DESIGN.md §3), and it is why the store is global
//! per name: a provider's id does not exist yet when its template is checked
//! (AD1).

use std::collections::BTreeSet;
use std::net::SocketAddr;

use axum::Json;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use poolrouter_core::admin::NilaiAman;
use poolrouter_core::audit::{
    AksiKonfigurasi, Alasan, EntitasKonfigurasi, KonfigurasiBaru, Penulis, Peran,
};
use serde_json::json;

use super::gerbang::{galat_validasi, masuk};
use super::sesi::{AdminState, Galat};
use crate::api::admin::{FieldError, SecretNilai, SecretRingkas};
use crate::api::problem::ProblemKodeAdmin;

/// Every operation here is platform-admin only (DESIGN.md §3).
const PERAN_PA: [Peran; 1] = [Peran::PlatformAdmin];

/// Contract name regex, same as the CHECK on `provider_secret.nama_secret` (AC-9):
/// a name must also be a legal `{{secret.<nama>}}` placeholder key.
const NAMA_PANJANG_MAKS: usize = 64;

pub(super) async fn list_secrets(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &method, &headers, peer, &PERAN_PA, "listSecrets").await?;
    super::gerbang::periksa_guard_mfa(&peminta)?;

    let baris: Vec<(String,)> =
        sqlx::query_as("SELECT ps.nama_secret FROM provider_secret ps ORDER BY ps.nama_secret")
            .fetch_all(&s.pool)
            .await
            .map_err(|e| galat_db(&e))?;

    let mut daftar = Vec::with_capacity(baris.len());
    for (nama,) in baris {
        daftar.push(SecretRingkas {
            dipakai_oleh_provider_count: provider_pemakai(&s.pool, &nama)
                .await?
                .len()
                .try_into()
                .unwrap_or(i64::MAX),
            nama,
            // The value is never returned, so every secret is masked in the UI.
            tersamar: true,
        });
    }
    Ok(Json(daftar).into_response())
}

pub(super) async fn put_secret(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(nama): Path<String>,
    body: Result<Json<SecretNilai>, axum::extract::rejection::JsonRejection>,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &method, &headers, peer, &PERAN_PA, "putSecret").await?;
    super::gerbang::periksa_guard_mfa(&peminta)?;
    // A body that is not `{"nilai": "..."}` is a validation failure, not a
    // parse error: the contract lists 422 for this operation, not 400.
    let axum::Json(req) = body.map_err(|_| {
        galat_validasi(vec![FieldError {
            field: "nilai",
            kode: "format",
            batas: None,
        }])
    })?;
    periksa_nama(&nama)?;

    let amplop = s
        .brankas
        .enkripsi(&nama, &NilaiAman::baru(req.nilai))
        .map_err(|e| {
            tracing::error!(error = %e, "admin: gagal menyegel secret");
            Galat::layanan()
        })?;

    let mut tx = s.pool.begin().await.map_err(|e| galat_db(&e))?;
    let lama: Option<i32> = sqlx::query_scalar(
        "SELECT secret_versi FROM provider_secret WHERE nama_secret = $1 FOR UPDATE",
    )
    .bind(&nama)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| galat_db(&e))?;

    let versi = match lama {
        Some(versi) => {
            sqlx::query(
                "UPDATE provider_secret SET secret_versi = $2, secret_ciphertext = $3, \
                 secret_dek_terbungkus = $4, secret_nonce = $5, kek_id = $6, \
                 diubah_at = now(), diubah_oleh = $7 WHERE nama_secret = $1",
            )
            .bind(&nama)
            .bind(versi + 1)
            .bind(&amplop.ciphertext)
            .bind(&amplop.dek_terbungkus)
            .bind(amplop.nonce.as_slice())
            .bind(&amplop.kek_id)
            .bind(peminta.pengguna_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| galat_db(&e))?;
            versi + 1
        }
        None => {
            sqlx::query(
                "INSERT INTO provider_secret (nama_secret, secret_versi, secret_ciphertext, \
                 secret_dek_terbungkus, secret_nonce, kek_id, diubah_oleh) \
                 VALUES ($1, 1, $2, $3, $4, $5, $6)",
            )
            .bind(&nama)
            .bind(&amplop.ciphertext)
            .bind(&amplop.dek_terbungkus)
            .bind(amplop.nonce.as_slice())
            .bind(&amplop.kek_id)
            .bind(peminta.pengguna_id)
            .execute(&mut *tx)
            .await
            .map_err(|e| galat_db(&e))?;
            1
        }
    };

    catat(
        &mut tx,
        &peminta,
        if lama.is_some() {
            AksiKonfigurasi::Ubah
        } else {
            AksiKonfigurasi::Buat
        },
        &nama,
        lama.map(|v| json!({ "secret_versi": v })),
        Some(json!({ "secret_versi": versi })),
        format!("putSecret {nama}"),
    )
    .await?;
    tx.commit().await.map_err(|e| galat_db(&e))?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

pub(super) async fn delete_secret(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(nama): Path<String>,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &method, &headers, peer, &PERAN_PA, "deleteSecret").await?;
    super::gerbang::periksa_guard_mfa(&peminta)?;
    periksa_nama(&nama)?;

    let pemakai = provider_pemakai(&s.pool, &nama).await?;
    if !pemakai.is_empty() {
        // 409 with the names of the active providers, never the value (AC-10).
        let detail = format!(
            "secret {nama} masih dipakai provider aktif: {}",
            pemakai.into_iter().collect::<Vec<_>>().join(", ")
        );
        return Err(Galat::konflik(ProblemKodeAdmin::SecretDipakai, detail));
    }

    let mut tx = s.pool.begin().await.map_err(|e| galat_db(&e))?;
    let lama: Option<i32> = sqlx::query_scalar(
        "SELECT secret_versi FROM provider_secret WHERE nama_secret = $1 FOR UPDATE",
    )
    .bind(&nama)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| galat_db(&e))?;
    let Some(versi) = lama else {
        return Err(Galat::tidak_ditemukan());
    };
    sqlx::query("DELETE FROM provider_secret WHERE nama_secret = $1")
        .bind(&nama)
        .execute(&mut *tx)
        .await
        .map_err(|e| galat_db(&e))?;

    catat(
        &mut tx,
        &peminta,
        AksiKonfigurasi::Cabut,
        &nama,
        Some(json!({ "secret_versi": versi })),
        Some(json!({})),
        format!("deleteSecret {nama}"),
    )
    .await?;
    tx.commit().await.map_err(|e| galat_db(&e))?;

    Ok(StatusCode::NO_CONTENT.into_response())
}

/// `PUT /admin/v1/secrets/`: an empty name reaches the same handler so
/// `periksa_nama` can answer 422 instead of the router answering 404 (AC-7).
pub(super) async fn put_secret_kosong(
    state: State<AdminState>,
    method: Method,
    headers: HeaderMap,
    peer: ConnectInfo<SocketAddr>,
    body: Result<Json<SecretNilai>, axum::extract::rejection::JsonRejection>,
) -> Result<Response, Galat> {
    put_secret(state, method, headers, peer, Path(String::new()), body).await
}

/// `DELETE /admin/v1/secrets/`, the empty-name counterpart of `delete_secret`.
pub(super) async fn delete_secret_kosong(
    state: State<AdminState>,
    method: Method,
    headers: HeaderMap,
    peer: ConnectInfo<SocketAddr>,
) -> Result<Response, Galat> {
    delete_secret(state, method, headers, peer, Path(String::new())).await
}

/// Names of the active providers whose current configuration mentions
/// `{{secret.<nama>}}`, sorted and de-duplicated.
async fn provider_pemakai(pool: &sqlx::PgPool, nama: &str) -> Result<BTreeSet<String>, Galat> {
    let corak = format!("{{{{secret.{nama}}}}}");
    // "Active configuration" is `config_versi_aktif` when the provider has one;
    // before the field is maintained (createProvider, later ticket) the highest
    // version is the one that would be used, so both are covered.
    let baris: Vec<(String,)> = sqlx::query_as(
        "SELECT p.nama FROM provider p \
         JOIN provider_config_versi v ON v.provider_id = p.provider_id \
           AND v.versi = coalesce(p.config_versi_aktif, \
             (SELECT max(v2.versi) FROM provider_config_versi v2 WHERE v2.provider_id = p.provider_id)) \
         WHERE p.aktif AND position($1 IN v.dokumen::text) > 0 \
         ORDER BY p.nama",
    )
    .bind(corak)
    .fetch_all(pool)
    .await
    .map_err(|e| galat_db(&e))?;
    Ok(baris.into_iter().map(|(n,)| n).collect())
}

/// 422 with the offending field when the name cannot be a secret key (AC-9).
fn periksa_nama(nama: &str) -> Result<(), Galat> {
    let panjang = nama.chars().count();
    if panjang > NAMA_PANJANG_MAKS {
        return Err(galat_validasi(vec![FieldError {
            field: "nama",
            kode: "rentang",
            batas: Some(NAMA_PANJANG_MAKS as f64),
        }]));
    }
    let mut karakter = nama.chars();
    let sah = matches!(karakter.next(), Some(c) if c.is_ascii_alphabetic())
        && karakter.all(|c| c.is_ascii_alphanumeric() || c == '_');
    if !sah {
        return Err(galat_validasi(vec![FieldError {
            field: "nama",
            kode: "format",
            batas: None,
        }]));
    }
    Ok(())
}

/// One `audit_konfigurasi` row in the caller's transaction; `nilai_lama` and
/// `nilai_baru` describe the change (version numbers) and gain the secret name,
/// never the value (AC-5).
async fn catat(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    peminta: &super::gerbang::Peminta,
    aksi: AksiKonfigurasi,
    nama: &str,
    nilai_lama: Option<serde_json::Value>,
    nilai_baru: Option<serde_json::Value>,
    alasan: String,
) -> Result<(), Galat> {
    let baris = KonfigurasiBaru {
        pelaku: peminta.pengguna_id,
        entitas: EntitasKonfigurasi::ProviderSecret,
        entitas_id: None,
        aksi,
        nilai_lama: dengan_nama(nilai_lama, nama),
        nilai_baru: dengan_nama(nilai_baru, nama),
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

/// The audit row names the secret: the name is not a secret, and `entitas_id` is
/// NULL because the store has no surrogate id (AD1).
fn dengan_nama(nilai: Option<serde_json::Value>, nama: &str) -> Option<serde_json::Value> {
    nilai.map(|mut v| {
        if let serde_json::Value::Object(ref mut m) = v {
            m.insert(
                "nama".to_owned(),
                serde_json::Value::String(nama.to_owned()),
            );
        }
        v
    })
}

fn galat_db(e: &sqlx::Error) -> Galat {
    tracing::error!(error = %e, "admin: kueri secret gagal");
    Galat::layanan()
}
