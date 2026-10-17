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
        Security::{GetTokenInformation, TokenUIAccess, TOKEN_QUERY},
        Foundation::{
            CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, HANDLE, HWND, LPARAM, LRESULT, POINT,
            WPARAM,
        },
        Graphics::{
            Dwm::{DwmSetWindowAttribute, DWMWA_BORDER_COLOR},
            Gdi::{
                GetMonitorInfoW, MonitorFromPoint, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
                MONITOR_DEFAULTTOPRIMARY,
            },
        },
        System::{
            DataExchange::{CloseClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard},
            Memory::{GlobalLock, GlobalSize, GlobalUnlock},
            Ole::CF_UNICODETEXT,
            Registry::{
                RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ,
                RRF_RT_REG_SZ,
            },
            LibraryLoader::{GetProcAddress, LoadLibraryW},
            SystemInformation::{GetSystemDirectoryW, GetVersionExW, OSVERSIONINFOW},
            Threading::{CreateEventW, GetCurrentProcess, OpenProcessToken, SetEvent, WaitForSingleObject, INFINITE},
        },
        UI::{
            Shell::{DefSubclassProc, SetWindowSubclass, ShellExecuteW},
            WindowsAndMessaging::{
                GetWindowLongPtrW, MessageBoxW, SetLayeredWindowAttributes, SetWindowLongPtrW, ShowWindow,
                GWL_EXSTYLE, IDYES, LWA_ALPHA, MB_ICONERROR, MB_ICONWARNING, MB_YESNO, MA_NOACTIVATE, SW_HIDE,
                SW_SHOWNORMAL,
                WM_MOUSEACTIVATE, WS_EX_LAYERED,
                WS_EX_TRANSPARENT,
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
             para reproducir los videos.\n\n¿Abrir la página de descarga?",
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

    fn monitor_work_area(monitor: windows_sys::Win32::Graphics::Gdi::HMONITOR) -> Option<Rect> {
        unsafe {
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return None;
            }
            let r = info.rcWork;
            Some(Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom })
        }
    }

    /// Área de trabajo del monitor que contiene el punto (x, y).
    pub fn work_area_at(x: i32, y: i32) -> Option<Rect> {
        monitor_work_area(unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) })
    }

    /// Área de trabajo del monitor principal (donde está la bandeja).
    pub fn primary_work_area() -> Option<Rect> {
        monitor_work_area(unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) })
    }

    /// Que un clic en la ventana (o en el webview que contiene) nunca la
    /// active: la app en primer plano lo sigue estando.
    pub fn never_activate_on_click(hwnd: isize) {
        unsafe extern "system" fn proc(
            hwnd: HWND,
            msg: u32,
            wparam: WPARAM,
            lparam: LPARAM,
            _id: usize,
            _data: usize,
        ) -> LRESULT {
            if msg == WM_MOUSEACTIVATE {
                return MA_NOACTIVATE as LRESULT;
            }
            DefSubclassProc(hwnd, msg, wparam, lparam)
        }
        unsafe { SetWindowSubclass(hwnd as HWND, Some(proc), 0x7974_6d70, 0) };
    }

    pub fn hide_window(hwnd: isize) {
        unsafe { ShowWindow(hwnd as HWND, SW_HIDE) };
    }

    /// Menús nativos (el de la bandeja) en oscuro, a juego con la app.
    ///
    /// Usa las mismas funciones sin documentar de uxtheme que el Explorador
    /// y tao (`SetPreferredAppMode` y `FlushMenuThemes`, ordinales 135 y
    /// 136, desde Windows 10 1903). Si no existen, no hace nada.
    pub fn use_dark_menus() {
        const SET_PREFERRED_APP_MODE: usize = 135;
        const FLUSH_MENU_THEMES: usize = 136;
        const FORCE_DARK: i32 = 2;
        unsafe {
            let mut info: OSVERSIONINFOW = std::mem::zeroed();
            info.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;
            // El manifiesto declara Windows 10, así que la versión es real.
            if GetVersionExW(&mut info) == 0 || info.dwMajorVersion < 10 || info.dwBuildNumber < 18362 {
                return;
            }
            let module = LoadLibraryW(wide("uxtheme.dll").as_ptr());
            if module.is_null() {
                return;
            }
            if let Some(f) = GetProcAddress(module, SET_PREFERRED_APP_MODE as *const u8) {
                let set_preferred_app_mode: unsafe extern "system" fn(i32) -> i32 = std::mem::transmute(f);
                set_preferred_app_mode(FORCE_DARK);
            }
            if let Some(f) = GetProcAddress(module, FLUSH_MENU_THEMES as *const u8) {
                let flush_menu_themes: unsafe extern "system" fn() = std::mem::transmute(f);
                flush_menu_themes();
            }
        }
    }

    /// Colorea el borde fino que Windows 11 dibuja alrededor de la ventana.
    /// Devuelve `false` en Windows 10, que no lo admite (ahí lo pinta la
    /// página).
    pub fn set_border_color(hwnd: isize, (r, g, b): (u8, u8, u8)) -> bool {
        // COLORREF: 0x00BBGGRR.
        let color: u32 = u32::from(r) | (u32::from(g) << 8) | (u32::from(b) << 16);
        unsafe {
            DwmSetWindowAttribute(
                hwnd as HWND,
                DWMWA_BORDER_COLOR as u32,
                (&color as *const u32).cast(),
                std::mem::size_of::<u32>() as u32,
            ) >= 0
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

    /// ¿Corre este proceso con UIAccess (modo de superposición máxima)?
    pub fn has_uiaccess() -> bool {
        unsafe {
            let mut token: HANDLE = null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return false;
            }
            let mut value: u32 = 0;
            let mut len = 0u32;
            let ok = GetTokenInformation(
                token,
                TokenUIAccess,
                (&mut value as *mut u32).cast(),
                std::mem::size_of::<u32>() as u32,
                &mut len,
            ) != 0;
            CloseHandle(token);
            ok && value != 0
        }
    }

    fn system_directory() -> Option<std::path::PathBuf> {
        let mut buffer = [0u16; 260];
        let len = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
        (len > 0 && len < buffer.len()).then(|| std::path::PathBuf::from(String::from_utf16_lossy(&buffer[..len])))
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
                let rundll32 = system_directory().map(|dir| dir.join("rundll32.exe"));
                let command = wide(super::autostart_command(&exe, has_uiaccess(), rundll32.as_deref()));
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
    pub fn has_uiaccess() -> bool {
        false
    }
    pub fn work_area_at(_: i32, _: i32) -> Option<Rect> {
        None
    }
    pub fn primary_work_area() -> Option<Rect> {
        None
    }
    pub fn hide_window(_: isize) {}
    pub fn never_activate_on_click(_: isize) {}
    pub fn use_dark_menus() {}
    pub fn set_border_color(_: isize, _: (u8, u8, u8)) -> bool {
        false
    }
    pub fn set_autostart(_: bool) -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "solo en Windows"))
    }
}

/// Línea de comandos para la clave Run.
///
/// Un `.exe` con UIAccess solo puede arrancarse mediante ShellExecute (el
/// servicio AppInfo valida su firma); la clave Run usa CreateProcess y
/// fallaría con ERROR_ELEVATION_REQUIRED. `ShellExec_RunDLL` de shell32 hace
/// ese ShellExecute sin abrir ninguna consola.
pub fn autostart_command(exe: &std::path::Path, uiaccess: bool, rundll32: Option<&std::path::Path>) -> String {
    match (uiaccess, rundll32) {
        (true, Some(rundll32)) => format!(
            "\"{}\" shell32.dll,ShellExec_RunDLL \"{}\" --hidden",
            rundll32.display(),
            exe.display()
        ),
        _ => format!("\"{}\" --hidden", exe.display()),
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

/// Dónde colocar una ventana emergente de `size` junto a `anchor` (el icono
/// de la bandeja), dentro del área de trabajo `area`. Se adapta a la barra
/// de tareas abajo, arriba, a la izquierda o a la derecha, y al panel de
/// iconos ocultos (anchor dentro del área de trabajo).
pub fn popup_position(area: Rect, anchor: (i32, i32), size: (i32, i32), margin: i32) -> (i32, i32) {
    let (ax, ay) = anchor;
    let (w, h) = size;
    let clamp_x = |x: i32| x.min(area.right - w - margin).max(area.left + margin);
    let clamp_y = |y: i32| y.min(area.bottom - h - margin).max(area.top + margin);

    if ay >= area.bottom {
        (clamp_x(ax - w / 2), area.bottom - h - margin)
    } else if ay < area.top {
        (clamp_x(ax - w / 2), area.top + margin)
    } else if ax < area.left {
        (area.left + margin, clamp_y(ay - h / 2))
    } else if ax >= area.right {
        (area.right - w - margin, clamp_y(ay - h / 2))
    } else {
        // Panel de iconos ocultos: encima del icono.
        (clamp_x(ax - w / 2), clamp_y(ay - h - margin))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::Corner;

    const AREA: Rect = Rect { left: 0, top: 0, right: 1920, bottom: 1040 };

    #[test]
    fn autostart_commands() {
        use std::path::Path;
        let exe = Path::new(r"C:\Program Files\YT Mini Player\yt-mini-player.exe");
        let rundll = Path::new(r"C:\Windows\system32\rundll32.exe");
        assert_eq!(
            autostart_command(exe, false, Some(rundll)),
            r#""C:\Program Files\YT Mini Player\yt-mini-player.exe" --hidden"#
        );
        assert_eq!(
            autostart_command(exe, true, Some(rundll)),
            r#""C:\Windows\system32\rundll32.exe" shell32.dll,ShellExec_RunDLL "C:\Program Files\YT Mini Player\yt-mini-player.exe" --hidden"#
        );
        assert_eq!(autostart_command(exe, true, None), autostart_command(exe, false, None));
    }

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

    // 1920×1080 con la barra de tareas abajo (48 px).
    const TASKBAR_AREA: Rect = Rect { left: 0, top: 0, right: 1920, bottom: 1032 };
    const POPUP: (i32, i32) = (380, 160);

    #[test]
    fn taskbar_at_the_bottom() {
        assert_eq!(popup_position(TASKBAR_AREA, (1700, 1050), POPUP, 12), (1510, 860));
        // Icono muy a la derecha: no se sale de la pantalla.
        assert_eq!(popup_position(TASKBAR_AREA, (1910, 1050), POPUP, 12), (1528, 860));
        // Atajo de teclado (esquina inferior derecha).
        assert_eq!(popup_position(TASKBAR_AREA, (1919, 1032), POPUP, 12), (1528, 860));
    }

    #[test]
    fn taskbar_on_other_sides() {
        let top = Rect { top: 48, bottom: 1080, ..TASKBAR_AREA };
        assert_eq!(popup_position(top, (1700, 20), POPUP, 12), (1510, 60));
        let left = Rect { left: 64, ..TASKBAR_AREA };
        assert_eq!(popup_position(left, (30, 1000), POPUP, 12), (76, 860));
        let right = Rect { right: 1856, ..TASKBAR_AREA };
        assert_eq!(popup_position(right, (1880, 500), POPUP, 12), (1464, 420));
    }

    #[test]
    fn overflow_panel_and_other_monitors() {
        // Icono dentro del panel de iconos ocultos (sobre la barra).
        assert_eq!(popup_position(TASKBAR_AREA, (1500, 950), POPUP, 12), (1310, 778));
        // Monitor a la izquierda, con coordenadas negativas.
        let left_monitor = Rect { left: -1920, top: 0, right: 0, bottom: 1040 };
        assert_eq!(popup_position(left_monitor, (-100, 1060), POPUP, 12), (-392, 868));
    }
}
