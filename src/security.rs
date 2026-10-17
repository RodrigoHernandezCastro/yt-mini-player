//! Protección del sistema: retirada del cifrado EFS de la 0.6.0, avisos de
//! bloqueo de Windows y suspensión, y utilidades de diálogo.
//!
//! La 0.6.0 cifraba la carpeta de datos con EFS. La primera vez que una
//! cuenta usa EFS, Windows muestra "Haga una copia de seguridad del
//! certificado y de la clave de cifrado de archivos", que asustaba sin
//! motivo. Desde la 0.6.1 no se usa EFS: lo sensible ya va cifrado sin
//! avisos (cookies por Chromium con DPAPI; los datos de la app en la bóveda,
//! con la clave atada al TPM del equipo y a la cuenta de Windows, ver
//! vault.rs). Si la carpeta quedó cifrada con EFS, se descifra al abrir.

#[cfg(not(windows))]
use std::path::Path;

/// Qué se hizo con el cifrado EFS de versiones anteriores.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyEfs {
    /// La carpeta no estaba cifrada con EFS.
    None,
    /// Se descifró (cuántos archivos y carpetas).
    Removed { items: usize },
    /// Algunos archivos no se pudieron descifrar (en uso): se reintenta al
    /// próximo arranque.
    Partial { failed: usize },
}

impl LegacyEfs {
    /// Para el resumen de la protección (vacío si no hay nada que contar).
    pub fn describe(&self) -> &'static str {
        match self {
            LegacyEfs::Partial { .. } => {
                "\n\nQuedan algunos archivos cifrados con EFS por la versión anterior; se descifrarán al volver a abrir la app."
            }
            _ => "",
        }
    }
}

/// Cachés del perfil: se regeneran solas. Se borran antes de descifrar para
/// no tener que reescribir cientos de MB.
const REBUILDABLE: &[&str] = &["Cache", "Code Cache", "GPUCache", "GrShaderCache", "ShaderCache", "DawnCache"];

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
mod win {
    use std::{
        fs,
        os::windows::ffi::OsStrExt,
        path::Path,
        sync::OnceLock,
    };

    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, WPARAM},
        Storage::FileSystem::{DecryptFileW, FILE_ATTRIBUTE_ENCRYPTED},
        System::RemoteDesktop::{WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION},
        UI::{
            Shell::{DefSubclassProc, SetWindowSubclass, ShellExecuteW},
            WindowsAndMessaging::{
                MessageBoxW, IDYES, MB_DEFBUTTON2, MB_ICONINFORMATION, MB_ICONWARNING, MB_OK, MB_SETFOREGROUND,
                MB_TOPMOST, MB_YESNO, SW_SHOWNORMAL, WM_POWERBROADCAST, WM_WTSSESSION_CHANGE,
            },
        },
    };

    use super::{LegacyEfs, REBUILDABLE};

    const WTS_SESSION_LOCK: usize = 0x7;
    const PBT_APMSUSPEND: usize = 0x4;
    /// Marca que dejaba la 0.6.0 al terminar de cifrar.
    const MARKER: &str = ".efs-ok";

    fn wide(s: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
        s.as_ref().encode_wide().chain(Some(0)).collect()
    }

    fn is_encrypted(path: &Path) -> bool {
        use std::os::windows::fs::MetadataExt;
        fs::symlink_metadata(path).is_ok_and(|m| m.file_attributes() & FILE_ATTRIBUTE_ENCRYPTED != 0)
    }

    fn decrypt(path: &Path) -> bool {
        unsafe { DecryptFileW(wide(path).as_ptr(), 0) != 0 }
    }

    /// Si la carpeta de datos quedó cifrada con EFS (0.6.0), la descifra:
    /// archivos y carpetas (así lo nuevo ya no se cifra). Descifrar no
    /// muestra ningún aviso de Windows. Llamar ANTES de arrancar WebView2:
    /// con el perfil abierto, sus archivos están en uso.
    pub fn remove_legacy_efs(root: &Path, profile: &Path) -> LegacyEfs {
        if !is_encrypted(root) && !root.join(MARKER).exists() {
            return LegacyEfs::None;
        }
        for cache in walk_dirs(profile).into_iter().filter(|d| {
            d.file_name().and_then(|n| n.to_str()).is_some_and(|n| REBUILDABLE.contains(&n))
        }) {
            let _ = fs::remove_dir_all(cache);
        }
        let (mut done, mut failed) = (0, 0);
        let mut dirs = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let path = entry.path();
                let Ok(kind) = entry.file_type() else { continue };
                if kind.is_symlink() {
                    continue;
                }
                if kind.is_dir() {
                    stack.push(path.clone());
                    dirs.push(path);
                } else if is_encrypted(&path) {
                    if decrypt(&path) {
                        done += 1;
                    } else {
                        failed += 1;
                    }
                }
            }
        }
        if failed > 0 {
            // La raíz sigue marcada como cifrada: se reintenta al próximo
            // arranque.
            return LegacyEfs::Partial { failed };
        }
        // Las carpetas al final (de la más honda a la raíz).
        dirs.reverse();
        dirs.push(root.to_path_buf());
        for dir in dirs.iter().filter(|d| is_encrypted(d)) {
            if decrypt(dir) {
                done += 1;
            }
        }
        let _ = fs::remove_file(root.join(MARKER));
        LegacyEfs::Removed { items: done }
    }

    fn walk_dirs(root: &Path) -> Vec<std::path::PathBuf> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                if entry.file_type().is_ok_and(|t| t.is_dir() && !t.is_symlink()) {
                    out.push(entry.path());
                    stack.push(entry.path());
                }
            }
        }
        out
    }

    static ON_LOCK: OnceLock<Box<dyn Fn() + Send + Sync>> = OnceLock::new();

    /// Llama a `on_lock` cuando se bloquea la sesión de Windows (Win+L,
    /// cambiar de usuario) o el equipo se suspende.
    pub fn on_session_lock(hwnd: isize, on_lock: impl Fn() + Send + Sync + 'static) {
        unsafe extern "system" fn proc(
            hwnd: HWND,
            msg: u32,
            wparam: WPARAM,
            lparam: LPARAM,
            _id: usize,
            _data: usize,
        ) -> LRESULT {
            let lock = (msg == WM_WTSSESSION_CHANGE && wparam == WTS_SESSION_LOCK)
                || (msg == WM_POWERBROADCAST && wparam == PBT_APMSUSPEND);
            if lock {
                if let Some(callback) = ON_LOCK.get() {
                    callback();
                }
            }
            DefSubclassProc(hwnd, msg, wparam, lparam)
        }
        if ON_LOCK.set(Box::new(on_lock)).is_err() {
            return;
        }
        unsafe {
            WTSRegisterSessionNotification(hwnd as HWND, NOTIFY_FOR_THIS_SESSION);
            SetWindowSubclass(hwnd as HWND, Some(proc), 0x7974_6d6c, 0);
        }
    }

    /// Abre una dirección web en el navegador predeterminado.
    pub fn open_in_browser(url: &str) {
        unsafe {
            ShellExecuteW(
                std::ptr::null_mut(),
                wide("open").as_ptr(),
                wide(url).as_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                SW_SHOWNORMAL,
            );
        }
    }

    /// Pregunta Sí/No (por defecto No). Diálogo del sistema: la página web
    /// no puede dibujarlo ni pulsarlo.
    pub fn confirm(hwnd: isize, title: &str, text: &str) -> bool {
        let flags = MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2 | MB_TOPMOST | MB_SETFOREGROUND;
        unsafe { MessageBoxW(hwnd as HWND, wide(text).as_ptr(), wide(title).as_ptr(), flags) == IDYES }
    }

    pub fn inform(hwnd: isize, title: &str, text: &str) {
        let flags = MB_OK | MB_ICONINFORMATION | MB_TOPMOST | MB_SETFOREGROUND;
        unsafe { MessageBoxW(hwnd as HWND, wide(text).as_ptr(), wide(title).as_ptr(), flags) };
    }
}

#[cfg(not(windows))]
pub fn remove_legacy_efs(_: &Path, _: &Path) -> LegacyEfs {
    let _ = REBUILDABLE;
    LegacyEfs::None
}

#[cfg(not(windows))]
pub fn on_session_lock(_: isize, _: impl Fn() + Send + Sync + 'static) {}

#[cfg(not(windows))]
pub fn open_in_browser(_: &str) {}

#[cfg(not(windows))]
pub fn confirm(_: isize, _: &str, _: &str) -> bool {
    false
}

#[cfg(not(windows))]
pub fn inform(_: isize, _: &str, _: &str) {}
