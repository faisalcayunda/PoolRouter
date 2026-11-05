//! Access auditing (`audit_akses`, `audit_login`): ADR-0003 rev 5 section
//! 3.6/3.6b/3.6c, data contract section 8.2, EN-07/EN-08.
//!
//! Two rules drive everything here:
//!
//! 1. An audit row is never written on its own connection. It shares the
//!    transaction of the thing it describes (ADR-0003 section 3.6c), so a
//!    rolled-back role change cannot leave an `ubah_peran` row claiming it
//!    happened. [`Penulis`] therefore borrows a `&mut` connection that is
//!    already inside a transaction and offers no way to open one.
//! 2. `pengguna_id` and `peran` are required even for a refusal. A denied
//!    access is a first-class row with `hasil = 'ditolak'`, not an absence of
//!    a row — that is what makes US-16 reviewable.
//!
//! `audit_akses` is append-only in the database (trigger `tg_append_only`), so
//! the API here is write-only by construction: there is no read path. Reading
//! goes through `fn_lihat_audit` (E-06), which is itself audited and lives in
//! the webserver, not here.
//!
//! Login is a separate table on purpose (Klaus, round 2): a failed login for an
//! unknown username has no `pengguna_id`, while `audit_akses.pengguna_id` is
//! `NOT NULL`. [`catat_login`] keeps the two trails apart.

use std::fmt;

use chrono::{DateTime, Utc};
use sqlx::{PgConnection, Postgres, Transaction};
use uuid::Uuid;

/// Role names exactly as the contract spells them (section 8.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Peran {
    PlatformAdmin,
    Auditor,
    Operator,
    ClientOwner,
}

impl Peran {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PlatformAdmin => "platform_admin",
            Self::Auditor => "auditor",
            Self::Operator => "operator",
            Self::ClientOwner => "client_owner",
        }
    }

    /// Every role, in contract order. Used by the CLI to validate `--peran`
    /// and by tests to prove the list matches the database check constraint.
    pub const SEMUA: [Self; 4] = [
        Self::PlatformAdmin,
        Self::Auditor,
        Self::Operator,
        Self::ClientOwner,
    ];

    /// Roles whose grant needs a named approver outside the requester
    /// (ADR-0003 section 3.6c, C-07 / SPTJM): `auditor` and `operator`.
    pub const fn butuh_persetujuan(self) -> bool {
        matches!(self, Self::Auditor | Self::Operator)
    }
}

impl fmt::Display for Peran {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Parse a role from its contract spelling. Case-sensitive: the CLI takes the
/// literal database value so a typo fails loudly instead of silently matching.
impl std::str::FromStr for Peran {
    type Err = PeranTidakDikenal;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::SEMUA
            .into_iter()
            .find(|p| p.as_str() == s)
            .ok_or_else(|| PeranTidakDikenal(s.to_owned()))
    }
}

/// A role string that is not one of the four contract values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeranTidakDikenal(pub String);

impl fmt::Display for PeranTidakDikenal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "peran tidak dikenal: {:?} (pilih salah satu dari {})",
            self.0,
            Peran::SEMUA.map(Peran::as_str).join(", ")
        )
    }
}

impl std::error::Error for PeranTidakDikenal {}

/// What the caller was looking at. Matches `aksi` in the check constraint of
/// `audit_akses`. Only the actions F0 can produce are listed; adding one means
/// a migration, not a string here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Aksi {
    LihatLog,
    CariLog,
    CariNik,
    LihatPayload,
    Ekspor,
    LihatAudit,
    RestoreArsip,
    UbahPeran,
    AksesDitolak,
}

impl Aksi {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LihatLog => "lihat_log",
            Self::CariLog => "cari_log",
            Self::CariNik => "cari_nik",
            Self::LihatPayload => "lihat_payload",
            Self::Ekspor => "ekspor",
            Self::LihatAudit => "lihat_audit",
            Self::RestoreArsip => "restore_arsip",
            Self::UbahPeran => "ubah_peran",
            Self::AksesDitolak => "akses_ditolak",
        }
    }

    pub const SEMUA: [Self; 9] = [
        Self::LihatLog,
        Self::CariLog,
        Self::CariNik,
        Self::LihatPayload,
        Self::Ekspor,
        Self::LihatAudit,
        Self::RestoreArsip,
        Self::UbahPeran,
        Self::AksesDitolak,
    ];
}

impl fmt::Display for Aksi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// How the NIK appeared in the response. A search that returns nothing still
/// records what the response *would* have shown, so "tidak_ada" is a real value
/// and not a NULL (contract 8.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TampilanNik {
    Utuh,
    Tersamar,
    TidakAda,
}

impl TampilanNik {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Utuh => "utuh",
            Self::Tersamar => "tersamar",
            Self::TidakAda => "tidak_ada",
        }
    }
}

impl fmt::Display for TampilanNik {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Whether the action was carried out. `ditolak` is what a wrong-role request
/// produces; it always comes with [`Aksi::AksesDitolak`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Hasil {
    Berhasil,
    Ditolak,
}

impl Hasil {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Berhasil => "berhasil",
            Self::Ditolak => "ditolak",
        }
    }
}

impl fmt::Display for Hasil {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A NIK as an HMAC (contract 8.2): 64 lowercase hex, keyed by the server-side
/// HMAC key, never the plain NIK. Constructed only through [`NikHmac::parse`]
/// so a plaintext NIK cannot reach the INSERT by mistake.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct NikHmac(String);

impl NikHmac {
    /// Accepts lowercase hex only, for the same reason the column check
    /// requires it: two encodings of one digest would break the lookup index.
    pub fn parse(raw: impl Into<String>) -> Result<Self, NikHmacTidakValid> {
        let s = raw.into();
        if s.len() == 64
            && s.bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            Ok(Self(s))
        } else {
            Err(NikHmacTidakValid)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The value given to [`NikHmac::parse`] is not 64 lowercase hex characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NikHmacTidakValid;

impl fmt::Display for NikHmacTidakValid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("nik_hmac harus 64 karakter hex huruf kecil")
    }
}

impl std::error::Error for NikHmacTidakValid {}

/// A reason that satisfies the contract length (ADR-0003 section 3.6b: 10–500
/// characters for the auditor actions). Trimmed before measuring, because a
/// reason of eleven spaces is not a reason.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Alasan(String);

impl Alasan {
    pub const MIN: usize = 10;
    pub const MAX: usize = 500;

    pub fn baru(raw: impl Into<String>) -> Result<Self, AlasanTidakValid> {
        let s = raw.into();
        let t = s.trim();
        let n = t.chars().count();
        if (Self::MIN..=Self::MAX).contains(&n) {
            Ok(Self(t.to_owned()))
        } else {
            Err(AlasanTidakValid { panjang: n })
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A reason whose trimmed length is outside 10..=500 characters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AlasanTidakValid {
    pub panjang: usize,
}

impl fmt::Display for AlasanTidakValid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "alasan harus {}–{} karakter (sekarang {})",
            Alasan::MIN,
            Alasan::MAX,
            self.panjang
        )
    }
}

impl std::error::Error for AlasanTidakValid {}

/// Who performed the action. `peran` is the role the action was taken under,
/// not the whole role list: A-02 is reviewed by which role authorised it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pelaku {
    pub pengguna_id: Uuid,
    pub peran: Peran,
}

impl Pelaku {
    pub const fn baru(pengguna_id: Uuid, peran: Peran) -> Self {
        Self { pengguna_id, peran }
    }
}

/// One `audit_akses` row, before it is written. Ids and times are filled by the
/// database/caller split described on [`Penulis`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AksesBaru {
    pub pelaku: Pelaku,
    pub aksi: Aksi,
    pub tampilan_nik: TampilanNik,
    pub hasil: Hasil,
    pub filter_kueri: Option<serde_json::Value>,
    pub nik_hmac: Option<NikHmac>,
    pub nik_tersamar: Option<String>,
    pub jumlah_record: Option<i64>,
    pub alasan: Option<Alasan>,
    pub sumber_ip: Option<String>,
    pub sesi_id: Option<String>,
}

impl AksesBaru {
    /// A plain successful access (log list, export) with no NIK involved.
    pub fn berhasil(pelaku: Pelaku, aksi: Aksi) -> Self {
        Self {
            pelaku,
            aksi,
            tampilan_nik: TampilanNik::TidakAda,
            hasil: Hasil::Berhasil,
            filter_kueri: None,
            nik_hmac: None,
            nik_tersamar: None,
            jumlah_record: None,
            alasan: None,
            sumber_ip: None,
            sesi_id: None,
        }
    }

    /// The row a wrong-role request leaves behind (contract 8.1: 403
    /// `akses_ditolak` and an `akses_ditolak` row). `aksi` is always
    /// [`Aksi::AksesDitolak`] here so the two can never disagree; what was
    /// attempted goes in `filter_kueri` at the call site if it matters.
    pub fn ditolak(pelaku: Pelaku) -> Self {
        Self {
            aksi: Aksi::AksesDitolak,
            tampilan_nik: TampilanNik::TidakAda,
            hasil: Hasil::Ditolak,
            ..Self::berhasil(pelaku, Aksi::AksesDitolak)
        }
    }

    pub fn dengan_tampilan_nik(mut self, tampilan: TampilanNik) -> Self {
        self.tampilan_nik = tampilan;
        self
    }

    pub fn dengan_filter(mut self, filter: serde_json::Value) -> Self {
        self.filter_kueri = Some(filter);
        self
    }

    pub fn dengan_nik_hmac(mut self, hmac: NikHmac) -> Self {
        self.nik_hmac = Some(hmac);
        self
    }

    pub fn dengan_nik_tersamar(mut self, tersamar: impl Into<String>) -> Self {
        self.nik_tersamar = Some(tersamar.into());
        self
    }

    pub fn dengan_jumlah_record(mut self, n: i64) -> Self {
        self.jumlah_record = Some(n);
        self
    }

    pub fn dengan_alasan(mut self, alasan: Alasan) -> Self {
        self.alasan = Some(alasan);
        self
    }

    pub fn dengan_sumber_ip(mut self, ip: impl Into<String>) -> Self {
        self.sumber_ip = Some(ip.into());
        self
    }

    pub fn dengan_sesi(mut self, sesi_id: impl Into<String>) -> Self {
        self.sesi_id = Some(sesi_id.into());
        self
    }

    /// Contract 8.2: an auditor action that shows or exports NIK needs a
    /// reason. This is checked here rather than in SQL because the NULL is
    /// legitimate for other actions, so the database cannot tell which call
    /// site forgot.
    pub fn wajib_alasan(&self) -> bool {
        matches!(
            self.aksi,
            Aksi::CariNik | Aksi::LihatPayload | Aksi::Ekspor | Aksi::LihatAudit
        )
    }
}

/// Why an audit row could not be written.
#[derive(Debug)]
pub enum GalatAudit {
    /// An action that requires a reason was submitted without one.
    AlasanHilang { aksi: Aksi },
    /// The database rejected the row.
    BasisData(sqlx::Error),
}

impl fmt::Display for GalatAudit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlasanHilang { aksi } => write!(f, "aksi {aksi} wajib menyertakan alasan"),
            Self::BasisData(e) => write!(f, "gagal menulis audit: {e}"),
        }
    }
}

impl std::error::Error for GalatAudit {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::AlasanHilang { .. } => None,
            Self::BasisData(e) => Some(e),
        }
    }
}

impl From<sqlx::Error> for GalatAudit {
    fn from(e: sqlx::Error) -> Self {
        Self::BasisData(e)
    }
}

/// Which kind of object a configuration change touched. Mirrors the
/// `entitas_jenis` CHECK of `audit_konfigurasi`; a new variant needs a migration,
/// not a string at the call site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EntitasKonfigurasi {
    Client,
    Kredensial,
    DownstreamEndpoint,
    DownstreamPool,
    UpstreamPool,
    Provider,
    ClientWebhook,
    WebhookDomainAllowlist,
    ProviderConfig,
    EndpointConfig,
    EndpointMetadataField,
    KonfigurasiPlatform,
    AdminUser,
    /// One entry of the global provider secret store (ARC-48, AD1).
    ProviderSecret,
}

impl EntitasKonfigurasi {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Kredensial => "kredensial",
            Self::DownstreamEndpoint => "downstream_endpoint",
            Self::DownstreamPool => "downstream_pool",
            Self::UpstreamPool => "upstream_pool",
            Self::Provider => "provider",
            Self::ClientWebhook => "client_webhook",
            Self::WebhookDomainAllowlist => "webhook_domain_allowlist",
            Self::ProviderConfig => "provider_config",
            Self::EndpointConfig => "endpoint_config",
            Self::EndpointMetadataField => "endpoint_metadata_field",
            Self::KonfigurasiPlatform => "konfigurasi_platform",
            Self::AdminUser => "admin_user",
            Self::ProviderSecret => "provider_secret",
        }
    }
}

/// What happened to the object. `Cabut` is the closest the contract has to a
/// delete, so removing a secret value is a revocation, not a new verb.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AksiKonfigurasi {
    Buat,
    Ubah,
    Nonaktifkan,
    Aktifkan,
    Cabut,
}

impl AksiKonfigurasi {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Buat => "buat",
            Self::Ubah => "ubah",
            Self::Nonaktifkan => "nonaktifkan",
            Self::Aktifkan => "aktifkan",
            Self::Cabut => "cabut",
        }
    }
}

/// One `audit_konfigurasi` row, before it is written. `nilai_lama`/`nilai_baru`
/// are JSON descriptions of the change, never secret material.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KonfigurasiBaru {
    pub pelaku: Uuid,
    pub entitas: EntitasKonfigurasi,
    pub entitas_id: Option<Uuid>,
    pub aksi: AksiKonfigurasi,
    pub nilai_lama: Option<serde_json::Value>,
    pub nilai_baru: Option<serde_json::Value>,
    pub alasan: Alasan,
}

/// Writes audit rows inside the caller's transaction.
///
/// The `&mut Transaction` is the whole point: it makes "audit and effect commit
/// or roll back together" a compile-time property instead of a convention. A
/// caller cannot hand over a raw pool, because that would allow the audit row to
/// commit while the role change fails.
#[derive(Debug)]
pub struct Penulis<'tx> {
    tx: &'tx mut Transaction<'static, Postgres>,
}

impl<'tx> Penulis<'tx> {
    pub fn baru(tx: &'tx mut Transaction<'static, Postgres>) -> Self {
        Self { tx }
    }

    /// Write one `audit_akses` row and return its id.
    ///
    /// The id is a UUIDv7 made in the application (migration header rule), so
    /// the caller can reference the row — e.g. to put it in the response — even
    /// though it was written inside a transaction that has not committed yet.
    pub async fn catat_akses(&mut self, baris: AksesBaru) -> Result<Uuid, GalatAudit> {
        if baris.wajib_alasan() && baris.alasan.is_none() {
            return Err(GalatAudit::AlasanHilang { aksi: baris.aksi });
        }

        let audit_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO audit_akses \
             (audit_id, pengguna_id, peran, aksi, tampilan_nik, filter_kueri, nik_hmac, nik_tersamar, \
              jumlah_record, alasan, hasil, sumber_ip, sesi_id) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13)",
        )
        .bind(audit_id)
        .bind(baris.pelaku.pengguna_id)
        .bind(baris.pelaku.peran.as_str())
        .bind(baris.aksi.as_str())
        .bind(baris.tampilan_nik.as_str())
        .bind(baris.filter_kueri)
        .bind(baris.nik_hmac.as_ref().map(NikHmac::as_str))
        .bind(baris.nik_tersamar)
        .bind(baris.jumlah_record.unwrap_or(0))
        .bind(baris.alasan.as_ref().map(Alasan::as_str))
        .bind(baris.hasil.as_str())
        .bind(baris.sumber_ip)
        .bind(baris.sesi_id)
        .execute(&mut **self.tx)
        .await?;

        Ok(audit_id)
    }

    /// Write one `audit_konfigurasi` row and return its id (EN-07/EN-08, AC-5).
    ///
    /// `nilai_lama` and `nilai_baru` never carry a secret value, a credential, a
    /// webhook secret, or a NIK: they describe the change (name, version,
    /// status), not the material. The caller builds them, and the tests of the
    /// calling module assert it.
    pub async fn catat_konfigurasi(&mut self, baris: &KonfigurasiBaru) -> Result<Uuid, GalatAudit> {
        let audit_id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO audit_konfigurasi \
             (audit_konfigurasi_id, pengguna_id, entitas_jenis, entitas_id, aksi, nilai_lama, \
              nilai_baru, alasan) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
        )
        .bind(audit_id)
        .bind(baris.pelaku)
        .bind(baris.entitas.as_str())
        .bind(baris.entitas_id)
        .bind(baris.aksi.as_str())
        .bind(baris.nilai_lama.as_ref())
        .bind(baris.nilai_baru.as_ref())
        .bind(baris.alasan.as_str())
        .execute(&mut **self.tx)
        .await?;

        Ok(audit_id)
    }

    /// Escape hatch for the role-change path, which must write the
    /// `ubah_peran` row **and** revoke the target's sessions in one
    /// transaction (ADR-0003 section 3.6c). Exposed so that logic can live in
    /// one place without copying the transaction type around.
    // `&mut **self.tx` is required here: `&mut *self.tx` is a `&mut
    // Transaction`, which sqlx does not accept as an `Executor`, whereas the
    // deref-coerced `&mut PgConnection` is. Clippy's auto-deref suggestion does
    // not typecheck, so the lint is silenced with that reason.
    #[allow(clippy::explicit_auto_deref)]
    pub fn koneksi(&mut self) -> &mut PgConnection {
        &mut **self.tx
    }
}

/// Login trail (`audit_login`), separate from `audit_akses` (see module docs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AksiLogin {
    Login,
    Logout,
}

impl AksiLogin {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Login => "login",
            Self::Logout => "logout",
        }
    }
}

impl fmt::Display for AksiLogin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Outcome of a login attempt, stored in `audit_login.hasil`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HasilLogin {
    Berhasil,
    Gagal,
    /// Refused by the login limiter before any password work.
    Terkunci,
}

impl HasilLogin {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Berhasil => "berhasil",
            Self::Gagal => "gagal",
            Self::Terkunci => "terkunci",
        }
    }
}

impl fmt::Display for HasilLogin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One login/logout event. `pengguna_id` is `Option` on purpose: a failure for
/// an unknown username has none (module docs, Klaus round 2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginBaru {
    pub pengguna_id: Option<Uuid>,
    pub aksi: AksiLogin,
    pub hasil: HasilLogin,
    /// Recorded as the session id, not the token: `audit_login.sesi_id` is text
    /// and must never become a second copy of the credential.
    pub sesi_id: Option<String>,
    pub sumber_ip: Option<String>,
    pub user_agent: Option<String>,
}

/// Record a login event through `fn_catat_login` (SECURITY DEFINER), which is
/// the only write path granted for this table.
///
/// This runs on the caller's transaction as well: a successful login must not
/// leave a `berhasil` row if issuing the session then fails.
pub async fn catat_login(conn: &mut PgConnection, kejadian: &LoginBaru) -> Result<(), GalatAudit> {
    sqlx::query("SELECT fn_catat_login($1, $2, $3, $4, $5, $6)")
        .bind(kejadian.pengguna_id)
        .bind(kejadian.aksi.as_str())
        .bind(kejadian.hasil.as_str())
        .bind(kejadian.sesi_id.as_deref())
        .bind(kejadian.sumber_ip.as_deref())
        .bind(kejadian.user_agent.as_deref())
        .execute(conn)
        .await?;
    Ok(())
}

/// `terjadi_at` of a row written by this crate's callers. Kept as a helper so
/// tests and the CLI report the same clock the database used rather than a
/// second `Utc::now()` that could differ by a few microseconds.
pub fn waktu_sekarang() -> DateTime<Utc> {
    Utc::now()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pelaku() -> Pelaku {
        Pelaku::baru(Uuid::now_v7(), Peran::Auditor)
    }

    #[test]
    fn peran_round_trips_through_contract_spelling() {
        for p in Peran::SEMUA {
            assert_eq!(p.as_str().parse::<Peran>().unwrap(), p);
        }
        // Case matters: the CLI passes the literal database value.
        assert!("Auditor".parse::<Peran>().is_err());
        assert!("admin".parse::<Peran>().is_err());
    }

    #[test]
    fn peran_display_matches_column_value() {
        assert_eq!(Peran::PlatformAdmin.to_string(), "platform_admin");
        assert_eq!(Peran::ClientOwner.to_string(), "client_owner");
    }

    #[test]
    fn only_auditor_and_operator_need_an_approver() {
        assert!(Peran::Auditor.butuh_persetujuan());
        assert!(Peran::Operator.butuh_persetujuan());
        assert!(!Peran::PlatformAdmin.butuh_persetujuan());
        assert!(!Peran::ClientOwner.butuh_persetujuan());
    }

    #[test]
    fn aksi_values_are_the_column_check_constraint() {
        // Mirrors migrations/20261103000002_request_log.sql. If this list and
        // the constraint drift, every write of the new value fails at runtime
        // in production only, so pin it here.
        assert_eq!(
            Aksi::SEMUA.map(Aksi::as_str),
            [
                "lihat_log",
                "cari_log",
                "cari_nik",
                "lihat_payload",
                "ekspor",
                "lihat_audit",
                "restore_arsip",
                "ubah_peran",
                "akses_ditolak"
            ]
        );
    }

    #[test]
    fn nik_hmac_rejects_plaintext_and_uppercase() {
        let hex = "a".repeat(64);
        assert_eq!(NikHmac::parse(hex.clone()).unwrap().as_str(), hex);

        // A plaintext NIK is 16 digits: must not be mistaken for a digest.
        assert!(NikHmac::parse("3201234567890001").is_err());
        assert!(NikHmac::parse("A".repeat(64)).is_err());
        assert!(NikHmac::parse("g".repeat(64)).is_err());
        assert!(NikHmac::parse("a".repeat(63)).is_err());
        assert!(NikHmac::parse("a".repeat(65)).is_err());
    }

    #[test]
    fn alasan_trims_and_enforces_length() {
        assert_eq!(
            Alasan::baru("  permintaan SPTJM 123/2026  ")
                .unwrap()
                .as_str(),
            "permintaan SPTJM 123/2026"
        );
        assert!(Alasan::baru("pendek").is_err());
        assert!(Alasan::baru("           ").is_err());
        assert!(Alasan::baru("a".repeat(500)).is_ok());
        assert!(Alasan::baru("a".repeat(501)).is_err());
        // Counted in characters, not bytes: an Indonesian reason with accents
        // must not be cut short by a byte-based limit.
        assert!(Alasan::baru("á".repeat(500)).is_ok());
    }

    #[test]
    fn alasan_error_names_the_limit_not_the_value() {
        let e = Alasan::baru("pendek").unwrap_err();
        let msg = e.to_string();
        assert!(msg.contains("10–500"));
        // The rejected text must not leak into the message (contract rule 3:
        // error text names what is wrong, never echoes masked values).
        assert!(!msg.contains("pendek"));
    }

    #[test]
    fn ditolak_always_pairs_akses_ditolak_with_ditolak() {
        let baris = AksesBaru::ditolak(pelaku());
        assert_eq!(baris.aksi, Aksi::AksesDitolak);
        assert_eq!(baris.hasil, Hasil::Ditolak);
        assert_eq!(baris.tampilan_nik, TampilanNik::TidakAda);
    }

    #[test]
    fn a_denied_row_never_asks_for_a_reason() {
        // The refusal is recorded without an auditor reason: the user never got
        // far enough to supply one.
        let baris = AksesBaru::ditolak(pelaku());
        assert!(!baris.wajib_alasan());
    }

    #[test]
    fn auditor_actions_require_a_reason() {
        let p = pelaku();
        for aksi in [
            Aksi::CariNik,
            Aksi::LihatPayload,
            Aksi::Ekspor,
            Aksi::LihatAudit,
        ] {
            assert!(
                AksesBaru::berhasil(p.clone(), aksi).wajib_alasan(),
                "{aksi}"
            );
        }
        for aksi in [
            Aksi::LihatLog,
            Aksi::CariLog,
            Aksi::UbahPeran,
            Aksi::RestoreArsip,
        ] {
            assert!(
                !AksesBaru::berhasil(p.clone(), aksi).wajib_alasan(),
                "{aksi}"
            );
        }
    }

    #[test]
    fn builder_sets_only_what_it_is_given() {
        let hmac = NikHmac::parse("b".repeat(64)).unwrap();
        let baris = AksesBaru::berhasil(pelaku(), Aksi::CariNik)
            .dengan_tampilan_nik(TampilanNik::Utuh)
            .dengan_nik_hmac(hmac.clone())
            .dengan_nik_tersamar("3201********0001")
            .dengan_jumlah_record(3)
            .dengan_alasan(Alasan::baru("permintaan SPTJM 123/2026").unwrap())
            .dengan_sumber_ip("10.0.0.7")
            .dengan_sesi("01a19fc2-0000-7000-8000-000000000000")
            .dengan_filter(serde_json::json!({"status": ["done"]}));

        assert_eq!(baris.aksi, Aksi::CariNik);
        assert_eq!(baris.tampilan_nik, TampilanNik::Utuh);
        assert_eq!(baris.nik_hmac, Some(hmac));
        assert_eq!(baris.nik_tersamar.as_deref(), Some("3201********0001"));
        assert_eq!(baris.jumlah_record, Some(3));
        assert_eq!(baris.sumber_ip.as_deref(), Some("10.0.0.7"));
        assert!(baris.filter_kueri.is_some());
    }

    #[test]
    fn hasil_and_tampilan_display_match_column_values() {
        assert_eq!(Hasil::Berhasil.to_string(), "berhasil");
        assert_eq!(Hasil::Ditolak.to_string(), "ditolak");
        assert_eq!(TampilanNik::Utuh.to_string(), "utuh");
        assert_eq!(TampilanNik::Tersamar.to_string(), "tersamar");
        assert_eq!(TampilanNik::TidakAda.to_string(), "tidak_ada");
        assert_eq!(AksiLogin::Login.to_string(), "login");
        assert_eq!(HasilLogin::Gagal.to_string(), "gagal");
        assert_eq!(HasilLogin::Terkunci.to_string(), "terkunci");
    }
}
