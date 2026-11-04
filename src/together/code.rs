//! Watch together: the invitation code.
//!
//! The host's app keeps a random 256-bit secret `S` for the session. Time is
//! cut into 30-minute windows, `w = ⌊unix time / 1800⌋`, and the code of a
//! window is
//!
//! ```text
//! T    = HMAC-SHA256(S, "ytmp-together/code/v1" ‖ w as 8 big-endian bytes)
//! code = the first 55 bits of T in Crockford Base32 (11 characters)
//!        + 1 Luhn mod 32 check character        →  K7Q4-9XM2-PF3T
//! ```
//!
//! So the code changes by itself every 30 minutes, nobody can work out the
//! next one without `S` (it never leaves the host's PC), and a typo is caught
//! before asking the server (Luhn mod 32 catches every single wrong
//! character and almost every swap of two neighbours).
//!
//! The server never sees the code. Both secrets that come from it go
//! through Argon2id first (19 MiB, 2 passes), then HKDF:
//!
//! ```text
//! M        = Argon2id(code, "ytmp-together/code-kdf/v2", 19 MiB, t = 2, p = 1)
//! lookup   = HKDF-SHA256(M, "ytmp-together/v2", "lookup")   what the room is found by
//! join_key = HKDF-SHA256(M, "ytmp-together/v2", "join")     the first message's key
//! ```
//!
//! The relay sees the lookup. With a fast hash it could try all 2⁵⁵ codes
//! within the half hour one lasts (a few hundred rented GPUs); Argon2id makes
//! every guess cost 19 MiB and tens of milliseconds, so that is out of reach.

use std::sync::{Arc, OnceLock};

use hmac::{Hmac, Mac};
use sha2::Sha256;
use zeroize::Zeroizing;

/// How long a code lasts.
pub const PERIOD_SECS: u64 = 30 * 60;
/// The previous code still lets people in for a while, so a code shared a
/// minute before it changes still works.
pub const GRACE_SECS: u64 = 5 * 60;
/// Crockford's Base32: no I, L, O or U (they look like 1, 1, 0 and V).
const ALPHABET: &[u8; 32] = b"0123456789ABCDEFGHJKMNPQRSTVWXYZ";
const DATA_CHARS: usize = 11;
pub const CODE_CHARS: usize = DATA_CHARS + 1;
const CODE_LABEL: &[u8] = b"ytmp-together/code/v1";
const KDF_SALT: &[u8] = b"ytmp-together/code-kdf/v2";
const HKDF_SALT: &[u8] = b"ytmp-together/v2";
/// Argon2id's cost per code: OWASP's minimum for passwords.
pub const KDF_MEMORY_KIB: u32 = 19 * 1024;
pub const KDF_PASSES: u32 = 2;
const _: () = assert!(KDF_MEMORY_KIB >= 19 * 1024 && KDF_PASSES >= 2, "OWASP's minimum for Argon2id");

/// Argon2id of the code (32 bytes).
fn master_with(code: &str, memory_kib: u32, passes: u32) -> Zeroizing<[u8; 32]> {
    use argon2::{Algorithm, Argon2, Params, Version};
    let params = Params::new(memory_kib, passes, 1, Some(32)).expect("valid Argon2 parameters");
    let mut out = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(code.as_bytes(), KDF_SALT, out.as_mut_slice())
        .expect("valid Argon2 lengths");
    out
}

/// The tests make hundreds of codes, so there the cost is light; the real
/// one is pinned by `fixed_vector`.
fn master(code: &str) -> Zeroizing<[u8; 32]> {
    if cfg!(test) {
        master_with(code, 64, 1)
    } else {
        master_with(code, KDF_MEMORY_KIB, KDF_PASSES)
    }
}

/// The 30-minute window a moment belongs to.
pub fn window(unix_secs: u64) -> u64 {
    unix_secs / PERIOD_SECS
}

/// When a window ends (the next code starts), in Unix seconds.
pub fn window_end(window: u64) -> u64 {
    (window + 1) * PERIOD_SECS
}

/// A valid code: 12 characters of the alphabet, check character included.
/// Its Argon2id result is worked out once and shared by its copies.
#[derive(Clone)]
pub struct Code(String, Arc<OnceLock<Zeroizing<[u8; 32]>>>);

impl PartialEq for Code {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for Code {}

impl std::fmt::Debug for Code {
    // Never in logs whole.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Code({}…)", &self.0[..2])
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CodeError {
    /// Not 12 characters once spaces and dashes are left out.
    Length,
    /// A character that is not in the alphabet.
    Character,
    /// The check character does not match: a typo.
    Check,
}

impl Code {
    /// The code of `window` for the session secret.
    pub fn for_window(secret: &[u8; 32], window: u64) -> Self {
        let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(secret).expect("HMAC takes any key length");
        mac.update(CODE_LABEL);
        mac.update(&window.to_be_bytes());
        let tag = mac.finalize().into_bytes();
        // First 55 bits: 11 groups of 5.
        let bits = u64::from_be_bytes(tag[..8].try_into().expect("8 bytes")) >> (64 - 5 * DATA_CHARS as u32);
        let mut digits: Vec<u8> = (0..DATA_CHARS).rev().map(|i| ((bits >> (5 * i)) & 31) as u8).collect();
        digits.push(check_digit(&digits));
        Code::from_digits(digits)
    }

    fn from_digits(digits: Vec<u8>) -> Self {
        Code(digits.into_iter().map(|d| ALPHABET[d as usize] as char).collect(), Arc::default())
    }

    /// Reads what someone typed: any case, with or without spaces or
    /// dashes; O is read as 0 and I or L as 1.
    pub fn parse(input: &str) -> Result<Self, CodeError> {
        let mut digits = Vec::with_capacity(CODE_CHARS);
        for c in input.chars() {
            if c.is_whitespace() || c == '-' || c == '_' || c == '.' {
                continue;
            }
            let c = match c.to_ascii_uppercase() {
                'O' => '0',
                'I' | 'L' => '1',
                c => c,
            };
            let digit = ALPHABET.iter().position(|&a| a as char == c).ok_or(CodeError::Character)?;
            digits.push(digit as u8);
            if digits.len() > CODE_CHARS {
                return Err(CodeError::Length);
            }
        }
        if digits.len() != CODE_CHARS {
            return Err(CodeError::Length);
        }
        if !luhn_valid(&digits) {
            return Err(CodeError::Check);
        }
        Ok(Code::from_digits(digits))
    }

    /// For people: `K7Q4-9XM2-PF3T`.
    pub fn display(&self) -> String {
        format!("{}-{}-{}", &self.0[..4], &self.0[4..8], &self.0[8..])
    }

    /// What the server knows the room by (64 hex characters).
    pub fn lookup(&self) -> String {
        hex(&super::crypto::hkdf(self.master().as_slice(), HKDF_SALT, b"lookup"))
    }

    /// Argon2id of the code, worked out the first time it is needed.
    fn master(&self) -> &Zeroizing<[u8; 32]> {
        self.1.get_or_init(|| master(&self.0))
    }

    /// Key of the first message between guest and host.
    pub fn join_key(&self) -> [u8; 32] {
        super::crypto::hkdf(self.master().as_slice(), HKDF_SALT, b"join")
    }
}

/// Luhn mod N check digit (N = 32) for the data digits.
fn check_digit(digits: &[u8]) -> u8 {
    let mut factor = 2;
    let mut sum = 0u32;
    for &d in digits.iter().rev() {
        let addend = factor * u32::from(d);
        factor = if factor == 2 { 1 } else { 2 };
        sum += addend / 32 + addend % 32;
    }
    ((32 - sum % 32) % 32) as u8
}

fn luhn_valid(digits: &[u8]) -> bool {
    let mut factor = 1;
    let mut sum = 0u32;
    for &d in digits.iter().rev() {
        let addend = factor * u32::from(d);
        factor = if factor == 2 { 1 } else { 2 };
        sum += addend / 32 + addend % 32;
    }
    sum.is_multiple_of(32)
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: [u8; 32] = [7; 32];

    #[test]
    fn windows_are_thirty_minutes() {
        assert_eq!(window(0), 0);
        assert_eq!(window(1799), 0);
        assert_eq!(window(1800), 1);
        assert_eq!(window_end(window(1_800_000_123)), 1_800_001_800);
    }

    #[test]
    fn the_code_depends_on_the_secret_and_the_window() {
        let a = Code::for_window(&SECRET, 1000);
        assert_eq!(a, Code::for_window(&SECRET, 1000), "the same inputs give the same code");
        assert_ne!(a, Code::for_window(&SECRET, 1001), "a new window, a new code");
        assert_ne!(a, Code::for_window(&[8; 32], 1000), "another session, another code");
        let shown = a.display();
        assert_eq!(shown.len(), 14);
        assert_eq!(shown.matches('-').count(), 2);
        assert_eq!(Code::parse(&shown), Ok(a.clone()));
    }

    #[test]
    fn fixed_vector() {
        // Pins the algorithm (worked out separately, with Python's
        // argon2-cffi, hmac and hashlib): a change here would break codes
        // between versions.
        let code = Code::for_window(&SECRET, 987_654);
        assert_eq!(code.display(), "PTQ7-K20N-PW5J");
        let real = master_with(&code.0, KDF_MEMORY_KIB, KDF_PASSES);
        assert_eq!(hex(&super::super::crypto::hkdf(real.as_slice(), HKDF_SALT, b"lookup")), "d8768cf9032a368c73e24e53e83d04ee918dd4bed8aef821d09b2748a1b584a2");
        assert_eq!(hex(&super::super::crypto::hkdf(real.as_slice(), HKDF_SALT, b"join")), "338f2242610e4225752d7414ef41a8bd9ef4d872beff3fc2ed85e860d03b290c");
        // The light cost the tests use.
        assert_eq!(code.lookup(), "e6426934a25cb8ad794e2505469d4c9c252297cb0e62acc1e0817b59ce3cddc7");
        assert_eq!(hex(&code.join_key()), "6b46e37d5452274f765e1e644b944d1d0d1fef15e246450bde8e501cdc0f803f");
    }

    #[test]
    fn typing_is_forgiving_but_typos_are_caught() {
        let code = Code::for_window(&SECRET, 42);
        let plain = code.0.clone();
        assert_eq!(Code::parse(&plain.to_lowercase()), Ok(code.clone()));
        assert_eq!(Code::parse(&format!(" {} {} {} ", &plain[..4], &plain[4..8], &plain[8..])), Ok(code.clone()));
        // Every single wrong character is noticed.
        for i in 0..CODE_CHARS {
            for &other in ALPHABET.iter() {
                if plain.as_bytes()[i] == other {
                    continue;
                }
                let mut typo = plain.clone().into_bytes();
                typo[i] = other;
                assert_eq!(Code::parse(std::str::from_utf8(&typo).unwrap()), Err(CodeError::Check), "{i}");
            }
        }
        assert_eq!(Code::parse("K7Q4-9XM2"), Err(CodeError::Length));
        assert_eq!(Code::parse(&format!("{plain}0")), Err(CodeError::Length));
        assert_eq!(Code::parse("K7Q4-9XM2-PF3!"), Err(CodeError::Character));
        assert_eq!(Code::parse("UUUU-UUUU-UUUU"), Err(CodeError::Character), "U is not in the alphabet");
    }

    #[test]
    fn look_alike_letters_are_read_as_digits() {
        // Finds a code with a 0 or 1 and types it with O, I or L.
        for w in 0..500 {
            let code = Code::for_window(&SECRET, w);
            if code.0.contains('0') || code.0.contains('1') {
                let typed = code.0.replace('0', "o").replacen('1', "I", 1).replace('1', "l");
                assert_eq!(Code::parse(&typed), Ok(code));
                return;
            }
        }
        panic!("no code with 0 or 1 in 500 windows");
    }

    #[test]
    fn the_server_only_sees_a_hash() {
        let code = Code::for_window(&SECRET, 1);
        let lookup = code.lookup();
        assert_eq!(lookup.len(), 64);
        assert!(!lookup.to_uppercase().contains(&code.0));
        assert_ne!(lookup, Code::for_window(&SECRET, 2).lookup());
        assert_ne!(code.join_key(), Code::for_window(&SECRET, 2).join_key());
        assert!(!format!("{code:?}").contains(&code.0[2..]), "not printed whole");
    }
}

#[cfg(test)]
#[path = "../tests/code_cases.rs"]
mod cases;
