//! Vigilante de "siempre visible" para juegos y apps a pantalla completa.
//!
//! `HWND_TOPMOST` solo pone la ventana por encima de las ventanas *normales*.
//! Muchos juegos en "pantalla completa sin bordes" también se marcan como
//! topmost, y entre dos ventanas topmost Windows deja arriba la que se activó
//! al último: al hacer clic en el juego, el reproductor queda detrás.
//!
//! El vigilante vuelve a subir el reproductor cuando **la ventana activa
//! cubre todo su monitor y está por encima de él**. Se revisa al instante
//! cuando cambia la ventana activa (gancho `EVENT_SYSTEM_FOREGROUND`), unas
//! cuantas veces seguidas después (los juegos suelen cambiar de modo justo
//! tras activarse) y una vez por segundo mientras está activo.
//!
//! Lo que deliberadamente NO hace:
//! - Subirse sobre el shell (Inicio, Alt+Tab, recortes, bloqueo...), sobre
//!   herramientas de captura ni sobre capas transparentes (overlays).
//! - Insistir con un juego que se minimiza al taparlo: eso indica pantalla
//!   completa *exclusiva*, que ninguna ventana normal puede cubrir.
//! - Robar el foco: siempre usa `SWP_NOACTIVATE`.

use crate::platform::Rect;

/// Procesos del sistema y herramientas cuyas ventanas a pantalla completa
/// nunca hay que tapar (en minúsculas, sin ruta).
const NEVER_COVER: &[&str] = &[
    "explorer.exe",
    "shellexperiencehost.exe",
    "startmenuexperiencehost.exe",
    "searchhost.exe",
    "searchapp.exe",
    "screenclippinghost.exe",
    "snippingtool.exe",
    "lockapp.exe",
    "logonui.exe",
    "textinputhost.exe",
    "shellhost.exe",
    "dwm.exe",
    "magnify.exe",
    "sharex.exe",
    "greenshot.exe",
    "lightshot.exe",
    "flameshot.exe",
];

/// Lo que el vigilante sabe de la ventana activa.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Foreground<'a> {
    pub rect: Rect,
    pub ex_style: u32,
    pub cloaked: bool,
    /// ¿Está por encima del reproductor en el orden Z?
    pub above_us: bool,
    /// Nombre del ejecutable, o vacío si no se pudo leer.
    pub exe: &'a str,
}

const WS_EX_TRANSPARENT: u32 = 0x20;
const WS_EX_TOOLWINDOW: u32 = 0x80;
const WS_EX_LAYERED: u32 = 0x80000;
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;

/// ¿La ventana cubre el monitor entero? (Una ventana maximizada normal no:
/// deja libre la barra de tareas.)
pub fn covers(rect: Rect, monitor: Rect) -> bool {
    rect.left <= monitor.left
        && rect.top <= monitor.top
        && rect.right >= monitor.right
        && rect.bottom >= monitor.bottom
}

/// Capas flotantes (overlays de GPU, capturadores, etc.): no son juegos.
pub fn is_overlay(ex_style: u32) -> bool {
    ex_style & WS_EX_TOOLWINDOW != 0
        || ex_style & WS_EX_NOACTIVATE != 0
        || (ex_style & WS_EX_LAYERED != 0 && ex_style & WS_EX_TRANSPARENT != 0)
}

pub fn is_protected_process(exe: &str) -> bool {
    let exe = exe.to_ascii_lowercase();
    // PowerToys tiene varias superposiciones a pantalla completa (regla,
    // selector de color, editor de FancyZones...).
    exe.starts_with("powertoys") || exe.ends_with(".scr") || NEVER_COVER.contains(&exe.as_str())
}

/// Decisión final: ¿hay que volver a subir el reproductor?
pub fn should_raise(fg: &Foreground, monitor: Rect) -> bool {
    fg.above_us
        && !fg.cloaked
        && covers(fg.rect, monitor)
        && !is_overlay(fg.ex_style)
        && !is_protected_process(fg.exe)
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(windows)]
pub use win::{set_active, start};

#[cfg(windows)]
mod win {
    use std::{
        ptr::null_mut,
        sync::{
            atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering},
            Mutex, OnceLock,
        },
        thread::{self, Thread},
        time::{Duration, Instant},
    };

    use windows_sys::Win32::{
        Foundation::{CloseHandle, HWND, RECT},
        Graphics::{
            Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED},
            Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST},
        },
        System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION},
        UI::{
            Accessibility::{SetWinEventHook, HWINEVENTHOOK},
            WindowsAndMessaging::{
                GetForegroundWindow, GetWindow, GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId,
                IsIconic, IsWindow, IsWindowVisible, SetWindowPos, EVENT_SYSTEM_FOREGROUND, GWL_EXSTYLE,
                GW_HWNDPREV, HWND_TOPMOST, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOOWNERZORDER,
                SWP_NOSIZE, WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS,
            },
        },
    };

    use super::{should_raise, Foreground};
    use crate::platform::Rect;

    /// Revisiones seguidas tras un cambio de ventana activa.
    const BURST_CHECKS: u32 = 4;
    const BURST_INTERVAL: Duration = Duration::from_millis(200);
    const IDLE_INTERVAL: Duration = Duration::from_secs(1);
    /// Mientras hay una app a pantalla completa delante, se revisa más a
    /// menudo: algunas se reafirman como topmost sin cambiar de foco.
    const FULLSCREEN_INTERVAL: Duration = Duration::from_millis(250);
    /// Si el juego se minimiza en este plazo tras taparlo, es exclusivo.
    const EXCLUSIVE_WINDOW: Duration = Duration::from_secs(3);

    static OURS: AtomicIsize = AtomicIsize::new(0);
    static ACTIVE: AtomicBool = AtomicBool::new(false);
    static BURST: AtomicU32 = AtomicU32::new(0);
    static WORKER: OnceLock<Thread> = OnceLock::new();
    static STATE: Mutex<State> = Mutex::new(State { skip: Vec::new(), last_raise: None });

    struct State {
        /// Ventanas en pantalla completa exclusiva: no se insiste con ellas.
        skip: Vec<isize>,
        last_raise: Option<(isize, Instant)>,
    }

    /// Arranca el vigilante para la ventana `hwnd`. Llamar desde el hilo de
    /// la interfaz: el gancho entrega sus avisos en el bucle de mensajes de
    /// ese hilo.
    pub fn start(hwnd: isize) {
        OURS.store(hwnd, Ordering::Relaxed);
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
        if let Ok(handle) = thread::Builder::new().name("topmost-guard".into()).spawn(worker) {
            let _ = WORKER.set(handle.thread().clone());
        }
    }

    /// Activo = "siempre visible" encendido y ventana a la vista.
    pub fn set_active(active: bool) {
        ACTIVE.store(active, Ordering::Relaxed);
        if active {
            wake();
        }
    }

    fn wake() {
        BURST.store(BURST_CHECKS, Ordering::Relaxed);
        if let Some(worker) = WORKER.get() {
            worker.unpark();
        }
    }

    unsafe extern "system" fn on_foreground(
        _hook: HWINEVENTHOOK,
        _event: u32,
        _hwnd: HWND,
        id_object: i32,
        _id_child: i32,
        _thread: u32,
        _time: u32,
    ) {
        // OBJID_WINDOW = 0: el aviso es sobre la ventana, no sobre un objeto.
        if id_object == 0 && ACTIVE.load(Ordering::Relaxed) {
            wake();
        }
    }

    fn worker() {
        loop {
            if !ACTIVE.load(Ordering::Relaxed) {
                // Sin coste mientras "siempre visible" está apagado.
                thread::park();
                continue;
            }
            let fullscreen_in_front = unsafe { check() };
            let burst = BURST.load(Ordering::Relaxed);
            if burst > 0 {
                BURST.store(burst - 1, Ordering::Relaxed);
                thread::park_timeout(BURST_INTERVAL);
            } else if fullscreen_in_front {
                thread::park_timeout(FULLSCREEN_INTERVAL);
            } else {
                thread::park_timeout(IDLE_INTERVAL);
            }
        }
    }

    /// Revisa y, si hace falta, sube el reproductor. Devuelve si la ventana
    /// activa es una app a pantalla completa en nuestro monitor.
    unsafe fn check() -> bool {
        let ours = OURS.load(Ordering::Relaxed) as HWND;
        if ours.is_null() || IsWindowVisible(ours) == 0 || IsIconic(ours) != 0 {
            return false;
        }
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());

        // Olvida ventanas que ya no existen.
        state.skip.retain(|&h| IsWindow(h as HWND) != 0);
        // ¿El último juego que tapamos se minimizó enseguida? Es pantalla
        // completa exclusiva: insistir lo minimizaría una y otra vez.
        if let Some((hwnd, at)) = state.last_raise {
            if at.elapsed() < EXCLUSIVE_WINDOW && IsIconic(hwnd as HWND) != 0 {
                if state.skip.len() < 32 {
                    state.skip.push(hwnd);
                }
                state.last_raise = None;
            } else if at.elapsed() >= EXCLUSIVE_WINDOW {
                state.last_raise = None;
            }
        }

        let fg = GetForegroundWindow();
        if fg.is_null() || fg == ours || state.skip.contains(&(fg as isize)) {
            return false;
        }
        let (Some(monitor), Some(rect)) = (monitor_rect(ours), window_rect(fg)) else { return false };
        // Comprobaciones baratas primero; el nombre del proceso, al final.
        if !super::covers(rect, monitor) {
            return false;
        }
        if !is_above(fg, ours) {
            return true;
        }
        let exe = process_name(fg);
        let foreground = Foreground {
            rect,
            ex_style: GetWindowLongPtrW(fg, GWL_EXSTYLE) as u32,
            cloaked: is_cloaked(fg),
            above_us: true,
            exe: &exe,
        };
        if should_raise(&foreground, monitor) {
            SetWindowPos(
                ours,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS,
            );
            state.last_raise = Some((fg as isize, Instant::now()));
        }
        true
    }

    unsafe fn monitor_rect(hwnd: HWND) -> Option<Rect> {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        (GetMonitorInfoW(monitor, &mut info) != 0).then(|| to_rect(info.rcMonitor))
    }

    unsafe fn window_rect(hwnd: HWND) -> Option<Rect> {
        let mut rect: RECT = std::mem::zeroed();
        (GetWindowRect(hwnd, &mut rect) != 0).then(|| to_rect(rect))
    }

    fn to_rect(r: RECT) -> Rect {
        Rect { left: r.left, top: r.top, right: r.right, bottom: r.bottom }
    }

    /// ¿`other` está por encima de `ours` en el orden Z?
    unsafe fn is_above(other: HWND, ours: HWND) -> bool {
        let mut hwnd = GetWindow(ours, GW_HWNDPREV);
        // Límite por si el orden Z cambia mientras se recorre.
        for _ in 0..4096 {
            if hwnd.is_null() {
                return false;
            }
            if hwnd == other {
                return true;
            }
            hwnd = GetWindow(hwnd, GW_HWNDPREV);
        }
        false
    }

    /// Ventanas "ocultas" por DWM (apps UWP suspendidas, otros escritorios).
    unsafe fn is_cloaked(hwnd: HWND) -> bool {
        let mut cloaked: u32 = 0;
        DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED as u32,
            (&mut cloaked as *mut u32).cast(),
            std::mem::size_of::<u32>() as u32,
        ) == 0
            && cloaked != 0
    }

    /// Nombre del ejecutable (`juego.exe`) o vacío si no se puede leer.
    unsafe fn process_name(hwnd: HWND) -> String {
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid == 0 {
            return String::new();
        }
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if process.is_null() {
            return String::new();
        }
        let mut buffer = [0u16; 1024];
        let mut len = buffer.len() as u32;
        let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buffer.as_mut_ptr(), &mut len) != 0;
        CloseHandle(process);
        if !ok {
            return String::new();
        }
        let path = String::from_utf16_lossy(&buffer[..len as usize]);
        path.rsplit(['\\', '/']).next().unwrap_or_default().to_owned()
    }
}

#[cfg(not(windows))]
pub fn start(_: isize) {}

#[cfg(not(windows))]
pub fn set_active(_: bool) {}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITOR: Rect = Rect { left: 0, top: 0, right: 1920, bottom: 1080 };
    const SECOND: Rect = Rect { left: 1920, top: 0, right: 4480, bottom: 1440 };

    fn game(rect: Rect) -> Foreground<'static> {
        Foreground { rect, ex_style: 0, cloaked: false, above_us: true, exe: "Game-Win64-Shipping.exe" }
    }

    #[test]
    fn raises_over_borderless_game_above_us() {
        assert!(should_raise(&game(MONITOR), MONITOR));
        // Algunos juegos se pasan unos píxeles del monitor.
        let oversized = Rect { left: -8, top: -8, right: 1928, bottom: 1088 };
        assert!(should_raise(&game(oversized), MONITOR));
        assert!(should_raise(&game(SECOND), SECOND));
    }

    #[test]
    fn ignores_windows_that_do_not_cover_the_monitor() {
        // Maximizada normal: deja libre la barra de tareas.
        let maximized = Rect { left: -8, top: -8, right: 1928, bottom: 1040 };
        assert!(!should_raise(&game(maximized), MONITOR));
        // A pantalla completa, pero en el otro monitor.
        assert!(!should_raise(&game(SECOND), MONITOR));
        let small = Rect { left: 100, top: 100, right: 900, bottom: 700 };
        assert!(!should_raise(&game(small), MONITOR));
    }

    #[test]
    fn does_nothing_if_already_on_top_or_hidden_window() {
        assert!(!should_raise(&Foreground { above_us: false, ..game(MONITOR) }, MONITOR));
        assert!(!should_raise(&Foreground { cloaked: true, ..game(MONITOR) }, MONITOR));
    }

    #[test]
    fn never_covers_shell_or_capture_tools() {
        for exe in [
            "explorer.exe",
            "ScreenClippingHost.exe",
            "SnippingTool.exe",
            "StartMenuExperienceHost.exe",
            "LockApp.exe",
            "ShareX.exe",
            "PowerToys.ScreenRuler.exe",
            "PowerToys.ColorPickerUI.exe",
            "Mystify.scr",
        ] {
            assert!(!should_raise(&Foreground { exe, ..game(MONITOR) }, MONITOR), "{exe}");
        }
        // Si no se pudo leer el nombre, se trata como un juego.
        assert!(should_raise(&Foreground { exe: "", ..game(MONITOR) }, MONITOR));
    }

    #[test]
    fn never_covers_overlays() {
        for ex_style in [WS_EX_TOOLWINDOW, WS_EX_NOACTIVATE, WS_EX_LAYERED | WS_EX_TRANSPARENT] {
            assert!(!should_raise(&Foreground { ex_style, ..game(MONITOR) }, MONITOR), "{ex_style:#x}");
        }
        // Capa sin "clic a través" (p. ej. un juego con WS_EX_LAYERED): sí.
        assert!(should_raise(&Foreground { ex_style: WS_EX_LAYERED, ..game(MONITOR) }, MONITOR));
    }
}
