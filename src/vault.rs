//! Bóveda: cifrado en disco de los datos propios de la app.
//!
//! Esquema de *cifrado de sobre* (envelope encryption), el mismo que usan
//! Chrome/Edge para cookies y contraseñas ("os_crypt"), Electron
//! (`safeStorage`) o Signal Desktop:
//!
//! - Una **clave de datos** de 256 bits, aleatoria (generador del sistema),
//!   cifra cada archivo con **AES-256-GCM** (cifrado autenticado: cualquier
//!   cambio en el archivo se detecta y se rechaza).
//! - Esa clave nunca se guarda en claro: se guarda envuelta por **DPAPI**
//!   (`CryptProtectData`), que la ata a la cuenta de Windows del usuario.
//!   Otro usuario del equipo, o el disco montado en otra máquina, no puede
//!   descifrarla.
//! - Cada archivo lleva su propio *nonce* aleatorio de 96 bits y, como datos
//!   asociados, su propósito (`settings`, `window`...): un archivo cifrado
//!   no se puede hacer pasar por otro.
//! - La clave vive en memoria en un búfer que se borra al soltarlo
//!   (`zeroize`).
//! - **Atada al equipo** cuando hay chip de seguridad (TPM 2.0, el que
//!   exige Windows 11): antes de DPAPI, la clave se cifra con una clave RSA
//!   que crea el propio TPM (proveedor "Microsoft Platform Crypto Provider")
//!   y que nunca sale de él. Así, aunque alguien copie el disco y conozca la
//!   contraseña de Windows, en otro equipo no puede abrir la bóveda. Es lo
//!   que hace Windows Hello con sus claves. Sin avisos ni diálogos. Sin TPM,
//!   se queda solo DPAPI.
//!
//! Formato de un archivo cifrado: `YTV1` | nonce (12) | texto cifrado + tag (16).
//! Formato de `vault.key`: `YTK1` | DPAPI(clave), o
//! `YTK2` | DPAPI(variante (1) | RSA-OAEP del TPM(clave)).

use std::{fs, io, path::Path};
#[cfg(test)]
use std::path::PathBuf;

use aes_gcm::{
    aead::{Aead, KeyInit, Payload},
    Aes256Gcm, Nonce,
};
use zeroize::Zeroizing;

const KEY_MAGIC: &[u8; 4] = b"YTK1";
const KEY_MAGIC_HARDWARE: &[u8; 4] = b"YTK2";
const DATA_MAGIC: &[u8; 4] = b"YTV1";
const NONCE_LEN: usize = 12;
const TAG_LEN: usize = 16;
const KEY_LEN: usize = 32;
/// Ningún archivo de la app se acerca a esto; más grande = corrupto.
const MAX_FILE: u64 = 4 * 1024 * 1024;

/// Quien envuelve la clave de datos (DPAPI en Windows).
pub trait Protector {
    fn protect(&self, secret: &[u8]) -> io::Result<Vec<u8>>;
    fn unprotect(&self, blob: &[u8]) -> io::Result<Zeroizing<Vec<u8>>>;
}

/// Qué pasó al abrir la bóveda.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opened {
    /// Se usó la clave existente.
    Existing,
    /// No había clave: se creó una nueva.
    Created,
    /// La clave existente no se pudo desenvolver (perfil de Windows
    /// restablecido, carpeta copiada de otro usuario o equipo): se creó una
    /// nueva y lo cifrado antes ya no se puede leer.
    Replaced,
}

pub struct Vault {
    key: Zeroizing<[u8; KEY_LEN]>,
}

/// Quién protege la clave guardada en `vault.key`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyProtection {
    /// La cuenta de Windows (DPAPI).
    Account,
    /// El chip de seguridad del equipo (TPM) y la cuenta de Windows.
    DeviceAndAccount,
}

/// Lee de qué tipo es `vault.key` (sin descifrar nada).
pub fn key_protection(key_path: &Path) -> Option<KeyProtection> {
    let bytes = read_bounded(key_path).ok()??;
    if bytes.starts_with(KEY_MAGIC_HARDWARE) {
        Some(KeyProtection::DeviceAndAccount)
    } else if bytes.starts_with(KEY_MAGIC) {
        Some(KeyProtection::Account)
    } else {
        None
    }
}

/// Envuelve con `hardware` y luego con `account`.
fn wrap_layered(secret: &[u8], account: &dyn Protector, hardware: &dyn Protector) -> io::Result<Vec<u8>> {
    let inner = Zeroizing::new(hardware.protect(secret)?);
    account.protect(&inner)
}

fn unwrap_layered(blob: &[u8], account: &dyn Protector, hardware: &dyn Protector) -> io::Result<Zeroizing<Vec<u8>>> {
    let inner = account.unprotect(blob)?;
    hardware.unprotect(&inner)
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_owned())
}

/// Bytes aleatorios del generador criptográfico del sistema.
pub fn random_bytes<const N: usize>() -> io::Result<[u8; N]> {
    let mut out = [0u8; N];
    getrandom::fill(&mut out).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(out)
}

/// Escritura atómica: archivo temporal + rename, para no dejar un archivo a
/// medias si el proceso muere a mitad de la escritura.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

fn read_bounded(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::metadata(path) {
        Ok(meta) if meta.len() > MAX_FILE => Err(invalid("archivo demasiado grande")),
        Ok(_) => fs::read(path).map(Some),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

impl Vault {
    /// Abre la bóveda de `key_path` o crea una clave nueva (protegida por la
    /// cuenta; `upgrade` la pasa después al TPM). Sin TPM: para las pruebas.
    #[cfg(test)]
    pub fn open(key_path: &Path, protector: &dyn Protector) -> io::Result<(Self, Opened)> {
        Self::open_with(key_path, protector, None)
    }

    /// Igual, sabiendo abrir también claves atadas al TPM (`hardware`).
    pub fn open_with(key_path: &Path, protector: &dyn Protector, hardware: Option<&dyn Protector>) -> io::Result<(Self, Opened)> {
        if let Some(bytes) = read_bounded(key_path)? {
            let unwrapped = if let Some(blob) = bytes.strip_prefix(KEY_MAGIC_HARDWARE.as_slice()) {
                match hardware {
                    Some(hardware) => unwrap_layered(blob, protector, hardware),
                    None => Err(invalid("la clave está atada a un TPM que no está disponible")),
                }
            } else {
                bytes
                    .strip_prefix(KEY_MAGIC.as_slice())
                    .ok_or_else(|| invalid("vault.key con formato desconocido"))
                    .and_then(|blob| protector.unprotect(blob))
            };
            if let Ok(secret) = unwrapped {
                if secret.len() == KEY_LEN {
                    let mut key = Zeroizing::new([0u8; KEY_LEN]);
                    key.copy_from_slice(&secret);
                    return Ok((Self { key }, Opened::Existing));
                }
            }
            let vault = Self::create(key_path, protector)?;
            return Ok((vault, Opened::Replaced));
        }
        Ok((Self::create(key_path, protector)?, Opened::Created))
    }

    fn create(key_path: &Path, protector: &dyn Protector) -> io::Result<Self> {
        let key = Zeroizing::new(random_bytes::<KEY_LEN>()?);
        let mut file = KEY_MAGIC.to_vec();
        file.extend(protector.protect(key.as_slice())?);
        write_atomic(key_path, &file)?;
        Ok(Self { key })
    }

    /// Ata la clave (la misma: los archivos cifrados siguen valiendo) al
    /// TPM además de a la cuenta. Antes de reemplazar `vault.key` comprueba
    /// que se puede desenvolver de vuelta: si el TPM falla en algo, todo
    /// queda como estaba. Lento la primera vez (el TPM genera la clave RSA):
    /// llamar desde un hilo aparte.
    pub fn upgrade(&self, key_path: &Path, account: &dyn Protector, hardware: &dyn Protector) -> io::Result<()> {
        if key_protection(key_path) == Some(KeyProtection::DeviceAndAccount) {
            return Ok(());
        }
        let blob = wrap_layered(self.key.as_slice(), account, hardware)?;
        let back = unwrap_layered(&blob, account, hardware)?;
        if back.as_slice() != self.key.as_slice() {
            return Err(invalid("el TPM no devolvió la misma clave"));
        }
        let mut file = KEY_MAGIC_HARDWARE.to_vec();
        file.extend(blob);
        write_atomic(key_path, &file)
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new_from_slice(self.key.as_slice()).expect("clave de 32 bytes")
    }

    fn aad(purpose: &str) -> Vec<u8> {
        [DATA_MAGIC.as_slice(), b"/", purpose.as_bytes()].concat()
    }

    /// Cifra `plaintext` para el propósito dado.
    pub fn seal(&self, purpose: &str, plaintext: &[u8]) -> io::Result<Vec<u8>> {
        let nonce = random_bytes::<NONCE_LEN>()?;
        let aad = Self::aad(purpose);
        let ciphertext = self
            .cipher()
            .encrypt(Nonce::from_slice(&nonce), Payload { msg: plaintext, aad: &aad })
            .map_err(|_| io::Error::other("no se pudo cifrar"))?;
        let mut out = Vec::with_capacity(4 + NONCE_LEN + ciphertext.len());
        out.extend_from_slice(DATA_MAGIC);
        out.extend_from_slice(&nonce);
        out.extend(ciphertext);
        Ok(out)
    }

    /// Descifra y verifica. Falla si el archivo se modificó, es de otro
    /// propósito o se cifró con otra clave.
    pub fn unseal(&self, purpose: &str, data: &[u8]) -> io::Result<Zeroizing<Vec<u8>>> {
        let body = data.strip_prefix(DATA_MAGIC.as_slice()).ok_or_else(|| invalid("no es un archivo de la bóveda"))?;
        if body.len() < NONCE_LEN + TAG_LEN {
            return Err(invalid("archivo de la bóveda truncado"));
        }
        let (nonce, ciphertext) = body.split_at(NONCE_LEN);
        let aad = Self::aad(purpose);
        self.cipher()
            .decrypt(Nonce::from_slice(nonce), Payload { msg: ciphertext, aad: &aad })
            .map(Zeroizing::new)
            .map_err(|_| invalid("el archivo no pasó la verificación (modificado o de otra clave)"))
    }

    pub fn write(&self, path: &Path, purpose: &str, plaintext: &[u8]) -> io::Result<()> {
        write_atomic(path, &self.seal(purpose, plaintext)?)
    }

    /// `Ok(None)` si el archivo no existe.
    pub fn read(&self, path: &Path, purpose: &str) -> io::Result<Option<Zeroizing<Vec<u8>>>> {
        read_bounded(path)?.map(|data| self.unseal(purpose, &data)).transpose()
    }
}

/// El protector del sistema: DPAPI en Windows. Fuera de Windows (solo para
/// compilar y probar) no hay equivalente y la clave queda sin envolver.
pub fn system_protector() -> Box<dyn Protector> {
    #[cfg(windows)]
    {
        Box::new(Dpapi)
    }
    #[cfg(not(windows))]
    {
        struct Plain;
        impl Protector for Plain {
            fn protect(&self, secret: &[u8]) -> io::Result<Vec<u8>> {
                Ok(secret.to_vec())
            }
            fn unprotect(&self, blob: &[u8]) -> io::Result<Zeroizing<Vec<u8>>> {
                Ok(Zeroizing::new(blob.to_vec()))
            }
        }
        Box::new(Plain)
    }
}

/// El chip de seguridad (TPM) si el equipo tiene uno utilizable.
pub fn hardware_protector() -> Option<Box<dyn Protector>> {
    #[cfg(windows)]
    {
        tpm::Tpm::available().then(|| Box::new(tpm::Tpm) as Box<dyn Protector>)
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Clave RSA-2048 creada dentro del TPM con el proveedor de Windows
/// ("Microsoft Platform Crypto Provider"): no exportable, solo para
/// descifrar, sin PIN ni diálogos (`NCRYPT_SILENT_FLAG`).
#[cfg(windows)]
mod tpm {
    use std::{io, ptr::null_mut};

    use windows_sys::{
        core::PCWSTR,
        Win32::Security::Cryptography::{
            NCryptCreatePersistedKey, NCryptDecrypt, NCryptEncrypt, NCryptFinalizeKey, NCryptFreeObject, NCryptOpenKey,
            NCryptOpenStorageProvider, NCryptSetProperty, BCRYPT_OAEP_PADDING_INFO, BCRYPT_RSA_ALGORITHM,
            BCRYPT_SHA1_ALGORITHM, BCRYPT_SHA256_ALGORITHM, MS_PLATFORM_CRYPTO_PROVIDER, NCRYPT_ALLOW_DECRYPT_FLAG,
            NCRYPT_KEY_USAGE_PROPERTY, NCRYPT_LENGTH_PROPERTY, NCRYPT_PAD_OAEP_FLAG, NCRYPT_SILENT_FLAG,
        },
    };
    use zeroize::Zeroizing;

    use super::Protector;

    const KEY_NAME: PCWSTR = windows_sys::core::w!("YT Mini Player vault key v1");
    /// Relleno OAEP con SHA-256 (1) o, si el TPM no lo admite, SHA-1 (2).
    const VARIANTS: [(u8, PCWSTR); 2] = [(1, BCRYPT_SHA256_ALGORITHM), (2, BCRYPT_SHA1_ALGORITHM)];

    pub struct Tpm;

    /// Libera el objeto de NCrypt al salir del ámbito.
    struct Handle(usize);
    impl Drop for Handle {
        fn drop(&mut self) {
            if self.0 != 0 {
                unsafe { NCryptFreeObject(self.0) };
            }
        }
    }

    fn check(status: i32, what: &str) -> io::Result<()> {
        if status == 0 {
            Ok(())
        } else {
            Err(io::Error::other(format!("TPM: {what} falló ({:#010x})", status as u32)))
        }
    }

    fn provider() -> io::Result<Handle> {
        let mut handle = 0usize;
        check(unsafe { NCryptOpenStorageProvider(&mut handle, MS_PLATFORM_CRYPTO_PROVIDER, 0) }, "abrir el proveedor")?;
        Ok(Handle(handle))
    }

    fn open_key(provider: &Handle) -> io::Result<Handle> {
        let mut key = 0usize;
        check(unsafe { NCryptOpenKey(provider.0, &mut key, KEY_NAME, 0, NCRYPT_SILENT_FLAG) }, "abrir la clave")?;
        Ok(Handle(key))
    }

    fn create_key(provider: &Handle) -> io::Result<Handle> {
        let mut key = 0usize;
        check(
            unsafe { NCryptCreatePersistedKey(provider.0, &mut key, BCRYPT_RSA_ALGORITHM, KEY_NAME, 0, 0) },
            "crear la clave",
        )?;
        let key = Handle(key);
        let length = 2048u32.to_le_bytes();
        let usage = NCRYPT_ALLOW_DECRYPT_FLAG.to_le_bytes();
        unsafe {
            // Tamaño y uso: si el TPM no admite fijarlos, valen los de
            // fábrica (RSA-2048); lo que importa se comprueba al cifrar.
            NCryptSetProperty(key.0, NCRYPT_LENGTH_PROPERTY, length.as_ptr(), 4, NCRYPT_SILENT_FLAG);
            NCryptSetProperty(key.0, NCRYPT_KEY_USAGE_PROPERTY, usage.as_ptr(), 4, NCRYPT_SILENT_FLAG);
            check(NCryptFinalizeKey(key.0, NCRYPT_SILENT_FLAG), "generar la clave")?;
        }
        Ok(key)
    }

    fn padding(hash: PCWSTR) -> BCRYPT_OAEP_PADDING_INFO {
        BCRYPT_OAEP_PADDING_INFO { pszAlgId: hash, pbLabel: null_mut(), cbLabel: 0 }
    }

    fn encrypt(key: &Handle, hash: PCWSTR, data: &[u8]) -> io::Result<Vec<u8>> {
        let pad = padding(hash);
        let pad_ptr = (&pad as *const BCRYPT_OAEP_PADDING_INFO).cast();
        let flags = NCRYPT_PAD_OAEP_FLAG | NCRYPT_SILENT_FLAG;
        let mut size = 0u32;
        unsafe {
            check(NCryptEncrypt(key.0, data.as_ptr(), data.len() as u32, pad_ptr, null_mut(), 0, &mut size, flags), "cifrar")?;
            let mut out = vec![0u8; size as usize];
            check(NCryptEncrypt(key.0, data.as_ptr(), data.len() as u32, pad_ptr, out.as_mut_ptr(), size, &mut size, flags), "cifrar")?;
            out.truncate(size as usize);
            Ok(out)
        }
    }

    fn decrypt(key: &Handle, hash: PCWSTR, data: &[u8]) -> io::Result<Zeroizing<Vec<u8>>> {
        let pad = padding(hash);
        let pad_ptr = (&pad as *const BCRYPT_OAEP_PADDING_INFO).cast();
        let flags = NCRYPT_PAD_OAEP_FLAG | NCRYPT_SILENT_FLAG;
        let mut size = 0u32;
        unsafe {
            check(NCryptDecrypt(key.0, data.as_ptr(), data.len() as u32, pad_ptr, null_mut(), 0, &mut size, flags), "descifrar")?;
            let mut out = Zeroizing::new(vec![0u8; size as usize]);
            check(NCryptDecrypt(key.0, data.as_ptr(), data.len() as u32, pad_ptr, out.as_mut_ptr(), size, &mut size, flags), "descifrar")?;
            out.truncate(size as usize);
            Ok(out)
        }
    }

    impl Tpm {
        /// ¿Hay TPM y el proveedor de Windows responde?
        pub fn available() -> bool {
            provider().is_ok()
        }
    }

    impl Protector for Tpm {
        fn protect(&self, secret: &[u8]) -> io::Result<Vec<u8>> {
            let provider = provider()?;
            let key = match open_key(&provider) {
                Ok(key) => key,
                Err(_) => create_key(&provider)?,
            };
            let mut last = None;
            for (variant, hash) in VARIANTS {
                match encrypt(&key, hash, secret) {
                    Ok(ciphertext) => {
                        let mut out = vec![variant];
                        out.extend(ciphertext);
                        return Ok(out);
                    }
                    Err(err) => last = Some(err),
                }
            }
            Err(last.unwrap_or_else(|| io::Error::other("TPM: sin relleno admitido")))
        }

        fn unprotect(&self, blob: &[u8]) -> io::Result<Zeroizing<Vec<u8>>> {
            let (&variant, ciphertext) = blob.split_first().ok_or_else(|| io::Error::other("TPM: blob vacío"))?;
            let hash = VARIANTS
                .iter()
                .find(|(v, _)| *v == variant)
                .map(|(_, h)| *h)
                .ok_or_else(|| io::Error::other("TPM: variante desconocida"))?;
            let provider = provider()?;
            let key = open_key(&provider)?;
            decrypt(&key, hash, ciphertext)
        }
    }
}

/// DPAPI de Windows: la clave queda atada a la cuenta del usuario.
#[cfg(windows)]
pub struct Dpapi;

#[cfg(windows)]
impl Dpapi {
    /// Entropía adicional: otra app del mismo usuario que llame a DPAPI sin
    /// conocerla no desenvuelve la clave con una llamada genérica.
    const ENTROPY: &'static [u8] = b"YtMiniPlayer/vault/v1";

    fn blob(bytes: &[u8]) -> windows_sys::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB {
        windows_sys::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB {
            cbData: bytes.len() as u32,
            pbData: bytes.as_ptr() as *mut u8,
        }
    }

    unsafe fn take(out: windows_sys::Win32::Security::Cryptography::CRYPT_INTEGER_BLOB) -> Vec<u8> {
        let bytes = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        windows_sys::Win32::Foundation::LocalFree(out.pbData.cast());
        bytes
    }
}

#[cfg(windows)]
impl Protector for Dpapi {
    fn protect(&self, secret: &[u8]) -> io::Result<Vec<u8>> {
        use windows_sys::Win32::Security::Cryptography::{CryptProtectData, CRYPTPROTECT_UI_FORBIDDEN};
        let description: Vec<u16> = "YT Mini Player".encode_utf16().chain(Some(0)).collect();
        let input = Self::blob(secret);
        let entropy = Self::blob(Self::ENTROPY);
        let mut out = unsafe { std::mem::zeroed() };
        let ok = unsafe {
            CryptProtectData(
                &input,
                description.as_ptr(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(unsafe { Self::take(out) })
    }

    fn unprotect(&self, blob: &[u8]) -> io::Result<Zeroizing<Vec<u8>>> {
        use windows_sys::Win32::Security::Cryptography::{CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN};
        let input = Self::blob(blob);
        let entropy = Self::blob(Self::ENTROPY);
        let mut out = unsafe { std::mem::zeroed() };
        let ok = unsafe {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                &entropy,
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut bytes = unsafe { Self::take(out) };
        let secret = Zeroizing::new(bytes.clone());
        zeroize::Zeroize::zeroize(&mut bytes);
        Ok(secret)
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// Sustituto de DPAPI para las pruebas (no es seguro: solo invierte bytes).
    pub struct TestProtector(pub u8);

    impl Protector for TestProtector {
        fn protect(&self, secret: &[u8]) -> io::Result<Vec<u8>> {
            Ok(secret.iter().map(|b| b ^ self.0).collect())
        }
        fn unprotect(&self, blob: &[u8]) -> io::Result<Zeroizing<Vec<u8>>> {
            Ok(Zeroizing::new(blob.iter().map(|b| b ^ self.0).collect()))
        }
    }

    pub fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ytmp-vault-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn creates_reuses_and_replaces_the_key() {
        let dir = temp_dir("key");
        let key = dir.join("vault.key");
        let (a, how) = Vault::open(&key, &TestProtector(0x5a)).unwrap();
        assert_eq!(how, Opened::Created);
        let raw = fs::read(&key).unwrap();
        assert!(raw.starts_with(b"YTK1"));
        assert!(!raw.windows(KEY_LEN).any(|w| w == a.key.as_slice()), "la clave no se guarda en claro");

        let sealed = a.seal("settings", b"hola").unwrap();
        let (b, how) = Vault::open(&key, &TestProtector(0x5a)).unwrap();
        assert_eq!(how, Opened::Existing);
        assert_eq!(b.unseal("settings", &sealed).unwrap().as_slice(), b"hola");

        // Otro "usuario": la clave no se desenvuelve bien → se reemplaza.
        fs::write(&key, b"YTK1corto").unwrap();
        let (c, how) = Vault::open(&key, &TestProtector(0x5a)).unwrap();
        assert_eq!(how, Opened::Replaced);
        assert!(c.unseal("settings", &sealed).is_err(), "lo de la clave vieja ya no se lee");
        fs::remove_dir_all(&dir).unwrap();
    }

    /// "TPM" de pruebas: otra transformación reversible, con un byte de
    /// variante como el de verdad. Como un TPM real, el de otro equipo no
    /// descifra lo que cifró este (falla en vez de devolver basura).
    struct FakeTpm(u8);
    impl Protector for FakeTpm {
        fn protect(&self, secret: &[u8]) -> io::Result<Vec<u8>> {
            Ok([1, self.0].into_iter().chain(secret.iter().map(|b| b.wrapping_add(self.0))).collect())
        }
        fn unprotect(&self, blob: &[u8]) -> io::Result<Zeroizing<Vec<u8>>> {
            match blob {
                [1, device, rest @ ..] if *device == self.0 => Ok(Zeroizing::new(rest.iter().map(|b| b.wrapping_sub(self.0)).collect())),
                _ => Err(invalid("no es de este TPM")),
            }
        }
    }

    /// Un TPM que cifra pero no devuelve lo mismo (o falla al descifrar).
    struct BrokenTpm;
    impl Protector for BrokenTpm {
        fn protect(&self, secret: &[u8]) -> io::Result<Vec<u8>> {
            Ok(secret.to_vec())
        }
        fn unprotect(&self, _: &[u8]) -> io::Result<Zeroizing<Vec<u8>>> {
            Ok(Zeroizing::new(vec![0u8; KEY_LEN]))
        }
    }

    #[test]
    fn key_moves_to_the_tpm_without_losing_data() {
        let dir = temp_dir("tpm");
        let key = dir.join("vault.key");
        let account = TestProtector(0x33);
        let (vault, how) = Vault::open_with(&key, &account, Some(&FakeTpm(9))).unwrap();
        assert_eq!(how, Opened::Created);
        assert_eq!(key_protection(&key), Some(KeyProtection::Account), "se crea con la cuenta; el TPM va después");
        let sealed = vault.seal("settings", b"ajustes").unwrap();

        // Un TPM defectuoso: no se toca nada.
        assert!(vault.upgrade(&key, &account, &BrokenTpm).is_err());
        assert_eq!(key_protection(&key), Some(KeyProtection::Account));

        vault.upgrade(&key, &account, &FakeTpm(9)).unwrap();
        assert_eq!(key_protection(&key), Some(KeyProtection::DeviceAndAccount));
        assert!(fs::read(&key).unwrap().starts_with(b"YTK2"));
        vault.upgrade(&key, &account, &FakeTpm(9)).unwrap();

        // Con el TPM: misma clave, los archivos de antes se leen.
        let (again, how) = Vault::open_with(&key, &account, Some(&FakeTpm(9))).unwrap();
        assert_eq!(how, Opened::Existing);
        assert_eq!(again.unseal("settings", &sealed).unwrap().as_slice(), b"ajustes");

        // En "otro equipo" (sin ese TPM) o sin TPM: no se abre; empieza de cero.
        let (_, how) = Vault::open_with(&key, &account, Some(&FakeTpm(10))).unwrap();
        assert_eq!(how, Opened::Replaced);
        fs::write(&key, [b"YTK2".as_slice(), &account.protect(&FakeTpm(9).protect(&[7u8; KEY_LEN]).unwrap()).unwrap()].concat()).unwrap();
        let (_, how) = Vault::open(&key, &account).unwrap();
        assert_eq!(how, Opened::Replaced, "sin TPM disponible no se puede abrir una clave atada a él");
        assert_eq!(key_protection(&dir.join("no-existe")), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn authenticated_encryption_detects_tampering() {
        let dir = temp_dir("aead");
        let (vault, _) = Vault::open(&dir.join("vault.key"), &TestProtector(1)).unwrap();
        let secret = br#"{"pin_hash":"$argon2id$..."}"#;
        let sealed = vault.seal("settings", secret).unwrap();
        assert!(!sealed.windows(8).any(|w| w == b"argon2id"), "no queda texto en claro");
        assert_eq!(sealed.len(), 4 + NONCE_LEN + secret.len() + TAG_LEN);
        assert_ne!(vault.seal("settings", secret).unwrap(), sealed, "nonce distinto en cada escritura");

        for i in [4, 4 + NONCE_LEN, sealed.len() - 1] {
            let mut bad = sealed.clone();
            bad[i] ^= 1;
            assert!(vault.unseal("settings", &bad).is_err(), "byte {i} modificado");
        }
        assert!(vault.unseal("window", &sealed).is_err(), "no se puede hacer pasar por otro archivo");
        assert!(vault.unseal("settings", &sealed[..20]).is_err());
        assert!(vault.unseal("settings", b"{\"plano\":1}").is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn files_round_trip_and_missing_is_none() {
        let dir = temp_dir("files");
        let (vault, _) = Vault::open(&dir.join("vault.key"), &TestProtector(7)).unwrap();
        let path = dir.join("settings.enc");
        assert!(vault.read(&path, "settings").unwrap().is_none());
        vault.write(&path, "settings", b"datos").unwrap();
        assert_eq!(vault.read(&path, "settings").unwrap().unwrap().as_slice(), b"datos");
        assert!(!dir.join("settings.tmp").exists());
        fs::write(&path, vec![0u8; (MAX_FILE + 1) as usize]).unwrap();
        assert!(vault.read(&path, "settings").is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}
