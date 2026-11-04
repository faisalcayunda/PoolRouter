//! Login limiter: 5 failures per username in 15 minutes lock that username for
//! 15 minutes (ADR-0003 rev 5 §3.6, "Pembatasan login").
//!
//! The ADR names Redis as the shared store. This is the in-process
//! implementation: correct for one webserver instance, and the clock is a
//! parameter so tests move time instead of sleeping. With several instances
//! each would count separately (up to 5 per instance), so a Redis-backed
//! implementation must replace it before the webserver is scaled out; the
//! handler only depends on these three calls.
//!
//! Keys are the normalised username as typed, known or not, so the limiter
//! cannot be used to tell an existing account from a missing one. The map is
//! bounded: past `BATAS_ENTRI` expired entries are swept, and if it is still
//! full a new username is simply not tracked (fail open for the *unknown* name,
//! never for one already counted).

use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;

use chrono::{DateTime, Duration, Utc};

pub const MAKS_GAGAL: usize = 5;
pub const JENDELA_MENIT: i64 = 15;
const BATAS_ENTRI: usize = 10_000;

#[derive(Default)]
struct Entri {
    gagal: VecDeque<DateTime<Utc>>,
    kunci_sampai: Option<DateTime<Utc>>,
}

#[derive(Default)]
pub struct PembatasMemori {
    peta: Mutex<HashMap<String, Entri>>,
}

impl PembatasMemori {
    pub fn baru() -> Self {
        Self::default()
    }

    /// Seconds until the username may try again, if it is locked at `sekarang`.
    pub fn terkunci(&self, username: &str, sekarang: DateTime<Utc>) -> Option<u64> {
        let mut peta = self.peta.lock().unwrap_or_else(|e| e.into_inner());
        let entri = peta.get_mut(username)?;
        match entri.kunci_sampai {
            Some(sampai) if sampai > sekarang => {
                Some((sampai - sekarang).num_seconds().max(1) as u64)
            }
            Some(_) => {
                entri.kunci_sampai = None;
                None
            }
            None => None,
        }
    }

    /// Count one failed attempt. The fifth inside the window starts the lock.
    pub fn gagal(&self, username: &str, sekarang: DateTime<Utc>) {
        let mut peta = self.peta.lock().unwrap_or_else(|e| e.into_inner());
        if peta.len() >= BATAS_ENTRI && !peta.contains_key(username) {
            peta.retain(|_, e| {
                e.kunci_sampai.is_some_and(|s| s > sekarang)
                    || e.gagal
                        .back()
                        .is_some_and(|t| *t > sekarang - Duration::minutes(JENDELA_MENIT))
            });
            if peta.len() >= BATAS_ENTRI {
                return;
            }
        }
        let entri = peta.entry(username.to_owned()).or_default();
        let batas = sekarang - Duration::minutes(JENDELA_MENIT);
        while entri.gagal.front().is_some_and(|t| *t <= batas) {
            entri.gagal.pop_front();
        }
        entri.gagal.push_back(sekarang);
        if entri.gagal.len() >= MAKS_GAGAL {
            entri.kunci_sampai = Some(sekarang + Duration::minutes(JENDELA_MENIT));
            entri.gagal.clear();
        }
    }

    /// A successful login clears the count.
    pub fn berhasil(&self, username: &str) {
        self.peta
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(username);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t(m: i64) -> DateTime<Utc> {
        DateTime::<Utc>::UNIX_EPOCH + Duration::minutes(m)
    }

    #[test]
    fn lima_kegagalan_mengunci_lima_belas_menit() {
        let p = PembatasMemori::baru();
        for i in 0..4 {
            p.gagal("a", t(i));
            assert!(p.terkunci("a", t(i)).is_none());
        }
        p.gagal("a", t(4));
        assert_eq!(p.terkunci("a", t(4)), Some(15 * 60));
        assert!(p.terkunci("a", t(18)).is_some());
        assert!(p.terkunci("a", t(19)).is_none());
    }

    #[test]
    fn kegagalan_di_luar_jendela_tidak_dihitung() {
        let p = PembatasMemori::baru();
        for i in 0..4 {
            p.gagal("a", t(i));
        }
        p.gagal("a", t(20));
        assert!(p.terkunci("a", t(20)).is_none());
    }

    #[test]
    fn username_lain_tidak_terpengaruh_dan_berhasil_mereset() {
        let p = PembatasMemori::baru();
        for i in 0..5 {
            p.gagal("a", t(i));
        }
        assert!(p.terkunci("b", t(5)).is_none());
        p.berhasil("a");
        assert!(p.terkunci("a", t(5)).is_none());
    }
}
