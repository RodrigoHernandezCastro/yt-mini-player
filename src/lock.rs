//! App lock: Windows Hello or a PIN.
//!
//! - **Windows Hello** (fingerprint, face or the Windows PIN): Windows does
//!   the check (`UserConsentVerifier`), with its own attempt limit. The app
//!   never sees the biometric data or the Windows PIN.
//! - **Own PIN**: only its **Argon2id** hash is kept (a slow, memory-hard
//!   key derivation function, the one OWASP recommends for passwords) with a
//!   random salt, inside the encrypted vault. After 5 failed attempts, each
//!   further attempt doubles the wait (30 s, 1 min, 2 min… up to 15 min);
//!   the counter survives restarting the app.
//!
//! The lock protects what is seen and controlled in the app (open sessions,
//! what is playing) from another person with access to the PC. Encryption
//! on disk comes from the vault and DPAPI (vault.rs).

use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Algorithm, Argon2, Params, Version,
};
use zeroize::Zeroizing;

pub const PIN_MIN: usize = 6;
pub const PIN_MAX: usize = 64;
/// Free failures before waiting starts.
const FREE_ATTEMPTS: u32 = 5;
const FIRST_DELAY: u64 = 30;
const MAX_DELAY: u64 = 15 * 60;

/// Argon2id with 64 MiB, 3 passes, 1 thread: ~0.2–0.4 s per attempt on a
/// normal PC; brute-forcing PINs outside the app is expensive.
fn argon2() -> Argon2<'static> {
    let params = Params::new(64 * 1024, 3, 1, None).expect("valid parameters");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// Is it usable as a PIN? If not, the i18n key of the reason.
pub fn check_pin(pin: &str) -> Result<(), &'static str> {
    let len = pin.chars().count();
    if len < PIN_MIN {
        return Err("lock.pin_too_short");
    }
    if len > PIN_MAX || pin.chars().any(char::is_control) {
        return Err("lock.pin_invalid");
    }
    let first = pin.chars().next().unwrap_or_default();
    if pin.chars().all(|c| c == first) {
        return Err("lock.pin_repeated");
    }
    let digits: Vec<u32> = pin.chars().filter_map(|c| c.to_digit(10)).collect();
    if digits.len() == len && (digits.windows(2).all(|w| w[1] == w[0] + 1) || digits.windows(2).all(|w| w[0] == w[1] + 1)) {
        return Err("lock.pin_sequence");
    }
    Ok(())
}

/// Argon2id hash as a PHC string (`$argon2id$v=19$m=65536,t=3,p=1$salt$hash`).
pub fn hash_pin(pin: &str) -> Result<String, String> {
    let salt_bytes = Zeroizing::new(crate::vault::random_bytes::<16>().map_err(|e| e.to_string())?);
    let salt = SaltString::encode_b64(salt_bytes.as_slice()).map_err(|e| e.to_string())?;
    argon2().hash_password(pin.as_bytes(), &salt).map(|h| h.to_string()).map_err(|e| e.to_string())
}

/// Checks the PIN against the saved hash (constant-time comparison inside
/// `argon2`).
pub fn verify_pin(pin: &str, phc: &str) -> bool {
    let Ok(hash) = PasswordHash::new(phc) else { return false };
    // Verified with the parameters stored in the hash itself.
    Argon2::default().verify_password(pin.as_bytes(), &hash).is_ok()
}

/// Mandatory wait after `failures` failures in a row (seconds).
pub fn delay_after(failures: u32) -> u64 {
    if failures < FREE_ATTEMPTS {
        return 0;
    }
    let doublings = (failures - FREE_ATTEMPTS).min(16);
    (FIRST_DELAY << doublings).min(MAX_DELAY)
}

/// Seconds left before another attempt is allowed, or 0. Never more than the
/// longest wait: if the clock went back after a failure (a time sync, a
/// flat CMOS battery), the saved moment lies far ahead and would otherwise
/// keep the PIN locked for as long as the clock was off.
pub fn remaining(blocked_until: u64, now: u64) -> u64 {
    blocked_until.saturating_sub(now).min(MAX_DELAY)
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Why Windows Hello could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HelloError {
    /// Not available on this PC (the Windows error code).
    Unavailable(i32),
    NotConfigured,
    DisabledByPolicy,
    RetriesExhausted,
}

impl HelloError {
    /// A full sentence for the lock screen.
    pub fn text(&self) -> String {
        match self {
            HelloError::Unavailable(code) => t!("lock.hello_unavailable", code = format!("{code:#010x}")),
            HelloError::NotConfigured => t!("lock.hello_not_configured"),
            HelloError::DisabledByPolicy => t!("lock.hello_disabled"),
            HelloError::RetriesExhausted => t!("lock.hello_retries"),
        }
    }
}

#[cfg(windows)]
pub use hello::hello_verify;

/// Windows Hello through `UserConsentVerifier`. Blocking: call from a
/// separate thread.
#[cfg(windows)]
mod hello {
    use windows::{
        core::{factory, HSTRING},
        Security::Credentials::UI::{UserConsentVerificationResult, UserConsentVerifier},
        Win32::{
            Foundation::HWND,
            System::WinRT::{IUserConsentVerifierInterop, RoInitialize, RO_INIT_MULTITHREADED},
        },
    };
    use windows_future::IAsyncOperation;

    fn init_thread() {
        // Already initialized returns an error that does not matter.
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
    }

    use super::HelloError;

    /// Asks for verification with the Windows dialog, tied to `hwnd`.
    /// `Ok(true)` only if Windows confirms it is the user; `Ok(false)` if it
    /// was cancelled; `Err` with the reason if Windows Hello cannot be used.
    pub fn hello_verify(hwnd: isize, message: &str) -> Result<bool, HelloError> {
        init_thread();
        let unavailable = |e: windows::core::Error| HelloError::Unavailable(e.code().0);
        let interop = factory::<UserConsentVerifier, IUserConsentVerifierInterop>().map_err(unavailable)?;
        let operation: IAsyncOperation<UserConsentVerificationResult> = unsafe {
            interop.RequestVerificationForWindowAsync(HWND(hwnd as _), &HSTRING::from(message))
        }
        .map_err(unavailable)?;
        let result = operation.join().map_err(unavailable)?;
        match result {
            UserConsentVerificationResult::Verified => Ok(true),
            UserConsentVerificationResult::Canceled | UserConsentVerificationResult::DeviceBusy => Ok(false),
            UserConsentVerificationResult::DeviceNotPresent | UserConsentVerificationResult::NotConfiguredForUser => {
                Err(HelloError::NotConfigured)
            }
            UserConsentVerificationResult::DisabledByPolicy => Err(HelloError::DisabledByPolicy),
            UserConsentVerificationResult::RetriesExhausted => Err(HelloError::RetriesExhausted),
            _ => Ok(false),
        }
    }
}

#[cfg(not(windows))]
pub fn hello_verify(_: isize, _: &str) -> Result<bool, HelloError> {
    Err(HelloError::Unavailable(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_rules() {
        assert!(check_pin("482915").is_ok());
        assert!(check_pin("correct horse").is_ok());
        for bad in ["", "12345", "111111", "aaaaaaa", "123456", "987654", "12345678", "abc\u{7}def"] {
            assert!(check_pin(bad).is_err(), "{bad:?}");
        }
        assert!(check_pin(&"x1".repeat(40)).is_err(), "too long");
        assert_eq!(check_pin("123456"), Err("lock.pin_sequence"));
    }

    #[test]
    fn pin_hash_is_argon2id_salted_and_verifiable() {
        let a = hash_pin("482915").unwrap();
        assert!(a.starts_with("$argon2id$v=19$m=65536,t=3,p=1$"), "{a}");
        assert!(!a.contains("482915"));
        let b = hash_pin("482915").unwrap();
        assert_ne!(a, b, "a different salt each time");
        assert!(verify_pin("482915", &a) && verify_pin("482915", &b));
        assert!(!verify_pin("482916", &a));
        assert!(!verify_pin("482915", "not a hash"));
        assert!(!verify_pin("", &a));
    }

    #[test]
    fn waiting_grows_after_five_failures() {
        assert_eq!((0..5).map(delay_after).collect::<Vec<_>>(), vec![0; 5]);
        assert_eq!(delay_after(5), 30);
        assert_eq!(delay_after(6), 60);
        assert_eq!(delay_after(7), 120);
        assert_eq!(delay_after(9), 480);
        assert_eq!(delay_after(10), 900);
        assert_eq!(delay_after(u32::MAX), 900);
        assert_eq!(remaining(100, 40), 60);
        assert_eq!(remaining(100, 140), 0);
        // The clock went back a year after a failure: at most the longest wait.
        assert_eq!(remaining(1_800_000_000 + 900, 1_800_000_000 - 365 * 86_400), MAX_DELAY);
        assert_eq!(remaining(u64::MAX, 0), MAX_DELAY);
    }
}
