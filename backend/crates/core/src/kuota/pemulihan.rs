//! Failover and restart detection, fail-closed readiness and rebuild of the
//! Redis quota state (ADR-0001 rev 15 section 5 and section 5a.2, addendum
//! SHAA-195 D1.2, D1.7, D2.6).
//!
//! The contract in short:
//!
//! 1. Every Lua script compares `GET pr:siap` with the `run_id` its caller read
//!    from `INFO server`. A restart or a replica promotion changes `run_id`, so
//!    every script answers `belum_siap` until somebody rebuilds the state.
//! 2. [`periksa_kesiapan`] is how the controller notices that: it reads the live
//!    `run_id` and `pr:siap`.
//! 3. [`bangun_ulang`] rebuilds the counters from PostgreSQL facts handed in by a
//!    [`SumberRebuild`] and writes `pr:siap` as its very last step. Until then
//!    the scripts keep refusing, so a half-built state is never used to hand out
//!    a token (M-02).
//!
//! This module does not talk to PostgreSQL. The facts it needs are plain data
//! ([`KeadaanPool`]); how they are queried is the job of the [`SumberRebuild`]
//! implementation, which keeps the rebuild rules testable without a database.

use std::future::Future;

use chrono::{DateTime, Utc};
use redis::aio::ConnectionLike;

use super::periode::{GalatKuota, JUMLAH_EMBER_HARIAN, MENIT_EMBER};

/// Readiness marker, `KEYS[1]` of every quota script (D1.7).
pub const KUNCI_SIAP: &str = "pr:siap";

/// Slack added to the TTL of a TAT key so it never expires before the bucket
/// has refilled (D2.4). Same value as `kuota_ambil_token.lua`.
const SLACK_TAT_MS: i64 = 60_000;

/// Why a recovery step could not complete. Every variant means "Redis is not
/// usable for quota decisions"; callers keep failing closed.
#[derive(Debug, thiserror::Error)]
pub enum GalatPemulihan {
    #[error("Redis tidak dapat dihubungi atau menolak perintah: {0}")]
    Redis(#[from] redis::RedisError),
    #[error("`INFO server` tidak memuat run_id; Redis tidak dapat diverifikasi")]
    RunIdTidakAda,
    #[error(
        "run_id berubah selama rebuild (dari {awal} ke {sekarang}); rebuild dibatalkan dan harus diulang"
    )]
    RunIdBerubah { awal: String, sekarang: String },
    #[error("pool_id {0:?} tidak sah untuk kunci Redis")]
    PoolTidakSah(String),
    #[error("{0}")]
    Kuota(#[from] GalatKuota),
    #[error("sumber rebuild gagal: {0}")]
    Sumber(String),
    #[error("data rebuild tidak sah: {0}")]
    DataTidakSah(String),
}

// ---------------------------------------------------------------- kunci (D1.7)

/// Pool ids go into the key between braces (a hash tag), so they may not
/// contain anything that changes how the key parses.
pub(super) fn periksa_pool(pool: &str) -> Result<(), GalatPemulihan> {
    let sah = !pool.is_empty()
        && pool
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if sah {
        Ok(())
    } else {
        Err(GalatPemulihan::PoolTidakSah(pool.to_owned()))
    }
}

/// `pr:attempts:{<pool>}:<periode_mulai_unix>`.
pub fn kunci_attempts(pool: &str, periode_mulai: DateTime<Utc>) -> String {
    format!("pr:attempts:{{{pool}}}:{}", periode_mulai.timestamp())
}

/// `pr:ember:{<pool>}`, one hash per pool and not per period (D1.8).
pub fn kunci_ember(pool: &str) -> String {
    format!("pr:ember:{{{pool}}}")
}

/// `pr:antre:{<pool>}`.
pub fn kunci_antre(pool: &str) -> String {
    format!("pr:antre:{{{pool}}}")
}

/// `pr:antre:dp:{<pool>}:<downstream_pool_id>`.
pub fn kunci_antre_dp(pool: &str, downstream_pool_id: i64) -> String {
    format!("pr:antre:dp:{{{pool}}}:{downstream_pool_id}")
}

/// `pr:slot:{<pool>}`.
pub fn kunci_slot(pool: &str) -> String {
    format!("pr:slot:{{{pool}}}")
}

/// `pr:anggaran:{<pool>}`, burden per budget source (AC-6). Fields are
/// `<sumber>:<ember>` and hold the number of committed attempts.
pub fn kunci_anggaran(pool: &str) -> String {
    format!("pr:anggaran:{{{pool}}}")
}

/// `pr:slot_sumber:{<pool>}`, slot member → budget source (AC-6).
pub fn kunci_slot_sumber(pool: &str) -> String {
    format!("pr:slot_sumber:{{{pool}}}")
}

/// `pr:retry:{<pool>}`.
pub fn kunci_retry(pool: &str) -> String {
    format!("pr:retry:{{{pool}}}")
}

/// `pr:rl:{<pool>}:<jenis>` or `pr:rl:{<pool>}:<jenis>:sync`.
pub fn kunci_rl(pool: &str, jenis: &str, porsi_sync: bool) -> String {
    if porsi_sync {
        format!("pr:rl:{{{pool}}}:{jenis}:sync")
    } else {
        format!("pr:rl:{{{pool}}}:{jenis}")
    }
}

/// `pr:tolak:{<client_id>}`, the rejection limiter of one client (ADR-0002 rev 5
/// D6). Separate from every pool key.
pub fn kunci_tolak(client: &str) -> String {
    format!("pr:tolak:{{{client}}}")
}

/// Slot member `<request_id>:<nomor_percobaan>` (D1.2). One function, so a slot
/// the rebuild recreates can be released by its owner.
pub fn member_slot(request_id: &str, nomor_percobaan: i64) -> String {
    format!("{request_id}:{nomor_percobaan}")
}

/// Bucket number of an instant, the unit of the `b<nomor>` hash fields (D2.8).
pub fn nomor_ember_absolut(waktu: DateTime<Utc>) -> i64 {
    waktu.timestamp().div_euclid(MENIT_EMBER * 60)
}

// ------------------------------------------------------------- deteksi failover

/// `run_id` from the text `INFO server` returns, or `None`.
pub fn urai_run_id(info: &str) -> Option<&str> {
    info.lines()
        .find_map(|baris| baris.trim().strip_prefix("run_id:"))
        .map(str::trim)
        .filter(|id| !id.is_empty())
}

/// The `run_id` of the Redis this connection is talking to right now.
pub async fn baca_run_id<C: ConnectionLike>(koneksi: &mut C) -> Result<String, GalatPemulihan> {
    let info: String = redis::cmd("INFO")
        .arg("server")
        .query_async(koneksi)
        .await?;
    urai_run_id(&info)
        .map(str::to_owned)
        .ok_or(GalatPemulihan::RunIdTidakAda)
}

/// Whether the quota scripts will accept calls from a process holding `run_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Kesiapan {
    /// `pr:siap` equals the live `run_id`; the scripts answer normally.
    Siap { run_id: String },
    /// Missing or different `pr:siap`: restart or promotion, scripts answer
    /// `belum_siap`. `tercatat` is what `pr:siap` held, if anything.
    PerluRebuild {
        run_id: String,
        tercatat: Option<String>,
    },
}

/// The check the controller leader runs (section 5a.2 point 3) and the probe
/// behind `/readyz` (section 5a.9).
pub async fn periksa_kesiapan<C: ConnectionLike>(
    koneksi: &mut C,
) -> Result<Kesiapan, GalatPemulihan> {
    let run_id = baca_run_id(koneksi).await?;
    let tercatat: Option<String> = redis::cmd("GET")
        .arg(KUNCI_SIAP)
        .query_async(koneksi)
        .await?;
    Ok(if tercatat.as_deref() == Some(run_id.as_str()) {
        Kesiapan::Siap { run_id }
    } else {
        Kesiapan::PerluRebuild { run_id, tercatat }
    })
}

// ---------------------------------------------------------------------- rebuild

/// One open attempt that holds a semaphore slot: a request in `dispatched` or
/// `in_flight`, with the `attempt_count` of that attempt (D1.2).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlotAktif {
    pub request_id: String,
    pub nomor_percobaan: i64,
}

/// One GCRA window of an upstream pool: `jenis` is `tps`, `tpm` or `tph`,
/// `t_mikro` the spacing between tokens and `burst` the bucket capacity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JendelaRate {
    pub jenis: String,
    pub t_mikro: i64,
    pub burst: i64,
}

/// Everything the rebuild of one upstream pool needs from PostgreSQL.
#[derive(Debug, Clone, PartialEq)]
pub struct KeadaanPool {
    pub pool_id: String,
    /// Period containing `sekarang`, from [`super::periode_untuk`].
    pub periode_mulai: DateTime<Utc>,
    /// `jumlah` of `upstream_attempt_daily` for this `periode_mulai`, 0 if the
    /// controller never snapshotted it.
    pub snapshot_attempts: i64,
    /// `COUNT` of `transaction_log` rows with `periode_mulai <= requested_at <
    /// periode_selesai` (same predicate as DQ-05).
    pub log_periode: i64,
    /// `transaction_log` rows per bucket number for the last 24h10m (section
    /// 5a.2). Buckets outside the rolling window are ignored.
    pub log_ember: Vec<(i64, i64)>,
    /// `queued` plus `dispatched` (K-12).
    pub antre: i64,
    /// Requests in `dispatched` and `in_flight`.
    pub slot: Vec<SlotAktif>,
    /// `timeout_total_upstream + 60 s`, in microseconds.
    pub ttl_slot_mikro: i64,
    /// Rate windows configured for the pool.
    pub jendela_rate: Vec<JendelaRate>,
    /// Sync share of each window, `0..=1` (K-10).
    pub porsi_sync: f64,
}

/// PostgreSQL facts for a rebuild. The `sqlx` implementation lives in
/// `core::kuota` as well (decision Shu, SHAA-195, 2026-11-04 07:39 UTC): the
/// controller only calls [`pulihkan_bila_perlu`], and the webserver reads the
/// same queries for the quota tracker. Tests implement it with fixed data.
pub trait SumberRebuild {
    /// The facts for every active upstream pool, as of `sekarang`.
    fn keadaan(
        &self,
        sekarang: DateTime<Utc>,
    ) -> impl Future<Output = Result<Vec<KeadaanPool>, GalatPemulihan>> + Send;
}

/// Mutual exclusion between concurrent rebuilders (review 225be8d, blocking 1).
/// Held only while one rebuild reads its facts and writes; it expires on its own
/// if the holder dies, so a crashed controller cannot wedge recovery.
pub const KUNCI_REBUILD: &str = "pr:rebuild";
const TTL_KUNCI_REBUILD_MS: i64 = 30_000;

/// Outcome of [`bangun_ulang`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HasilRebuild {
    /// This call rebuilt Redis and wrote `pr:siap`.
    Dibangun(RingkasanRebuild),
    /// `pr:siap` already equals `run_id` (another rebuild finished first). Nothing
    /// was written: state that is live now is newer than any fact read earlier.
    SudahSiap,
    /// Another rebuild holds [`KUNCI_REBUILD`]. Nothing was written; try again on
    /// the next tick.
    DikerjakanLain,
}

/// What a rebuild wrote, for logs and the `poolrouter_redis_rebuild_total`
/// metric.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RingkasanRebuild {
    pub run_id: String,
    pub pool: usize,
    /// `(pool_id, nilai counter attempts)` after the rebuild.
    pub attempts: Vec<(String, i64)>,
}

/// Counter value after a rebuild: the larger of the snapshot and the log count
/// (section 5, "selalu memilih angka yang lebih konservatif untuk M-02").
pub fn nilai_counter(snapshot: i64, log: i64) -> i64 {
    snapshot.max(log).max(0)
}

/// TAT a GCRA key gets right after a rebuild (D2.6): `now + tau` with
/// `tau = (burst - 1) * T`, which leaves exactly one token and no burst.
pub fn tat_setelah_rebuild(sekarang_mikro: i64, t_mikro: i64, burst: i64) -> i64 {
    sekarang_mikro + (burst - 1).max(0) * t_mikro
}

/// Burst of the sync-share bucket of a window; same rule as the script.
pub fn burst_porsi(burst: i64, porsi: f64) -> i64 {
    (((burst as f64) * porsi).floor() as i64).max(1)
}

fn periksa_keadaan(k: &KeadaanPool) -> Result<(), GalatPemulihan> {
    periksa_pool(&k.pool_id)?;
    let negatif = [
        ("snapshot_attempts", k.snapshot_attempts),
        ("log_periode", k.log_periode),
        ("antre", k.antre),
    ];
    for (nama, nilai) in negatif {
        if nilai < 0 {
            return Err(GalatPemulihan::DataTidakSah(format!(
                "{nama} negatif untuk pool {}",
                k.pool_id
            )));
        }
    }
    if k.ttl_slot_mikro < 1 {
        return Err(GalatPemulihan::DataTidakSah(format!(
            "ttl_slot_mikro harus positif untuk pool {}",
            k.pool_id
        )));
    }
    if !(0.0..=1.0).contains(&k.porsi_sync) {
        return Err(GalatPemulihan::DataTidakSah(format!(
            "porsi_sync di luar 0..1 untuk pool {}",
            k.pool_id
        )));
    }
    for j in &k.jendela_rate {
        let jenis_sah = !j.jenis.is_empty()
            && j.jenis
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit());
        if !jenis_sah || j.t_mikro < 1 || j.burst < 1 {
            return Err(GalatPemulihan::DataTidakSah(format!(
                "jendela rate {:?} tidak sah untuk pool {}",
                j.jenis, k.pool_id
            )));
        }
    }
    for s in &k.slot {
        if s.request_id.is_empty() || s.request_id.contains(':') || s.nomor_percobaan < 1 {
            return Err(GalatPemulihan::DataTidakSah(format!(
                "slot {:?} tidak sah untuk pool {}",
                s.request_id, k.pool_id
            )));
        }
    }
    Ok(())
}

/// Redis clock in microseconds. The scripts judge slot scores and TATs against
/// `TIME`, so the rebuild writes them in the same clock.
async fn waktu_redis_mikro<C: ConnectionLike>(koneksi: &mut C) -> Result<i64, GalatPemulihan> {
    let (detik, mikro): (i64, i64) = redis::cmd("TIME").query_async(koneksi).await?;
    Ok(detik * 1_000_000 + mikro)
}

/// Read the facts and write the quota state plus `pr:siap` (see
/// [`bangun_ulang`] for the guards around it).
///
/// What it writes per pool (section 5a.2 point 3, D1.2, D2.6):
/// - `attempts:{pool}:{periode_mulai}` = max(snapshot, log count);
/// - the rolling bucket hash, from the log, inside the 145-bucket window;
/// - `antre`, from PostgreSQL;
/// - the semaphore, one member per open attempt with a full TTL (conservative);
/// - every GCRA key (global and sync share) at `now + tau`, so recovery does
///   not produce a burst toward the upstream.
///
/// Existing keys of these kinds are deleted first, because after a promotion a
/// replica may hold partial or stale values.
async fn tulis_rebuild<C, S>(
    koneksi: &mut C,
    run_id: &str,
    sumber: &S,
    sekarang: DateTime<Utc>,
) -> Result<RingkasanRebuild, GalatPemulihan>
where
    C: ConnectionLike,
    S: SumberRebuild,
{
    let pool = sumber.keadaan(sekarang).await?;
    for k in &pool {
        periksa_keadaan(k)?;
    }

    let sekarang_redis = waktu_redis_mikro(koneksi).await?;
    let ember_sekarang = nomor_ember_absolut(sekarang);
    let ember_terlama = ember_sekarang - (i64::from(JUMLAH_EMBER_HARIAN) - 1);

    let mut ringkasan = Vec::with_capacity(pool.len());
    let mut pipa = redis::pipe();
    pipa.atomic();
    for k in &pool {
        let counter = nilai_counter(k.snapshot_attempts, k.log_periode);
        ringkasan.push((k.pool_id.clone(), counter));

        pipa.cmd("SET")
            .arg(kunci_attempts(&k.pool_id, k.periode_mulai))
            .arg(counter)
            .ignore();

        let ember = kunci_ember(&k.pool_id);
        pipa.cmd("DEL").arg(&ember).ignore();
        for (nomor, jumlah) in &k.log_ember {
            if *jumlah > 0 && (ember_terlama..=ember_sekarang).contains(nomor) {
                pipa.cmd("HINCRBY")
                    .arg(&ember)
                    .arg(format!("b{nomor}"))
                    .arg(*jumlah)
                    .ignore();
            }
        }

        pipa.cmd("SET")
            .arg(kunci_antre(&k.pool_id))
            .arg(k.antre)
            .ignore();

        let slot = kunci_slot(&k.pool_id);
        pipa.cmd("DEL").arg(&slot).ignore();
        for s in &k.slot {
            pipa.cmd("ZADD")
                .arg(&slot)
                .arg("NX")
                .arg(sekarang_redis + k.ttl_slot_mikro)
                .arg(member_slot(&s.request_id, s.nomor_percobaan))
                .ignore();
        }

        // The retry budget restarts empty: one minute of lost retries only
        // makes the budget more permissive for at most a minute, and the
        // attempts counter above already bounds M-02.
        pipa.cmd("DEL").arg(kunci_retry(&k.pool_id)).ignore();

        for j in &k.jendela_rate {
            for porsi in [false, true] {
                let burst = if porsi {
                    burst_porsi(j.burst, k.porsi_sync)
                } else {
                    j.burst
                };
                let tat = tat_setelah_rebuild(sekarang_redis, j.t_mikro, burst);
                let ttl_ms = (tat - sekarang_redis) / 1000 + SLACK_TAT_MS;
                pipa.cmd("SET")
                    .arg(kunci_rl(&k.pool_id, &j.jenis, porsi))
                    .arg(tat)
                    .arg("PX")
                    .arg(ttl_ms)
                    .ignore();
            }
        }
    }
    // `pr:siap` is the last command of the same MULTI/EXEC as the writes, so
    // there is no instant at which the state is built but not marked, or marked
    // but not built. If Redis failed over between the `run_id` check and this
    // EXEC, the marker names a `run_id` that is no longer live and the scripts
    // keep answering `belum_siap` (fail-closed); the next tick rebuilds again.
    pipa.cmd("SET").arg(KUNCI_SIAP).arg(run_id).ignore();
    pipa.query_async::<()>(koneksi).await?;

    Ok(RingkasanRebuild {
        run_id: run_id.to_owned(),
        pool: pool.len(),
        attempts: ringkasan,
    })
}

/// Rebuild the quota state for `run_id` and mark Redis ready, unless somebody
/// else already did or is doing it.
///
/// Two rebuilders may race (two controllers, or a standby that takes over
/// mid-tick). Without a guard the slower one would overwrite state that is
/// already serving: `DEL pr:slot:*` drops claimed slots (concurrency above
/// `maks_konkurensi`) and the GCRA keys go back to "one token", both toward
/// M-02 (review 225be8d, blocking 1). So, in order:
///
/// 1. the live `run_id` must still be `run_id`;
/// 2. take [`KUNCI_REBUILD`] (`SET NX PX`); if held, return `DikerjakanLain`;
/// 3. **only then** read `pr:siap`: if it already equals `run_id`, return
///    `SudahSiap` without reading or writing anything else;
/// 4. read the facts and write everything plus `pr:siap` in one MULTI/EXEC;
/// 5. release the lock (best effort; it also expires by itself).
///
/// The check in step 3 happens under the lock, and the lock is released only
/// after `pr:siap` is written, so a rebuilder that takes the lock later always
/// sees the marker of the one before it. This module does not depend on a
/// single leader controller (that guard is SHAA-199).
///
/// Clocks: the bucket window uses `sekarang` (application clock, the same one
/// that stamps `requested_at`), while slot scores and TATs use the Redis `TIME`
/// because the scripts compare them with that clock. The two differ by NTP
/// skew only, which section 5a.1 already accepts.
///
/// Fail-closed by construction: if anything fails before the EXEC, Redis stays
/// "not ready" and the caller retries.
pub async fn bangun_ulang<C, S>(
    koneksi: &mut C,
    run_id: &str,
    sumber: &S,
    sekarang: DateTime<Utc>,
) -> Result<HasilRebuild, GalatPemulihan>
where
    C: ConnectionLike,
    S: SumberRebuild,
{
    let aktual = baca_run_id(koneksi).await?;
    if aktual != run_id {
        return Err(GalatPemulihan::RunIdBerubah {
            awal: run_id.to_owned(),
            sekarang: aktual,
        });
    }

    let diambil: Option<String> = redis::cmd("SET")
        .arg(KUNCI_REBUILD)
        .arg(run_id)
        .arg("NX")
        .arg("PX")
        .arg(TTL_KUNCI_REBUILD_MS)
        .query_async(koneksi)
        .await?;
    if diambil.is_none() {
        return Ok(HasilRebuild::DikerjakanLain);
    }

    let hasil = async {
        let tercatat: Option<String> = redis::cmd("GET")
            .arg(KUNCI_SIAP)
            .query_async(koneksi)
            .await?;
        if tercatat.as_deref() == Some(run_id) {
            return Ok(HasilRebuild::SudahSiap);
        }
        tulis_rebuild(koneksi, run_id, sumber, sekarang)
            .await
            .map(HasilRebuild::Dibangun)
    }
    .await;

    // Release even when the rebuild failed, so the retry does not wait for the
    // TTL. A failed release only delays the next attempt by the TTL.
    let _ = redis::cmd("DEL")
        .arg(KUNCI_REBUILD)
        .query_async::<()>(koneksi)
        .await;
    hasil
}

/// One controller tick: if Redis is not ready for its live `run_id`, rebuild it.
/// Returns `None` when nothing needed doing or another rebuild is in charge.
pub async fn pulihkan_bila_perlu<C, S>(
    koneksi: &mut C,
    sumber: &S,
    sekarang: DateTime<Utc>,
) -> Result<Option<RingkasanRebuild>, GalatPemulihan>
where
    C: ConnectionLike,
    S: SumberRebuild,
{
    match periksa_kesiapan(koneksi).await? {
        Kesiapan::Siap { .. } => Ok(None),
        Kesiapan::PerluRebuild { run_id, .. } => {
            match bangun_ulang(koneksi, &run_id, sumber, sekarang).await? {
                HasilRebuild::Dibangun(ringkasan) => {
                    super::metrik::redis_rebuild(super::metrik::Komponen::Controller);
                    Ok(Some(ringkasan))
                }
                HasilRebuild::SudahSiap | HasilRebuild::DikerjakanLain => Ok(None),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn utc(y: i32, m: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, mi, 0).unwrap()
    }

    #[test]
    fn run_id_dibaca_dari_info_server() {
        let info = "# Server\r\nredis_version:7.4.1\r\nrun_id:0a1b2c3d4e5f\r\ntcp_port:6379\r\n";
        assert_eq!(urai_run_id(info), Some("0a1b2c3d4e5f"));
    }

    #[test]
    fn info_tanpa_run_id_tidak_diterima() {
        assert_eq!(urai_run_id("# Server\r\nredis_version:7.4.1\r\n"), None);
        assert_eq!(urai_run_id("run_id:\r\n"), None);
        assert_eq!(urai_run_id(""), None);
    }

    #[test]
    fn kunci_mengikuti_tabel_d1_7() {
        let mulai = utc(2026, 11, 3, 17, 0);
        assert_eq!(
            kunci_attempts("p1", mulai),
            format!("pr:attempts:{{p1}}:{}", mulai.timestamp())
        );
        assert_eq!(kunci_ember("p1"), "pr:ember:{p1}");
        assert_eq!(kunci_antre("p1"), "pr:antre:{p1}");
        assert_eq!(kunci_slot("p1"), "pr:slot:{p1}");
        assert_eq!(kunci_retry("p1"), "pr:retry:{p1}");
        assert_eq!(kunci_rl("p1", "tps", false), "pr:rl:{p1}:tps");
        assert_eq!(kunci_rl("p1", "tph", true), "pr:rl:{p1}:tph:sync");
        assert_eq!(member_slot("req-1", 2), "req-1:2");
        assert_eq!(kunci_tolak("c1"), "pr:tolak:{c1}");
    }

    #[test]
    fn pool_id_tidak_boleh_mengubah_arti_kunci() {
        for buruk in ["", "a:b", "a{b", "a}b", "a b", "a\n"] {
            assert!(matches!(
                periksa_pool(buruk),
                Err(GalatPemulihan::PoolTidakSah(_))
            ));
        }
        assert!(periksa_pool("0194f2a0-7b1c-7cde-8f00-123456789abc").is_ok());
    }

    #[test]
    fn counter_memilih_angka_terbesar() {
        assert_eq!(nilai_counter(10, 25), 25);
        assert_eq!(nilai_counter(30, 25), 30);
        assert_eq!(nilai_counter(0, 0), 0);
    }

    #[test]
    fn tat_rebuild_menyisakan_satu_token() {
        // tau = (burst - 1) * T; the script lets a call through when
        // tat - tau <= now, so exactly one passes.
        assert_eq!(tat_setelah_rebuild(1_000, 100, 5), 1_000 + 4 * 100);
        assert_eq!(tat_setelah_rebuild(1_000, 100, 1), 1_000);
    }

    #[test]
    fn burst_porsi_minimal_satu() {
        assert_eq!(burst_porsi(10, 0.3), 3);
        assert_eq!(burst_porsi(10, 0.0), 1);
        assert_eq!(burst_porsi(1, 0.5), 1);
    }

    #[test]
    fn nomor_ember_absolut_satuan_ember() {
        assert_eq!(nomor_ember_absolut(utc(1970, 1, 1, 0, 9)), 0);
        assert_eq!(nomor_ember_absolut(utc(1970, 1, 1, 0, 10)), 1);
    }

    fn contoh() -> KeadaanPool {
        KeadaanPool {
            pool_id: "p1".into(),
            periode_mulai: utc(2026, 11, 3, 17, 0),
            snapshot_attempts: 0,
            log_periode: 0,
            log_ember: vec![],
            antre: 0,
            slot: vec![],
            ttl_slot_mikro: 1_000_000,
            jendela_rate: vec![],
            porsi_sync: 0.2,
        }
    }

    #[test]
    fn keadaan_tidak_sah_ditolak() {
        let mut k = contoh();
        assert!(periksa_keadaan(&k).is_ok());

        k.antre = -1;
        assert!(periksa_keadaan(&k).is_err());

        let mut k = contoh();
        k.ttl_slot_mikro = 0;
        assert!(periksa_keadaan(&k).is_err());

        let mut k = contoh();
        k.porsi_sync = 1.5;
        assert!(periksa_keadaan(&k).is_err());

        let mut k = contoh();
        k.jendela_rate = vec![JendelaRate {
            jenis: "TPS".into(),
            t_mikro: 1,
            burst: 1,
        }];
        assert!(periksa_keadaan(&k).is_err());

        let mut k = contoh();
        k.slot = vec![SlotAktif {
            request_id: "a:b".into(),
            nomor_percobaan: 1,
        }];
        assert!(periksa_keadaan(&k).is_err());
    }
}
