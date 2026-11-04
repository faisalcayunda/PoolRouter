//! Synthetic users for `dev` and `ci` (ADR-0003 rev 5 §3.6c, D-4; SHAA-196
//! scope item 5).
//!
//! One user per role, so every role can be exercised end to end:
//!
//! | username                 | role            | note                                   |
//! |--------------------------|-----------------|----------------------------------------|
//! | `seed-platform-admin`    | platform_admin  | proposer for operator and auditor      |
//! | `seed-platform-admin-2`  | platform_admin  | approver (A-02: three distinct people) |
//! | `seed-operator`          | operator        | `bahasa_ui = 'en'`                     |
//! | `seed-auditor`           | auditor         | role expires after 365 days            |
//! | `seed-client-owner`      | client_owner    | bound to the synthetic client          |
//!
//! Rules:
//! - **Refused outside `dev`/`ci`.** The check is the first statement, before
//!   any hashing or database work, and is independent of the config loader's
//!   own refusal to read a seed password in other environments.
//! - **The password is a parameter, never read here.** The caller takes it from
//!   a `_FILE`, not from an argument or an env value (D-4). It must satisfy the
//!   normal 15-character policy; nothing weaker is accepted for seeds.
//! - **Idempotent.** Every insert is `ON CONFLICT DO NOTHING`, so running twice
//!   adds nothing, and an existing user keeps their current password hash.
//! - **One transaction.** A failure leaves no half-seeded set.
//!
//! Role grants here do not write `audit_akses`: they are fixtures for
//! non-production databases, not operator actions. Real grants and revocations
//! go through the CLI, which audits them in the same transaction (§3.6c).

use std::fmt;

use sqlx::PgPool;
use uuid::Uuid;

use super::password::{HashGagal, KebijakanPassword, ParameterArgon2, hash_password};
use crate::config::Environment;

const NAMA_CLIENT: &str = "seed-client-sintetis";

/// How many rows a run added. All zeros on a second run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LaporanSeed {
    pub client_baru: u64,
    pub pengguna_baru: u64,
    pub peran_baru: u64,
}

#[derive(Debug)]
pub enum GalatSeed {
    /// Not `dev`/`ci`. Nothing was touched.
    LingkunganDitolak(Environment),
    /// The password failed the policy (or could not be hashed).
    Password(HashGagal),
    BasisData(sqlx::Error),
}

impl fmt::Display for GalatSeed {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LingkunganDitolak(e) => {
                write!(f, "seed ditolak: lingkungan {e:?} bukan dev atau ci")
            }
            Self::Password(e) => write!(f, "password seed ditolak: {e}"),
            Self::BasisData(e) => write!(f, "seed gagal: {e}"),
        }
    }
}

impl std::error::Error for GalatSeed {}

impl From<sqlx::Error> for GalatSeed {
    fn from(e: sqlx::Error) -> Self {
        Self::BasisData(e)
    }
}

/// Seed the synthetic users. See the module docs for the rules.
pub async fn jalankan(
    pool: &PgPool,
    lingkungan: Environment,
    password: &str,
) -> Result<LaporanSeed, GalatSeed> {
    if !lingkungan.allows_seed() {
        return Err(GalatSeed::LingkunganDitolak(lingkungan));
    }
    // One hash for all five users: they share the seed password, and hashing is
    // the slow part. Salts are per hash, not per user, which is fine for fixtures.
    let hash = hash_password(password, KebijakanPassword::f0(), ParameterArgon2::OWASP)
        .map_err(GalatSeed::Password)?;

    let mut tx = pool.begin().await?;
    let mut laporan = LaporanSeed::default();

    let client_id = Uuid::now_v7();
    laporan.client_baru = sqlx::query(
        "INSERT INTO client (client_id, nama) VALUES ($1, $2) ON CONFLICT (nama) DO NOTHING",
    )
    .bind(client_id)
    .bind(NAMA_CLIENT)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    let client_id: Uuid = sqlx::query_scalar("SELECT client_id FROM client WHERE nama = $1")
        .bind(NAMA_CLIENT)
        .fetch_one(&mut *tx)
        .await?;

    // (username, bahasa_ui, client)
    let pengguna: [(&str, Option<&str>, bool); 5] = [
        ("seed-platform-admin", None, false),
        ("seed-platform-admin-2", None, false),
        ("seed-operator", Some("en"), false),
        ("seed-auditor", None, false),
        ("seed-client-owner", None, true),
    ];
    for (username, bahasa, terikat) in pengguna {
        laporan.pengguna_baru += sqlx::query(
            "INSERT INTO admin_user (pengguna_id, username, password_hash, bahasa_ui, client_id) \
             VALUES ($1, $2, $3, $4, $5) ON CONFLICT (username) DO NOTHING",
        )
        .bind(Uuid::now_v7())
        .bind(username)
        .bind(hash.as_str())
        .bind(bahasa)
        .bind(terikat.then_some(client_id))
        .execute(&mut *tx)
        .await?
        .rows_affected();
    }

    // Roles. platform_admin and client_owner are self-proposed; operator and
    // auditor are proposed by the first admin and approved by the second, so
    // the A-02 check (proposer, approver and subject all distinct) holds.
    let peran = sqlx::query(
        "INSERT INTO admin_user_peran (pengguna_id, peran, diajukan_oleh, disetujui_oleh, berlaku_sampai) \
         SELECT u.pengguna_id, r.peran, \
                CASE WHEN r.perlu_persetujuan THEN a1.pengguna_id ELSE u.pengguna_id END, \
                CASE WHEN r.perlu_persetujuan THEN a2.pengguna_id END, \
                CASE WHEN r.peran = 'auditor' THEN now() + interval '365 days' END \
         FROM (VALUES \
                ('seed-platform-admin',   'platform_admin', false), \
                ('seed-platform-admin-2', 'platform_admin', false), \
                ('seed-operator',         'operator',       true), \
                ('seed-auditor',          'auditor',        true), \
                ('seed-client-owner',     'client_owner',   false) \
              ) AS r(username, peran, perlu_persetujuan) \
         JOIN admin_user u ON u.username = r.username \
         CROSS JOIN (SELECT pengguna_id FROM admin_user WHERE username = 'seed-platform-admin') a1 \
         CROSS JOIN (SELECT pengguna_id FROM admin_user WHERE username = 'seed-platform-admin-2') a2 \
         ON CONFLICT (pengguna_id, peran) DO NOTHING",
    )
    .execute(&mut *tx)
    .await?;
    laporan.peran_baru = peran.rows_affected();

    tx.commit().await?;
    Ok(laporan)
}
