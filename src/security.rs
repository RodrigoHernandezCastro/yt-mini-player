//! System protection: removing 0.6.0's EFS encryption, Windows lock and
//! sleep notifications, and system dialogs.
//!
//! 0.6.0 encrypted the data folder with EFS. The first time an account uses
//! EFS, Windows shows "Back up your file encryption certificate and key",
//! which scared people for no reason. Since 0.6.1 EFS is not used: the
//! sensitive parts are already encrypted without prompts (cookies by
//! Chromium with DPAPI; the app's data in the vault, with the key bound to
//! the PC's TPM and to the Windows account, see vault.rs). If the folder was
//! left encrypted with EFS, it is decrypted on start.
//!
//! The dialogs are Windows task dialogs (not drawn by the web page, which
//! can neither draw nor click them) with buttons that say what they do
//! ("Load", "Don't load") in the app's language, rather than Yes/No in the
//! language of Windows.

#[cfg(not(windows))]
use std::path::Path;

/// What happened to the EFS encryption of older versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegacyEfs {
    /// The folder was not encrypted with EFS.
    None,
    /// It was decrypted (how many files and folders).
    Removed { items: usize },
    /// Some files could not be decrypted (in use): retried on next start.
    Partial { failed: usize },
}

impl LegacyEfs {
    /// For the protection summary (empty if there is nothing to tell).
    pub fn describe(&self) -> Option<String> {
        match self {
            LegacyEfs::Partial { .. } => Some(t!("security.efs_partial")),
            _ => None,
        }
    }
}

/// A yes/no question. The safe answer (`no`) is the default button.
pub struct Question<'a> {
    pub title: &'a str,
    /// The question itself, in large type.
    pub heading: &'a str,
    pub text: &'a str,
    /// Button that confirms ("Load", "Sign out").
    pub yes: &'a str,
    /// Button that declines ("Don't load", "Cancel").
    pub no: &'a str,
}

/// Profile caches: they rebuild themselves. Deleted before decrypting so
/// hundreds of MB need not be rewritten.
const REBUILDABLE: &[&str] = &["Cache", "Code Cache", "GPUCache", "GrShaderCache", "ShaderCache", "DawnCache"];

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
mod win {
    use std::{fs, os::windows::ffi::OsStrExt, path::Path, sync::OnceLock};

    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, S_OK, WPARAM},
        Storage::FileSystem::{DecryptFileW, FILE_ATTRIBUTE_ENCRYPTED},
        System::{
            LibraryLoader::{GetProcAddress, LoadLibraryW},
            RemoteDesktop::{WTSRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION},
        },
        UI::{
            Controls::{
                TASKDIALOGCONFIG, TASKDIALOG_BUTTON, TDF_ALLOW_DIALOG_CANCELLATION, TDF_POSITION_RELATIVE_TO_WINDOW,
                TDN_CREATED,
            },
            Shell::{DefSubclassProc, SetWindowSubclass, ShellExecuteW},
            WindowsAndMessaging::{
                MessageBoxW, SetForegroundWindow, SetWindowPos, HWND_TOPMOST, IDYES, MB_DEFBUTTON2, MB_ICONINFORMATION,
                MB_ICONWARNING, MB_OK, MB_SETFOREGROUND, MB_TOPMOST, MB_YESNO, SWP_NOMOVE, SWP_NOSIZE, SW_SHOWNORMAL,
                WM_POWERBROADCAST, WM_WTSSESSION_CHANGE,
            },
        },
    };

    use super::{LegacyEfs, Question, REBUILDABLE};

    const WTS_SESSION_LOCK: usize = 0x7;
    const PBT_APMSUSPEND: usize = 0x4;
    /// The marker 0.6.0 left when it finished encrypting.
    const MARKER: &str = ".efs-ok";
    /// `MAKEINTRESOURCE(-1)` and `(-3)`: the task dialog's built-in icons.
    const TD_WARNING_ICON: usize = 0xFFFF;
    const TD_INFORMATION_ICON: usize = 0xFFFD;
    const ID_YES: i32 = 100;
    const ID_NO: i32 = 101;

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

    /// If the data folder was left encrypted with EFS (0.6.0), decrypts it:
    /// files and folders (so new files are no longer encrypted). Decrypting
    /// shows no Windows prompt. Call BEFORE starting WebView2: with the
    /// profile open, its files are in use.
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
            // The root is still marked as encrypted: retried on next start.
            return LegacyEfs::Partial { failed };
        }
        // Folders last (deepest first, the root at the end).
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

    /// Calls `on_lock` when the Windows session locks (Win+L, switching
    /// users) or the PC goes to sleep.
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

    /// Opens a web address in the default browser.
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

    type TaskDialogIndirect = unsafe extern "system" fn(*const TASKDIALOGCONFIG, *mut i32, *mut i32, *mut i32) -> i32;

    /// `TaskDialogIndirect` from Common Controls 6 (the manifest asks for
    /// it). Looked up at run time: an executable without the manifest still
    /// starts, and falls back to a message box.
    fn task_dialog() -> Option<TaskDialogIndirect> {
        static FUNCTION: OnceLock<Option<usize>> = OnceLock::new();
        let address = *FUNCTION.get_or_init(|| unsafe {
            let module = LoadLibraryW(wide("comctl32.dll").as_ptr());
            if module.is_null() {
                return None;
            }
            GetProcAddress(module, c"TaskDialogIndirect".as_ptr().cast()).map(|f| f as usize)
        });
        address.map(|a| unsafe { std::mem::transmute::<usize, TaskDialogIndirect>(a) })
    }

    /// On top and in front, like the message boxes it replaces.
    unsafe extern "system" fn bring_forward(hwnd: HWND, msg: u32, _: WPARAM, _: LPARAM, _: isize) -> i32 {
        if msg == TDN_CREATED as u32 {
            SetWindowPos(hwnd, HWND_TOPMOST, 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE);
            SetForegroundWindow(hwnd);
        }
        S_OK
    }

    /// Runs a task dialog; `None` if task dialogs are unavailable.
    fn run_dialog(hwnd: isize, title: &str, heading: &str, text: &str, icon: usize, buttons: &[(i32, &str)], default: i32) -> Option<i32> {
        let show = task_dialog()?;
        let (title, heading, text) = (wide(title), wide(heading), wide(text));
        let labels: Vec<Vec<u16>> = buttons.iter().map(|(_, label)| wide(label)).collect();
        let items: Vec<TASKDIALOG_BUTTON> = buttons
            .iter()
            .zip(&labels)
            .map(|((id, _), label)| TASKDIALOG_BUTTON { nButtonID: *id, pszButtonText: label.as_ptr() })
            .collect();
        let mut config = TASKDIALOGCONFIG {
            cbSize: std::mem::size_of::<TASKDIALOGCONFIG>() as u32,
            hwndParent: hwnd as HWND,
            dwFlags: TDF_ALLOW_DIALOG_CANCELLATION | TDF_POSITION_RELATIVE_TO_WINDOW,
            pszWindowTitle: title.as_ptr(),
            pszMainInstruction: heading.as_ptr(),
            pszContent: text.as_ptr(),
            cButtons: items.len() as u32,
            pButtons: items.as_ptr(),
            nDefaultButton: default,
            pfCallback: Some(bring_forward),
            ..Default::default()
        };
        config.Anonymous1.pszMainIcon = icon as *const u16;
        let mut pressed = 0;
        let result = unsafe { show(&config, &mut pressed, std::ptr::null_mut(), std::ptr::null_mut()) };
        (result == S_OK).then_some(pressed)
    }

    /// Asks a question (the safe answer is the default). A system dialog:
    /// the web page can neither draw it nor press it.
    pub fn ask(hwnd: isize, question: &Question) -> bool {
        let buttons = [(ID_YES, question.yes), (ID_NO, question.no)];
        match run_dialog(hwnd, question.title, question.heading, question.text, TD_WARNING_ICON, &buttons, ID_NO) {
            Some(pressed) => pressed == ID_YES,
            None => {
                let text = format!("{}\n\n{}", question.heading, question.text);
                let flags = MB_YESNO | MB_ICONWARNING | MB_DEFBUTTON2 | MB_TOPMOST | MB_SETFOREGROUND;
                unsafe { MessageBoxW(hwnd as HWND, wide(text).as_ptr(), wide(question.title).as_ptr(), flags) == IDYES }
            }
        }
    }

    pub fn inform(hwnd: isize, title: &str, heading: &str, text: &str) {
        let ok = t!("common.ok");
        if run_dialog(hwnd, title, heading, text, TD_INFORMATION_ICON, &[(ID_YES, &ok)], ID_YES).is_none() {
            let text = format!("{heading}\n\n{text}");
            let flags = MB_OK | MB_ICONINFORMATION | MB_TOPMOST | MB_SETFOREGROUND;
            unsafe { MessageBoxW(hwnd as HWND, wide(text).as_ptr(), wide(title).as_ptr(), flags) };
        }
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
pub fn ask(_: isize, question: &Question) -> bool {
    let _ = (question.title, question.heading, question.text, question.yes, question.no);
    false
}

#[cfg(not(windows))]
pub fn inform(_: isize, _: &str, _: &str, _: &str) {}
