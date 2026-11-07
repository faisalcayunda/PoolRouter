//! Request log, NIK search, payload opening, export, and audit reading
//! (US-14, US-16, R-18; AC-32..AC-38).
//!
//! Every operation here reads personal data from `transaction_log`, so the rule
//! that shapes the module is: **the audit row is not optional**.
//! `searchLog` and `getLogEntry` write their own row through
//! [`Penulis`]; the auditor operations call a `SECURITY DEFINER` function that
//! writes the row in the same statement that reads the data (ADR-0003 §3.6c).
//! A request cannot observe a NIK without leaving `cari_log`, `cari_nik`,
//! `lihat_payload`, `ekspor`, or `lihat_audit` behind. A wrong role is caught by
//! [`masuk`] before any of that and lands as `akses_ditolak` (AC-38, AC-4).
//!
//! Two shapes follow from the schema rather than being invented here:
//! - OP sees masked NIK only, always from `v_transaction_log_tersamar`. The view
//!   is the masking; this module never holds the full NIK for an OP caller.
//! - The export bytes, their checksum, and the `ekspor_checksum` audit column all
//!   come out of `fn_ekspor_log_auditor` in one round trip (AC-36, E-01..E-05).
//!   Rust relays the bytes and re-derives the digest header from them so the
//!   client can verify what it received against what was recorded.

use std::net::SocketAddr;

use axum::Json;
use axum::body::Body;
use axum::extract::{ConnectInfo, Path, State};
use axum::http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Duration, Utc};
use poolrouter_core::admin::{Nik, mengandung_potongan_nik, tersamar_audit};
use poolrouter_core::audit::{AksesBaru, Aksi, NikHmac, Pelaku, Penulis, Peran, TampilanNik};
use serde_json::Value;
use uuid::Uuid;

use super::gerbang::{Peminta, galat_validasi, masuk};
use super::sesi::{AdminState, Galat, periksa_origin};

use crate::api::admin::{
    AuditAksesBaris, AuditAksesPage, AuditCariRequest, EksporRequest, FieldError, LogBaris,
    LogCariRequest, LogEntry, LogPage, NikCariRequest, PayloadOpenRequest, PayloadView,
};
use crate::api::problem::ProblemKodeAdmin;

/// Roles that may read the request log (AC-32): operator and auditor.
const PERAN_LOG: &[Peran] = &[Peran::Operator, Peran::Auditor];

/// Auditor-only operations (AC-34..AC-38). A platform admin that is not an
/// auditor is refused too, because AU is the only accepted role here.
const PERAN_AU: &[Peran] = &[Peran::Auditor];

/// `searchAuditAkses` refuses a range wider than this (AC-37).
const BATAS_RENTANG_AUDIT: Duration = Duration::days(31);

/// `alasan` bounds for the auditor operations (AC-34).
const ALASAN_MIN: f64 = 10.0;
const ALASAN_MAKS: f64 = 500.0;

/// Page size for the log and audit listings; the cursor scheme is a later
/// ticket, so reads cap and say so.
const BATAS_BARIS: i32 = 100;

// ---------------------------------------------------------------------------
// searchLog / getLogEntry (AC-32, AC-33)
// ---------------------------------------------------------------------------

/// `searchLog`: mandatory time range, every free-text parameter screened for a
/// NIK fragment *before* the query runs, one `cari_log` row per call.
pub(super) async fn search_log(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    body: Result<Json<LogCariRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &method, &headers, peer, PERAN_LOG, "searchLog").await?;
    periksa_origin(&method, &headers, &s.asal)?;

    // The body is extracted before the handler runs, so a malformed one
    // must not shadow the 401/403 the gate just decided: unwrap it only now.
    let axum::Json(body) = body.map_err(|_| {
        galat_validasi(vec![FieldError {
            field: "body",
            kode: "format",
            batas: None,
        }])
    })?;

    if let Some(v) = parameter_ber_nik(&body) {
        let _ = v;
        catat_ditolak(&s, &peminta, Aksi::CariLog).await?;
        return Err(Galat::baru(
            StatusCode::FORBIDDEN,
            ProblemKodeAdmin::PencarianNikDilarang,
        ));
    }

    if body
        .kode_transaksi
        .as_deref()
        .is_some_and(|k| Uuid::parse_str(k).is_err())
    {
        return Err(galat_validasi(vec![FieldError {
            field: "kode_transaksi",
            kode: "format",
            batas: None,
        }]));
    }

    periksa_rentang(body.dari, body.sampai)?;

    let baris = baris_log(&s, &body).await?;
    let jumlah = baris.len() as i64;

    let akses = AksesBaru::berhasil(pelaku_dari(&peminta), Aksi::CariLog)
        .dengan_tampilan_nik(TampilanNik::Tersamar)
        .dengan_filter(filter_cari_log(&body))
        .dengan_jumlah_record(jumlah)
        .dengan_sumber_ip(peminta.sumber_ip.clone())
        .dengan_sesi(peminta.sesi_id.to_string());
    tulis_akses(&s, akses).await?;

    Ok(Json(LogPage {
        baris,
        kursor: None,
        di_luar_retensi_online: false,
    })
    .into_response())
}

/// `getLogEntry`: one masked row for OP, one `lihat_log` row, 404 if absent.
pub(super) async fn get_log_entry(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(kode): Path<String>,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &method, &headers, peer, PERAN_LOG, "getLogEntry").await?;

    let kode = Uuid::parse_str(&kode).map_err(|_| Galat::tidak_ditemukan())?;
    let baris: Option<BarisLog> = sqlx::query_as(
        "SELECT v.nik_tersamar, v.requested_at, coalesce(v.status_akses, '') AS status_akses,
                v.kode_transaksi, v.request_id, v.client_id, c.nama AS client_nama
           FROM v_transaction_log_tersamar v
           JOIN client c ON c.client_id = v.client_id
          WHERE v.kode_transaksi = $1",
    )
    .bind(kode)
    .fetch_optional(&s.pool)
    .await
    .map_err(|e| galat_kueri(&e))?;

    let Some(baris) = baris else {
        return Err(Galat::tidak_ditemukan());
    };

    // `chk_filter_wajib`: every `lihat_log` row must say which row was read.
    let akses = AksesBaru::berhasil(pelaku_dari(&peminta), Aksi::LihatLog)
        .dengan_tampilan_nik(TampilanNik::Tersamar)
        .dengan_filter(serde_json::json!({ "kode_transaksi": kode }))
        .dengan_jumlah_record(1)
        .dengan_sumber_ip(peminta.sumber_ip.clone())
        .dengan_sesi(peminta.sesi_id.to_string());
    tulis_akses(&s, akses).await?;

    Ok(Json(LogEntry {
        nik: baris.nik_tersamar,
        requested_at: baris.requested_at,
        status_akses: baris.status_akses,
        kode_transaksi: baris.kode_transaksi.to_string(),
        queue_id: baris.request_id,
        client_id: baris.client_id,
        client_nama: baris.client_nama,
    })
    .into_response())
}

/// The first `searchLog` parameter that carries a NIK fragment (AC-32).
///
/// Only free-text fields are screened. `dari`, `sampai`, `queue_id`, and
/// `client_id` are typed and cannot carry a NIK: a timestamp is digits with
/// separators, and a UUID is not a run of digits. `kode_transaksi` is a UUID but
/// arrives as a string, so it is screened too.
fn parameter_ber_nik(body: &LogCariRequest) -> Option<&str> {
    [
        body.kode_transaksi.as_deref(),
        body.status_akses.as_deref(),
        body.kursor.as_deref(),
    ]
    .into_iter()
    .flatten()
    .find(|v| mengandung_potongan_nik(v))
}

// ---------------------------------------------------------------------------
// searchLogByNik (AC-34)
// ---------------------------------------------------------------------------

/// `searchLogByNik`: exact 16-digit NIK, `alasan` 10..500, one `cari_nik` row
/// per call including page turns and empty results, written by
/// `fn_auditor_cari_nik` itself.
pub(super) async fn search_log_by_nik(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    body: Result<Json<NikCariRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &method, &headers, peer, PERAN_AU, "searchLogByNik").await?;
    super::sesi::periksa_guard_mfa(&s, &peminta).await?;

    // The body is extracted before the handler runs, so a malformed one
    // must not shadow the 401/403 the gate just decided: unwrap it only now.
    let axum::Json(body) = body.map_err(|_| {
        galat_validasi(vec![FieldError {
            field: "body",
            kode: "format",
            batas: None,
        }])
    })?;

    let mut galat = Vec::new();
    let nik = match Nik::parse(body.nik.trim()) {
        Ok(n) => Some(n),
        Err(_) => {
            galat.push(FieldError {
                field: "nik",
                kode: "format",
                batas: Some(Nik::PANJANG as f64),
            });
            None
        }
    };
    let alasan = periksa_alasan(&body.alasan, &mut galat);
    rentang_ke_galat(body.dari, body.sampai, &mut galat);
    if !galat.is_empty() {
        return Err(galat_validasi(galat));
    }
    let (Some(nik), Some(alasan)) = (nik, alasan) else {
        return Err(Galat::layanan());
    };

    // The HMAC column is what lets a later `audit_akses` row say "the same NIK
    // was searched again" without storing the NIK (contract 8.2).
    let hmac = NikHmac::parse(hex_dari(sha256(nik.utuh().expose().as_bytes())))
        .map_err(|_| Galat::layanan())?;

    let baris: Vec<BarisNik> = sqlx::query_as(
        "SELECT l.kode_transaksi, l.request_id, l.client_id, l.requested_at, l.status_akses,
                c.nama AS client_nama
           FROM fn_auditor_cari_nik($1, $2, $3, $4, $5, $6, $7, $8)
             l JOIN client c ON c.client_id = l.client_id
          ORDER BY requested_at DESC, kode_transaksi DESC",
    )
    .bind(peminta.pengguna_id)
    .bind(nik.utuh().expose())
    .bind(hmac.as_str())
    .bind(body.dari)
    .bind(body.sampai)
    .bind(&alasan)
    .bind(&peminta.sumber_ip)
    .bind(peminta.sesi_id.to_string())
    .fetch_all(&s.pool)
    .await
    .map_err(|e| galat_kueri(&e))?;

    // AC-34: the answer carries no payload and no secret metadata, and only the
    // auditor sees the full NIK the caller already supplied.
    let nik_utuh = nik.utuh().expose().to_string();
    let baris = baris
        .into_iter()
        .map(|b| LogBaris {
            nik: nik_utuh.clone(),
            requested_at: b.requested_at,
            status_akses: b.status_akses.unwrap_or_default(),
            kode_transaksi: b.kode_transaksi.to_string(),
            queue_id: b.request_id,
            client_id: b.client_id,
            client_nama: b.client_nama,
        })
        .collect();

    Ok(Json(LogPage {
        baris,
        kursor: None,
        di_luar_retensi_online: false,
    })
    .into_response())
}

// ---------------------------------------------------------------------------
// openPayload (AC-35)
// ---------------------------------------------------------------------------

/// `openPayload`: exactly one of `request_id` or `kode_transaksi`, plus
/// `alasan`. The `lihat_payload` row is written by
/// `fn_auditor_lihat_transaksi` in the same statement that returns the payload.
pub(super) async fn open_payload(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    body: Result<Json<PayloadOpenRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &method, &headers, peer, PERAN_AU, "openPayload").await?;

    // The body is extracted before the handler runs, so a malformed one
    // must not shadow the 401/403 the gate just decided: unwrap it only now.
    let axum::Json(body) = body.map_err(|_| {
        galat_validasi(vec![FieldError {
            field: "body",
            kode: "format",
            batas: None,
        }])
    })?;

    let mut galat = Vec::new();
    let kode_terisi = body
        .kode_transaksi
        .as_deref()
        .map(str::trim)
        .is_some_and(|k| !k.is_empty());
    if body.request_id.is_some() == kode_terisi {
        // Neither or both: AC-35 asks for exactly one.
        galat.push(FieldError {
            field: "request_id",
            kode: "wajib_pilih_satu",
            batas: None,
        });
    }
    let alasan = periksa_alasan(&body.alasan, &mut galat);
    if !galat.is_empty() {
        return Err(galat_validasi(galat));
    }
    let Some(alasan) = alasan else {
        return Err(Galat::layanan());
    };

    let kode = match (body.request_id, body.kode_transaksi.as_deref()) {
        (_, Some(k)) if kode_terisi => match Uuid::parse_str(k.trim()) {
            Ok(kode) => Some(kode),
            Err(_) => return Err(Galat::tidak_ditemukan()),
        },
        // `pr_admin_api` has no direct grant on `transaction_log`; the mapping is
        // read from the masked view, which is what the role is allowed to see.
        (Some(rid), _) => sqlx::query_scalar::<_, Option<Uuid>>(
            "SELECT final_kode_transaksi FROM v_request_tersamar WHERE request_id = $1",
        )
        .bind(rid)
        .fetch_optional(&s.pool)
        .await
        .map_err(|e| galat_kueri(&e))?
        .flatten(),
        _ => None,
    };
    let Some(kode) = kode else {
        return Err(Galat::tidak_ditemukan());
    };

    let baris: Option<BarisPayload> = sqlx::query_as(
        "SELECT request_id, requested_at, coalesce(status_akses, '') AS status_akses, log_mode,
                request_payload, response_payload, provider_response_code
           FROM fn_auditor_lihat_transaksi($1, $2, $3, $4, $5)",
    )
    .bind(peminta.pengguna_id)
    .bind(&alasan)
    .bind(kode)
    .bind(&peminta.sumber_ip)
    .bind(peminta.sesi_id.to_string())
    .fetch_optional(&s.pool)
    .await
    .map_err(|e| galat_kueri(&e))?;

    let Some(b) = baris else {
        return Err(Galat::tidak_ditemukan());
    };

    Ok(Json(PayloadView {
        request_id: b.request_id,
        kode_transaksi: kode.to_string(),
        status_akses: b.status_akses,
        requested_at: b.requested_at,
        input: b.request_payload,
        metadata: b.provider_response_code.map(Value::from),
        hasil: b.response_payload,
    })
    .into_response())
}

// ---------------------------------------------------------------------------
// exportLogAuditor (AC-36)
// ---------------------------------------------------------------------------

/// `exportLogAuditor`: the CSV bytes, their checksum, and the `ekspor` audit row
/// come from one function call. Over the row limit the answer is 422 with no
/// file and a `ditolak` row (E-03).
pub(super) async fn export_log_auditor(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    body: Result<Json<EksporRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &method, &headers, peer, PERAN_AU, "exportLogAuditor").await?;

    // The body is extracted before the handler runs, so a malformed one
    // must not shadow the 401/403 the gate just decided: unwrap it only now.
    let axum::Json(body) = body.map_err(|_| {
        galat_validasi(vec![FieldError {
            field: "body",
            kode: "format",
            batas: None,
        }])
    })?;

    let mut galat = Vec::new();
    let nik_bersih = body.nik.trim().to_owned();
    if Nik::parse(&nik_bersih).is_err() {
        galat.push(FieldError {
            field: "nik",
            kode: "format",
            batas: Some(Nik::PANJANG as f64),
        });
    }
    let alasan = periksa_alasan(&body.alasan, &mut galat);
    rentang_ke_galat(body.dari, body.sampai, &mut galat);
    if !galat.is_empty() {
        return Err(galat_validasi(galat));
    }
    let Some(alasan) = alasan else {
        return Err(Galat::layanan());
    };

    let (nik_hmac, nik_tersamar) = match Nik::parse(&nik_bersih) {
        Ok(n) => (
            hex_dari(sha256(n.utuh().expose().as_bytes())),
            // `audit_akses.nik_tersamar` is constrained to `^\*{12}[0-9]{4}$`.
            tersamar_audit(&nik_bersih),
        ),
        Err(_) => return Err(Galat::layanan()),
    };

    let audit_id = Uuid::now_v7();
    let baris: Option<BarisEkspor> = sqlx::query_as(
        "SELECT audit_id, berkas, ekspor_checksum, jumlah_record, hasil
           FROM fn_ekspor_log_auditor($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(peminta.pengguna_id)
    .bind(audit_id)
    .bind(&alasan)
    .bind(&nik_hmac)
    .bind(&nik_tersamar)
    .bind(body.dari)
    .bind(body.sampai)
    .bind(Option::<Uuid>::None)
    .bind(&peminta.sumber_ip)
    .bind(peminta.sesi_id.to_string())
    .bind(&nik_bersih)
    .fetch_optional(&s.pool)
    .await
    .map_err(|e| galat_kueri(&e))?;

    let Some(b) = baris else {
        return Err(Galat::layanan());
    };
    if b.hasil.as_deref() == Some("ditolak") {
        return Err(Galat::baru(
            StatusCode::UNPROCESSABLE_ENTITY,
            ProblemKodeAdmin::EksporTerlaluBesar,
        ));
    }
    let (Some(berkas), Some(checksum)) = (b.berkas.clone(), b.ekspor_checksum.clone()) else {
        return Err(Galat::layanan());
    };

    // AC-36: re-derive the digest from the bytes actually sent. If the two ever
    // disagree the audit row describes something the client did not receive,
    // which is worse than a 500 — so it is an error, not a warning.
    let sebenarnya = hex_dari(sha256(&berkas));
    if sebenarnya != checksum {
        tracing::error!(
            audit_id = %audit_id,
            "admin: checksum ekspor tidak cocok dengan byte respons"
        );
        return Err(Galat::layanan());
    }

    // `Content-Disposition` never carries a NIK: the filename is the date range
    // and nothing else (AC-36).
    let nama = format!(
        "log-{}-{}.csv",
        body.dari.format("%Y%m%d"),
        body.sampai.format("%Y%m%d")
    );

    let mut respon = Response::new(Body::from(berkas));
    let h = respon.headers_mut();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/csv; charset=utf-8"),
    );
    h.insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!("attachment; filename=\"{nama}\""))
            .map_err(|_| Galat::layanan())?,
    );
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("none"));
    h.insert(
        HeaderName::from_static("repr-digest"),
        HeaderValue::from_str(&format!("sha-256=:{checksum}:")).map_err(|_| Galat::layanan())?,
    );
    Ok(respon)
}

// ---------------------------------------------------------------------------
// searchAuditAkses (AC-37)
// ---------------------------------------------------------------------------

/// `searchAuditAkses`: at most 31 days, never returns `nik_hmac`, one
/// `lihat_audit` row per call written by `fn_lihat_audit`.
pub(super) async fn search_audit_akses(
    State(s): State<AdminState>,
    method: Method,
    headers: HeaderMap,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    body: Result<Json<AuditCariRequest>, axum::extract::rejection::JsonRejection>,
) -> Result<Response, Galat> {
    let peminta = masuk(&s, &method, &headers, peer, PERAN_AU, "searchAuditAkses").await?;

    // The body is extracted before the handler runs, so a malformed one
    // must not shadow the 401/403 the gate just decided: unwrap it only now.
    let axum::Json(body) = body.map_err(|_| {
        galat_validasi(vec![FieldError {
            field: "body",
            kode: "format",
            batas: None,
        }])
    })?;

    let mut galat = Vec::new();
    rentang_ke_galat(body.dari, body.sampai, &mut galat);
    if body.sampai - body.dari > BATAS_RENTANG_AUDIT {
        // AC-37: the 31-day bound is about scan cost, and the client is told the
        // numeric rule so it can narrow the range rather than guess.
        galat.push(FieldError {
            field: "sampai",
            kode: "rentang",
            batas: Some(31.0),
        });
    }
    if !galat.is_empty() {
        return Err(galat_validasi(galat));
    }

    // The function returns no `nik_hmac` column at all, which is what keeps AC-37
    // true structurally instead of by a filter someone could delete.
    let batas = BATAS_BARIS;
    let baris: Vec<BarisAudit> = sqlx::query_as(
        "SELECT audit_id, pengguna_id, peran, aksi, hasil, alasan, terjadi_at
           FROM fn_lihat_audit($1, $2, $3, $4, $5, $6)
          WHERE ($7::text IS NULL OR aksi = $7)
            AND ($8::text IS NULL OR hasil = $8)
            AND ($9::uuid IS NULL OR pengguna_id = $9)
          ORDER BY terjadi_at DESC, audit_id DESC
          LIMIT $10",
    )
    .bind(peminta.pengguna_id)
    .bind(body.dari)
    .bind(body.sampai)
    .bind(batas)
    .bind(&peminta.sumber_ip)
    .bind(peminta.sesi_id.to_string())
    .bind(&body.aksi)
    .bind(&body.hasil)
    .bind(body.pengguna_id)
    .bind(batas)
    .fetch_all(&s.pool)
    .await
    .map_err(|e| galat_kueri(&e))?;

    let baris = baris
        .into_iter()
        .map(|b| AuditAksesBaris {
            audit_id: b.audit_id,
            dilakukan_pada: b.terjadi_at,
            pengguna_id: b.pengguna_id,
            aksi: b.aksi,
            hasil: b.hasil,
            alasan: b.alasan,
        })
        .collect();

    Ok(Json(AuditAksesPage {
        baris,
        kursor: None,
    })
    .into_response())
}

// ---------------------------------------------------------------------------
// helpers
// ---------------------------------------------------------------------------

fn pelaku_dari(peminta: &Peminta) -> Pelaku {
    Pelaku::baru(peminta.pengguna_id, peminta.peran)
}

/// A driver error is logged, never returned: the text can carry connection
/// details (same rule as `gerbang::galat_db`, which is private to that module).
fn galat_kueri(e: &sqlx::Error) -> Galat {
    tracing::error!(error = %e, "admin: kueri log gagal");
    Galat::layanan()
}

/// Validates `alasan` (AC-34: 10..500 characters after trimming) and returns the
/// trimmed value. Checked here so the caller gets a field error rather than a
/// check violation from the database.
fn periksa_alasan(raw: &str, galat: &mut Vec<FieldError>) -> Option<String> {
    let bersih = raw.trim();
    let panjang = bersih.chars().count() as f64;
    if panjang < ALASAN_MIN || panjang > ALASAN_MAKS {
        galat.push(FieldError {
            field: "alasan",
            kode: "rentang",
            batas: Some(ALASAN_MAKS),
        });
        return None;
    }
    Some(bersih.to_owned())
}

/// `sampai` must be strictly after `dari`; an empty or reversed range is a field
/// error, not an empty page.
fn rentang_ke_galat(dari: DateTime<Utc>, sampai: DateTime<Utc>, galat: &mut Vec<FieldError>) {
    if sampai <= dari {
        galat.push(FieldError {
            field: "sampai",
            kode: "rentang",
            batas: None,
        });
    }
}

fn periksa_rentang(dari: DateTime<Utc>, sampai: DateTime<Utc>) -> Result<(), Galat> {
    let mut galat = Vec::new();
    rentang_ke_galat(dari, sampai, &mut galat);
    if galat.is_empty() {
        Ok(())
    } else {
        Err(galat_validasi(galat))
    }
}

/// The `filter_kueri` recorded with a `cari_log` row: which filters were used,
/// never the values that could identify a person.
fn filter_cari_log(body: &LogCariRequest) -> Value {
    serde_json::json!({
        "dari": body.dari,
        "sampai": body.sampai,
        "kode_transaksi": body.kode_transaksi,
        "queue_id": body.queue_id,
        "client_id": body.client_id,
        "status_akses": body.status_akses,
    })
}

async fn baris_log(s: &AdminState, body: &LogCariRequest) -> Result<Vec<LogBaris>, Galat> {
    let baris: Vec<BarisLog> = sqlx::query_as(
        "SELECT v.nik_tersamar, v.requested_at, coalesce(v.status_akses, '') AS status_akses,
                v.kode_transaksi, v.request_id, v.client_id, c.nama AS client_nama
           FROM v_transaction_log_tersamar v
           JOIN client c ON c.client_id = v.client_id
          WHERE v.requested_at >= $1 AND v.requested_at < $2
            AND ($3::uuid IS NULL OR v.kode_transaksi = $3)
            AND ($4::uuid IS NULL OR v.client_id = $4)
            AND ($5::uuid IS NULL OR v.request_id = $5)
            AND ($6::text IS NULL OR v.status_akses = $6)
          ORDER BY v.requested_at DESC, v.kode_transaksi DESC
           LIMIT $7",
    )
    .bind(body.dari)
    .bind(body.sampai)
    .bind(
        body.kode_transaksi
            .as_deref()
            .and_then(|k| Uuid::parse_str(k).ok()),
    )
    .bind(body.client_id)
    .bind(body.queue_id)
    .bind(&body.status_akses)
    .bind(BATAS_BARIS)
    .fetch_all(&s.pool)
    .await
    .map_err(|e| galat_kueri(&e))?;

    Ok(baris
        .into_iter()
        .map(|b| LogBaris {
            nik: b.nik_tersamar,
            requested_at: b.requested_at,
            status_akses: b.status_akses,
            kode_transaksi: b.kode_transaksi.to_string(),
            queue_id: b.request_id,
            client_id: b.client_id,
            client_nama: b.client_nama,
        })
        .collect())
}

/// Writes one audit row in its own transaction. Used where the read has no other
/// effect to share a transaction with.
async fn tulis_akses(s: &AdminState, akses: AksesBaru) -> Result<(), Galat> {
    let mut tx = s.pool.begin().await.map_err(|e| galat_kueri(&e))?;
    Penulis::baru(&mut tx)
        .catat_akses(akses)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "admin: gagal menulis audit akses");
            Galat::layanan()
        })?;
    tx.commit().await.map_err(|e| galat_kueri(&e))?;
    Ok(())
}

/// The `akses_ditolak` row AC-32 requires when a search smuggles a NIK in.
async fn catat_ditolak(s: &AdminState, peminta: &Peminta, aksi: Aksi) -> Result<(), Galat> {
    let _ = aksi;
    // `chk_filter_wajib` applies to `akses_ditolak` too. The filter says why the
    // refusal happened; it never echoes the NIK-shaped value back into the trail.
    let akses = AksesBaru::ditolak(pelaku_dari(peminta))
        .dengan_filter(serde_json::json!({ "sebab": "potongan_nik" }))
        .dengan_sumber_ip(peminta.sumber_ip.clone())
        .dengan_sesi(peminta.sesi_id.to_string());
    tulis_akses(s, akses).await
}

fn sha256(bytes: &[u8]) -> aws_lc_rs::digest::Digest {
    aws_lc_rs::digest::digest(&aws_lc_rs::digest::SHA256, bytes)
}

fn hex_dari(digest: aws_lc_rs::digest::Digest) -> String {
    use std::fmt::Write as _;
    digest.as_ref().iter().fold(String::new(), |mut s, b| {
        let _ = write!(s, "{b:02x}");
        s
    })
}

/// Row shapes for `query_as`; kept private so the OpenAPI types stay the
/// contract and these stay an implementation detail.
#[derive(sqlx::FromRow)]
struct BarisLog {
    nik_tersamar: String,
    requested_at: DateTime<Utc>,
    status_akses: String,
    kode_transaksi: Uuid,
    request_id: Uuid,
    client_id: Uuid,
    client_nama: String,
}

#[derive(sqlx::FromRow)]
struct BarisNik {
    kode_transaksi: Uuid,
    request_id: Uuid,
    client_id: Uuid,
    requested_at: DateTime<Utc>,
    status_akses: Option<String>,
    client_nama: String,
}

#[derive(sqlx::FromRow)]
struct BarisPayload {
    request_id: Uuid,
    requested_at: DateTime<Utc>,
    status_akses: String,
    #[allow(dead_code)]
    log_mode: Option<String>,
    request_payload: Option<Value>,
    response_payload: Option<Value>,
    provider_response_code: Option<String>,
}

#[derive(sqlx::FromRow)]
struct BarisEkspor {
    #[allow(dead_code)]
    audit_id: Uuid,
    berkas: Option<Vec<u8>>,
    ekspor_checksum: Option<String>,
    #[allow(dead_code)]
    jumlah_record: i32,
    hasil: Option<String>,
}

#[derive(sqlx::FromRow)]
struct BarisAudit {
    audit_id: Uuid,
    pengguna_id: Uuid,
    #[allow(dead_code)]
    peran: String,
    aksi: String,
    hasil: String,
    alasan: Option<String>,
    terjadi_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dari_sampai() -> (DateTime<Utc>, DateTime<Utc>) {
        let dari = Utc::now();
        (dari, dari + Duration::hours(1))
    }

    #[test]
    fn alasan_di_luar_batas_ditolak() {
        let mut galat = Vec::new();
        assert!(periksa_alasan("pendek", &mut galat).is_none());
        assert_eq!(galat.len(), 1);
        assert_eq!(galat[0].field, "alasan");

        let mut galat = Vec::new();
        let panjang = "a".repeat(501);
        assert!(periksa_alasan(&panjang, &mut galat).is_none());
        assert_eq!(galat.len(), 1);

        let mut galat = Vec::new();
        assert!(periksa_alasan(&"a".repeat(10), &mut galat).is_some());
        assert!(galat.is_empty());
        assert!(periksa_alasan(&"a".repeat(500), &mut galat).is_some());
    }

    #[test]
    fn alasan_dipangkas_sebelum_dihitung() {
        let mut galat = Vec::new();
        // Ten characters' worth once the padding is gone.
        let isi = format!("   {}   ", "a".repeat(10));
        assert_eq!(
            periksa_alasan(&isi, &mut galat).as_deref(),
            Some("aaaaaaaaaa")
        );
        assert!(galat.is_empty());
    }

    #[test]
    fn rentang_terbalik_atau_kosong_ditolak() {
        let (dari, sampai) = dari_sampai();
        let mut galat = Vec::new();
        rentang_ke_galat(dari, dari, &mut galat);
        assert_eq!(galat.len(), 1);
        assert_eq!(galat[0].field, "sampai");

        let mut galat = Vec::new();
        rentang_ke_galat(sampai, dari, &mut galat);
        assert_eq!(galat.len(), 1);

        let mut galat = Vec::new();
        rentang_ke_galat(dari, sampai, &mut galat);
        assert!(galat.is_empty());
    }

    fn cari(kode: Option<&str>, status: Option<&str>, kursor: Option<&str>) -> LogCariRequest {
        let (dari, sampai) = dari_sampai();
        LogCariRequest {
            dari,
            sampai,
            kode_transaksi: kode.map(str::to_owned),
            queue_id: None,
            client_id: None,
            status_akses: status.map(str::to_owned),
            kursor: kursor.map(str::to_owned),
        }
    }

    #[test]
    fn nik_di_parameter_mana_pun_terdeteksi() {
        // AC-32: six consecutive digits in any field is a NIK fragment.
        assert!(parameter_ber_nik(&cari(Some("3201234567890001"), None, None)).is_some());
        assert!(parameter_ber_nik(&cari(None, Some("klien 3201234567890001"), None)).is_some());
        assert!(parameter_ber_nik(&cari(None, None, Some("000123"))).is_some());
        // Ordinary filters pass through.
        assert!(parameter_ber_nik(&cari(None, Some("ditolak"), None)).is_none());
        assert!(parameter_ber_nik(&cari(None, None, Some("12345"))).is_none());
        assert!(parameter_ber_nik(&cari(None, None, None)).is_none());
    }
}
