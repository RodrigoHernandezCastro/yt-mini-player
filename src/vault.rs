//! Vault: on-disk encryption of the app's own data.
//!
//! An *envelope encryption* scheme, the same Chrome/Edge use for cookies
//! and passwords ("os_crypt"), Electron (`safeStorage`) or Signal Desktop:
//!
//! - A random 256-bit **data key** (system generator) encrypts each file
//!   with **AES-256-GCM** (authenticated encryption: any change to the file
//!   is detected and rejected).
//! - That key is never stored in the clear: it is stored wrapped by
//!   **DPAPI** (`CryptProtectData`), which binds it to the user's Windows
//!   account. Another user of the PC, or the disk mounted on another
//!   machine, cannot decrypt it.
//! - Each file carries its own random 96-bit *nonce* and, as associated
//!   data, its purpose (`settings`, `window`...): one encrypted file cannot
//!   pass for another.
//! - The key lives in memory in a buffer that is wiped when dropped
//!   (`zeroize`).
//! - **Bound to the PC** when there is a security chip (TPM 2.0, which
//!   Windows 11 requires): before DPAPI, the key is encrypted with an RSA
//!   key the TPM itself creates (the "Microsoft Platform Crypto Provider")
//!   and that never leaves it. So even someone who copies the disk and knows
//!   the Windows password cannot open the vault on another PC. It is what
//!   Windows Hello does with its keys. No prompts or dialogs. Without a TPM,
//!   only DPAPI remains.
//!
//! Encrypted file format: `YTV1` | nonce (12) | ciphertext + tag (16).
//! `vault.key` format: `YTK1` | DPAPI(key), or
//! `YTK2` | DPAPI(variant (1) | TPM RSA-OAEP(key)).

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
/// No file of the app comes near this (the biggest music library is a few
/// MB); bigger = corrupt.
const MAX_FILE: u64 = 64 * 1024 * 1024;

/// What wraps the data key (DPAPI on Windows).
pub trait Protector {
    fn protect(&self, secret: &[u8]) -> io::Result<Vec<u8>>;
    fn unprotect(&self, blob: &[u8]) -> io::Result<Zeroizing<Vec<u8>>>;
}

/// What happened when opening the vault.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opened {
    /// The existing key was used.
    Existing,
    /// There was no key: a new one was created.
    Created,
    /// The existing key could not be unwrapped (Windows profile reset,
    /// folder copied from another user or PC): a new one was created and
    /// what was encrypted before can no longer be read.
    Replaced,
}

pub struct Vault {
    key: Zeroizing<[u8; KEY_LEN]>,
}

/// What protects the key stored in `vault.key`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyProtection {
    /// The Windows account (DPAPI).
    Account,
    /// The PC's security chip (TPM) and the Windows account.
    DeviceAndAccount,
}

/// Reads what kind of `vault.key` it is (without decrypting anything).
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

/// Wraps with `hardware` and then with `account`.
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

/// Random bytes from the system's cryptographic generator.
pub fn random_bytes<const N: usize>() -> io::Result<[u8; N]> {
    let mut out = [0u8; N];
    getrandom::fill(&mut out).map_err(|e| io::Error::other(e.to_string()))?;
    Ok(out)
}

/// Atomic write: temporary file + rename, so no half-written file is left
/// if the process dies in the middle of writing.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

fn read_bounded(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match fs::metadata(path) {
        Ok(meta) if meta.len() > MAX_FILE => Err(invalid("file too large")),
        Ok(_) => fs::read(path).map(Some),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

impl Vault {
    /// Opens the vault of `key_path` or creates a new key (protected by the
    /// account; `upgrade` moves it to the TPM later). Without a TPM: for
    /// the tests.
    #[cfg(test)]
    pub fn open(key_path: &Path, protector: &dyn Protector) -> io::Result<(Self, Opened)> {
        Self::open_with(key_path, protector, None)
    }

    /// The same, also able to open keys bound to the TPM (`hardware`).
    pub fn open_with(key_path: &Path, protector: &dyn Protector, hardware: Option<&dyn Protector>) -> io::Result<(Self, Opened)> {
        if let Some(bytes) = read_bounded(key_path)? {
            let unwrapped = if let Some(blob) = bytes.strip_prefix(KEY_MAGIC_HARDWARE.as_slice()) {
                match hardware {
                    Some(hardware) => unwrap_layered(blob, protector, hardware),
                    None => Err(invalid("the key is bound to a TPM that is not available")),
                }
            } else {
                bytes
                    .strip_prefix(KEY_MAGIC.as_slice())
                    .ok_or_else(|| invalid("vault.key has an unknown format"))
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

    /// Binds the key (the same one: encrypted files stay valid) to the TPM
    /// as well as to the account. Before replacing `vault.key` it checks the
    /// key unwraps back: if the TPM fails at anything, everything stays as
    /// it was. Slow the first time (the TPM generates the RSA key): call
    /// from a separate thread.
    pub fn upgrade(&self, key_path: &Path, account: &dyn Protector, hardware: &dyn Protector) -> io::Result<()> {
        if key_protection(key_path) == Some(KeyProtection::DeviceAndAccount) {
            return Ok(());
        }
        let blob = wrap_layered(self.key.as_slice(), account, hardware)?;
        let back = unwrap_layered(&blob, account, hardware)?;
        if back.as_slice() != self.key.as_slice() {
            return Err(invalid("the TPM did not return the same key"));
        }
        let mut file = KEY_MAGIC_HARDWARE.to_vec();
        file.extend(blob);
        write_atomic(key_path, &file)
    }

    fn cipher(&self) -> Aes256Gcm {
        Aes256Gcm::new_from_slice(self.key.as_slice()).expect("a 32-byte key")
    }

    fn aad(purpose: &str) -> Vec<u8> {
        [DATA_MAGIC.as_slice(), b"/", purpose.as_bytes()].concat()
    }

    /// Encrypts `plaintext` for the given purpose.
    pub fn seal(&self, purpose: &str, plaintext: &[u8]) -> io::Result<Vec<u8>> {
        let nonce = random_bytes::<NONCE_LEN>()?;
        let aad = Self::aad(purpose);
        let ciphertext = self
            .cipher()
            .encrypt(Nonce::from_slice(&nonce), Payload { msg: plaintext, aad: &aad })
            .map_err(|_| io::Error::other("could not encrypt"))?;
        let mut out = Vec::with_capacity(4 + NONCE_LEN + ciphertext.len());
        out.extend_from_slice(DATA_MAGIC);
        out.extend_from_slice(&nonce);
        out.extend(ciphertext);
        Ok(out)
    }

    /// Decrypts and verifies. Fails if the file was modified, belongs to
    /// another purpose or was encrypted with another key.
    pub fn unseal(&self, purpose: &str, data: &[u8]) -> io::Result<Zeroizing<Vec<u8>>> {
        let body = data.strip_prefix(DATA_MAGIC.as_slice()).ok_or_else(|| invalid("not a vault file"))?;
        if body.len() < NONCE_LEN + TAG_LEN {
            return Err(invalid("truncated vault file"));
        }
        let (nonce, ciphertext) = body.split_at(NONCE_LEN);
        let aad = Self::aad(purpose);
        self.cipher()
            .decrypt(Nonce::from_slice(nonce), Payload { msg: ciphertext, aad: &aad })
            .map(Zeroizing::new)
            .map_err(|_| invalid("the file failed verification (modified or from another key)"))
    }

    pub fn write(&self, path: &Path, purpose: &str, plaintext: &[u8]) -> io::Result<()> {
        write_atomic(path, &self.seal(purpose, plaintext)?)
    }

    /// `Ok(None)` if the file does not exist.
    pub fn read(&self, path: &Path, purpose: &str) -> io::Result<Option<Zeroizing<Vec<u8>>>> {
        read_bounded(path)?.map(|data| self.unseal(purpose, &data)).transpose()
    }
}

/// The system protector: DPAPI on Windows. Outside Windows (only to build
/// and test) there is no equivalent and the key stays unwrapped.
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

/// The security chip (TPM) if the PC has a usable one.
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

/// An RSA-2048 key created inside the TPM with the Windows provider
/// ("Microsoft Platform Crypto Provider"): not exportable, decryption only,
/// no PIN or dialogs (`NCRYPT_SILENT_FLAG`).
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
    /// OAEP padding with SHA-256 (1) or, if the TPM does not support it, SHA-1 (2).
    const VARIANTS: [(u8, PCWSTR); 2] = [(1, BCRYPT_SHA256_ALGORITHM), (2, BCRYPT_SHA1_ALGORITHM)];

    pub struct Tpm;

    /// Frees the NCrypt object when it goes out of scope.
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
            Err(io::Error::other(format!("TPM: {what} failed ({:#010x})", status as u32)))
        }
    }

    fn provider() -> io::Result<Handle> {
        let mut handle = 0usize;
        check(unsafe { NCryptOpenStorageProvider(&mut handle, MS_PLATFORM_CRYPTO_PROVIDER, 0) }, "opening the provider")?;
        Ok(Handle(handle))
    }

    fn open_key(provider: &Handle) -> io::Result<Handle> {
        let mut key = 0usize;
        check(unsafe { NCryptOpenKey(provider.0, &mut key, KEY_NAME, 0, NCRYPT_SILENT_FLAG) }, "opening the key")?;
        Ok(Handle(key))
    }

    fn create_key(provider: &Handle) -> io::Result<Handle> {
        let mut key = 0usize;
        check(
            unsafe { NCryptCreatePersistedKey(provider.0, &mut key, BCRYPT_RSA_ALGORITHM, KEY_NAME, 0, 0) },
            "creating the key",
        )?;
        let key = Handle(key);
        let length = 2048u32.to_le_bytes();
        let usage = NCRYPT_ALLOW_DECRYPT_FLAG.to_le_bytes();
        unsafe {
            // Size and usage: if the TPM does not allow setting them, the
            // defaults (RSA-2048) apply; what matters is checked on encrypt.
            NCryptSetProperty(key.0, NCRYPT_LENGTH_PROPERTY, length.as_ptr(), 4, NCRYPT_SILENT_FLAG);
            NCryptSetProperty(key.0, NCRYPT_KEY_USAGE_PROPERTY, usage.as_ptr(), 4, NCRYPT_SILENT_FLAG);
            check(NCryptFinalizeKey(key.0, NCRYPT_SILENT_FLAG), "generating the key")?;
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
            check(NCryptEncrypt(key.0, data.as_ptr(), data.len() as u32, pad_ptr, null_mut(), 0, &mut size, flags), "encrypting")?;
            let mut out = vec![0u8; size as usize];
            check(NCryptEncrypt(key.0, data.as_ptr(), data.len() as u32, pad_ptr, out.as_mut_ptr(), size, &mut size, flags), "encrypting")?;
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
            check(NCryptDecrypt(key.0, data.as_ptr(), data.len() as u32, pad_ptr, null_mut(), 0, &mut size, flags), "decrypting")?;
            let mut out = Zeroizing::new(vec![0u8; size as usize]);
            check(NCryptDecrypt(key.0, data.as_ptr(), data.len() as u32, pad_ptr, out.as_mut_ptr(), size, &mut size, flags), "decrypting")?;
            out.truncate(size as usize);
            Ok(out)
        }
    }

    impl Tpm {
        /// Is there a TPM and does the Windows provider answer?
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
            Err(last.unwrap_or_else(|| io::Error::other("TPM: no supported padding")))
        }

        fn unprotect(&self, blob: &[u8]) -> io::Result<Zeroizing<Vec<u8>>> {
            let (&variant, ciphertext) = blob.split_first().ok_or_else(|| io::Error::other("TPM: empty blob"))?;
            let hash = VARIANTS
                .iter()
                .find(|(v, _)| *v == variant)
                .map(|(_, h)| *h)
                .ok_or_else(|| io::Error::other("TPM: unknown variant"))?;
            let provider = provider()?;
            let key = open_key(&provider)?;
            decrypt(&key, hash, ciphertext)
        }
    }
}

/// Windows DPAPI: the key is bound to the user's account.
#[cfg(windows)]
pub struct Dpapi;

#[cfg(windows)]
impl Dpapi {
    /// Extra entropy: another app of the same user calling DPAPI without
    /// knowing it cannot unwrap the key with a generic call.
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

    /// DPAPI stand-in for the tests (not secure: it only flips bytes).
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
        assert!(!raw.windows(KEY_LEN).any(|w| w == a.key.as_slice()), "the key is not stored in the clear");

        let sealed = a.seal("settings", b"hello").unwrap();
        let (b, how) = Vault::open(&key, &TestProtector(0x5a)).unwrap();
        assert_eq!(how, Opened::Existing);
        assert_eq!(b.unseal("settings", &sealed).unwrap().as_slice(), b"hello");

        // Another "user": the key does not unwrap properly → it is replaced.
        fs::write(&key, b"YTK1short").unwrap();
        let (c, how) = Vault::open(&key, &TestProtector(0x5a)).unwrap();
        assert_eq!(how, Opened::Replaced);
        assert!(c.unseal("settings", &sealed).is_err(), "what the old key encrypted can no longer be read");
        fs::remove_dir_all(&dir).unwrap();
    }

    /// Test "TPM": another reversible transformation, with a variant byte
    /// like the real one. Like a real TPM, another PC's cannot decrypt what
    /// this one encrypted (it fails instead of returning garbage).
    struct FakeTpm(u8);
    impl Protector for FakeTpm {
        fn protect(&self, secret: &[u8]) -> io::Result<Vec<u8>> {
            Ok([1, self.0].into_iter().chain(secret.iter().map(|b| b.wrapping_add(self.0))).collect())
        }
        fn unprotect(&self, blob: &[u8]) -> io::Result<Zeroizing<Vec<u8>>> {
            match blob {
                [1, device, rest @ ..] if *device == self.0 => Ok(Zeroizing::new(rest.iter().map(|b| b.wrapping_sub(self.0)).collect())),
                _ => Err(invalid("not from this TPM")),
            }
        }
    }

    /// A TPM that encrypts but does not give the same back (or fails to decrypt).
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
        assert_eq!(key_protection(&key), Some(KeyProtection::Account), "created with the account; the TPM comes later");
        let sealed = vault.seal("settings", b"settings").unwrap();

        // A faulty TPM: nothing is touched.
        assert!(vault.upgrade(&key, &account, &BrokenTpm).is_err());
        assert_eq!(key_protection(&key), Some(KeyProtection::Account));

        vault.upgrade(&key, &account, &FakeTpm(9)).unwrap();
        assert_eq!(key_protection(&key), Some(KeyProtection::DeviceAndAccount));
        assert!(fs::read(&key).unwrap().starts_with(b"YTK2"));
        vault.upgrade(&key, &account, &FakeTpm(9)).unwrap();

        // With the TPM: the same key, earlier files can be read.
        let (again, how) = Vault::open_with(&key, &account, Some(&FakeTpm(9))).unwrap();
        assert_eq!(how, Opened::Existing);
        assert_eq!(again.unseal("settings", &sealed).unwrap().as_slice(), b"settings");

        // On "another PC" (without that TPM) or without a TPM: it does not open; it starts fresh.
        let (_, how) = Vault::open_with(&key, &account, Some(&FakeTpm(10))).unwrap();
        assert_eq!(how, Opened::Replaced);
        fs::write(&key, [b"YTK2".as_slice(), &account.protect(&FakeTpm(9).protect(&[7u8; KEY_LEN]).unwrap()).unwrap()].concat()).unwrap();
        let (_, how) = Vault::open(&key, &account).unwrap();
        assert_eq!(how, Opened::Replaced, "without the TPM a key bound to it cannot be opened");
        assert_eq!(key_protection(&dir.join("does-not-exist")), None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn authenticated_encryption_detects_tampering() {
        let dir = temp_dir("aead");
        let (vault, _) = Vault::open(&dir.join("vault.key"), &TestProtector(1)).unwrap();
        let secret = br#"{"pin_hash":"$argon2id$..."}"#;
        let sealed = vault.seal("settings", secret).unwrap();
        assert!(!sealed.windows(8).any(|w| w == b"argon2id"), "no plain text is left");
        assert_eq!(sealed.len(), 4 + NONCE_LEN + secret.len() + TAG_LEN);
        assert_ne!(vault.seal("settings", secret).unwrap(), sealed, "a different nonce on every write");

        for i in [4, 4 + NONCE_LEN, sealed.len() - 1] {
            let mut bad = sealed.clone();
            bad[i] ^= 1;
            assert!(vault.unseal("settings", &bad).is_err(), "byte {i} modified");
        }
        assert!(vault.unseal("window", &sealed).is_err(), "it cannot pass for another file");
        assert!(vault.unseal("settings", &sealed[..20]).is_err());
        assert!(vault.unseal("settings", b"{\"plain\":1}").is_err());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn files_round_trip_and_missing_is_none() {
        let dir = temp_dir("files");
        let (vault, _) = Vault::open(&dir.join("vault.key"), &TestProtector(7)).unwrap();
        let path = dir.join("settings.enc");
        assert!(vault.read(&path, "settings").unwrap().is_none());
        vault.write(&path, "settings", b"data").unwrap();
        assert_eq!(vault.read(&path, "settings").unwrap().unwrap().as_slice(), b"data");
        assert!(!dir.join("settings.tmp").exists());
        fs::write(&path, vec![0u8; (MAX_FILE + 1) as usize]).unwrap();
        assert!(vault.read(&path, "settings").is_err());
        fs::remove_dir_all(&dir).unwrap();
    }
}
