//! Kodek body pesan untuk apalis (ARC-68).
//!
//! `apalis-amqp` 1.0.0 bertipe `Durable` dan bawaannya mendekode body sebagai
//! JSON. Body PoolRouter adalah teks `request_id` apa adanya (spike S11:
//! body hanya `request_id`), jadi kodek bawaan diganti lewat `WithCodec`
//! supaya format kabel tidak berubah dan publish lapin tetap sederhana.

use std::fmt;

use apalis::prelude::Codec;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct KodekRequestId;

/// Body bukan UUID teks. Pesan hanya disebut panjangnya, bukan isinya.
#[derive(Debug)]
pub(crate) struct GalatKodek(usize);

impl fmt::Display for GalatKodek {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "body pesan bukan request_id ({} byte)", self.0)
    }
}

impl std::error::Error for GalatKodek {}

impl Codec<Uuid> for KodekRequestId {
    type Error = GalatKodek;
    type Compact = Vec<u8>;

    fn encode(&self, val: &Uuid) -> Result<Vec<u8>, GalatKodek> {
        Ok(body(*val))
    }

    fn decode(&self, val: &Vec<u8>) -> Result<Uuid, GalatKodek> {
        let galat = GalatKodek(val.len());
        std::str::from_utf8(val)
            .ok()
            .and_then(|teks| Uuid::parse_str(teks).ok())
            .ok_or(galat)
    }
}

/// Body kabel untuk satu `request_id`: teks UUID berhuruf kecil.
pub(crate) fn body(request_id: Uuid) -> Vec<u8> {
    request_id.to_string().into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn body_bolak_balik_tanpa_json() {
        let id = Uuid::parse_str("0190a000-0000-7000-8000-000000000001").unwrap();
        let b = body(id);
        assert_eq!(b, b"0190a000-0000-7000-8000-000000000001");
        assert_eq!(KodekRequestId.decode(&b).unwrap(), id);
        assert_eq!(KodekRequestId.encode(&id).unwrap(), b);
    }

    #[test]
    fn body_rusak_ditolak_tanpa_mencetak_isi() {
        let galat = KodekRequestId
            .decode(&b"\"bukan-uuid\"".to_vec())
            .unwrap_err();
        assert_eq!(galat.to_string(), "body pesan bukan request_id (12 byte)");
    }
}
