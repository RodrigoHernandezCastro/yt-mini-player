//! The Settings window: its own page (`https://ytmp.localhost/settings`),
//! its own closed IPC protocol (`ipc::parse_settings`) and navigation
//! pinned to that page, like the "Watch together" window.
//!
//! It is a window of its own (not a menu inside the player) so it can be
//! read and used while the player is small, click-through (ghost mode) or
//! showing a video. Closing it only hides it.

use std::cell::Cell;

use tao::{
    dpi::LogicalSize,
    error::OsError,
    event_loop::{EventLoopProxy, EventLoopWindowTarget},
    window::{Window, WindowBuilder, WindowId},
};
use wry::{http::Request, WebContext, WebView, WebViewBuilder};

use crate::{
    home::{self, SharedHomeState},
    ipc::{self, SettingsSection, UserEvent},
};

const URL: &str = "https://ytmp.localhost/settings";
const WIDTH: f64 = 760.0;
const HEIGHT: f64 = 560.0;

pub fn build_window<T: 'static>(target: &EventLoopWindowTarget<T>) -> Result<Window, OsError> {
    let mut builder = WindowBuilder::new()
        .with_title(t!("settings.window_title"))
        .with_window_icon(crate::icon::window_icon())
        .with_visible(false)
        .with_resizable(true)
        .with_inner_size(LogicalSize::new(WIDTH, HEIGHT))
        .with_min_inner_size(LogicalSize::new(420.0, 400.0));
    #[cfg(windows)]
    {
        use tao::platform::windows::WindowBuilderExtWindows;
        builder = builder.with_drag_and_drop(false).with_window_classname("YtMiniPlayerSettings");
    }
    builder.build(target)
}

pub struct Panel {
    window: Window,
    webview: Option<WebView>,
    /// The section to show, sent with the next state.
    section: Cell<Option<SettingsSection>>,
    /// The page said it is ready (until then a state would be lost, and
    /// with it the section asked for).
    ready: Cell<bool>,
}

impl Panel {
    pub fn new(window: Window) -> Self {
        Self { window, webview: None, section: Cell::new(None), ready: Cell::new(false) }
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    /// Its page is loaded and listening (it may be hidden).
    pub fn loaded(&self) -> bool {
        self.webview.is_some() && self.ready.get()
    }

    /// The page said "ready" (after loading or reloading).
    pub fn set_ready(&self) {
        self.ready.set(self.webview.is_some());
    }

    pub fn visible(&self) -> bool {
        self.webview.is_some() && self.window.is_visible()
    }

    /// For dialogs asked from Settings (they open above it).
    #[cfg(windows)]
    pub fn hwnd(&self) -> isize {
        use tao::platform::windows::WindowExtWindows;
        self.window.hwnd()
    }

    fn ensure_webview(
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
                if let Some(command) = ipc::parse_settings(request.body()) {
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
                crate::webview::watch_processes(&webview, "settings");
                self.webview = Some(webview);
                self.ready.set(false);
                true
            }
            Err(err) => {
                eprintln!("could not create the settings window: {err}");
                false
            }
        }
    }

    /// Shows the window (at `section`, if given) above the player.
    pub fn show(
        &mut self,
        section: Option<SettingsSection>,
        main: &WebView,
        web_context: &mut WebContext,
        proxy: &EventLoopProxy<UserEvent>,
        home_state: &SharedHomeState,
    ) -> bool {
        if section.is_some() {
            self.section.set(section);
        }
        if !self.ensure_webview(main, web_context, proxy, home_state) {
            return false;
        }
        self.window.set_visible(true);
        if self.window.is_minimized() {
            self.window.set_minimized(false);
        }
        // Above the player, which is usually always on top (set after the
        // webview exists: see app.rs about UIAccess).
        self.window.set_always_on_top(true);
        self.window.set_focus();
        if let Some(webview) = &self.webview {
            let _ = webview.focus();
        }
        true
    }

    pub fn hide(&mut self) {
        self.window.set_visible(false);
    }

    /// The section asked for (once).
    pub fn take_section(&self) -> Option<SettingsSection> {
        self.section.take()
    }

    /// After a language change: the title and the page (served again with
    /// the new strings; it asks for the state when it loads).
    pub fn relocalize(&self) {
        self.window.set_title(&t!("settings.window_title"));
        if let Some(webview) = &self.webview {
            self.ready.set(false);
            let _ = webview.load_url(URL);
        }
    }

    pub fn push(&self, json: &str) {
        if let Some(webview) = &self.webview {
            let _ = webview.evaluate_script(&format!("window.__ytmpSettings && window.__ytmpSettings({json})"));
        }
    }
}
