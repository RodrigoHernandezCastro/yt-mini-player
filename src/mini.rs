//! Music bar: media controls inside the taskbar.
//!
//! - It appears by itself while a music service is open (YouTube Music and
//!   whatever services.rs marks as music), whether the player is showing or
//!   hidden. With any other service it appears on request (tray icon click,
//!   media shortcuts with the player hidden) and hides itself a few seconds
//!   later.
//! - It sits inside the taskbar, to the left of the notification area, and
//!   can be dragged along it (dock.rs). With a vertical taskbar, it floats
//!   next to the tray.
//! - It hides if the taskbar hides (auto-hide) or if a fullscreen app is in
//!   front, and comes back when it reappears.
//! - It never takes the focus (WS_EX_NOACTIVATE): it can be used in the
//!   middle of a game without taking it out of the foreground. That is why
//!   it is shown and hidden with Win32 directly: tao always activates the
//!   window when showing it.
//! - Its webview uses the SAME WebView2 environment as the player. Two
//!   environments with different options on the same profile folder cannot
//!   coexist: WebView2 rejects the second (which is why, up to 0.4.1, the
//!   mini controller never appeared).

use std::time::{Duration, Instant};

use tao::{
    dpi::LogicalSize,
    error::OsError,
    event_loop::{EventLoopProxy, EventLoopWindowTarget},
    window::{Window, WindowBuilder, WindowId},
};
use wry::{http::Request, WebContext, WebView, WebViewBuilder};

use crate::{
    dock::{self, Layout, Placement},
    home::{self, SharedHomeState},
    ipc::{self, DragPhase, UserEvent},
    media::MediaState,
    platform::{self, Rect},
    taskbar,
};

const URL: &str = "https://ytmp.localhost/mini";
/// Window border: the palette's bright teal blue (mini.html draws the same
/// line itself on Windows 10).
const BORDER: (u8, u8, u8) = (0x00, 0x77, 0xb6);
/// Distance from the edges when floating (logical px).
const FLOAT_MARGIN: f64 = 12.0;
/// How often the taskbar is checked while the bar is showing.
const TICK: Duration = Duration::from_millis(300);
/// While hidden by a fullscreen app (or without a taskbar), checking more
/// slowly is enough.
const SLOW_TICK: Duration = Duration::from_secs(1);

/// How long it stays visible (when it is not a music service).
pub const STAY_AFTER_CLICK: Duration = Duration::from_secs(6);
pub const STAY_AFTER_HOTKEY: Duration = Duration::from_millis(2500);
pub const STAY_AFTER_LEAVE: Duration = Duration::from_millis(1800);

pub struct Mini {
    window: Window,
    webview: Option<WebView>,
    /// Windows 10 does not colour borders: the page draws its own.
    css_border: bool,
    /// A music service is open: it stays meanwhile.
    pinned: bool,
    /// Shown on request until this moment.
    until: Option<Instant>,
    hovered: bool,
    shown: bool,
    docked: bool,
    /// Offset chosen by the user (logical px to the left).
    shift: i32,
    layout: Option<Layout>,
    /// Drag in progress: (cursor x, window x) when it started.
    drag: Option<(i32, i32)>,
    /// The tray icon, for when it floats.
    anchor: Option<(i32, i32)>,
    next_tick: Option<Instant>,
    media: MediaState,
    source: Option<String>,
    /// Domain of the page when it is not a known service (shown so nobody
    /// can pass themselves off as something else in the bar).
    origin: Option<String>,
    /// The global shortcuts that work on this PC, for the tooltips
    /// (`next` → `Ctrl+Alt+N`, or `Ctrl+Alt+.` on a Polish keyboard).
    shortcuts: serde_json::Map<String, serde_json::Value>,
}

pub fn build_window<T: 'static>(target: &EventLoopWindowTarget<T>) -> Result<Window, OsError> {
    let mut builder = WindowBuilder::new()
        .with_title(t!("mini.window_title"))
        .with_window_icon(crate::icon::window_icon())
        .with_decorations(false)
        .with_resizable(false)
        .with_visible(false)
        // Topmost is applied after creating the webview (UIAccess, see app.rs).
        .with_always_on_top(false)
        .with_focusable(false)
        .with_inner_size(LogicalSize::new(dock::WIDTH, dock::FLOAT_HEIGHT));
    #[cfg(windows)]
    {
        use tao::platform::windows::WindowBuilderExtWindows;
        builder = builder
            .with_skip_taskbar(true)
            // No shadow: tao gets one by leaving an invisible frame around the
            // client area, which in this window showed as a white border
            // several pixels wide (taskbar::prepare removes it completely).
            .with_undecorated_shadow(false)
            .with_drag_and_drop(false)
            .with_window_classname("YtMiniPlayerControls");
    }
    builder.build(target)
}

impl Mini {
    pub fn new(window: Window, shift: i32) -> Self {
        Self {
            window,
            webview: None,
            css_border: false,
            pinned: false,
            until: None,
            hovered: false,
            shown: false,
            docked: false,
            shift,
            layout: None,
            drag: None,
            anchor: None,
            next_tick: None,
            media: MediaState::default(),
            source: None,
            origin: None,
            shortcuts: serde_json::Map::new(),
        }
    }

    pub fn set_shortcuts(&mut self, shortcuts: serde_json::Map<String, serde_json::Value>) {
        self.shortcuts = shortcuts;
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    pub fn is_visible(&self) -> bool {
        self.shown
    }

    pub fn is_pinned(&self) -> bool {
        self.pinned
    }

    #[cfg(windows)]
    fn hwnd(&self) -> isize {
        use tao::platform::windows::WindowExtWindows;
        self.window.hwnd()
    }

    #[cfg(not(windows))]
    fn hwnd(&self) -> isize {
        0
    }

    /// Creates the webview the first time it is needed.
    pub fn ensure_webview(
        &mut self,
        main: &WebView,
        web_context: &mut WebContext,
        proxy: &EventLoopProxy<UserEvent>,
        home_state: &SharedHomeState,
    ) -> bool {
        if self.webview.is_some() {
            return true;
        }
        let proxy = proxy.clone();
        let home_state = home_state.clone();
        let mut builder = WebViewBuilder::new_with_web_context(web_context)
            .with_url(URL)
            .with_background_color((3, 4, 94, 255))
            .with_custom_protocol("ytmp".into(), move |_id, request| home::respond(&request, &home_state))
            .with_ipc_handler(move |request: Request<String>| {
                if let Some(command) = ipc::parse_mini(request.body()) {
                    let _ = proxy.send_event(UserEvent::Command(command));
                }
            })
            // Pins the page: no links, windows, downloads or permissions.
            .with_navigation_handler(|url| url == URL)
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
            .with_download_started_handler(|_, _| false)
            .with_permission_handler(|_| wry::PermissionResponse::Deny);
        #[cfg(windows)]
        {
            use wry::{WebViewBuilderExtWindows, WebViewExtWindows};
            builder = builder
                .with_environment(main.environment())
                // Just in case: the same environment options as the player.
                .with_browser_extensions_enabled(true)
                .with_additional_browser_args(crate::webview::browser_args())
                .with_https_scheme(true)
                .with_default_context_menus(false);
        }
        #[cfg(not(windows))]
        let _ = main;
        match builder.build(&self.window) {
            Ok(webview) => {
                crate::webview::harden(&webview);
                self.webview = Some(webview);
                // Now: always on top (before, WebView2 fails with UIAccess).
                // Before `prepare`: tao rewrites the styles when it changes.
                self.window.set_always_on_top(true);
                let hwnd = self.hwnd();
                taskbar::prepare(hwnd);
                self.css_border = cfg!(windows) && !platform::set_border_color(hwnd, BORDER);
                platform::never_activate_on_click(hwnd);
                taskbar::keep_above(hwnd);
                true
            }
            Err(err) => {
                eprintln!("could not create the music bar: {err}");
                false
            }
        }
    }

    pub fn set_pinned(&mut self, pinned: bool) {
        self.pinned = pinned;
    }

    /// Shows it on request for `stay` (or longer, if already requested).
    pub fn show_for(&mut self, stay: Duration, anchor: Option<(i32, i32)>) {
        if anchor.is_some() {
            self.anchor = anchor;
        }
        let at = Instant::now() + stay;
        self.until = Some(self.until.map_or(at, |current| current.max(at)));
    }

    /// Ends the on-request showing (a music service keeps it).
    pub fn end_transient(&mut self) {
        self.until = None;
    }

    pub fn set_hovered(&mut self, hovered: bool) {
        self.hovered = hovered && self.shown;
        if !hovered && self.shown && !self.pinned {
            self.show_for(STAY_AFTER_LEAVE, None);
        }
    }

    fn wanted(&self, now: Instant) -> bool {
        self.pinned || self.until.is_some_and(|at| now < at) || (self.hovered && self.shown) || self.drag.is_some()
    }

    /// Decides whether it shows and where. Called after any change and every
    /// `TICK` while it should show.
    pub fn update(&mut self, now: Instant) {
        if self.until.is_some_and(|at| now >= at) {
            self.until = None;
        }
        if !self.wanted(now) || self.webview.is_none() {
            self.hide_now();
            self.next_tick = None;
            return;
        }
        self.next_tick = Some(now + TICK);
        let hwnd = self.hwnd();
        let Some((taskbar_hwnd, info)) = taskbar::info() else {
            self.next_tick = Some(now + SLOW_TICK);
            self.float();
            return;
        };
        match dock::place(&info, taskbar::fullscreen_in_front(info.monitor)) {
            Placement::Docked(layout) => {
                self.layout = Some(layout);
                // While dragging, the current position wins over the saved one.
                let rect = match (self.drag, taskbar::window_rect(hwnd)) {
                    (Some(_), Some(current)) if self.shown => {
                        let width = current.right - current.left;
                        let (lo, hi) = layout.drag_range(width);
                        let x = current.left.clamp(lo, hi);
                        Rect { left: x, top: layout.y, right: x + width, bottom: layout.y + layout.height }
                    }
                    _ => layout.rect(self.shift),
                };
                self.show_at(rect, true);
                taskbar::raise_if_covered(hwnd, taskbar_hwnd);
                taskbar::set_scanning(true);
            }
            Placement::Hidden => {
                self.next_tick = Some(now + SLOW_TICK);
                self.hide_now();
            }
            Placement::Unavailable => {
                self.next_tick = Some(now + SLOW_TICK);
                self.float();
            }
        }
    }

    /// Without a horizontal taskbar: it floats next to the tray.
    fn float(&mut self) {
        self.layout = None;
        self.drag = None;
        taskbar::set_scanning(false);
        let area = match self.anchor {
            Some((x, y)) => platform::work_area_at(x, y),
            None => platform::primary_work_area(),
        };
        let scale = self.window.scale_factor();
        let size = ((dock::WIDTH * scale).round() as i32, (dock::FLOAT_HEIGHT * scale).round() as i32);
        let margin = (FLOAT_MARGIN * scale).round() as i32;
        let (x, y) = match area {
            Some(area) => {
                let anchor = self.anchor.unwrap_or((area.right - 1, area.bottom));
                platform::popup_position(area, anchor, size, margin)
            }
            None => (0, 0),
        };
        self.show_at(Rect { left: x, top: y, right: x + size.0, bottom: y + size.1 }, false);
    }

    fn show_at(&mut self, rect: Rect, docked: bool) {
        let hwnd = self.hwnd();
        let current = if self.shown { taskbar::window_rect(hwnd) } else { None };
        if current != Some(rect) {
            taskbar::place(hwnd, rect);
        }
        let changed = !self.shown || docked != self.docked;
        self.shown = true;
        self.docked = docked;
        if changed {
            self.push();
        }
    }

    fn hide_now(&mut self) {
        if self.shown {
            platform::hide_window(self.hwnd());
        }
        taskbar::set_scanning(false);
        self.shown = false;
        self.hovered = false;
        self.drag = None;
    }

    /// Explicit close (right click, exit).
    pub fn close(&mut self) {
        self.until = None;
        self.drag = None;
        self.hide_now();
        self.next_tick = None;
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.next_tick
    }

    pub fn on_timer(&mut self, now: Instant) {
        if self.next_tick.is_some_and(|at| now >= at) {
            self.update(now);
        }
    }

    /// Dragging along the taskbar. On release it returns the new offset, to
    /// save it.
    pub fn drag(&mut self, phase: DragPhase) -> Option<i32> {
        let hwnd = self.hwnd();
        let Some(layout) = self.layout.filter(|_| self.shown && self.docked) else {
            self.drag = None;
            return None;
        };
        match phase {
            DragPhase::Start => {
                if let (Some(cursor), Some(rect)) = (taskbar::cursor_x(), taskbar::window_rect(hwnd)) {
                    self.drag = Some((cursor, rect.left));
                }
                None
            }
            DragPhase::Move => {
                if let (Some((cursor0, x0)), Some(cursor)) = (self.drag, taskbar::cursor_x()) {
                    let width = taskbar::window_rect(hwnd).map_or(layout.full_width, |r| r.right - r.left);
                    let (lo, hi) = layout.drag_range(width);
                    let x = (x0 + cursor - cursor0).clamp(lo, hi);
                    let rect = Rect { left: x, top: layout.y, right: x + width, bottom: layout.y + layout.height };
                    taskbar::place(hwnd, rect);
                }
                None
            }
            DragPhase::End => {
                self.drag.take()?;
                let x = taskbar::window_rect(hwnd).map_or_else(|| layout.rect(self.shift).left, |r| r.left);
                self.shift = layout.shift_for(x);
                Some(self.shift)
            }
        }
    }

    /// After a language change: the window title and the page (served again
    /// with the new strings; it asks for the state when it loads).
    pub fn relocalize(&self) {
        self.window.set_title(&t!("mini.window_title"));
        if let Some(webview) = &self.webview {
            let _ = webview.load_url(URL);
        }
    }

    /// Keeps what is playing and, if it shows, sends it to the page.
    pub fn set_media(&mut self, media: &MediaState, source: Option<String>, origin: Option<String>) {
        self.media = media.clone();
        self.source = source;
        self.origin = origin;
        if self.shown {
            self.push();
        }
    }

    /// Sends the state to the bar's page.
    pub fn push(&self) {
        let Some(webview) = &self.webview else { return };
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Payload<'a> {
            media: &'a MediaState,
            css_border: bool,
            docked: bool,
            source: Option<&'a str>,
            origin: Option<&'a str>,
            shortcuts: &'a serde_json::Map<String, serde_json::Value>,
        }
        let payload = Payload {
            media: &self.media,
            css_border: self.css_border,
            docked: self.docked,
            source: self.source.as_deref(),
            origin: self.origin.as_deref(),
            shortcuts: &self.shortcuts,
        };
        if let Ok(json) = serde_json::to_string(&payload) {
            let _ = webview.evaluate_script(&format!("window.__ytmpMini && window.__ytmpMini({json})"));
        }
    }
}
