//! Penerap migrasi skema (ADR-0001 §7.2: role `poolrouter_owner` hanya untuk job migrasi).
//!
//! `backend/migrations/` disematkan ke biner saat build, jadi image memuat skema yang sama dengan
//! revisi kodenya dan tidak butuh mount atau sqlx-cli. Biner `migrate` (service Compose `migrate`)
//! memakai `Component::Migrate`, yaitu hanya role owner, terpisah dari `backend-cli` yang berjalan
//! sebagai `pr_admin_api` (ADR-0003 §3.6c).
//!
//! Konvensi berkas sama dengan penerap uji (`backend/db/test/run.sh`): satu berkas per versi, tanpa
//! pasangan `.down` (rollback ada di `backend/db/rollback/`), dan baris pertama `-- no-transaction`
//! untuk `CREATE INDEX CONCURRENTLY` (ARC-199). sqlx mengenali penanda itu.

use sqlx::PgPool;
use sqlx::migrate::{MigrateError, Migrator};

/// Semua migrasi di `backend/migrations/`, urut menurut versi.
pub static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

/// Terapkan migrasi yang belum tercatat di `_sqlx_migrations`. Aman diulang: yang sudah tercatat
/// dilewati, dan checksum berkas yang sudah diterapkan dicek (berkas lama tidak boleh diubah).
///
/// Basis data yang pernah dimigrasi manual lewat psql tidak punya `_sqlx_migrations`, sehingga semua
/// migrasi akan dicoba lagi dan gagal pada objek yang sudah ada. Tandai dulu secara manual.
pub async fn terapkan(pool: &PgPool) -> Result<(), MigrateError> {
    MIGRATOR.run(pool).await
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::MIGRATOR;

    fn berkas_sql() -> Vec<(i64, bool)> {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../migrations");
        let mut hasil: Vec<(i64, bool)> = std::fs::read_dir(&dir)
            .expect("backend/migrations terbaca")
            .map(|e| e.expect("entri direktori").path())
            .filter(|p| p.extension().is_some_and(|x| x == "sql"))
            .map(|p| {
                let nama = p.file_name().and_then(|n| n.to_str()).expect("nama berkas");
                let versi = nama
                    .split('_')
                    .next()
                    .and_then(|v| v.parse::<i64>().ok())
                    .unwrap_or_else(|| panic!("{nama}: awalan bukan versi numerik"));
                let isi = std::fs::read_to_string(&p).expect("isi berkas");
                (versi, isi.starts_with("-- no-transaction"))
            })
            .collect();
        hasil.sort_unstable();
        hasil
    }

    #[test]
    fn migrator_memuat_setiap_berkas_migrasi_dengan_mode_transaksinya() {
        let di_disk = berkas_sql();
        assert!(
            !di_disk.is_empty(),
            "tidak ada migrasi di backend/migrations"
        );
        let tersemat: Vec<(i64, bool)> = MIGRATOR.iter().map(|m| (m.version, m.no_tx)).collect();
        assert_eq!(tersemat, di_disk);
    }
}
