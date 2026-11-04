//! Session storage over `admin_session` (ADR-0003 rev 5 §3.6, §3.6f).
//!
//! The time rules live in [`super::sesi`] as pure functions; this module only
//! reads the row, asks those functions what to do, and writes the answer with a
//! guarded `UPDATE`. Three properties are enforced here rather than in the
//! handler so they cannot drift:
//!
//! - **`getSesi` never writes.** [`SesiStore::status`] is a `SELECT`; polling it
//!   cannot push the idle deadline (AC "getSesi berulang tidak menunda").
//! - **`perpanjangSesi` moves `terakhir_aktif_at` only.** [`SesiStore::perpanjang`]
//!   has no write path for `dibuat_at` or `kedaluwarsa_at`, and it does not
//!   touch the token (no rotation, no `Set-Cookie`).
//! - **Expired sessions are never resurrected.** The `UPDATE` repeats the idle
//!   and absolute predicates, so a request racing the deadline loses in SQL even
//!   if the in-memory evaluation said "alive" a moment earlier.
//!
//! The clock is a parameter on every call, which is what lets the integration
//! tests advance time instead of sleeping.

use std::fmt;

use chrono::{DateTime, Duration, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::sesi::{
    BatasSesi, EfekPermintaan, HasilPerpanjang, KeadaanSesi, SesiKedaluwarsa, StatusSesi, evaluasi,
    perpanjang,
};
use super::token::HashToken;

/// A session as read back: the owner, plus the two instants the rules need.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SesiTersimpan {
    pub sesi_id: Uuid,
    pub pengguna_id: Uuid,
    pub keadaan: KeadaanSesi,
}

/// Why a token does not identify a usable session. All variants answer 401; they
/// stay distinct so the caller can log the reason without leaking it to the
/// client.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SesiDitolak {
    /// No row for this token (never issued, revoked, or the user is inactive).
    TidakDikenal,
    Kedaluwarsa(SesiKedaluwarsa),
}

#[derive(Debug)]
pub enum GalatSesi {
    BasisData(sqlx::Error),
}

impl fmt::Display for GalatSesi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BasisData(e) => write!(f, "penyimpanan sesi gagal: {e}"),
        }
    }
}

impl std::error::Error for GalatSesi {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::BasisData(e) => Some(e),
        }
    }
}

impl From<sqlx::Error> for GalatSesi {
    fn from(e: sqlx::Error) -> Self {
        Self::BasisData(e)
    }
}

/// Maximum stored `user_agent` length (`CHECK (length(user_agent) <= 512)`).
const PANJANG_USER_AGENT: usize = 512;

#[derive(Clone)]
pub struct SesiStore {
    pool: PgPool,
    batas: BatasSesi,
}

impl SesiStore {
    pub fn baru(pool: PgPool) -> Self {
        Self::dengan_batas(pool, BatasSesi::f0())
    }

    pub fn dengan_batas(pool: PgPool, batas: BatasSesi) -> Self {
        Self { pool, batas }
    }

    pub const fn batas(&self) -> BatasSesi {
        self.batas
    }

    /// Insert a session at login. `kedaluwarsa_at` is the absolute limit, fixed
    /// here and never updated afterwards.
    pub async fn terbitkan(
        &self,
        pengguna_id: Uuid,
        hash: &HashToken,
        user_agent: Option<&str>,
        sekarang: DateTime<Utc>,
    ) -> Result<SesiTersimpan, GalatSesi> {
        let sesi_id = Uuid::now_v7();
        let kedaluwarsa = sekarang + Duration::seconds(self.batas.absolut_detik);
        let ua = user_agent.map(potong_user_agent);
        sqlx::query(
            "INSERT INTO admin_session \
             (sesi_id, token_sha256, pengguna_id, dibuat_at, terakhir_aktif_at, kedaluwarsa_at, user_agent) \
             VALUES ($1, decode($2, 'hex'), $3, $4, $4, $5, $6)",
        )
        .bind(sesi_id)
        .bind(hash.as_str())
        .bind(pengguna_id)
        .bind(sekarang)
        .bind(kedaluwarsa)
        .bind(ua)
        .execute(&self.pool)
        .await?;
        Ok(SesiTersimpan {
            sesi_id,
            pengguna_id,
            keadaan: KeadaanSesi::baru(sekarang, sekarang),
        })
    }

    /// Read a session without changing it. Rows of deactivated users are
    /// invisible, so deactivating a user ends their sessions even before the
    /// explicit revocation runs.
    pub async fn baca(&self, hash: &HashToken) -> Result<Option<SesiTersimpan>, GalatSesi> {
        let baris: Option<(Uuid, Uuid, DateTime<Utc>, DateTime<Utc>)> = sqlx::query_as(
            "SELECT s.sesi_id, s.pengguna_id, s.dibuat_at, s.terakhir_aktif_at \
             FROM admin_session s JOIN admin_user u USING (pengguna_id) \
             WHERE s.token_sha256 = decode($1, 'hex') AND u.aktif",
        )
        .bind(hash.as_str())
        .fetch_optional(&self.pool)
        .await?;
        Ok(
            baris.map(|(sesi_id, pengguna_id, dibuat, aktif)| SesiTersimpan {
                sesi_id,
                pengguna_id,
                keadaan: KeadaanSesi::baru(dibuat, aktif),
            }),
        )
    }

    /// `getSesi`: the status body, with no write.
    pub async fn status(
        &self,
        hash: &HashToken,
        sekarang: DateTime<Utc>,
    ) -> Result<Result<StatusSesi, SesiDitolak>, GalatSesi> {
        let Some(sesi) = self.baca(hash).await? else {
            return Ok(Err(SesiDitolak::TidakDikenal));
        };
        // Same limits as a normal request, so a dead session answers 401 here
        // too instead of reporting a deadline in the past.
        if let EfekPermintaan::Ditolak(alasan) = evaluasi(sesi.keadaan, sekarang, self.batas) {
            return Ok(Err(SesiDitolak::Kedaluwarsa(alasan)));
        }
        Ok(Ok(StatusSesi::hitung(sesi.keadaan, sekarang, self.batas)))
    }

    /// Authenticate an ordinary admin request: evaluate both limits and push
    /// `terakhir_aktif_at` to now when that is a change.
    pub async fn autentikasi(
        &self,
        hash: &HashToken,
        sekarang: DateTime<Utc>,
    ) -> Result<Result<SesiTersimpan, SesiDitolak>, GalatSesi> {
        let Some(mut sesi) = self.baca(hash).await? else {
            return Ok(Err(SesiDitolak::TidakDikenal));
        };
        match evaluasi(sesi.keadaan, sekarang, self.batas) {
            EfekPermintaan::Ditolak(alasan) => Ok(Err(SesiDitolak::Kedaluwarsa(alasan))),
            EfekPermintaan::TidakBerubah => Ok(Ok(sesi)),
            EfekPermintaan::DiperpanjangIdle { terakhir_aktif_at } => {
                if self.dorong_idle(hash, terakhir_aktif_at, sekarang).await? {
                    sesi.keadaan.terakhir_aktif_at = terakhir_aktif_at;
                    Ok(Ok(sesi))
                } else {
                    // Lost the race against the deadline.
                    Ok(Err(SesiDitolak::Kedaluwarsa(SesiKedaluwarsa::Idle)))
                }
            }
        }
    }

    /// `perpanjangSesi`: idle only. Returns the same body shape as `status`.
    pub async fn perpanjang(
        &self,
        hash: &HashToken,
        sekarang: DateTime<Utc>,
    ) -> Result<Result<StatusSesi, SesiDitolak>, GalatSesi> {
        let Some(sesi) = self.baca(hash).await? else {
            return Ok(Err(SesiDitolak::TidakDikenal));
        };
        match perpanjang(sesi.keadaan, sekarang, self.batas) {
            HasilPerpanjang::Ditolak(alasan) => Ok(Err(SesiDitolak::Kedaluwarsa(alasan))),
            HasilPerpanjang::Diperpanjang {
                terakhir_aktif_at, ..
            } => {
                if terakhir_aktif_at > sesi.keadaan.terakhir_aktif_at
                    && !self.dorong_idle(hash, terakhir_aktif_at, sekarang).await?
                {
                    return Ok(Err(SesiDitolak::Kedaluwarsa(SesiKedaluwarsa::Idle)));
                }
                let baru = KeadaanSesi::baru(
                    sesi.keadaan.dibuat_at,
                    terakhir_aktif_at.max(sesi.keadaan.terakhir_aktif_at),
                );
                Ok(Ok(StatusSesi::hitung(baru, sekarang, self.batas)))
            }
        }
    }

    /// Logout: delete the row. Returns whether a session existed.
    pub async fn cabut(&self, hash: &HashToken) -> Result<bool, GalatSesi> {
        let r = sqlx::query("DELETE FROM admin_session WHERE token_sha256 = decode($1, 'hex')")
            .bind(hash.as_str())
            .execute(&self.pool)
            .await?;
        Ok(r.rows_affected() > 0)
    }

    /// Write `terakhir_aktif_at`, repeating both limits in the predicate.
    /// `GREATEST` keeps the column monotonic when two requests race.
    async fn dorong_idle(
        &self,
        hash: &HashToken,
        baru: DateTime<Utc>,
        sekarang: DateTime<Utc>,
    ) -> Result<bool, GalatSesi> {
        let r = sqlx::query(
            "UPDATE admin_session \
             SET terakhir_aktif_at = GREATEST(terakhir_aktif_at, $2) \
             WHERE token_sha256 = decode($1, 'hex') \
               AND kedaluwarsa_at > $3 \
               AND terakhir_aktif_at + make_interval(secs => $4) > $3",
        )
        .bind(hash.as_str())
        .bind(baru)
        .bind(sekarang)
        .bind(self.batas.idle_detik as f64)
        .execute(&self.pool)
        .await?;
        Ok(r.rows_affected() > 0)
    }
}

/// Cut on a char boundary at or below the column limit.
fn potong_user_agent(ua: &str) -> &str {
    if ua.len() <= PANJANG_USER_AGENT {
        return ua;
    }
    let mut akhir = PANJANG_USER_AGENT;
    while !ua.is_char_boundary(akhir) {
        akhir -= 1;
    }
    &ua[..akhir]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_agent_pendek_tidak_diubah() {
        assert_eq!(potong_user_agent("Mozilla/5.0"), "Mozilla/5.0");
    }

    #[test]
    fn user_agent_dipotong_di_batas_karakter() {
        let panjang = "é".repeat(400); // 800 bytes
        let hasil = potong_user_agent(&panjang);
        assert!(hasil.len() <= PANJANG_USER_AGENT);
        assert!(hasil.chars().all(|c| c == 'é'));
    }
}
