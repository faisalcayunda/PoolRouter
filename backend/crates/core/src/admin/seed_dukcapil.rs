//! `seed dukcapil` (spec f0-admin-api AC-43, ARC-237): satu rantai konfigurasi sintetis lengkap, yaitu
//! provider, upstream pool, downstream pool beserta rutenya, dan downstream endpoint, supaya
//! lingkungan dev/ci punya jalur sync dan async yang bisa dicoba tanpa mengisi konfigurasi lewat UI.
//!
//! Urutan statement sama dengan handler admin API (provider dan endpoint disisipkan dengan penunjuk
//! versi NULL, versi 1 ditulis, lalu penunjuk dinaikkan), dan setiap baris baru tercatat di
//! `audit_konfigurasi` atas nama pengguna `seed-platform-admin`. Itu sebabnya `seed pengguna` harus
//! dijalankan lebih dulu.
//!
//! Batasan yang disengaja:
//! - Template tidak memakai `{{secret.*}}`: CLI tidak punya KEK dan tidak membuat `provider_secret`.
//! - `config_schema_version` 1 dan `data_balikan_aktif` false. Skema data balikan Dukcapil masih
//!   keputusan terbuka D-08. `respons_skema` memakai bentuk `status_skor` yang dibaca parser runtime
//!   (`adapter-provider::dokumen::baca_respons_skema`), supaya jalur sync dan async tidak menolaknya.
//! - `base_url` memakai host `.example.test` karena aturan handler mewajibkan `https://`. Operator
//!   mengarahkannya ke mock lewat admin API bila perlu.

use serde_json::{Value, json};
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::admin::kelola_pengguna::{GalatKelola, SEED_PLATFORM_ADMIN};
use crate::audit::{AksiKonfigurasi, Alasan, EntitasKonfigurasi, KonfigurasiBaru, Penulis};
use crate::config::Environment;

pub const SEED_PROVIDER: &str = "seed-dukcapil-provider";
pub const SEED_UPSTREAM: &str = "seed-dukcapil-upstream";
pub const SEED_DOWNSTREAM: &str = "seed-dukcapil-downstream";
pub const SEED_ENDPOINT: &str = "seed-dukcapil-endpoint";

const ALASAN_SEED: &str = "seed dukcapil sintetis dev/ci (ARC-237)";
const KUOTA_HARIAN_SEED: i32 = 1000;
/// Kunci advisory untuk menyerialkan dua seed yang berjalan bersamaan ("POOLSDKP").
const KUNCI_SEED_DUKCAPIL: i64 = 0x504f_4f4c_5344_4b50;

/// Template dan aturan parameter provider sintetis. Diperiksa `validasi_template` di uji, supaya seed
/// tidak menghasilkan konfigurasi yang ditolak jalur admin API.
pub const TEMPLATE_SEED: &str = r#"{"NIK":"{{input.NIK}}","NAMA_LGKP":"{{input.NAMA_LGKP}}"}"#;
pub const PARAMETER_SEED: [&str; 2] = ["NIK", "NAMA_LGKP"];

/// Berapa baris yang benar-benar dibuat (semuanya 0 pada jalan kedua).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HasilSeedDukcapil {
    pub provider_baru: u32,
    pub upstream_baru: u32,
    pub downstream_baru: u32,
    pub endpoint_baru: u32,
}

/// Publik supaya uji di crate adapter bisa memastikan dokumen ini terbaca parser runtime.
pub fn dokumen_provider() -> Value {
    let parameter: Vec<Value> = PARAMETER_SEED
        .iter()
        .map(|n| json!({ "nama": n }))
        .collect();
    json!({
        "metode": "POST",
        "base_url": "https://dukcapil-sintetis.example.test/api",
        "body_template": TEMPLATE_SEED,
        "parameters": parameter,
        "respons_skema": {
            "parser": "status_skor",
            "literal": [
                { "teks": "Sesuai", "status": "sesuai" },
                { "teks": "Tidak Sesuai", "status": "tidak_sesuai" }
            ],
            "parameter_output": [{ "nama": "NAMA_LGKP" }]
        },
        "tls_trust": "system",
        "log_mode": "metadata_only"
    })
}

/// `sql` wajib literal `SELECT <id> FROM <tabel> WHERE nama = $1` (sqlx 0.9 menolak SQL dinamis).
async fn id_menurut_nama(
    tx: &mut Transaction<'static, Postgres>,
    sql: &'static str,
    nama: &str,
) -> Result<Option<Uuid>, sqlx::Error> {
    sqlx::query_scalar(sql)
        .bind(nama)
        .fetch_optional(&mut **tx)
        .await
}

async fn catat(
    tx: &mut Transaction<'static, Postgres>,
    pelaku: Uuid,
    entitas: EntitasKonfigurasi,
    id: Uuid,
    nilai: Value,
    alasan: &Alasan,
) -> Result<(), GalatKelola> {
    Penulis::baru(tx)
        .catat_konfigurasi(&KonfigurasiBaru {
            pelaku,
            entitas,
            entitas_id: Some(id),
            aksi: AksiKonfigurasi::Buat,
            nilai_lama: None,
            nilai_baru: Some(nilai),
            alasan: alasan.clone(),
        })
        .await?;
    Ok(())
}

/// `seed dukcapil`: ditolak tanpa perubahan di luar `dev`/`ci`, dan idempoten (objek yang namanya
/// sudah ada tidak dibuat ulang dan tidak menulis audit baru).
pub async fn seed_dukcapil(
    pool: &PgPool,
    env: Environment,
) -> Result<HasilSeedDukcapil, GalatKelola> {
    if !env.allows_seed() {
        return Err(GalatKelola::SeedDitolak);
    }
    let alasan = Alasan::baru(ALASAN_SEED)?;
    let mut hasil = HasilSeedDukcapil::default();
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(KUNCI_SEED_DUKCAPIL)
        .execute(&mut *tx)
        .await?;

    let pelaku =
        sqlx::query_scalar::<_, Uuid>("SELECT pengguna_id FROM admin_user WHERE username = $1")
            .bind(SEED_PLATFORM_ADMIN)
            .fetch_optional(&mut *tx)
            .await?
            .ok_or(GalatKelola::SeedPenggunaBelumAda)?;

    let provider = match id_menurut_nama(
        &mut tx,
        "SELECT provider_id FROM provider WHERE nama = $1",
        SEED_PROVIDER,
    )
    .await?
    {
        Some(id) => id,
        None => {
            let id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO provider (provider_id, nama, aktif, config_versi_aktif) \
                 VALUES ($1, $2, true, NULL)",
            )
            .bind(id)
            .bind(SEED_PROVIDER)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO provider_config_versi \
                 (provider_id, versi, config_schema_version, log_mode, dokumen, dibuat_oleh) \
                 VALUES ($1, 1, 1, 'metadata_only', $2, $3)",
            )
            .bind(id)
            .bind(sqlx::types::Json(dokumen_provider()))
            .bind(pelaku)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "UPDATE provider SET config_versi_aktif = 1, diubah_at = now() \
                 WHERE provider_id = $1",
            )
            .bind(id)
            .execute(&mut *tx)
            .await?;
            catat(
                &mut tx,
                pelaku,
                EntitasKonfigurasi::Provider,
                id,
                json!({ "nama": SEED_PROVIDER, "versi": 1 }),
                &alasan,
            )
            .await?;
            hasil.provider_baru += 1;
            id
        }
    };

    let upstream = match id_menurut_nama(
        &mut tx,
        "SELECT upstream_pool_id FROM upstream_pool WHERE nama = $1",
        SEED_UPSTREAM,
    )
    .await?
    {
        Some(id) => id,
        None => {
            let id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO upstream_pool (upstream_pool_id, provider_id, nama, kuota_harian) \
                 VALUES ($1, $2, $3, $4)",
            )
            .bind(id)
            .bind(provider)
            .bind(SEED_UPSTREAM)
            .bind(KUOTA_HARIAN_SEED)
            .execute(&mut *tx)
            .await?;
            catat(
                &mut tx,
                pelaku,
                EntitasKonfigurasi::UpstreamPool,
                id,
                json!({ "nama": SEED_UPSTREAM, "provider_id": provider, "kuota_harian": KUOTA_HARIAN_SEED }),
                &alasan,
            )
            .await?;
            hasil.upstream_baru += 1;
            id
        }
    };

    let downstream = match id_menurut_nama(
        &mut tx,
        "SELECT downstream_pool_id FROM downstream_pool WHERE nama = $1",
        SEED_DOWNSTREAM,
    )
    .await?
    {
        Some(id) => id,
        None => {
            let id = Uuid::now_v7();
            sqlx::query(
                "INSERT INTO downstream_pool (downstream_pool_id, nama, prioritas) \
                 VALUES ($1, $2, 1)",
            )
            .bind(id)
            .bind(SEED_DOWNSTREAM)
            .execute(&mut *tx)
            .await?;
            sqlx::query(
                "INSERT INTO downstream_pool_rute (downstream_pool_id, upstream_pool_id) \
                 VALUES ($1, $2)",
            )
            .bind(id)
            .bind(upstream)
            .execute(&mut *tx)
            .await?;
            catat(
                &mut tx,
                pelaku,
                EntitasKonfigurasi::DownstreamPool,
                id,
                json!({ "nama": SEED_DOWNSTREAM, "upstream_pool_ids": [upstream] }),
                &alasan,
            )
            .await?;
            hasil.downstream_baru += 1;
            id
        }
    };

    if id_menurut_nama(
        &mut tx,
        "SELECT downstream_endpoint_id FROM downstream_endpoint WHERE nama = $1",
        SEED_ENDPOINT,
    )
    .await?
    .is_none()
    {
        let id = Uuid::now_v7();
        sqlx::query(
            "INSERT INTO downstream_endpoint \
             (downstream_endpoint_id, nama, provider_id, downstream_pool_id, aktif, config_versi_aktif) \
             VALUES ($1, $2, $3, $4, true, NULL)",
        )
        .bind(id)
        .bind(SEED_ENDPOINT)
        .bind(provider)
        .bind(downstream)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "INSERT INTO endpoint_config_versi \
             (downstream_endpoint_id, versi, config_schema_version, log_mode, dokumen, dibuat_oleh) \
             VALUES ($1, 1, 1, 'metadata_only', $2, $3)",
        )
        .bind(id)
        .bind(sqlx::types::Json(json!({ "parameter_literal": {} })))
        .bind(pelaku)
        .execute(&mut *tx)
        .await?;
        sqlx::query(
            "UPDATE downstream_endpoint SET config_versi_aktif = 1, diubah_at = now() \
             WHERE downstream_endpoint_id = $1",
        )
        .bind(id)
        .execute(&mut *tx)
        .await?;
        catat(
            &mut tx,
            pelaku,
            EntitasKonfigurasi::DownstreamEndpoint,
            id,
            json!({ "nama": SEED_ENDPOINT, "provider_id": provider, "downstream_pool_id": downstream }),
            &alasan,
        )
        .await?;
        hasil.endpoint_baru += 1;
    }

    tx.commit().await?;
    Ok(hasil)
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::*;
    use crate::template::{ParameterTemplate, validasi_template};

    #[test]
    fn template_seed_lolos_validasi_admin_api() {
        let parameter: Vec<ParameterTemplate> = PARAMETER_SEED
            .iter()
            .map(|n| ParameterTemplate {
                nama: (*n).to_owned(),
            })
            .collect();
        let hasil = validasi_template(
            "/body_template",
            TEMPLATE_SEED,
            &parameter,
            &BTreeSet::new(),
        );
        assert!(hasil.is_ok(), "{hasil:?}");
    }

    #[test]
    fn dokumen_provider_tanpa_secret_dan_https() {
        let d = dokumen_provider();
        assert!(!d["body_template"].as_str().unwrap().contains("secret."));
        assert!(d["base_url"].as_str().unwrap().starts_with("https://"));
    }
}
