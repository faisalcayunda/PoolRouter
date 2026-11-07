//! Lookup of an admin user for login (ADR-0003 rev 5 §3.6).
//!
//! One query, kept in `core` so the webserver and any future caller read the
//! same columns. The username is matched exactly against the stored lowercase
//! form; the caller normalises first.

use sqlx::PgPool;
use uuid::Uuid;

/// What login needs about a user: identity, the stored hash, and whether the
/// account may log in at all.
#[derive(Clone)]
pub struct PenggunaLogin {
    pub pengguna_id: Uuid,
    pub password_hash: String,
    pub aktif: bool,
}

// Not derived: `password_hash` must not reach a log line through `{:?}`.
impl std::fmt::Debug for PenggunaLogin {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PenggunaLogin")
            .field("pengguna_id", &self.pengguna_id)
            .field("aktif", &self.aktif)
            .finish_non_exhaustive()
    }
}

pub async fn cari_login(
    pool: &PgPool,
    username: &str,
) -> Result<Option<PenggunaLogin>, sqlx::Error> {
    let baris: Option<(Uuid, String, bool)> = sqlx::query_as(
        "SELECT pengguna_id, password_hash, aktif FROM admin_user WHERE username = $1",
    )
    .bind(username)
    .fetch_optional(pool)
    .await?;
    Ok(
        baris.map(|(pengguna_id, password_hash, aktif)| PenggunaLogin {
            pengguna_id,
            password_hash,
            aktif,
        }),
    )
}

/// Parse a role as the contract spells it.
pub fn peran_dari_teks(t: &str) -> Option<crate::audit::Peran> {
    use crate::audit::Peran;
    match t {
        "auditor" => Some(Peran::Auditor),
        "operator" => Some(Peran::Operator),
        "platform_admin" => Some(Peran::PlatformAdmin),
        "client_owner" => Some(Peran::ClientOwner),
        _ => None,
    }
}
