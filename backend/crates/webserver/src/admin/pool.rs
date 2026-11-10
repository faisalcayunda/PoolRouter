//! CRUD upstream pool dan downstream pool, plus quota tracker (ARC-53; AC-16..AC-18).
//!
//! Aturan yang terlihat di lapisan ini:
//! - Semua operasi tulis adalah platform admin saja, lewat [`masuk`] dan guard
//!   MFA, sama dengan `klien.rs`. `getQuotaTracker` juga terbuka untuk operator.
//! - `PUT` adalah penggantian penuh: field yang hilang adalah 422, bukan nilai
//!   bawaan server (AC-16). `POST` mengisi nilai bawaan kontrak untuk field yang
//!   punya `default`.
//! - Kolom `aktif` tidak pernah disentuh di sini. Penonaktifan hanya lewat jalur
//!   K-20 (`k20::K20Service`), dan grant `pr_admin_api` memang tidak memberi
//!   UPDATE atas kolom itu.
//! - Field kontrak yang tidak punya kolom (`rate_limit_per_jam`, `jeda_min`,
//!   `jeda_maks`, `ambang_berdampingan`) disimpan di `parameter_dispatch`, dalam
//!   bentuk yang dibaca dispatcher (`ParameterDispatch`, ARC-70) dan dalam bentuk
//!   aslinya di `konfigurasi_admin` supaya GET mengembalikan persis yang di-PUT.
//! - Perubahan `reset_kuota_*` berlaku di akhir periode berjalan (KP-02): nilainya
//!   masuk kolom `*_berikut` dengan `reset_kuota_berlaku_mulai` = akhir periode,
//!   dihitung dengan `periode_dari_baris` yang sama dengan rebuild kuota.
//! - Trigger alokasi (`tg_alokasi_dalam_kuota`) adalah constraint trigger yang
//!   ditunda sampai commit. Handler memanggil `fn_alokasi_kelebihan` sebelum
//!   commit supaya kelebihan alokasi menjadi 422, bukan galat commit.
//! - Quota tracker membaca `upstream_attempt_daily` dan view
//!   `v_transaction_log_tersamar`, tidak pernah Redis (KP-07).

use std::collections::BTreeMap;
use std::net::SocketAddr;

use axum::Json;
use axum::extract::rejection::JsonRejection;
use axum::extract::{ConnectInfo, Path, Query, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Duration, DurationRound, NaiveTime, Utc};
use poolrouter_core::audit::{
    AksiKonfigurasi, Alasan, EntitasKonfigurasi, KonfigurasiBaru, Penulis, Peran,
};
use poolrouter_core::kuota::pemulihan::nilai_counter;
use poolrouter_core::kuota::sumber_pg::periode_dari_baris;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

use super::gerbang::{Peminta, masuk, periksa_guard_mfa};
use super::k20;
use super::sesi::{AdminState, Galat};
use crate::api::admin::{
    DownstreamPool, DownstreamPoolConfig, KuotaTracker, UpstreamPool, UpstreamPoolConfig,
};
use crate::api::enums::{AlokasiJenis, JendelaKuota, SifatAlokasi};
use crate::api::problem::ValidasiErrorAdmin;

/// Konfigurasi pool hanya platform admin (DESIGN.md §3).
const PERAN_PA: [Peran; 1] = [Peran::PlatformAdmin];
/// Quota tracker: platform admin dan operator, baca saja.
const PERAN_KUOTA: [Peran; 2] = [Peran::PlatformAdmin, Peran::Operator];

/// Zona yang dikenal `zona_tetap` di `core::kuota::sumber_pg`. Zona IANA lain
/// lolos trigger database tetapi membuat rebuild kuota gagal untuk semua pool,
/// jadi ditolak saat simpan.
const ZONA_DIKENAL: [&str; 5] = [
    "Asia/Jakarta",
    "Asia/Pontianak",
    "Asia/Makassar",
    "Asia/Jayapura",
    "UTC",
];

const NAMA_MAX: usize = 200;
/// Batas `numeric(12,2)` kolom `downstream_pool_rute.alokasi_nilai`.
const ALOKASI_NILAI_MAX: f64 = 9_999_999_999.99;

/// Field `UpstreamPoolConfig` tanpa `default` di kontrak: wajib di POST dan PUT.
const UPSTREAM_WAJIB_SELALU: [&str; 7] = [
    "provider_id",
    "kuota_harian",
    "rate_limit_per_jam",
    "porsi_sync",
    "jeda_min",
    "jeda_maks",
    "ambang_berdampingan",
];
/// Field yang punya `default` di kontrak: opsional di POST, wajib di PUT.
const UPSTREAM_BERBAWAAN: [&str; 9] = [
    "margin_kuota",
    "batas_percobaan",
    "retry_backoff_base_detik",
    "retry_backoff_cap_detik",
    "retry_rasio",
    "retry_min_per_jam",
    "jendela_kuota",
    "reset_kuota_jam",
    "reset_kuota_zona",
];

fn bawaan_upstream() -> Vec<(&'static str, Value)> {
    vec![
        ("margin_kuota", json!(0.01)),
        ("batas_percobaan", json!(3)),
        ("retry_backoff_base_detik", json!(5)),
        ("retry_backoff_cap_detik", json!(300)),
        ("retry_rasio", json!(0.1)),
        ("retry_min_per_jam", json!(20)),
        ("jendela_kuota", json!("bergulir_24j")),
        ("reset_kuota_jam", json!(0)),
        ("reset_kuota_zona", json!("Asia/Jakarta")),
    ]
}

const DOWNSTREAM_WAJIB: [&str; 2] = ["nama", "upstream_pool_ids"];
const DOWNSTREAM_OPSIONAL: [&str; 6] = [
    "ttl_antrean_detik",
    "alokasi_jenis",
    "alokasi_nilai",
    "sifat",
    "boleh_pinjam",
    "alasan",
];

// ---------------------------------------------------------------------------
// Upstream pool: baca
// ---------------------------------------------------------------------------

/// Kolom entitas upstream pool. Jam dan zona reset yang dilaporkan adalah aturan
/// terakhir yang dikonfigurasi (yang terjadwal bila ada), supaya GET lalu PUT
/// tanpa perubahan tidak membatalkan perubahan yang sedang dijadwalkan (KP-02).
macro_rules! select_upstream {
    ($ekor:literal) => {
        concat!(
            "SELECT upstream_pool_id, provider_id, aktif, kuota_harian::bigint AS kuota_harian, \
             porsi_sync_maks::float8 AS porsi_sync, margin_kuota::float8 AS margin_kuota, \
             batas_percobaan::bigint AS batas_percobaan, \
             retry_backoff_base_detik::bigint AS retry_backoff_base_detik, \
             retry_backoff_cap_detik::bigint AS retry_backoff_cap_detik, \
             retry_rasio::float8 AS retry_rasio, retry_min_per_jam::bigint AS retry_min_per_jam, \
             jendela_kuota, \
             extract(hour FROM coalesce(reset_kuota_jam_berikut, reset_kuota_jam))::bigint AS reset_jam, \
             coalesce(reset_kuota_zona_berikut, reset_kuota_zona)::text AS reset_zona, \
             parameter_dispatch \
             FROM upstream_pool",
            $ekor
        )
    };
}

#[derive(sqlx::FromRow)]
struct BarisUpstream {
    upstream_pool_id: Uuid,
    provider_id: Uuid,
    aktif: bool,
    kuota_harian: i64,
    porsi_sync: f64,
    margin_kuota: f64,
    batas_percobaan: i64,
    retry_backoff_base_detik: i64,
    retry_backoff_cap_detik: i64,
    retry_rasio: f64,
    retry_min_per_jam: i64,
    jendela_kuota: String,
    reset_jam: i64,
    reset_zona: String,
    parameter_dispatch: sqlx::types::Json<Value>,
}

impl BarisUpstream {
    fn ke_entitas(&self) -> UpstreamPool {
        let admin = KonfigurasiAdmin::dari_parameter(&self.parameter_dispatch.0);
        UpstreamPool {
            upstream_pool_id: self.upstream_pool_id,
            provider_id: self.provider_id,
            kuota_harian: self.kuota_harian,
            rate_limit_per_jam: admin.rate_limit_per_jam,
            porsi_sync: self.porsi_sync,
            margin_kuota: self.margin_kuota,
            batas_percobaan: self.batas_percobaan,
            retry_backoff_base_detik: self.retry_backoff_base_detik,
            retry_backoff_cap_detik: self.retry_backoff_cap_detik,
            retry_rasio: self.retry_rasio,
            retry_min_per_jam: self.retry_min_per_jam,
            jendela_kuota: jendela_dari_kolom(&self.jendela_kuota),
            reset_kuota_jam: self.reset_jam,
            reset_kuota_zona: self.reset_zona.clone(),
            jeda_min: admin.jeda_min,
            jeda_maks: admin.jeda_maks,
            ambang_berdampingan: admin.ambang_berdampingan,
            aktif: self.aktif,
        }
    }
}

/// Empat field kontrak yang tidak punya kolom sendiri.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct KonfigurasiAdmin {
    rate_limit_per_jam: i64,
    jeda_min: i64,
    jeda_maks: i64,
    ambang_berdampingan: i64,
}

impl KonfigurasiAdmin {
    fn dari_body(b: &UpstreamPoolConfig) -> Self {
        Self {
            rate_limit_per_jam: b.rate_limit_per_jam,
            jeda_min: b.jeda_min,
            jeda_maks: b.jeda_maks,
            ambang_berdampingan: b.ambang_berdampingan,
        }
    }

    /// Dibaca dari `konfigurasi_admin`; pool yang dibuat di luar admin API (seed,
    /// fixture) diturunkan dari kunci dispatcher supaya GET tidak gagal.
    fn dari_parameter(p: &Value) -> Self {
        let angka = |v: Option<&Value>| v.and_then(Value::as_i64);
        if let Some(a) = p.get("konfigurasi_admin")
            && let (Some(r), Some(jmin), Some(jmaks), Some(amb)) = (
                angka(a.get("rate_limit_per_jam")),
                angka(a.get("jeda_min")),
                angka(a.get("jeda_maks")),
                angka(a.get("ambang_berdampingan")),
            )
        {
            return Self {
                rate_limit_per_jam: r,
                jeda_min: jmin,
                jeda_maks: jmaks,
                ambang_berdampingan: amb,
            };
        }
        // Jendela terketat menentukan laju per jam.
        let rate_limit_per_jam = p
            .get("rate")
            .and_then(Value::as_object)
            .and_then(|o| {
                o.values()
                    .filter_map(|w| angka(w.get("t_mikro")))
                    .filter(|t| *t > 0)
                    .map(|t| 3_600_000_000 / t)
                    .min()
            })
            .unwrap_or(1)
            .max(1);
        let jeda_maks = angka(p.get("jeda_acak_maks_mikro")).map_or(0, |m| m / 1_000);
        Self {
            rate_limit_per_jam,
            jeda_min: angka(p.get("jeda_acak_min_mikro")).map_or(0, |m| m / 1_000),
            jeda_maks,
            ambang_berdampingan: angka(p.get("maks_konkurensi"))
                .unwrap_or(poolrouter_core::kuota::sumber_pg::MAKS_KONKURENSI_BAWAAN),
        }
    }

    /// Gabungkan ke `parameter_dispatch` yang ada. Kunci lain (mis.
    /// `timeout_total_upstream_detik`) dibiarkan; `rate` diganti utuh (ARC-233).
    ///
    /// Pemetaan ke kunci dispatcher (ARC-70):
    /// - `rate` = `{ tph }` saja: satu token per `3_600_000_000 / rate_limit_per_jam`
    ///   µs, burst 1. Jendela `tps`/`tpm` lama dibuang supaya tidak diam-diam
    ///   membatasi di bawah `rate_limit_per_jam` yang baru (sama dengan POST);
    /// - `jeda_acak_min_mikro` = `jeda_min` ms dalam µs;
    /// - `jeda_acak_maks_mikro` = `jeda_maks` ms dalam µs (dispatcher memotongnya ke
    ///   satu slot token);
    /// - `maks_konkurensi` = `ambang_berdampingan` (ambang konkurensi, US-22).
    fn gabung_ke(self, lama: &Value) -> Value {
        let mut obj = lama.as_object().cloned().unwrap_or_default();
        obj.insert(
            "rate".to_owned(),
            json!({ "tph": { "t_mikro": 3_600_000_000_i64 / self.rate_limit_per_jam.max(1), "burst": 1 } }),
        );
        obj.insert(
            "jeda_acak_min_mikro".to_owned(),
            json!(self.jeda_min.saturating_mul(1_000)),
        );
        obj.insert(
            "jeda_acak_maks_mikro".to_owned(),
            json!(self.jeda_maks.saturating_mul(1_000)),
        );
        obj.insert(
            "maks_konkurensi".to_owned(),
            json!(self.ambang_berdampingan),
        );
        obj.insert(
            "konfigurasi_admin".to_owned(),
            json!({
                "rate_limit_per_jam": self.rate_limit_per_jam,
                "jeda_min": self.jeda_min,
                "jeda_maks": self.jeda_maks,
                "ambang_berdampingan": self.ambang_berdampingan,
            }),
        );
        Value::Object(obj)
    }
}

/// Jendela `rate` selain `tph` yang akan dibuang PUT (ARC-233).
fn jendela_rate_dibuang(lama: &Value) -> Vec<&str> {
    lama.get("rate")
        .and_then(Value::as_object)
        .map(|o| {
            o.keys()
                .map(String::as_str)
                .filter(|k| *k != "tph")
                .collect()
        })
        .unwrap_or_default()
}

fn jendela_dari_kolom(k: &str) -> JendelaKuota {
    if k == "kalender" {
        JendelaKuota::Kalender
    } else {
        JendelaKuota::Bergulir24j
    }
}

const fn jendela_ke_kolom(j: JendelaKuota) -> &'static str {
    match j {
        JendelaKuota::Bergulir24j => "bergulir_24j",
        JendelaKuota::Kalender => "kalender",
    }
}

/// `listUpstreamPools` (PA).
pub(super) async fn list_upstream_pools(
    State(s): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Response, Galat> {
    let peminta = masuk(
        &s,
        &Method::GET,
        &headers,
        peer,
        &PERAN_PA,
        "listUpstreamPools",
    )
    .await?;
    periksa_guard_mfa(&peminta)?;
    let baris: Vec<BarisUpstream> =
        sqlx::query_as(select_upstream!(" ORDER BY dibuat_at, upstream_pool_id"))
            .fetch_all(&s.pool)
            .await
            .map_err(|e| galat_db(&e))?;
    let daftar: Vec<UpstreamPool> = baris.iter().map(BarisUpstream::ke_entitas).collect();
    Ok((StatusCode::OK, Json(daftar)).into_response())
}

/// `getUpstreamPool` (PA).
pub(super) async fn get_upstream_pool(
    State(s): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, Galat> {
    let peminta = masuk(
        &s,
        &Method::GET,
        &headers,
        peer,
        &PERAN_PA,
        "getUpstreamPool",
    )
    .await?;
    periksa_guard_mfa(&peminta)?;
    let entitas = baca_upstream(&s.pool, id)
        .await?
        .ok_or_else(Galat::tidak_ditemukan)?;
    Ok((StatusCode::OK, Json(entitas)).into_response())
}

pub(super) async fn baca_upstream<'e, E>(
    eksekutor: E,
    id: Uuid,
) -> Result<Option<UpstreamPool>, Galat>
where
    E: sqlx::PgExecutor<'e>,
{
    let baris: Option<BarisUpstream> =
        sqlx::query_as(select_upstream!(" WHERE upstream_pool_id = $1"))
            .bind(id)
            .fetch_optional(eksekutor)
            .await
            .map_err(|e| galat_db(&e))?;
    Ok(baris.as_ref().map(BarisUpstream::ke_entitas))
}

// ---------------------------------------------------------------------------
// Upstream pool: tulis
// ---------------------------------------------------------------------------

/// `createUpstreamPool` (PA). Satu transaksi: baris pool dan baris audit.
pub(super) async fn create_upstream_pool(
    State(s): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Result<Response, Galat> {
    let peminta = masuk(
        &s,
        &Method::POST,
        &headers,
        peer,
        &PERAN_PA,
        "createUpstreamPool",
    )
    .await?;
    periksa_guard_mfa(&peminta)?;
    let opsional: Vec<&str> = UPSTREAM_BERBAWAAN
        .iter()
        .copied()
        .chain(["alasan"])
        .collect();
    let body: UpstreamPoolConfig =
        urai_body(body, &UPSTREAM_WAJIB_SELALU, &opsional, &bawaan_upstream())?;

    let mut galat = periksa_upstream(&body, false);
    let mut tx = s.pool.begin().await.map_err(|e| galat_db(&e))?;
    if !provider_ada(&mut tx, body.provider_id).await? {
        galat.push(galat_field("provider_id", "tidak_ada", None));
    }
    if !galat.is_empty() {
        return Err(Galat::validasi(galat));
    }

    let id = Uuid::now_v7();
    let parameter = KonfigurasiAdmin::dari_body(&body).gabung_ke(&json!({}));
    sqlx::query(
        "INSERT INTO upstream_pool (upstream_pool_id, provider_id, nama, aktif, kuota_harian, \
         porsi_sync_maks, parameter_dispatch, margin_kuota, batas_percobaan, \
         retry_backoff_base_detik, retry_backoff_cap_detik, retry_rasio, retry_min_per_jam, \
         jendela_kuota, reset_kuota_jam, reset_kuota_zona) \
         VALUES ($1, $2, $3, true, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)",
    )
    .bind(id)
    .bind(body.provider_id)
    // Kontrak tidak punya `nama` untuk upstream pool, kolomnya NOT NULL UNIQUE.
    .bind(format!("upstream-{id}"))
    .bind(body.kuota_harian as i32)
    .bind(body.porsi_sync)
    .bind(sqlx::types::Json(&parameter))
    .bind(body.margin_kuota)
    .bind(body.batas_percobaan as i32)
    .bind(body.retry_backoff_base_detik as i32)
    .bind(body.retry_backoff_cap_detik as i32)
    .bind(body.retry_rasio)
    .bind(body.retry_min_per_jam as i32)
    .bind(jendela_ke_kolom(body.jendela_kuota))
    .bind(jam_reset(body.reset_kuota_jam))
    .bind(&body.reset_kuota_zona)
    .execute(&mut *tx)
    .await
    .map_err(|e| galat_tulis(&e, "provider_id"))?;

    let entitas = baca_upstream(&mut *tx, id)
        .await?
        .ok_or_else(Galat::layanan)?;
    catat(
        &mut tx,
        &peminta,
        EntitasKonfigurasi::UpstreamPool,
        AksiKonfigurasi::Buat,
        id,
        None,
        Some(snapshot(&entitas)),
        body.alasan
            .clone()
            .unwrap_or_else(|| "createUpstreamPool".to_owned()),
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| galat_tulis(&e, "kuota_harian"))?;
    Ok((StatusCode::CREATED, Json(entitas)).into_response())
}

/// `updateUpstreamPool` (PA). Penggantian penuh; `alasan` wajib; perubahan
/// `reset_kuota_*` dijadwalkan ke akhir periode berjalan (KP-02).
pub(super) async fn update_upstream_pool(
    State(s): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    body: Result<Json<Value>, JsonRejection>,
) -> Result<Response, Galat> {
    let peminta = masuk(
        &s,
        &Method::PUT,
        &headers,
        peer,
        &PERAN_PA,
        "updateUpstreamPool",
    )
    .await?;
    periksa_guard_mfa(&peminta)?;
    let wajib: Vec<&str> = UPSTREAM_WAJIB_SELALU
        .iter()
        .chain(UPSTREAM_BERBAWAAN.iter())
        .copied()
        .collect();
    let body: UpstreamPoolConfig = urai_body(body, &wajib, &["alasan"], &[])?;

    let mut galat = periksa_upstream(&body, true);
    let mut tx = s.pool.begin().await.map_err(|e| galat_db(&e))?;
    // Kunci baris dulu supaya dua update bersamaan tidak menjadwalkan reset
    // dengan dasar periode yang sama-sama basi.
    let lama: Option<BarisUpstream> =
        sqlx::query_as(select_upstream!(" WHERE upstream_pool_id = $1 FOR UPDATE"))
            .bind(id)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| galat_db(&e))?;
    let Some(lama) = lama else {
        return Err(Galat::tidak_ditemukan());
    };
    if !provider_ada(&mut tx, body.provider_id).await? {
        galat.push(galat_field("provider_id", "tidak_ada", None));
    }
    if !galat.is_empty() {
        return Err(Galat::validasi(galat));
    }
    let entitas_lama = lama.ke_entitas();
    let sekarang = (s.jam)();

    jadwalkan_reset(&mut tx, id, &body, sekarang).await?;

    let dibuang = jendela_rate_dibuang(&lama.parameter_dispatch.0);
    if !dibuang.is_empty() {
        tracing::warn!(
            upstream_pool_id = %id,
            jendela = ?dibuang,
            "admin: PUT upstream pool membuang jendela rate lama; rate hanya tph"
        );
    }
    let parameter = KonfigurasiAdmin::dari_body(&body).gabung_ke(&lama.parameter_dispatch.0);
    sqlx::query(
        "UPDATE upstream_pool SET provider_id = $2, kuota_harian = $3, porsi_sync_maks = $4, \
         parameter_dispatch = $5, margin_kuota = $6, batas_percobaan = $7, \
         retry_backoff_base_detik = $8, retry_backoff_cap_detik = $9, retry_rasio = $10, \
         retry_min_per_jam = $11, jendela_kuota = $12, diubah_at = now() \
         WHERE upstream_pool_id = $1",
    )
    .bind(id)
    .bind(body.provider_id)
    .bind(body.kuota_harian as i32)
    .bind(body.porsi_sync)
    .bind(sqlx::types::Json(&parameter))
    .bind(body.margin_kuota)
    .bind(body.batas_percobaan as i32)
    .bind(body.retry_backoff_base_detik as i32)
    .bind(body.retry_backoff_cap_detik as i32)
    .bind(body.retry_rasio)
    .bind(body.retry_min_per_jam as i32)
    .bind(jendela_ke_kolom(body.jendela_kuota))
    .execute(&mut *tx)
    .await
    .map_err(|e| galat_tulis(&e, "provider_id"))?;

    // Kuota yang turun bisa membuat alokasi rute yang sudah ada melebihi kuota
    // efektif; laporkan sebelum commit (AC-17).
    periksa_kelebihan(&mut tx, id, "kuota_harian").await?;

    let entitas = baca_upstream(&mut *tx, id)
        .await?
        .ok_or_else(Galat::layanan)?;
    catat(
        &mut tx,
        &peminta,
        EntitasKonfigurasi::UpstreamPool,
        AksiKonfigurasi::Ubah,
        id,
        Some(snapshot(&entitas_lama)),
        Some(snapshot(&entitas)),
        body.alasan.clone().unwrap_or_default(),
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| galat_tulis(&e, "kuota_harian"))?;
    Ok((StatusCode::OK, Json(entitas)).into_response())
}

/// KP-02 di dalam transaksi pemanggil, setelah baris dikunci:
/// 1. perubahan terjadwal yang `berlaku_mulai`-nya sudah lewat dinaikkan menjadi
///    aturan aktif;
/// 2. permintaan sama dengan aturan aktif: jadwal yang tertunda dibatalkan;
/// 3. permintaan sama dengan jadwal yang tertunda: tidak ada yang berubah;
/// 4. selain itu: jadwal baru pada akhir periode berjalan.
async fn jadwalkan_reset(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    id: Uuid,
    body: &UpstreamPoolConfig,
    sekarang: DateTime<Utc>,
) -> Result<(), Galat> {
    sqlx::query(
        "UPDATE upstream_pool SET reset_kuota_jam = reset_kuota_jam_berikut, \
         reset_kuota_zona = reset_kuota_zona_berikut, reset_kuota_jam_berikut = NULL, \
         reset_kuota_zona_berikut = NULL, reset_kuota_berlaku_mulai = NULL \
         WHERE upstream_pool_id = $1 AND reset_kuota_berlaku_mulai IS NOT NULL \
           AND reset_kuota_berlaku_mulai <= $2",
    )
    .bind(id)
    .bind(sekarang)
    .execute(&mut **tx)
    .await
    .map_err(|e| galat_db(&e))?;

    let baris = sqlx::query(
        "SELECT reset_kuota_jam, reset_kuota_zona::text AS reset_kuota_zona, \
         reset_kuota_berlaku_mulai, reset_kuota_jam_berikut, \
         reset_kuota_zona_berikut::text AS reset_kuota_zona_berikut \
         FROM upstream_pool WHERE upstream_pool_id = $1",
    )
    .bind(id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| galat_db(&e))?;
    let jam_aktif: NaiveTime = baris.try_get("reset_kuota_jam").map_err(|e| galat_db(&e))?;
    let zona_aktif: String = baris
        .try_get("reset_kuota_zona")
        .map_err(|e| galat_db(&e))?;
    let jam_berikut: Option<NaiveTime> = baris
        .try_get("reset_kuota_jam_berikut")
        .map_err(|e| galat_db(&e))?;
    let zona_berikut: Option<String> = baris
        .try_get("reset_kuota_zona_berikut")
        .map_err(|e| galat_db(&e))?;

    let jam_baru = jam_reset(body.reset_kuota_jam);
    let zona_baru = body.reset_kuota_zona.as_str();

    if jam_baru == jam_aktif && zona_baru == zona_aktif {
        sqlx::query(
            "UPDATE upstream_pool SET reset_kuota_jam_berikut = NULL, \
             reset_kuota_zona_berikut = NULL, reset_kuota_berlaku_mulai = NULL \
             WHERE upstream_pool_id = $1",
        )
        .bind(id)
        .execute(&mut **tx)
        .await
        .map_err(|e| galat_db(&e))?;
        return Ok(());
    }
    if jam_berikut == Some(jam_baru) && zona_berikut.as_deref() == Some(zona_baru) {
        return Ok(());
    }

    let periode = periode_dari_baris(&baris, sekarang).map_err(|e| {
        tracing::error!(upstream_pool_id = %id, error = %e, "admin: periode upstream pool tidak bisa dihitung");
        Galat::layanan()
    })?;
    sqlx::query(
        "UPDATE upstream_pool SET reset_kuota_jam_berikut = $2, reset_kuota_zona_berikut = $3, \
         reset_kuota_berlaku_mulai = $4 WHERE upstream_pool_id = $1",
    )
    .bind(id)
    .bind(jam_baru)
    .bind(zona_baru)
    .bind(periode.selesai)
    .execute(&mut **tx)
    .await
    .map_err(|e| galat_tulis(&e, "reset_kuota_zona"))?;
    Ok(())
}

/// Semua pemeriksaan tanpa I/O untuk body upstream pool, dikumpulkan sekaligus
/// (AC-7, AC-16).
fn periksa_upstream(body: &UpstreamPoolConfig, update: bool) -> Vec<ValidasiErrorAdmin> {
    let mut galat: Vec<ValidasiErrorAdmin> = match body.validate() {
        Ok(()) => Vec::new(),
        Err(e) => e
            .into_iter()
            .map(|f| galat_field(f.field, f.kode, f.batas))
            .collect(),
    };
    let sudah = |g: &[ValidasiErrorAdmin], f: &str| g.iter().any(|e| e.field == f);
    // Kolom `integer`: angka di atas i32 tidak boleh menjadi 500.
    for (field, nilai) in [
        ("kuota_harian", body.kuota_harian),
        ("rate_limit_per_jam", body.rate_limit_per_jam),
        ("ambang_berdampingan", body.ambang_berdampingan),
    ] {
        if nilai > i64::from(i32::MAX) && !sudah(&galat, field) {
            galat.push(galat_field(field, "rentang", Some(f64::from(i32::MAX))));
        }
    }
    if !ZONA_DIKENAL.contains(&body.reset_kuota_zona.as_str()) {
        galat.push(galat_field("reset_kuota_zona", "format", None));
    }
    let alasan_wajib = update || body.jendela_kuota == JendelaKuota::Kalender;
    if let Some(e) = periksa_alasan(body.alasan.as_deref(), alasan_wajib) {
        galat.push(e);
    }
    galat
}

fn jam_reset(jam: i64) -> NaiveTime {
    NaiveTime::from_hms_opt(u32::try_from(jam.clamp(0, 23)).unwrap_or(0), 0, 0)
        .unwrap_or(NaiveTime::MIN)
}

async fn provider_ada(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    id: Uuid,
) -> Result<bool, Galat> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM provider WHERE provider_id = $1)")
        .bind(id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| galat_db(&e))
}

// ---------------------------------------------------------------------------
// Downstream pool
// ---------------------------------------------------------------------------

macro_rules! select_downstream {
    ($ekor:literal) => {
        concat!(
            "SELECT d.downstream_pool_id, d.nama, d.ttl_antrean_detik::bigint AS ttl_antrean_detik, \
             d.prioritas::bigint AS prioritas, d.aktif, r.upstream_pool_id, r.alokasi_jenis, \
             r.alokasi_nilai::float8 AS alokasi_nilai, r.sifat, r.boleh_pinjam \
             FROM downstream_pool d \
             LEFT JOIN downstream_pool_rute r ON r.downstream_pool_id = d.downstream_pool_id",
            $ekor
        )
    };
}

#[derive(sqlx::FromRow)]
struct BarisDownstream {
    downstream_pool_id: Uuid,
    nama: String,
    ttl_antrean_detik: i64,
    prioritas: i64,
    aktif: bool,
    upstream_pool_id: Option<Uuid>,
    alokasi_jenis: Option<String>,
    alokasi_nilai: Option<f64>,
    sifat: Option<String>,
    boleh_pinjam: Option<bool>,
}

/// Satukan baris join per downstream pool, urutan pertama dipertahankan. F0/F1
/// memakai satu rute per downstream pool; alokasi dibaca dari rute pertama.
fn kelompokkan(baris: Vec<BarisDownstream>) -> Vec<DownstreamPool> {
    let mut urutan: Vec<Uuid> = Vec::new();
    let mut per_id: BTreeMap<Uuid, DownstreamPool> = BTreeMap::new();
    for b in baris {
        let entri = per_id.entry(b.downstream_pool_id).or_insert_with(|| {
            urutan.push(b.downstream_pool_id);
            DownstreamPool {
                downstream_pool_id: b.downstream_pool_id,
                nama: b.nama.clone(),
                ttl_antrean_detik: b.ttl_antrean_detik,
                upstream_pool_ids: Vec::new(),
                alokasi_jenis: b.alokasi_jenis.as_deref().and_then(alokasi_dari_kolom),
                alokasi_nilai: b.alokasi_nilai,
                sifat: b.sifat.as_deref().and_then(sifat_dari_kolom),
                boleh_pinjam: b.boleh_pinjam.unwrap_or(true),
                prioritas: b.prioritas,
                aktif: b.aktif,
            }
        });
        if let Some(u) = b.upstream_pool_id {
            entri.upstream_pool_ids.push(u);
        }
    }
    urutan
        .into_iter()
        .filter_map(|id| per_id.remove(&id))
        .collect()
}

fn alokasi_dari_kolom(k: &str) -> Option<AlokasiJenis> {
    match k {
        "exact" => Some(AlokasiJenis::Exact),
        "persen" => Some(AlokasiJenis::Persen),
        _ => None,
    }
}

fn sifat_dari_kolom(k: &str) -> Option<SifatAlokasi> {
    match k {
        "reserved" => Some(SifatAlokasi::Reserved),
        "shareable" => Some(SifatAlokasi::Shareable),
        _ => None,
    }
}

/// `listDownstreamPools` (PA).
pub(super) async fn list_downstream_pools(
    State(s): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
) -> Result<Response, Galat> {
    let peminta = masuk(
        &s,
        &Method::GET,
        &headers,
        peer,
        &PERAN_PA,
        "listDownstreamPools",
    )
    .await?;
    periksa_guard_mfa(&peminta)?;
    let baris: Vec<BarisDownstream> =
        sqlx::query_as(select_downstream!(" ORDER BY d.nama, r.upstream_pool_id"))
            .fetch_all(&s.pool)
            .await
            .map_err(|e| galat_db(&e))?;
    Ok((StatusCode::OK, Json(kelompokkan(baris))).into_response())
}

/// `getDownstreamPool` (PA).
pub(super) async fn get_downstream_pool(
    State(s): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
) -> Result<Response, Galat> {
    let peminta = masuk(
        &s,
        &Method::GET,
        &headers,
        peer,
        &PERAN_PA,
        "getDownstreamPool",
    )
    .await?;
    periksa_guard_mfa(&peminta)?;
    let entitas = baca_downstream(&s.pool, id)
        .await?
        .ok_or_else(Galat::tidak_ditemukan)?;
    Ok((StatusCode::OK, Json(entitas)).into_response())
}

pub(super) async fn baca_downstream<'e, E>(
    eksekutor: E,
    id: Uuid,
) -> Result<Option<DownstreamPool>, Galat>
where
    E: sqlx::PgExecutor<'e>,
{
    let baris: Vec<BarisDownstream> = sqlx::query_as(select_downstream!(
        " WHERE d.downstream_pool_id = $1 ORDER BY r.upstream_pool_id"
    ))
    .bind(id)
    .fetch_all(eksekutor)
    .await
    .map_err(|e| galat_db(&e))?;
    Ok(kelompokkan(baris).into_iter().next())
}

/// Alokasi rute yang sudah diperiksa, siap ditulis ke `downstream_pool_rute`.
#[derive(Debug, Clone, Copy, PartialEq)]
struct AlokasiRute {
    jenis: Option<AlokasiJenis>,
    nilai: Option<f64>,
    sifat: Option<SifatAlokasi>,
    boleh_pinjam: bool,
}

/// Pemeriksaan tanpa I/O body downstream pool (AC-7, AC-17), termasuk CHECK
/// `chk_alokasi_*` supaya pelanggarannya menjadi 422, bukan 500.
fn periksa_downstream(
    body: &DownstreamPoolConfig,
    update: bool,
) -> (Vec<ValidasiErrorAdmin>, AlokasiRute) {
    let mut galat = Vec::new();
    let nama = body.nama.trim();
    if nama.is_empty() {
        galat.push(galat_field("nama", "wajib", None));
    } else if nama.chars().count() > NAMA_MAX {
        galat.push(galat_field("nama", "rentang", Some(NAMA_MAX as f64)));
    }
    if let Some(ttl) = body.ttl_antrean_detik
        && !(1..=i64::from(i32::MAX)).contains(&ttl)
    {
        let batas = if ttl < 1 { 1.0 } else { f64::from(i32::MAX) };
        galat.push(galat_field("ttl_antrean_detik", "rentang", Some(batas)));
    }
    if body.upstream_pool_ids.len() != 1 {
        galat.push(galat_field("upstream_pool_ids", "jumlah", Some(1.0)));
    }

    let ada = [
        body.alokasi_jenis.is_some(),
        body.alokasi_nilai.is_some(),
        body.sifat.is_some(),
    ];
    let beralokasi = ada.iter().any(|a| *a);
    if beralokasi {
        // `chk_alokasi_lengkap`: ketiganya terisi atau ketiganya kosong.
        for (field, terisi) in ["alokasi_jenis", "alokasi_nilai", "sifat"].iter().zip(ada) {
            if !terisi {
                galat.push(galat_field(*field, "wajib", None));
            }
        }
    } else if body.boleh_pinjam == Some(false) {
        // `chk_alokasi_tanpa_alokasi_boleh_pinjam`.
        galat.push(galat_field("boleh_pinjam", "tidak_diizinkan", None));
    }
    // `chk_alokasi_nilai`.
    if let (Some(jenis), Some(nilai)) = (body.alokasi_jenis, body.alokasi_nilai) {
        match jenis {
            AlokasiJenis::Exact => {
                if !nilai.is_finite() || nilai < 1.0 {
                    galat.push(galat_field("alokasi_nilai", "rentang", Some(1.0)));
                } else if nilai > ALOKASI_NILAI_MAX {
                    galat.push(galat_field(
                        "alokasi_nilai",
                        "rentang",
                        Some(ALOKASI_NILAI_MAX),
                    ));
                } else if nilai.fract() != 0.0 {
                    galat.push(galat_field("alokasi_nilai", "format", None));
                }
            }
            AlokasiJenis::Persen => {
                if !nilai.is_finite() || nilai < 0.01 {
                    galat.push(galat_field("alokasi_nilai", "rentang", Some(0.01)));
                } else if nilai > 100.0 {
                    galat.push(galat_field("alokasi_nilai", "rentang", Some(100.0)));
                }
            }
        }
    }
    if let Some(e) = periksa_alasan(body.alasan.as_deref(), update) {
        galat.push(e);
    }
    let alokasi = AlokasiRute {
        jenis: body.alokasi_jenis,
        nilai: body.alokasi_nilai,
        sifat: body.sifat,
        // Rute beralokasi tanpa `boleh_pinjam` eksplisit tidak meminjam.
        boleh_pinjam: body.boleh_pinjam.unwrap_or(!beralokasi),
    };
    (galat, alokasi)
}

async fn upstream_ada(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    id: Uuid,
) -> Result<bool, Galat> {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM upstream_pool WHERE upstream_pool_id = $1)")
        .bind(id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| galat_db(&e))
}

async fn tulis_rute(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    downstream_pool_id: Uuid,
    upstream_pool_id: Uuid,
    alokasi: AlokasiRute,
) -> Result<(), Galat> {
    sqlx::query("DELETE FROM downstream_pool_rute WHERE downstream_pool_id = $1")
        .bind(downstream_pool_id)
        .execute(&mut **tx)
        .await
        .map_err(|e| galat_db(&e))?;
    sqlx::query(
        "INSERT INTO downstream_pool_rute (downstream_pool_id, upstream_pool_id, alokasi_jenis, \
         alokasi_nilai, sifat, boleh_pinjam) VALUES ($1, $2, $3, $4::numeric, $5, $6)",
    )
    .bind(downstream_pool_id)
    .bind(upstream_pool_id)
    .bind(alokasi.jenis.map(AlokasiJenis::as_str))
    .bind(alokasi.nilai)
    .bind(alokasi.sifat.map(SifatAlokasi::as_str))
    .bind(alokasi.boleh_pinjam)
    .execute(&mut **tx)
    .await
    .map_err(|e| galat_tulis(&e, "upstream_pool_ids/0"))?;
    periksa_kelebihan(tx, upstream_pool_id, "alokasi_nilai").await
}

/// AL-01 sebelum commit: kelebihan alokasi atas kuota efektif upstream pool
/// menjadi 422 dengan `batas` = jumlah kelebihannya (U-64).
async fn periksa_kelebihan(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    upstream_pool_id: Uuid,
    field: &str,
) -> Result<(), Galat> {
    let lebih: i64 = sqlx::query_scalar("SELECT fn_alokasi_kelebihan($1)")
        .bind(upstream_pool_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| galat_db(&e))?;
    if lebih > 0 {
        return Err(Galat::validasi(vec![galat_field(
            field,
            "alokasi_kelebihan",
            Some(lebih as f64),
        )]));
    }
    Ok(())
}

/// `createDownstreamPool` (PA): pool, satu rute, audit, satu transaksi.
pub(super) async fn create_downstream_pool(
    State(s): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Result<Json<Value>, JsonRejection>,
) -> Result<Response, Galat> {
    let peminta = masuk(
        &s,
        &Method::POST,
        &headers,
        peer,
        &PERAN_PA,
        "createDownstreamPool",
    )
    .await?;
    periksa_guard_mfa(&peminta)?;
    let body: DownstreamPoolConfig = urai_body(body, &DOWNSTREAM_WAJIB, &DOWNSTREAM_OPSIONAL, &[])?;
    let (mut galat, alokasi) = periksa_downstream(&body, false);

    let mut tx = s.pool.begin().await.map_err(|e| galat_db(&e))?;
    let upstream = body.upstream_pool_ids.first().copied();
    if let Some(u) = upstream
        && !upstream_ada(&mut tx, u).await?
    {
        galat.push(galat_field("upstream_pool_ids/0", "tidak_ada", None));
    }
    if !galat.is_empty() {
        return Err(Galat::validasi(galat));
    }
    let upstream = upstream.ok_or_else(Galat::layanan)?;

    let id = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO downstream_pool (downstream_pool_id, nama, aktif, prioritas, ttl_antrean_detik) \
         VALUES ($1, $2, true, 1, $3)",
    )
    .bind(id)
    .bind(body.nama.trim())
    .bind(body.ttl_antrean_detik.unwrap_or(86_400) as i32)
    .execute(&mut *tx)
    .await
    .map_err(|e| galat_tulis(&e, "nama"))?;
    tulis_rute(&mut tx, id, upstream, alokasi).await?;

    let entitas = baca_downstream(&mut *tx, id)
        .await?
        .ok_or_else(Galat::layanan)?;
    catat(
        &mut tx,
        &peminta,
        EntitasKonfigurasi::DownstreamPool,
        AksiKonfigurasi::Buat,
        id,
        None,
        Some(snapshot(&entitas)),
        body.alasan
            .clone()
            .unwrap_or_else(|| "createDownstreamPool".to_owned()),
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| galat_tulis(&e, "alokasi_nilai"))?;
    Ok((StatusCode::CREATED, Json(entitas)).into_response())
}

/// `updateDownstreamPool` (PA). Penggantian penuh termasuk rute dan alokasinya;
/// `prioritas` tetap 1 dan tidak bisa ditulis.
pub(super) async fn update_downstream_pool(
    State(s): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Path(id): Path<Uuid>,
    body: Result<Json<Value>, JsonRejection>,
) -> Result<Response, Galat> {
    let peminta = masuk(
        &s,
        &Method::PUT,
        &headers,
        peer,
        &PERAN_PA,
        "updateDownstreamPool",
    )
    .await?;
    periksa_guard_mfa(&peminta)?;
    let body: DownstreamPoolConfig = urai_body(body, &DOWNSTREAM_WAJIB, &DOWNSTREAM_OPSIONAL, &[])?;
    let (mut galat, alokasi) = periksa_downstream(&body, true);

    let mut tx = s.pool.begin().await.map_err(|e| galat_db(&e))?;
    let terkunci: Option<(Uuid,)> = sqlx::query_as(
        "SELECT downstream_pool_id FROM downstream_pool WHERE downstream_pool_id = $1 FOR UPDATE",
    )
    .bind(id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| galat_db(&e))?;
    if terkunci.is_none() {
        return Err(Galat::tidak_ditemukan());
    }
    let upstream = body.upstream_pool_ids.first().copied();
    if let Some(u) = upstream
        && !upstream_ada(&mut tx, u).await?
    {
        galat.push(galat_field("upstream_pool_ids/0", "tidak_ada", None));
    }
    if !galat.is_empty() {
        return Err(Galat::validasi(galat));
    }
    let upstream = upstream.ok_or_else(Galat::layanan)?;
    let lama = baca_downstream(&mut *tx, id)
        .await?
        .ok_or_else(Galat::layanan)?;

    // TTL yang tidak dikirim tetap 24 jam (default kontrak), bukan nilai lama:
    // PUT adalah penggantian penuh.
    sqlx::query(
        "UPDATE downstream_pool SET nama = $2, ttl_antrean_detik = $3, diubah_at = now() \
         WHERE downstream_pool_id = $1",
    )
    .bind(id)
    .bind(body.nama.trim())
    .bind(body.ttl_antrean_detik.unwrap_or(86_400) as i32)
    .execute(&mut *tx)
    .await
    .map_err(|e| galat_tulis(&e, "nama"))?;
    tulis_rute(&mut tx, id, upstream, alokasi).await?;

    let entitas = baca_downstream(&mut *tx, id)
        .await?
        .ok_or_else(Galat::layanan)?;
    catat(
        &mut tx,
        &peminta,
        EntitasKonfigurasi::DownstreamPool,
        AksiKonfigurasi::Ubah,
        id,
        Some(snapshot(&lama)),
        Some(snapshot(&entitas)),
        body.alasan.clone().unwrap_or_default(),
    )
    .await?;
    tx.commit()
        .await
        .map_err(|e| galat_tulis(&e, "alokasi_nilai"))?;
    Ok((StatusCode::OK, Json(entitas)).into_response())
}

// ---------------------------------------------------------------------------
// Quota tracker (AC-18)
// ---------------------------------------------------------------------------

#[derive(Debug, serde::Deserialize)]
pub(super) struct KuotaQuery {
    upstream_pool_id: Option<Uuid>,
}

/// `getQuotaTracker` (PA, OP). Angka dari database, bukan Redis (KP-07):
/// `terpakai_periode` = `max(snapshot upstream_attempt_daily, baris log periode)`,
/// aturan yang sama dengan rebuild kuota (`nilai_counter`).
pub(super) async fn get_quota_tracker(
    State(s): State<AdminState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    query: Result<Query<KuotaQuery>, axum::extract::rejection::QueryRejection>,
) -> Result<Response, Galat> {
    let peminta = masuk(
        &s,
        &Method::GET,
        &headers,
        peer,
        &PERAN_KUOTA,
        "getQuotaTracker",
    )
    .await?;
    periksa_guard_mfa(&peminta)?;
    let Ok(Query(query)) = query else {
        return Err(Galat::validasi(vec![galat_field(
            "upstream_pool_id",
            "format",
            None,
        )]));
    };
    let sekarang = (s.jam)();

    let mut tx = s.pool.begin().await.map_err(|e| galat_db(&e))?;
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await
        .map_err(|e| galat_db(&e))?;
    let pools = sqlx::query(
        "SELECT upstream_pool_id, kuota_harian::bigint AS kuota_harian, \
         margin_kuota::float8 AS margin_kuota, porsi_sync_maks::float8 AS porsi_sync, \
         parameter_dispatch, reset_kuota_jam, reset_kuota_zona::text AS reset_kuota_zona, \
         reset_kuota_berlaku_mulai, reset_kuota_jam_berikut, \
         reset_kuota_zona_berikut::text AS reset_kuota_zona_berikut \
         FROM upstream_pool WHERE ($1::uuid IS NULL OR upstream_pool_id = $1) \
         ORDER BY dibuat_at, upstream_pool_id",
    )
    .bind(query.upstream_pool_id)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e| galat_db(&e))?;
    if query.upstream_pool_id.is_some() && pools.is_empty() {
        return Err(Galat::tidak_ditemukan());
    }

    let jam_mulai = sekarang
        .duration_trunc(Duration::hours(1))
        .map_err(|_| Galat::layanan())?;
    let mut hasil = Vec::with_capacity(pools.len());
    for b in &pools {
        hasil.push(baris_tracker(&mut tx, b, sekarang, jam_mulai).await?);
    }
    tx.commit().await.map_err(|e| galat_db(&e))?;
    Ok((StatusCode::OK, Json(hasil)).into_response())
}

async fn baris_tracker(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    b: &sqlx::postgres::PgRow,
    sekarang: DateTime<Utc>,
    jam_mulai: DateTime<Utc>,
) -> Result<KuotaTracker, Galat> {
    let id: Uuid = b.try_get("upstream_pool_id").map_err(|e| galat_db(&e))?;
    let kuota_harian: i64 = b.try_get("kuota_harian").map_err(|e| galat_db(&e))?;
    let margin: f64 = b.try_get("margin_kuota").map_err(|e| galat_db(&e))?;
    let porsi_sync: f64 = b.try_get("porsi_sync").map_err(|e| galat_db(&e))?;
    let parameter: sqlx::types::Json<Value> =
        b.try_get("parameter_dispatch").map_err(|e| galat_db(&e))?;
    let periode = periode_dari_baris(b, sekarang).map_err(|e| {
        tracing::error!(upstream_pool_id = %id, error = %e, "admin: periode kuota tidak bisa dihitung");
        Galat::layanan()
    })?;

    let snapshot: Option<i64> = sqlx::query_scalar(
        "SELECT jumlah FROM upstream_attempt_daily \
         WHERE upstream_pool_id = $1 AND periode_mulai = $2",
    )
    .bind(id)
    .bind(periode.mulai)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| galat_db(&e))?;
    let (log_periode, log_jam, sync_jam): (i64, i64, i64) = sqlx::query_as(
        "SELECT \
           count(*) FILTER (WHERE l.requested_at >= $2 AND l.requested_at < $3), \
           count(*) FILTER (WHERE l.requested_at >= $4 AND l.requested_at < $5), \
           count(*) FILTER (WHERE l.requested_at >= $4 AND l.requested_at < $5 \
                              AND r.mode = 'sync') \
         FROM v_transaction_log_tersamar l \
         LEFT JOIN v_request_tersamar r ON r.request_id = l.request_id \
         WHERE l.upstream_pool_id = $1 AND l.requested_at >= least($2, $4)",
    )
    .bind(id)
    .bind(periode.mulai)
    .bind(periode.selesai)
    .bind(jam_mulai)
    .bind(jam_mulai + Duration::hours(1))
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| galat_db(&e))?;

    let kuota_efektif = kuota_efektif(kuota_harian, margin);
    let terpakai = nilai_counter(snapshot.unwrap_or(0), log_periode);
    let rate_limit_per_jam = KonfigurasiAdmin::dari_parameter(&parameter.0).rate_limit_per_jam;
    Ok(KuotaTracker {
        upstream_pool_id: id,
        periode_mulai: periode.mulai,
        periode_selesai: periode.selesai,
        kuota_harian,
        kuota_efektif,
        terpakai_periode: terpakai,
        sisa_periode: (kuota_efektif - terpakai).max(0),
        porsi_sync_maks: porsi_sync_per_jam(rate_limit_per_jam, porsi_sync),
        terpakai_sync_jam_berjalan: sync_jam,
        terpakai_jam_berjalan: log_jam,
        rate_limit_per_jam,
        dihitung_pada: sekarang,
    })
}

/// `Q = floor(kuota_harian * (1 - margin_kuota))`, sama dengan
/// `fn_alokasi_kelebihan` (AL-01).
fn kuota_efektif(kuota_harian: i64, margin: f64) -> i64 {
    ((kuota_harian as f64) * (1.0 - margin)).floor() as i64
}

/// Porsi sync dalam permintaan per jam: `floor(rate_limit_per_jam * porsi_sync_maks)`,
/// pasangan `terpakai_sync_jam_berjalan` (K-10).
fn porsi_sync_per_jam(rate_limit_per_jam: i64, porsi: f64) -> i64 {
    ((rate_limit_per_jam as f64) * porsi).floor() as i64
}

// ---------------------------------------------------------------------------
// Bantuan bersama (dipakai juga oleh `endpoint.rs`)
// ---------------------------------------------------------------------------

pub(super) fn galat_field(
    field: impl Into<String>,
    kode: &str,
    batas: Option<f64>,
) -> ValidasiErrorAdmin {
    ValidasiErrorAdmin {
        field: field.into(),
        kode: kode.to_owned(),
        batas,
        posisi: None,
        nama: None,
    }
}

/// Urai body JSON menjadi `T` dengan semua pelanggaran bentuk sekaligus (AC-7):
/// field `wajib` yang hilang dan field yang tidak dikenal. Nilai input tidak
/// pernah dikembalikan; galat tipe dilaporkan sebagai `body`/`format`.
/// `bawaan` mengisi field opsional yang tidak dikirim (hanya untuk POST).
pub(super) fn urai_body<T: serde::de::DeserializeOwned>(
    body: Result<Json<Value>, JsonRejection>,
    wajib: &[&str],
    opsional: &[&str],
    bawaan: &[(&str, Value)],
) -> Result<T, Galat> {
    let format = || Galat::validasi(vec![galat_field("body", "format", None)]);
    let Ok(Json(mut nilai)) = body else {
        return Err(format());
    };
    let Some(objek) = nilai.as_object_mut() else {
        return Err(format());
    };
    let mut galat: Vec<ValidasiErrorAdmin> = wajib
        .iter()
        .filter(|k| !objek.contains_key(**k))
        .map(|k| galat_field(*k, "wajib", None))
        .collect();
    galat.extend(
        objek
            .keys()
            .filter(|k| !wajib.contains(&k.as_str()) && !opsional.contains(&k.as_str()))
            .map(|k| galat_field(k.clone(), "tidak_dikenal", None)),
    );
    if !galat.is_empty() {
        return Err(Galat::validasi(galat));
    }
    for (k, v) in bawaan {
        objek.entry((*k).to_owned()).or_insert_with(|| v.clone());
    }
    serde_json::from_value(nilai).map_err(|e| {
        // Hanya posisi; teks galat serde bisa memuat nilai input.
        tracing::debug!(
            baris = e.line(),
            kolom = e.column(),
            "admin: body tidak sesuai skema"
        );
        format()
    })
}

/// `alasan` 10–500 karakter setelah trim (aturan yang sama dengan K-20);
/// `wajib` pada update.
pub(super) fn periksa_alasan(alasan: Option<&str>, wajib: bool) -> Option<ValidasiErrorAdmin> {
    match alasan {
        None if wajib => Some(galat_field("alasan", "wajib", None)),
        None => None,
        Some(a) if !k20::validasi_alasan(a) => Some(galat_field(
            "alasan",
            "rentang",
            Some(k20::ALASAN_MAX as f64),
        )),
        Some(_) => None,
    }
}

/// Isi `nilai_lama`/`nilai_baru` audit: entitas baca, tanpa nilai rahasia
/// (konfigurasi pool dan endpoint tidak memuat secret atau NIK).
pub(super) fn snapshot<T: serde::Serialize>(entitas: &T) -> Value {
    serde_json::to_value(entitas).unwrap_or(Value::Null)
}

/// Satu baris `audit_konfigurasi` di transaksi pemanggil (AC-5).
#[allow(clippy::too_many_arguments)]
pub(super) async fn catat(
    tx: &mut sqlx::Transaction<'static, sqlx::Postgres>,
    peminta: &Peminta,
    entitas: EntitasKonfigurasi,
    aksi: AksiKonfigurasi,
    entitas_id: Uuid,
    nilai_lama: Option<Value>,
    nilai_baru: Option<Value>,
    alasan: String,
) -> Result<(), Galat> {
    let baris = KonfigurasiBaru {
        pelaku: peminta.pengguna_id,
        entitas,
        entitas_id: Some(entitas_id),
        aksi,
        nilai_lama,
        nilai_baru,
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

/// Galat tulis: nama ganda menjadi 422 `sudah_dipakai` pada `nama`; pelanggaran
/// FK atau CHECK (termasuk trigger alokasi saat commit) menjadi 422 pada
/// `field_fk`; selain itu 503.
pub(super) fn galat_tulis(e: &sqlx::Error, field_fk: &str) -> Galat {
    if let sqlx::Error::Database(db) = e {
        if db.is_unique_violation() {
            tracing::debug!("admin: nama sudah dipakai");
            return Galat::validasi(vec![galat_field("nama", "sudah_dipakai", None)]);
        }
        if db.is_foreign_key_violation() {
            tracing::debug!(field = field_fk, "admin: rujukan tidak ada");
            return Galat::validasi(vec![galat_field(field_fk, "tidak_ada", None)]);
        }
        if db.is_check_violation() {
            tracing::debug!(field = field_fk, "admin: CHECK ditolak");
            return Galat::validasi(vec![galat_field(field_fk, "rentang", None)]);
        }
    }
    galat_db(e)
}

pub(super) fn galat_db(e: &sqlx::Error) -> Galat {
    tracing::error!(error = %e, "admin: kueri pool/endpoint gagal");
    Galat::layanan()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool_valid() -> UpstreamPoolConfig {
        serde_json::from_value(json!({
            "provider_id": Uuid::nil(),
            "kuota_harian": 1000,
            "rate_limit_per_jam": 3600,
            "porsi_sync": 0.2,
            "margin_kuota": 0.01,
            "batas_percobaan": 3,
            "retry_backoff_base_detik": 5,
            "retry_backoff_cap_detik": 300,
            "retry_rasio": 0.1,
            "retry_min_per_jam": 20,
            "jendela_kuota": "bergulir_24j",
            "reset_kuota_jam": 0,
            "reset_kuota_zona": "Asia/Jakarta",
            "jeda_min": 100,
            "jeda_maks": 500,
            "ambang_berdampingan": 4
        }))
        .unwrap()
    }

    fn field(g: &[ValidasiErrorAdmin]) -> Vec<(&str, &str)> {
        g.iter()
            .map(|e| (e.field.as_str(), e.kode.as_str()))
            .collect()
    }

    #[test]
    fn parameter_dispatch_dibaca_dispatcher_dan_kembali_utuh() {
        let p = KonfigurasiAdmin::dari_body(&pool_valid())
            .gabung_ke(&json!({ "timeout_total_upstream_detik": 20, "rate": { "tps": { "t_mikro": 100_000, "burst": 2 } } }));
        let d = poolrouter_core::kuota::sumber_pg::ParameterDispatch::dari_json(&p).unwrap();
        assert_eq!(
            d.timeout_total_upstream_detik, 20,
            "kunci lain dipertahankan"
        );
        assert_eq!(d.maks_konkurensi, 4);
        assert_eq!(d.jeda_acak_maks_mikro, Some(500_000));
        assert_eq!(p["jeda_acak_min_mikro"], json!(100_000));
        let tph = d.jendela_rate.iter().find(|j| j.jenis == "tph").unwrap();
        assert_eq!((tph.t_mikro, tph.burst), (1_000_000, 1));
        assert_eq!(
            d.jendela_rate.len(),
            1,
            "jendela tps seed dibuang (ARC-233)"
        );
        let a = KonfigurasiAdmin::dari_parameter(&p);
        assert_eq!(a, KonfigurasiAdmin::dari_body(&pool_valid()));
    }

    /// ARC-233 A1: `jeda_min` ms sampai ke dispatcher sebagai batas bawah jeda.
    #[test]
    fn jeda_min_menjadi_kunci_dispatcher() {
        let p = KonfigurasiAdmin::dari_body(&pool_valid()).gabung_ke(&json!({}));
        assert_eq!(p["jeda_acak_min_mikro"], json!(100_000));
        let d = poolrouter_core::kuota::sumber_pg::ParameterDispatch::dari_json(&p).unwrap();
        assert_eq!(d.jeda_acak_min_mikro, Some(100_000));
        assert_eq!(d.rentang_jeda_mikro(), (100_000, 500_000));
    }

    /// ARC-233 A1: pool tanpa `konfigurasi_admin` membaca `jeda_min` dari kunci
    /// dispatcher, bukan hardcode 0.
    #[test]
    fn jeda_min_dibaca_dari_kunci_dispatcher_tanpa_konfigurasi_admin() {
        let a = KonfigurasiAdmin::dari_parameter(&json!({
            "rate": { "tph": { "t_mikro": 1_000_000, "burst": 1 } },
            "jeda_acak_min_mikro": 100_000,
            "jeda_acak_maks_mikro": 500_000
        }));
        assert_eq!((a.jeda_min, a.jeda_maks), (100, 500));
    }

    /// ARC-233 B1: PUT di atas pool seed bertjendela `tps` menghasilkan `rate`
    /// berisi `tph` saja; kunci lain tetap.
    #[test]
    fn put_membuang_jendela_tps_tpm_lama() {
        let lama = json!({
            "timeout_total_upstream_detik": 20,
            "rate": {
                "tps": { "t_mikro": 100_000, "burst": 1 },
                "tpm": { "t_mikro": 6_000_000, "burst": 1 },
                "tph": { "t_mikro": 1, "burst": 9 }
            }
        });
        assert_eq!(jendela_rate_dibuang(&lama), ["tpm", "tps"]);
        let p = KonfigurasiAdmin::dari_body(&pool_valid()).gabung_ke(&lama);
        assert_eq!(
            p["rate"],
            json!({ "tph": { "t_mikro": 1_000_000, "burst": 1 } })
        );
        assert_eq!(p["timeout_total_upstream_detik"], json!(20));
    }

    #[test]
    fn parameter_tanpa_konfigurasi_admin_diturunkan() {
        let a = KonfigurasiAdmin::dari_parameter(
            &json!({ "rate": { "tps": { "t_mikro": 100_000, "burst": 1 } } }),
        );
        assert_eq!(a.rate_limit_per_jam, 36_000);
        assert_eq!(
            a.ambang_berdampingan,
            poolrouter_core::kuota::sumber_pg::MAKS_KONKURENSI_BAWAAN
        );
    }

    #[test]
    fn upstream_menolak_zona_tak_dikenal_kalender_tanpa_alasan_dan_i32() {
        let mut b = pool_valid();
        b.reset_kuota_zona = "Europe/Berlin".to_owned();
        b.jendela_kuota = JendelaKuota::Kalender;
        b.kuota_harian = i64::from(i32::MAX) + 1;
        let g = periksa_upstream(&b, false);
        let f = field(&g);
        assert!(f.contains(&("reset_kuota_zona", "format")));
        assert!(f.contains(&("alasan", "wajib")));
        assert!(f.contains(&("kuota_harian", "rentang")));
    }

    #[test]
    fn update_upstream_mewajibkan_alasan() {
        assert!(periksa_upstream(&pool_valid(), false).is_empty());
        assert_eq!(
            field(&periksa_upstream(&pool_valid(), true)),
            vec![("alasan", "wajib")]
        );
    }

    fn dp(v: Value) -> DownstreamPoolConfig {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn downstream_memeriksa_check_alokasi() {
        let (g, a) = periksa_downstream(
            &dp(json!({ "nama": "BGN", "upstream_pool_ids": [Uuid::nil()] })),
            false,
        );
        assert!(g.is_empty());
        assert!(a.boleh_pinjam && a.jenis.is_none());

        let (g, _) = periksa_downstream(
            &dp(
                json!({ "nama": "BGN", "upstream_pool_ids": [Uuid::nil()], "alokasi_jenis": "exact" }),
            ),
            false,
        );
        assert_eq!(
            field(&g),
            vec![("alokasi_nilai", "wajib"), ("sifat", "wajib")]
        );

        let (g, _) = periksa_downstream(
            &dp(
                json!({ "nama": "BGN", "upstream_pool_ids": [Uuid::nil()], "boleh_pinjam": false }),
            ),
            false,
        );
        assert_eq!(field(&g), vec![("boleh_pinjam", "tidak_diizinkan")]);

        let (g, a) = periksa_downstream(
            &dp(json!({ "nama": "BGN", "upstream_pool_ids": [Uuid::nil()],
                        "alokasi_jenis": "exact", "alokasi_nilai": 2.5, "sifat": "reserved" })),
            false,
        );
        assert_eq!(field(&g), vec![("alokasi_nilai", "format")]);
        assert!(!a.boleh_pinjam, "rute beralokasi tidak meminjam tanpa izin");

        let (g, _) = periksa_downstream(
            &dp(json!({ "nama": "BGN", "upstream_pool_ids": [Uuid::nil()],
                        "alokasi_jenis": "persen", "alokasi_nilai": 100.5, "sifat": "shareable" })),
            false,
        );
        assert_eq!(field(&g), vec![("alokasi_nilai", "rentang")]);
    }

    #[test]
    fn downstream_tepat_satu_upstream_dan_nama() {
        let (g, _) = periksa_downstream(
            &dp(json!({ "nama": " ", "upstream_pool_ids": [Uuid::nil(), Uuid::from_u128(1)] })),
            true,
        );
        assert_eq!(
            field(&g),
            vec![
                ("nama", "wajib"),
                ("upstream_pool_ids", "jumlah"),
                ("alasan", "wajib")
            ]
        );
    }

    #[test]
    fn urai_body_melaporkan_semua_field_hilang_dan_asing() {
        let body: Result<Json<Value>, JsonRejection> = Ok(Json(json!({ "nama": "x", "asing": 1 })));
        let Err(g) =
            urai_body::<DownstreamPoolConfig>(body, &DOWNSTREAM_WAJIB, &DOWNSTREAM_OPSIONAL, &[])
        else {
            panic!("harus 422");
        };
        let teks = format!("{:?}", g.problem());
        assert!(teks.contains("upstream_pool_ids"), "{teks}");
        assert!(teks.contains("asing"), "{teks}");
    }

    #[test]
    fn kuota_efektif_dan_porsi_sync() {
        assert_eq!(kuota_efektif(1000, 0.01), 990);
        assert_eq!(kuota_efektif(41_667, 0.01), 41_250);
        assert_eq!(porsi_sync_per_jam(3600, 0.2), 720);
    }
}
