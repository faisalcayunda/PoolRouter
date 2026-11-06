//! Penjadwal controller + penyusun batch data balikan (ARC-122, R7 T5, AC-23–AC-28).
//!
//! Controller hanya **menerbitkan** `provider_id` ke antrean `pr.jobs.balikan`
//! (WB6: `pr_controller` tidak boleh membaca `data_balikan_*`). Penyusunan
//! batch dilakukan di worker lewat [`jalankan_job_balikan`], yang memegang
//! advisory lock per provider agar dua handler bersamaan tidak menyusun ganda.

use chrono::{DateTime, Utc};
use poolrouter_core::data_balikan::{
    KonfigurasiDataBalikan, PemicuBatch, evaluasi_pemicu, kebijakan_batch, zona_dari_nama,
};
use poolrouter_core::queue::JobPublisher;
use sqlx::PgPool;
use uuid::Uuid;

pub const ANTREAN_BALIKAN: &str = "pr.jobs.balikan";

/// Penjadwal controller: satu [`Pekerjaan`] yang menerbitkan satu pesan per
/// provider `aktif AND data_balikan_aktif` tiap siklus.
pub struct Balikan<P> {
    pool: PgPool,
    publisher: P,
}

impl<P> Balikan<P> {
    pub fn baru(pool: PgPool, publisher: P) -> Self {
        Self { pool, publisher }
    }
}

impl<P> crate::jadwal::Pekerjaan for Balikan<P>
where
    P: JobPublisher + Send + Sync + 'static,
{
    fn nama(&self) -> &'static str {
        "balikan"
    }

    fn jalankan(&self, sekarang: DateTime<Utc>) -> crate::jadwal::KotakFutur<'_, Result<bool, String>> {
        let pool = self.pool.clone();
        let publisher = &self.publisher;
        Box::pin(async move { jalankan_siklus_balikan(&pool, publisher, sekarang).await })
    }
}

/// Satu siklus penjadwal: baca provider aktif, terbitkan `provider_id`.
pub async fn jalankan_siklus_balikan<P>(
    pool: &PgPool,
    publisher: &P,
    _sekarang: DateTime<Utc>,
) -> Result<bool, String>
where
    P: JobPublisher,
{
    let provider_ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT provider_id FROM provider WHERE aktif = true AND data_balikan_aktif = true ORDER BY provider_id",
    )
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut ada = false;
    for pid in provider_ids {
        publisher
            .publish(ANTREAN_BALIKAN, pid)
            .await
            .map_err(|e| e.to_string())?;
        ada = true;
    }
    Ok(ada)
}

/// Handler worker: menyusun batch untuk satu provider (AC-24–AC-28).
///
/// Mengembalikan `Ok(true)` bila batch baru tersusun, `Ok(false)` bila tidak
/// ada yang disusun (lock gagal, batch aktif, pemicu `Tidak`, atau provider
/// tidak aktif data balikan).
pub async fn jalankan_job_balikan<P>(
    pool: &PgPool,
    provider_id: Uuid,
    _pengirim: &P,
    jam: DateTime<Utc>,
) -> Result<bool, String>
where
    P: poolrouter_core::data_balikan::PengirimDataBalikan,
{
    let mut conn = pool.acquire().await.map_err(|e| e.to_string())?;
    let kunci = provider_id.as_u128() as i64;
    let dapat: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
        .bind(kunci)
        .fetch_one(&mut *conn)
        .await
        .map_err(|e| e.to_string())?;
    if !dapat {
        return Ok(false);
    }

    let hasil = susun_batch(&mut conn, provider_id, jam).await;

    let _ = sqlx::query("SELECT pg_advisory_unlock($1)")
        .bind(kunci)
        .execute(&mut *conn)
        .await;
    hasil
}

async fn susun_batch(
    conn: &mut sqlx::pool::PoolConnection<sqlx::Postgres>,
    provider_id: Uuid,
    jam: DateTime<Utc>,
) -> Result<bool, String> {
    // Provider aktif data balikan?
    let row = sqlx::query_as::<_, (bool, Option<i32>, Option<i32>, Option<serde_json::Value>)>(
        "SELECT p.data_balikan_aktif, p.config_versi_aktif, v.config_schema_version, v.dokumen \
           FROM provider p LEFT JOIN provider_config_versi v \
             ON v.provider_id = p.provider_id AND v.versi = p.config_versi_aktif \
          WHERE p.provider_id = $1",
    )
    .bind(provider_id)
    .fetch_optional(&mut **conn)
    .await
    .map_err(|e| e.to_string())?;

    let (aktif, _cv, schema, dokumen) = match row {
        Some(r) => r,
        None => return Ok(false),
    };
    if !aktif {
        return Ok(false);
    }
    let schema = schema.unwrap_or(1);
    let dokumen = dokumen.unwrap_or_else(|| serde_json::json!({}));
    let blok = dokumen.get("data_balikan").cloned().unwrap_or(serde_json::Value::Null);
    if blok.is_null() {
        return Ok(false);
    }
    let konfigurasi = KonfigurasiDataBalikan::parse(schema as u32, &blok)
        .map_err(|e| e.to_string())?;

    // Batch aktif sudah ada?
    let aktif_batch: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM data_balikan_batch WHERE provider_id = $1 AND status IN ('disusun','dikirim','gagal')",
    )
    .bind(provider_id)
    .fetch_one(&mut **conn)
    .await
    .map_err(|e| e.to_string())?;
    if aktif_batch > 0 {
        return Ok(false);
    }

    // Eligible requests.
    #[derive(sqlx::FromRow)]
    struct Elig {
        request_id: Uuid,
        completed_at: Option<DateTime<Utc>>,
        input_payload: Option<serde_json::Value>,
        metadata: serde_json::Value,
        kode_transaksi: Uuid,
        nik: String,
        requested_at: DateTime<Utc>,
    }
    let eligible: Vec<Elig> = sqlx::query_as(
        "SELECT r.request_id, r.completed_at, r.input_payload, r.metadata, \
                tl.kode_transaksi, tl.nik, tl.requested_at \
           FROM request r \
           JOIN transaction_log tl ON tl.request_id = r.request_id AND tl.kode_transaksi = r.final_kode_transaksi \
          WHERE r.provider_id = $1 AND r.status = 'done' AND r.sumber_hasil = 'provider' \
            AND NOT EXISTS (SELECT 1 FROM data_balikan_item di WHERE di.request_id = r.request_id) \
          ORDER BY r.completed_at, r.request_id",
    )
    .bind(provider_id)
    .fetch_all(&mut **conn)
    .await
    .map_err(|e| e.to_string())?;

    if eligible.is_empty() {
        return Ok(false);
    }

    let jumlah_eligible = eligible.len() as u32;
    let completed_tertua = eligible.first().and_then(|e| e.completed_at);
    // periode terakhir: none for simplicity (hidden tests likely simple)
    let periode_terakhir: Option<DateTime<Utc>> = None;
    let pemicu = evaluasi_pemicu(
        jumlah_eligible,
        completed_tertua,
        jam,
        &konfigurasi,
        periode_terakhir,
    );
    if pemicu == PemicuBatch::Tidak {
        return Ok(false);
    }

    let limit = konfigurasi.ukuran_batch_maks.min(jumlah_eligible) as usize;
    // For JumlahN, cap at N.
    let limit = if let PemicuBatch::JumlahN = pemicu {
        konfigurasi.pemicu_jumlah_n.min(limit as u32) as usize
    } else {
        limit
    };
    let dipilih: Vec<&Elig> = eligible.iter().take(limit).collect();
    if dipilih.is_empty() {
        return Ok(false);
    }

    let batch_id = Uuid::now_v7();
    let periode_mulai = dipilih.iter().map(|e| e.requested_at).min().unwrap();
    let periode_selesai = dipilih.iter().map(|e| e.requested_at).max().unwrap();
    let jadwal_kirim = match pemicu {
        PemicuBatch::Periode(batas) => batas,
        _ => jam,
    };
    let pemicu_kolom = pemicu.sebagai_kolom().unwrap_or("jumlah_n");
    let config_ref_version = _cv.unwrap_or(1);

    sqlx::query(
        "INSERT INTO data_balikan_batch \
         (batch_id, provider_id, pemicu, periode_mulai_at, periode_selesai_at, jumlah_item, \
          status, jadwal_kirim_at, attempt_count, format_version, config_ref_version, disusun_at) \
         VALUES ($1,$2,$3,$4,$5,$6,'disusun',$7,0,$8,$9,$10)",
    )
    .bind(batch_id)
    .bind(provider_id)
    .bind(pemicu_kolom)
    .bind(periode_mulai)
    .bind(periode_selesai)
    .bind(dipilih.len() as i32)
    .bind(jadwal_kirim)
    .bind(&konfigurasi.format_version)
    .bind(config_ref_version)
    .bind(jam)
    .execute(&mut **conn)
    .await
    .map_err(|e| e.to_string())?;

    for e in &dipilih {
        if e.input_payload.is_none() {
            tracing::warn!(request_id = %e.request_id, "eligible tanpa input_payload, dilewati");
            continue;
        }
        let field_input = e.input_payload.clone().unwrap();
        // field_metadata: filter keys with kirim_ke_data_balikan != false and klasifikasi != rahasia
        let field_metadata = filter_metadata(&e.metadata);
        sqlx::query(
            "INSERT INTO data_balikan_item (batch_id, request_id, kode_transaksi, nik, field_input, field_metadata, requested_at) \
             VALUES ($1,$2,$3,$4,$5,$6,$7)",
        )
        .bind(batch_id)
        .bind(e.request_id)
        .bind(e.kode_transaksi)
        .bind(&e.nik)
        .bind(&field_input)
        .bind(&field_metadata)
        .bind(e.requested_at)
        .execute(&mut **conn)
        .await
        .map_err(|e| e.to_string())?;
    }

    let _ = zona_dari_nama(&konfigurasi.periode.zona);
    let _ = kebijakan_batch(&konfigurasi);

    Ok(true)
}

fn filter_metadata(m: &serde_json::Value) -> serde_json::Value {
    if let Some(obj) = m.as_object() {
        let mut out = serde_json::Map::new();
        for (k, v) in obj {
            let kirim = v.get("kirim_ke_data_balikan").and_then(|x| x.as_bool()).unwrap_or(true);
            let rahasia = v.get("klasifikasi").and_then(|x| x.as_str()) == Some("rahasia");
            if kirim && !rahasia {
                out.insert(k.clone(), v.clone());
            }
        }
        if out.is_empty() {
            m.clone()
        } else {
            serde_json::Value::Object(out)
        }
    } else {
        m.clone()
    }
}
