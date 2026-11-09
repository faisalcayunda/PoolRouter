//! `SumberRebuild` over PostgreSQL (ARC-69, ADR-0001 rev 15 section 5a.2,
//! addendum SHAA-195 D1.2 and D2.6).
//!
//! [`super::pemulihan::bangun_ulang`] owns every rule about *how* the Redis
//! state is written. This module only answers *what PostgreSQL says* as of a
//! given instant: one [`KeadaanPool`] per active upstream pool. The queries live
//! here, not in the controller, so the dispatcher and the reconciler of later
//! tickets read the same facts from one place.
//!
//! Three predicates have to stay identical to the ones the rest of the system
//! uses, or the rebuilt counters disagree with the log on the very first
//! request:
//!
//! - the period comes from [`periode_untuk`] over the pool's reset history, and
//!   the log count uses the same `periode_mulai <= requested_at <
//!   periode_selesai` window as DQ-05;
//! - the bucket numbers come from [`nomor_ember`], the same function the Lua
//!   scripts receive;
//! - the semaphore holds `dispatched` plus `in_flight`, the pair the dispatcher
//!   and the worker claim against.
//!
//! All pools are read inside one transaction, so a rebuild never mixes a
//! half-written period with a half-written queue depth.

use chrono::{DateTime, NaiveTime, Utc};
use sqlx::postgres::PgRow;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use super::pemulihan::{GalatPemulihan, JendelaRate, KeadaanPool, SlotAktif, SumberRebuild};
use super::periode::{
    AturanReset, JUMLAH_EMBER_HARIAN, Periode, ZonaTetap, nomor_ember, periode_untuk,
};

/// A slot lives `timeout_total_upstream + 60 s` (section 5a.2): the worker
/// releases it on its own timeout, and the extra minute keeps a rebuild from
/// dropping a slot whose worker is still in flight.
const TTL_SLOT_TAMBAHAN_DETIK: i64 = 60;

/// Timeout assumed when `parameter_dispatch` does not name one. The column is
/// `jsonb NOT NULL DEFAULT '{}'`, so an empty object is a normal state between
/// a pool insert and the dispatcher ticket that fills it in.
const TIMEOUT_UPSTREAM_BAWAAN_DETIK: i64 = 30;

/// Rolling window of the per-bucket counts: `JUMLAH_EMBER_HARIAN` buckets of
/// [`super::periode::MENIT_EMBER`] minutes each, as the scripts derive it.
fn batas_jendela_gulir(sekarang: DateTime<Utc>) -> DateTime<Utc> {
    sekarang
        - chrono::Duration::minutes(i64::from(JUMLAH_EMBER_HARIAN) * super::periode::MENIT_EMBER)
}

/// The PostgreSQL implementation of [`SumberRebuild`].
///
/// Holds a pool, not a transaction: every [`SumberRebuild::keadaan`] call opens
/// its own read-only transaction, so the caller controls `sekarang`.
#[derive(Debug, Clone)]
pub struct SumberPostgres {
    pool: PgPool,
}

impl SumberPostgres {
    pub fn baru(pool: PgPool) -> Self {
        Self { pool }
    }
}

impl SumberRebuild for SumberPostgres {
    async fn keadaan(&self, sekarang: DateTime<Utc>) -> Result<Vec<KeadaanPool>, GalatPemulihan> {
        let mut tx = self
            .pool
            .begin()
            .await
            .map_err(|e| GalatPemulihan::Sumber(format!("buka transaksi: {e}")))?;

        let pools = sqlx::query(
            "SELECT upstream_pool_id, parameter_dispatch,
                    porsi_sync_maks::text AS porsi_sync_maks,
                    reset_kuota_jam, reset_kuota_zona,
                    reset_kuota_berlaku_mulai,
                    reset_kuota_jam_berikut, reset_kuota_zona_berikut
               FROM upstream_pool
              WHERE aktif
              ORDER BY upstream_pool_id",
        )
        .fetch_all(&mut *tx)
        .await
        .map_err(|e| GalatPemulihan::Sumber(format!("baca upstream_pool: {e}")))?;

        let mut hasil = Vec::with_capacity(pools.len());
        for baris in pools {
            hasil.push(satu_pool(&mut tx, &baris, sekarang).await?);
        }

        // Read-only by construction; the commit only releases the snapshot.
        tx.commit()
            .await
            .map_err(|e| GalatPemulihan::Sumber(format!("tutup transaksi: {e}")))?;
        Ok(hasil)
    }
}

async fn satu_pool(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    baris: &PgRow,
    sekarang: DateTime<Utc>,
) -> Result<KeadaanPool, GalatPemulihan> {
    let pool_id = kolom::<Uuid>(baris, "upstream_pool_id")?;
    let periode = periode_dari_baris(baris, sekarang)?;

    let snapshot_attempts = jumlah_snapshot(tx, pool_id, periode.mulai).await?;
    let log_periode = jumlah_log(tx, pool_id, periode).await?;
    let log_ember = ember_pool(tx, pool_id, sekarang).await?;
    let antre = antre_pool(tx, pool_id).await?;
    let slot = slot_pool(tx, pool_id).await?;

    let parameter = kolom::<sqlx::types::Json<serde_json::Value>>(baris, "parameter_dispatch")?.0;
    let jendela_rate = jendela_rate(&parameter)?;
    let porsi_sync = kolom::<String>(baris, "porsi_sync_maks")?
        .parse::<f64>()
        .map_err(|e| GalatPemulihan::Sumber(format!("porsi_sync_maks bukan angka: {e}")))?;
    let timeout_detik = parameter
        .get("timeout_total_upstream_detik")
        .and_then(serde_json::Value::as_i64)
        .unwrap_or(TIMEOUT_UPSTREAM_BAWAAN_DETIK);

    Ok(KeadaanPool {
        pool_id: pool_id.to_string(),
        periode_mulai: periode.mulai,
        snapshot_attempts,
        log_periode,
        log_ember,
        antre,
        slot,
        ttl_slot_mikro: (timeout_detik + TTL_SLOT_TAMBAHAN_DETIK) * 1_000_000,
        jendela_rate,
        porsi_sync,
    })
}

fn kolom<'r, T>(baris: &'r PgRow, nama: &str) -> Result<T, GalatPemulihan>
where
    T: sqlx::Decode<'r, sqlx::Postgres> + sqlx::Type<sqlx::Postgres>,
{
    baris
        .try_get(nama)
        .map_err(|e| GalatPemulihan::Sumber(format!("kolom {nama}: {e}")))
}

/// The period containing `sekarang`, from the pool's reset configuration.
///
/// `upstream_pool` has no reset-history table: it holds the rule in force
/// (`reset_kuota_jam`, `reset_kuota_zona`) plus, when a change is scheduled, the
/// rule that replaces it (`reset_kuota_*_berikut`) and the instant it takes
/// effect (`reset_kuota_berlaku_mulai`, which KP-02 pins to a period boundary),
/// so the history handed to [`periode_untuk`] has at most two entries.
///
/// The current rule has no start of its own, so it is anchored one day before
/// `sekarang`. That is always early enough: the period containing *now* began
/// within the last 24 h of its own reset hour, so an anchor a day back precedes
/// it, and [`periode_untuk`] walks forward from the first entry.
///
/// Public because the snapshot job (`poolrouter-controller`) has to write the
/// same `periode_mulai` the rebuild later looks up; two implementations of this
/// arithmetic would let the snapshot and the rebuild disagree on the key (M-02).
pub fn periode_dari_baris(
    baris: &PgRow,
    sekarang: DateTime<Utc>,
) -> Result<Periode, GalatPemulihan> {
    let jam = kolom::<NaiveTime>(baris, "reset_kuota_jam")?;
    let zona = kolom::<String>(baris, "reset_kuota_zona")?;
    let berlaku = kolom::<Option<DateTime<Utc>>>(baris, "reset_kuota_berlaku_mulai")?;
    let jam_berikut = kolom::<Option<NaiveTime>>(baris, "reset_kuota_jam_berikut")?;
    let zona_berikut = kolom::<Option<String>>(baris, "reset_kuota_zona_berikut")?;

    let mut riwayat = vec![AturanReset::baru(
        sekarang - chrono::Duration::days(1),
        jam,
        zona_tetap(&zona)?,
    )];

    // KP-02 keeps the three `*_berikut`/`berlaku_mulai` columns all-or-none
    // (`reset_berikut_lengkap`), so a partially scheduled change cannot occur.
    if let (Some(berlaku), Some(jam_b), Some(zona_b)) = (berlaku, jam_berikut, zona_berikut) {
        // A change already in the past has taken effect; `periode_untuk` would
        // ignore an entry before its predecessor, so only a future change is
        // appended. Ordering by `berlaku_mulai` is what the walk requires.
        if berlaku > sekarang - chrono::Duration::days(1) {
            riwayat.push(AturanReset::baru(berlaku, jam_b, zona_tetap(&zona_b)?));
        }
    }

    periode_untuk(sekarang, &riwayat).map_err(GalatPemulihan::Kuota)
}

/// Zone from an IANA name. `upstream_pool`'s trigger (KP-01) only accepts the
/// fixed-offset zones of this deployment plus `UTC`.
fn zona_tetap(nama: &str) -> Result<ZonaTetap, GalatPemulihan> {
    let detik = match nama {
        "Asia/Jakarta" | "Asia/Pontianak" => 7 * 3600,
        "Asia/Makassar" => 8 * 3600,
        "Asia/Jayapura" => 9 * 3600,
        "UTC" => 0,
        lain => {
            return Err(GalatPemulihan::Sumber(format!(
                "reset_kuota_zona {lain:?} bukan zona tetap yang dikenal"
            )));
        }
    };
    ZonaTetap::dari_detik(detik)
        .ok_or_else(|| GalatPemulihan::Sumber(format!("offset {detik} detik tidak sah")))
}

async fn jumlah_snapshot(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    pool_id: Uuid,
    periode_mulai: DateTime<Utc>,
) -> Result<i64, GalatPemulihan> {
    let jumlah: Option<i64> = sqlx::query_scalar(
        "SELECT jumlah
           FROM upstream_attempt_daily
          WHERE upstream_pool_id = $1
            AND periode_mulai = $2",
    )
    .bind(pool_id)
    .bind(periode_mulai)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| GalatPemulihan::Sumber(format!("baca snapshot: {e}")))?;
    Ok(jumlah.unwrap_or(0))
}

async fn jumlah_log(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    pool_id: Uuid,
    periode: Periode,
) -> Result<i64, GalatPemulihan> {
    let jumlah: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM transaction_log
          WHERE upstream_pool_id = $1
            AND requested_at >= $2
            AND requested_at < $3",
    )
    .bind(pool_id)
    .bind(periode.mulai)
    .bind(periode.selesai)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| GalatPemulihan::Sumber(format!("hitung log periode: {e}")))?;
    Ok(jumlah)
}

/// `transaction_log` rows per absolute bucket number for the last 24h10m.
async fn ember_pool(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    pool_id: Uuid,
    sekarang: DateTime<Utc>,
) -> Result<Vec<(i64, i64)>, GalatPemulihan> {
    let baris = sqlx::query(
        "SELECT requested_at
           FROM transaction_log
          WHERE upstream_pool_id = $1
            AND requested_at >= $2",
    )
    .bind(pool_id)
    .bind(batas_jendela_gulir(sekarang))
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| GalatPemulihan::Sumber(format!("baca log ember: {e}")))?;

    let mut per_ember = std::collections::BTreeMap::<i64, i64>::new();
    for b in baris {
        let requested_at = kolom::<DateTime<Utc>>(&b, "requested_at")?;
        let nomor = nomor_ember(requested_at).map_err(GalatPemulihan::Kuota)?;
        *per_ember.entry(nomor).or_insert(0) += 1;
    }
    Ok(per_ember.into_iter().collect())
}

/// `queued` plus `dispatched`: what the pool still owes, claimed or not.
async fn antre_pool(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    pool_id: Uuid,
) -> Result<i64, GalatPemulihan> {
    let jumlah: i64 = sqlx::query_scalar(
        "SELECT COUNT(*)
           FROM request
          WHERE upstream_pool_id = $1
            AND status IN ('queued', 'dispatched')",
    )
    .bind(pool_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| GalatPemulihan::Sumber(format!("hitung antre: {e}")))?;
    Ok(jumlah)
}

/// The semaphore occupants: `dispatched` and `in_flight` with their attempt.
async fn slot_pool(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    pool_id: Uuid,
) -> Result<Vec<SlotAktif>, GalatPemulihan> {
    let baris = sqlx::query(
        "SELECT request_id, attempt_count
           FROM request
          WHERE upstream_pool_id = $1
            AND status IN ('dispatched', 'in_flight')",
    )
    .bind(pool_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| GalatPemulihan::Sumber(format!("baca slot: {e}")))?;

    baris
        .into_iter()
        .map(|b| {
            Ok(SlotAktif {
                request_id: kolom::<Uuid>(&b, "request_id")?.to_string(),
                nomor_percobaan: i64::from(kolom::<i32>(&b, "attempt_count")?),
            })
        })
        .collect()
}

/// The GCRA windows of the pool, from `parameter_dispatch.rate`.
///
/// The column is a jsonb document without a database constraint on its shape
/// (see `docs/db/skema-f0.md`), so a wrong shape is reported here rather than
/// silently rebuilt as "no rate limit": fail-closed (ADR-0001 section 5).
/// Dipakai juga jalur sync webserver (ARC-66) supaya token sync membaca
/// jendela rate dengan aturan yang sama dengan rebuild.
pub fn jendela_rate(parameter: &serde_json::Value) -> Result<Vec<JendelaRate>, GalatPemulihan> {
    let Some(rate) = parameter.get("rate") else {
        return Ok(Vec::new());
    };
    let rate = rate.as_object().ok_or_else(|| {
        GalatPemulihan::DataTidakSah("parameter_dispatch.rate bukan objek".to_owned())
    })?;

    let mut hasil = Vec::new();
    for jenis in ["tps", "tpm", "tph"] {
        let Some(nilai) = rate.get(jenis) else {
            continue;
        };
        let nilai = nilai.as_object().ok_or_else(|| {
            GalatPemulihan::DataTidakSah(format!("parameter_dispatch.rate.{jenis} bukan objek"))
        })?;
        let t_mikro = nilai
            .get("t_mikro")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| {
                GalatPemulihan::DataTidakSah(format!(
                    "parameter_dispatch.rate.{jenis}.t_mikro tidak ada atau bukan bilangan"
                ))
            })?;
        let burst = nilai
            .get("burst")
            .and_then(serde_json::Value::as_i64)
            .ok_or_else(|| {
                GalatPemulihan::DataTidakSah(format!(
                    "parameter_dispatch.rate.{jenis}.burst tidak ada atau bukan bilangan"
                ))
            })?;
        hasil.push(JendelaRate {
            jenis: jenis.to_owned(),
            t_mikro,
            burst,
        });
    }
    Ok(hasil)
}
