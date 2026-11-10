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

/// Konkurensi bawaan bila `parameter_dispatch.maks_konkurensi` tidak diisi.
/// Kuota tetap dijaga skrip token; angka ini hanya membatasi panggilan paralel.
pub const MAKS_KONKURENSI_BAWAAN: i64 = 10;

/// Upstream pool rute pertama yang aktif untuk `r.downstream_pool_id`, menurut
/// `upstream_pool_id` (asumsi spec `f0-controller-worker`: F0 satu rute per
/// downstream pool; CW4). Fragmen SQL dengan alias `r` untuk tabel `request`.
pub const SQL_RUTE_PERTAMA: &str = "(SELECT rt.upstream_pool_id
           FROM downstream_pool_rute rt
           JOIN upstream_pool u ON u.upstream_pool_id = rt.upstream_pool_id
          WHERE rt.downstream_pool_id = r.downstream_pool_id AND u.aktif
          ORDER BY rt.upstream_pool_id
          LIMIT 1)";

/// Pool yang counter `antre`-nya memuat request `r` (CW3, AC-25): upstream pool
/// yang sudah terisi saat dispatch, atau rute pertama yang aktif bila request
/// belum pernah di-dispatch (EN-04: `upstream_pool_id` baru terisi saat
/// dispatch). Rebuild, dispatcher, dan penyapu memakai fragmen yang sama.
pub fn sql_pool_antre() -> String {
    format!("COALESCE(r.upstream_pool_id, {SQL_RUTE_PERTAMA})")
}

/// Isi `upstream_pool.parameter_dispatch` yang dipakai dispatcher dan rebuild
/// (bentuknya dikunci ARC-70; `docs/db/skema-f0.md`):
///
/// ```json
/// { "rate": { "tps": { "t_mikro": 100000, "burst": 1 } },
///   "timeout_total_upstream_detik": 20,
///   "maks_konkurensi": 10,
///   "jeda_acak_min_mikro": 20000,
///   "jeda_acak_maks_mikro": 100000 }
/// ```
///
/// Semua kunci boleh absen; `rate` yang absen berarti dispatcher tidak bisa
/// mengambil token (fail-closed di skrip), bukan "tanpa batas".
#[derive(Debug, Clone, PartialEq)]
pub struct ParameterDispatch {
    pub jendela_rate: Vec<JendelaRate>,
    pub timeout_total_upstream_detik: i64,
    pub maks_konkurensi: i64,
    /// Batas atas jeda acak antar-publish K-05 yang diminta admin, sebelum
    /// dipotong ke satu slot token. `None` = satu slot token penuh.
    pub jeda_acak_maks_mikro: Option<i64>,
    /// Batas bawah jeda acak K-05 yang diminta admin (`jeda_min`), sebelum
    /// dipotong ke batas atas. `None` = 0.
    pub jeda_acak_min_mikro: Option<i64>,
}

impl ParameterDispatch {
    pub fn dari_json(parameter: &serde_json::Value) -> Result<Self, GalatPemulihan> {
        let angka_opsional = |kunci: &str| -> Result<Option<i64>, GalatPemulihan> {
            match parameter.get(kunci) {
                None | Some(serde_json::Value::Null) => Ok(None),
                Some(v) => v.as_i64().filter(|n| *n >= 0).map(Some).ok_or_else(|| {
                    GalatPemulihan::DataTidakSah(format!(
                        "parameter_dispatch.{kunci} bukan bilangan bulat >= 0"
                    ))
                }),
            }
        };
        let maks_konkurensi = angka_opsional("maks_konkurensi")?.unwrap_or(MAKS_KONKURENSI_BAWAAN);
        if maks_konkurensi < 1 {
            return Err(GalatPemulihan::DataTidakSah(
                "parameter_dispatch.maks_konkurensi harus >= 1".to_owned(),
            ));
        }
        Ok(Self {
            jendela_rate: jendela_rate(parameter)?,
            timeout_total_upstream_detik: angka_opsional("timeout_total_upstream_detik")?
                .unwrap_or(TIMEOUT_UPSTREAM_BAWAAN_DETIK),
            maks_konkurensi,
            jeda_acak_maks_mikro: angka_opsional("jeda_acak_maks_mikro")?,
            jeda_acak_min_mikro: angka_opsional("jeda_acak_min_mikro")?,
        })
    }

    /// TTL slot = timeout total upstream + 60 s (§5a.1 butir 5), dalam mikrodetik.
    pub fn ttl_slot_mikro(&self) -> i64 {
        (self.timeout_total_upstream_detik + TTL_SLOT_TAMBAHAN_DETIK) * 1_000_000
    }

    /// Ambang reconciler dan penyapu log: timeout total upstream + 60 s.
    pub fn ambang_reconciler_detik(&self) -> i64 {
        self.timeout_total_upstream_detik + TTL_SLOT_TAMBAHAN_DETIK
    }

    /// Rentang jeda acak K-05 `(min, maks)` dalam µs: di dalam satu slot token,
    /// yaitu `1/rate` jendela terketat (t_mikro terkecil), dan tidak lebih dari
    /// permintaan admin. Dipotong 1 s supaya jendela TPM/TPH tidak membuat
    /// dispatcher tidur lama. `min` dipotong ke `0..=maks`.
    pub fn rentang_jeda_mikro(&self) -> (i64, i64) {
        let slot = self
            .jendela_rate
            .iter()
            .map(|j| j.t_mikro)
            .filter(|t| *t > 0)
            .min()
            .unwrap_or(0)
            .min(1_000_000);
        let maks = match self.jeda_acak_maks_mikro {
            Some(maks) => maks.clamp(0, slot),
            None => slot,
        };
        let min = self.jeda_acak_min_mikro.unwrap_or(0).clamp(0, maks);
        (min, maks)
    }
}

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
    let parameter = ParameterDispatch::dari_json(&parameter)?;
    let porsi_sync = kolom::<String>(baris, "porsi_sync_maks")?
        .parse::<f64>()
        .map_err(|e| GalatPemulihan::Sumber(format!("porsi_sync_maks bukan angka: {e}")))?;

    Ok(KeadaanPool {
        pool_id: pool_id.to_string(),
        periode_mulai: periode.mulai,
        snapshot_attempts,
        log_periode,
        log_ember,
        antre,
        slot,
        ttl_slot_mikro: parameter.ttl_slot_mikro(),
        jendela_rate: parameter.jendela_rate,
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
/// The rule in force has no start of its own, so it is anchored one day before
/// `sekarang`. That is always early enough: the period containing *now* began
/// within the last 24 h of its own reset hour, so an anchor a day back precedes
/// it, and [`periode_untuk`] walks forward from the first entry. A scheduled
/// change whose `berlaku_mulai` has passed is the rule in force, however long
/// ago it took effect (see [`periode_dari_aturan`]).
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

    periode_dari_aturan(
        jam,
        &zona,
        berlaku,
        jam_berikut,
        zona_berikut.as_deref(),
        sekarang,
    )
}

/// Isi [`periode_dari_baris`] tanpa baris PostgreSQL, supaya aritmetikanya bisa
/// diuji langsung.
///
/// Jadwal `*_berikut` yang `berlaku_mulai`-nya sudah lewat adalah aturan aktif,
/// berapa pun umurnya (ARC-232): tidak ada job yang menaikkannya ke kolom aktif,
/// jadi pembacalah yang menerapkannya. Hanya jadwal yang masih di masa depan
/// yang menunggu.
fn periode_dari_aturan(
    jam: NaiveTime,
    zona: &str,
    berlaku: Option<DateTime<Utc>>,
    jam_berikut: Option<NaiveTime>,
    zona_berikut: Option<&str>,
    sekarang: DateTime<Utc>,
) -> Result<Periode, GalatPemulihan> {
    let jangkar = sekarang - chrono::Duration::days(1);
    let mut riwayat = vec![AturanReset::baru(jangkar, jam, zona_tetap(zona)?)];

    // KP-02 keeps the three `*_berikut`/`berlaku_mulai` columns all-or-none
    // (`reset_berikut_lengkap`), so a partially scheduled change cannot occur.
    if let (Some(berlaku), Some(jam_b), Some(zona_b)) = (berlaku, jam_berikut, zona_berikut) {
        let berikut = zona_tetap(zona_b)?;
        if berlaku > jangkar {
            // Perubahan di dalam 24 jam terakhir atau yang akan datang: jadi
            // entri kedua, urut menurut `berlaku_mulai` seperti yang dituntut
            // `periode_untuk`.
            riwayat.push(AturanReset::baru(berlaku, jam_b, berikut));
        } else {
            // Perubahan yang sudah berlaku lebih dari sehari: aturan lama sudah
            // tidak memegang periode mana pun yang relevan. Aturan baru yang
            // menjangkar di sini sah karena reset pertamanya sesudah
            // `berlaku_mulai` jatuh paling lama 24 jam kemudian, yaitu sebelum
            // `sekarang`, jadi periode yang memuat `sekarang` dimulai pada
            // reset sungguhan aturan baru.
            riwayat[0] = AturanReset::baru(jangkar, jam_b, berikut);
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
///
/// Request `queued` yang belum pernah di-dispatch belum punya
/// `upstream_pool_id` (EN-04), jadi pool-nya diturunkan lewat
/// [`sql_pool_antre`], kunci yang sama dengan koreksi `antre` di controller
/// (ARC-72, AC-25).
async fn antre_pool(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    pool_id: Uuid,
) -> Result<i64, GalatPemulihan> {
    let sql = format!(
        "SELECT COUNT(*)
           FROM request r
          WHERE r.status IN ('queued', 'dispatched')
            AND {} = $1",
        sql_pool_antre()
    );
    let jumlah: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(sql.as_str()))
        .bind(pool_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e| GalatPemulihan::Sumber(format!("hitung antre: {e}")))?;
    Ok(jumlah)
}

/// The semaphore occupants: `dispatched` and `in_flight` with their attempt.
///
/// Member slot memakai nomor percobaan yang *akan* atau *sedang* berjalan:
/// dispatcher mengambil token untuk `attempt_count + 1` (request `dispatched`
/// belum menaikkan `attempt_count`), sedangkan `in_flight` sudah menaikkannya.
/// Tanpa `+ 1` itu rebuild menolak slot `dispatched` percobaan pertama
/// (`nomor_percobaan` 0) dan worker tidak menemukan slotnya (ARC-70).
async fn slot_pool(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    pool_id: Uuid,
) -> Result<Vec<SlotAktif>, GalatPemulihan> {
    let baris = sqlx::query(
        "SELECT request_id,
                CASE WHEN status = 'dispatched' THEN attempt_count + 1 ELSE attempt_count END
                  AS attempt_count
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

#[cfg(test)]
mod uji_parameter {
    use super::*;

    #[test]
    fn parameter_kosong_memakai_bawaan() {
        let p = ParameterDispatch::dari_json(&serde_json::json!({})).expect("objek kosong sah");
        assert!(p.jendela_rate.is_empty());
        assert_eq!(p.maks_konkurensi, MAKS_KONKURENSI_BAWAAN);
        assert_eq!(
            p.timeout_total_upstream_detik,
            TIMEOUT_UPSTREAM_BAWAAN_DETIK
        );
        assert_eq!(p.ttl_slot_mikro(), (30 + 60) * 1_000_000);
        assert_eq!(p.rentang_jeda_mikro(), (0, 0), "tanpa rate tidak ada jeda");
    }

    #[test]
    fn jeda_k05_berada_di_dalam_slot_token() {
        let p = ParameterDispatch::dari_json(&serde_json::json!({
            "rate": { "tps": { "t_mikro": 100_000, "burst": 1 },
                      "tph": { "t_mikro": 360_000_000, "burst": 10 } },
            "jeda_acak_maks_mikro": 500_000
        }))
        .expect("parameter sah");
        // 10 TPS: satu slot = 100 ms; permintaan 500 ms dipotong ke slot.
        assert_eq!(p.rentang_jeda_mikro(), (0, 100_000));

        let kecil = ParameterDispatch::dari_json(&serde_json::json!({
            "rate": { "tps": { "t_mikro": 100_000, "burst": 1 } },
            "jeda_acak_maks_mikro": 20_000
        }))
        .expect("parameter sah");
        assert_eq!(kecil.rentang_jeda_mikro(), (0, 20_000));

        let lambat = ParameterDispatch::dari_json(&serde_json::json!({
            "rate": { "tpm": { "t_mikro": 6_000_000, "burst": 1 } }
        }))
        .expect("parameter sah");
        assert_eq!(lambat.rentang_jeda_mikro(), (0, 1_000_000), "dipotong 1 s");
    }

    /// ARC-233: batas bawah jeda ikut terbaca dan dipotong ke `0..=maks`.
    #[test]
    fn jeda_min_k05_dipotong_ke_rentang_maks() {
        let rentang = |min: i64, maks: i64| {
            ParameterDispatch::dari_json(&serde_json::json!({
                "rate": { "tps": { "t_mikro": 100_000, "burst": 1 } },
                "jeda_acak_min_mikro": min,
                "jeda_acak_maks_mikro": maks
            }))
            .expect("parameter sah")
            .rentang_jeda_mikro()
        };
        assert_eq!(rentang(20_000, 60_000), (20_000, 60_000));
        assert_eq!(rentang(80_000, 60_000), (60_000, 60_000), "min <= maks");
        assert_eq!(rentang(0, 500_000), (0, 100_000), "maks dipotong slot");
        assert_eq!(
            rentang(500_000, 500_000),
            (100_000, 100_000),
            "min ikut slot"
        );

        let tanpa_maks = ParameterDispatch::dari_json(&serde_json::json!({
            "rate": { "tps": { "t_mikro": 100_000, "burst": 1 } },
            "jeda_acak_min_mikro": 30_000
        }))
        .expect("parameter sah");
        assert_eq!(tanpa_maks.rentang_jeda_mikro(), (30_000, 100_000));

        let negatif = ParameterDispatch::dari_json(&serde_json::json!({
            "jeda_acak_min_mikro": -1
        }));
        assert!(negatif.is_err(), "min negatif ditolak seperti maks");
    }

    #[test]
    fn konkurensi_nol_atau_negatif_ditolak() {
        assert!(
            ParameterDispatch::dari_json(&serde_json::json!({ "maks_konkurensi": 0 })).is_err()
        );
        assert!(
            ParameterDispatch::dari_json(&serde_json::json!({ "maks_konkurensi": -1 })).is_err()
        );
        assert!(
            ParameterDispatch::dari_json(&serde_json::json!({ "maks_konkurensi": "x" })).is_err()
        );
    }
}

#[cfg(test)]
mod uji_periode {
    use super::*;
    use chrono::TimeZone;

    fn utc(hari: u32, jam: u32, menit: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, hari, jam, menit, 0).unwrap()
    }

    fn jam(h: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, 0, 0).unwrap()
    }

    /// Aturan lama 00:00 Jakarta (reset 17:00 UTC); aturan baru 12:00 Jakarta
    /// (reset 05:00 UTC). Sekarang 10 Nov 09:30 UTC.
    fn periode(berlaku: Option<DateTime<Utc>>) -> Periode {
        let (jam_b, zona_b) =
            berlaku.map_or((None, None), |_| (Some(jam(12)), Some("Asia/Jakarta")));
        periode_dari_aturan(
            jam(0),
            "Asia/Jakarta",
            berlaku,
            jam_b,
            zona_b,
            utc(10, 9, 30),
        )
        .expect("periode sah")
    }

    #[test]
    fn tanpa_jadwal_memakai_aturan_aktif() {
        assert_eq!(periode(None).mulai, utc(9, 17, 0));
        assert_eq!(periode(None).selesai, utc(10, 17, 0));
    }

    #[test]
    fn jadwal_di_masa_depan_menunggu() {
        let p = periode(Some(utc(10, 17, 0)));
        assert_eq!(p.mulai, utc(9, 17, 0));
        assert_eq!(p.selesai, utc(10, 17, 0));
    }

    #[test]
    fn jadwal_yang_berlaku_dalam_24_jam_terakhir_dipakai() {
        let p = periode(Some(utc(9, 17, 0)));
        assert_eq!(p.mulai, utc(10, 5, 0));
        assert_eq!(p.selesai, utc(11, 5, 0));
    }

    #[test]
    fn jadwal_yang_sudah_berlaku_lebih_dari_sehari_dipakai() {
        // ARC-232: dulu jatuh kembali ke aturan lama (periode 9 Nov 17:00).
        let p = periode(Some(utc(7, 17, 0)));
        assert_eq!(p.mulai, utc(10, 5, 0));
        assert_eq!(p.selesai, utc(11, 5, 0));
    }

    #[test]
    fn jadwal_tepat_sehari_lalu_dipakai() {
        let p = periode(Some(utc(9, 9, 30)));
        assert_eq!(p.mulai, utc(10, 5, 0));
        assert_eq!(p.selesai, utc(11, 5, 0));
    }

    #[test]
    fn jadwal_lama_sekali_dan_zona_berbeda_dipakai() {
        // Aturan baru 06:00 UTC, berlaku sejak bulan lalu.
        let p = periode_dari_aturan(
            jam(0),
            "Asia/Jakarta",
            Some(utc(1, 17, 0)),
            Some(jam(6)),
            Some("UTC"),
            utc(10, 9, 30),
        )
        .expect("periode sah");
        assert_eq!(p.mulai, utc(10, 6, 0));
        assert_eq!(p.selesai, utc(11, 6, 0));
    }
}
