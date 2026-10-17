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
//! **Modo juego.** Mientras hay un juego (o cualquier app) a pantalla
//! completa en primer plano, un clic en el reproductor NO lo activa
//! (`WS_EX_NOACTIVATE` temporal y `WM_MOUSEACTIVATE` → `MA_NOACTIVATE`): el
//! juego sigue con el foco y el teclado. Muchos
//! juegos (Minecraft y todo lo que usa GLFW, SDL…) se minimizan o se pausan
//! al perder el foco en pantalla completa; así se puede pausar el video o
//! moverse por él sin sacar al juego de su modo.
//!
//! **Reacción inmediata.** Además del gancho de primer plano, uno de
//! `EVENT_OBJECT_REORDER` despierta al vigilante cuando cambia el orden Z
//! con un juego delante (juegos que se reafirman topmost a cada rato), y
//! mientras el juego insiste se revisa cada 15 ms. Entre dos subidas pasan
//! al menos `MIN_RAISE_GAP`, para no entrar en un bucle sin freno.
//!
//! Lo que deliberadamente NO hace:
//! - Subirse sobre el shell (Inicio, Alt+Tab, recortes, bloqueo...), sobre
//!   herramientas de captura ni sobre capas transparentes (overlays).
//! - Insistir con un juego que se minimiza al taparlo: eso indica pantalla
//!   completa *exclusiva*, que ninguna ventana normal puede cubrir. Solo
//!   cuenta si se minimizó *estando delante*: los juegos que se minimizan
//!   al perder el foco (Alt+Tab; Minecraft y GLFW lo hacen) no son
//!   exclusivos y se vuelven a cubrir al volver a ellos.
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

/// Orden de los avisos de Windows: ¿quién era la ventana activa cuando otra
/// empezó a minimizarse? Si era ella misma, se minimizó por su cuenta (no
/// por un Alt+Tab o un clic en otra ventana, que la desactivan antes).
#[derive(Debug, Default)]
pub struct FocusLog {
    foreground: isize,
    minimized_in_front: Option<(isize, std::time::Instant)>,
}

impl FocusLog {
    pub const fn new() -> Self {
        Self { foreground: 0, minimized_in_front: None }
    }

    /// Aviso `EVENT_SYSTEM_FOREGROUND`.
    pub fn foreground(&mut self, hwnd: isize) {
        self.foreground = hwnd;
    }

    /// Aviso `EVENT_SYSTEM_MINIMIZESTART`.
    pub fn minimize_start(&mut self, hwnd: isize, at: std::time::Instant) {
        if hwnd != 0 && hwnd == self.foreground {
            self.minimized_in_front = Some((hwnd, at));
        }
    }

    pub fn take_minimized_in_front(&mut self) -> Option<(isize, std::time::Instant)> {
        self.minimized_in_front.take()
    }
}

/// ¿Se minimizó (estando delante) poco después de taparla? Entonces es
/// pantalla completa exclusiva.
pub fn exclusive(raised: std::time::Instant, minimized: std::time::Instant, window: std::time::Duration) -> bool {
    minimized >= raised && minimized.duration_since(raised) < window
}

/// ¿Es una app que se puede tapar? (No el shell, ni una capa flotante, ni
/// una ventana oculta por DWM.)
pub fn coverable(fg: &Foreground) -> bool {
    !fg.cloaked && !is_overlay(fg.ex_style) && !is_protected_process(fg.exe)
}

/// Decisión final: ¿hay que volver a subir el reproductor? (El vigilante
/// hace lo mismo por partes, guardando `coverable` por ventana.)
#[cfg(test)]
pub fn should_raise(fg: &Foreground, monitor: Rect) -> bool {
    fg.above_us && covers(fg.rect, monitor) && coverable(fg)
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
        Foundation::{CloseHandle, HWND, LPARAM, LRESULT, RECT, WPARAM},
        Graphics::{
            Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED},
            Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST},
        },
        System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION},
        UI::{
            Accessibility::{SetWinEventHook, HWINEVENTHOOK},
            Shell::{DefSubclassProc, SetWindowSubclass},
            WindowsAndMessaging::{
                GetForegroundWindow, GetWindow, GetWindowLongPtrW, GetWindowRect, GetWindowThreadProcessId,
                IsIconic, IsWindow, IsWindowVisible, SetWindowPos, EVENT_OBJECT_REORDER, EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_MINIMIZESTART,
                GWL_EXSTYLE, GW_HWNDPREV, HWND_TOPMOST, MA_NOACTIVATE, SWP_ASYNCWINDOWPOS, SWP_NOACTIVATE,
                SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SetWindowLongPtrW, WINEVENT_OUTOFCONTEXT,
                WINEVENT_SKIPOWNPROCESS, WM_MOUSEACTIVATE, WS_EX_NOACTIVATE,
            },
        },
    };

    use super::{FocusLog, Foreground};
    use crate::platform::Rect;

    /// Revisiones seguidas tras un cambio de ventana activa.
    const BURST_CHECKS: u32 = 4;
    const BURST_INTERVAL: Duration = Duration::from_millis(200);
    const IDLE_INTERVAL: Duration = Duration::from_secs(1);
    /// Mientras hay una app a pantalla completa delante, se revisa más a
    /// menudo: algunas se reafirman como topmost sin cambiar de foco.
    const FULLSCREEN_INTERVAL: Duration = Duration::from_millis(100);
    /// Si el juego nos volvió a tapar hace poco (se reafirma topmost a cada
    /// rato), aún más a menudo: es barato (sin abrir procesos).
    const FIGHT_INTERVAL: Duration = Duration::from_millis(15);
    const FIGHT_MEMORY: Duration = Duration::from_secs(1);
    /// Separación mínima entre dos subidas (freno contra bucles).
    const MIN_RAISE_GAP: Duration = Duration::from_millis(15);
    /// Si el juego se minimiza estando delante en este plazo tras taparlo,
    /// es exclusivo.
    const EXCLUSIVE_WINDOW: Duration = Duration::from_secs(3);

    static OURS: AtomicIsize = AtomicIsize::new(0);
    static ACTIVE: AtomicBool = AtomicBool::new(false);
    /// Hay una app a pantalla completa en primer plano (en cualquier monitor).
    static GAME: AtomicBool = AtomicBool::new(false);
    static BURST: AtomicU32 = AtomicU32::new(0);
    /// Va aparte de STATE: los ganchos corren en el hilo de la interfaz y
    /// nunca deben esperar al vigilante (que le envía mensajes).
    static FOCUS: Mutex<FocusLog> = Mutex::new(FocusLog::new());

    enum Poll {
        Idle,
        Fullscreen,
        Fight,
    }
    static WORKER: OnceLock<Thread> = OnceLock::new();
    static STATE: Mutex<State> =
        Mutex::new(State { skip: Vec::new(), last_raise: None, last_any_raise: None, known: None });

    struct State {
        /// Ventanas en pantalla completa exclusiva: no se insiste con ellas.
        skip: Vec<isize>,
        /// Última subida sobre una ventana a pantalla completa.
        last_raise: Option<(isize, Instant)>,
        last_any_raise: Option<Instant>,
        /// Última ventana clasificada: (hwnd, ¿se puede tapar?). Evita abrir
        /// el proceso en cada revisión.
        known: Option<(isize, bool)>,
    }

    /// Entra o sale del modo juego. Con `WS_EX_NOACTIVATE` Windows no activa
    /// la ventana al hacer clic, ni siquiera en el webview (que es de otro
    /// proceso y decide su propia activación).
    /// Se comprueba el estilo cada vez (es barato: no envía mensajes) por si
    /// tao lo recalculó al cambiar otra opción de la ventana.
    fn set_game(on: bool) {
        GAME.store(on, Ordering::Relaxed);
        let ours = OURS.load(Ordering::Relaxed) as HWND;
        if ours.is_null() {
            return;
        }
        unsafe {
            let style = GetWindowLongPtrW(ours, GWL_EXSTYLE);
            let next = if on { style | WS_EX_NOACTIVATE as isize } else { style & !(WS_EX_NOACTIVATE as isize) };
            if next != style {
                SetWindowLongPtrW(ours, GWL_EXSTYLE, next);
            }
        }
    }

    unsafe extern "system" fn no_steal_focus(
        hwnd: HWND,
        msg: u32,
        wparam: WPARAM,
        lparam: LPARAM,
        _id: usize,
        _data: usize,
    ) -> LRESULT {
        if msg == WM_MOUSEACTIVATE && GAME.load(Ordering::Relaxed) {
            return MA_NOACTIVATE as LRESULT;
        }
        DefSubclassProc(hwnd, msg, wparam, lparam)
    }

    /// Arranca el vigilante para la ventana `hwnd`. Llamar desde el hilo de
    /// la interfaz: el gancho entrega sus avisos en el bucle de mensajes de
    /// ese hilo.
    pub fn start(hwnd: isize) {
        OURS.store(hwnd, Ordering::Relaxed);
        unsafe {
            SetWindowSubclass(hwnd as HWND, Some(no_steal_focus), 0x7974_6d67, 0);
            // Sin SKIPOWNPROCESS: también interesa saber cuándo se activa el
            // reproductor (orden de los avisos para detectar exclusivos).
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                null_mut(),
                Some(on_foreground),
                0,
                0,
                WINEVENT_OUTOFCONTEXT,
            );
            SetWinEventHook(
                EVENT_SYSTEM_MINIMIZESTART,
                EVENT_SYSTEM_MINIMIZESTART,
                null_mut(),
                Some(on_minimize),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            );
            SetWinEventHook(
                EVENT_OBJECT_REORDER,
                EVENT_OBJECT_REORDER,
                null_mut(),
                Some(on_reorder),
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
        } else {
            set_game(false);
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
        hwnd: HWND,
        id_object: i32,
        _id_child: i32,
        _thread: u32,
        _time: u32,
    ) {
        // OBJID_WINDOW = 0: el aviso es sobre la ventana, no sobre un objeto.
        if id_object != 0 {
            return;
        }
        FOCUS.lock().unwrap_or_else(|e| e.into_inner()).foreground(hwnd as isize);
        if ACTIVE.load(Ordering::Relaxed) {
            wake();
        }
    }

    /// Una ventana empieza a minimizarse. Si para Windows seguía siendo la
    /// activa (no hubo Alt+Tab ni clic en otra antes), se minimizó por su
    /// cuenta: posible pantalla completa exclusiva.
    unsafe extern "system" fn on_minimize(
        _hook: HWINEVENTHOOK,
        _event: u32,
        hwnd: HWND,
        id_object: i32,
        _id_child: i32,
        _thread: u32,
        _time: u32,
    ) {
        if id_object == 0 {
            FOCUS.lock().unwrap_or_else(|e| e.into_inner()).minimize_start(hwnd as isize, Instant::now());
        }
    }

    /// Cambió el orden Z de alguna ventana: con un juego delante, se revisa
    /// ya (no hace falta esperar a la siguiente vuelta).
    unsafe extern "system" fn on_reorder(
        _hook: HWINEVENTHOOK,
        _event: u32,
        _hwnd: HWND,
        _id_object: i32,
        _id_child: i32,
        _thread: u32,
        _time: u32,
    ) {
        if ACTIVE.load(Ordering::Relaxed) && GAME.load(Ordering::Relaxed) {
            if let Some(worker) = WORKER.get() {
                worker.unpark();
            }
        }
    }

    fn worker() {
        loop {
            if !ACTIVE.load(Ordering::Relaxed) {
                // Sin coste mientras "siempre visible" está apagado.
                set_game(false);
                thread::park();
                continue;
            }
            let poll = unsafe { check() };
            let burst = BURST.load(Ordering::Relaxed);
            let wait = match poll {
                Poll::Fight => FIGHT_INTERVAL,
                _ if burst > 0 => BURST_INTERVAL,
                Poll::Fullscreen => FULLSCREEN_INTERVAL,
                Poll::Idle => IDLE_INTERVAL,
            };
            if burst > 0 {
                BURST.store(burst - 1, Ordering::Relaxed);
            }
            thread::park_timeout(wait);
        }
    }

    /// Revisa y, si hace falta, sube el reproductor. Devuelve cada cuánto
    /// conviene volver a mirar.
    unsafe fn check() -> Poll {
        let ours = OURS.load(Ordering::Relaxed) as HWND;
        if ours.is_null() || IsWindowVisible(ours) == 0 || IsIconic(ours) != 0 {
            set_game(false);
            return Poll::Idle;
        }
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());

        // Olvida ventanas que ya no existen.
        state.skip.retain(|&h| IsWindow(h as HWND) != 0);
        // ¿El último juego que tapamos se minimizó enseguida y estando
        // delante? Es pantalla completa exclusiva: insistir lo minimizaría
        // una y otra vez.
        let minimized = FOCUS.lock().unwrap_or_else(|e| e.into_inner()).take_minimized_in_front();
        if let (Some((raised, at)), Some((hwnd, when))) = (state.last_raise, minimized) {
            if hwnd == raised && super::exclusive(at, when, EXCLUSIVE_WINDOW) {
                if state.skip.len() < 32 {
                    state.skip.push(hwnd);
                }
                state.last_raise = None;
            }
        }
        if state.last_raise.is_some_and(|(_, at)| at.elapsed() >= EXCLUSIVE_WINDOW) {
            state.last_raise = None;
        }

        let fg = GetForegroundWindow();
        if fg == ours {
            // Solo se llega aquí si el usuario activó el reproductor a
            // propósito (Alt+Tab, barra de tareas): fuera del modo juego.
            set_game(false);
            return Poll::Idle;
        }
        if fg.is_null() || state.skip.contains(&(fg as isize)) {
            set_game(false);
            return Poll::Idle;
        }
        let Some(rect) = window_rect(fg) else {
            set_game(false);
            return Poll::Idle;
        };
        // ¿Juego a pantalla completa en SU monitor? (Puede estar en otro: un
        // clic en el reproductor tampoco debe quitarle el foco.)
        let fg_fullscreen = monitor_rect(fg).is_some_and(|m| super::covers(rect, m));
        if !fg_fullscreen {
            set_game(false);
            state.known = None;
            return Poll::Idle;
        }
        // Clasificación (proceso, estilo, capas): una vez por ventana.
        let coverable = match state.known {
            Some((hwnd, coverable)) if hwnd == fg as isize => coverable,
            _ => {
                let exe = process_name(fg);
                let candidate = Foreground {
                    rect,
                    ex_style: GetWindowLongPtrW(fg, GWL_EXSTYLE) as u32,
                    cloaked: is_cloaked(fg),
                    above_us: true,
                    exe: &exe,
                };
                let coverable = super::coverable(&candidate);
                state.known = Some((fg as isize, coverable));
                coverable
            }
        };
        set_game(coverable);
        let Some(monitor) = monitor_rect(ours) else { return Poll::Idle };
        if !coverable || !super::covers(rect, monitor) {
            return if coverable { Poll::Fullscreen } else { Poll::Idle };
        }
        let fighting = state.last_any_raise.is_some_and(|at| at.elapsed() < FIGHT_MEMORY);
        if !is_above(fg, ours) {
            return if fighting { Poll::Fight } else { Poll::Fullscreen };
        }
        // Contra peleas: si el juego se reafirma sin parar, como mucho una
        // subida cada MIN_RAISE_GAP.
        if let Some(at) = state.last_any_raise {
            let since = at.elapsed();
            if since < MIN_RAISE_GAP {
                drop(state);
                thread::sleep(MIN_RAISE_GAP - since);
                state = STATE.lock().unwrap_or_else(|e| e.into_inner());
            }
        }
        SetWindowPos(
            ours,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE | SWP_NOOWNERZORDER | SWP_ASYNCWINDOWPOS,
        );
        let now = Instant::now();
        state.last_any_raise = Some(now);
        state.last_raise = Some((fg as isize, now));
        Poll::Fight
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
    fn minimized_in_front_follows_event_order() {
        use std::time::Instant;
        let (game, other, player) = (10, 20, 30);
        let now = Instant::now();
        // Pantalla completa exclusiva: se minimiza siendo la activa.
        let mut log = FocusLog::new();
        log.foreground(game);
        log.minimize_start(game, now);
        assert_eq!(log.take_minimized_in_front(), Some((game, now)));
        assert_eq!(log.take_minimized_in_front(), None, "se consume");
        // Alt+Tab (o clic en otra app): primero cambia la ventana activa,
        // después el juego (GLFW, SDL) se minimiza al perder el foco.
        log.foreground(game);
        log.foreground(other);
        log.minimize_start(game, now);
        assert_eq!(log.take_minimized_in_front(), None);
        // Si se activa el reproductor (Alt+Tab hacia él), igual.
        log.foreground(game);
        log.foreground(player);
        log.minimize_start(game, now);
        assert_eq!(log.take_minimized_in_front(), None);
        // Sin ningún aviso previo no se acusa a nadie.
        let mut empty = FocusLog::new();
        empty.minimize_start(0, now);
        assert_eq!(empty.take_minimized_in_front(), None);
    }

    #[test]
    fn exclusive_only_if_minimized_soon_after_covering() {
        use std::time::{Duration, Instant};
        let t = Instant::now();
        let s = |n: u64| t + Duration::from_millis(n);
        let window = Duration::from_secs(3);
        assert!(exclusive(t, s(200), window));
        assert!(exclusive(t, s(2999), window));
        assert!(!exclusive(t, s(3000), window), "mucho después: otra causa");
        assert!(!exclusive(s(500), t, window), "se minimizó antes de taparlo");
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
