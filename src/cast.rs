//! Sending the host's window (Watch together, "Share my window").
//!
//! - **Picture**: DevTools' screencast of the main page (devtools.rs), JPEG
//!   pictures as they are drawn, at most 30 a second, up to 1280×720.
//! - **Sound**: the app's own sound, captured with Windows' process
//!   loopback (loopback.rs): what the window plays, nothing else. If that
//!   fails, or nothing is heard while the page plays, the host may switch
//!   to all of the PC's sound for this sharing session (the panel says what
//!   that includes); it goes back to the app's sound when sharing stops.
//! - **Sending**: a hidden page (`https://ytmp.localhost/cast`, cast.html)
//!   turns both into WebRTC tracks and keeps one connection per guest
//!   (offers and answers travel sealed through the room, see together/).
//!
//! That page lives in its own hidden window and webview, pinned to its
//! address, with its own closed IPC protocol (`ipc::parse_cast`). It exists
//! only while the window is being shared.

use std::{cell::Cell, rc::Rc, time::Instant};

use tao::{
    dpi::LogicalSize,
    error::OsError,
    event_loop::{EventLoopProxy, EventLoopWindowTarget},
    window::{Window, WindowBuilder},
};
use wry::{http::Request, WebContext, WebView, WebViewBuilder};

use crate::{
    devtools::{self, Screencast},
    home::{self, SharedHomeState},
    ipc::{self, Command, UserEvent},
    loopback::{self, Source, SoundWatch},
    remote::Viewport,
    together::Sound,
};

const URL: &str = "https://ytmp.localhost/cast";

pub fn build_window<T: 'static>(target: &EventLoopWindowTarget<T>) -> Result<Window, OsError> {
    let mut builder = WindowBuilder::new()
        .with_title("YT Mini Player")
        .with_visible(false)
        .with_inner_size(LogicalSize::new(320.0, 240.0));
    #[cfg(windows)]
    {
        use tao::platform::windows::WindowBuilderExtWindows;
        builder = builder
            .with_skip_taskbar(true)
            .with_drag_and_drop(false)
            .with_window_classname("YtMiniPlayerCast");
    }
    builder.build(target)
}

pub struct Cast {
    window: Window,
    webview: Option<WebView>,
    /// The page said it is ready; until then its messages wait here.
    ready: bool,
    loaded_once: bool,
    queue: Vec<String>,
    screencast: Option<Screencast>,
    audio: Option<loopback::Capture>,
    viewport: Rc<Cell<Option<(f64, f64)>>>,
    /// Can the page send sound (MediaStreamTrackGenerator + AudioData)?
    page_sound: Option<bool>,
    /// Did the loopback start?
    capture_sound: Option<bool>,
    /// Which capture a status belongs to (a replaced one's is ignored).
    capture_id: u32,
    source: Source,
    /// The app's own sound could not be captured in this sharing session.
    app_unavailable: bool,
    /// Is anything heard while the page plays?
    watch: SoundWatch,
    next_check: Option<Instant>,
}

impl Cast {
    pub fn new(window: Window) -> Self {
        Self {
            window,
            webview: None,
            ready: false,
            loaded_once: false,
            queue: Vec::new(),
            screencast: None,
            audio: None,
            viewport: Rc::new(Cell::new(None)),
            page_sound: None,
            capture_sound: None,
            capture_id: 0,
            source: Source::App,
            app_unavailable: false,
            watch: SoundWatch::default(),
            next_check: None,
        }
    }

    pub fn active(&self) -> bool {
        self.webview.is_some()
    }

    /// The host page's size (CSS px), from the last picture.
    pub fn viewport(&self) -> Option<Viewport> {
        self.viewport.get().map(|(width, height)| Viewport { width, height })
    }

    /// Starts sending: the page, the pictures and the sound.
    pub fn start(
        &mut self,
        main: &WebView,
        web_context: &mut WebContext,
        proxy: &EventLoopProxy<UserEvent>,
        home_state: &SharedHomeState,
    ) -> bool {
        if self.webview.is_none() {
            crate::diag::note("cast: creating the sending page");
            match build_webview(&self.window, main, web_context, proxy, home_state) {
                Ok(webview) => {
                    crate::webview::watch_processes(&webview, "cast");
                    crate::diag::note("cast: page created");
                    self.webview = Some(webview);
                    self.ready = false;
                    self.loaded_once = false;
                }
                Err(err) => {
                    crate::diag::note(format!("cast: could not create the page ({err})"));
                    return false;
                }
            }
        }
        if self.screencast.is_none() {
            if let Some(target) = &self.webview {
                crate::diag::note("screencast: starting");
                self.screencast = devtools::start_screencast(main, target, self.viewport.clone());
                if self.screencast.is_none() {
                    crate::diag::note("screencast: could not subscribe");
                }
            }
        }
        if self.audio.is_none() && crate::diag::sound_crashed() {
            // The last session closed while this was starting: not again
            // (unless the host asks for a source from the panel).
            crate::diag::note("sound: skipped (the last session closed while starting it)");
            self.capture_id = self.capture_id.wrapping_add(1);
            let _ = proxy.send_event(UserEvent::Command(Command::CastSound(self.capture_id, false)));
        } else if self.audio.is_none() {
            self.start_capture(proxy);
        }
        self.screencast.is_some() || cfg!(not(windows))
    }

    fn start_capture(&mut self, proxy: &EventLoopProxy<UserEvent>) {
        self.capture_id = self.capture_id.wrapping_add(1);
        let id = self.capture_id;
        let chunk_proxy = proxy.clone();
        let status_proxy = proxy.clone();
        self.capture_sound = None;
        self.watch = SoundWatch::default();
        self.next_check = Some(Instant::now() + loopback::CHECK_EVERY);
        self.audio = Some(loopback::Capture::start(
            self.source,
            move |chunk| {
                let _ = chunk_proxy.send_event(UserEvent::Sound(chunk));
            },
            move |ok| {
                let _ = status_proxy.send_event(UserEvent::Command(Command::CastSound(id, ok)));
            },
        ));
    }

    /// The host chose where the sound comes from (for this sharing
    /// session): the capture starts again from there. `None` if nothing
    /// changed.
    pub fn set_source(&mut self, source: Source, proxy: &EventLoopProxy<UserEvent>) -> Option<Sound> {
        if !self.active() || (source == self.source && self.audio.is_some()) {
            return None;
        }
        crate::diag::note(format!("sound: switching to {}", source.label()));
        if let Some(audio) = self.audio.take() {
            audio.stop();
        }
        self.source = source;
        self.start_capture(proxy);
        Some(self.sound_status())
    }

    /// When the sound should next be checked (while capturing).
    pub fn sound_deadline(&self) -> Option<Instant> {
        self.next_check.filter(|_| self.audio.is_some())
    }

    /// Is anything heard while the page plays? `Some` when the answer
    /// changes. `page_playing` asks WebView2 (only when needed).
    pub fn check_sound(&mut self, now: Instant, page_playing: impl FnOnce() -> Option<bool>) -> Option<Sound> {
        let audio = self.audio.as_ref()?;
        if self.next_check.is_some_and(|at| now < at) {
            return None;
        }
        self.next_check = Some(now + loopback::CHECK_EVERY);
        let heard = audio.take_heard();
        if self.capture_sound != Some(true) {
            return None;
        }
        let deaf = self.watch.check(heard, page_playing())?;
        crate::diag::note(if deaf {
            format!("sound: nothing heard from {} for 15 s while the page plays", self.source.label())
        } else {
            "sound: heard again".to_owned()
        });
        Some(self.sound_status())
    }

    /// Stops everything and closes the page (its connections end with it).
    pub fn stop(&mut self) {
        if self.webview.is_some() {
            crate::diag::note("cast: stopping");
        }
        self.screencast = None;
        if let Some(audio) = self.audio.take() {
            audio.stop();
        }
        self.webview = None;
        self.ready = false;
        self.queue.clear();
        self.viewport.set(None);
        self.page_sound = None;
        self.capture_sound = None;
        // The PC's sound is for one session: the next one starts with the
        // app's own again.
        self.source = Source::App;
        self.app_unavailable = false;
        self.watch = SoundWatch::default();
        self.next_check = None;
    }

    /// A message for the page (JSON), now or once it is ready.
    pub fn send(&mut self, json: &str) {
        if !self.active() {
            return;
        }
        if self.ready {
            if let Some(webview) = &self.webview {
                devtools::post(webview, json);
            }
        } else {
            self.queue.push(json.to_owned());
        }
    }

    /// The page loaded. `true` if it had loaded before (it reloaded or its
    /// process restarted: its connections are gone and must be made again).
    pub fn on_ready(&mut self) -> bool {
        let again = std::mem::replace(&mut self.loaded_once, true);
        crate::diag::note(if again { "cast: page ready again (reloaded)" } else { "cast: page ready" });
        self.ready = true;
        if again {
            // What was waiting belongs to the connections that are gone.
            self.queue.clear();
        }
        if let Some(webview) = &self.webview {
            for json in self.queue.drain(..) {
                devtools::post(webview, &json);
            }
        }
        again
    }

    /// A piece of the window's sound (48 kHz stereo s16le).
    pub fn sound(&self, chunk: &[u8]) {
        use base64::{engine::general_purpose::STANDARD, Engine};
        if let (true, Some(webview)) = (self.ready, &self.webview) {
            devtools::post(webview, &format!("a{}", STANDARD.encode(chunk)));
        }
    }

    pub fn set_page_sound(&mut self, ok: bool) -> Sound {
        self.page_sound = Some(ok);
        self.sound_status()
    }

    /// Capture `id` says whether it works (`None` if that capture was
    /// replaced or sharing stopped).
    pub fn set_capture_sound(&mut self, id: u32, ok: bool) -> Option<Sound> {
        if id != self.capture_id || !self.active() {
            return None;
        }
        self.capture_sound = Some(ok);
        if !ok && self.source == Source::App {
            self.app_unavailable = true;
        }
        Some(self.sound_status())
    }

    /// How the sound is going, for the panel.
    pub fn sound_status(&self) -> Sound {
        Sound::new(self.page_sound, self.capture_sound, self.watch.deaf(), self.source == Source::Pc, self.app_unavailable)
    }
}

fn build_webview(
    window: &Window,
    main: &WebView,
    web_context: &mut WebContext,
    proxy: &EventLoopProxy<UserEvent>,
    home_state: &SharedHomeState,
) -> wry::Result<WebView> {
    let proxy = proxy.clone();
    let home_state = home_state.clone();
    let mut builder = WebViewBuilder::new_with_web_context(web_context)
        .with_url(URL)
        .with_custom_protocol("ytmp".into(), move |_id, request| home::respond(&request, &home_state))
        .with_ipc_handler(move |request: Request<String>| {
            if let Some(command) = ipc::parse_cast(request.body()) {
                let _ = proxy.send_event(UserEvent::Command(command));
            }
        })
        // Pinned to its page: no links, windows, downloads or permissions.
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
    let webview = builder.build(window)?;
    crate::webview::harden(&webview);
    Ok(webview)
}
