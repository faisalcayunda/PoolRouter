//! MFA module: database-backed store for TOTP, recovery codes, registration code (AC-5–AC-8).
//!
//! Callers use [`MfaStore`]; secret values are wrapped in [`NilaiAman`] or opaque
//! types so a log line or panic cannot print them (AC-3).

use std::sync::Arc;

use crate::admin::{
    NilaiAman,
    password::{sama_konstan, verifikasi_password},
};
use crate::audit::{AksesBaru, Aksi, GalatAudit, Pelaku, Penulis};
use argon2::{Argon2, PasswordHasher};
use chrono::{DateTime, Utc};
use sha2::Digest as _;
use sqlx::{PgPool, Postgres, Transaction};
use uuid::Uuid;

mod metrik;
pub mod totp;

/// 10 base32 characters = 50 bits per recovery code.
const PANJANG_KODE_PEMULIHAN: usize = 10;
const JUMLAH_KODE_PEMULIHAN: usize = 10;
const UMUR_KODE_DAFTAR_JAM: i64 = 24;
const BATAS_GAGAL_KODE_DAFTAR: i32 = 5;

/// Why an MFA operation was refused. Messages never carry a secret, code, or hash.
#[derive(Debug, thiserror::Error)]
pub enum GalatMfa {
    #[error("pengguna tidak terdaftar")]
    PenggunaTidakAda,
    #[error("keadaan MFA tidak cocok")]
    KeadaanTidakCocok,
    #[error("kode tidak cocok")]
    KodeTidakCocok,
    #[error("kode pemulihan sudah dipakai")]
    KodeSudahDipakai,
    #[error("kode pendaftaran tidak berlaku")]
    KodeDaftarTidakBerlaku,
    #[error("penerbit tidak boleh menjadi target")]
    PenerbitSamaDenganTarget,
    #[error("data MFA rusak")]
    DataRusak,
    #[error("sumber acak gagal")]
    AcakGagal,
    #[error("kripto: {0}")]
    Brankas(#[from] crate::secret::GalatBrankas),
    #[error("basis data: {0}")]
    BasisData(#[from] sqlx::Error),
    #[error("audit: {0}")]
    Audit(#[from] GalatAudit),
}

/// A pending MFA secret (base32), shown once before confirmation.
#[derive(Clone)]
pub struct Pendaftaran {
    pub pengguna_id: Uuid,
    /// Secret in base32. `Debug`/`Display` are masked.
    pub rahasia: NilaiAman<String>,
}

impl Pendaftaran {
    #[must_use]
    pub fn rahasia(&self) -> &str {
        self.rahasia.expose()
    }
}

/// Recovery codes returned at confirmation; display once.
#[derive(Clone)]
pub struct KodePemulihan {
    pub kode: Vec<String>,
}

impl KodePemulihan {
    #[must_use]
    pub fn jumlah(&self) -> usize {
        self.kode.len()
    }

    #[must_use]
    pub fn daftar(&self) -> &[String] {
        &self.kode
    }
}

/// Registration code shown once during onboarding; not reusable after use.
#[derive(Clone)]
pub struct KodeDaftar(NilaiAman<String>);

impl KodeDaftar {
    #[must_use]
    pub fn expose(&self) -> &str {
        self.0.expose()
    }
}

/// Outcome of a TOTP verification.
#[derive(Debug)]
pub enum HasilVerifikasi {
    Terima { langkah: u64 },
    Tolak(GalatMfa),
}

/// Outcome of using a recovery code.
#[derive(Debug)]
pub enum HasilPemulihan {
    Terima { sisa_kode_pemulihan: i64 },
    Tolak(GalatMfa),
}

/// Outcome of checking a registration code.
#[derive(Debug)]
pub enum HasilPeriksa {
    Berlaku,
    TidakBerlaku(GalatMfa),
}

/// PostgreSQL-backed MFA store.
#[derive(Clone)]
pub struct MfaStore {
    pub pool: PgPool,
    pub brankas: Arc<crate::secret::Brankas>,
}

impl MfaStore {
    #[must_use]
    pub fn baru(pool: PgPool, brankas: crate::secret::Brankas) -> Self {
        Self {
            pool,
            brankas: Arc::new(brankas),
        }
    }

    /// Create a pending 160-bit secret. Replaces an existing pending registration;
    /// refuses to downgrade an already-active one (reset goes through the CLI).
    pub async fn daftar(&self, pengguna_id: Uuid) -> Result<Pendaftaran, GalatMfa> {
        let mut raw = [0u8; 20];
        getrandom::fill(&mut raw).map_err(|_| GalatMfa::AcakGagal)?;
        let base32 = totp::encode(&raw);

        let amplop = self
            .brankas
            .enkripsi(&pengguna_id.to_string(), &NilaiAman::baru(base32.clone()))?;

        let r = sqlx::query(
            "INSERT INTO pengguna_mfa \
             (pengguna_id, secret_ciphertext, secret_dek_terbungkus, secret_nonce, kek_id, status) \
             VALUES ($1,$2,$3,$4,$5,'menunggu_konfirmasi') \
             ON CONFLICT (pengguna_id) DO UPDATE SET \
               secret_ciphertext = EXCLUDED.secret_ciphertext, \
               secret_dek_terbungkus = EXCLUDED.secret_dek_terbungkus, \
               secret_nonce = EXCLUDED.secret_nonce, kek_id = EXCLUDED.kek_id, \
               status = 'menunggu_konfirmasi', dikonfirmasi_at = NULL, didaftarkan_at = now() \
             WHERE pengguna_mfa.status = 'menunggu_konfirmasi'",
        )
        .bind(pengguna_id)
        .bind(&amplop.ciphertext)
        .bind(&amplop.dek_terbungkus)
        .bind(amplop.nonce)
        .bind(&amplop.kek_id)
        .execute(&self.pool)
        .await?;

        if r.rows_affected() == 0 {
            return Err(GalatMfa::KeadaanTidakCocok);
        }
        Ok(Pendaftaran {
            pengguna_id,
            rahasia: NilaiAman::baru(base32),
        })
    }

    /// Activate the pending secret with a TOTP code and mint 10 recovery codes.
    /// Consumes the active registration code in the same transaction (AC-13).
    pub async fn konfirmasi(
        &self,
        pengguna_id: Uuid,
        kode_totp: &str,
        sekarang: DateTime<Utc>,
    ) -> Result<KodePemulihan, GalatMfa> {
        let mut tx = self.pool.begin().await?;

        let b = sqlx::query_as::<_, BarisMfa>(
            "SELECT status, langkah_terakhir, secret_ciphertext, secret_dek_terbungkus, \
                    secret_nonce, kek_id \
             FROM pengguna_mfa WHERE pengguna_id=$1",
        )
        .bind(pengguna_id)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or(GalatMfa::PenggunaTidakAda)?;

        if b.status != "menunggu_konfirmasi" {
            return Err(GalatMfa::KeadaanTidakCocok);
        }
        let secret = self.buka_rahasia(pengguna_id, &b)?;
        let langkah = totp::cocok(&secret, kode_totp, sekarang).ok_or(GalatMfa::KodeTidakCocok)?;

        let r = sqlx::query(
            "UPDATE pengguna_mfa SET status='aktif', dikonfirmasi_at=$3, langkah_terakhir=$2 \
             WHERE pengguna_id=$1 AND status='menunggu_konfirmasi' AND langkah_terakhir < $2",
        )
        .bind(pengguna_id)
        .bind(langkah as i64)
        .bind(sekarang)
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() != 1 {
            return Err(GalatMfa::KodeTidakCocok);
        }

        let mut kode = Vec::with_capacity(JUMLAH_KODE_PEMULIHAN);
        for _ in 0..JUMLAH_KODE_PEMULIHAN {
            let mut raw = [0u8; 7];
            getrandom::fill(&mut raw).map_err(|_| GalatMfa::AcakGagal)?;
            let teks = totp::encode(&raw)[..PANJANG_KODE_PEMULIHAN].to_string();
            let hash = Argon2::default()
                .hash_password(teks.as_bytes())
                .map_err(|_| GalatMfa::DataRusak)?
                .to_string();
            sqlx::query(
                "INSERT INTO kode_pemulihan_mfa (kode_id, pengguna_id, kode_hash) VALUES ($1,$2,$3)",
            )
            .bind(Uuid::now_v7())
            .bind(pengguna_id)
            .bind(&hash)
            .execute(&mut *tx)
            .await?;
            kode.push(teks);
        }

        sqlx::query(
            "UPDATE kode_daftar_mfa SET dipakai_at=$2 \
             WHERE pengguna_id=$1 AND dipakai_at IS NULL AND gugur_at IS NULL",
        )
        .bind(pengguna_id)
        .bind(sekarang)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok(KodePemulihan { kode })
    }

    /// Verify a TOTP code and advance the anti-replay step atomically (AC-5).
    pub async fn verifikasi(
        &self,
        pengguna_id: Uuid,
        kode: &str,
        sekarang: DateTime<Utc>,
    ) -> HasilVerifikasi {
        match self.verifikasi_dalam(pengguna_id, kode, sekarang).await {
            Ok(langkah) => HasilVerifikasi::Terima { langkah },
            Err(e) => HasilVerifikasi::Tolak(e),
        }
    }

    async fn verifikasi_dalam(
        &self,
        pengguna_id: Uuid,
        kode: &str,
        sekarang: DateTime<Utc>,
    ) -> Result<u64, GalatMfa> {
        let b = sqlx::query_as::<_, BarisMfa>(
            "SELECT status, langkah_terakhir, secret_ciphertext, secret_dek_terbungkus, \
                    secret_nonce, kek_id \
             FROM pengguna_mfa WHERE pengguna_id=$1",
        )
        .bind(pengguna_id)
        .fetch_optional(&self.pool)
        .await?
        .ok_or(GalatMfa::PenggunaTidakAda)?;

        if b.status != "aktif" {
            return Err(GalatMfa::KeadaanTidakCocok);
        }
        let secret = self.buka_rahasia(pengguna_id, &b)?;
        let langkah = totp::cocok(&secret, kode, sekarang).ok_or(GalatMfa::KodeTidakCocok)?;

        // Only one of N parallel callers can move the counter past this step.
        let r = sqlx::query(
            "UPDATE pengguna_mfa SET langkah_terakhir=$2 \
             WHERE pengguna_id=$1 AND status='aktif' AND langkah_terakhir < $2",
        )
        .bind(pengguna_id)
        .bind(langkah as i64)
        .execute(&self.pool)
        .await?;
        if r.rows_affected() != 1 {
            return Err(GalatMfa::KodeTidakCocok);
        }
        Ok(langkah)
    }

    /// Consume one recovery code, write the AC-7 audit row, bump the counter.
    pub async fn pakai_kode_pemulihan(
        &self,
        pengguna_id: Uuid,
        kode: &str,
        sumber_ip: &str,
        sesi_id: &str,
        sekarang: DateTime<Utc>,
    ) -> HasilPemulihan {
        match self
            .pakai_dalam(pengguna_id, kode, sumber_ip, sesi_id, sekarang)
            .await
        {
            Ok(sisa) => HasilPemulihan::Terima {
                sisa_kode_pemulihan: sisa,
            },
            Err(e) => HasilPemulihan::Tolak(e),
        }
    }

    async fn pakai_dalam(
        &self,
        pengguna_id: Uuid,
        kode: &str,
        sumber_ip: &str,
        sesi_id: &str,
        sekarang: DateTime<Utc>,
    ) -> Result<i64, GalatMfa> {
        let mut tx = self.pool.begin().await?;

        let status: Option<String> =
            sqlx::query_scalar("SELECT status FROM pengguna_mfa WHERE pengguna_id=$1")
                .bind(pengguna_id)
                .fetch_optional(&mut *tx)
                .await?;
        if status.as_deref() != Some("aktif") {
            return Err(GalatMfa::KeadaanTidakCocok);
        }

        let baris: Vec<(Uuid, String)> = sqlx::query_as(
            "SELECT kode_id, kode_hash FROM kode_pemulihan_mfa \
             WHERE pengguna_id=$1 AND dipakai_at IS NULL",
        )
        .bind(pengguna_id)
        .fetch_all(&mut *tx)
        .await?;
        let kode_id = baris
            .iter()
            .find(|(_, hash)| verifikasi_password(kode, hash))
            .map(|(id, _)| *id)
            .ok_or(GalatMfa::KodeTidakCocok)?;

        // The trigger allows NULL → not-NULL only once; a second winner gets 0 rows.
        let r = sqlx::query(
            "UPDATE kode_pemulihan_mfa SET dipakai_at=$2 \
             WHERE kode_id=$1 AND dipakai_at IS NULL",
        )
        .bind(kode_id)
        .bind(sekarang)
        .execute(&mut *tx)
        .await?;
        if r.rows_affected() != 1 {
            return Err(GalatMfa::KodeSudahDipakai);
        }

        let peran = self.peran_pengguna(&mut tx, pengguna_id).await?;
        let mut penulis = Penulis::baru(&mut tx);
        let mut baris_audit =
            AksesBaru::berhasil(Pelaku::baru(pengguna_id, peran), Aksi::PakaiKodePemulihan);
        baris_audit.sumber_ip = Some(sumber_ip.to_string());
        baris_audit.sesi_id = Some(sesi_id.to_string());
        baris_audit.filter_kueri = Some(serde_json::json!({ "kode_id": kode_id.to_string() }));
        penulis.catat_akses(baris_audit).await?;

        let sisa: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM kode_pemulihan_mfa \
             WHERE pengguna_id=$1 AND dipakai_at IS NULL",
        )
        .bind(pengguna_id)
        .fetch_one(&mut *tx)
        .await?;

        tx.commit().await?;
        metrik::kode_pemulihan();
        Ok(sisa)
    }

    async fn peran_pengguna(
        &self,
        tx: &mut Transaction<'_, Postgres>,
        pengguna_id: Uuid,
    ) -> Result<crate::audit::Peran, GalatMfa> {
        let peran = sqlx::query_scalar::<_, String>(
            "SELECT peran FROM admin_user_peran \
             WHERE pengguna_id=$1 AND dicabut_at IS NULL \
             ORDER BY CASE peran WHEN 'platform_admin' THEN 0 WHEN 'auditor' THEN 1 ELSE 2 END \
             LIMIT 1",
        )
        .bind(pengguna_id)
        .fetch_optional(&mut **tx)
        .await?;
        use crate::audit::Peran;
        match peran.as_deref() {
            Some("platform_admin") => Ok(Peran::PlatformAdmin),
            Some("auditor") => Ok(Peran::Auditor),
            Some("operator") => Ok(Peran::Operator),
            Some("client_owner") => Ok(Peran::ClientOwner),
            _ => Err(GalatMfa::DataRusak),
        }
    }

    /// Issue a fresh 128-bit registration code bound to `target_id`, revoking the
    /// previous active code in the same transaction (AC-8).
    pub async fn terbit_kode_daftar(
        &self,
        penerbit_id: Uuid,
        target_id: Uuid,
        sekarang: DateTime<Utc>,
    ) -> Result<(Uuid, KodeDaftar), GalatMfa> {
        if penerbit_id == target_id {
            return Err(GalatMfa::PenerbitSamaDenganTarget);
        }
        let mut tx = self.pool.begin().await?;

        sqlx::query(
            "UPDATE kode_daftar_mfa SET gugur_at=$2, gugur_alasan='diganti' \
             WHERE pengguna_id=$1 AND dipakai_at IS NULL AND gugur_at IS NULL",
        )
        .bind(target_id)
        .bind(sekarang)
        .execute(&mut *tx)
        .await?;

        let mut raw = [0u8; 16];
        getrandom::fill(&mut raw).map_err(|_| GalatMfa::AcakGagal)?;
        let plaintext = totp::encode(&raw);
        let hash = hex(&sha2::Sha256::digest(plaintext.as_bytes()));
        let kode_id = Uuid::now_v7();
        let kedaluwarsa = sekarang + chrono::Duration::hours(UMUR_KODE_DAFTAR_JAM);

        sqlx::query(
            "INSERT INTO kode_daftar_mfa \
             (kode_id, pengguna_id, kode_hash, diterbitkan_oleh, diterbitkan_at, kedaluwarsa_at) \
             VALUES ($1,$2,$3,$4,$5,$6)",
        )
        .bind(kode_id)
        .bind(target_id)
        .bind(&hash)
        .bind(penerbit_id)
        .bind(sekarang)
        .bind(kedaluwarsa)
        .execute(&mut *tx)
        .await?;

        tx.commit().await?;
        Ok((kode_id, KodeDaftar(NilaiAman::baru(plaintext))))
    }

    /// Check a registration code without consuming it. A wrong value increments
    /// `gagal_count`; the fifth one sets `gugur_alasan = 'gagal_5x'` (AC-8).
    pub async fn periksa_kode_daftar(
        &self,
        target_id: Uuid,
        kode_input: &str,
        sekarang: DateTime<Utc>,
    ) -> HasilPeriksa {
        match self.periksa_dalam(target_id, kode_input, sekarang).await {
            Ok(()) => HasilPeriksa::Berlaku,
            Err(e) => HasilPeriksa::TidakBerlaku(e),
        }
    }

    async fn periksa_dalam(
        &self,
        target_id: Uuid,
        kode_input: &str,
        sekarang: DateTime<Utc>,
    ) -> Result<(), GalatMfa> {
        let mut tx = self.pool.begin().await?;
        let baris: Option<(Uuid, i32, DateTime<Utc>)> = sqlx::query_as(
            "SELECT kode_id, gagal_count, kedaluwarsa_at FROM kode_daftar_mfa \
             WHERE pengguna_id=$1 AND dipakai_at IS NULL AND gugur_at IS NULL",
        )
        .bind(target_id)
        .fetch_optional(&mut *tx)
        .await?;
        let (kode_id, count, kedaluwarsa) = baris.ok_or(GalatMfa::KodeDaftarTidakBerlaku)?;
        if sekarang >= kedaluwarsa {
            return Err(GalatMfa::KodeDaftarTidakBerlaku);
        }

        let hash_input = hex(&sha2::Sha256::digest(kode_input.as_bytes()));
        let hash_db: String =
            sqlx::query_scalar("SELECT kode_hash FROM kode_daftar_mfa WHERE kode_id=$1")
                .bind(kode_id)
                .fetch_one(&mut *tx)
                .await?;
        if sama_konstan(hash_input.as_bytes(), hash_db.as_bytes()) {
            return Ok(());
        }

        let count = count + 1;
        if count >= BATAS_GAGAL_KODE_DAFTAR {
            sqlx::query(
                "UPDATE kode_daftar_mfa SET gagal_count=$2, gugur_at=$3, gugur_alasan='gagal_5x' \
                 WHERE kode_id=$1",
            )
            .bind(kode_id)
            .bind(count)
            .bind(sekarang)
            .execute(&mut *tx)
            .await?;
        } else {
            sqlx::query("UPDATE kode_daftar_mfa SET gagal_count=$2 WHERE kode_id=$1")
                .bind(kode_id)
                .bind(count)
                .execute(&mut *tx)
                .await?;
        }
        // Commit the failure counter; otherwise the rollback erases every attempt
        // and the 5-strike rule can never fire (AC-8).
        tx.commit().await?;
        Err(GalatMfa::KodeDaftarTidakBerlaku)
    }

    fn buka_rahasia(&self, pengguna_id: Uuid, b: &BarisMfa) -> Result<String, GalatMfa> {
        // The column is bytea; AES-GCM nonces are 12 bytes.
        let nonce = b
            .secret_nonce
            .as_slice()
            .try_into()
            .map_err(|_| GalatMfa::DataRusak)?;
        let amplop = crate::secret::Amplop {
            nama: pengguna_id.to_string(),
            ciphertext: b.secret_ciphertext.clone(),
            dek_terbungkus: b.secret_dek_terbungkus.clone(),
            nonce,
            kek_id: b.kek_id.clone(),
        };
        Ok(self.brankas.dekripsi(&amplop)?.ke_dalam())
    }
}

#[derive(sqlx::FromRow)]
struct BarisMfa {
    status: String,
    #[allow(dead_code)]
    langkah_terakhir: i64,
    secret_ciphertext: Vec<u8>,
    secret_dek_terbungkus: Vec<u8>,
    secret_nonce: Vec<u8>,
    kek_id: String,
}

fn hex(data: &[u8]) -> String {
    const TAB: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(data.len() * 2);
    for &b in data {
        out.push(TAB[(b >> 4) as usize] as char);
        out.push(TAB[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_sha256_adalah_64_karakter() {
        assert_eq!(hex(&[0xff, 0x00]).len(), 4);
        assert_eq!(
            hex(&sha2::Sha256::digest(b"a")),
            "ca978112ca1bbdcafac231b39a23dc4da786eff8147c4e72b9807785afee48bb"
        );
    }

    #[test]
    fn pengguna_id_menjadi_aad() {
        // AAD = nama amplop; a ciphertext sealed for A must not open as B (AC-6).
        let brankas = crate::secret::Brankas::dari_isi_kek(&"5a".repeat(32)).unwrap();
        let a = brankas
            .enkripsi(
                "aaaaaaaa-0000-0000-0000-000000000000",
                &NilaiAman::baru("rahasia".into()),
            )
            .unwrap();
        let mut salah = a.clone();
        salah.nama = "bbbbbbbb-0000-0000-0000-000000000000".into();
        assert!(brankas.dekripsi(&salah).is_err());
        assert!(brankas.dekripsi(&a).is_ok());
    }
}
