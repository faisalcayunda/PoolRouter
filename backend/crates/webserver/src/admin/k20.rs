//! Transactional K-20 entity operations.
//!
//! The database functions own locking, mass-cancel semantics, and audit rows;
//! this seam deliberately only maps the closed entity-kind vocabulary and
//! keeps all calls on the admin pool.

use sqlx::PgPool;
use uuid::Uuid;

pub const ALASAN_MIN: usize = 10;
pub const ALASAN_MAX: usize = 500;

/// Memastikan alasan memenuhi kontrak K-20 sebelum diteruskan ke SQL.
pub fn validasi_alasan(alasan: &str) -> bool {
    (ALASAN_MIN..=ALASAN_MAX).contains(&alasan.chars().count())
}

/// Entity kinds accepted by the K-20 database functions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JenisEntitas {
    Client,
    Kredensial,
    DownstreamEndpoint,
    DownstreamPool,
    UpstreamPool,
    Provider,
}

impl JenisEntitas {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Client => "client",
            Self::Kredensial => "kredensial",
            Self::DownstreamEndpoint => "downstream_endpoint",
            Self::DownstreamPool => "downstream_pool",
            Self::UpstreamPool => "upstream_pool",
            Self::Provider => "provider",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HasilK20 {
    pub jumlah_cancelled: i32,
    pub jumlah_failed: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PratinjauK20 {
    pub akan_cancelled: i32,
    pub akan_failed: i32,
    pub dihitung_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GalatJenisEntitas {
    TidakDidukung,
}

impl TryFrom<&str> for JenisEntitas {
    type Error = GalatJenisEntitas;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        match value {
            "client" => Ok(Self::Client),
            "kredensial" => Ok(Self::Kredensial),
            "downstream_endpoint" => Ok(Self::DownstreamEndpoint),
            "downstream_pool" => Ok(Self::DownstreamPool),
            "upstream_pool" => Ok(Self::UpstreamPool),
            "provider" => Ok(Self::Provider),
            _ => Err(GalatJenisEntitas::TidakDidukung),
        }
    }
}

#[derive(Clone)]
pub struct K20Service {
    pool: PgPool,
}

impl K20Service {
    pub fn baru(pool: PgPool) -> Self { Self { pool } }

    pub async fn pratinjau(&self, jenis: JenisEntitas, id: Uuid) -> Result<PratinjauK20, sqlx::Error> {
        sqlx::query_as::<_, PratinjauK20Row>(
            "SELECT akan_cancelled, akan_failed, dihitung_at FROM fn_pratinjau_nonaktif($1, $2)",
        )
        .bind(jenis.as_str()).bind(id).fetch_one(&self.pool).await.map(Into::into)
    }

    pub async fn nonaktifkan(&self, jenis: JenisEntitas, id: Uuid, pengguna_id: Uuid, alasan: &str) -> Result<HasilK20, sqlx::Error> {
        debug_assert!(validasi_alasan(alasan));
        sqlx::query_as::<_, HasilK20Row>(
            "SELECT jumlah_cancelled, jumlah_failed FROM fn_nonaktifkan_entitas($1, $2, $3, $4)",
        )
        .bind(jenis.as_str()).bind(id).bind(pengguna_id).bind(alasan)
        .fetch_one(&self.pool).await.map(Into::into)
    }

    pub async fn aktifkan(&self, jenis: JenisEntitas, id: Uuid, pengguna_id: Uuid, alasan: &str) -> Result<(), sqlx::Error> {
        debug_assert!(validasi_alasan(alasan));
        sqlx::query("SELECT fn_aktifkan_entitas($1, $2, $3, $4)")
            .bind(jenis.as_str()).bind(id).bind(pengguna_id).bind(alasan)
            .execute(&self.pool).await.map(|_| ())
    }
}

#[derive(sqlx::FromRow)]
struct HasilK20Row { jumlah_cancelled: i32, jumlah_failed: i32 }
impl From<HasilK20Row> for HasilK20 { fn from(r: HasilK20Row) -> Self { Self { jumlah_cancelled: r.jumlah_cancelled, jumlah_failed: r.jumlah_failed } } }

#[derive(sqlx::FromRow)]
struct PratinjauK20Row { akan_cancelled: i32, akan_failed: i32, dihitung_at: chrono::DateTime<chrono::Utc> }
impl From<PratinjauK20Row> for PratinjauK20 { fn from(r: PratinjauK20Row) -> Self { Self { akan_cancelled: r.akan_cancelled, akan_failed: r.akan_failed, dihitung_at: r.dihitung_at } } }

#[cfg(test)]
mod tests {
    use super::{JenisEntitas, validasi_alasan};
    #[test]
    fn entity_kind_mapping_is_closed_and_matches_sql_contract() {
        assert_eq!(JenisEntitas::Client.as_str(), "client");
        assert_eq!(JenisEntitas::Kredensial.as_str(), "kredensial");
        assert_eq!(JenisEntitas::DownstreamEndpoint.as_str(), "downstream_endpoint");
        assert_eq!(JenisEntitas::DownstreamPool.as_str(), "downstream_pool");
        assert_eq!(JenisEntitas::UpstreamPool.as_str(), "upstream_pool");
        assert_eq!(JenisEntitas::Provider.as_str(), "provider");
        assert!(JenisEntitas::try_from("nik").is_err());
    }

    #[test]
    fn alasan_uses_character_limits() {
        assert!(!validasi_alasan("pendek"));
        assert!(validasi_alasan(&"x".repeat(10)));
        assert!(validasi_alasan(&"🙂".repeat(500)));
        assert!(!validasi_alasan(&"x".repeat(501)));
    }
}
