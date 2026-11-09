//! Pintu kuota webserver: kuota intake, limiter penolakan, token sync, counter
//! percobaan, dan pelepasan slot, lewat [`KuotaRedis`] (ADR-0001 rev 15 §5,
//! §5a.1, §5a.2).
//!
//! `KuotaStore` memakai `impl Future`, jadi tidak bisa menjadi trait object.
//! [`GerbangKuota`] adalah seam yang bisa: handler memegang
//! `Arc<dyn GerbangKuota>`, produksi memakai [`KuotaRedisMalas`], uji router
//! memakai [`fake::FakeKuota`].
//!
//! Fail-closed: galat transport, `belum_siap`, argumen ditolak skrip, dan
//! perintah yang melewati batas 250 ms (§5a.4) semuanya menjadi
//! [`RedisTidakSiap`]. Tidak ada jalur yang mengubahnya menjadi "kuota ada".

use std::sync::{Arc, Mutex};
use std::time::Duration;

use poolrouter_core::kuota::metrik::Komponen;
use poolrouter_core::kuota::store::{
    AsalLepas, Bacaan, HasilIntake, HasilPenolakan, HasilTambah, HasilToken, KonteksKuota,
    KuotaRedis, KuotaStore, PermintaanToken,
};
use redis::aio::ConnectionManager;

use super::downstream_sesi::Fut;

/// Batas satu perintah Redis (§5a.4). Lewat batas = Redis tidak tersedia.
pub const BATAS_REDIS: Duration = Duration::from_millis(250);

/// Redis tidak bisa memberi keputusan. Selalu 503 `layanan_tidak_tersedia`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RedisTidakSiap;

pub type HasilKuota<T> = Result<T, RedisTidakSiap>;

pub trait GerbangKuota: Send + Sync + 'static {
    fn intake(
        &self,
        ctx: KonteksKuota,
        sync: bool,
        client_id: String,
    ) -> Fut<'_, HasilKuota<HasilIntake>>;

    fn penolakan(
        &self,
        client_id: String,
        isi_ulang_mikro: i64,
        burst: i64,
    ) -> Fut<'_, HasilKuota<HasilPenolakan>>;

    fn token(&self, ctx: KonteksKuota, p: PermintaanToken) -> Fut<'_, HasilKuota<HasilToken>>;

    fn tambah(
        &self,
        ctx: KonteksKuota,
        request_id: String,
        nomor: i64,
    ) -> Fut<'_, HasilKuota<HasilTambah>>;

    fn lepas(&self, pool_id: String, request_id: String, nomor: i64) -> Fut<'_, HasilKuota<bool>>;

    fn baca(&self, ctx: KonteksKuota) -> Fut<'_, HasilKuota<Bacaan>>;
}

/// [`KuotaRedis`] yang dibuat saat dipakai pertama kali. Redis yang mati saat
/// webserver start tidak menggagalkan proses; request menjawab 503 sampai
/// koneksi dan `run_id` terbaca.
pub struct KuotaRedisMalas {
    client: redis::Client,
    isi: Mutex<Option<KuotaRedis<ConnectionManager>>>,
}

impl KuotaRedisMalas {
    pub fn baru(client: redis::Client) -> Arc<Self> {
        Arc::new(Self {
            client,
            isi: Mutex::new(None),
        })
    }

    async fn store(&self) -> HasilKuota<KuotaRedis<ConnectionManager>> {
        if let Some(s) = self.isi.lock().ok().and_then(|g| g.clone()) {
            return Ok(s);
        }
        let buat = async {
            let cm = ConnectionManager::new(self.client.clone())
                .await
                .map_err(|_| RedisTidakSiap)?;
            KuotaRedis::baru(cm)
                .await
                .map(|s| s.dengan_komponen(Komponen::Webserver))
                .map_err(|_| RedisTidakSiap)
        };
        let s = tokio::time::timeout(BATAS_REDIS * 4, buat)
            .await
            .map_err(|_| RedisTidakSiap)??;
        if let Ok(mut g) = self.isi.lock() {
            *g = Some(s.clone());
        }
        Ok(s)
    }
}

/// Satu perintah dengan batas waktu; galat apa pun menjadi [`RedisTidakSiap`].
async fn dengan_batas<T, E>(
    f: impl std::future::Future<Output = Result<T, E>>,
) -> HasilKuota<T> {
    match tokio::time::timeout(BATAS_REDIS, f).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(_)) | Err(_) => Err(RedisTidakSiap),
    }
}

impl GerbangKuota for KuotaRedisMalas {
    fn intake(
        &self,
        ctx: KonteksKuota,
        sync: bool,
        client_id: String,
    ) -> Fut<'_, HasilKuota<HasilIntake>> {
        Box::pin(async move {
            let s = self.store().await?;
            dengan_batas(s.intake(&ctx, sync, &client_id)).await
        })
    }

    fn penolakan(
        &self,
        client_id: String,
        isi_ulang_mikro: i64,
        burst: i64,
    ) -> Fut<'_, HasilKuota<HasilPenolakan>> {
        Box::pin(async move {
            let s = self.store().await?;
            dengan_batas(s.ambil_token_penolakan(&client_id, isi_ulang_mikro, burst)).await
        })
    }

    fn token(&self, ctx: KonteksKuota, p: PermintaanToken) -> Fut<'_, HasilKuota<HasilToken>> {
        Box::pin(async move {
            let s = self.store().await?;
            dengan_batas(s.ambil_token(&ctx, &p)).await
        })
    }

    fn tambah(
        &self,
        ctx: KonteksKuota,
        request_id: String,
        nomor: i64,
    ) -> Fut<'_, HasilKuota<HasilTambah>> {
        Box::pin(async move {
            let s = self.store().await?;
            dengan_batas(s.tambah_attempts(&ctx, &request_id, nomor)).await
        })
    }

    fn lepas(&self, pool_id: String, request_id: String, nomor: i64) -> Fut<'_, HasilKuota<bool>> {
        Box::pin(async move {
            let s = self.store().await?;
            dengan_batas(s.lepas_slot(&pool_id, &request_id, nomor, AsalLepas::Kas)).await
        })
    }

    fn baca(&self, ctx: KonteksKuota) -> Fut<'_, HasilKuota<Bacaan>> {
        Box::pin(async move {
            let s = self.store().await?;
            dengan_batas(s.baca(&ctx)).await
        })
    }
}

pub mod fake {
    //! Kuota di memori untuk uji router. Menghitung setiap panggilan supaya uji
    //! bisa membuktikan bahwa replay idempoten dan polling tidak menyentuh
    //! Redis (AC-11, AC-16).

    use poolrouter_core::kuota::store::StatusToken;

    use super::*;

    #[derive(Debug, Default)]
    pub struct Hitungan {
        pub intake: u32,
        pub penolakan: u32,
        pub token: u32,
        pub tambah: u32,
        pub lepas: u32,
    }

    pub struct FakeKuota {
        pub hitung: Mutex<Hitungan>,
        /// Redis mati: semua panggilan [`RedisTidakSiap`].
        pub mati: Mutex<bool>,
        /// Sisa token limiter penolakan; 0 berarti `Dibatasi`.
        pub token_penolakan: Mutex<i64>,
        /// Sisa kuota intake; 0 berarti `KuotaHabis`.
        pub kuota: Mutex<i64>,
        /// Jawaban token sync berikutnya; `None` berarti diberikan.
        pub token_ditolak: Mutex<Option<HasilToken>>,
    }

    impl FakeKuota {
        pub fn baru() -> Arc<Self> {
            Arc::new(Self {
                hitung: Mutex::new(Hitungan::default()),
                mati: Mutex::new(false),
                token_penolakan: Mutex::new(20),
                kuota: Mutex::new(1_000),
                token_ditolak: Mutex::new(None),
            })
        }

        fn cek(&self) -> HasilKuota<()> {
            if *self.mati.lock().unwrap() {
                Err(RedisTidakSiap)
            } else {
                Ok(())
            }
        }
    }

    pub fn status_token(tunggu_mikro: i64) -> StatusToken {
        StatusToken {
            terpakai: 0,
            slot_aktif: 0,
            kedaluwarsa: 0,
            tunggu_mikro,
            isi: 0,
            isi_porsi: 0,
        }
    }

    impl GerbangKuota for FakeKuota {
        fn intake(
            &self,
            _ctx: KonteksKuota,
            _sync: bool,
            _client_id: String,
        ) -> Fut<'_, HasilKuota<HasilIntake>> {
            Box::pin(async move {
                self.cek()?;
                self.hitung.lock().unwrap().intake += 1;
                let mut k = self.kuota.lock().unwrap();
                if *k <= 0 {
                    return Ok(HasilIntake::KuotaHabis {
                        terpakai: 0,
                        antre: 0,
                    });
                }
                *k -= 1;
                Ok(HasilIntake::Lolos {
                    terpakai: 0,
                    antre: 0,
                })
            })
        }

        fn penolakan(
            &self,
            _client_id: String,
            _isi_ulang_mikro: i64,
            _burst: i64,
        ) -> Fut<'_, HasilKuota<HasilPenolakan>> {
            Box::pin(async move {
                self.cek()?;
                self.hitung.lock().unwrap().penolakan += 1;
                let mut t = self.token_penolakan.lock().unwrap();
                if *t <= 0 {
                    return Ok(HasilPenolakan::Dibatasi {
                        tunggu_mikro: 1_500_000,
                    });
                }
                *t -= 1;
                Ok(HasilPenolakan::Lolos { sisa_token: *t })
            })
        }

        fn token(&self, _ctx: KonteksKuota, _p: PermintaanToken) -> Fut<'_, HasilKuota<HasilToken>> {
            Box::pin(async move {
                self.cek()?;
                self.hitung.lock().unwrap().token += 1;
                Ok(self
                    .token_ditolak
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or(HasilToken::Diberikan(status_token(0))))
            })
        }

        fn tambah(
            &self,
            _ctx: KonteksKuota,
            _request_id: String,
            _nomor: i64,
        ) -> Fut<'_, HasilKuota<HasilTambah>> {
            Box::pin(async move {
                self.cek()?;
                self.hitung.lock().unwrap().tambah += 1;
                Ok(HasilTambah::Dicatat {
                    attempts: 1,
                    terpakai: 1,
                    kuota: 1_000,
                })
            })
        }

        fn lepas(
            &self,
            _pool_id: String,
            _request_id: String,
            _nomor: i64,
        ) -> Fut<'_, HasilKuota<bool>> {
            Box::pin(async move {
                self.cek()?;
                self.hitung.lock().unwrap().lepas += 1;
                Ok(true)
            })
        }

        fn baca(&self, _ctx: KonteksKuota) -> Fut<'_, HasilKuota<Bacaan>> {
            Box::pin(async move {
                self.cek()?;
                Ok(Bacaan {
                    terpakai: 0,
                    attempts: 0,
                    slot_aktif: 0,
                    retry_after_detik: 42,
                })
            })
        }
    }
}
