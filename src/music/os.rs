//! Lo que la música necesita de Windows: elegir carpeta, papelera,
//! "Mostrar en la carpeta" y espacio libre.

use std::path::{Path, PathBuf};

#[cfg(windows)]
mod win {
    use std::{
        os::windows::ffi::OsStrExt,
        path::{Path, PathBuf},
        ptr::{null, null_mut},
    };

    use windows_sys::Win32::{
        Storage::FileSystem::GetDiskFreeSpaceExW,
        System::Com::{CoInitializeEx, COINIT_APARTMENTTHREADED},
        UI::Shell::{
            ILCreateFromPathW, ILFree, SHFileOperationW, SHOpenFolderAndSelectItems, FOF_ALLOWUNDO, FOF_NOCONFIRMATION,
            FOF_NOERRORUI, FOF_SILENT, FO_DELETE, SHFILEOPSTRUCTW,
        },
    };

    fn wide(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain([0]).collect()
    }

    /// Hilos que usan el shell: COM de un solo hilo (lo que espera).
    pub fn init_thread() {
        unsafe {
            CoInitializeEx(null(), COINIT_APARTMENTTHREADED as u32);
        }
    }

    /// Envía archivos a la Papelera (se pueden recuperar).
    pub fn to_recycle_bin(paths: &[PathBuf]) -> bool {
        if paths.is_empty() {
            return true;
        }
        // Lista terminada en doble NUL.
        let mut from: Vec<u16> = Vec::new();
        for p in paths {
            from.extend(p.as_os_str().encode_wide());
            from.push(0);
        }
        from.push(0);
        let mut op = SHFILEOPSTRUCTW {
            hwnd: null_mut(),
            wFunc: FO_DELETE,
            pFrom: from.as_ptr(),
            pTo: null(),
            fFlags: (FOF_ALLOWUNDO | FOF_NOCONFIRMATION | FOF_SILENT | FOF_NOERRORUI) as u16,
            fAnyOperationsAborted: 0,
            hNameMappings: null_mut(),
            lpszProgressTitle: null(),
        };
        unsafe { SHFileOperationW(&mut op) == 0 && op.fAnyOperationsAborted == 0 }
    }

    /// Abre el Explorador con el archivo seleccionado.
    pub fn reveal(path: &Path) {
        let w = wide(path);
        unsafe {
            let pidl = ILCreateFromPathW(w.as_ptr());
            if pidl.is_null() {
                if let Some(parent) = path.parent() {
                    crate::platform::open_path(parent);
                }
                return;
            }
            SHOpenFolderAndSelectItems(pidl, 0, null(), 0);
            ILFree(pidl);
        }
    }

    pub fn free_space(path: &Path) -> Option<u64> {
        let mut dir = path.to_path_buf();
        while !dir.exists() {
            dir = dir.parent()?.to_path_buf();
        }
        let w = wide(&dir);
        let mut free = 0u64;
        let ok = unsafe { GetDiskFreeSpaceExW(w.as_ptr(), &mut free, null_mut(), null_mut()) };
        (ok != 0).then_some(free)
    }

    /// Diálogo "Elegir carpeta" del sistema (modal sobre `owner`).
    pub fn pick_folder(owner: isize, title: &str) -> Option<PathBuf> {
        use windows::{
            core::HSTRING,
            Win32::{
                Foundation::HWND,
                System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_INPROC_SERVER},
                UI::Shell::{FileOpenDialog, IFileOpenDialog, FOS_FORCEFILESYSTEM, FOS_PATHMUSTEXIST, FOS_PICKFOLDERS, SIGDN_FILESYSPATH},
            },
        };
        unsafe {
            let dialog: IFileOpenDialog = CoCreateInstance(&FileOpenDialog, None, CLSCTX_INPROC_SERVER).ok()?;
            let options = dialog.GetOptions().ok()?;
            dialog.SetOptions(options | FOS_PICKFOLDERS | FOS_FORCEFILESYSTEM | FOS_PATHMUSTEXIST).ok()?;
            let _ = dialog.SetTitle(&HSTRING::from(title));
            dialog.Show(Some(HWND(owner as _))).ok()?;
            let item = dialog.GetResult().ok()?;
            let name = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
            let path = name.to_string().ok();
            CoTaskMemFree(Some(name.0 as _));
            path.map(PathBuf::from)
        }
    }
}

#[cfg(windows)]
pub use win::{free_space, init_thread, pick_folder, reveal, to_recycle_bin};

#[cfg(not(windows))]
pub fn init_thread() {}

/// Fuera de Windows (pruebas): borra de verdad.
#[cfg(not(windows))]
pub fn to_recycle_bin(paths: &[PathBuf]) -> bool {
    paths.iter().all(|p| std::fs::remove_file(p).is_ok())
}

#[cfg(not(windows))]
pub fn reveal(_: &Path) {}

#[cfg(not(windows))]
pub fn free_space(_: &Path) -> Option<u64> {
    None
}

#[cfg(not(windows))]
pub fn pick_folder(_: isize, _: &str) -> Option<PathBuf> {
    None
}

/// Carpeta de música del usuario (`Música`), donde van las descargas.
pub fn music_dir() -> Option<PathBuf> {
    directories::UserDirs::new().and_then(|u| u.audio_dir().map(Path::to_path_buf))
}
