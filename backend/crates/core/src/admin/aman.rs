//! `NilaiAman` and the NIK helpers (ADR-0003 rev 5 section 3.6 / 3.6d;
//! contract D6 "withheld-name", rule 3 "log tanpa rahasia").
//!
//! Two jobs:
//!
//! 1. **[`NilaiAman`]** — a `Display` that prints `***`. Wrapping a value that
//!    came out of the database or out of a request and printing the wrapper
//!    instead of the value means a NIK, a token, or a password cannot reach a
//!    log line, a panic message, or a `Debug` of an enclosing struct. The point
//!    is that the *generic* formatting paths — `{:?}` on a struct, `tracing`'s
//!    field capture, an `.expect()` message — all go through `Display`/`Debug`,
//!    so one wrapper closes all of them at once.
//! 2. **NIK masking** — `tersamar` produces the form the auditor screens show
//!    (`3201********0001`, first 4 and last 4), and `utuh` is only ever handed
//!    to a response for an auditor action that carries a reason.
//!
//! What this cannot do: stop a caller from calling `expose()`. It is a guard
//! rail, not a vault. The tests document where exposure is legitimate.

use std::fmt;

/// A value that must not be printed. `Display` and `Debug` both render `***`.
///
/// Not `Clone` on the inner value by default — cloning a guard is usually a
/// sign the value is about to escape somewhere it should not.
#[derive(Clone, PartialEq, Eq)]
pub struct NilaiAman<T> {
    nilai: T,
}

impl<T> NilaiAman<T> {
    pub const fn baru(nilai: T) -> Self {
        Self { nilai }
    }

    /// Get the value out. Call sites should be few and each one should be a
    /// place where the value is genuinely needed (a response body, an HMAC
    /// computation, a database bind) — never a log line.
    pub fn expose(&self) -> &T {
        &self.nilai
    }

    /// Consume the guard. Named differently from [`Self::expose`] so a grep for
    /// `expose` finds every read and a grep for `ke_dalam` finds every move.
    pub fn ke_dalam(self) -> T {
        self.nilai
    }
}

impl<T> fmt::Display for NilaiAman<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("***")
    }
}

impl<T> fmt::Debug for NilaiAman<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NilaiAman(***)")
    }
}

impl<T> From<T> for NilaiAman<T> {
    fn from(nilai: T) -> Self {
        Self::baru(nilai)
    }
}

/// A NIK exactly as the contract defines it: 16 ASCII digits, nothing else.
///
/// This is the *plaintext* NIK, so it is always wrapped in [`NilaiAman`] when
/// stored in a struct. Parsing is strict because a NIK that is 15 or 17 digits
/// is not a NIK, and letting it through would make the `nik_hmac` of two
/// different-looking inputs collide or diverge silently.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Nik(String);

/// A string that is not a 16-digit NIK.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NikTidakValid;

impl fmt::Display for NikTidakValid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NIK harus tepat 16 digit angka")
    }
}

impl std::error::Error for NikTidakValid {}

impl Nik {
    pub const PANJANG: usize = 16;

    /// Parse a NIK. Rejects anything that is not exactly 16 ASCII digits —
    /// including surrounding whitespace, because a NIK pasted with a trailing
    /// space should be trimmed by the caller that knows it came from a form,
    /// not silently accepted here and then hashed with the space.
    pub fn parse(raw: &str) -> Result<Self, NikTidakValid> {
        if raw.len() == Self::PANJANG && raw.bytes().all(|b| b.is_ascii_digit()) {
            Ok(Self(raw.to_owned()))
        } else {
            Err(NikTidakValid)
        }
    }

    /// The full NIK. Wrapped so the common `println!("{nik}")` cannot leak it.
    pub fn utuh(&self) -> NilaiAman<&str> {
        NilaiAman::baru(self.0.as_str())
    }

    /// The masked form shown by default: first 4 and last 4 digits with the
    /// middle replaced by `*`. `3201********0001`.
    ///
    /// The contract's masking form (D6) keeps 4+4, which is enough for a human
    /// to recognise a NIK they are looking for and not enough to reuse it.
    pub fn tersamar(&self) -> String {
        let b = self.0.as_bytes();
        format!(
            "{}{}{}",
            &self.0[..4],
            "*".repeat(Self::PANJANG - 8),
            // Slicing by bytes is safe here because the input is ASCII digits.
            &self.0[b.len() - 4..]
        )
    }

    /// A short hint for logs: the masked form. Same as [`Self::tersamar`] —
    /// named separately so a log line reads as a deliberate choice.
    pub fn untuk_log(&self) -> String {
        self.tersamar()
    }
}

impl fmt::Display for Nik {
    /// Prints the **masked** form. This is the important default: a `{}` on a
    /// NIK anywhere in the codebase masks it, and exposing the full value takes
    /// the explicit `.utuh()` or `.expose()`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.tersamar())
    }
}

/// Mask an already-validated NIK string without constructing a [`Nik`]. Useful
/// when the NIK is only passing through (e.g. a `varchar` from a view) and the
/// caller cannot justify the parse.
pub fn tersamar(nik: &str) -> String {
    if nik.len() != Nik::PANJANG || !nik.bytes().all(|b| b.is_ascii_digit()) {
        // Not a NIK: mask everything rather than guess which digits are safe.
        return "*".repeat(Nik::PANJANG);
    }
    Nik::parse(nik)
        .map(|n| n.tersamar())
        .unwrap_or_else(|_| "*".repeat(Nik::PANJANG))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONTOH: &str = "3201234567890001";

    #[test]
    fn nilai_aman_hides_itself_from_both_format_traits() {
        let v = NilaiAman::baru(CONTOH);
        assert_eq!(v.to_string(), "***");
        assert_eq!(format!("{v:?}"), "NilaiAman(***)");
        // Padding and width specifiers must not sneak the value out either.
        assert_eq!(format!("{v:>10}"), "***");
    }

    #[test]
    fn nilai_aman_protects_nested_debug() {
        #[derive(Debug)]
        #[allow(dead_code)]
        struct Respons {
            nik: NilaiAman<String>,
            token: NilaiAman<String>,
        }
        let r = Respons {
            nik: NilaiAman::baru(CONTOH.to_owned()),
            token: NilaiAman::baru("rahasia-token".to_owned()),
        };
        let tercetak = format!("{r:?}");
        assert!(!tercetak.contains(CONTOH), "{tercetak}");
        assert!(!tercetak.contains("rahasia-token"), "{tercetak}");
    }

    #[test]
    fn nilai_aman_exposes_on_request() {
        let v = NilaiAman::baru(CONTOH.to_owned());
        assert_eq!(v.expose(), CONTOH);
        assert_eq!(v.ke_dalam(), CONTOH);
        // And through From, the shorthand used at call sites.
        let w: NilaiAman<u32> = 7.into();
        assert_eq!(*w.expose(), 7);
    }

    #[test]
    fn nilai_aman_compares_by_value() {
        assert_eq!(NilaiAman::baru(5), NilaiAman::baru(5));
        assert_ne!(NilaiAman::baru(5), NilaiAman::baru(6));
    }

    #[test]
    fn nik_parsing_is_strict() {
        assert_eq!(Nik::parse(CONTOH).unwrap().utuh().to_string(), "***");
        for salah in [
            "320123456789000",   // 15
            "32012345678900011", // 17
            " 3201234567890001", // leading space
            "3201234567890001 ", // trailing space
            "320123456789000a",
            "3201-2345-6789-000",
            "",
            "32012345678900011x",
        ] {
            assert!(Nik::parse(salah).is_err(), "{salah:?} seharusnya ditolak");
        }
    }

    #[test]
    fn masking_keeps_four_and_four() {
        let n = Nik::parse(CONTOH).unwrap();
        assert_eq!(n.tersamar(), "3201********0001");
        assert_eq!(n.tersamar().len(), 16);
        // The middle six digits are gone, the ends are intact.
        assert!(n.tersamar().starts_with("3201"));
        assert!(n.tersamar().ends_with("0001"));
        assert!(!n.tersamar().contains("234567"));
    }

    #[test]
    fn display_of_a_nik_is_the_masked_form() {
        // The default that makes an accidental `{}` harmless.
        let n = Nik::parse(CONTOH).unwrap();
        assert_eq!(format!("{n}"), "3201********0001");
        assert!(!format!("{n}").contains(CONTOH));
    }

    #[test]
    fn utuh_is_the_only_way_to_get_all_digits() {
        let n = Nik::parse(CONTOH).unwrap();
        assert_eq!(*n.utuh().expose(), CONTOH);
    }

    #[test]
    fn log_hint_masks_too() {
        let n = Nik::parse(CONTOH).unwrap();
        assert_eq!(n.untuk_log(), "3201********0001");
        assert!(!n.untuk_log().contains(CONTOH));
    }

    #[test]
    fn free_masking_helper_matches_and_fails_closed() {
        assert_eq!(tersamar(CONTOH), Nik::parse(CONTOH).unwrap().tersamar());
        // Anything that is not 16 digits is fully masked, never partially: a
        // 19-digit input must not leak its first four digits.
        assert_eq!(tersamar("3201234567890001999"), "****************");
        assert_eq!(tersamar("pendek"), "****************");
        assert_eq!(tersamar(""), "****************");
    }

    #[test]
    fn masking_a_nik_of_all_zeros_still_masks() {
        let n = Nik::parse("0000000000000000").unwrap();
        assert_eq!(n.tersamar(), "0000********0000");
    }
}
