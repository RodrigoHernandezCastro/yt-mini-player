//! Funciones de sistema (Win32). Fuera de Windows hay versiones vacías para
//! poder compilar y probar la lógica pura.

pub const APP_NAME: &str = "YT Mini Player";
const WEBVIEW2_DOWNLOAD_URL: &str = "https://developer.microsoft.com/microsoft-edge/webview2/";

/// Rectángulo en píxeles físicos: izquierda, arriba, derecha, abajo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Resultado de reclamar la instancia única.
pub enum Instance {
    /// Somos la primera instancia.
    Primary(ActivationListener),
    /// Ya había otra; se le pidió que se muestre y debemos salir.
    Secondary,
}

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
mod win {
    use std::{
        ffi::OsStr,
        io,
        os::windows::ffi::OsStrExt,
        path::Path,
        ptr::{null, null_mut},
        thread,
        time::Duration,
    };

    use windows_sys::Win32::{
        Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, HANDLE, HWND},
        Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST},
        System::{
            DataExchange::{CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard},
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
            Ole::CF_UNICODETEXT,
            Registry::{
                RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ,
                RRF_RT_REG_SZ,
            },
            Threading::{CreateEventW, SetEvent, WaitForSingleObject, INFINITE},
        },
        UI::{
            Shell::ShellExecuteW,
            WindowsAndMessaging::{
                GetWindowLongPtrW, MessageBoxW, SetLayeredWindowAttributes, SetWindowLongPtrW,
                GWL_EXSTYLE, IDYES, LWA_ALPHA, MB_ICONERROR, MB_ICONWARNING, MB_YESNO,
                SW_SHOWNORMAL, WS_EX_LAYERED, WS_EX_TRANSPARENT,
            },
        },
    };

    use super::{Instance, Rect, APP_NAME, WEBVIEW2_DOWNLOAD_URL};

    const ACTIVATE_EVENT: &str = "Local\\YtMiniPlayer.Activate";
    const RUN_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Run";

    fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
        s.as_ref().encode_wide().chain(std::iter::once(0)).collect()
    }

    pub fn show_error(message: &str) {
        let (text, caption) = (wide(message), wide(APP_NAME));
        unsafe { MessageBoxW(null_mut(), text.as_ptr(), caption.as_ptr(), MB_ICONERROR) };
    }

    pub fn prompt_install_webview2() {
        let text = wide(
            "No se encontró el runtime de Microsoft Edge WebView2, necesario \
             para reproducir YouTube.\n\n¿Abrir la página de descarga?",
        );
        let caption = wide(APP_NAME);
        let answer = unsafe {
            MessageBoxW(null_mut(), text.as_ptr(), caption.as_ptr(), MB_YESNO | MB_ICONWARNING)
        };
        if answer == IDYES {
            open(OsStr::new(WEBVIEW2_DOWNLOAD_URL));
        }
    }

    fn open(target: &OsStr) {
        let (verb, target) = (wide("open"), wide(target));
        unsafe {
            ShellExecuteW(null_mut(), verb.as_ptr(), target.as_ptr(), null(), null(), SW_SHOWNORMAL)
        };
    }

    /// Abre una carpeta en el Explorador.
    pub fn open_path(path: &Path) {
        open(path.as_os_str());
    }

    /// Texto del portapapeles, si lo hay.
    pub fn read_clipboard_text() -> Option<String> {
        unsafe {
            if IsClipboardFormatAvailable(CF_UNICODETEXT as u32) == 0 {
                return None;
            }
            // Otro programa puede tener el portapapeles abierto un instante.
            let mut opened = false;
            for _ in 0..5 {
                if OpenClipboard(null_mut()) != 0 {
                    opened = true;
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
            if !opened {
                return None;
            }
            let text = (|| {
                let handle = GetClipboardData(CF_UNICODETEXT as u32);
                if handle.is_null() {
                    return None;
                }
                let ptr = GlobalLock(handle) as *const u16;
                if ptr.is_null() {
                    return None;
                }
                // Se limita a lo que mide el bloque, por si no hay NUL.
                let max = GlobalSize(handle) / 2;
                let len = (0..max).take_while(|&i| *ptr.add(i) != 0).count();
                let text = String::from_utf16_lossy(std::slice::from_raw_parts(ptr, len));
                GlobalUnlock(handle);
                Some(text)
            })();
            CloseClipboard();
            text
        }
    }

    /// Aplica opacidad y "clic a través" al HWND de nivel superior.
    ///
    /// tao reescribe `GWL_EXSTYLE` completo cada vez que cambia una de sus
    /// banderas (siempre visible, pantalla completa...), así que esto debe
    /// llamarse de nuevo después de cada una de esas operaciones.
    pub fn apply_window_effects(hwnd: isize, opacity_percent: u8, click_through: bool) {
        let hwnd = hwnd as HWND;
        let translucent = opacity_percent < 100;
        unsafe {
            let mut style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32;
            let before = style;
            if translucent || click_through {
                style |= WS_EX_LAYERED;
            } else {
                style &= !WS_EX_LAYERED;
            }
            if click_through {
                style |= WS_EX_TRANSPARENT;
            } else {
                style &= !WS_EX_TRANSPARENT;
            }
            if style != before {
                SetWindowLongPtrW(hwnd, GWL_EXSTYLE, style as isize);
            }
            // Una ventana con WS_EX_LAYERED es invisible hasta que se le
            // asignan atributos: hay que llamarlo siempre que esté activo.
            if style & WS_EX_LAYERED != 0 {
                let alpha = (u32::from(opacity_percent.min(100)) * 255 / 100) as u8;
                SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA);
            }
        }
    }

    /// Área de trabajo (sin barra de tareas) del monitor de la ventana.
    pub fn work_area(hwnd: isize) -> Option<Rect> {
        unsafe {
            let monitor = MonitorFromWindow(hwnd as HWND, MONITOR_DEFAULTTONEAREST);
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return None;
            }
            let r = info.rcWork;
            Some(Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom })
        }
    }

    /// Escucha peticiones de "muéstrate" de instancias posteriores.
    pub struct ActivationListener(isize);

    impl ActivationListener {
        /// Llama a `on_activate` (desde otro hilo) cada vez que alguien abre
        /// el programa estando ya abierto.
        pub fn listen(self, on_activate: impl Fn() + Send + 'static) {
            let handle = self.0;
            if handle == 0 {
                return;
            }
            thread::spawn(move || loop {
                if unsafe { WaitForSingleObject(handle as HANDLE, INFINITE) } != 0 {
                    break;
                }
                on_activate();
            });
        }
    }

    /// Reclama la instancia única. Si ya hay una abierta, le pide que se
    /// muestre y devuelve `Secondary`.
    pub fn claim_instance() -> Instance {
        let name = wide(ACTIVATE_EVENT);
        let handle = unsafe { CreateEventW(null(), 0, 0, name.as_ptr()) };
        if handle.is_null() {
            // No se pudo crear el evento: se sigue sin instancia única.
            return Instance::Primary(ActivationListener(0));
        }
        if unsafe { GetLastError() } != ERROR_ALREADY_EXISTS {
            return Instance::Primary(ActivationListener(handle as isize));
        }
        unsafe {
            SetEvent(handle);
            CloseHandle(handle);
        }
        Instance::Secondary
    }

    /// ¿Está registrado el inicio con Windows?
    pub fn autostart_enabled() -> bool {
        let (key, value) = (wide(RUN_KEY), wide(APP_NAME));
        let mut size = 0u32;
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                null_mut(),
                null_mut(),
                &mut size,
            )
        };
        status == ERROR_SUCCESS
    }

    /// Registra o quita el inicio con Windows (arranca oculto en la bandeja).
    pub fn set_autostart(enabled: bool) -> io::Result<()> {
        let (key, value) = (wide(RUN_KEY), wide(APP_NAME));
        let status = unsafe {
            if enabled {
                let exe = std::env::current_exe()?;
                let command = wide(format!("\"{}\" --hidden", exe.display()));
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    key.as_ptr(),
                    value.as_ptr(),
                    REG_SZ,
                    command.as_ptr().cast(),
                    (command.len() * 2) as u32,
                )
            } else {
                let status = RegDeleteKeyValueW(HKEY_CURRENT_USER, key.as_ptr(), value.as_ptr());
                // Si no existía, ya está desactivado.
                if status == windows_sys::Win32::Foundation::ERROR_FILE_NOT_FOUND {
                    ERROR_SUCCESS
                } else {
                    status
                }
            }
        };
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(status as i32))
        }
    }
}

#[cfg(not(windows))]
pub use stub::*;

#[cfg(not(windows))]
mod stub {
    use std::{io, path::Path};

    use super::*;

    pub struct ActivationListener;
    impl ActivationListener {
        pub fn listen(self, _: impl Fn() + Send + 'static) {}
    }

    pub fn show_error(message: &str) {
        eprintln!("{APP_NAME}: {message}");
    }
    pub fn prompt_install_webview2() {
        eprintln!("{APP_NAME}: falta el runtime de WebView2 ({WEBVIEW2_DOWNLOAD_URL})");
    }
    pub fn open_path(_: &Path) {}
    pub fn read_clipboard_text() -> Option<String> {
        None
    }
    pub fn apply_window_effects(_: isize, _: u8, _: bool) {}
    pub fn work_area(_: isize) -> Option<Rect> {
        None
    }
    pub fn claim_instance() -> Instance {
        Instance::Primary(ActivationListener)
    }
    pub fn autostart_enabled() -> bool {
        false
    }
    pub fn set_autostart(_: bool) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "solo en Windows"))
    }
}

/// Posición (esquina superior izquierda) para colocar una ventana de
/// `size` en la esquina indicada del área de trabajo, con `margin`.
pub fn corner_position(
    area: Rect,
    size: (i32, i32),
    margin: i32,
    corner: crate::ipc::Corner,
) -> (i32, i32) {
    use crate::ipc::Corner::*;
    let x = match corner {
        TopLeft | BottomLeft => area.left + margin,
        TopRight | BottomRight => area.right - size.0 - margin,
    };
    let y = match corner {
        TopLeft | TopRight => area.top + margin,
        BottomLeft | BottomRight => area.bottom - size.1 - margin,
    };
    // Si la ventana no cabe, se alinea al borde superior izquierdo.
    (x.max(area.left), y.max(area.top))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::Corner;

    const AREA: Rect = Rect { left: 0, top: 0, right: 1920, bottom: 1040 };

    #[test]
    fn corners() {
        assert_eq!(corner_position(AREA, (640, 360), 16, Corner::TopLeft), (16, 16));
        assert_eq!(corner_position(AREA, (640, 360), 16, Corner::TopRight), (1264, 16));
        assert_eq!(corner_position(AREA, (640, 360), 16, Corner::BottomLeft), (16, 664));
        assert_eq!(corner_position(AREA, (640, 360), 16, Corner::BottomRight), (1264, 664));
    }

    #[test]
    fn corners_on_secondary_monitor_and_oversized() {
        let left = Rect { left: -1920, top: 0, right: 0, bottom: 1080 };
        assert_eq!(corner_position(left, (640, 360), 0, Corner::BottomRight), (-640, 720));
        assert_eq!(corner_position(AREA, (4000, 3000), 16, Corner::BottomRight), (0, 0));
    }
}
