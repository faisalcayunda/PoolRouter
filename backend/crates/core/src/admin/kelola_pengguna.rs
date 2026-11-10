//! Layanan pengguna dan peran admin (ADR-0003 rev 6 §3.6c, spec f0-admin-api AC-39–AC-42).
//!
//! Modul ini dipakai `backend-cli` sekarang dan oleh handler admin API pengguna/peran di F1, supaya
//! aturan A-02, audit, dan pencabutan sesi hanya ada di satu tempat. Setiap operasi tulis berjalan
//! dalam satu transaksi: efek, baris `audit_konfigurasi` (entitas `admin_user`), baris `audit_akses`
//! `ubah_peran` (perubahan peran dan status aktif), serta penghapusan sesi target ikut commit atau
//! rollback bersama.
//!
//! Pelaku di CLI dinyatakan lewat `--oleh`, tidak dibuktikan (risiko sisa §3.6c). Karena itu baris
//! `audit_akses` dari jalur ini memakai penanda tetap [`SUMBER_IP_CLI`] dan [`SESI_CLI`].
//!
//! Password tidak pernah masuk struktur audit, pesan galat, maupun `Debug`: modul ini hanya menerima
//! `&str` lalu langsung meng-hash-nya di luar transaksi.

use chrono::{DateTime, Utc};
use serde_json::{Value, json};
use sqlx::{PgConnection, PgPool, Postgres, Transaction};
use uuid::Uuid;

use crate::admin::password::{HashGagal, KebijakanPassword, ParameterArgon2, hash_password};
use crate::audit::{
    AksesBaru, Aksi, AksiKonfigurasi, Alasan, AlasanTidakValid, EntitasKonfigurasi, GalatAudit,
    KonfigurasiBaru, Pelaku, Penulis, Peran,
};
use crate::config::Environment;

/// Nilai `audit_akses.sumber_ip` untuk aksi lewat CLI (kolom NOT NULL; CLI tidak punya IP klien).
pub const SUMBER_IP_CLI: &str = "cli";
/// Nilai `audit_akses.sesi_id` untuk aksi lewat CLI (kolom NOT NULL; CLI tidak punya sesi).
pub const SESI_CLI: &str = "cli";

/// Kunci advisory untuk menyerialkan bootstrap, supaya dua bootstrap bersamaan tidak sama-sama
/// melihat tabel kosong.
const KUNCI_BOOTSTRAP: i64 = 0x504f_4f4c_4253_5450; // "POOLBSTP"

/// Nama pengguna seed per peran (AC-42). Semuanya sintetis dan hanya ada di `dev`/`ci`.
pub const SEED_PLATFORM_ADMIN: &str = "seed-platform-admin";
/// Platform admin kedua yang menyetujui peran auditor dan operator seed. Dibutuhkan karena CHECK
/// `peran_pemisahan_tugas` mewajibkan penyetuju berbeda dari pengaju dan target.
pub const SEED_PENYETUJU: &str = "seed-penyetuju";
pub const SEED_OPERATOR: &str = "seed-operator";
pub const SEED_AUDITOR: &str = "seed-auditor";
pub const SEED_CLIENT_OWNER: &str = "seed-client-owner";
/// Platform admin yang juga auditor (uji e2e US-16, ADR-0003 §3.6c).
pub const SEED_ADMIN_AUDITOR: &str = "seed-admin-auditor";
/// Client sintetis tempat `seed-client-owner` terikat.
pub const SEED_CLIENT_NAMA: &str = "Client Sintetis Seed";

/// Alasan yang dicatat di audit untuk semua perubahan oleh `seed pengguna`.
const ALASAN_SEED: &str = "seed pengguna sintetis dev/ci (ADR-0003 §3.6c)";

/// Kenapa sebuah operasi pengguna/peran ditolak. Pesan tidak pernah memuat password atau hash.
#[derive(Debug, thiserror::Error)]
pub enum GalatKelola {
    #[error("pelaku `{0}` tidak ditemukan")]
    PelakuTidakAda(String),
    #[error("pelaku `{0}` tidak aktif")]
    PelakuTidakAktif(String),
    #[error("pelaku `{0}` bukan platform admin aktif")]
    PelakuBukanAdmin(String),
    #[error("pengguna `{0}` tidak ditemukan")]
    TargetTidakAda(String),
    #[error("username `{0}` sudah dipakai")]
    UsernameDipakai(String),
    #[error("username wajib huruf kecil dan 3–100 karakter")]
    UsernameTidakValid,
    #[error("A-02: pelaku tidak boleh memberi dirinya sendiri peran {0}")]
    PeranDiriSendiri(Peran),
    #[error("A-02: peran {0} wajib disetujui platform admin lain (--disetujui-oleh)")]
    PenyetujuWajib(Peran),
    #[error("A-02: penyetuju harus berbeda dari pelaku dan dari target")]
    PenyetujuTidakSah,
    #[error("peran client_owner hanya untuk pengguna yang terikat client (--client saat buat)")]
    ClientOwnerTanpaClient,
    #[error("client `{0}` tidak ditemukan")]
    ClientTidakAda(Uuid),
    #[error("peran auditor di lingkungan poc wajib --berlaku-sampai (R-12)")]
    BerlakuSampaiWajib,
    #[error("--berlaku-sampai harus di masa depan")]
    BerlakuSampaiLewat,
    #[error("pengguna sudah memegang peran {0}")]
    PeranSudahAda(Peran),
    #[error("pengguna tidak memegang peran {0}")]
    PeranTidakAda(Peran),
    #[error("pengguna sudah {0}")]
    StatusSama(&'static str),
    #[error("pelaku tidak boleh menonaktifkan dirinya sendiri")]
    NonaktifkanDiriSendiri,
    #[error(
        "`{0}` adalah platform_admin aktif terakhir; mencabut peran atau menonaktifkannya \
         mengunci sistem. Angkat platform_admin lain terlebih dahulu"
    )]
    AdminTerakhir(String),
    #[error("bootstrap hanya boleh saat tabel pengguna kosong")]
    BootstrapDitolak,
    #[error("seed hanya boleh di POOLROUTER_ENV dev atau ci")]
    SeedDitolak,
    #[error("{0}")]
    Password(HashGagal),
    #[error("{0}")]
    Alasan(#[from] AlasanTidakValid),
    #[error("audit: {0}")]
    Audit(#[from] GalatAudit),
    #[error("basis data: {0}")]
    BasisData(#[from] sqlx::Error),
}

/// Satu pengguna untuk perintah daftar. Tanpa hash password.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RingkasanPengguna {
    pub pengguna_id: Uuid,
    pub username: String,
    pub aktif: bool,
    pub client_id: Option<Uuid>,
    /// Peran yang berlaku sekarang (belum dicabut dan belum lewat `berlaku_sampai`), urut nama.
    pub peran: Vec<Peran>,
}

/// Permintaan memberi peran.
#[derive(Debug, Clone)]
pub struct PemberianPeran<'a> {
    pub target: &'a str,
    pub peran: Peran,
    pub oleh: &'a str,
    pub disetujui_oleh: Option<&'a str>,
    pub berlaku_sampai: Option<DateTime<Utc>>,
}

/// Hasil `seed pengguna`: berapa baris yang benar-benar dibuat (0 pada jalan kedua).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HasilSeed {
    pub pengguna_baru: u32,
    pub peran_baru: u32,
    pub client_baru: u32,
}

/// Pemeriksaan bentuk username sesuai CHECK `admin_user.username`.
pub fn username_sah(username: &str) -> bool {
    let n = username.chars().count();
    (3..=100).contains(&n) && username == username.to_lowercase()
}

fn hash(password: &str) -> Result<String, GalatKelola> {
    let h = hash_password(password, KebijakanPassword::f0(), ParameterArgon2::OWASP)
        .map_err(GalatKelola::Password)?;
    Ok(h.to_string())
}

#[derive(Debug, Clone)]
struct BarisPengguna {
    pengguna_id: Uuid,
    aktif: bool,
    client_id: Option<Uuid>,
}

async fn cari(
    conn: &mut PgConnection,
    username: &str,
) -> Result<Option<BarisPengguna>, sqlx::Error> {
    let baris: Option<(Uuid, bool, Option<Uuid>)> =
        sqlx::query_as("SELECT pengguna_id, aktif, client_id FROM admin_user WHERE username = $1")
            .bind(username)
            .fetch_optional(conn)
            .await?;
    Ok(baris.map(|(pengguna_id, aktif, client_id)| BarisPengguna {
        pengguna_id,
        aktif,
        client_id,
    }))
}

async fn peran_berlaku(
    conn: &mut PgConnection,
    pengguna_id: Uuid,
    sekarang: DateTime<Utc>,
) -> Result<Vec<Peran>, sqlx::Error> {
    let baris: Vec<(String,)> = sqlx::query_as(
        "SELECT peran FROM admin_user_peran \
         WHERE pengguna_id = $1 AND dicabut_at IS NULL \
           AND (berlaku_sampai IS NULL OR berlaku_sampai > $2) \
         ORDER BY peran",
    )
    .bind(pengguna_id)
    .bind(sekarang)
    .fetch_all(conn)
    .await?;
    Ok(baris
        .iter()
        .filter_map(|(p,)| super::pengguna::peran_dari_teks(p))
        .collect())
}

/// Pelaku wajib ada, aktif, dan memegang peran platform admin yang berlaku (§3.6c).
async fn pelaku_admin(
    conn: &mut PgConnection,
    username: &str,
    sekarang: DateTime<Utc>,
) -> Result<Uuid, GalatKelola> {
    let p = cari(conn, username)
        .await?
        .ok_or_else(|| GalatKelola::PelakuTidakAda(username.to_owned()))?;
    if !p.aktif {
        return Err(GalatKelola::PelakuTidakAktif(username.to_owned()));
    }
    if !peran_berlaku(conn, p.pengguna_id, sekarang)
        .await?
        .contains(&Peran::PlatformAdmin)
    {
        return Err(GalatKelola::PelakuBukanAdmin(username.to_owned()));
    }
    Ok(p.pengguna_id)
}

async fn target(conn: &mut PgConnection, username: &str) -> Result<BarisPengguna, GalatKelola> {
    cari(conn, username)
        .await?
        .ok_or_else(|| GalatKelola::TargetTidakAda(username.to_owned()))
}

async fn sisipkan_pengguna(
    conn: &mut PgConnection,
    username: &str,
    password_hash: &str,
    client_id: Option<Uuid>,
) -> Result<Uuid, GalatKelola> {
    let id = Uuid::now_v7();
    let hasil = sqlx::query(
        "INSERT INTO admin_user (pengguna_id, username, password_hash, client_id) \
         VALUES ($1, $2, $3, $4) ON CONFLICT (username) DO NOTHING",
    )
    .bind(id)
    .bind(username)
    .bind(password_hash)
    .bind(client_id)
    .execute(conn)
    .await?;
    if hasil.rows_affected() == 0 {
        return Err(GalatKelola::UsernameDipakai(username.to_owned()));
    }
    Ok(id)
}

/// Tulis peran. Peran yang pernah dicabut atau kedaluwarsa diperbarui di baris yang sama karena
/// kunci primernya `(pengguna_id, peran)`.
async fn simpan_peran(
    conn: &mut PgConnection,
    target: Uuid,
    peran: Peran,
    diajukan_oleh: Uuid,
    disetujui_oleh: Option<Uuid>,
    berlaku_sampai: Option<DateTime<Utc>>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO admin_user_peran \
           (pengguna_id, peran, diajukan_oleh, disetujui_oleh, berlaku_sampai) \
         VALUES ($1, $2, $3, $4, $5) \
         ON CONFLICT (pengguna_id, peran) DO UPDATE SET \
           diajukan_oleh = EXCLUDED.diajukan_oleh, disetujui_oleh = EXCLUDED.disetujui_oleh, \
           berlaku_sampai = EXCLUDED.berlaku_sampai, diberikan_at = now(), dicabut_at = NULL",
    )
    .bind(target)
    .bind(peran.as_str())
    .bind(diajukan_oleh)
    .bind(disetujui_oleh)
    .bind(berlaku_sampai)
    .execute(conn)
    .await?;
    Ok(())
}

/// Hapus semua sesi target (§3.6: perubahan peran/penonaktifan berlaku seketika).
async fn cabut_sesi(conn: &mut PgConnection, target: Uuid) -> Result<u64, sqlx::Error> {
    Ok(
        sqlx::query("DELETE FROM admin_session WHERE pengguna_id = $1")
            .bind(target)
            .execute(conn)
            .await?
            .rows_affected(),
    )
}

/// Satu baris `audit_akses` `ubah_peran`. Target dan perubahan masuk `filter_kueri`, karena
/// `target_pengguna_id` hanya boleh terisi untuk aksi MFA (`chk_audit_target_pengguna`).
async fn catat_ubah_peran(
    tx: &mut Transaction<'static, Postgres>,
    pelaku: Uuid,
    rincian: Value,
    alasan: &Alasan,
) -> Result<(), GalatKelola> {
    let baris = AksesBaru::berhasil(Pelaku::baru(pelaku, Peran::PlatformAdmin), Aksi::UbahPeran)
        .dengan_filter(rincian)
        .dengan_alasan(alasan.clone())
        .dengan_sumber_ip(SUMBER_IP_CLI)
        .dengan_sesi(SESI_CLI);
    Penulis::baru(tx).catat_akses(baris).await?;
    Ok(())
}

async fn catat_konfigurasi(
    tx: &mut Transaction<'static, Postgres>,
    baris: KonfigurasiBaru,
) -> Result<(), GalatKelola> {
    Penulis::baru(tx).catat_konfigurasi(&baris).await?;
    Ok(())
}

fn nilai_pengguna(username: &str, aktif: bool, client_id: Option<Uuid>) -> Value {
    json!({ "username": username, "aktif": aktif, "client_id": client_id })
}

/// Bootstrap platform admin pertama. Hanya berhasil bila `admin_user` kosong; pemeriksaan dan
/// penyisipan diserialkan dengan advisory lock di transaksi yang sama.
pub async fn bootstrap(
    pool: &PgPool,
    username: &str,
    password: &str,
    alasan: &Alasan,
) -> Result<Uuid, GalatKelola> {
    if !username_sah(username) {
        return Err(GalatKelola::UsernameTidakValid);
    }
    let password_hash = hash(password)?;
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(KUNCI_BOOTSTRAP)
        .execute(&mut *tx)
        .await?;
    let ada: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM admin_user)")
        .fetch_one(&mut *tx)
        .await?;
    if ada {
        return Err(GalatKelola::BootstrapDitolak);
    }
    let id = buat_admin_pertama(&mut tx, username, &password_hash, alasan, "bootstrap").await?;
    tx.commit().await?;
    Ok(id)
}

/// Platform admin yang mengajukan perannya sendiri. Hanya dipanggil oleh bootstrap (tabel kosong) dan
/// oleh seed (`dev`/`ci`); keduanya sudah memeriksa syaratnya sebelum ke sini.
async fn buat_admin_pertama(
    tx: &mut Transaction<'static, Postgres>,
    username: &str,
    password_hash: &str,
    alasan: &Alasan,
    operasi: &str,
) -> Result<Uuid, GalatKelola> {
    let id = sisipkan_pengguna(tx, username, password_hash, None).await?;
    simpan_peran(tx, id, Peran::PlatformAdmin, id, None, None).await?;
    catat_konfigurasi(
        tx,
        KonfigurasiBaru {
            pelaku: id,
            entitas: EntitasKonfigurasi::AdminUser,
            entitas_id: Some(id),
            aksi: AksiKonfigurasi::Buat,
            nilai_lama: None,
            nilai_baru: Some(nilai_pengguna(username, true, None)),
            alasan: alasan.clone(),
        },
    )
    .await?;
    catat_ubah_peran(
        tx,
        id,
        json!({ "operasi": operasi, "target_pengguna_id": id, "target": username,
                "peran": Peran::PlatformAdmin.as_str(), "perubahan": "beri" }),
        alasan,
    )
    .await?;
    Ok(id)
}

/// Buat pengguna tanpa peran. Peran diberikan terpisah lewat [`beri_peran`].
pub async fn buat_pengguna(
    pool: &PgPool,
    username: &str,
    client_id: Option<Uuid>,
    password: &str,
    oleh: &str,
    alasan: &Alasan,
    sekarang: DateTime<Utc>,
) -> Result<Uuid, GalatKelola> {
    if !username_sah(username) {
        return Err(GalatKelola::UsernameTidakValid);
    }
    let password_hash = hash(password)?;
    let mut tx = pool.begin().await?;
    let pelaku = pelaku_admin(&mut tx, oleh, sekarang).await?;
    if let Some(c) = client_id {
        let ada: bool =
            sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM client WHERE client_id = $1)")
                .bind(c)
                .fetch_one(&mut *tx)
                .await?;
        if !ada {
            return Err(GalatKelola::ClientTidakAda(c));
        }
    }
    let id = sisipkan_pengguna(&mut tx, username, &password_hash, client_id).await?;
    catat_konfigurasi(
        &mut tx,
        KonfigurasiBaru {
            pelaku,
            entitas: EntitasKonfigurasi::AdminUser,
            entitas_id: Some(id),
            aksi: AksiKonfigurasi::Buat,
            nilai_lama: None,
            nilai_baru: Some(nilai_pengguna(username, true, client_id)),
            alasan: alasan.clone(),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

/// Fakta yang dibutuhkan pemeriksaan A-02, dikumpulkan dari basis data oleh pemanggil.
#[derive(Debug, Clone, Copy)]
pub struct SyaratPemberian {
    pub peran: Peran,
    pub pelaku: Uuid,
    pub target: Uuid,
    pub penyetuju: Option<Uuid>,
    pub target_punya_client: bool,
    pub berlaku_sampai: Option<DateTime<Utc>>,
}

/// Pemeriksaan A-02 dan syarat peran sebelum menyentuh basis data. Fungsi murni supaya aturannya
/// teruji tanpa PostgreSQL.
pub fn periksa_pemberian(
    s: &SyaratPemberian,
    env: Environment,
    sekarang: DateTime<Utc>,
) -> Result<(), GalatKelola> {
    if s.peran.butuh_persetujuan() {
        if s.pelaku == s.target {
            return Err(GalatKelola::PeranDiriSendiri(s.peran));
        }
        let Some(penyetuju) = s.penyetuju else {
            return Err(GalatKelola::PenyetujuWajib(s.peran));
        };
        if penyetuju == s.pelaku || penyetuju == s.target {
            return Err(GalatKelola::PenyetujuTidakSah);
        }
    }
    if s.peran == Peran::ClientOwner && !s.target_punya_client {
        return Err(GalatKelola::ClientOwnerTanpaClient);
    }
    if s.peran == Peran::Auditor && env == Environment::Poc && s.berlaku_sampai.is_none() {
        return Err(GalatKelola::BerlakuSampaiWajib);
    }
    if s.berlaku_sampai.is_some_and(|b| b <= sekarang) {
        return Err(GalatKelola::BerlakuSampaiLewat);
    }
    Ok(())
}

/// Beri peran. Menulis peran, satu `audit_akses` `ubah_peran`, satu `audit_konfigurasi`, dan
/// mencabut semua sesi target dalam satu transaksi (AC-41).
pub async fn beri_peran(
    pool: &PgPool,
    p: &PemberianPeran<'_>,
    alasan: &Alasan,
    env: Environment,
    sekarang: DateTime<Utc>,
) -> Result<(), GalatKelola> {
    let mut tx = pool.begin().await?;
    let pelaku = pelaku_admin(&mut tx, p.oleh, sekarang).await?;
    let t = target(&mut tx, p.target).await?;
    let penyetuju = match p.disetujui_oleh {
        Some(u) => Some(pelaku_admin(&mut tx, u, sekarang).await?),
        None => None,
    };
    periksa_pemberian(
        &SyaratPemberian {
            peran: p.peran,
            pelaku,
            target: t.pengguna_id,
            penyetuju,
            target_punya_client: t.client_id.is_some(),
            berlaku_sampai: p.berlaku_sampai,
        },
        env,
        sekarang,
    )?;
    let lama = peran_berlaku(&mut tx, t.pengguna_id, sekarang).await?;
    if lama.contains(&p.peran) {
        return Err(GalatKelola::PeranSudahAda(p.peran));
    }
    simpan_peran(
        &mut tx,
        t.pengguna_id,
        p.peran,
        pelaku,
        penyetuju,
        p.berlaku_sampai,
    )
    .await?;
    let sesi = cabut_sesi(&mut tx, t.pengguna_id).await?;
    let mut baru = lama.clone();
    baru.push(p.peran);
    baru.sort_by_key(|r| r.as_str());
    tulis_audit_peran(
        &mut tx,
        pelaku,
        t.pengguna_id,
        p.target,
        p.peran,
        "beri",
        &lama,
        &baru,
        json!({ "disetujui_oleh": penyetuju, "berlaku_sampai": p.berlaku_sampai, "sesi_dicabut": sesi }),
        alasan,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Cabut peran, dengan audit dan pencabutan sesi yang sama dengan [`beri_peran`].
pub async fn cabut_peran(
    pool: &PgPool,
    target_username: &str,
    peran: Peran,
    oleh: &str,
    alasan: &Alasan,
    sekarang: DateTime<Utc>,
) -> Result<(), GalatKelola> {
    let mut tx = pool.begin().await?;
    let pelaku = pelaku_admin(&mut tx, oleh, sekarang).await?;
    let t = target(&mut tx, target_username).await?;
    let lama = peran_berlaku(&mut tx, t.pengguna_id, sekarang).await?;
    if !lama.contains(&peran) {
        return Err(GalatKelola::PeranTidakAda(peran));
    }
    sqlx::query(
        "UPDATE admin_user_peran SET dicabut_at = now() \
         WHERE pengguna_id = $1 AND peran = $2 AND dicabut_at IS NULL",
    )
    .bind(t.pengguna_id)
    .bind(peran.as_str())
    .execute(&mut *tx)
    .await?;
    let sesi = cabut_sesi(&mut tx, t.pengguna_id).await?;
    let baru: Vec<Peran> = lama.iter().copied().filter(|r| *r != peran).collect();
    tulis_audit_peran(
        &mut tx,
        pelaku,
        t.pengguna_id,
        target_username,
        peran,
        "cabut",
        &lama,
        &baru,
        json!({ "sesi_dicabut": sesi }),
        alasan,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

fn nama_peran(daftar: &[Peran]) -> Vec<&'static str> {
    daftar.iter().map(|r| r.as_str()).collect()
}

#[allow(clippy::too_many_arguments)]
async fn tulis_audit_peran(
    tx: &mut Transaction<'static, Postgres>,
    pelaku: Uuid,
    target_id: Uuid,
    target_username: &str,
    peran: Peran,
    perubahan: &str,
    lama: &[Peran],
    baru: &[Peran],
    tambahan: Value,
    alasan: &Alasan,
) -> Result<(), GalatKelola> {
    catat_ubah_peran(
        tx,
        pelaku,
        json!({ "operasi": "peran", "target_pengguna_id": target_id, "target": target_username,
                "peran": peran.as_str(), "perubahan": perubahan, "rincian": tambahan }),
        alasan,
    )
    .await?;
    catat_konfigurasi(
        tx,
        KonfigurasiBaru {
            pelaku,
            entitas: EntitasKonfigurasi::AdminUser,
            entitas_id: Some(target_id),
            aksi: AksiKonfigurasi::Ubah,
            nilai_lama: Some(json!({ "peran": nama_peran(lama) })),
            nilai_baru: Some(json!({ "peran": nama_peran(baru) })),
            alasan: alasan.clone(),
        },
    )
    .await
}

/// Nonaktifkan (`aktif = false`) atau aktifkan kembali pengguna. Penonaktifan mencabut semua sesinya.
pub async fn ubah_aktif(
    pool: &PgPool,
    target_username: &str,
    aktif: bool,
    oleh: &str,
    alasan: &Alasan,
    sekarang: DateTime<Utc>,
) -> Result<(), GalatKelola> {
    let mut tx = pool.begin().await?;
    let pelaku = pelaku_admin(&mut tx, oleh, sekarang).await?;
    let t = target(&mut tx, target_username).await?;
    if !aktif && t.pengguna_id == pelaku {
        return Err(GalatKelola::NonaktifkanDiriSendiri);
    }
    if t.aktif == aktif {
        return Err(GalatKelola::StatusSama(if aktif {
            "aktif"
        } else {
            "nonaktif"
        }));
    }
    sqlx::query("UPDATE admin_user SET aktif = $2, diubah_at = now() WHERE pengguna_id = $1")
        .bind(t.pengguna_id)
        .bind(aktif)
        .execute(&mut *tx)
        .await?;
    let sesi = cabut_sesi(&mut tx, t.pengguna_id).await?;
    let operasi = if aktif { "aktifkan" } else { "nonaktifkan" };
    catat_ubah_peran(
        &mut tx,
        pelaku,
        json!({ "operasi": operasi, "target_pengguna_id": t.pengguna_id,
                "target": target_username, "sesi_dicabut": sesi }),
        alasan,
    )
    .await?;
    catat_konfigurasi(
        &mut tx,
        KonfigurasiBaru {
            pelaku,
            entitas: EntitasKonfigurasi::AdminUser,
            entitas_id: Some(t.pengguna_id),
            aksi: if aktif {
                AksiKonfigurasi::Aktifkan
            } else {
                AksiKonfigurasi::Nonaktifkan
            },
            nilai_lama: Some(json!({ "aktif": t.aktif })),
            nilai_baru: Some(json!({ "aktif": aktif })),
            alasan: alasan.clone(),
        },
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Daftar pengguna dan peran yang berlaku, urut username.
pub async fn daftar(
    pool: &PgPool,
    sekarang: DateTime<Utc>,
) -> Result<Vec<RingkasanPengguna>, GalatKelola> {
    let mut conn = pool.acquire().await?;
    let baris: Vec<(Uuid, String, bool, Option<Uuid>)> = sqlx::query_as(
        "SELECT pengguna_id, username, aktif, client_id FROM admin_user ORDER BY username",
    )
    .fetch_all(&mut *conn)
    .await?;
    let mut hasil = Vec::with_capacity(baris.len());
    for (pengguna_id, username, aktif, client_id) in baris {
        let peran = peran_berlaku(&mut conn, pengguna_id, sekarang).await?;
        hasil.push(RingkasanPengguna {
            pengguna_id,
            username,
            aktif,
            client_id,
            peran,
        });
    }
    Ok(hasil)
}

/// `seed pengguna` (AC-42): satu pengguna sintetis per peran plus platform admin yang juga auditor,
/// semuanya dengan password seed yang sama. Ditolak tanpa perubahan di luar `dev`/`ci`. Idempoten:
/// pengguna, client, dan peran yang sudah ada tidak dibuat ulang dan tidak menulis audit baru.
pub async fn seed_pengguna(
    pool: &PgPool,
    env: Environment,
    password: &str,
    sekarang: DateTime<Utc>,
) -> Result<HasilSeed, GalatKelola> {
    if !env.allows_seed() {
        return Err(GalatKelola::SeedDitolak);
    }
    let alasan = Alasan::baru(ALASAN_SEED)?;
    let password_hash = hash(password)?;
    let mut hasil = HasilSeed::default();
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(KUNCI_BOOTSTRAP)
        .execute(&mut *tx)
        .await?;

    let admin = match cari(&mut tx, SEED_PLATFORM_ADMIN).await? {
        Some(p) => p.pengguna_id,
        None => {
            hasil.pengguna_baru += 1;
            hasil.peran_baru += 1;
            buat_admin_pertama(
                &mut tx,
                SEED_PLATFORM_ADMIN,
                &password_hash,
                &alasan,
                "seed",
            )
            .await?
        }
    };

    let client = seed_client(&mut tx, admin, &alasan, &mut hasil).await?;

    let rencana: [(&str, Option<Uuid>, &[Peran]); 5] = [
        (SEED_PENYETUJU, None, &[Peran::PlatformAdmin]),
        (SEED_OPERATOR, None, &[Peran::Operator]),
        (SEED_AUDITOR, None, &[Peran::Auditor]),
        (SEED_CLIENT_OWNER, Some(client), &[Peran::ClientOwner]),
        (
            SEED_ADMIN_AUDITOR,
            None,
            &[Peran::PlatformAdmin, Peran::Auditor],
        ),
    ];
    let mut penyetuju = None;
    for (username, client_id, daftar_peran) in rencana {
        let id = match cari(&mut tx, username).await? {
            Some(p) => p.pengguna_id,
            None => {
                let id = sisipkan_pengguna(&mut tx, username, &password_hash, client_id).await?;
                catat_konfigurasi(
                    &mut tx,
                    KonfigurasiBaru {
                        pelaku: admin,
                        entitas: EntitasKonfigurasi::AdminUser,
                        entitas_id: Some(id),
                        aksi: AksiKonfigurasi::Buat,
                        nilai_lama: None,
                        nilai_baru: Some(nilai_pengguna(username, true, client_id)),
                        alasan: alasan.clone(),
                    },
                )
                .await?;
                hasil.pengguna_baru += 1;
                id
            }
        };
        if username == SEED_PENYETUJU {
            penyetuju = Some(id);
        }
        let mut sekarang_dipegang = peran_berlaku(&mut tx, id, sekarang).await?;
        for peran in daftar_peran {
            if sekarang_dipegang.contains(peran) {
                continue;
            }
            let disetujui = if peran.butuh_persetujuan() {
                penyetuju
            } else {
                None
            };
            simpan_peran(&mut tx, id, *peran, admin, disetujui, None).await?;
            let sebelum = sekarang_dipegang.clone();
            sekarang_dipegang.push(*peran);
            sekarang_dipegang.sort_by_key(|r| r.as_str());
            tulis_audit_peran(
                &mut tx,
                admin,
                id,
                username,
                *peran,
                "beri",
                &sebelum,
                &sekarang_dipegang,
                json!({ "operasi_seed": true, "disetujui_oleh": disetujui }),
                &alasan,
            )
            .await?;
            hasil.peran_baru += 1;
        }
    }
    tx.commit().await?;
    Ok(hasil)
}

async fn seed_client(
    tx: &mut Transaction<'static, Postgres>,
    admin: Uuid,
    alasan: &Alasan,
    hasil: &mut HasilSeed,
) -> Result<Uuid, GalatKelola> {
    let ada: Option<Uuid> = sqlx::query_scalar("SELECT client_id FROM client WHERE nama = $1")
        .bind(SEED_CLIENT_NAMA)
        .fetch_optional(&mut **tx)
        .await?;
    if let Some(id) = ada {
        return Ok(id);
    }
    let id = Uuid::now_v7();
    sqlx::query("INSERT INTO client (client_id, nama) VALUES ($1, $2)")
        .bind(id)
        .bind(SEED_CLIENT_NAMA)
        .execute(&mut **tx)
        .await?;
    catat_konfigurasi(
        tx,
        KonfigurasiBaru {
            pelaku: admin,
            entitas: EntitasKonfigurasi::Client,
            entitas_id: Some(id),
            aksi: AksiKonfigurasi::Buat,
            nilai_lama: None,
            nilai_baru: Some(json!({ "nama": SEED_CLIENT_NAMA })),
            alasan: alasan.clone(),
        },
    )
    .await?;
    hasil.client_baru += 1;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn t0() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 11, 9, 3, 0, 0).unwrap()
    }

    fn syarat(
        peran: Peran,
        pelaku: Uuid,
        target: Uuid,
        penyetuju: Option<Uuid>,
    ) -> SyaratPemberian {
        SyaratPemberian {
            peran,
            pelaku,
            target,
            penyetuju,
            target_punya_client: false,
            berlaku_sampai: None,
        }
    }

    fn periksa(
        peran: Peran,
        pelaku: Uuid,
        target: Uuid,
        penyetuju: Option<Uuid>,
    ) -> Result<(), GalatKelola> {
        periksa_pemberian(
            &syarat(peran, pelaku, target, penyetuju),
            Environment::Dev,
            t0(),
        )
    }

    #[test]
    fn a02_auditor_untuk_diri_sendiri_ditolak() {
        let a = Uuid::now_v7();
        let s = Uuid::now_v7();
        assert!(matches!(
            periksa(Peran::Auditor, a, a, Some(s)),
            Err(GalatKelola::PeranDiriSendiri(Peran::Auditor))
        ));
        assert!(matches!(
            periksa(Peran::Operator, a, a, Some(s)),
            Err(GalatKelola::PeranDiriSendiri(Peran::Operator))
        ));
    }

    #[test]
    fn a02_auditor_dan_operator_wajib_penyetuju_yang_berbeda() {
        let (a, t, s) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        assert!(matches!(
            periksa(Peran::Auditor, a, t, None),
            Err(GalatKelola::PenyetujuWajib(Peran::Auditor))
        ));
        assert!(matches!(
            periksa(Peran::Operator, a, t, Some(a)),
            Err(GalatKelola::PenyetujuTidakSah)
        ));
        assert!(matches!(
            periksa(Peran::Operator, a, t, Some(t)),
            Err(GalatKelola::PenyetujuTidakSah)
        ));
        assert!(periksa(Peran::Auditor, a, t, Some(s)).is_ok());
        assert!(periksa(Peran::PlatformAdmin, a, t, None).is_ok());
    }

    #[test]
    fn client_owner_butuh_client() {
        let (a, t) = (Uuid::now_v7(), Uuid::now_v7());
        assert!(matches!(
            periksa(Peran::ClientOwner, a, t, None),
            Err(GalatKelola::ClientOwnerTanpaClient)
        ));
        let s = SyaratPemberian {
            target_punya_client: true,
            ..syarat(Peran::ClientOwner, a, t, None)
        };
        assert!(periksa_pemberian(&s, Environment::Dev, t0()).is_ok());
    }

    #[test]
    fn auditor_di_poc_wajib_berlaku_sampai_di_masa_depan() {
        let (a, t, s) = (Uuid::now_v7(), Uuid::now_v7(), Uuid::now_v7());
        let poc = |b| {
            let x = SyaratPemberian {
                berlaku_sampai: b,
                ..syarat(Peran::Auditor, a, t, Some(s))
            };
            periksa_pemberian(&x, Environment::Poc, t0())
        };
        assert!(matches!(poc(None), Err(GalatKelola::BerlakuSampaiWajib)));
        assert!(matches!(
            poc(Some(t0() - chrono::Duration::days(1))),
            Err(GalatKelola::BerlakuSampaiLewat)
        ));
        assert!(poc(Some(t0() + chrono::Duration::days(30))).is_ok());
    }

    #[test]
    fn username_mengikuti_check_tabel() {
        assert!(username_sah("operator-satu"));
        assert!(!username_sah("ab"));
        assert!(!username_sah("Operator"));
        assert!(!username_sah(&"a".repeat(101)));
    }
}
