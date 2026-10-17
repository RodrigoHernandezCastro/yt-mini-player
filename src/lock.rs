//! Bloqueo de la app: Windows Hello o PIN.
//!
//! - **Windows Hello** (huella, rostro o el PIN de Windows): la verificación
//!   la hace Windows (`UserConsentVerifier`), con su propio límite de
//!   intentos. La app nunca ve el dato biométrico ni el PIN de Windows.
//! - **PIN propio**: se guarda solo su hash **Argon2id** (función de
//!   derivación lenta y con mucha memoria, la recomendada por OWASP para
//!   contraseñas) con sal aleatoria, dentro de la bóveda cifrada. Tras 5
//!   intentos fallidos, cada intento más obliga a esperar el doble (30 s,
//!   1 min, 2 min… hasta 15 min); el contador sobrevive a reiniciar la app.
//!
//! El bloqueo protege lo que se ve y se controla en la app (sesiones
//! abiertas, lo que suena) frente a otra persona con acceso al equipo. El
//! cifrado en disco lo dan la bóveda, DPAPI y EFS (security.rs).

use argon2::{
    password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Algorithm, Argon2, Params, Version,
};
use zeroize::Zeroizing;

pub const PIN_MIN: usize = 6;
pub const PIN_MAX: usize = 64;
/// Fallos gratis antes de empezar a esperar.
const FREE_ATTEMPTS: u32 = 5;
const FIRST_DELAY: u64 = 30;
const MAX_DELAY: u64 = 15 * 60;

/// Argon2id con 64 MiB, 3 pasadas, 1 hilo: ~0,2–0,4 s por intento en un
/// equipo normal; probar PINs por fuerza bruta fuera de la app es caro.
fn argon2() -> Argon2<'static> {
    let params = Params::new(64 * 1024, 3, 1, None).expect("parámetros válidos");
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
}

/// ¿Sirve como PIN? Devuelve el motivo si no.
pub fn check_pin(pin: &str) -> Result<(), &'static str> {
    let len = pin.chars().count();
    if len < PIN_MIN {
        return Err("El PIN debe tener al menos 6 caracteres");
    }
    if len > PIN_MAX || pin.chars().any(char::is_control) {
        return Err("El PIN tiene caracteres no válidos o es demasiado largo");
    }
    let first = pin.chars().next().unwrap_or_default();
    if pin.chars().all(|c| c == first) {
        return Err("El PIN no puede ser un mismo carácter repetido");
    }
    let digits: Vec<u32> = pin.chars().filter_map(|c| c.to_digit(10)).collect();
    if digits.len() == len && (digits.windows(2).all(|w| w[1] == w[0] + 1) || digits.windows(2).all(|w| w[0] == w[1] + 1)) {
        return Err("El PIN no puede ser una secuencia como 123456");
    }
    Ok(())
}

/// Hash Argon2id en formato PHC (`$argon2id$v=19$m=65536,t=3,p=1$sal$hash`).
pub fn hash_pin(pin: &str) -> Result<String, String> {
    let salt_bytes = Zeroizing::new(crate::vault::random_bytes::<16>().map_err(|e| e.to_string())?);
    let salt = SaltString::encode_b64(salt_bytes.as_slice()).map_err(|e| e.to_string())?;
    argon2().hash_password(pin.as_bytes(), &salt).map(|h| h.to_string()).map_err(|e| e.to_string())
}

/// Comprueba el PIN contra el hash guardado (comparación en tiempo
/// constante dentro de `argon2`).
pub fn verify_pin(pin: &str, phc: &str) -> bool {
    let Ok(hash) = PasswordHash::new(phc) else { return false };
    // Se verifica con los parámetros guardados en el propio hash.
    Argon2::default().verify_password(pin.as_bytes(), &hash).is_ok()
}

/// Espera obligatoria tras `failures` fallos seguidos (segundos).
pub fn delay_after(failures: u32) -> u64 {
    if failures < FREE_ATTEMPTS {
        return 0;
    }
    let doublings = (failures - FREE_ATTEMPTS).min(16);
    (FIRST_DELAY << doublings).min(MAX_DELAY)
}

/// Segundos que faltan para poder intentarlo, o 0.
pub fn remaining(blocked_until: u64, now: u64) -> u64 {
    blocked_until.saturating_sub(now)
}

pub fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(windows)]
pub use hello::hello_verify;

/// Windows Hello vía `UserConsentVerifier`. Bloqueantes: llamar desde un
/// hilo aparte.
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
        // Ya inicializado devuelve un error que no importa.
        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
    }

    /// Pide la verificación con el diálogo de Windows, ligado a `hwnd`.
    /// `Ok(true)` solo si Windows confirma que es el usuario; `Ok(false)` si
    /// se canceló; `Err` con el motivo si Windows Hello no se puede usar.
    pub fn hello_verify(hwnd: isize, message: &str) -> Result<bool, String> {
        init_thread();
        let unavailable = |e: windows::core::Error| format!("no está disponible en este equipo (código {:#010x})", e.code().0);
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
                Err("no está configurado en este equipo; configúralo en Configuración → Cuentas → Opciones de inicio de sesión, o usa un PIN propio".into())
            }
            UserConsentVerificationResult::DisabledByPolicy => Err("está desactivado por una directiva del equipo".into()),
            UserConsentVerificationResult::RetriesExhausted => Err("demasiados intentos; espera un momento".into()),
            _ => Ok(false),
        }
    }
}

#[cfg(not(windows))]
pub fn hello_verify(_: isize, _: &str) -> Result<bool, String> {
    Err("solo en Windows".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_rules() {
        assert!(check_pin("482915").is_ok());
        assert!(check_pin("correcto caballo").is_ok());
        for bad in ["", "12345", "111111", "aaaaaaa", "123456", "987654", "12345678", "abc\u{7}def"] {
            assert!(check_pin(bad).is_err(), "{bad:?}");
        }
        assert!(check_pin(&"x1".repeat(40)).is_err(), "demasiado largo");
    }

    #[test]
    fn pin_hash_is_argon2id_salted_and_verifiable() {
        let a = hash_pin("482915").unwrap();
        assert!(a.starts_with("$argon2id$v=19$m=65536,t=3,p=1$"), "{a}");
        assert!(!a.contains("482915"));
        let b = hash_pin("482915").unwrap();
        assert_ne!(a, b, "sal distinta cada vez");
        assert!(verify_pin("482915", &a) && verify_pin("482915", &b));
        assert!(!verify_pin("482916", &a));
        assert!(!verify_pin("482915", "no es un hash"));
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
    }
}
