//! Alur `urai` lalu `jalankan` terhadap PostgreSQL sungguhan (ARC-238): argumen mentah seperti yang
//! diketik di terminal diurai menjadi `Perintah` oleh `urai`, bukan dirakit tangan seperti di
//! `cli_pg.rs`, lalu dijalankan sebagai `pr_admin_api`. `#[ignore]`: dijalankan `make -C backend uji-pg`.
//!
//! Basis data dipakai bersama binary lain, jadi setiap uji membuat pengguna sintetis bernama unik.

use std::io;

use chrono::{DateTime, Utc};
use poolrouter_cli::{Konteks, PembacaPassword, jalankan, urai};
use poolrouter_core::config::Environment;
use sqlx::PgPool;
use uuid::Uuid;
use zeroize::Zeroizing;

const NILAI_UJI: &str = "nilai-uji-sintetis-qx-01";
const ALASAN: &str = "SPTJM-UJI-001 penugasan sintetis ARC-238";

struct PasswordSintetis;

impl PembacaPassword for PasswordSintetis {
    fn baca_password_baru(&mut self) -> io::Result<Zeroizing<String>> {
        Ok(Zeroizing::new(NILAI_UJI.to_owned()))
    }
}

async fn pool(var: &str) -> PgPool {
    let url = std::env::var(var)
        .unwrap_or_else(|_| panic!("{var} belum diset; jalankan lewat `make -C backend uji-pg`"));
    PgPool::connect(&url)
        .await
        .expect("database tidak dapat dihubungi")
}

fn args(kata: &[&str]) -> Vec<String> {
    kata.iter().map(|k| (*k).to_owned()).collect()
}

/// Fixture: platform admin aktif sintetis, ditulis langsung sebagai `pr_admin_api`.
async fn platform_admin(admin: &PgPool, awalan: &str) -> String {
    let id = Uuid::now_v7();
    let username = format!("{awalan}-{}", id.simple());
    sqlx::query(
        "INSERT INTO admin_user (pengguna_id, username, password_hash) VALUES ($1, $2, 'x')",
    )
    .bind(id)
    .bind(&username)
    .execute(admin)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO admin_user_peran (pengguna_id, peran, diajukan_oleh) \
         VALUES ($1, 'platform_admin', $1)",
    )
    .bind(id)
    .execute(admin)
    .await
    .unwrap();
    username
}

#[tokio::test]
#[ignore]
async fn urai_lalu_jalankan_pengguna_buat_dan_peran_beri() {
    let admin = pool("POOLROUTER_TEST_DATABASE_URL").await;
    let su = pool("POOLROUTER_TEST_DATABASE_URL_SU").await;
    let pelaku = platform_admin(&admin, "pa-urai").await;
    let penyetuju = platform_admin(&admin, "pa-urai-setuju").await;
    let baru = format!("op-urai-{}", Uuid::now_v7().simple());
    let k = Konteks {
        pool: admin,
        env: Environment::Ci,
        seed_password: None,
        sekarang: Utc::now(),
    };

    let perintah = urai(&args(&[
        "pengguna",
        "buat",
        "--username",
        &baru,
        "--oleh",
        &pelaku,
        "--alasan",
        ALASAN,
    ]))
    .unwrap();
    let mut out = Vec::new();
    jalankan(perintah, &k, &mut PasswordSintetis, &mut out)
        .await
        .unwrap();
    let teks = String::from_utf8(out).unwrap();
    assert!(teks.contains(&baru) && !teks.contains(NILAI_UJI));
    let (id, aktif, hash): (Uuid, bool, String) = sqlx::query_as(
        "SELECT pengguna_id, aktif, password_hash FROM admin_user WHERE username = $1",
    )
    .bind(&baru)
    .fetch_one(&su)
    .await
    .unwrap();
    assert!(aktif);
    assert!(hash.starts_with("$argon2id$"));
    let (pembuat, alasan): (Uuid, String) = sqlx::query_as(
        "SELECT pengguna_id, alasan FROM audit_konfigurasi \
         WHERE entitas_jenis = 'admin_user' AND entitas_id = $1",
    )
    .bind(id)
    .fetch_one(&su)
    .await
    .unwrap();
    let id_pelaku: Uuid =
        sqlx::query_scalar("SELECT pengguna_id FROM admin_user WHERE username = $1")
            .bind(&pelaku)
            .fetch_one(&su)
            .await
            .unwrap();
    assert_eq!(pembuat, id_pelaku, "--oleh menjadi pelaku di audit");
    assert_eq!(alasan, ALASAN, "--alasan tercatat apa adanya");

    // `--berlaku-sampai` tanggal murni dibaca akhir hari itu WIB (2099-01-31 24.00 +07.00).
    let perintah = urai(&args(&[
        "peran",
        "beri",
        "--username",
        &baru,
        "--peran",
        "auditor",
        "--oleh",
        &pelaku,
        "--alasan",
        ALASAN,
        "--disetujui-oleh",
        &penyetuju,
        "--berlaku-sampai",
        "2099-01-31",
    ]))
    .unwrap();
    jalankan(perintah, &k, &mut PasswordSintetis, &mut Vec::new())
        .await
        .unwrap();
    let (peran, berlaku_sampai): (String, Option<DateTime<Utc>>) = sqlx::query_as(
        "SELECT peran, berlaku_sampai FROM admin_user_peran \
         WHERE pengguna_id = $1 AND dicabut_at IS NULL",
    )
    .bind(id)
    .fetch_one(&su)
    .await
    .unwrap();
    assert_eq!(peran, "auditor");
    assert_eq!(
        berlaku_sampai,
        Some(
            DateTime::parse_from_rfc3339("2099-01-31T17:00:00Z")
                .unwrap()
                .with_timezone(&Utc)
        )
    );
}
