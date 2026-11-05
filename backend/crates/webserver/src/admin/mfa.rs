//! Second login step and enrollment: `POST /auth/mfa`, `POST /auth/mfa/daftar`,
//! `POST /auth/mfa/daftar/konfirmasi` (ADR-0006 §9, AC-10..AC-13).
//!
//! Every entry point here starts from the `__Host-pr_mfa` cookie, never from a
//! session cookie: the password step already decided who the user is, and the
//! pre-session is the only thing that says so.
//!
//! Invariants this module keeps:
//! - **One token, one use.** [`PraSesiStore::pakai`] consumes the token in the
//!   same `GETDEL` that reads it, so a second call with the same cookie is 401
//!   even when it races the first (AC-10, AC-13).
//! - **The wrong step is 401, not 403.** A `daftar_mfa` token cannot verify a
//!   code and an `mfa` token cannot start enrollment; both are simply not
//!   accepted here (AC-10 "token `daftar_mfa` ... → 401").
//! - **One failure message.** TOTP and recovery-code failures are the same 401
//!   `kredensial_salah`; the fifth failure destroys the token (AC-11).
//! - **Nothing sensitive is logged.** Codes, secrets, the `otpauth://` URI and
//!   the recovery codes never reach `tracing`, `audit_akses` or an error body
//!   (AC-3). The enrollment URI and the codes appear once, in a 200 body.

use axum::extract::ConnectInfo;
use axum::extract::State;
use axum::extract::rejection::JsonRejection;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use poolrouter_core::admin::mfa::{HasilPemulihan, HasilPeriksa, HasilVerifikasi, KodePemulihan};
use poolrouter_core::admin::pra_sesi::{JenisPraSesi, PraSesi};
use serde::Deserialize;
use uuid::Uuid;

use super::login::{COOKIE_PRA_SESI, sumber_ip};
use super::pembatas::HasilPeriksa as HasilBatas;
use super::sesi::{AdminState, COOKIE_SESI, Galat, periksa_origin};
use crate::api::problem::ProblemKodeAdmin;

/// `POST /auth/mfa` body: either a TOTP code or a recovery code (AC-10).
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct KodeMfaRequest {
    #[serde(default)]
    pub kode: Option<String>,
    #[serde(default)]
    pub kode_pemulihan: Option<String>,
}

/// `POST /auth/mfa/daftar` body: the registration code issued to this user
/// (AC-12). Enrollment cannot start without one.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DaftarRequest {
    pub kode_pendaftaran: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct KonfirmasiRequest {
    pub kode_totp: String,
}

#[derive(Debug, serde::Serialize)]
pub(super) struct DaftarBody {
    /// `otpauth://totp/...` with issuer `PoolRouter` and the username label.
    pub otpauth_uri: String,
    pub rahasia_base32: String,
}

#[derive(Debug, serde::Serialize)]
pub(super) struct KonfirmasiBody {
    pub kode_pemulihan: Vec<String>,
}

/// Read the pre-session token from its cookie. Missing or empty is `None`; the
/// caller turns that into the same 401 as an unknown token.
fn token_pra_sesi(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| {
            let (nama, nilai) = pair.split_once('=')?;
            (nama.trim() == COOKIE_PRA_SESI).then(|| nilai.trim().to_owned())
        })
        .find(|v| !v.is_empty())
}

fn cookie_sesi(token: &str) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{COOKIE_SESI}={token}; Path=/; HttpOnly; Secure; SameSite=Strict"
    ))
    .expect("token base64url selalu ASCII")
}

/// Clear the pre-session cookie: the token is spent whatever happens next.
fn cookie_hapus_pra_sesi() -> HeaderValue {
    HeaderValue::from_static("__Host-pr_mfa=; Path=/; HttpOnly; Secure; SameSite=Strict; Max-Age=0")
}

fn salah() -> Galat {
    Galat::baru(StatusCode::UNAUTHORIZED, ProblemKodeAdmin::KredensialSalah)
}

/// Resolve the cookie to a live pre-session of the expected kind.
///
/// A missing cookie, an unknown token, and the wrong kind all land on the same
/// 401 so the response never says which one it was.
async fn pra_sesi_dari(
    s: &AdminState,
    headers: &HeaderMap,
    jenis: JenisPraSesi,
) -> Result<(String, PraSesi), Galat> {
    let token = token_pra_sesi(headers).ok_or_else(salah)?;
    let pra = s.pra_sesi.pakai(&token).await.map_err(|e| {
        tracing::error!(error = %e, "admin: pra-sesi tidak terbaca");
        Galat::layanan()
    })?;
    match pra {
        Some(p) if p.jenis == jenis => Ok((token, p)),
        // Wrong kind: consume and fail. A `daftar_mfa` token presented at
        // `/auth/mfa` must not survive to be tried again.
        Some(_) | None => Err(salah()),
    }
}

/// Issue the real session after a successful second step (AC-10, AC-13).
async fn terbitkan_sesi_mfa(
    s: &AdminState,
    pengguna_id: Uuid,
    headers: &HeaderMap,
    ip: &str,
) -> Result<Response, Galat> {
    let sekarang = (s.jam)();
    let token = poolrouter_core::admin::TokenSesi::baru();
    let ua = headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.chars().take(512).collect::<String>());
    let sesi = s
        .sesi
        .terbitkan_terverifikasi_mfa(pengguna_id, &token.hash(), ua.as_deref(), sekarang)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "admin: penerbitan sesi MFA gagal");
            Galat::layanan()
        })?;
    let dicatat = super::login::catat_login_berhasil(s, pengguna_id, sesi.sesi_id, ip, &ua).await;
    if !dicatat {
        let _ = s.sesi.cabut(&token.hash()).await;
        return Err(Galat::layanan());
    }
    let mut res = StatusCode::NO_CONTENT.into_response();
    res.headers_mut()
        .insert(header::SET_COOKIE, cookie_sesi(token.sebagai_b64()));
    res.headers_mut()
        .append(header::SET_COOKIE, cookie_hapus_pra_sesi());
    Ok(res)
}

pub(super) async fn mfa(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    body: Result<axum::Json<KodeMfaRequest>, JsonRejection>,
) -> Result<Response, Galat> {
    periksa_origin(&method, &headers, &s.asal)?;
    let axum::Json(req) = body.map_err(|_| {
        Galat::baru(
            StatusCode::UNPROCESSABLE_ENTITY,
            ProblemKodeAdmin::ValidasiGagal,
        )
    })?;
    let ip = sumber_ip(&headers, peer);
    let ua = super::login::user_agent(&headers);
    let sekarang: DateTime<Utc> = (s.jam)();
    let (token, pra) = pra_sesi_dari(&s, &headers, JenisPraSesi::Mfa).await?;
    let pengguna_id = pra.pengguna_id;

    // AC-11: the code step shares the password limiter's bucket, so guessing
    // codes is bounded by the same 5-per-15-minutes rule.
    let username = super::login::username_dari(&s.pool, pengguna_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "admin: pencarian username gagal");
            Galat::layanan()
        })?;
    match s.pembatas.periksa(&username).await {
        Err(_) => return Err(Galat::layanan()),
        Ok(HasilBatas::Terkunci { coba_lagi_detik }) => {
            // AC-11: a refused attempt is an `audit_login` row too.
            super::login::audit(
                &s,
                Some(pengguna_id),
                poolrouter_core::audit::HasilLogin::Terkunci,
                None,
                &ip,
                &ua,
            )
            .await;
            return Err(Galat::dibatasi(coba_lagi_detik));
        }
        Ok(HasilBatas::Bebas) => {}
    }

    // A TOTP code and a recovery code are mutually exclusive; neither, or both,
    // is a client error and does not count as a guessing attempt.
    let hasil = match (req.kode.as_deref(), req.kode_pemulihan.as_deref()) {
        (Some(kode), None) => match s.mfa.verifikasi(pengguna_id, kode.trim(), sekarang).await {
            HasilVerifikasi::Terima { .. } => Ok(()),
            HasilVerifikasi::Tolak(_) => Err(()),
        },
        (None, Some(kode)) => {
            // The store writes its own `kode_pemulihan` audit row; the session
            // id is not issued yet, so it is recorded as the pre-session's user.
            match s
                .mfa
                .pakai_kode_pemulihan(
                    pengguna_id,
                    kode.trim(),
                    &ip,
                    &pengguna_id.to_string(),
                    sekarang,
                )
                .await
            {
                HasilPemulihan::Terima { .. } => Ok(()),
                HasilPemulihan::Tolak(_) => Err(()),
            }
        }
        _ => {
            return Err(Galat::baru(
                StatusCode::UNPROCESSABLE_ENTITY,
                ProblemKodeAdmin::ValidasiGagal,
            ));
        }
    };

    match hasil {
        Ok(()) => {
            if s.pembatas.berhasil(&username).await.is_err() {
                tracing::warn!("admin: hitungan gagal tidak terhapus setelah MFA berhasil");
            }
            terbitkan_sesi_mfa(&s, pengguna_id, &headers, &ip).await
        }
        Err(()) => {
            // One generic failure for both code kinds (AC-11).
            super::login::audit(
                &s,
                Some(pengguna_id),
                poolrouter_core::audit::HasilLogin::Gagal,
                None,
                &ip,
                &ua,
            )
            .await;
            let _ = s.pembatas.catat_gagal(&username).await;
            let hangus = s.pra_sesi.catat_gagal(&token).await.unwrap_or(true);
            if hangus {
                tracing::info!("admin: token pra-sesi hangus setelah batas kegagalan");
            }
            Err(salah())
        }
    }
}

pub(super) async fn daftar(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    body: Result<axum::Json<DaftarRequest>, JsonRejection>,
) -> Result<Response, Galat> {
    periksa_origin(&method, &headers, &s.asal)?;
    // AC-12: no code, someone else's code, an expired code or a dropped one is
    // refused. The body is required, so a missing code is 422, not "allowed".
    let axum::Json(req) = body.map_err(|_| {
        Galat::baru(
            StatusCode::UNPROCESSABLE_ENTITY,
            ProblemKodeAdmin::ValidasiGagal,
        )
    })?;
    let ip = sumber_ip(&headers, peer);
    let sekarang = (s.jam)();
    let (token, pra) = pra_sesi_dari(&s, &headers, JenisPraSesi::DaftarMfa).await?;

    let pengguna_id = pra.pengguna_id;
    let username = super::login::username_dari(&s.pool, pengguna_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "admin: pencarian username gagal");
            Galat::layanan()
        })?;

    // AC-8: the code is checked without being consumed here; consumption
    // happens at confirmation (AC-13), so a failed enrollment start leaves it
    // usable, while a wrong value still counts toward its 5-failure cap.
    let sah = matches!(
        s.mfa
            .periksa_kode_daftar(pengguna_id, req.kode_pendaftaran.trim(), sekarang)
            .await,
        HasilPeriksa::Berlaku
    );
    catat_daftar_mfa(&s, pengguna_id, sah, &ip).await;
    if !sah {
        s.pra_sesi.catat_gagal(&token).await.ok();
        return Err(salah());
    }

    let pendaftaran = s.mfa.daftar(pengguna_id).await.map_err(|e| {
        tracing::error!(error = %e, "admin: pendaftaran MFA gagal");
        Galat::layanan()
    })?;
    // Issuer `PoolRouter`, label the username, no external service (AC-12).
    let uri = format!(
        "otpauth://totp/PoolRouter:{}?secret={}&issuer=PoolRouter&algorithm=SHA1&digits=6&period=30",
        url_encode(&username),
        pendaftaran.rahasia()
    );
    let rahasia = pendaftaran.rahasia().to_owned();
    Ok((
        StatusCode::OK,
        axum::Json(DaftarBody {
            otpauth_uri: uri,
            rahasia_base32: rahasia,
        }),
    )
        .into_response())
}

/// One `audit_akses` row with action `daftar_mfa`, success or failure (AC-12,
/// AC-13). Written before the response so a refused start is always on record.
async fn catat_daftar_mfa(s: &AdminState, pengguna_id: Uuid, berhasil: bool, ip: &str) {
    use poolrouter_core::audit::{AksesBaru, Aksi, Hasil, Pelaku, Penulis, Peran, TampilanNik};

    // The audit row must not outlive a failed request, so it travels in its own
    // tiny transaction; a broken audit trail is logged, never silently dropped.
    let tx = match s.pool.begin().await {
        Ok(tx) => tx,
        Err(e) => {
            tracing::error!(error = %e, "admin: audit daftar_mfa tidak bisa dimulai");
            return;
        }
    };
    let mut tx = tx;
    let baris = AksesBaru {
        // The role is only used to label the actor; enrollment is self-service,
        // so the caller's own roles are what `me` reports. Auditor is the
        // narrowest label that never grants more than they hold at this step.
        pelaku: Pelaku::baru(pengguna_id, Peran::Auditor),
        aksi: Aksi::DaftarMfa,
        tampilan_nik: TampilanNik::TidakAda,
        hasil: if berhasil {
            Hasil::Berhasil
        } else {
            Hasil::Ditolak
        },
        filter_kueri: None,
        nik_hmac: None,
        nik_tersamar: None,
        jumlah_record: None,
        alasan: None,
        sumber_ip: Some(ip.to_owned()),
        sesi_id: Some(pengguna_id.to_string()),
    };
    let hasil = Penulis::baru(&mut tx).catat_akses(baris).await;
    match hasil {
        Ok(_) => {
            if let Err(e) = tx.commit().await {
                tracing::error!(error = %e, "admin: audit daftar_mfa gagal di-commit");
            }
        }
        Err(e) => {
            tracing::error!(error = %e, "admin: audit daftar_mfa gagal ditulis");
        }
    }
}

pub(super) async fn konfirmasi(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<std::net::SocketAddr>,
    body: Result<axum::Json<KonfirmasiRequest>, JsonRejection>,
) -> Result<Response, Galat> {
    periksa_origin(&method, &headers, &s.asal)?;
    let axum::Json(req) = body.map_err(|_| {
        Galat::baru(
            StatusCode::UNPROCESSABLE_ENTITY,
            ProblemKodeAdmin::ValidasiGagal,
        )
    })?;
    let ip = sumber_ip(&headers, peer);
    let ua = super::login::user_agent(&headers);
    let sekarang = (s.jam)();
    let (token, pra) = pra_sesi_dari(&s, &headers, JenisPraSesi::DaftarMfa).await?;
    let pengguna_id = pra.pengguna_id;

    let username = super::login::username_dari(&s.pool, pengguna_id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "admin: pencarian username gagal");
            Galat::layanan()
        })?;
    match s.pembatas.periksa(&username).await {
        Err(_) => return Err(Galat::layanan()),
        Ok(HasilBatas::Terkunci { coba_lagi_detik }) => {
            // AC-11: a refused attempt is an `audit_login` row too.
            super::login::audit(
                &s,
                Some(pengguna_id),
                poolrouter_core::audit::HasilLogin::Terkunci,
                None,
                &ip,
                &ua,
            )
            .await;
            return Err(Galat::dibatasi(coba_lagi_detik));
        }
        Ok(HasilBatas::Bebas) => {}
    }

    let kode = req.kode_totp.trim().to_owned();
    let konfirmasi = s
        .mfa
        .konfirmasi(pengguna_id, &kode, sekarang)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "admin: konfirmasi MFA gagal");
            Galat::layanan()
        });
    let daftar_kode: KodePemulihan = match konfirmasi {
        Ok(k) => k,
        Err(_) => {
            super::login::audit(
                &s,
                Some(pengguna_id),
                poolrouter_core::audit::HasilLogin::Gagal,
                None,
                &ip,
                &ua,
            )
            .await;
            let _ = s.pembatas.catat_gagal(&username).await;
            s.pra_sesi.catat_gagal(&token).await.ok();
            catat_daftar_mfa(&s, pengguna_id, false, &ip).await;
            return Err(salah());
        }
    };
    if s.pembatas.berhasil(&username).await.is_err() {
        tracing::warn!("admin: hitungan gagal tidak terhapus setelah konfirmasi berhasil");
    }
    // AC-13: the successful enrollment is on record before the session exists.
    catat_daftar_mfa(&s, pengguna_id, true, &ip).await;
    // AC-13: the session is issued with `mfa_terverifikasi_at`, and the codes
    // travel in this one body only.
    let res = terbitkan_sesi_mfa(&s, pengguna_id, &headers, &ip).await?;
    let kode_json = serde_json::to_vec(&KonfirmasiBody {
        kode_pemulihan: daftar_kode.daftar().to_vec(),
    })
    .map_err(|_| Galat::layanan())?;
    // `terbitkan_sesi_mfa` answered 204 with the `Set-Cookie` headers; the
    // enrollment adds a JSON body and keeps those headers.
    let status = StatusCode::OK;
    let mut akhir = Response::new(axum::body::Body::from(kode_json));
    *akhir.status_mut() = status;
    akhir.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    for v in res.headers().get_all(header::SET_COOKIE).iter() {
        akhir.headers_mut().append(header::SET_COOKIE, v.clone());
    }
    Ok(akhir)
}

/// Percent-encode the `otpauth://` label (RFC 3986 unreserved set kept).
fn url_encode(teks: &str) -> String {
    let mut out = String::with_capacity(teks.len());
    for b in teks.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                use std::fmt::Write as _;
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}
