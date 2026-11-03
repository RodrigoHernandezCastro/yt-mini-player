//! Lock screen: a webview of its own on top of the player.
//!
//! It is another webview (a child of the main window) with its own page
//! (`https://ytmp.localhost/lock`), its own closed IPC protocol
//! (`ipc::parse_lock`) and navigation pinned to that page. While locked, the
//! player's webview is hidden: it gets no keyboard or clicks and what was on
//! screen cannot be seen.

use serde::Serialize;
use tao::{event_loop::EventLoopProxy, window::Window};
use wry::{
    dpi::{PhysicalPosition, PhysicalSize},
    http::Request,
    Rect, WebContext, WebView, WebViewBuilder,
};

use crate::{
    home::{self, SharedHomeState},
    ipc::{self, UserEvent},
};

const URL: &str = "https://ytmp.localhost/lock";

/// What the page shows.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Screen {
    /// `unlock`, `verify` (to change the lock) or `new-pin`.
    pub stage: &'static str,
    /// `hello` or `pin`.
    pub method: &'static str,
    pub busy: bool,
    pub error: Option<String>,
    /// Seconds to wait after failed attempts.
    pub wait: u64,
    pub can_cancel: bool,
}

#[derive(Default)]
pub struct LockView {
    webview: Option<WebView>,
    visible: bool,
}

fn full_bounds(window: &Window) -> Rect {
    let size = window.inner_size();
    Rect {
        position: PhysicalPosition::new(0, 0).into(),
        size: PhysicalSize::new(size.width, size.height).into(),
    }
}

impl LockView {
    pub fn ensure(
        &mut self,
        window: &Window,
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
            .with_bounds(full_bounds(window))
            .with_visible(false)
            .with_background_color((3, 4, 94, 255))
            .with_custom_protocol("ytmp".into(), move |_id, request| home::respond(&request, &home_state))
            .with_ipc_handler(move |request: Request<String>| {
                if let Some(command) = ipc::parse_lock(request.body()) {
                    let _ = proxy.send_event(UserEvent::Command(command));
                }
            })
            // Pins the page: no links or redirects.
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
        match builder.build_as_child(window) {
            Ok(webview) => {
                crate::webview::harden(&webview);
                self.webview = Some(webview);
                true
            }
            Err(err) => {
                eprintln!("could not create the lock screen: {err}");
                false
            }
        }
    }

    pub fn show(&mut self, window: &Window) {
        if let Some(webview) = &self.webview {
            let _ = webview.set_bounds(full_bounds(window));
            let _ = webview.set_visible(true);
            let _ = webview.focus();
            self.visible = true;
        }
    }

    pub fn hide(&mut self) {
        if let Some(webview) = &self.webview {
            let _ = webview.set_visible(false);
        }
        self.visible = false;
    }

    /// After the window is resized.
    pub fn resize(&self, window: &Window) {
        if let (Some(webview), true) = (&self.webview, self.visible) {
            let _ = webview.set_bounds(full_bounds(window));
        }
    }

    /// After a language change: the page is served again with the new
    /// strings (it asks for the state when it loads).
    pub fn relocalize(&self) {
        if let Some(webview) = &self.webview {
            let _ = webview.load_url(URL);
        }
    }

    pub fn push(&self, screen: &Screen) {
        let Some(webview) = &self.webview else { return };
        if let Ok(json) = serde_json::to_string(screen) {
            let _ = webview.evaluate_script(&format!("window.__ytmpLock && window.__ytmpLock({json})"));
        }
    }
}
