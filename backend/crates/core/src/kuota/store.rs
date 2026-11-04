//! `KuotaStore`: the one Rust door to the quota scripts (SHAA-195, addendum
//! D1.7, ADR-0001 rev 15 section 5 and 5a).
//!
//! Callers (webserver sync path, dispatcher, worker, reconciler) never build
//! `KEYS`/`ARGV` or read a raw script reply. They call a method here and get a
//! typed answer. Three rules live in this file and nowhere else:
//!
//! - **`belum_siap` is mapped once.** Every script answers `belum_siap` when
//!   `pr:siap` does not match the caller's `run_id`. [`GalatStore::BelumSiap`]
//!   is the single representation of that; callers treat it as "Redis is not
//!   available" (sync and intake 503 `layanan_tidak_tersedia`, dispatcher stops).
//!   A transport error is [`GalatStore::Redis`], with the same meaning. There
//!   is no path that turns either into "quota available" (M-02).
//! - **`run_id` follows reconnects.** The store holds the `run_id` it last read
//!   from `INFO server`. After a `belum_siap` it reads it again, so once the
//!   controller has rebuilt Redis (and written `pr:siap` for the live `run_id`)
//!   the next call succeeds without restarting the process.
//! - **`slot_kedaluwarsa` means do not call the upstream.** See
//!   [`HasilTambah::SlotKedaluwarsa`].
//!
//! Period and bucket arguments come from the caller, computed once from
//! `requested_at` (section 5a.1 point 1d); this module never reads a clock.

use std::future::Future;
use std::sync::{Arc, RwLock};

use chrono::{DateTime, Utc};
use redis::aio::ConnectionLike;
use redis::{Script, Value};

use super::pemulihan::{
    JendelaRate, KUNCI_SIAP, baca_run_id, kunci_antre, kunci_attempts, kunci_ember, kunci_retry,
    kunci_rl, kunci_slot, kunci_tolak, member_slot, nomor_ember_absolut, periksa_pool,
};
use super::periode::JUMLAH_EMBER_HARIAN;

const SKRIP_INTAKE: &str = include_str!("lua/rakitan/kuota_intake.lua");
const SKRIP_TOKEN: &str = include_str!("lua/rakitan/kuota_ambil_token.lua");
const SKRIP_KLAIM: &str = include_str!("lua/rakitan/slot_klaim.lua");
const SKRIP_LEPAS: &str = include_str!("lua/rakitan/slot_lepas.lua");
const SKRIP_TAMBAH: &str = include_str!("lua/rakitan/attempts_tambah.lua");
const SKRIP_BACA: &str = include_str!("lua/rakitan/kuota_baca.lua");
const SKRIP_TOLAK: &str = include_str!("lua/rakitan/penolakan_ambil.lua");

/// Why a store call produced no decision. Every variant means "do not let the
/// request through on the strength of Redis".
#[derive(Debug, thiserror::Error)]
pub enum GalatStore {
    /// `pr:siap` does not match this process's `run_id`: Redis restarted or a
    /// replica was promoted and has not been rebuilt yet (section 5a.2).
    #[error("Redis belum siap (pr:siap tidak cocok dengan run_id); menunggu rebuild")]
    BelumSiap,
    #[error("Redis tidak dapat dihubungi atau menolak perintah: {0}")]
    Redis(#[from] redis::RedisError),
    /// A script rejected its arguments. This is a caller or configuration bug,
    /// never a quota decision.
    #[error("skrip menolak argumen: {0}")]
    Skrip(String),
    #[error("jawaban skrip tidak terduga: {0}")]
    JawabanTakTerduga(String),
    #[error("{0}")]
    ArgumenTidakSah(String),
}

/// How `terpakai` is counted (section 5a.1 point 1a).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModeJendela {
    /// Rolling 24 hours of 10-minute buckets (default).
    Bergulir,
    /// Counter of the current period.
    Kalender,
}

impl ModeJendela {
    fn sebagai_arg(self) -> &'static str {
        match self {
            Self::Bergulir => "bergulir",
            Self::Kalender => "kalender",
        }
    }
}

/// What every quota call needs to know about one attempt. `periode_mulai` and
/// `requested_at` come from [`super::periode_untuk`] and the log row; this type
/// is how they reach Redis without a script reading the clock.
#[derive(Debug, Clone)]
pub struct KonteksKuota {
    pub pool_id: String,
    pub periode_mulai: DateTime<Utc>,
    pub requested_at: DateTime<Utc>,
    pub mode: ModeJendela,
    /// `floor(kuota_harian * (1 - margin))`, greater than zero.
    pub kuota_efektif: i64,
}

/// One request for a dispatch/sync token (`kuota_ambil_token.lua`).
#[derive(Debug, Clone)]
pub struct PermintaanToken {
    pub request_id: String,
    /// `attempt_count` of this attempt after the `dispatched -> in_flight` CAS.
    pub nomor_percobaan: i64,
    pub sync: bool,
    pub maks_konkurensi: i64,
    pub ttl_slot_mikro: i64,
    /// Retries allowed per minute; 0 disables the budget.
    pub batas_retry: i64,
    pub porsi_sync: f64,
    pub jendela_rate: Vec<JendelaRate>,
}

/// Intake decision (K-12).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HasilIntake {
    Lolos {
        terpakai: i64,
        antre: i64,
    },
    /// `terpakai >= kuota_efektif`: 429 `kuota_habis`.
    KuotaHabis {
        terpakai: i64,
        antre: i64,
    },
    /// `terpakai + antre >= kuota_efektif`: 429, a different code for metrics.
    AntreanPenuh {
        terpakai: i64,
        antre: i64,
    },
}

/// Numbers every token reply carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusToken {
    pub terpakai: i64,
    pub slot_aktif: i64,
    /// Slots the script cleaned because their TTL passed; feeds
    /// `poolrouter_slot_kedaluwarsa_total` (section 5a.1 point 6).
    pub kedaluwarsa: i64,
    pub tunggu_mikro: i64,
    pub isi: i64,
    pub isi_porsi: i64,
}

/// The M-02 gate. Only [`HasilToken::Diberikan`] allows an attempt to start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HasilToken {
    Diberikan(StatusToken),
    KuotaHabis(StatusToken),
    KonkurensiPenuh(StatusToken),
    RetryBudgetHabis(StatusToken),
    PorsiSyncHabis(StatusToken),
    RateLimit(StatusToken),
    /// The member already holds a slot: a caller bug, refused (fail-closed).
    SlotGanda(StatusToken),
}

impl HasilToken {
    /// Whether the caller may start the attempt.
    pub fn diberikan(&self) -> bool {
        matches!(self, Self::Diberikan(_))
    }

    /// `Retry-After` seconds for the rate-shaped refusals (at least 1,
    /// section 5a.7). `None` for refusals whose recovery time is not known.
    pub fn retry_after_detik(&self) -> Option<u64> {
        match self {
            Self::RateLimit(s) | Self::PorsiSyncHabis(s) => Some(detik_naik(s.tunggu_mikro)),
            _ => None,
        }
    }
}

/// `slot_klaim.lua` outcome (async worker, before the CAS to `in_flight`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HasilKlaim {
    Diperpanjang {
        skor_lama: i64,
        antre: i64,
    },
    /// The slot was never reserved.
    SlotTidakAda,
    /// The slot's score passed. The worker acks without writing or calling; the
    /// reconciler returns the request to `queued` (D1.3).
    SlotKedaluwarsa,
}

/// `attempts_tambah.lua` outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HasilTambah {
    Dicatat {
        attempts: i64,
        terpakai: i64,
        kuota: i64,
    },
    /// Counted, but `terpakai` passed `kuota`: the safety-net alarm. The attempt
    /// is still recorded; the caller raises the alarm metric.
    DicatatTerlampaui {
        attempts: i64,
        terpakai: i64,
        kuota: i64,
    },
    /// The slot is gone (TTL passed). Nothing was counted.
    ///
    /// **The caller must NOT call the upstream.** The attempt has no slot, so
    /// it is not covered by the quota gate. Close the log row as
    /// `tidak_diketahui` and count `poolrouter_slot_kedaluwarsa_total` (D1.4).
    SlotKedaluwarsa { attempts: i64 },
}

impl HasilTambah {
    /// Whether the upstream may be called now.
    pub fn boleh_memanggil_upstream(&self) -> bool {
        !matches!(self, Self::SlotKedaluwarsa { .. })
    }
}

/// `kuota_baca.lua` outcome (read-only; quota panel and 429 `Retry-After`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Bacaan {
    pub terpakai: i64,
    pub attempts: i64,
    pub slot_aktif: i64,
    /// Only filled in `bergulir` mode; 0 otherwise (calendar mode computes it
    /// from the period end).
    pub retry_after_detik: u64,
}

/// Who releases a slot, for the caller's audit log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsalLepas {
    Kas,
    Dispatcher,
    Reconciler,
}

impl AsalLepas {
    fn sebagai_arg(self) -> &'static str {
        match self {
            Self::Kas => "kas",
            Self::Dispatcher => "dispatcher",
            Self::Reconciler => "reconciler",
        }
    }
}

/// Rejection limiter outcome (ADR-0002 rev 5 D6, contract L-01..L-08).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HasilPenolakan {
    /// A token was taken; answer the rejection as usual.
    Lolos { sisa_token: i64 },
    /// Bucket empty: answer 429 `penolakan_berlebih` with no `request` row.
    Dibatasi { tunggu_mikro: i64 },
}

impl HasilPenolakan {
    /// `Retry-After` seconds for [`HasilPenolakan::Dibatasi`] (at least 1).
    pub fn retry_after_detik(&self) -> Option<u64> {
        match self {
            Self::Dibatasi { tunggu_mikro } => Some(detik_naik(*tunggu_mikro)),
            Self::Lolos { .. } => None,
        }
    }
}

/// Microseconds to whole seconds, rounded up, at least 1 (section 5a.7).
pub fn detik_naik(mikro: i64) -> u64 {
    let mikro = mikro.max(0) as u64;
    mikro.div_ceil(1_000_000).max(1)
}

/// Rejection limiter defaults (L-03, L-04). Platform admin may change them per
/// client; the caller passes the configured values.
pub const BURST_PENOLAKAN: i64 = 20;
pub const ISI_ULANG_PENOLAKAN_MIKRO: i64 = 1_000_000;

/// The quota operations, independent of how they are stored.
pub trait KuotaStore: Send + Sync {
    fn intake(
        &self,
        ctx: &KonteksKuota,
        sync: bool,
        client_id: &str,
    ) -> impl Future<Output = Result<HasilIntake, GalatStore>> + Send;

    fn ambil_token(
        &self,
        ctx: &KonteksKuota,
        permintaan: &PermintaanToken,
    ) -> impl Future<Output = Result<HasilToken, GalatStore>> + Send;

    fn klaim_slot(
        &self,
        pool_id: &str,
        request_id: &str,
        nomor_percobaan: i64,
        ttl_slot_mikro: i64,
        dari_antre: bool,
    ) -> impl Future<Output = Result<HasilKlaim, GalatStore>> + Send;

    fn tambah_attempts(
        &self,
        ctx: &KonteksKuota,
        request_id: &str,
        nomor_percobaan: i64,
    ) -> impl Future<Output = Result<HasilTambah, GalatStore>> + Send;

    fn lepas_slot(
        &self,
        pool_id: &str,
        request_id: &str,
        nomor_percobaan: i64,
        asal: AsalLepas,
    ) -> impl Future<Output = Result<bool, GalatStore>> + Send;

    fn baca(&self, ctx: &KonteksKuota) -> impl Future<Output = Result<Bacaan, GalatStore>> + Send;

    /// Take one token of the client's rejection bucket.
    fn ambil_token_penolakan(
        &self,
        client_id: &str,
        isi_ulang_mikro: i64,
        burst: i64,
    ) -> impl Future<Output = Result<HasilPenolakan, GalatStore>> + Send;
}

struct KumpulanSkrip {
    intake: Script,
    token: Script,
    klaim: Script,
    lepas: Script,
    tambah: Script,
    baca: Script,
    tolak: Script,
}

impl KumpulanSkrip {
    fn baru() -> Self {
        Self {
            intake: Script::new(SKRIP_INTAKE),
            token: Script::new(SKRIP_TOKEN),
            klaim: Script::new(SKRIP_KLAIM),
            lepas: Script::new(SKRIP_LEPAS),
            tambah: Script::new(SKRIP_TAMBAH),
            baca: Script::new(SKRIP_BACA),
            tolak: Script::new(SKRIP_TOLAK),
        }
    }
}

/// [`KuotaStore`] over Redis scripts. Cheap to clone; clones share the `run_id`.
pub struct KuotaRedis<C> {
    koneksi: C,
    skrip: Arc<KumpulanSkrip>,
    run_id: Arc<RwLock<String>>,
}

impl<C: Clone> Clone for KuotaRedis<C> {
    fn clone(&self) -> Self {
        Self {
            koneksi: self.koneksi.clone(),
            skrip: Arc::clone(&self.skrip),
            run_id: Arc::clone(&self.run_id),
        }
    }
}

impl<C> KuotaRedis<C>
where
    C: ConnectionLike + Clone + Send + Sync,
{
    /// Connects the store to Redis and reads the live `run_id` once. It does not
    /// check `pr:siap`: a not-ready Redis simply answers `belum_siap`.
    pub async fn baru(mut koneksi: C) -> Result<Self, GalatStore> {
        let run_id = baca_run_id(&mut koneksi).await.map_err(galat_pemulihan)?;
        Ok(Self {
            koneksi,
            skrip: Arc::new(KumpulanSkrip::baru()),
            run_id: Arc::new(RwLock::new(run_id)),
        })
    }

    /// The `run_id` currently sent to the scripts.
    pub fn run_id(&self) -> String {
        self.run_id
            .read()
            .map(|r| r.clone())
            .unwrap_or_else(|racun| racun.into_inner().clone())
    }

    /// Reads `run_id` again from `INFO server`. Called on every `belum_siap` and
    /// available for a caller that reconnects.
    pub async fn segarkan_run_id(&self) -> Result<String, GalatStore> {
        let mut c = self.koneksi.clone();
        let baru = baca_run_id(&mut c).await.map_err(galat_pemulihan)?;
        match self.run_id.write() {
            Ok(mut r) => *r = baru.clone(),
            Err(racun) => *racun.into_inner() = baru.clone(),
        }
        Ok(baru)
    }

    /// Runs one script and returns its reply as typed values. `belum_siap` is
    /// turned into [`GalatStore::BelumSiap`] here, for every script.
    async fn jalankan(
        &self,
        skrip: &Script,
        kunci: Vec<String>,
        mut arg: Vec<String>,
    ) -> Result<Vec<Value>, GalatStore> {
        // ARGV[1] is always `run_id`.
        arg.insert(0, self.run_id());
        let mut c = self.koneksi.clone();
        let mut panggilan = skrip.prepare_invoke();
        for k in &kunci {
            panggilan.key(k);
        }
        for a in &arg {
            panggilan.arg(a);
        }
        let jawab: Vec<Value> = panggilan.invoke_async(&mut c).await?;
        match jawab.first().map(teks).transpose()?.as_deref() {
            Some("belum_siap") => {
                self.segarkan_run_id().await?;
                Err(GalatStore::BelumSiap)
            }
            Some(_) => Ok(jawab),
            None => Err(GalatStore::JawabanTakTerduga("jawaban kosong".into())),
        }
    }
}

fn galat_pemulihan(g: super::pemulihan::GalatPemulihan) -> GalatStore {
    match g {
        super::pemulihan::GalatPemulihan::Redis(e) => GalatStore::Redis(e),
        lain => GalatStore::JawabanTakTerduga(lain.to_string()),
    }
}

fn teks(v: &Value) -> Result<String, GalatStore> {
    match v {
        Value::BulkString(b) => Ok(String::from_utf8_lossy(b).into_owned()),
        Value::SimpleString(s) => Ok(s.clone()),
        lain => Err(GalatStore::JawabanTakTerduga(format!(
            "bukan teks: {lain:?}"
        ))),
    }
}

fn angka(v: Option<&Value>) -> Result<i64, GalatStore> {
    match v {
        Some(Value::Int(i)) => Ok(*i),
        Some(lain @ (Value::BulkString(_) | Value::SimpleString(_))) => teks(lain)?
            .parse()
            .map_err(|_| GalatStore::JawabanTakTerduga("bukan angka".into())),
        lain => Err(GalatStore::JawabanTakTerduga(format!(
            "bukan angka: {lain:?}"
        ))),
    }
}

/// The error a script reports for bad arguments (any code that is not a
/// decision).
fn galat_skrip(kode: &str) -> GalatStore {
    GalatStore::Skrip(kode.to_owned())
}

fn periksa_id(nama: &str, nilai: &str) -> Result<(), GalatStore> {
    periksa_pool(nilai)
        .map_err(|_| GalatStore::ArgumenTidakSah(format!("{nama} {nilai:?} tidak sah")))
}

fn periksa_ctx(ctx: &KonteksKuota) -> Result<(), GalatStore> {
    periksa_id("pool_id", &ctx.pool_id)?;
    if ctx.kuota_efektif < 1 {
        return Err(GalatStore::ArgumenTidakSah(
            "kuota_efektif harus positif".into(),
        ));
    }
    Ok(())
}

fn periksa_member(request_id: &str, nomor: i64) -> Result<String, GalatStore> {
    if request_id.is_empty() || request_id.contains(':') || nomor < 1 {
        return Err(GalatStore::ArgumenTidakSah(format!(
            "request_id {request_id:?} atau nomor_percobaan {nomor} tidak sah"
        )));
    }
    Ok(member_slot(request_id, nomor))
}

fn no_ember(ctx: &KonteksKuota) -> i64 {
    nomor_ember_absolut(ctx.requested_at)
}

impl<C> KuotaStore for KuotaRedis<C>
where
    C: ConnectionLike + Clone + Send + Sync,
{
    async fn intake(
        &self,
        ctx: &KonteksKuota,
        sync: bool,
        client_id: &str,
    ) -> Result<HasilIntake, GalatStore> {
        periksa_ctx(ctx)?;
        periksa_id("client_id", client_id)?;
        let kunci = vec![
            KUNCI_SIAP.to_owned(),
            kunci_attempts(&ctx.pool_id, ctx.periode_mulai),
            kunci_ember(&ctx.pool_id),
            kunci_antre(&ctx.pool_id),
        ];
        let arg = vec![
            ctx.mode.sebagai_arg().to_owned(),
            if sync { "1" } else { "0" }.to_owned(),
            client_id.to_owned(),
            ctx.kuota_efektif.to_string(),
            JUMLAH_EMBER_HARIAN.to_string(),
            no_ember(ctx).to_string(),
        ];
        let j = self.jalankan(&self.skrip.intake, kunci, arg).await?;
        let kode = teks(&j[0])?;
        let terpakai = angka(j.get(1));
        let antre = angka(j.get(2));
        match kode.as_str() {
            "lolos" => Ok(HasilIntake::Lolos {
                terpakai: terpakai?,
                antre: antre?,
            }),
            "kuota_habis" => Ok(HasilIntake::KuotaHabis {
                terpakai: terpakai?,
                antre: antre?,
            }),
            "antrean_penuh" => Ok(HasilIntake::AntreanPenuh {
                terpakai: terpakai?,
                antre: antre?,
            }),
            lain => Err(galat_skrip(lain)),
        }
    }

    async fn ambil_token(
        &self,
        ctx: &KonteksKuota,
        p: &PermintaanToken,
    ) -> Result<HasilToken, GalatStore> {
        periksa_ctx(ctx)?;
        periksa_member(&p.request_id, p.nomor_percobaan)?;
        if p.jendela_rate.is_empty() {
            return Err(GalatStore::ArgumenTidakSah(
                "paling sedikit satu jendela rate".into(),
            ));
        }
        let mut kunci = vec![
            KUNCI_SIAP.to_owned(),
            kunci_attempts(&ctx.pool_id, ctx.periode_mulai),
            kunci_ember(&ctx.pool_id),
            kunci_slot(&ctx.pool_id),
            kunci_retry(&ctx.pool_id),
        ];
        for j in &p.jendela_rate {
            kunci.push(kunci_rl(&ctx.pool_id, &j.jenis, false));
            kunci.push(kunci_rl(&ctx.pool_id, &j.jenis, true));
        }
        let mut arg = vec![
            p.request_id.clone(),
            p.nomor_percobaan.to_string(),
            ctx.mode.sebagai_arg().to_owned(),
            if p.sync { "1" } else { "0" }.to_owned(),
            ctx.kuota_efektif.to_string(),
            JUMLAH_EMBER_HARIAN.to_string(),
            no_ember(ctx).to_string(),
            p.maks_konkurensi.to_string(),
            p.ttl_slot_mikro.to_string(),
            p.batas_retry.to_string(),
            p.porsi_sync.to_string(),
            p.jendela_rate.len().to_string(),
        ];
        for j in &p.jendela_rate {
            arg.push(j.t_mikro.to_string());
            arg.push(j.burst.to_string());
        }
        let j = self.jalankan(&self.skrip.token, kunci, arg).await?;
        let kode = teks(&j[0])?;
        let status = || -> Result<StatusToken, GalatStore> {
            Ok(StatusToken {
                terpakai: angka(j.get(1))?,
                slot_aktif: angka(j.get(2))?,
                kedaluwarsa: angka(j.get(3))?,
                tunggu_mikro: angka(j.get(4))?,
                isi: angka(j.get(5))?,
                isi_porsi: angka(j.get(6))?,
            })
        };
        match kode.as_str() {
            "ok" => Ok(HasilToken::Diberikan(status()?)),
            "kuota_habis" => Ok(HasilToken::KuotaHabis(status()?)),
            "konkurensi_penuh" => Ok(HasilToken::KonkurensiPenuh(status()?)),
            "retry_budget_habis" => Ok(HasilToken::RetryBudgetHabis(status()?)),
            "porsi_sync_habis" => Ok(HasilToken::PorsiSyncHabis(status()?)),
            "rate_limit" => Ok(HasilToken::RateLimit(status()?)),
            "slot_ganda" => Ok(HasilToken::SlotGanda(status()?)),
            lain => Err(galat_skrip(lain)),
        }
    }

    async fn klaim_slot(
        &self,
        pool_id: &str,
        request_id: &str,
        nomor_percobaan: i64,
        ttl_slot_mikro: i64,
        dari_antre: bool,
    ) -> Result<HasilKlaim, GalatStore> {
        periksa_id("pool_id", pool_id)?;
        let member = periksa_member(request_id, nomor_percobaan)?;
        let kunci = vec![
            KUNCI_SIAP.to_owned(),
            kunci_slot(pool_id),
            kunci_antre(pool_id),
        ];
        let arg = vec![
            member,
            ttl_slot_mikro.to_string(),
            if dari_antre { "1" } else { "0" }.to_owned(),
        ];
        let j = self.jalankan(&self.skrip.klaim, kunci, arg).await?;
        match teks(&j[0])?.as_str() {
            "ok" => Ok(HasilKlaim::Diperpanjang {
                skor_lama: angka(j.get(1))?,
                antre: angka(j.get(2))?,
            }),
            "slot_tidak_ada" => Ok(HasilKlaim::SlotTidakAda),
            "slot_kedaluwarsa" => Ok(HasilKlaim::SlotKedaluwarsa),
            lain => Err(galat_skrip(lain)),
        }
    }

    async fn tambah_attempts(
        &self,
        ctx: &KonteksKuota,
        request_id: &str,
        nomor_percobaan: i64,
    ) -> Result<HasilTambah, GalatStore> {
        periksa_ctx(ctx)?;
        let member = periksa_member(request_id, nomor_percobaan)?;
        let kunci = vec![
            KUNCI_SIAP.to_owned(),
            kunci_attempts(&ctx.pool_id, ctx.periode_mulai),
            kunci_ember(&ctx.pool_id),
            kunci_slot(&ctx.pool_id),
        ];
        let arg = vec![
            member,
            ctx.mode.sebagai_arg().to_owned(),
            ctx.kuota_efektif.to_string(),
            JUMLAH_EMBER_HARIAN.to_string(),
            no_ember(ctx).to_string(),
        ];
        let j = self.jalankan(&self.skrip.tambah, kunci, arg).await?;
        let kode = teks(&j[0])?;
        let tiga = || -> Result<(i64, i64, i64), GalatStore> {
            Ok((angka(j.get(1))?, angka(j.get(2))?, angka(j.get(3))?))
        };
        match kode.as_str() {
            "ok" => {
                let (attempts, terpakai, kuota) = tiga()?;
                Ok(HasilTambah::Dicatat {
                    attempts,
                    terpakai,
                    kuota,
                })
            }
            "ok_terlampaui" => {
                let (attempts, terpakai, kuota) = tiga()?;
                Ok(HasilTambah::DicatatTerlampaui {
                    attempts,
                    terpakai,
                    kuota,
                })
            }
            "slot_kedaluwarsa" => Ok(HasilTambah::SlotKedaluwarsa {
                attempts: angka(j.get(1))?,
            }),
            lain => Err(galat_skrip(lain)),
        }
    }

    async fn lepas_slot(
        &self,
        pool_id: &str,
        request_id: &str,
        nomor_percobaan: i64,
        asal: AsalLepas,
    ) -> Result<bool, GalatStore> {
        periksa_id("pool_id", pool_id)?;
        let member = periksa_member(request_id, nomor_percobaan)?;
        let kunci = vec![KUNCI_SIAP.to_owned(), kunci_slot(pool_id)];
        let arg = vec![member, asal.sebagai_arg().to_owned()];
        let j = self.jalankan(&self.skrip.lepas, kunci, arg).await?;
        match teks(&j[0])?.as_str() {
            "ok" => Ok(angka(j.get(1))? > 0),
            lain => Err(galat_skrip(lain)),
        }
    }

    async fn baca(&self, ctx: &KonteksKuota) -> Result<Bacaan, GalatStore> {
        periksa_ctx(ctx)?;
        let kunci = vec![
            KUNCI_SIAP.to_owned(),
            kunci_attempts(&ctx.pool_id, ctx.periode_mulai),
            kunci_ember(&ctx.pool_id),
            kunci_slot(&ctx.pool_id),
        ];
        let arg = vec![
            ctx.mode.sebagai_arg().to_owned(),
            ctx.kuota_efektif.to_string(),
            JUMLAH_EMBER_HARIAN.to_string(),
            no_ember(ctx).to_string(),
        ];
        let j = self.jalankan(&self.skrip.baca, kunci, arg).await?;
        match teks(&j[0])?.as_str() {
            "ok" => Ok(Bacaan {
                terpakai: angka(j.get(1))?,
                attempts: angka(j.get(2))?,
                slot_aktif: angka(j.get(3))?,
                retry_after_detik: angka(j.get(4))?.max(0) as u64,
            }),
            lain => Err(galat_skrip(lain)),
        }
    }

    async fn ambil_token_penolakan(
        &self,
        client_id: &str,
        isi_ulang_mikro: i64,
        burst: i64,
    ) -> Result<HasilPenolakan, GalatStore> {
        periksa_id("client_id", client_id)?;
        let kunci = vec![KUNCI_SIAP.to_owned(), kunci_tolak(client_id)];
        let arg = vec![isi_ulang_mikro.to_string(), burst.to_string()];
        let j = self.jalankan(&self.skrip.tolak, kunci, arg).await?;
        match teks(&j[0])?.as_str() {
            "lolos" => Ok(HasilPenolakan::Lolos {
                sisa_token: angka(j.get(1))?,
            }),
            "dibatasi" => Ok(HasilPenolakan::Dibatasi {
                tunggu_mikro: angka(j.get(1))?,
            }),
            lain => Err(galat_skrip(lain)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_after_dibulatkan_ke_atas_minimal_satu() {
        assert_eq!(detik_naik(0), 1);
        assert_eq!(detik_naik(1), 1);
        assert_eq!(detik_naik(1_000_000), 1);
        assert_eq!(detik_naik(1_000_001), 2);
        assert_eq!(detik_naik(-5), 1);
    }

    fn status(tunggu: i64) -> StatusToken {
        StatusToken {
            terpakai: 0,
            slot_aktif: 0,
            kedaluwarsa: 0,
            tunggu_mikro: tunggu,
            isi: 0,
            isi_porsi: 0,
        }
    }

    #[test]
    fn hanya_diberikan_yang_boleh_memulai_percobaan() {
        assert!(HasilToken::Diberikan(status(0)).diberikan());
        for h in [
            HasilToken::KuotaHabis(status(0)),
            HasilToken::KonkurensiPenuh(status(0)),
            HasilToken::RetryBudgetHabis(status(0)),
            HasilToken::PorsiSyncHabis(status(0)),
            HasilToken::RateLimit(status(0)),
            HasilToken::SlotGanda(status(0)),
        ] {
            assert!(!h.diberikan(), "{h:?}");
        }
    }

    #[test]
    fn retry_after_hanya_untuk_penolakan_berbentuk_rate() {
        assert_eq!(
            HasilToken::RateLimit(status(2_500_000)).retry_after_detik(),
            Some(3)
        );
        assert_eq!(
            HasilToken::PorsiSyncHabis(status(0)).retry_after_detik(),
            Some(1)
        );
        assert_eq!(HasilToken::KuotaHabis(status(9)).retry_after_detik(), None);
        assert_eq!(
            HasilPenolakan::Dibatasi {
                tunggu_mikro: 400_000
            }
            .retry_after_detik(),
            Some(1)
        );
        assert_eq!(
            HasilPenolakan::Lolos { sisa_token: 3 }.retry_after_detik(),
            None
        );
    }

    #[test]
    fn slot_kedaluwarsa_berarti_jangan_panggil_upstream() {
        assert!(!HasilTambah::SlotKedaluwarsa { attempts: 5 }.boleh_memanggil_upstream());
        assert!(
            HasilTambah::Dicatat {
                attempts: 1,
                terpakai: 1,
                kuota: 10
            }
            .boleh_memanggil_upstream()
        );
        // A recorded attempt over quota is an alarm, not a refusal: the attempt
        // already counts, so the call proceeds (the log row exists).
        assert!(
            HasilTambah::DicatatTerlampaui {
                attempts: 11,
                terpakai: 11,
                kuota: 10
            }
            .boleh_memanggil_upstream()
        );
    }

    #[test]
    fn member_dan_id_divalidasi_sebelum_ke_redis() {
        assert!(periksa_member("req-1", 1).is_ok());
        assert!(periksa_member("req:1", 1).is_err());
        assert!(periksa_member("", 1).is_err());
        assert!(periksa_member("req-1", 0).is_err());
        assert!(periksa_id("client_id", "c-1").is_ok());
        assert!(periksa_id("client_id", "c{1}").is_err());
    }
}
