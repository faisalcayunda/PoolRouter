//! Closed value sets shared by downstream and admin APIs. Names and values
//! follow data contract v0.7.10 (see `backend/openapi/DESIGN.md`).

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Status permintaan yang tampil di polling dan webhook. Tidak pernah mundur ke
/// status sebelumnya.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ApiStatus {
    Queued,
    Processing,
    Done,
    Failed,
    Expired,
    Cancelled,
}

/// `failure_reason` sebuah permintaan berstatus `failed`. Daftar nilai tertutup
/// dan tampil ke klien; `kode` pada body 502/504 sinkron sama dengan nilai ini.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FailureReason {
    /// Percobaan terakhir gagal pada kelas gangguan sementara dan tidak ada
    /// percobaan tersisa, atau jendela retry sudah habis. Berlaku juga untuk
    /// pengiriman sinkron (hanya ada satu percobaan efektif).
    RetryHabis,
    /// Percobaan gagal pada kelas yang tidak diulang (autentikasi, kuota provider,
    /// kode yang tidak dikenal, respons tidak sah, TLS).
    ProviderErrorPermanen,
    /// Panggilan provider sinkron tidak selesai dalam batas waktu.
    TimeoutSync,
    /// Objek penyedia dinonaktifkan saat permintaan sedang menunggu retry
    /// (`attempt_count > 0`).
    ObjekNonaktif,
}

/// `cancel_reason` sebuah permintaan berstatus `cancelled`. Satu nilai untuk tiap
/// jenis objek; kedua jenis pool memakai `pool_nonaktif`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CancelReason {
    DibatalkanClient,
    ClientNonaktif,
    KredensialNonaktif,
    EndpointNonaktif,
    PoolNonaktif,
    ProviderNonaktif,
}

/// `kode` sebuah respons 429. Setiap nilainya disertai `Retry-After`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RateLimitKode {
    KuotaHabis,
    PorsiSyncHabis,
    PenolakanBerlebih,
}

/// Interface language (ADR-0003 Addendum A).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum BahasaUi {
    Id,
    En,
}

/// Jenis objek yang bisa dinonaktifkan. Dikirim di `jenis_objek` pada 403/503
/// `objek_nonaktif`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum JenisObjek {
    Endpoint,
    DownstreamPool,
    UpstreamPool,
    Provider,
}

/// Jenis rule validasi pada konfigurasi provider (BRD US-03). F0 tidak
/// menyediakan rule kustom; nilai di luar enum ini ditolak 422.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum RuleJenis {
    Regex,
    Enum,
    Panjang,
    FormatTanggal,
}

/// Jenis kredensial downstream. Hanya dua nilai ini yang ada.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum JenisKredensial {
    Oauth2,
    ApiKey,
}

/// Periode kuota upstream pool.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum JendelaKuota {
    /// Jendela bergulir 24 jam.
    #[serde(rename = "bergulir_24j")]
    Bergulir24j,
    Kalender,
}

/// Status per elemen pada `hasil`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StatusHasil {
    Sesuai,
    TidakSesuai,
}

/// Peran admin. Daftar nilai tertutup; tiap pengguna hanya punya satu peran, dan
/// `client_id` hanya diisi untuk `client_owner`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Peran {
    Auditor,
    Operator,
    PlatformAdmin,
    ClientOwner,
}

/// Credential lifecycle state (contract EN-02/EN-03, schema `status`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StatusKredensial {
    Aktif,
    Kedaluwarsa,
    Dicabut,
}

/// Webhook delivery outcome (schema CHECK on `webhook_log.hasil`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum HasilWebhook {
    /// 2xx.
    Terkirim,
    /// Non-2xx after delivery.
    GagalHttp,
    /// No usable connection.
    GagalJaringan,
    /// Destination outside the allowlist; never sent.
    DitolakAllowlist,
    /// Failure inside the sender (e.g. secret decryption).
    GagalInternal,
}

/// Failure class of a webhook delivery (contract section 3.3). `provider_sementara`
/// comes from rev 9 (contract v0.7.5 section 11 item 32c).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum KelasErrorWebhook {
    Timeout,
    Koneksi,
    RateLimitProvider,
    ResponsTidakValid,
    Tls,
    ProviderSementara,
}
