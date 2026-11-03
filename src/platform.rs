//! System functions (Win32). Outside Windows there are empty versions so the
//! pure logic can be compiled and tested.

pub const APP_NAME: &str = "YT Mini Player";
const WEBVIEW2_DOWNLOAD_URL: &str = "https://developer.microsoft.com/microsoft-edge/webview2/";

/// Rectangle in physical pixels: left, top, right, bottom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Result of claiming the single instance.
pub enum Instance {
    /// We are the first instance.
    Primary(ActivationListener),
    /// There was another one; it was asked to show itself and we must quit.
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
            CloseHandle, GetLastError, GlobalFree, ERROR_ALREADY_EXISTS, ERROR_SUCCESS, HANDLE, HWND, LPARAM, LRESULT, POINT,
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
            DataExchange::{
                CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
                SetClipboardData,
            },
            Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE},
            Ole::CF_UNICODETEXT,
            Registry::{
                RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ,
                RRF_RT_REG_SZ,
            },
            LibraryLoader::{GetProcAddress, LoadLibraryW},
            SystemInformation::{GetSystemDirectoryW, GetVersionExW, OSVERSIONINFOW},
            Threading::{
                CreateEventW, GetCurrentProcess, OpenProcess, OpenProcessToken, SetEvent, WaitForSingleObject, INFINITE,
                PROCESS_SYNCHRONIZE,
            },
        },
        UI::{
            Shell::{DefSubclassProc, SetWindowSubclass, ShellExecuteW},
            WindowsAndMessaging::{
                GetWindowLongPtrW, MessageBoxW, SetLayeredWindowAttributes, SetWindowLongPtrW, ShowWindow,
                GWL_EXSTYLE, LWA_ALPHA, MB_ICONERROR, MA_NOACTIVATE, SW_HIDE,
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
        let question = crate::security::Question {
            title: APP_NAME,
            heading: &t!("startup.webview2_title"),
            text: &t!("startup.webview2_text"),
            yes: &t!("startup.webview2_download"),
            no: &t!("startup.webview2_later"),
        };
        if crate::security::ask(0, &question) {
            open(OsStr::new(WEBVIEW2_DOWNLOAD_URL));
        }
    }

    fn open(target: &OsStr) {
        let (verb, target) = (wide("open"), wide(target));
        unsafe {
            ShellExecuteW(null_mut(), verb.as_ptr(), target.as_ptr(), null(), null(), SW_SHOWNORMAL)
        };
    }

    /// Opens a folder in File Explorer.
    pub fn open_path(path: &Path) {
        open(path.as_os_str());
    }

    /// Text on the clipboard, if any.
    pub fn read_clipboard_text() -> Option<String> {
        unsafe {
            if IsClipboardFormatAvailable(CF_UNICODETEXT as u32) == 0 {
                return None;
            }
            // Another program may hold the clipboard open for a moment.
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
                // Limited to the size of the block, in case there is no NUL.
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

    /// Puts text on the clipboard (the watch together code).
    pub fn write_clipboard_text(text: &str) -> bool {
        let units: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        unsafe {
            let mut opened = false;
            for _ in 0..5 {
                if OpenClipboard(null_mut()) != 0 {
                    opened = true;
                    break;
                }
                thread::sleep(Duration::from_millis(20));
            }
            if !opened {
                return false;
            }
            let done = (|| {
                if EmptyClipboard() == 0 {
                    return false;
                }
                let block = GlobalAlloc(GMEM_MOVEABLE, units.len() * 2);
                if block.is_null() {
                    return false;
                }
                let ptr = GlobalLock(block) as *mut u16;
                if ptr.is_null() {
                    GlobalFree(block);
                    return false;
                }
                std::ptr::copy_nonoverlapping(units.as_ptr(), ptr, units.len());
                GlobalUnlock(block);
                // On success the clipboard owns the block.
                if SetClipboardData(CF_UNICODETEXT as u32, block).is_null() {
                    GlobalFree(block);
                    return false;
                }
                true
            })();
            CloseClipboard();
            done
        }
    }

    /// Applies opacity and "click-through" to the top-level HWND.
    ///
    /// tao rewrites the whole `GWL_EXSTYLE` every time one of its flags
    /// changes (always on top, fullscreen...), so this must be called again
    /// after each of those operations.
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
            // A WS_EX_LAYERED window is invisible until it gets attributes:
            // this must be called whenever the style is on.
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

    /// Work area of the monitor that contains the point (x, y).
    pub fn work_area_at(x: i32, y: i32) -> Option<Rect> {
        monitor_work_area(unsafe { MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST) })
    }

    /// Work area of the primary monitor (where the tray is).
    pub fn primary_work_area() -> Option<Rect> {
        monitor_work_area(unsafe { MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY) })
    }

    /// A click on the window (or the webview inside it) never activates it:
    /// the app in the foreground stays there.
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

    /// Native menus (the tray's) in dark mode, to match the app.
    ///
    /// Uses the same undocumented uxtheme functions as File Explorer and tao
    /// (`SetPreferredAppMode` and `FlushMenuThemes`, ordinals 135 and 136,
    /// since Windows 10 1903). If they don't exist, it does nothing.
    pub fn use_dark_menus() {
        const SET_PREFERRED_APP_MODE: usize = 135;
        const FLUSH_MENU_THEMES: usize = 136;
        const FORCE_DARK: i32 = 2;
        unsafe {
            let mut info: OSVERSIONINFOW = std::mem::zeroed();
            info.dwOSVersionInfoSize = std::mem::size_of::<OSVERSIONINFOW>() as u32;
            // The manifest declares Windows 10, so the version is the real one.
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

    /// Colours the thin border Windows 11 draws around the window. Returns
    /// `false` on Windows 10, which does not support it (the page paints it
    /// there).
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

    /// Work area (without the taskbar) of the window's monitor.
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

    /// Listens for "show yourself" requests from later instances.
    pub struct ActivationListener(isize);

    impl ActivationListener {
        /// Calls `on_activate` (from another thread) every time someone opens
        /// the program while it is already running.
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

    /// Claims the single instance. If one is already running, asks it to
    /// show itself and returns `Secondary`.
    /// `suffix`: another instance's name (the test build with its own data
    /// folder, see e2e.rs).
    pub fn claim_instance(suffix: Option<&str>) -> Instance {
        let name = wide(&match suffix {
            Some(suffix) => format!("{ACTIVATE_EVENT}.{suffix}"),
            None => ACTIVATE_EVENT.to_owned(),
        });
        let handle = unsafe { CreateEventW(null(), 0, 0, name.as_ptr()) };
        if handle.is_null() {
            // The event could not be created: carry on without a single instance.
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

    /// Does this process run with UIAccess (maximum overlay mode)?
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

    /// Starts a new copy that waits for this one to quit (a setting that
    /// needs a fresh start). Through ShellExecute, like the Run key: a
    /// UIAccess build can only be started that way.
    pub fn relaunch() -> io::Result<()> {
        let exe = wide(std::env::current_exe()?);
        let args = wide(format!("--wait-pid={}", std::process::id()));
        let result = unsafe { ShellExecuteW(null_mut(), null(), exe.as_ptr(), args.as_ptr(), null(), SW_SHOWNORMAL) };
        if result as isize > 32 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    /// Waits (at most `timeout`) for a process to end.
    pub fn wait_for_exit(pid: u32, timeout: Duration) {
        unsafe {
            let handle = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if handle.is_null() {
                return;
            }
            WaitForSingleObject(handle, timeout.as_millis().min(u128::from(u32::MAX - 1)) as u32);
            CloseHandle(handle);
        }
    }

    /// Is starting with Windows registered?
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

    /// Registers or removes starting with Windows (starts hidden in the tray).
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
                // If it did not exist, it is already off.
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
        eprintln!("{APP_NAME}: the WebView2 runtime is missing ({WEBVIEW2_DOWNLOAD_URL})");
    }
    pub fn open_path(_: &Path) {}
    pub fn read_clipboard_text() -> Option<String> {
        None
    }
    pub fn write_clipboard_text(_: &str) -> bool {
        false
    }
    pub fn apply_window_effects(_: isize, _: u8, _: bool) {}
    pub fn work_area(_: isize) -> Option<Rect> {
        None
    }
    pub fn claim_instance(_: Option<&str>) -> Instance {
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
        Err(io::Error::new(io::ErrorKind::Unsupported, "Windows only"))
    }
    pub fn relaunch() -> io::Result<()> {
        Err(io::Error::new(io::ErrorKind::Unsupported, "Windows only"))
    }
    pub fn wait_for_exit(_: u32, _: std::time::Duration) {}
}

/// Command line for the Run key.
///
/// A UIAccess `.exe` can only be started through ShellExecute (the AppInfo
/// service checks its signature); the Run key uses CreateProcess and would
/// fail with ERROR_ELEVATION_REQUIRED. shell32's `ShellExec_RunDLL` does
/// that ShellExecute without opening any console.
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

/// Position (top-left corner) to place a window of `size` in the given
/// corner of the work area, with `margin`.
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
    // If the window does not fit, it aligns to the top-left edge.
    (x.max(area.left), y.max(area.top))
}

/// Where to place a popup of `size` next to `anchor` (the tray icon),
/// inside the work area `area`. Adapts to a taskbar at the bottom, top,
/// left or right, and to the hidden icons panel (anchor inside the work
/// area).
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
        // Hidden icons panel: above the icon.
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

    // 1920×1080 with the taskbar at the bottom (48 px).
    const TASKBAR_AREA: Rect = Rect { left: 0, top: 0, right: 1920, bottom: 1032 };
    const POPUP: (i32, i32) = (380, 160);

    #[test]
    fn taskbar_at_the_bottom() {
        assert_eq!(popup_position(TASKBAR_AREA, (1700, 1050), POPUP, 12), (1510, 860));
        // Icon far to the right: it stays on the screen.
        assert_eq!(popup_position(TASKBAR_AREA, (1910, 1050), POPUP, 12), (1528, 860));
        // Keyboard shortcut (bottom-right corner).
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
        // Icon inside the hidden icons panel (above the bar).
        assert_eq!(popup_position(TASKBAR_AREA, (1500, 950), POPUP, 12), (1310, 778));
        // Monitor on the left, with negative coordinates.
        let left_monitor = Rect { left: -1920, top: 0, right: 0, bottom: 1040 };
        assert_eq!(popup_position(left_monitor, (-100, 1060), POPUP, 12), (-392, 868));
    }
}
