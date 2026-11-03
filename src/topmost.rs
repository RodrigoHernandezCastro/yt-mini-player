//! "Always on top" guard for games and fullscreen apps.
//!
//! `HWND_TOPMOST` only puts the window above *normal* windows. Many games in
//! "borderless fullscreen" mark themselves topmost too, and between two
//! topmost windows Windows keeps the one activated last on top: clicking the
//! game leaves the player behind.
//!
//! The guard raises the player again when **the active window covers its
//! whole monitor and is above it**. It checks right away when the active
//! window changes (an `EVENT_SYSTEM_FOREGROUND` hook), a few times in a row
//! afterwards (games often switch mode right after activating) and once a
//! second while active.
//!
//! **Game mode.** While a game (or any app) is fullscreen in the foreground,
//! a click on the player does NOT activate it (a temporary
//! `WS_EX_NOACTIVATE` and `WM_MOUSEACTIVATE` → `MA_NOACTIVATE`): the game
//! keeps the focus and the keyboard. Many games (Minecraft and everything
//! that uses GLFW, SDL…) minimize or pause when they lose the focus in
//! fullscreen; this way the video can be paused or scrubbed without taking
//! the game out of its mode.
//!
//! **Immediate reaction.** Besides the foreground hook, an
//! `EVENT_OBJECT_REORDER` hook wakes the guard when the Z order changes with
//! a game in front (games that re-assert topmost every so often), and while
//! the game insists it checks every 15 ms. At least `MIN_RAISE_GAP` passes
//! between two raises, so it never enters an unbraked loop.
//!
//! **Exclusive full screen.** Some games take the monitor for themselves
//! (Direct3D exclusive mode, a display mode of their own, Minecraft's
//! "Exclusive Fullscreen"): nothing is composed over them, not even the
//! UIAccess layer of the maximum overlay install. The guard tells them
//! apart (Windows' notification state, the display mode, a game that
//! minimizes when covered), stops trying, and when the game leaves the
//! front the player says once what to change in it ([`GameHint`]). Every
//! step goes to the session diary (`guard:` lines).
//!
//! What it deliberately does NOT do:
//! - Rise above the shell (Start, Alt+Tab, snipping, lock screen...),
//!   capture tools or transparent layers (overlays).
//! - Insist with a game that minimizes when covered: that means *exclusive*
//!   fullscreen, which no normal window can cover. It only counts if it
//!   minimized *while in front*: games that minimize when they lose the
//!   focus (Alt+Tab; Minecraft and GLFW do) are not exclusive and are
//!   covered again when the user goes back to them.
//! - Steal the focus: it always uses `SWP_NOACTIVATE`.

use crate::platform::Rect;

/// System processes and tools whose fullscreen windows must never be
/// covered (lowercase, no path).
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

/// What the guard knows about the active window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Foreground<'a> {
    pub rect: Rect,
    pub ex_style: u32,
    pub cloaked: bool,
    /// Is it above the player in the Z order?
    pub above_us: bool,
    /// The executable's name, or empty if it could not be read.
    pub exe: &'a str,
}

const WS_EX_TRANSPARENT: u32 = 0x20;
const WS_EX_TOOLWINDOW: u32 = 0x80;
const WS_EX_LAYERED: u32 = 0x80000;
const WS_EX_NOACTIVATE: u32 = 0x0800_0000;

/// Does the window cover the whole monitor? (A normal maximized window does
/// not: it leaves the taskbar free.)
pub fn covers(rect: Rect, monitor: Rect) -> bool {
    rect.left <= monitor.left
        && rect.top <= monitor.top
        && rect.right >= monitor.right
        && rect.bottom >= monitor.bottom
}

/// Floating layers (GPU overlays, capture tools, etc.): not games.
pub fn is_overlay(ex_style: u32) -> bool {
    ex_style & WS_EX_TOOLWINDOW != 0
        || ex_style & WS_EX_NOACTIVATE != 0
        || (ex_style & WS_EX_LAYERED != 0 && ex_style & WS_EX_TRANSPARENT != 0)
}

pub fn is_protected_process(exe: &str) -> bool {
    let exe = exe.to_ascii_lowercase();
    // PowerToys has several fullscreen overlays (ruler, color picker,
    // FancyZones editor...).
    exe.starts_with("powertoys") || exe.ends_with(".scr") || NEVER_COVER.contains(&exe.as_str())
}

/// Order of Windows' events: which window was active when another one
/// started to minimize? If it was the same one, it minimized on its own (not
/// because of an Alt+Tab or a click on another window, which deactivate it
/// first).
#[derive(Debug, Default)]
pub struct FocusLog {
    foreground: isize,
    minimized_in_front: Option<(isize, std::time::Instant)>,
}

impl FocusLog {
    pub const fn new() -> Self {
        Self { foreground: 0, minimized_in_front: None }
    }

    /// An `EVENT_SYSTEM_FOREGROUND` event.
    pub fn foreground(&mut self, hwnd: isize) {
        self.foreground = hwnd;
    }

    /// An `EVENT_SYSTEM_MINIMIZESTART` event.
    pub fn minimize_start(&mut self, hwnd: isize, at: std::time::Instant) {
        if hwnd != 0 && hwnd == self.foreground {
            self.minimized_in_front = Some((hwnd, at));
        }
    }

    pub fn take_minimized_in_front(&mut self) -> Option<(isize, std::time::Instant)> {
        self.minimized_in_front.take()
    }
}

/// Did it minimize (while in front) shortly after being covered? Then it is
/// exclusive fullscreen.
pub fn exclusive(raised: std::time::Instant, minimized: std::time::Instant, window: std::time::Duration) -> bool {
    minimized >= raised && minimized.duration_since(raised) < window
}

/// Is it an app that may be covered? (Not the shell, nor a floating layer,
/// nor a window DWM hides.)
pub fn coverable(fg: &Foreground) -> bool {
    !fg.cloaked && !is_overlay(fg.ex_style) && !is_protected_process(fg.exe)
}

/// Final decision: must the player be raised again? (The guard does the
/// same in parts, caching `coverable` per window.)
#[cfg(test)]
pub fn should_raise(fg: &Foreground, monitor: Rect) -> bool {
    fg.above_us && covers(fg.rect, monitor) && coverable(fg)
}

/// Why the game in front is taken to be in exclusive full screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Exclusive {
    /// It minimized while in front right after being covered.
    Minimized,
    /// Windows says a Direct3D app holds the screen exclusively.
    Direct3D,
    /// The monitor runs a display mode other than the one set in Windows.
    DisplayMode,
}

impl Exclusive {
    pub fn label(self) -> &'static str {
        match self {
            Self::Minimized => "it minimized when covered",
            Self::Direct3D => "Windows reports Direct3D exclusive full screen",
            Self::DisplayMode => "it changed the display mode",
        }
    }
}

/// `SHQueryUserNotificationState`'s value for a Direct3D exclusive app.
pub const QUNS_RUNNING_D3D_FULL_SCREEN: i32 = 3;

/// From what Windows reports while a full-screen window is in front.
pub fn exclusive_signs(notification: i32, mode_changed: bool) -> Option<Exclusive> {
    if notification == QUNS_RUNNING_D3D_FULL_SCREEN {
        Some(Exclusive::Direct3D)
    } else if mode_changed {
        Some(Exclusive::DisplayMode)
    } else {
        None
    }
}

/// `SHQueryUserNotificationState`'s answer, for the diary.
pub fn notification_label(state: i32) -> &'static str {
    match state {
        1 => "screen saver or locked",
        2 => "full-screen app",
        3 => "Direct3D exclusive full screen",
        4 => "presentation mode",
        5 => "normal",
        6 => "quiet time",
        7 => "full-screen Store app",
        _ => "unknown",
    }
}

/// The window's z-order band (`GetWindowBand`), for the diary: 2 is the
/// UIAccess layer the maximum overlay install gets.
pub fn band_label(band: u32) -> &'static str {
    match band {
        0 => "default",
        1 => "normal windows",
        2 => "UIAccess (maximum overlay)",
        _ => "a system band",
    }
}

/// What to tell the user after a game could not be covered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GameHint {
    /// Minecraft Java Edition (javaw.exe): its own setting is known.
    Minecraft,
    /// Any other game, by its executable's name (without `.exe`; empty if
    /// it could not be read, as with games protected by anti-cheat).
    Other(String),
}

pub fn game_hint(exe: &str) -> GameHint {
    let lower = exe.to_ascii_lowercase();
    if lower == "javaw.exe" || lower == "java.exe" {
        return GameHint::Minecraft;
    }
    let name = exe.strip_suffix(".exe").or_else(|| exe.strip_suffix(".EXE")).unwrap_or(exe);
    GameHint::Other(name.to_owned())
}

// ---------------------------------------------------------------------------
// Windows
// ---------------------------------------------------------------------------

#[cfg(windows)]
pub use win::{set_active, start};

/// Called (from the guard's thread) when a game it could not cover leaves
/// the front, once per game and session.
pub type OnHint = Box<dyn Fn(GameHint) + Send + Sync>;

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
            Gdi::{
                EnumDisplaySettingsW, GetMonitorInfoW, MonitorFromWindow, DEVMODEW, ENUM_CURRENT_SETTINGS,
                ENUM_REGISTRY_SETTINGS, MONITORINFO, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST,
            },
        },
        System::{
            LibraryLoader::{GetModuleHandleW, GetProcAddress},
            Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION},
        },
        UI::{
            Accessibility::{SetWinEventHook, HWINEVENTHOOK},
            Shell::{DefSubclassProc, SHQueryUserNotificationState, SetWindowSubclass},
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

    use super::{FocusLog, Foreground, GameHint, OnHint};
    use crate::platform::Rect;

    /// Checks in a row after the active window changes.
    const BURST_CHECKS: u32 = 4;
    const BURST_INTERVAL: Duration = Duration::from_millis(200);
    const IDLE_INTERVAL: Duration = Duration::from_secs(1);
    /// While a fullscreen app is in front, it checks more often: some
    /// re-assert topmost without changing the focus.
    const FULLSCREEN_INTERVAL: Duration = Duration::from_millis(100);
    /// If the game covered us again recently (it re-asserts topmost every so
    /// often), even more often: it is cheap (no processes opened).
    const FIGHT_INTERVAL: Duration = Duration::from_millis(15);
    const FIGHT_MEMORY: Duration = Duration::from_secs(1);
    /// Minimum gap between two raises (a brake against loops).
    const MIN_RAISE_GAP: Duration = Duration::from_millis(15);
    /// If the game minimizes while in front within this time after being
    /// covered, it is exclusive.
    const EXCLUSIVE_WINDOW: Duration = Duration::from_secs(3);
    /// How often the signs of exclusive full screen are read while a game is
    /// in front (two display mode queries and the notification state).
    const SIGNS_INTERVAL: Duration = Duration::from_millis(500);

    static OURS: AtomicIsize = AtomicIsize::new(0);
    static ACTIVE: AtomicBool = AtomicBool::new(false);
    /// A fullscreen app is in the foreground (on any monitor).
    static GAME: AtomicBool = AtomicBool::new(false);
    static BURST: AtomicU32 = AtomicU32::new(0);
    /// Separate from STATE: the hooks run on the UI thread and must never
    /// wait for the guard (which sends it messages).
    static FOCUS: Mutex<FocusLog> = Mutex::new(FocusLog::new());

    enum Poll {
        Idle,
        Fullscreen,
        Fight,
    }
    static WORKER: OnceLock<Thread> = OnceLock::new();
    static ON_HINT: OnceLock<OnHint> = OnceLock::new();
    static STATE: Mutex<State> = Mutex::new(State {
        skip: Vec::new(),
        last_raise: None,
        last_any_raise: None,
        known: None,
        front: None,
        pending_hint: None,
        hinted: Vec::new(),
        last_signs: None,
    });

    /// The full-screen window in front, for the diary.
    struct Front {
        hwnd: isize,
        exe: String,
        since: Instant,
        raises: u32,
    }

    struct State {
        /// Windows in exclusive fullscreen: not insisted on.
        skip: Vec<isize>,
        /// Last raise over a fullscreen window.
        last_raise: Option<(isize, Instant)>,
        last_any_raise: Option<Instant>,
        /// Last window classified: (hwnd, may it be covered?). Avoids opening
        /// the process on every check.
        known: Option<(isize, bool)>,
        front: Option<Front>,
        /// Advice to give when the game it could not cover leaves the front.
        pending_hint: Option<GameHint>,
        /// Games already advised about this session.
        hinted: Vec<GameHint>,
        last_signs: Option<Instant>,
    }

    impl State {
        /// A game it cannot cover: not insisted on, advised about later.
        fn give_up(&mut self, hwnd: isize, reason: super::Exclusive) {
            if !self.skip.contains(&hwnd) && self.skip.len() < 32 {
                self.skip.push(hwnd);
            }
            self.last_raise = None;
            let exe = match &self.front {
                Some(front) if front.hwnd == hwnd => front.exe.clone(),
                _ => unsafe { process_name(hwnd as HWND) },
            };
            crate::diag::note(format!(
                "guard: {} is in exclusive full screen ({}): no window can be shown over it, not insisting",
                show(&exe),
                reason.label()
            ));
            let hint = super::game_hint(&exe);
            if !self.hinted.contains(&hint) {
                self.pending_hint = Some(hint);
            }
        }

        /// The full-screen window left the front: its summary, and the advice
        /// if it could not be covered.
        fn leave_front(&mut self) {
            if let Some(front) = self.front.take() {
                crate::diag::note(format!(
                    "guard: {} left the front after {:.0} s ({} raises)",
                    show(&front.exe),
                    front.since.elapsed().as_secs_f64(),
                    front.raises
                ));
            }
            if let Some(hint) = self.pending_hint.take() {
                self.hinted.push(hint.clone());
                if let Some(on_hint) = ON_HINT.get() {
                    on_hint(hint);
                }
            }
        }
    }

    /// An executable's name for the diary.
    fn show(exe: &str) -> &str {
        if exe.is_empty() {
            "a window of an unknown program"
        } else {
            exe
        }
    }

    /// Enters or leaves game mode. With `WS_EX_NOACTIVATE` Windows does not
    /// activate the window on click, not even in the webview (which belongs
    /// to another process and decides its own activation).
    /// The style is checked every time (it is cheap: no messages sent) in
    /// case tao recomputed it when another window option changed.
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

    /// Starts the guard for the window `hwnd`. Call from the UI thread: the
    /// hook delivers its events in that thread's message loop. `on_hint`
    /// gets the advice about a game that could not be covered.
    pub fn start(hwnd: isize, on_hint: OnHint) {
        OURS.store(hwnd, Ordering::Relaxed);
        let _ = ON_HINT.set(on_hint);
        match unsafe { window_band(hwnd as HWND) } {
            Some(band) => crate::diag::note(format!("guard: player window in band {band} ({})", super::band_label(band))),
            None => crate::diag::note("guard: player window band unknown"),
        }
        unsafe {
            SetWindowSubclass(hwnd as HWND, Some(no_steal_focus), 0x7974_6d67, 0);
            // No SKIPOWNPROCESS: knowing when the player activates matters too
            // (the order of the events detects exclusive fullscreen).
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

    /// Active = "always on top" on and the window showing.
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
        // OBJID_WINDOW = 0: the event is about the window, not an object.
        if id_object != 0 {
            return;
        }
        FOCUS.lock().unwrap_or_else(|e| e.into_inner()).foreground(hwnd as isize);
        if ACTIVE.load(Ordering::Relaxed) {
            wake();
        }
    }

    /// A window starts to minimize. If Windows still had it as the active one
    /// (no Alt+Tab or click elsewhere first), it minimized on its own:
    /// possibly exclusive fullscreen.
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

    /// Some window's Z order changed: with a game in front, check now (no
    /// need to wait for the next round).
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
                // No cost while "always on top" is off.
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

    /// Checks and, if needed, raises the player. Returns how soon to look
    /// again.
    unsafe fn check() -> Poll {
        let ours = OURS.load(Ordering::Relaxed) as HWND;
        if ours.is_null() || IsWindowVisible(ours) == 0 || IsIconic(ours) != 0 {
            set_game(false);
            return Poll::Idle;
        }
        let mut state = STATE.lock().unwrap_or_else(|e| e.into_inner());

        // Forget windows that no longer exist.
        state.skip.retain(|&h| IsWindow(h as HWND) != 0);
        // Did the last game we covered minimize right away while in front?
        // That is exclusive fullscreen: insisting would minimize it over and
        // over.
        let minimized = FOCUS.lock().unwrap_or_else(|e| e.into_inner()).take_minimized_in_front();
        if let (Some((raised, at)), Some((hwnd, when))) = (state.last_raise, minimized) {
            if hwnd == raised && super::exclusive(at, when, EXCLUSIVE_WINDOW) {
                state.give_up(hwnd, super::Exclusive::Minimized);
            }
        }
        if state.last_raise.is_some_and(|(_, at)| at.elapsed() >= EXCLUSIVE_WINDOW) {
            state.last_raise = None;
        }

        let fg = GetForegroundWindow();
        if fg == ours {
            // Only reached if the user activated the player on purpose
            // (Alt+Tab, taskbar): out of game mode.
            set_game(false);
            state.leave_front();
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
        // A fullscreen game on ITS monitor? (It may be on another one: a
        // click on the player must not take its focus either.)
        let fg_fullscreen = monitor_rect(fg).is_some_and(|m| super::covers(rect, m));
        if !fg_fullscreen {
            set_game(false);
            state.known = None;
            state.leave_front();
            return Poll::Idle;
        }
        // Classification (process, style, layers): once per window.
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
                if state.front.as_ref().map(|f| f.hwnd) != Some(fg as isize) {
                    state.leave_front();
                    crate::diag::note(format!(
                        "guard: {} fills its screen ({}x{}){}; display mode {}; Windows says: {}",
                        show(&exe),
                        rect.right - rect.left,
                        rect.bottom - rect.top,
                        if coverable { "" } else { ", never covered (shell, tool or overlay)" },
                        if display_mode_changed(fg) { "CHANGED by it" } else { "as set in Windows" },
                        super::notification_label(notification_state())
                    ));
                    state.front = Some(Front { hwnd: fg as isize, exe, since: Instant::now(), raises: 0 });
                    state.last_signs = None;
                }
                coverable
            }
        };
        // Exclusive full screen? Read every so often while it is in front
        // (games often switch mode a moment after activating).
        if coverable && state.last_signs.is_none_or(|at| at.elapsed() >= SIGNS_INTERVAL) {
            state.last_signs = Some(Instant::now());
            if let Some(reason) = super::exclusive_signs(notification_state(), display_mode_changed(fg)) {
                state.give_up(fg as isize, reason);
                set_game(false);
                return Poll::Idle;
            }
        }
        set_game(coverable);
        let Some(monitor) = monitor_rect(ours) else { return Poll::Idle };
        if !coverable || !super::covers(rect, monitor) {
            return if coverable { Poll::Fullscreen } else { Poll::Idle };
        }
        let fighting = state.last_any_raise.is_some_and(|at| at.elapsed() < FIGHT_MEMORY);
        if !is_above(fg, ours) {
            return if fighting { Poll::Fight } else { Poll::Fullscreen };
        }
        // Against fights: if the game re-asserts itself non-stop, at most
        // one raise every MIN_RAISE_GAP.
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
        if let Some(front) = state.front.as_mut().filter(|f| f.hwnd == fg as isize) {
            if front.raises == 0 {
                crate::diag::note(format!("guard: raising the player over {}", show(&front.exe)));
            }
            front.raises = front.raises.saturating_add(1);
        }
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

    /// Is `other` above `ours` in the Z order?
    unsafe fn is_above(other: HWND, ours: HWND) -> bool {
        let mut hwnd = GetWindow(ours, GW_HWNDPREV);
        // A limit in case the Z order changes while walking it.
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

    /// Windows DWM "hides" (suspended UWP apps, other desktops).
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

    /// What Windows says about the user's screen (a Direct3D exclusive app,
    /// a full-screen app, presentation mode…); 0 if it cannot say.
    fn notification_state() -> i32 {
        let mut state = 0;
        if unsafe { SHQueryUserNotificationState(&mut state) } < 0 {
            return 0;
        }
        state
    }

    /// Does the monitor of `hwnd` run a display mode other than the one set
    /// in Windows (a game's own resolution or refresh rate)?
    unsafe fn display_mode_changed(hwnd: HWND) -> bool {
        let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
        let mut info: MONITORINFOEXW = std::mem::zeroed();
        info.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
        if GetMonitorInfoW(monitor, (&mut info as *mut MONITORINFOEXW).cast()) == 0 {
            return false;
        }
        let read = |which| {
            let mut mode: DEVMODEW = std::mem::zeroed();
            mode.dmSize = std::mem::size_of::<DEVMODEW>() as u16;
            (EnumDisplaySettingsW(info.szDevice.as_ptr(), which, &mut mode) != 0)
                .then_some((mode.dmPelsWidth, mode.dmPelsHeight, mode.dmDisplayFrequency))
        };
        match (read(ENUM_CURRENT_SETTINGS), read(ENUM_REGISTRY_SETTINGS)) {
            (Some(now), Some(set)) => now != set,
            _ => false,
        }
    }

    /// The window's z-order band (an undocumented user32 function, looked up
    /// at run time: `None` if Windows does not have it).
    unsafe fn window_band(hwnd: HWND) -> Option<u32> {
        type GetWindowBand = unsafe extern "system" fn(HWND, *mut u32) -> i32;
        let user32: Vec<u16> = "user32.dll\0".encode_utf16().collect();
        let module = GetModuleHandleW(user32.as_ptr());
        if module.is_null() {
            return None;
        }
        let function = GetProcAddress(module, c"GetWindowBand".as_ptr().cast())?;
        // SAFETY: GetWindowBand(HWND, DWORD*) -> BOOL, as user32 exports it.
        let get: GetWindowBand = std::mem::transmute(function);
        let mut band = 0u32;
        (get(hwnd, &mut band) != 0).then_some(band)
    }

    /// The executable's name (`game.exe`) or empty if it cannot be read.
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
pub fn start(_: isize, _: OnHint) {}

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
        // Some games go a few pixels past the monitor.
        let oversized = Rect { left: -8, top: -8, right: 1928, bottom: 1088 };
        assert!(should_raise(&game(oversized), MONITOR));
        assert!(should_raise(&game(SECOND), SECOND));
    }

    #[test]
    fn ignores_windows_that_do_not_cover_the_monitor() {
        // Normal maximized: leaves the taskbar free.
        let maximized = Rect { left: -8, top: -8, right: 1928, bottom: 1040 };
        assert!(!should_raise(&game(maximized), MONITOR));
        // Fullscreen, but on the other monitor.
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
        // If the name could not be read, it is treated as a game.
        assert!(should_raise(&Foreground { exe: "", ..game(MONITOR) }, MONITOR));
    }

    #[test]
    fn minimized_in_front_follows_event_order() {
        use std::time::Instant;
        let (game, other, player) = (10, 20, 30);
        let now = Instant::now();
        // Exclusive fullscreen: it minimizes while active.
        let mut log = FocusLog::new();
        log.foreground(game);
        log.minimize_start(game, now);
        assert_eq!(log.take_minimized_in_front(), Some((game, now)));
        assert_eq!(log.take_minimized_in_front(), None, "consumed");
        // Alt+Tab (or a click on another app): first the active window
        // changes, then the game (GLFW, SDL) minimizes on losing the focus.
        log.foreground(game);
        log.foreground(other);
        log.minimize_start(game, now);
        assert_eq!(log.take_minimized_in_front(), None);
        // The same if the player is activated (Alt+Tab to it).
        log.foreground(game);
        log.foreground(player);
        log.minimize_start(game, now);
        assert_eq!(log.take_minimized_in_front(), None);
        // Without any earlier event, nobody is blamed.
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
        assert!(!exclusive(t, s(3000), window), "much later: another cause");
        assert!(!exclusive(s(500), t, window), "it minimized before being covered");
    }

    #[test]
    fn exclusive_full_screen_is_told_by_windows_or_the_display_mode() {
        assert_eq!(exclusive_signs(QUNS_RUNNING_D3D_FULL_SCREEN, false), Some(Exclusive::Direct3D));
        assert_eq!(exclusive_signs(2, true), Some(Exclusive::DisplayMode), "its own resolution");
        // A borderless game: "full-screen app", the desktop's mode: covered.
        assert_eq!(exclusive_signs(2, false), None);
        assert_eq!(exclusive_signs(5, false), None);
        assert_eq!(exclusive_signs(0, false), None, "Windows could not say");
        assert_eq!(notification_label(3), "Direct3D exclusive full screen");
        assert_eq!(notification_label(42), "unknown");
        assert_eq!(band_label(2), "UIAccess (maximum overlay)");
        assert_eq!(band_label(1), "normal windows");
        assert!(Exclusive::Minimized.label().contains("minimized"));
    }

    #[test]
    fn the_advice_knows_minecraft_and_names_other_games() {
        assert_eq!(game_hint("javaw.exe"), GameHint::Minecraft);
        assert_eq!(game_hint("JAVAW.EXE"), GameHint::Minecraft);
        assert_eq!(game_hint("java.exe"), GameHint::Minecraft);
        assert_eq!(game_hint("Minecraft.Windows.exe"), GameHint::Other("Minecraft.Windows".into()), "Bedrock: not Java's setting");
        assert_eq!(game_hint("Game-Win64-Shipping.exe"), GameHint::Other("Game-Win64-Shipping".into()));
        assert_eq!(game_hint(""), GameHint::Other(String::new()), "named by the app, in its language");
    }

    #[test]
    fn never_covers_overlays() {
        for ex_style in [WS_EX_TOOLWINDOW, WS_EX_NOACTIVATE, WS_EX_LAYERED | WS_EX_TRANSPARENT] {
            assert!(!should_raise(&Foreground { ex_style, ..game(MONITOR) }, MONITOR), "{ex_style:#x}");
        }
        // A layer without "click-through" (e.g. a game with WS_EX_LAYERED): yes.
        assert!(should_raise(&Foreground { ex_style: WS_EX_LAYERED, ..game(MONITOR) }, MONITOR));
    }
}

#[cfg(test)]
#[path = "tests/topmost_cases.rs"]
mod cases;
