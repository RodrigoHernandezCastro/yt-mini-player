//! Win32 de la barra de música: leer la barra de tareas y mantenerse encima.
//!
//! La barra es una ventana propia, siempre encima (topmost), que nunca se
//! activa (WS_EX_NOACTIVATE) y que se coloca sobre la barra de tareas. No se
//! hace hija de la barra de tareas (SetParent entre procesos ata la cola de
//! entrada del Explorador a la nuestra: si un día la app se cuelga, se
//! colgaría también la barra de tareas).
//!
//! La barra de tareas también es topmost y, cuando se hace clic en ella,
//! Windows la sube por encima de las demás. Por eso:
//! - un gancho `EVENT_SYSTEM_FOREGROUND` vuelve a subir la barra al instante
//!   cuando la barra de tareas pasa a primer plano, y
//! - `raise_if_covered` (cada pocos cientos de ms mientras se ve) corrige
//!   cualquier otro caso.
//!
//! Para no tapar los iconos de las apps, un hilo aparte pregunta cada pocos
//! segundos a UI Automation dónde están los botones de la barra de tareas
//! (la de Windows 11 no tiene ventanas Win32 por icono). Si UI Automation
//! no responde, la barra se coloca igual, solo que sin esa información.

use crate::{dock::Taskbar, platform::Rect};

#[cfg(windows)]
pub use win::*;

#[cfg(windows)]
mod win {
    use std::{
        ptr::{null, null_mut},
        sync::{
            atomic::{AtomicBool, AtomicIsize, Ordering},
        },
    };

    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
        Graphics::{
            Dwm::{DwmGetWindowAttribute, DwmSetWindowAttribute, DWMWA_CLOAKED, DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUNDSMALL},
            Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTOPRIMARY},
        },
        UI::{
            Accessibility::{SetWinEventHook, HWINEVENTHOOK},
            HiDpi::GetDpiForWindow,
            Shell::{DefSubclassProc, SetWindowSubclass},
            WindowsAndMessaging::{
                FindWindowExW, FindWindowW, GetClassNameW, GetCursorPos, GetForegroundWindow, GetWindow,
                GetWindowLongPtrW, GetWindowRect, IsWindowVisible, SetWindowLongPtrW, SetWindowPos,
                EVENT_SYSTEM_FOREGROUND, GWL_EXSTYLE, GW_HWNDPREV, HWND_TOPMOST, SWP_FRAMECHANGED, SWP_NOACTIVATE,
                SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, WINEVENT_OUTOFCONTEXT,
                WINEVENT_SKIPOWNPROCESS, WM_NCCALCSIZE, WS_EX_TOOLWINDOW,
            },
        },
    };

    use super::{Rect, Taskbar};

    static BAR: AtomicIsize = AtomicIsize::new(0);
    static HOOKED: AtomicBool = AtomicBool::new(false);

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(Some(0)).collect()
    }

    fn to_rect(r: RECT) -> Rect {
        Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
    }

    pub fn window_rect(hwnd: isize) -> Option<Rect> {
        let mut rect: RECT = unsafe { std::mem::zeroed() };
        (unsafe { GetWindowRect(hwnd as HWND, &mut rect) } != 0).then(|| to_rect(rect))
    }

    fn class_name(hwnd: HWND) -> String {
        let mut buffer = [0u16; 64];
        let len = unsafe { GetClassNameW(hwnd, buffer.as_mut_ptr(), buffer.len() as i32) };
        String::from_utf16_lossy(&buffer[..len.max(0) as usize])
    }

    fn is_taskbar_class(class: &str) -> bool {
        matches!(class, "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
    }

    /// La barra de tareas principal: su ventana y lo que hace falta para
    /// colocar la barra.
    pub fn info() -> Option<(isize, Taskbar)> {
        unsafe {
            let taskbar = FindWindowW(wide("Shell_TrayWnd").as_ptr(), null());
            if taskbar.is_null() || IsWindowVisible(taskbar) == 0 {
                return None;
            }
            let rect = window_rect(taskbar as isize)?;
            let monitor = MonitorFromWindow(taskbar, MONITOR_DEFAULTTOPRIMARY);
            let mut mi: MONITORINFO = std::mem::zeroed();
            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            if GetMonitorInfoW(monitor, &mut mi) == 0 {
                return None;
            }
            let notify = FindWindowExW(taskbar, null_mut(), wide("TrayNotifyWnd").as_ptr(), null());
            let tray = if notify.is_null() || IsWindowVisible(notify) == 0 { None } else { window_rect(notify as isize) };
            let dpi = GetDpiForWindow(taskbar);
            let scale = if dpi == 0 { 1.0 } else { f64::from(dpi) / 96.0 };
            let icons_right = scan::latest()
                .and_then(|items| crate::dock::icons_right(&items, rect, tray.map(|t| t.left), scale));
            let monitor = to_rect(mi.rcMonitor);
            Some((taskbar as isize, Taskbar { rect, monitor, tray, icons_right, scale }))
        }
    }

    /// ¿Hay una app a pantalla completa en primer plano en ese monitor? (La
    /// barra de tareas queda debajo y la barra de música debe esconderse.)
    pub fn fullscreen_in_front(monitor: Rect) -> bool {
        unsafe {
            let fg = GetForegroundWindow();
            if fg.is_null() {
                return false;
            }
            // El escritorio cubre el monitor, pero no tapa la barra de tareas.
            let class = class_name(fg);
            if matches!(class.as_str(), "Progman" | "WorkerW" | "#32769") || is_taskbar_class(&class) {
                return false;
            }
            let Some(rect) = window_rect(fg as isize) else { return false };
            let mut cloaked: u32 = 0;
            let is_cloaked = DwmGetWindowAttribute(
                fg,
                DWMWA_CLOAKED as u32,
                (&mut cloaked as *mut u32).cast(),
                std::mem::size_of::<u32>() as u32,
            ) == 0
                && cloaked != 0;
            !is_cloaked && crate::topmost::covers(rect, monitor)
        }
    }

    /// Sin marco: toda la ventana es área cliente (el webview la llena de
    /// borde a borde). Se intercepta antes que tao.
    unsafe extern "system" fn no_frame(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        if msg == WM_NCCALCSIZE && wparam != 0 {
            // Dejar el rectángulo propuesto tal cual = área cliente completa.
            return 0;
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }

    /// Ajustes de la ventana de la barra (una vez, aún oculta): sin marco,
    /// fuera de Alt+Tab y esquinas pequeñas, como los elementos de la barra
    /// de tareas.
    pub fn prepare(hwnd: isize) {
        unsafe {
            SetWindowSubclass(hwnd as HWND, Some(no_frame), 0x7974_6d62, 0);
            let style = GetWindowLongPtrW(hwnd as HWND, GWL_EXSTYLE);
            SetWindowLongPtrW(hwnd as HWND, GWL_EXSTYLE, style | WS_EX_TOOLWINDOW as isize);
            // Que Windows recalcule el marco con el nuevo cálculo.
            SetWindowPos(
                hwnd as HWND,
                std::ptr::null_mut(),
                0,
                0,
                0,
                0,
                SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
            );
            let preference: i32 = DWMWCP_ROUNDSMALL;
            DwmSetWindowAttribute(
                hwnd as HWND,
                DWMWA_WINDOW_CORNER_PREFERENCE as u32,
                (&preference as *const i32).cast(),
                std::mem::size_of::<i32>() as u32,
            );
        }
    }

    /// La muestra (o mueve) en `rect`, encima de todo y sin activarla.
    pub fn place(hwnd: isize, rect: Rect) {
        unsafe {
            SetWindowPos(
                hwnd as HWND,
                HWND_TOPMOST,
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_NOOWNERZORDER,
            );
        }
    }

    fn raise(hwnd: HWND) {
        unsafe {
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER,
            );
        }
    }

    /// Si la barra de tareas quedó por encima, vuelve a subir la barra.
    pub fn raise_if_covered(hwnd: isize, taskbar: isize) {
        unsafe {
            let ours = hwnd as HWND;
            let mut above = GetWindow(ours, GW_HWNDPREV);
            // Límite por si el orden Z cambia mientras se recorre.
            for _ in 0..4096 {
                if above.is_null() {
                    return;
                }
                if above as isize == taskbar {
                    raise(ours);
                    return;
                }
                above = GetWindow(above, GW_HWNDPREV);
            }
        }
    }

    /// Gancho: en cuanto la barra de tareas pasa a primer plano, la barra de
    /// música vuelve encima. Llamar desde el hilo de la interfaz.
    pub fn keep_above(hwnd: isize) {
        BAR.store(hwnd, Ordering::Relaxed);
        if HOOKED.swap(true, Ordering::Relaxed) {
            return;
        }
        unsafe {
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                null_mut(),
                Some(on_foreground),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
        }
    }

    unsafe extern "system" fn on_foreground(
        _hook: HWINEVENTHOOK,
        _event: u32,
        hwnd: HWND,
        id_object: i32,
        _id_child: i32,
        _thread: u32,
        _time: u32,
    ) {
        let bar = BAR.load(Ordering::Relaxed) as HWND;
        if id_object != 0 || bar.is_null() || IsWindowVisible(bar) == 0 {
            return;
        }
        if is_taskbar_class(&class_name(hwnd)) {
            raise(bar);
        }
    }

    /// Posición horizontal del cursor (px físicos).
    pub fn cursor_x() -> Option<i32> {
        let mut point = POINT { x: 0, y: 0 };
        (unsafe { GetCursorPos(&mut point) } != 0).then_some(point.x)
    }

    pub use scan::set_scanning;

    /// Iconos de la barra de tareas vía UI Automation, en un hilo aparte (las
    /// consultas van al proceso del Explorador y pueden tardar).
    mod scan {
        use std::{
            sync::{
                atomic::{AtomicBool, Ordering},
                Mutex, OnceLock,
            },
            thread::{self, Thread},
            time::{Duration, Instant},
        };

        use windows::Win32::{
            Foundation::HWND,
            System::Com::{CoCreateInstance, CoInitializeEx, CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED},
            UI::Accessibility::{
                CUIAutomation, IUIAutomation, TreeScope_Descendants, UIA_BoundingRectanglePropertyId,
                UIA_ClassNamePropertyId,
            },
        };

        use super::{wide, Rect};

        const INTERVAL: Duration = Duration::from_secs(3);
        /// Un resultado más viejo que esto no se usa.
        const FRESH: Duration = Duration::from_secs(10);
        const MAX_ITEMS: i32 = 400;

        static WANTED: AtomicBool = AtomicBool::new(false);
        static WORKER: OnceLock<Thread> = OnceLock::new();
        /// Elementos de la barra de tareas: (rectángulo, clase).
        type Items = Vec<(Rect, String)>;
        static LATEST: Mutex<Option<(Instant, Items)>> = Mutex::new(None);

        /// Activo mientras la barra se ve; sin coste el resto del tiempo.
        pub fn set_scanning(on: bool) {
            // Solo al encenderlo se despierta el hilo (se llama en cada revisión).
            if WANTED.swap(on, Ordering::Relaxed) || !on {
                return;
            }
            let worker = WORKER.get_or_init(|| {
                thread::Builder::new()
                    .name("taskbar-scan".into())
                    .spawn(worker)
                    .map(|handle| handle.thread().clone())
                    .unwrap_or_else(|_| thread::current())
            });
            worker.unpark();
        }

        pub fn latest() -> Option<Vec<(Rect, String)>> {
            let latest = LATEST.lock().unwrap_or_else(|e| e.into_inner());
            latest.as_ref().filter(|(at, _)| at.elapsed() < FRESH).map(|(_, items)| items.clone())
        }

        fn worker() {
            let automation: Option<IUIAutomation> = unsafe {
                let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
                CoCreateInstance(&CUIAutomation, None, CLSCTX_INPROC_SERVER).ok()
            };
            let Some(automation) = automation else { return };
            loop {
                if !WANTED.load(Ordering::Relaxed) {
                    thread::park();
                    continue;
                }
                if let Ok(items) = unsafe { scan(&automation) } {
                    *LATEST.lock().unwrap_or_else(|e| e.into_inner()) = Some((Instant::now(), items));
                }
                thread::park_timeout(INTERVAL);
            }
        }

        unsafe fn scan(automation: &IUIAutomation) -> windows::core::Result<Vec<(Rect, String)>> {
            use windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW;
            let taskbar = FindWindowW(wide("Shell_TrayWnd").as_ptr(), std::ptr::null());
            if taskbar.is_null() {
                return Ok(Vec::new());
            }
            let root = automation.ElementFromHandle(HWND(taskbar))?;
            let cache = automation.CreateCacheRequest()?;
            cache.AddProperty(UIA_BoundingRectanglePropertyId)?;
            cache.AddProperty(UIA_ClassNamePropertyId)?;
            let found = root.FindAllBuildCache(TreeScope_Descendants, &automation.ControlViewCondition()?, &cache)?;
            let count = found.Length()?.min(MAX_ITEMS);
            let mut items = Vec::with_capacity(count as usize);
            for i in 0..count {
                let element = found.GetElement(i)?;
                let Ok(r) = element.CachedBoundingRectangle() else { continue };
                let class = element.CachedClassName().map(|c| c.to_string()).unwrap_or_default();
                items.push((Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom }, class));
            }
            Ok(items)
        }
    }
}

#[cfg(not(windows))]
mod stub {
    use super::{Rect, Taskbar};

    pub fn window_rect(_: isize) -> Option<Rect> {
        None
    }
    pub fn info() -> Option<(isize, Taskbar)> {
        None
    }
    pub fn fullscreen_in_front(_: Rect) -> bool {
        false
    }
    pub fn prepare(_: isize) {}
    pub fn place(_: isize, _: Rect) {}
    pub fn raise_if_covered(_: isize, _: isize) {}
    pub fn keep_above(_: isize) {}
    pub fn cursor_x() -> Option<i32> {
        None
    }
    pub fn set_scanning(_: bool) {}
}

#[cfg(not(windows))]
pub use stub::*;
