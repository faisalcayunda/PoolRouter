//! RFC 6238 TOTP over HMAC-SHA1, no TOTP crate (spec MF-3, AC-4).
//!
//! Secret is a base32 string (what an authenticator app carries). The window is
//! ±1 step of 30 s; the matched step is returned so the caller can enforce the
//! anti-replay monotonicity.

use chrono::{DateTime, Utc};

use crate::admin::password::sama_konstan;

/// Step in seconds. Fixed by ADR-0006 §9 for authenticator compatibility.
pub const LANGKAH_DETIK: i64 = 30;

/// Half-width of the accepted window, in steps.
pub const TOLERANSI_LANGKAH: i64 = 1;

const ABJAD: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

/// Encode raw bytes as RFC 4648 base32, uppercase, no padding.
#[must_use]
pub fn encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(5) * 8);
    let (mut buf, mut bits) = (0u32, 0u32);
    for &b in data {
        buf = (buf << 8) | u32::from(b);
        bits += 8;
        while bits >= 5 {
            out.push(ABJAD[((buf >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        out.push(ABJAD[((buf << (5 - bits)) & 31) as usize] as char);
    }
    out
}

/// Decode base32. Lowercase is accepted; any other character is invalid.
pub fn urai(teks: &str) -> Result<Vec<u8>, super::GalatMfa> {
    let mut out = Vec::with_capacity(teks.len() * 5 / 8);
    let (mut buf, mut bits) = (0u32, 0u32);
    for c in teks.bytes() {
        let v = match c.to_ascii_uppercase() {
            b'A'..=b'Z' => u32::from(c.to_ascii_uppercase() - b'A'),
            b'2'..=b'7' => u32::from(c - b'2') + 26,
            _ => return Err(super::GalatMfa::DataRusak),
        };
        buf = (buf << 5) | v;
        bits += 5;
        if bits >= 8 {
            out.push((buf >> (bits - 8)) as u8);
            bits -= 8;
        }
    }
    Ok(out)
}

/// One TOTP value for a step, zero-padded to six digits.
#[must_use]
pub fn hitung(secret_base32: &str, langkah: u64) -> String {
    let secret = urai(secret_base32).expect("secret TOTP harus base32");
    let kunci = aws_lc_rs::hmac::Key::new(aws_lc_rs::hmac::HMAC_SHA1_FOR_LEGACY_USE_ONLY, &secret);
    let tag = aws_lc_rs::hmac::sign(&kunci, &langkah.to_be_bytes());
    let h = tag.as_ref();
    let offset = usize::from(h[19] & 0x0f);
    let bin = ((u32::from(h[offset]) & 0x7f) << 24)
        | (u32::from(h[offset + 1]) << 16)
        | (u32::from(h[offset + 2]) << 8)
        | u32::from(h[offset + 3]);
    format!("{:06}", bin % 1_000_000)
}

/// Accept a code in the current ±1 window and return the matched step.
#[must_use]
pub fn cocok(secret_base32: &str, kode: &str, sekarang: DateTime<Utc>) -> Option<u64> {
    if kode.len() != 6 || !kode.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let sekarang = sekarang.timestamp() / LANGKAH_DETIK;
    for delta in 0..=2 * TOLERANSI_LANGKAH {
        let l = sekarang - TOLERANSI_LANGKAH + delta;
        if l < 0 {
            continue;
        }
        if sama_konstan(hitung(secret_base32, l as u64).as_bytes(), kode.as_bytes()) {
            return Some(l as u64);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn saat(detik: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(detik, 0).unwrap()
    }

    #[test]
    fn base32_bolak_balik() {
        let data = b"12345678901234567890";
        let teks = encode(data);
        assert_eq!(teks, "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ");
        assert_eq!(urai(&teks).unwrap(), data);
        assert_eq!(urai(&teks.to_ascii_lowercase()).unwrap(), data);
        assert!(urai("GEZD-GNBV").is_err());
    }

    #[test]
    fn rfc6238_lampiran_b_sha1() {
        // RFC 6238 Appendix B key "12345678901234567890", values truncated to 6.
        let secret = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
        for (detik, harap) in [
            (59, "287082"),
            (1111111109, "081804"),
            (1111111111, "050471"),
            (1234567890, "005924"),
            (2000000000, "279037"),
            (20000000000, "353130"),
        ] {
            assert_eq!(
                hitung(secret, (detik / LANGKAH_DETIK) as u64),
                harap,
                "detik {detik}"
            );
        }
    }

    #[test]
    fn toleransi_min_satu_nol_plus_satu() {
        let secret = "GEZDGNBVGY3TQOJQGEZDGNBVGY3TQOJQ";
        let acuan = saat(120);
        let kode = hitung(secret, 4);
        assert_eq!(
            cocok(secret, &kode, acuan - chrono::Duration::seconds(30)),
            Some(4)
        );
        assert_eq!(cocok(secret, &kode, acuan), Some(4));
        assert_eq!(
            cocok(secret, &kode, acuan + chrono::Duration::seconds(30)),
            Some(4)
        );
        assert_eq!(
            cocok(secret, &kode, acuan + chrono::Duration::seconds(60)),
            None
        );
        assert_eq!(
            cocok(secret, &kode, acuan - chrono::Duration::seconds(60)),
            None
        );
    }

    #[test]
    fn kode_bukan_enam_digit_ditolak() {
        let secret = "AAAAAAAAAAAAAAAA"; // test fixture only, low entropy
        assert_eq!(cocok(secret, "12345", saat(120)), None);
        assert_eq!(cocok(secret, "abcdef", saat(120)), None);
    }

    #[test]
    fn toleransi_ditolak_minus_2_plus_2() {
        let secret = "AAAAAAAAAAAAAAAA"; // test fixture only, low entropy
        let acuan = saat(120); // step 4
        assert_eq!(cocok(secret, &hitung(secret, 2), acuan), None);
        assert_eq!(cocok(secret, &hitung(secret, 6), acuan), None);
    }
}
