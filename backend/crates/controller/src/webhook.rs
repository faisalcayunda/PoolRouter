//! Job webhook: outbox `request` → HTTP keluar (ARC-73, AC-26..AC-31).
//!
//! Urutannya sengaja tetap dan sama untuk setiap baris: klaim compare-and-set,
//! baca konfigurasi, periksa allowlist, periksa IP hasil resolusi, dekripsi
//! secret, kirim tanpa redirect, lalu satu transaksi menulis `webhook_log` dan
//! status terminal. Pemeriksaan yang lebih murah dan yang **mencegah** koneksi
//! keluar berjalan sebelum dekripsi, jadi host terlarang tidak pernah membuat
//! ciphertext dibuka.
//!
//! Resolver dan jam masuk sebagai parameter (seam CW10) supaya uji bisa
//! mengembalikan IP terlarang tanpa menyentuh DNS sungguhan, dan bisa
//! memeriksa umur klaim tanpa menunggu lima menit.

use std::future::Future;
use std::net::{IpAddr, SocketAddr};
use std::pin::Pin;
use std::time::Duration;

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use chrono::{DateTime, Utc};
use poolrouter_core::secret::{Amplop, Brankas};
use poolrouter_core::status_api::{StatusInternal, StatusTerpetakan, status_api};
use sqlx::PgPool;
use uuid::Uuid;

use crate::jadwal::{KotakFutur, Pekerjaan};

/// Umur klaim `menunggu` sebelum ditutup `gagal` tanpa kirim ulang (AC-26).
pub const BATAS_KLAIM_DETIK: i64 = 300;
/// Baris per putaran; cukup kecil supaya satu putaran tidak menahan leader lama.
const BATAS_PER_PUTARAN: i64 = 20;
const BATAS_CONNECT: Duration = Duration::from_secs(5);
const BATAS_TOTAL: Duration = Duration::from_secs(10);
/// Prefix secret Standard Webhooks (§5a.6).
pub const PREFIX_SECRET: &str = "whsec_";

/// AAD amplop secret webhook. ARC-50 memakai fungsi ini, bukan menebak string.
pub fn nama_amplop(client_id: Uuid) -> String {
    format!("WEBHOOK_SECRET:{client_id}")
}

/// Resolver DNS yang bisa diganti di uji (CW10 seam).
pub trait Resolver: Send + Sync {
    fn selesaikan<'a>(
        &'a self,
        host: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<IpAddr>, String>> + Send + 'a>>;
}

/// Resolver produksi: DNS sistem, port tidak berpengaruh pada alamat.
pub struct ResolverSistem;

impl Resolver for ResolverSistem {
    fn selesaikan<'a>(
        &'a self,
        host: &'a str,
    ) -> Pin<Box<dyn Future<Output = Result<Vec<IpAddr>, String>> + Send + 'a>> {
        Box::pin(async move {
            let alamat = tokio::net::lookup_host((host, 443))
                .await
                .map_err(|e| e.to_string())?;
            Ok(alamat.map(|a| a.ip()).collect())
        })
    }
}

/// Rentang yang tidak boleh dituju webhook (W-07): loopback, privat,
/// link-local (metadata cloud), unspecified, multicast, broadcast, `0.0.0.0/8`,
/// CGNAT, `192.0.0.0/24`, `198.18.0.0/15`, serta padanan IPv6 termasuk
/// IPv4-mapped.
pub fn ip_terlarang(ip: IpAddr) -> bool {
    let v4 = |a: std::net::Ipv4Addr| {
        let o = a.octets();
        a.is_loopback()
            || a.is_private()
            || a.is_link_local()
            || a.is_unspecified()
            || a.is_broadcast()
            || a.is_multicast()
            || o[0] == 0
            || (o[0] == 100 && (64..=127).contains(&o[1]))
            || (o[0] == 192 && o[1] == 0 && o[2] == 0)
            || (o[0] == 198 && matches!(o[1], 18 | 19))
    };
    match ip {
        IpAddr::V4(a) => v4(a),
        IpAddr::V6(a) => match a.to_ipv4_mapped() {
            Some(mapped) => v4(mapped),
            None => {
                let s = a.segments();
                a.is_loopback()
                    || a.is_unspecified()
                    || a.is_multicast()
                    || (s[0] & 0xfe00) == 0xfc00
                    || (s[0] & 0xffc0) == 0xfe80
            }
        },
    }
}

/// Tanda tangan Standard Webhooks: `v1,base64(HMAC-SHA256(kunci,
/// "{id}.{cap}.{body}"))` (§5a.6). `kunci` adalah secret setelah `whsec_`
/// dibuka dari base64.
pub fn tanda_tangan(kunci: &[u8], id: &str, cap: i64, body: &[u8]) -> String {
    let mut pesan = Vec::with_capacity(id.len() + 24 + body.len());
    pesan.extend_from_slice(id.as_bytes());
    pesan.push(b'.');
    pesan.extend_from_slice(cap.to_string().as_bytes());
    pesan.push(b'.');
    pesan.extend_from_slice(body);
    let kunci_hmac = aws_lc_rs::hmac::Key::new(aws_lc_rs::hmac::HMAC_SHA256, kunci);
    let tag = aws_lc_rs::hmac::sign(&kunci_hmac, &pesan);
    format!("v1,{}", STANDARD.encode(tag.as_ref()))
}

/// Secret tersimpan berformat `whsec_<base64>`; yang dipakai HMAC adalah byte
/// hasil dekode.
pub fn kunci_hmac(secret: &str) -> Result<Vec<u8>, String> {
    let b64 = secret
        .strip_prefix(PREFIX_SECRET)
        .ok_or("secret webhook tidak berprefix whsec_")?;
    STANDARD
        .decode(b64)
        .map_err(|_| "secret webhook bukan base64".to_owned())
}

/// Satu baris `v_request_webhook` yang sudah diklaim.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Baris {
    pub request_id: Uuid,
    /// Hidup di `request` (grant kolom terpisah, W-02), bukan di view.
    pub client_id: Uuid,
    pub status: String,
    pub attempt_count: i32,
    pub cancel_reason: Option<String>,
    pub failure_reason: Option<String>,
    pub output_payload: Option<serde_json::Value>,
    pub webhook_attempt_count: i32,
    pub webhook_not_before: Option<DateTime<Utc>>,
}

/// Konfigurasi webhook milik client.
#[derive(Debug, Clone, sqlx::FromRow)]
struct Konfigurasi {
    webhook_url: String,
    aktif: bool,
    secret_versi: i32,
    secret_ciphertext: Vec<u8>,
    secret_dek_terbungkus: Vec<u8>,
    secret_nonce: Vec<u8>,
    kek_id: String,
}

/// Body terminal: bentuk `RequestStatus` di `downstream-api.yaml`, dibangun
/// lewat pemetaan status R1 T1. Body tidak disimpan (AC-27).
pub fn body(baris: &Baris) -> Result<Vec<u8>, String> {
    let internal: StatusInternal = baris
        .status
        .parse()
        .map_err(|_| format!("status tak dikenal: {}", baris.status))?;
    let StatusTerpetakan::Status(api) = status_api(internal, baris.attempt_count) else {
        return Err(format!("status {} tidak bisa dipolling", baris.status));
    };
    let mut objek = serde_json::json!({"queue_id": baris.request_id, "status": api});
    match api {
        poolrouter_core::status_api::StatusApi::Done => {
            objek["hasil"] = baris
                .output_payload
                .clone()
                .unwrap_or_else(|| serde_json::json!({}));
        }
        poolrouter_core::status_api::StatusApi::Failed => {
            if let Some(alasan) = &baris.failure_reason {
                objek["failure_reason"] = serde_json::json!(alasan);
            }
        }
        poolrouter_core::status_api::StatusApi::Cancelled => {
            if let Some(alasan) = &baris.cancel_reason {
                objek["cancel_reason"] = serde_json::json!(alasan);
            }
        }
        _ => {}
    }
    serde_json::to_vec(&objek).map_err(|e| e.to_string())
}

/// Hasil satu kiriman, sebelum ditulis ke `webhook_log`.
struct HasilKirim {
    hasil: &'static str,
    error_class: Option<&'static str>,
    http_status: Option<i32>,
    secret_versi: Option<i32>,
    status_akhir: &'static str,
    responded_at: Option<DateTime<Utc>>,
    duration_ms: i32,
}

impl HasilKirim {
    fn gagal(hasil: &'static str, error_class: &'static str, secret_versi: Option<i32>) -> Self {
        // `ditolak_allowlist` wajib tanpa `secret_versi`; `gagal_internal` boleh
        // tanpa versi. Constraint `chk_secret_versi` menegakkan kombinasinya.
        let versi = if hasil == "ditolak_allowlist" {
            None
        } else {
            secret_versi
        };
        Self {
            hasil,
            error_class: Some(error_class),
            http_status: None,
            secret_versi: versi,
            status_akhir: "gagal",
            responded_at: None,
            duration_ms: 0,
        }
    }

    /// AC-31: client tanpa konfigurasi atau dengan webhook nonaktif. Tidak ada
    /// percobaan, jadi tidak ada baris `webhook_log`.
    fn tidak_dikirim() -> Self {
        Self {
            hasil: "tidak_dikirim",
            error_class: None,
            http_status: None,
            secret_versi: None,
            status_akhir: "tidak_dikirim",
            responded_at: None,
            duration_ms: 0,
        }
    }
}

/// Job webhook sebagai [`Pekerjaan`] untuk scheduler controller.
pub struct Webhook {
    pool: PgPool,
    brankas: Brankas,
    resolver: Box<dyn Resolver>,
}

impl Webhook {
    pub fn baru(pool: PgPool, brankas: Brankas) -> Self {
        Self {
            pool,
            brankas,
            resolver: Box::new(ResolverSistem),
        }
    }

    /// Varian yang dipakai uji untuk mengganti resolver (CW10 seam).
    pub fn dengan_resolver(pool: PgPool, brankas: Brankas, resolver: Box<dyn Resolver>) -> Self {
        Self {
            pool,
            brankas,
            resolver,
        }
    }
}

impl Pekerjaan for Webhook {
    fn nama(&self) -> &'static str {
        "webhook"
    }

    fn jalankan(&self, sekarang: DateTime<Utc>) -> KotakFutur<'_, Result<bool, String>> {
        Box::pin(async move {
            jalankan_putaran(&self.pool, &self.brankas, self.resolver.as_ref(), sekarang).await
        })
    }
}

/// Satu putaran job: tutup klaim macet, lalu kirim baris `menunggu` yang belum
/// diklaim. Mengembalikan `true` bila ada baris yang disentuh.
pub async fn jalankan_putaran(
    pool: &PgPool,
    brankas: &Brankas,
    resolver: &dyn Resolver,
    sekarang: DateTime<Utc>,
) -> Result<bool, String> {
    let macet = tutup_macet(pool, sekarang).await?;
    let kandidat = sqlx::query_as::<_, Baris>(
        "SELECT v.request_id, r.client_id, v.status, v.attempt_count, v.cancel_reason, \
                v.failure_reason, v.output_payload, v.webhook_attempt_count, v.webhook_not_before \
           FROM v_request_webhook v JOIN request r ON r.request_id = v.request_id \
          WHERE v.webhook_status = 'menunggu' \
            AND (v.webhook_not_before IS NULL OR v.webhook_not_before <= $2) \
          ORDER BY v.completed_at \
          LIMIT $1",
    )
    .bind(BATAS_PER_PUTARAN)
    .bind(sekarang)
    .fetch_all(pool)
    .await
    .map_err(|e| e.to_string())?;

    let mut jumlah = macet > 0;
    for baris in kandidat {
        if klaim(pool, baris.request_id, sekarang, baris.webhook_attempt_count).await? {
            kirim(pool, brankas, resolver, sekarang, &baris).await?;
            jumlah = true;
        }
    }
    Ok(jumlah)
}

/// AC-26: klaim CAS 0 → 1. Dua job bersamaan hanya satu yang menang.
///
/// Lewat `fn_klaim_webhook` (SECURITY DEFINER, dipasang migration grant ARC-73):
/// trigger BEFORE UPDATE pada `request` membaca kolom yang sengaja tidak
/// diberikan ke `pr_webhook` (W-02), jadi UPDATE langsung akan ditolak di dalam
/// trigger meski grant kolomnya ada. Baris yang klaimnya sudah diambil job lain
/// menghasilkan 0 baris → `false`, bukan error yang menghentikan putaran.
async fn klaim(pool: &PgPool, request_id: Uuid, sekarang: DateTime<Utc>, diharapkan: i32) -> Result<bool, String> {
    let jumlah =
        sqlx::query_scalar::<_, i32>("SELECT fn_klaim_webhook($1, $2, NULL::integer, NULL::text, $3::integer)")
            .bind(request_id)
            .bind(sekarang)
            .bind(diharapkan)
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;
    Ok(jumlah == 1)
}

/// AC-26: klaim `menunggu` yang lebih tua dari lima menit ditutup `gagal`
/// tanpa kirim ulang, lewat jalur yang sama dengan [`klaim`].
async fn tutup_macet(pool: &PgPool, sekarang: DateTime<Utc>) -> Result<u64, String> {
    let ditutup =
        sqlx::query_scalar::<_, i64>("SELECT fn_tutup_klaim_macet($1::timestamptz, $2::integer)")
            .bind(sekarang)
            .bind(BATAS_KLAIM_DETIK)
            .fetch_one(pool)
            .await
            .map_err(|e| e.to_string())?;
    if ditutup > 0 {
        tracing::warn!(jumlah = ditutup, "klaim webhook macet ditutup gagal");
    }
    Ok(u64::try_from(ditutup).unwrap_or(u64::MAX))
}

async fn kirim(
    pool: &PgPool,
    brankas: &Brankas,
    resolver: &dyn Resolver,
    sekarang: DateTime<Utc>,
    baris: &Baris,
) -> Result<(), String> {
    let mut hasil = kirim_ke_jaringan(pool, brankas, resolver, sekarang, baris).await;
    if let Err(pesan) = &hasil {
        tracing::error!(request_id = %baris.request_id, galat = %pesan, "webhook gagal internal");
        hasil = Ok(HasilKirim::gagal("gagal_internal", "gagal_internal", None));
    }
    catat(pool, sekarang, baris, hasil?).await
}

async fn kirim_ke_jaringan(
    pool: &PgPool,
    brankas: &Brankas,
    resolver: &dyn Resolver,
    sekarang: DateTime<Utc>,
    baris: &Baris,
) -> Result<HasilKirim, String> {
    let konf = sqlx::query_as::<_, Konfigurasi>(
        "SELECT webhook_url, aktif, secret_versi, secret_ciphertext, \
                secret_dek_terbungkus, secret_nonce, kek_id \
           FROM client_webhook WHERE client_id = $1",
    )
    .bind(baris.client_id)
    .fetch_optional(pool)
    .await
    .map_err(|e| e.to_string())?;

    // Tanpa konfigurasi atau dengan webhook nonaktif tidak ada kiriman dan
    // tidak ada baris log: tidak ada percobaan yang bisa dicatat.
    let Some(konf) = konf else {
        return Ok(HasilKirim::tidak_dikirim());
    };
    if !konf.aktif {
        return Ok(HasilKirim::tidak_dikirim());
    }

    let url =
        reqwest::Url::parse(&konf.webhook_url).map_err(|_| "URL webhook tidak valid".to_owned())?;
    let host = url.host_str().ok_or("URL webhook tanpa host")?.to_owned();

    // AC-29: allowlist diperiksa saat kirim, bukan saat konfigurasi disimpan.
    let diizinkan = sqlx::query_scalar::<_, bool>(
        "SELECT EXISTS (SELECT 1 FROM webhook_domain_allowlist WHERE domain = $1 AND aktif)",
    )
    .bind(&host)
    .fetch_one(pool)
    .await
    .map_err(|e| e.to_string())?;
    if !diizinkan {
        return Ok(HasilKirim::gagal(
            "ditolak_allowlist",
            "domain_tidak_diizinkan",
            None,
        ));
    }

    let alamat = resolver.selesaikan(&host).await?;
    if alamat.is_empty() || alamat.iter().any(|ip| ip_terlarang(*ip)) {
        return Ok(HasilKirim::gagal("ditolak_allowlist", "ip_terlarang", None));
    }

    let amplop = Amplop {
        nama: nama_amplop(baris.client_id),
        ciphertext: konf.secret_ciphertext.clone(),
        dek_terbungkus: konf.secret_dek_terbungkus.clone(),
        nonce: konf
            .secret_nonce
            .as_slice()
            .try_into()
            .map_err(|_| "nonce secret webhook tidak 12 byte".to_owned())?,
        kek_id: konf.kek_id.clone(),
    };
    let kunci = match brankas.dekripsi(&amplop).map_err(|e| e.to_string()) {
        Ok(nilai) => kunci_hmac(nilai.expose())?,
        Err(_) => {
            return Ok(HasilKirim::gagal(
                "gagal_internal",
                "dekripsi_gagal",
                Some(konf.secret_versi),
            ));
        }
    };

    let body = body(baris)?;
    let id = baris.request_id.to_string();
    let cap = sekarang.timestamp();
    let tanda = tanda_tangan(&kunci, &id, cap, &body);

    let alamat: Vec<SocketAddr> = alamat
        .into_iter()
        .map(|ip| SocketAddr::new(ip, 443))
        .collect();
    let klien = reqwest::Client::builder()
        .https_only(true)
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(BATAS_CONNECT)
        .timeout(BATAS_TOTAL)
        .resolve_to_addrs(&host, &alamat)
        .build()
        .map_err(|e| e.to_string())?;

    let mulai = std::time::Instant::now();
    let jawab = klien
        .post(url)
        .header("content-type", "application/json")
        .header("webhook-id", &id)
        .header("webhook-timestamp", cap.to_string())
        .header("webhook-signature", tanda)
        // AC-28: versi secret yang benar-benar dipakai kiriman ini, dibaca
        // saat kirim (CW9) sehingga rotasi berlaku untuk pengiriman berikutnya.
        .header("poolrouter-secret-versi", konf.secret_versi.to_string())
        .body(body)
        .send()
        .await;
    let durasi = i32::try_from(mulai.elapsed().as_millis()).unwrap_or(i32::MAX);

    match jawab {
        Ok(respons) => {
            let kode = respons.status().as_u16();
            let (hasil, kelas, akhir) = klasifikasi(kode);
            Ok(HasilKirim {
                hasil,
                error_class: kelas,
                http_status: Some(i32::from(kode)),
                secret_versi: Some(konf.secret_versi),
                status_akhir: akhir,
                responded_at: Some(sekarang),
                duration_ms: durasi,
            })
        }
        Err(_) => Ok(HasilKirim {
            hasil: "gagal_jaringan",
            error_class: Some("gagal_jaringan"),
            http_status: None,
            secret_versi: Some(konf.secret_versi),
            status_akhir: "gagal",
            responded_at: None,
            duration_ms: durasi,
        }),
    }
}

/// Pemetaan kode HTTP ke kolom `webhook_log` (AC-29). Redirect tidak pernah
/// diikuti: `Policy::none` membuat 3xx sampai ke sini sebagai respons biasa.
pub fn klasifikasi(kode: u16) -> (&'static str, Option<&'static str>, &'static str) {
    match kode {
        200..=299 => ("terkirim", None, "terkirim"),
        300..=399 => ("gagal_http", Some("redirect_ditolak"), "gagal"),
        _ => ("gagal_http", Some("http_bukan_2xx"), "gagal"),
    }
}

async fn catat(
    pool: &PgPool,
    sekarang: DateTime<Utc>,
    baris: &Baris,
    hasil: HasilKirim,
) -> Result<(), String> {
    let mut tx = pool.begin().await.map_err(|e| e.to_string())?;
    if hasil.hasil != "tidak_dikirim" {
        sqlx::query(
            "INSERT INTO webhook_log (webhook_delivery_id, request_id, client_id, attempt_no, \
                    status_api, sent_at, responded_at, http_status, hasil, error_class, \
                    duration_ms, secret_versi) \
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)",
        )
        .bind(Uuid::now_v7())
        .bind(baris.request_id)
        .bind(baris.client_id)
        .bind(baris.webhook_attempt_count + 1)
        .bind(status_api_terminal(&baris.status))
        .bind(sekarang)
        .bind(hasil.responded_at)
        .bind(hasil.http_status)
        .bind(hasil.hasil)
        .bind(hasil.error_class)
        .bind(hasil.duration_ms)
        .bind(hasil.secret_versi)
        .execute(&mut *tx)
        .await
        .map_err(|e| e.to_string())?;
    }
    sqlx::query(
        "UPDATE request SET webhook_status = $2, webhook_last_at = $3, \
                webhook_last_http_status = $4, webhook_last_error = $5 \
          WHERE request_id = $1",
    )
    .bind(baris.request_id)
    .bind(hasil.status_akhir)
    .bind(sekarang)
    .bind(hasil.http_status)
    .bind(hasil.error_class)
    .execute(&mut *tx)
    .await
    .map_err(|e| e.to_string())?;
    tx.commit().await.map_err(|e| e.to_string())
}

fn status_api_terminal(internal: &str) -> &str {
    match internal {
        "done" => "done",
        "failed" => "failed",
        "expired" => "expired",
        _ => "cancelled",
    }
}

#[cfg(test)]
mod tes {
    use super::*;
    use std::net::Ipv4Addr;

    /// Rentang W-07 yang dilarang (AC-29).
    #[test]
    fn ac29_ip_terlarang() {
        let dilarang = [
            IpAddr::V4(Ipv4Addr::LOCALHOST),
            IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)),
            IpAddr::V4(Ipv4Addr::new(10, 1, 2, 3)),
            IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254)),
            IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1)),
            IpAddr::V4(Ipv4Addr::new(100, 64, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(192, 0, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(198, 19, 0, 1)),
            IpAddr::V4(Ipv4Addr::new(255, 255, 255, 255)),
            IpAddr::V6("::".parse().unwrap()),
            IpAddr::V6("::1".parse().unwrap()),
            IpAddr::V6("fd00::1".parse().unwrap()),
            // IPv4-mapped loopback dan metadata harus ikut ditolak.
            IpAddr::V6("::ffff:127.0.0.1".parse().unwrap()),
            IpAddr::V6("::ffff:169.254.169.254".parse().unwrap()),
        ];
        for ip in dilarang {
            assert!(ip_terlarang(ip), "{ip} seharusnya dilarang");
        }
        assert!(!ip_terlarang(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 10))));
        assert!(!ip_terlarang(IpAddr::V6("2001:db8::1".parse().unwrap())));
    }

    /// AC-28: tanda tangan dan body diverifikasi pustaka Standard Webhooks
    /// pihak ketiga (`standardwebhooks`, oleh Svix).
    #[test]
    fn ac28_diverifikasi_pustaka_pihak_ketiga() {
        let rahasia = format!("{}dGVzdC1zZWNyZXQta2V5", PREFIX_SECRET);
        let kunci = kunci_hmac(&rahasia).expect("secret uji valid");
        let baris = Baris {
            request_id: Uuid::now_v7(),
            client_id: Uuid::now_v7(),
            status: "done".to_string(),
            attempt_count: 1,
            cancel_reason: None,
            failure_reason: None,
            output_payload: Some(serde_json::json!({"data": {"nik": "3273"}})),
            webhook_attempt_count: 0,
        };
        let body = body(&baris).expect("body");
        let id = baris.request_id.to_string();
        let cap = Utc::now().timestamp();
        let ttd = tanda_tangan(&kunci, &id, cap, &body);

        let mut header = reqwest::header::HeaderMap::new();
        header.insert("webhook-id", id.parse().unwrap());
        header.insert("webhook-timestamp", cap.to_string().parse().unwrap());
        header.insert("webhook-signature", ttd.parse().unwrap());

        let penguji = standardwebhooks::Webhook::new(&rahasia).expect("pustaka menerima secret");
        penguji
            .verify(&body, &header)
            .expect("tanda tangan harus valid menurut pustaka pihak ketiga");

        // Body yang diubah tidak boleh lolos.
        let mut rusak = body.clone();
        rusak[1] = b'X';
        assert!(penguji.verify(&rusak, &header).is_err());
    }

    /// AC-27: body terminal memakai pemetaan status R1 T1 dan bentuk
    /// `RequestStatus` pada `downstream-api.yaml`.
    #[test]
    fn ac27_body_terminal() {
        let dasar = Baris {
            request_id: Uuid::now_v7(),
            client_id: Uuid::now_v7(),
            status: "done".to_string(),
            attempt_count: 1,
            cancel_reason: None,
            failure_reason: None,
            output_payload: Some(serde_json::json!({"data": {}})),
            webhook_attempt_count: 0,
        };
        let parse = |b: Baris| {
            serde_json::from_slice::<serde_json::Value>(&body(&b).expect("body")).expect("json")
        };

        let done = parse(dasar.clone());
        assert_eq!(done["status"], "done");
        assert!(done["queue_id"].is_string());
        assert!(done["hasil"].is_object());
        assert!(done.get("failure_reason").is_none());
        assert!(done.get("cancel_reason").is_none());

        let failed = parse(Baris {
            status: "failed".to_string(),
            failure_reason: Some("du_capil_timeout".to_string()),
            ..dasar.clone()
        });
        assert_eq!(failed["status"], "failed");
        assert_eq!(failed["failure_reason"], "du_capil_timeout");

        let cancel = parse(Baris {
            status: "cancelled".to_string(),
            cancel_reason: Some("batch_cancel".to_string()),
            ..dasar.clone()
        });
        assert_eq!(cancel["status"], "cancelled");
        assert_eq!(cancel["cancel_reason"], "batch_cancel");

        let expired = parse(Baris {
            status: "expired".to_string(),
            ..dasar.clone()
        });
        assert_eq!(expired["status"], "expired");
        assert!(expired.get("hasil").is_none());
    }

    /// AC-27: kunci dan nilai status yang dipakai body ada di skema OpenAPI,
    /// supaya body tidak melenceng dari kontrak yang dipublikasikan.
    #[test]
    fn ac27_skema_openapi_cocok() {
        let isi = include_str!("../../../openapi/downstream-api.yaml");
        let doc: serde_json::Value = yaml_serde::from_str(isi).expect("yaml");
        let rs = &doc["components"]["schemas"]["RequestStatus"];
        let wajib: Vec<&str> = rs["required"]
            .as_array()
            .expect("required")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        assert_eq!(wajib, ["queue_id", "status"]);

        let status = &doc["components"]["schemas"]["ApiStatus"];
        let nilai: Vec<&str> = status["enum"]
            .as_array()
            .expect("enum ApiStatus")
            .iter()
            .filter_map(|v| v.as_str())
            .collect();
        for dipakai in [
            "queued",
            "processing",
            "done",
            "failed",
            "expired",
            "cancelled",
        ] {
            assert!(nilai.contains(&dipakai), "{dipakai} harus ada di ApiStatus");
        }
    }

    /// AC-29/AC-30: pemetaan respons HTTP.
    #[test]
    fn ac29_klasifikasi_http() {
        assert_eq!(klasifikasi(200), ("terkirim", None, "terkirim"));
        assert_eq!(klasifikasi(204), ("terkirim", None, "terkirim"));
        assert_eq!(
            klasifikasi(302),
            ("gagal_http", Some("redirect_ditolak"), "gagal")
        );
        assert_eq!(
            klasifikasi(404),
            ("gagal_http", Some("http_bukan_2xx"), "gagal")
        );
        assert_eq!(
            klasifikasi(503),
            ("gagal_http", Some("http_bukan_2xx"), "gagal")
        );
    }

    /// Secret webhook wajib berprefix dan berbentuk base64 (W-06).
    #[test]
    fn kunci_hmac_menolak_bentuk_salah() {
        assert!(kunci_hmac("dGVzdA==").is_err());
        assert!(kunci_hmac("whsec_bukan base64!").is_err());
        assert_eq!(kunci_hmac("whsec_dGVzdA==").expect("valid"), b"test");
    }
}
