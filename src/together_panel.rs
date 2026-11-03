//! The "Watch together" window: its own page
//! (`https://ytmp.localhost/together`), its own closed IPC protocol
//! (`ipc::parse_together`) and navigation pinned to that page, like the
//! music bar and the lock screen.
//!
//! Closing it only hides it: the session goes on.

use tao::{
    dpi::LogicalSize,
    error::OsError,
    event_loop::{EventLoopProxy, EventLoopWindowTarget},
    window::{Window, WindowBuilder, WindowId},
};
use wry::{http::Request, WebContext, WebView, WebViewBuilder};

use crate::{
    home::{self, SharedHomeState},
    ipc::{self, UserEvent},
};

const URL: &str = "https://ytmp.localhost/together";
const WIDTH: f64 = 380.0;
const HEIGHT: f64 = 600.0;

pub fn build_window<T: 'static>(target: &EventLoopWindowTarget<T>) -> Result<Window, OsError> {
    let mut builder = WindowBuilder::new()
        .with_title(t!("together.window_title"))
        .with_window_icon(crate::icon::window_icon())
        .with_visible(false)
        .with_resizable(true)
        .with_inner_size(LogicalSize::new(WIDTH, HEIGHT))
        .with_min_inner_size(LogicalSize::new(320.0, 440.0));
    #[cfg(windows)]
    {
        use tao::platform::windows::WindowBuilderExtWindows;
        builder = builder.with_drag_and_drop(false).with_window_classname("YtMiniPlayerTogether");
    }
    builder.build(target)
}

pub struct Panel {
    window: Window,
    webview: Option<WebView>,
}

impl Panel {
    pub fn new(window: Window) -> Self {
        Self { window, webview: None }
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
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
                if let Some(command) = ipc::parse_together(request.body()) {
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
                crate::webview::watch_processes(&webview, "together");
                self.webview = Some(webview);
                true
            }
            Err(err) => {
                eprintln!("could not create the watch together window: {err}");
                false
            }
        }
    }

    pub fn show(
        &mut self,
        main: &WebView,
        web_context: &mut WebContext,
        proxy: &EventLoopProxy<UserEvent>,
        home_state: &SharedHomeState,
    ) -> bool {
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

    /// After a language change: the title and the page (served again with
    /// the new strings; it asks for the state when it loads).
    pub fn relocalize(&self) {
        self.window.set_title(&t!("together.window_title"));
        if let Some(webview) = &self.webview {
            let _ = webview.load_url(URL);
        }
    }

    pub fn push(&self, json: &str) {
        if let Some(webview) = &self.webview {
            let _ = webview.evaluate_script(&format!("window.__ytmpTogether && window.__ytmpTogether({json})"));
        }
    }

    /// "Copied" next to the button.
    pub fn copied(&self) {
        if let Some(webview) = &self.webview {
            let _ = webview.evaluate_script("window.__ytmpTogetherCopied && window.__ytmpTogetherCopied()");
        }
    }
}
