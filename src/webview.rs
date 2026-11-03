//! Web layer: WebView2 (through wry) on top of the tao window.
//!
//! Security (see also policy.rs):
//! - **Authenticated IPC**: our scripts go in a single wrapper that keeps,
//!   before the page runs, the real send function and a random session
//!   token. The page can call `window.ipc`, but without the token the app
//!   drops the message. In turn, the functions the app calls in the page
//!   (`__ytmpSync`, `__ytmpToast`, `__ytmpLocale`) require another key.
//! - **Filtered navigation**: only https (http on the local network),
//!   installed extensions' pages and the start screen; in maximum overlay
//!   mode, only the catalog.
//! - **New windows**: from the catalog → in this window; the rest → in the
//!   default browser.
//! - **Downloads blocked** and **permissions denied** (camera, microphone,
//!   location, notifications, clipboard, files...).
//! - **SmartScreen on** (wry turns it off by default).
//! - **No password saving or autofill** in the profile.

use std::sync::{Arc, Mutex};

use tao::{event_loop::EventLoopProxy, window::Window};
use wry::{http::Request, NewWindowResponse, PermissionKind, PermissionResponse, WebContext, WebView, WebViewBuilder};

use crate::{
    home::{self, SharedHomeState},
    ipc::{self, Command, UserEvent},
    policy::{self, Navigation, NewWindow},
};

/// The start screen's own protocol (see services::HOME_URL).
const HOME_PROTOCOL: &str = "ytmp";
/// Dragging and resizing (Alt+click, edges, dragging the video).
const INJECT_JS: &str = include_str!("inject.js");
/// Side bar, menu, local shortcuts, video-only mode and notices.
const UI_JS: &str = include_str!("ui.js");
/// Preventive reload in long sessions; lives in the page and uses no IPC.
const MAINTENANCE_JS: &str = include_str!("maintenance.js");
/// Reading and controlling playback (music bar, tray, shortcuts). Goes
/// first: it must run before the page to note its controls.
const MEDIA_JS: &str = include_str!("media.js");

/// Like wry's default, but WITHOUT turning SmartScreen off
/// (`msSmartScreenProtection`): arbitrary sites can be opened. Also:
/// - sound plays without a click first: the host's window opens in a
///   guest's player by itself (Watch together), and a player that needs a
///   click before it is heard would stay silent;
/// - a page that is not on screen keeps its normal priority: the hidden
///   page that sends the host's window (cast.rs) decodes and encodes 30
///   pictures a second, and in the background Windows would starve it
///   (stutter); the player hidden in the tray keeps playing smoothly too.
const BROWSER_ARGS: &str =
    "--disable-features=msWebOOUI,msPdfOOUI --autoplay-policy=no-user-gesture-required --disable-renderer-backgrounding";
/// "Hardware acceleration" off: no GPU process for drawing or video, like
/// the setting of the same name in Chrome or Edge.
const NO_GPU_ARGS: &str = " --disable-gpu --disable-gpu-compositing";

static ARGS: std::sync::OnceLock<String> = std::sync::OnceLock::new();

/// Fixed once at startup, before the first webview: every webview shares
/// one WebView2 environment, which takes its arguments only when created.
pub fn configure(hardware_acceleration: bool) {
    let mut args = if hardware_acceleration { BROWSER_ARGS.to_owned() } else { format!("{BROWSER_ARGS}{NO_GPU_ARGS}") };
    // Only the test build (e2e.rs): DevTools for the Windows tests.
    if let Some(arg) = crate::e2e::browser_arg() {
        args.push_str(&arg);
    }
    let _ = ARGS.set(args);
}

/// The arguments for WebView2's browser process.
pub fn browser_args() -> &'static str {
    ARGS.get().map_or(BROWSER_ARGS, String::as_str)
}

/// This session's keys (random on every start).
#[derive(Clone)]
pub struct Keys {
    /// Goes in front of every IPC message from our scripts.
    pub ipc: String,
    /// Required by the functions the app calls in the page.
    pub host: String,
}

impl Keys {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self { ipc: policy::new_token()?, host: policy::new_token()? })
    }
}

/// Wraps our scripts in a single scope with the authenticated channel.
///
/// `post_expr` is the expression that gets the send function. In the app it
/// is WebView2's, captured before the page runs (the page could replace
/// `window.chrome.webview.postMessage` later, but we already have the
/// original).
pub fn bundle(post_expr: &str, keys: &Keys) -> String {
    format!(
        r#"(() => {{
  'use strict';
  const __ytmpPost = {post_expr};
  const __ytmpSend = (m) => {{ if (__ytmpPost) {{ try {{ __ytmpPost("{ipc}|" + m); }} catch (_) {{}} }} }};
  const __ytmpHostKey = "{host}";
  const __ytmpShared = Object.create(null);
  // The start screen (ours, with a strict CSP) also sends commands.
  if (window.top === window && location.origin === 'https://ytmp.localhost') {{
    Object.defineProperty(window, '__ytmpHomeSend', {{ value: __ytmpSend }});
  }}
  // "My music": the app sends it its state (library, downloads...).
  if (window.top === window && location.origin === 'https://ytmp.localhost' && location.pathname === '/music') {{
    Object.defineProperty(window, '__ytmpMusic', {{
      value: (key, message) => {{
        if (key === __ytmpHostKey) window.dispatchEvent(new CustomEvent('ytmp:music', {{ detail: message }}));
      }},
    }});
  }}
  // The host's window (Watch together): its connection and its state.
  if (window.top === window && location.origin === 'https://ytmp.localhost' && location.pathname === '/stream') {{
    Object.defineProperty(window, '__ytmpStream', {{
      value: (key, message) => {{
        if (key === __ytmpHostKey) window.dispatchEvent(new CustomEvent('ytmp:stream', {{ detail: message }}));
      }},
    }});
  }}
{MEDIA_JS}
{INJECT_JS}
{UI_JS}
}})();"#,
        ipc = keys.ipc,
        host = keys.host,
    )
}

/// Does the message come from the "My music" page?
pub fn is_music_page(uri: &wry::http::Uri) -> bool {
    is_app_page(uri, "/music")
}

/// Does the message come from the page that shows the host's window?
pub fn is_stream_page(uri: &wry::http::Uri) -> bool {
    is_app_page(uri, "/stream")
}

fn is_app_page(uri: &wry::http::Uri, path: &str) -> bool {
    uri.scheme_str() == Some("https") && uri.host() == Some(crate::services::HOME_HOST) && uri.path() == path
}

/// The page that shows the host's window (Watch together).
pub const STREAM_URL: &str = "https://ytmp.localhost/stream";

/// Captures WebView2's real send function (only in the main frame).
const POST_EXPR: &str = "(() => { try { const w = window.top === window && window.chrome && window.chrome.webview; return w ? w.postMessage.bind(w) : null; } catch (_) { return null; } })()";

pub struct Options<'a> {
    pub start_url: &'a str,
    pub user_agent: Option<&'a str>,
    pub home_state: SharedHomeState,
    pub proxy: EventLoopProxy<UserEvent>,
    pub keys: Keys,
    /// Maximum overlay (UIAccess): navigation limited to the catalog.
    pub overlay: bool,
    /// Installed extensions' ids (their pages can be opened).
    pub extension_ids: Arc<Mutex<Vec<String>>>,
}

/// Creates the webview that fills the window's client area.
///
/// wry subclasses the parent window and resizes the WebView2 controller on
/// every `WM_SIZE`, so resizes need not be passed on by hand.
pub fn build(window: &Window, web_context: &mut WebContext, options: Options) -> wry::Result<WebView> {
    let Options { start_url, user_agent, home_state, proxy, keys, overlay, extension_ids } = options;
    let ipc_proxy = proxy.clone();
    let nav_proxy = proxy.clone();
    let popup_proxy = proxy.clone();
    let download_proxy = proxy.clone();
    let token = keys.ipc.clone();
    let mut builder = WebViewBuilder::new_with_web_context(web_context)
        .with_url(start_url)
        // Black instead of white while the first page loads.
        .with_background_color((0, 0, 0, 255))
        // On Windows wry injects them into iframes too; the wrapper and each
        // script guard themselves by checking `window.top`.
        .with_initialization_script(bundle(POST_EXPR, &keys))
        .with_initialization_script(MAINTENANCE_JS)
        // Asynchronous: songs are read from disk on another thread.
        .with_asynchronous_custom_protocol(HOME_PROTOCOL.into(), move |_id, request, responder| {
            if request.uri().path().starts_with("/media/") {
                let home_state = home_state.clone();
                std::thread::spawn(move || responder.respond(home::respond(&request, &home_state)));
            } else {
                responder.respond(home::respond(&request, &home_state));
            }
        })
        .with_ipc_handler(move |request: Request<String>| {
            // Without this session's token, the page wrote the message.
            let Some(message) = policy::authenticate(request.body(), &token) else { return };
            if message.starts_with("music:") {
                // Only our music page can use its protocol.
                if is_music_page(request.uri()) {
                    if let Some(command) = ipc::parse_music(message) {
                        let _ = ipc_proxy.send_event(UserEvent::Command(command));
                    }
                }
                return;
            }
            if message.starts_with("stream:") {
                // Likewise the page with the host's window.
                if is_stream_page(request.uri()) {
                    if let Some(command) = ipc::parse_stream(message) {
                        let _ = ipc_proxy.send_event(UserEvent::Command(command));
                    }
                }
                return;
            }
            if let Some(command) = ipc::parse(message) {
                // Only fails if the event loop has already ended.
                let _ = ipc_proxy.send_event(UserEvent::Command(command));
            }
        })
        .with_navigation_handler(move |url| {
            let ids = extension_ids.lock().map(|ids| ids.clone()).unwrap_or_default();
            match policy::navigation(&url, overlay, &ids) {
                Navigation::Allow => true,
                Navigation::Upgrade(secure) => {
                    let _ = nav_proxy.send_event(UserEvent::Command(Command::Navigate(secure)));
                    false
                }
                Navigation::Block(reason) => {
                    let _ = nav_proxy.send_event(UserEvent::Command(Command::Blocked(reason.to_owned())));
                    false
                }
            }
        })
        .with_new_window_req_handler(move |url, _features| {
            let command = match policy::new_window(&url, overlay) {
                NewWindow::SameWindow(url) => Command::Navigate(url),
                NewWindow::External(url) => Command::OpenExternal(url),
                NewWindow::Deny => Command::Blocked("blocked.popup".to_owned()),
            };
            let _ = popup_proxy.send_event(UserEvent::Command(command));
            NewWindowResponse::Deny
        })
        .with_download_started_handler(move |_url, _path| {
            let _ = download_proxy.send_event(UserEvent::Command(Command::Blocked("blocked.download".to_owned())));
            false
        })
        .with_permission_handler(permission);

    #[cfg(windows)]
    {
        use wry::WebViewBuilderExtWindows;
        builder = builder
            // https://ytmp.localhost: a secure context (needed for DRM).
            .with_https_scheme(true)
            .with_additional_browser_args(browser_args())
            // Extensions are installed later, one by one (extensions.rs).
            .with_browser_extensions_enabled(true);
    }

    if let Some(ua) = user_agent {
        builder = builder.with_user_agent(ua);
    }

    let webview = builder.build(window)?;
    harden(&webview);
    Ok(webview)
}

/// Permissions: everything a player does not need is denied. DRM does not
/// come through here as a permission of its own; unknown kinds keep the
/// default behaviour so as not to break it.
pub fn permission(kind: PermissionKind) -> PermissionResponse {
    match kind {
        PermissionKind::Autoplay => PermissionResponse::Allow,
        PermissionKind::MediaKeySystemAccess | PermissionKind::Other => PermissionResponse::Default,
        _ => PermissionResponse::Deny,
    }
}

/// Profile settings wry does not expose.
#[cfg(windows)]
pub fn harden(webview: &WebView) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Profile3, ICoreWebView2Settings4, ICoreWebView2Settings8, ICoreWebView2_13,
        COREWEBVIEW2_TRACKING_PREVENTION_LEVEL_BALANCED,
    };
    use windows::core::Interface;
    use wry::WebViewExtWindows;

    let core = webview.webview();
    let result = unsafe {
        core.Settings().and_then(|settings| {
            // No host objects in the page.
            settings.SetAreHostObjectsAllowed(false)?;
            // Passwords and forms: the profile keeps none.
            let s4 = settings.cast::<ICoreWebView2Settings4>()?;
            s4.SetIsPasswordAutosaveEnabled(false)?;
            s4.SetIsGeneralAutofillEnabled(false)?;
            // SmartScreen (if the runtime supports it).
            if let Ok(s8) = settings.cast::<ICoreWebView2Settings8>() {
                s8.SetIsReputationCheckingRequired(true)?;
            }
            Ok(())
        })
    };
    if let Err(err) = result {
        eprintln!("could not apply the security settings: {err}");
    }
    let tracking = unsafe {
        core.cast::<ICoreWebView2_13>()
            .and_then(|c| c.Profile())
            .and_then(|p| p.cast::<ICoreWebView2Profile3>())
            .and_then(|p| p.SetPreferredTrackingPreventionLevel(COREWEBVIEW2_TRACKING_PREVENTION_LEVEL_BALANCED))
    };
    if let Err(err) = tracking {
        eprintln!("tracking prevention unavailable: {err}");
    }
}

#[cfg(not(windows))]
pub fn harden(_: &WebView) {}

/// Watches a webview's processes: if one dies (renderer, GPU, the whole
/// browser), the diary says which and why (diag.rs).
#[cfg(windows)]
pub fn watch_processes(webview: &WebView, label: &'static str) {
    use webview2_com::{
        Microsoft::Web::WebView2::Win32::{
            ICoreWebView2ProcessFailedEventArgs2, COREWEBVIEW2_PROCESS_FAILED_KIND,
            COREWEBVIEW2_PROCESS_FAILED_REASON,
        },
        ProcessFailedEventHandler,
    };
    use windows::core::Interface;
    use wry::WebViewExtWindows;

    let handler = ProcessFailedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        let mut kind = COREWEBVIEW2_PROCESS_FAILED_KIND(-1);
        unsafe {
            let _ = args.ProcessFailedKind(&mut kind);
        }
        let mut detail = String::new();
        if let Ok(args2) = args.cast::<ICoreWebView2ProcessFailedEventArgs2>() {
            let mut reason = COREWEBVIEW2_PROCESS_FAILED_REASON(-1);
            let mut code = 0i32;
            unsafe {
                let _ = args2.Reason(&mut reason);
                let _ = args2.ExitCode(&mut code);
            }
            detail = format!(" reason={} exit=0x{:08x}", reason.0, code as u32);
        }
        crate::diag::note(format!("webview {label}: process failed, kind={}{detail}", kind.0));
        Ok(())
    }));
    let mut token = 0i64;
    unsafe {
        let _ = webview.webview().add_ProcessFailed(&handler, &mut token);
    }
}

#[cfg(not(windows))]
pub fn watch_processes(_: &WebView, _: &'static str) {}

/// Clears cookies, storage, cache, history and whatever sites saved (signs
/// out of every service). `done` receives whether it worked.
#[cfg(windows)]
pub fn clear_browsing_data(webview: &WebView, done: impl FnOnce(bool) + 'static) {
    use webview2_com::{
        ClearBrowsingDataCompletedHandler,
        Microsoft::Web::WebView2::Win32::{ICoreWebView2Profile2, ICoreWebView2_13},
    };
    use windows::core::Interface;
    use wry::WebViewExtWindows;

    let done = std::rc::Rc::new(std::cell::RefCell::new(Some(done)));
    let finish = {
        let done = done.clone();
        move |ok: bool| {
            if let Some(done) = done.borrow_mut().take() {
                done(ok);
            }
        }
    };
    let on_complete = finish.clone();
    let handler = ClearBrowsingDataCompletedHandler::create(Box::new(move |result| {
        on_complete(result.is_ok());
        Ok(())
    }));
    let started = unsafe {
        webview
            .webview()
            .cast::<ICoreWebView2_13>()
            .and_then(|c| c.Profile())
            .and_then(|p| p.cast::<ICoreWebView2Profile2>())
            .and_then(|p| p.ClearBrowsingDataAll(&handler))
    };
    if started.is_err() {
        finish(false);
    }
}

#[cfg(not(windows))]
pub fn clear_browsing_data(_: &WebView, done: impl FnOnce(bool) + 'static) {
    done(false);
}

/// Changes the User-Agent; it applies to the next navigation.
#[cfg(windows)]
pub fn set_user_agent(webview: &WebView, user_agent: &str) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings2;
    use windows::core::{Interface, HSTRING};
    use wry::WebViewExtWindows;

    let result = unsafe {
        webview
            .webview()
            .Settings()
            .and_then(|settings| settings.cast::<ICoreWebView2Settings2>())
            .and_then(|settings| settings.SetUserAgent(&HSTRING::from(user_agent)))
    };
    if let Err(err) = result {
        eprintln!("could not change the User-Agent: {err}");
    }
}

#[cfg(not(windows))]
pub fn set_user_agent(_: &WebView, _: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_keeps_secrets_out_of_reach() {
        let keys = Keys { ipc: "a".repeat(32), host: "b".repeat(32) };
        let js = bundle("null", &keys);
        assert!(js.starts_with("(() => {"));
        assert!(js.trim_end().ends_with("})();"));
        // The keys only appear in the wrapper, never in globals.
        assert_eq!(js.matches(&keys.ipc).count(), 1);
        assert_eq!(js.matches(&keys.host).count(), 1);
        assert!(!js.contains("window.ipc.postMessage"), "no script uses the channel without the token");
        assert!(!js.contains("window.__ytmpStartDrag"));
        assert!(js.contains("location.pathname === '/music'"), "the music bridge only exists on its page");
        assert!(js.contains("location.pathname === '/stream'"), "nor the window's");
    }

    #[test]
    fn hardware_acceleration_turns_the_gpu_off_only_when_asked() {
        assert!(!browser_args().contains("--disable-gpu") || ARGS.get().is_some());
        assert!(NO_GPU_ARGS.contains("--disable-gpu ") && NO_GPU_ARGS.contains("--disable-gpu-compositing"));
        assert!(!BROWSER_ARGS.contains("SmartScreen"), "SmartScreen stays on");
        assert!(BROWSER_ARGS.contains("--autoplay-policy=no-user-gesture-required"), "the shared window is heard at once");
        assert!(BROWSER_ARGS.contains("--disable-renderer-backgrounding"), "the sending page is not starved");
        assert_eq!(BROWSER_ARGS.matches("--disable-features").count(), 1, "one list (a second one would replace it)");
    }

    #[test]
    fn music_messages_only_from_the_music_page() {
        let uri = |s: &str| s.parse::<wry::http::Uri>().unwrap();
        assert!(is_music_page(&uri("https://ytmp.localhost/music")));
        assert!(is_music_page(&uri("https://ytmp.localhost/music?x=1")));
        for other in ["https://ytmp.localhost/", "https://ytmp.localhost/music/x", "https://www.youtube.com/music",
                      "http://ytmp.localhost/music", "https://ytmp.localhost.evil.com/music"] {
            assert!(!is_music_page(&uri(other)), "{other}");
        }
        assert!(is_stream_page(&uri(STREAM_URL)));
        for other in ["https://ytmp.localhost/music", "https://evil.example/stream", "https://ytmp.localhost/stream/x"] {
            assert!(!is_stream_page(&uri(other)), "{other}");
        }
    }

    #[test]
    fn permissions_are_denied_by_default() {
        for kind in [
            PermissionKind::Microphone,
            PermissionKind::Camera,
            PermissionKind::Geolocation,
            PermissionKind::Notifications,
            PermissionKind::ClipboardRead,
            PermissionKind::DisplayCapture,
            PermissionKind::FileSystemAccess,
            PermissionKind::LocalFonts,
            PermissionKind::AutomaticDownloads,
        ] {
            assert_eq!(permission(kind), PermissionResponse::Deny, "{kind}");
        }
        assert_eq!(permission(PermissionKind::Autoplay), PermissionResponse::Allow);
        assert_eq!(permission(PermissionKind::MediaKeySystemAccess), PermissionResponse::Default, "do not break DRM");
    }
}
