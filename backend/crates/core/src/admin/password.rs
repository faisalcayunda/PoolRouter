//! Password hashing and policy (ADR-0003 rev 5 section 3.6, R-11).
//!
//! The parameters are a contract, not a tuning knob: argon2id with m = 19 MiB,
//! t = 2, p = 1 is the OWASP minimum for Argon2id as of the ADR, and R-11 sets
//! the minimum length at 15 characters because in F0 the password is the only
//! factor (NIST SP 800-63B-4 section 3.1.1.2). There are no composition rules —
//! no "one upper, one digit" — on purpose: they push people towards `Password1!`.
//!
//! Two properties this module has to guarantee beyond correctness:
//!
//! - the plaintext never lands in an error message, `Debug` output, or a log
//!   line (contract rule 3: errors name the variable, never the value);
//! - a hash comparison is constant-time, so a wrong password cannot be
//!   narrowed down by timing.
//!
//! The *strength* limits live in [`KebijakanPassword`] so a later decision to
//! drop the minimum to 8 once MFA exists (ADR-0003 section 3.6, F1) is a value
//! change and not a rewrite.

use std::fmt;

use argon2::password_hash::phc::PasswordHash;
use argon2::password_hash::{PasswordHasher, PasswordVerifier};
use argon2::{Algorithm, Params, Version};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

/// Minimum characters in F0: the password is the only factor until MFA (F1).
pub const PANJANG_MINIMUM_F0: usize = 15;

/// Maximum accepted length. Not a security limit — argon2id has no meaningful
/// input limit — but an unbounded password lets a caller burn CPU on a login
/// attempt, and 1024 characters is far past any real password.
pub const PANJANG_MAKSIMUM: usize = 1024;

/// Argon2id cost parameters, pinned by the ADR.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParameterArgon2 {
    /// Memory in KiB: 19 MiB.
    pub memori_kib: u32,
    /// Iterations.
    pub iterasi: u32,
    /// Lanes.
    pub paralelisme: u32,
}

impl ParameterArgon2 {
    /// ADR-0003 section 3.6: OWASP minimum for argon2id.
    pub const OWASP: Self = Self {
        memori_kib: 19 * 1024,
        iterasi: 2,
        paralelisme: 1,
    };
}

impl Default for ParameterArgon2 {
    fn default() -> Self {
        Self::OWASP
    }
}

/// Why a password was rejected. Deliberately says nothing about the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PasswordDitolak {
    /// Shorter than [`KebijakanPassword::panjang_minimum`].
    TerlaluPendek { minimum: usize },
    /// Longer than [`PANJANG_MAKSIMUM`].
    TerlaluPanjang { maksimum: usize },
    /// Contains no character other than whitespace.
    Kosong,
    /// On the common-password list (R-11).
    Umum,
}

impl fmt::Display for PasswordDitolak {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TerlaluPendek { minimum } => {
                write!(f, "password minimal {minimum} karakter")
            }
            Self::TerlaluPanjang { maksimum } => {
                write!(f, "password maksimal {maksimum} karakter")
            }
            Self::Kosong => f.write_str("password tidak boleh kosong"),
            Self::Umum => f.write_str("password terlalu umum, pilih yang lain"),
        }
    }
}

impl std::error::Error for PasswordDitolak {}

/// Hashing failed for a reason that is not the caller's fault (bad salt, or
/// parameters the `argon2` crate refuses). Never carries the password.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GalatHash;

impl fmt::Display for GalatHash {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("password tidak dapat di-hash")
    }
}

impl std::error::Error for GalatHash {}

/// The password policy in force. F0 uses [`Self::f0`]; the constructor exists
/// so F1 can lower the minimum without touching call sites.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KebijakanPassword {
    pub panjang_minimum: usize,
    pub periksa_daftar_umum: bool,
}

impl KebijakanPassword {
    /// F0: 15 characters minimum and the common-password check (R-11).
    pub const fn f0() -> Self {
        Self {
            panjang_minimum: PANJANG_MINIMUM_F0,
            periksa_daftar_umum: true,
        }
    }

    /// Validate a candidate password without hashing it. The CLI uses this for
    /// a fast, friendly error before prompting for confirmation.
    pub fn periksa(&self, password: &str) -> Result<(), PasswordDitolak> {
        if password.trim().is_empty() {
            return Err(PasswordDitolak::Kosong);
        }
        // Counted in characters, not bytes: a passphrase in Indonesian is not
        // penalised for containing a non-ASCII character.
        let panjang = password.chars().count();
        if panjang < self.panjang_minimum {
            return Err(PasswordDitolak::TerlaluPendek {
                minimum: self.panjang_minimum,
            });
        }
        if panjang > PANJANG_MAKSIMUM {
            return Err(PasswordDitolak::TerlaluPanjang {
                maksimum: PANJANG_MAKSIMUM,
            });
        }
        if self.periksa_daftar_umum && terlalu_umum(password) {
            return Err(PasswordDitolak::Umum);
        }
        Ok(())
    }
}

impl Default for KebijakanPassword {
    fn default() -> Self {
        Self::f0()
    }
}

/// The common-password check (R-11), as far as F0 can go without a bundled
/// wordlist.
///
/// Two cheap, honest rules instead of a fake "list":
/// 1. a password made of one repeated character or one obvious sequence;
/// 2. the obvious F0 offenders (the product name, the role names) repeated or
///    followed by a counter — the shapes people actually type.
///
/// A real list (top 10k, HIBP-style) is a data dependency the ADR does not put
/// in F0. When it lands this function gains a lookup; the call site does not
/// change.
fn terlalu_umum(password: &str) -> bool {
    let lower = password.to_lowercase();
    let min = KebijakanPassword::f0().panjang_minimum;

    // One character repeated ("aaaaaaaaaaaaaaaa").
    let mut chars = lower.chars();
    if let Some(first) = chars.next()
        && chars.all(|c| c == first)
    {
        return true;
    }

    // A run of adjacent keys, counted as a *walk* over the keyboard rather than
    // a substring of one row: "qwertyuiopasdfgh" is one keystroke path that
    // crosses rows, so scanning rows separately would miss it. A walk of at
    // least `min` characters is what people type when they do not think.
    if terlalu_terurut(&lower, min) {
        return true;
    }

    // Product and role names, optionally doubled and with digits/punctuation
    // appended: "PoolRouter2026!", "auditorauditor1", "platform_admin!".
    const AKAR: [&str; 5] = [
        "poolrouter",
        "platform_admin_",
        "auditor",
        "operator",
        "admin",
    ];
    let tanpa_angka: String = lower
        .chars()
        .filter(|c| !c.is_ascii_digit() && !c.is_ascii_punctuation() && !c.is_whitespace())
        .collect();
    AKAR.iter().any(|akar| {
        // Compare like with like: the roots are normalised the same way the
        // candidate was, so "platform_admin" matches "platformadmin".
        let akar_bersih: String = akar
            .chars()
            .filter(|c| !c.is_ascii_digit() && !c.is_ascii_punctuation() && !c.is_whitespace())
            .collect();
        !tanpa_angka.is_empty()
            && (tanpa_angka == akar_bersih || tanpa_angka == format!("{akar_bersih}{akar_bersih}"))
    })
}
/// True when the password contains a straight run of adjacent keys, or a
/// sequence of adjacent rows walked in order (`qwertyuiop` then `asdfghjkl`).
///
/// This is a heuristic — the point is to catch the passwords people actually
/// type when they are not thinking, not to model a keyboard's geometry exactly.
/// A real word that happens to contain four adjacent keys is fine; a password
/// that is *mostly* a keyboard run is not.
fn terlalu_terurut(password: &str, min: usize) -> bool {
    // The three letter rows plus the digit row, in physical order.
    const BARIS: [&str; 4] = ["1234567890", "qwertyuiop", "asdfghjkl", "zxcvbnm"];

    // A run of `min` adjacent characters inside one row, in either direction:
    // "qwertyuiop", "poiuytrewq", "1234567890".
    for baris in BARIS {
        for jendela in baris.as_bytes().windows(min.min(baris.len())) {
            if jendela.len() < min {
                continue;
            }
            let s = std::str::from_utf8(jendela).unwrap();
            if password.contains(s) || password.contains(&s.chars().rev().collect::<String>()) {
                return true;
            }
        }
    }

    // Adjacent rows walked one after another: "qwertyuiopasdfgh" is row 1's
    // start then row 2's start, which a per-row scan never sees.
    for pasangan in BARIS.windows(2) {
        let (atas, bawah) = (pasangan[0], pasangan[1]);
        for i in 0..atas.len() {
            for j in 0..bawah.len() {
                let gabung = format!("{}{}", &atas[i..], &bawah[..=j.min(bawah.len() - 1)]);
                if gabung.len() >= min && password.contains(&gabung) {
                    return true;
                }
            }
        }
    }

    false
}

/// An argon2id hasher built from pinned parameters. Cheap to construct, so
/// call sites do not need to hold onto one.
fn hasher(param: ParameterArgon2) -> Result<argon2::Argon2<'static>, GalatHash> {
    let params = Params::new(param.memori_kib, param.iterasi, param.paralelisme, None)
        .map_err(|_| GalatHash)?;
    Ok(argon2::Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        params,
    ))
}

/// Hash a password into a PHC string (`$argon2id$v=19$m=19456,t=2,p=1$...`),
/// which is exactly what `admin_user.password_hash` stores.
///
/// The policy is enforced here and not only in the CLI: a hash written by any
/// path must satisfy R-11, or the rule is only as good as the most careless
/// caller.
///
/// The salt is generated by `password-hash` itself from the OS CSPRNG (the
/// `getrandom` feature, on by default), so this crate never picks salt bytes.
/// Output is wrapped in `Zeroizing` so the buffer is wiped on drop.
pub fn hash_password(
    password: &str,
    kebijakan: KebijakanPassword,
    param: ParameterArgon2,
) -> Result<Zeroizing<String>, HashGagal> {
    kebijakan.periksa(password).map_err(HashGagal::Ditolak)?;
    let hash = hasher(param)
        .map_err(HashGagal::Hash)?
        .hash_password(password.as_bytes())
        .map_err(|_| HashGagal::Hash(GalatHash))?;
    Ok(Zeroizing::new(hash.to_string()))
}

/// Either the password failed the policy or the hash could not be produced.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashGagal {
    Ditolak(PasswordDitolak),
    Hash(GalatHash),
}

impl fmt::Display for HashGagal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Ditolak(e) => e.fmt(f),
            Self::Hash(e) => e.fmt(f),
        }
    }
}

impl std::error::Error for HashGagal {}

/// Verify a password against a stored PHC string.
///
/// Returns `false` for every failure — wrong password, malformed hash, unknown
/// algorithm. The caller gets one bit because the response is one 401: telling
/// "no such user" from "wrong password" is the enumeration oracle the login
/// limiter exists to blunt.
///
/// A malformed stored hash is an operator error, and it fails *closed*: nobody
/// gets in, and the caller should log it (without the password).
pub fn verifikasi_password(password: &str, hash_phc: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash_phc) else {
        return false;
    };
    // The parameters come from the stored string, not from today's constant, so
    // a hash made under older parameters keeps working after the ADR changes
    // them (rehash-on-login can then be done deliberately).
    argon2::Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok()
}

/// Constant-time string comparison, for values the application compares itself
/// rather than through a hash library (today: nothing security-bearing, but the
/// helper keeps a `==` from creeping into that spot later).
pub fn sama_konstan(a: &[u8], b: &[u8]) -> bool {
    a.ct_eq(b).into()
}

/// Parameters recovered from a stored PHC string, so a login can decide whether
/// the cost is still acceptable and the hash should be upgraded (ADR-0003
/// section 3.6 "dapat ditingkatkan").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ParamTersimpan {
    pub memori_kib: u32,
    pub iterasi: u32,
    pub paralelisme: u32,
}

impl ParamTersimpan {
    pub fn baca(hash_phc: &str) -> Option<Self> {
        let parsed = PasswordHash::new(hash_phc).ok()?;
        Some(Self {
            memori_kib: parsed.params.get_decimal("m")?,
            iterasi: parsed.params.get_decimal("t")?,
            paralelisme: parsed.params.get_decimal("p")?,
        })
    }

    /// True when the stored hash is weaker than `param` and should be replaced
    /// after a successful login. Only ever upgrades: a stronger stored hash is
    /// left alone.
    pub fn lebih_lemah_dari(&self, param: ParameterArgon2) -> bool {
        self.memori_kib < param.memori_kib
            || self.iterasi < param.iterasi
            || self.paralelisme < param.paralelisme
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Real argon2id at 19 MiB is ~50 ms; tests must not pay that on every
    /// case, so the policy tests use the production parameters only where the
    /// parameters themselves are under test.
    fn cepat() -> ParameterArgon2 {
        ParameterArgon2 {
            memori_kib: 8,
            iterasi: 1,
            paralelisme: 1,
        }
    }

    #[test]
    fn owasp_parameters_are_the_ones_in_the_adr() {
        assert_eq!(ParameterArgon2::OWASP.memori_kib, 19 * 1024);
        assert_eq!(ParameterArgon2::OWASP.iterasi, 2);
        assert_eq!(ParameterArgon2::OWASP.paralelisme, 1);
        assert_eq!(ParameterArgon2::default(), ParameterArgon2::OWASP);
    }

    #[test]
    fn policy_minimum_is_fifteen() {
        assert_eq!(KebijakanPassword::f0().panjang_minimum, 15);
        // 14 fails, 15 passes: the boundary, not just "somewhere near". The
        // fixtures are ordinary passphrases so the common-password check is not
        // what is under test here.
        // 14 characters fails ...
        assert!(matches!(
            KebijakanPassword::f0().periksa("kucingoranyein"),
            Err(PasswordDitolak::TerlaluPendek { minimum: 15 })
        ));
        // ... and exactly 15 passes.
        assert_eq!(KebijakanPassword::f0().periksa("kucingoranyeind"), Ok(()));
    }

    #[test]
    fn policy_has_no_composition_rules() {
        // All lowercase, no digit, no punctuation: allowed, because R-11
        // deliberately drops composition rules.
        assert!(KebijakanPassword::f0().periksa("kucingoranyeindah").is_ok());
    }

    #[test]
    fn policy_counts_characters_not_bytes() {
        // Nine two-byte characters = 18 bytes but only 9 characters: must fail,
        // proving the limit is not a byte count that a short password could pass.
        let pendek = "áéíóúàèìò";
        assert_eq!(pendek.chars().count(), 9);
        assert_eq!(pendek.len(), 18);
        assert!(KebijakanPassword::f0().periksa(pendek).is_err());

        // Fifteen distinct accented characters is 30 bytes and must pass. A
        // byte-based limit would accept a short password for the wrong reason,
        // and a naive byte slice would panic on a character boundary.
        let panjang = "áéíóúàèìòùâêîôû";
        assert_eq!(panjang.chars().count(), 15);
        assert_eq!(panjang.len(), 30);
        assert_eq!(KebijakanPassword::f0().periksa(panjang), Ok(()));
    }

    #[test]
    fn keyboard_walks_are_rejected() {
        // Walks that cross rows, which a per-row substring scan would miss.
        for jalan in ["qwertyuiopasdfgh", "asdfghjklzxcvbn", "1234567890qwerty"] {
            assert!(
                terlalu_umum(jalan),
                "{jalan} seharusnya terdeteksi sebagai walk"
            );
        }
        // A short sequence inside a longer, otherwise random password is fine:
        // the rule is about the whole password being a walk.
        assert!(!terlalu_umum("MejaBiruDiPinggirJalan"));
    }

    #[test]
    fn a_walk_has_to_reach_the_minimum_length() {
        // Ten adjacent keys is not enough to call it a walk under R-11.
        assert!(!terlalu_terurut("qw", 15)); // far below the minimum
        assert!(terlalu_terurut("qwertyuiop", 10));
        assert!(!terlalu_terurut("qwertyuiop", 11));
        // Repeating a short walk does not make a long one.
        assert!(!terlalu_terurut("qwqwqwqwqwqwqw", 15));
    }

    #[test]
    fn policy_rejects_blank_and_oversized() {
        assert_eq!(
            KebijakanPassword::f0().periksa("               "),
            Err(PasswordDitolak::Kosong)
        );
        assert!(matches!(
            KebijakanPassword::f0().periksa(&"a1".repeat(PANJANG_MAKSIMUM)),
            Err(PasswordDitolak::TerlaluPanjang { .. })
        ));
    }

    #[test]
    fn common_passwords_are_rejected() {
        for lemah in [
            "aaaaaaaaaaaaaaaa",
            "qwertyuiopasdfgh",
            "1234567890qwerty",
            "poolrouter2026!",
            "POOLROUTER20260",
            "auditorauditor1",
            "platform_admin_",
        ] {
            assert_eq!(
                KebijakanPassword::f0().periksa(lemah),
                Err(PasswordDitolak::Umum),
                "{lemah} seharusnya ditolak"
            );
        }
    }

    #[test]
    fn a_real_passphrase_survives_the_common_list() {
        // Long, low entropy per character, but not on any list we carry.
        assert!(
            KebijakanPassword::f0()
                .periksa("kucingoranyeindahsekali")
                .is_ok()
        );
        assert!(
            KebijakanPassword::f0()
                .periksa("mejabirudipinggirjalan")
                .is_ok()
        );
    }

    #[test]
    fn common_list_can_be_switched_off() {
        let longgar = KebijakanPassword {
            panjang_minimum: 15,
            periksa_daftar_umum: false,
        };
        assert!(longgar.periksa("aaaaaaaaaaaaaaaa").is_ok());
    }

    #[test]
    fn hash_and_verify_round_trip() {
        let phc = hash_password("kucingoranyeindah", KebijakanPassword::f0(), cepat()).unwrap();
        assert!(phc.starts_with("$argon2id$"), "{}", *phc);
        assert!(verifikasi_password("kucingoranyeindah", &phc));
        assert!(!verifikasi_password("kucingoranyeindahh", &phc));
        assert!(!verifikasi_password("", &phc));
    }

    #[test]
    fn the_salt_makes_two_hashes_of_one_password_differ() {
        let a = hash_password("kucingoranyeindah", KebijakanPassword::f0(), cepat()).unwrap();
        let b = hash_password("kucingoranyeindah", KebijakanPassword::f0(), cepat()).unwrap();
        assert_ne!(*a, *b);
        // ... and both still verify.
        assert!(verifikasi_password("kucingoranyeindah", &a));
        assert!(verifikasi_password("kucingoranyeindah", &b));
    }

    #[test]
    fn hash_refuses_a_password_that_breaks_the_policy() {
        // The policy is enforced at the hash site too, so no code path can
        // write a weak hash into admin_user.
        let err = hash_password("pendek", KebijakanPassword::f0(), cepat()).unwrap_err();
        assert_eq!(
            err,
            HashGagal::Ditolak(PasswordDitolak::TerlaluPendek { minimum: 15 })
        );
    }

    #[test]
    fn the_phc_string_carries_the_parameters() {
        let phc = hash_password("kucingoranyeindah", KebijakanPassword::f0(), cepat()).unwrap();
        let tersimpan = ParamTersimpan::baca(&phc).unwrap();
        assert_eq!(tersimpan.memori_kib, 8);
        assert_eq!(tersimpan.iterasi, 1);
        assert_eq!(tersimpan.paralelisme, 1);
    }

    #[test]
    fn upgrade_detection_is_one_way() {
        let lama = ParamTersimpan {
            memori_kib: 8,
            iterasi: 1,
            paralelisme: 1,
        };
        assert!(lama.lebih_lemah_dari(ParameterArgon2::OWASP));

        let baru = ParamTersimpan {
            memori_kib: ParameterArgon2::OWASP.memori_kib,
            iterasi: ParameterArgon2::OWASP.iterasi,
            paralelisme: ParameterArgon2::OWASP.paralelisme,
        };
        assert!(!baru.lebih_lemah_dari(ParameterArgon2::OWASP));

        // Anything stronger stays put.
        let kuat = ParamTersimpan {
            memori_kib: 64 * 1024,
            iterasi: 4,
            paralelisme: 2,
        };
        assert!(!kuat.lebih_lemah_dari(ParameterArgon2::OWASP));
    }

    #[test]
    fn a_malformed_stored_hash_fails_closed() {
        // Fails closed: a corrupted row locks that account out rather than
        // letting anyone in.
        for rusak in ["", "bukan-phc", "$argon2id$v=19$m=x,t=2,p=1$c2FsdA$aGFzaA"] {
            assert!(!verifikasi_password("kucingoranyeindah", rusak), "{rusak}");
        }
        assert_eq!(ParamTersimpan::baca("bukan-phc"), None);
    }

    #[test]
    fn errors_never_echo_the_password() {
        let rahasia = "rahasia-yang-sangat-pendek";
        let gagal = hash_password(
            rahasia,
            KebijakanPassword {
                panjang_minimum: 100,
                periksa_daftar_umum: false,
            },
            cepat(),
        )
        .unwrap_err();
        let pesan = gagal.to_string();
        assert!(!pesan.contains(rahasia), "{pesan}");
        // Same for the debug form the CLI may print.
        assert!(!format!("{gagal:?}").contains(rahasia));
    }

    #[test]
    fn constant_time_helper_agrees_with_equality() {
        assert!(sama_konstan(b"abc", b"abc"));
        assert!(!sama_konstan(b"abc", b"abd"));
        assert!(!sama_konstan(b"abc", b"ab"));
    }

    #[test]
    fn production_parameters_produce_a_working_hash() {
        // One real-cost hash, so the pinned parameters are proven usable and
        // not merely numerically equal to the ADR.
        let phc = hash_password(
            "kucingoranyeindah",
            KebijakanPassword::f0(),
            ParameterArgon2::OWASP,
        )
        .unwrap();
        assert!(phc.contains("m=19456,t=2,p=1"), "{}", *phc);
        assert!(verifikasi_password("kucingoranyeindah", &phc));
    }
}
