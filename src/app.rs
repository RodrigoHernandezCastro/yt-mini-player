//! Application state and command handling.
//!
//! Every action arrives here as a [`Command`], whether it comes from the
//! page (side bar, local shortcuts), the tray or a global shortcut.
//!
//! While the app is locked only the commands in `allowed_while_locked` get
//! through; the rest are dropped.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::Serialize;
use tao::{
    dpi::{LogicalSize, PhysicalPosition},
    event_loop::EventLoopProxy,
    window::{Fullscreen, ResizeDirection, Window},
};
use wry::{WebContext, WebView};

use crate::{
    cast::Cast,
    config::{self, ExtensionIds, LockMode, Paths, Settings, Store},
    devtools,
    extensions::{self, ExtensionEvent, ExtensionHost, ExtensionStatus, LocalExtension},
    home::SharedHomeState,
    hotkeys::{self, Action, Hotkeys, ShortcutView},
    i18n::{self, Lang},
    ipc::{Command, Edge, Setting, UserEvent, MIN_OPACITY},
    lifecycle::Lifecycle,
    links,
    lock::{self, HelloError},
    lockview::{LockView, Screen},
    media::{MediaAction, MediaState},
    mini::{self, Mini},
    platform,
    remote::{self, Outcome, Remote},
    security::{self, LegacyEfs, Question},
    services::{self, Agent, Service},
    settings_panel::Panel as SettingsPanel,
    together::{self, protocol::Mode, session::StreamState, Together},
    together_panel::Panel,
    topmost,
    tray::{Tray, TrayState},
    user_agent::Agents,
    webview::{self as app_webview, Keys},
    window as app_window,
};

/// Highest opacity in ghost mode: so it is clear it is on.
const GHOST_MAX_OPACITY: u8 = 70;
/// Distance from the edges when moving to a corner (logical px).
const CORNER_MARGIN: f64 = 12.0;
/// With the lock on, it locks itself after being hidden this long.
const LOCK_AFTER_HIDDEN: Duration = Duration::from_secs(10 * 60);
/// At most one new browser tab every so often (against spam).
const EXTERNAL_INTERVAL: Duration = Duration::from_secs(2);
/// Notices about guests using the window, at most this often (each).
const USING_NOTICE: Duration = Duration::from_secs(60);
const BUSY_NOTICE: Duration = Duration::from_secs(3);
const BLOCKED_NOTICE: Duration = Duration::from_secs(20);

/// What the lock screen is open for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Goal {
    Unlock,
    /// Change the lock mode to this one.
    Change(LockMode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Verify with the current method.
    Verify,
    /// Choose a new PIN.
    NewPin,
    /// Try Windows Hello before turning it on.
    ConfirmHello,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Flow {
    goal: Goal,
    stage: Stage,
}

pub struct App {
    window: Window,
    webview: WebView,
    web_context: WebContext,
    proxy: EventLoopProxy<UserEvent>,
    /// Services whose page was found changed this session (warned once).
    page_warnings: BTreeSet<&'static str>,
    paths: Paths,
    settings: Settings,
    lifecycle: Lifecycle,

    hidden: bool,
    ghost: bool,
    fullscreen: bool,
    autostart: bool,

    ext_host: ExtensionHost,
    local_extensions: Vec<LocalExtension>,
    extensions: Vec<ExtensionStatus>,
    extension_ids: ExtensionIds,
    extension_problems: Vec<String>,
    extensions_error: Option<String>,

    tray: Option<Tray>,
    hotkeys: Hotkeys,

    agents: Agents,
    /// User-Agent of the last navigation.
    current_agent: Agent,
    home_state: SharedHomeState,
    /// Running with UIAccess (maximum overlay mode)?
    overlay: bool,

    /// What is playing now (media.js report).
    media: MediaState,
    /// Music bar (inside the taskbar).
    mini: Mini,
    /// The user closed the bar of this music service (right click): it does
    /// not come back by itself until the site changes.
    bar_dismissed: bool,
    /// To ignore the single click Windows sends after a double click.
    last_double_click: Option<Instant>,

    // Security
    store: Store,
    keys: Keys,
    legacy_efs: LegacyEfs,
    lock_view: LockView,
    locked: bool,
    flow: Option<Flow>,
    hello_busy: bool,
    pin_busy: bool,
    /// The wait after the last wrong PIN on a monotonic clock (when it
    /// started, seconds): changing the system clock does not shorten it.
    pin_wait: Option<(Instant, u64)>,
    lock_error: Option<String>,
    hidden_since: Option<Instant>,
    /// Installed extension ids, shared with the navigation filter.
    allowed_extensions: Arc<Mutex<Vec<String>>>,
    last_external: Option<Instant>,
    /// Extensions waiting for approval until unlocked (nobody can approve
    /// anything while the app is locked).
    approvals_pending: bool,
    /// Forgotten PIN without Windows Hello: it is removed once clearing the
    /// sessions has succeeded (not before).
    pin_reset_pending: bool,

    /// The "My music" service (library, downloads, licenses).
    music: Option<crate::music::Handle>,

    /// Watch together: the session and its window.
    together: Together,
    panel: Panel,
    /// Was a session on at the last look (the side bar and tray say so)?
    together_shown: Option<&'static str>,
    /// Sending the window (host) and guests using it.
    cast: Cast,
    remote: Remote,
    /// The app's menu or help is open (guests cannot click on it).
    menu_open: bool,
    /// The main page's size in CSS px, as the page says (exact even with
    /// the page zoomed; the pictures' size is the fallback).
    page_viewport: Option<remote::Viewport>,
    /// Guest: the page it was on before the host's window took its place.
    stream_return: Option<String>,
    using_notices: BTreeMap<String, Instant>,
    busy_notices: BTreeMap<String, Instant>,
    blocked_notice: Option<Instant>,
    /// The Settings window and the last state it got (sent again only
    /// when something changed).
    settings_panel: SettingsPanel,
    settings_shown: RefCell<String>,
    /// WebView2's version (Settings › Advanced).
    webview_version: String,
    /// Hardware acceleration as the engine started (a change applies on
    /// the next start).
    gpu_at_start: bool,
}

/// State sent to the page (side bar and menu).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PageState<'a> {
    version: &'static str,
    pinned: bool,
    video_only: bool,
    fullscreen_in_window: bool,
    opacity: u8,
    ghost: bool,
    fullscreen: bool,
    autostart: bool,
    tray: bool,
    extensions: &'a [ExtensionStatus],
    extension_problems: &'a [String],
    extensions_error: Option<&'a str>,
    hotkeys: Vec<ShortcutView>,
    last_service: Option<&'a str>,
    start_with_last: bool,
    overlay: bool,
    /// Watch together: `host`, `guest` or nothing.
    together: Option<&'static str>,
    hardware_acceleration: bool,
}

/// Strings for the scripts in the current page (see `push_locale`).
#[derive(Serialize)]
struct Locale {
    #[serde(flatten)]
    info: i18n::LanguageInfo,
    /// For the side bar (ui.js), on every page.
    ui: serde_json::Map<String, serde_json::Value>,
    /// For the app's own page (start screen, My music), if that is the page.
    page: Option<serde_json::Map<String, serde_json::Value>>,
}

/// State sent to the Settings window.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SettingsView<'a> {
    version: &'static str,
    webview: &'a str,
    /// The section to open at (once, when asked for).
    section: Option<&'static str>,
    pinned: bool,
    opacity: u8,
    ghost: bool,
    video_only: bool,
    fullscreen_in_window: bool,
    taskbar_bar: bool,
    autostart: bool,
    start_with_last: bool,
    tray: bool,
    hardware_acceleration: bool,
    /// What the engine runs with now (a change applies on restart).
    hardware_acceleration_now: bool,
    lock: LockMode,
    overlay: bool,
    together_name: &'a str,
    together_mode: Mode,
    together_active: bool,
    extensions: &'a [ExtensionStatus],
    extension_problems: &'a [String],
    extensions_error: Option<&'a str>,
    hotkeys: Vec<ShortcutView>,
}

pub struct Startup {
    pub window: Window,
    pub webview: WebView,
    pub web_context: WebContext,
    pub paths: Paths,
    pub settings: Settings,
    pub proxy: EventLoopProxy<UserEvent>,
    pub start_hidden: bool,
    pub agents: Agents,
    pub start_agent: Agent,
    pub home_state: SharedHomeState,
    pub mini_window: Window,
    pub store: Store,
    pub keys: Keys,
    /// What happened to 0.6.0's EFS encryption on start.
    pub legacy_efs: LegacyEfs,
    /// The vault key could not be opened and a new one was made.
    pub vault_reset: bool,
    /// The settings could not be read or did not match the profile, so the
    /// saved sign-ins were wiped at start (config::secure_profile).
    pub sessions_cleared: bool,
    pub extension_ids: ExtensionIds,
    pub allowed_extensions: Arc<Mutex<Vec<String>>>,
    pub music: Option<crate::music::Handle>,
    pub together_window: Window,
    pub settings_window: Window,
    pub cast_window: Window,
    pub webview_version: String,
}

/// After a double click on the tray another single click arrives: ignored.
const DOUBLE_CLICK_GRACE: Duration = Duration::from_millis(600);

impl App {
    pub fn new(s: Startup) -> Self {
        let autostart = platform::autostart_enabled();
        let hotkeys = hotkeys::register(s.proxy.clone());
        let ext_host = ExtensionHost::new(&s.webview, s.proxy.clone());
        let extension_ids = s.extension_ids;

        let tray_state = TrayState {
            visible: !s.start_hidden,
            pinned: s.settings.always_on_top,
            taskbar_bar: s.settings.taskbar_bar,
            lock: s.settings.lock,
            autostart,
            tooltip: platform::APP_NAME.to_owned(),
            language: i18n::current(),
            ..TrayState::default()
        };
        let accelerators = [
            Action::ToggleVisible,
            Action::PlayPause,
            Action::Next,
            Action::Previous,
            Action::Ghost,
            Action::OpacityReset,
            Action::Lock,
        ]
        .into_iter()
        .filter_map(|action| hotkeys.keys_for(action).map(|keys| (action, keys)))
        .collect();
        // The music bar's tooltips name the shortcuts that work here.
        let bar_shortcuts: serde_json::Map<String, serde_json::Value> = [
            ("toggleVisible", Action::ToggleVisible),
            ("playPause", Action::PlayPause),
            ("next", Action::Next),
            ("previous", Action::Previous),
            ("mute", Action::Mute),
            ("seekForward", Action::SeekForward),
            ("seekBack", Action::SeekBack),
        ]
        .into_iter()
        .filter_map(|(name, action)| hotkeys.keys_for(action).map(|keys| (name.to_owned(), keys.into())))
        .collect();
        let bar_shift = s.settings.bar_shift;
        let tray = match Tray::new(s.proxy.clone(), tray_state, accelerators) {
            Ok(tray) => Some(tray),
            Err(err) => {
                eprintln!("no tray icon: {err}");
                None
            }
        };

        // Watch together: the build's relay, or the one typed in the window.
        let (relay, relay_fixed) = match together::built_in_relay() {
            Some(relay) => (Some(relay), true),
            None => (s.settings.together_relay.as_deref().and_then(together::link::parse_relay), false),
        };
        let link_proxy = s.proxy.clone();
        let together = Together::new(
            Arc::new(move |id, event| {
                let _ = link_proxy.send_event(UserEvent::Together(id, event));
            }),
            relay,
            relay_fixed,
            &s.settings.together_name,
            if s.settings.together_share { Mode::Share } else { Mode::Sync },
        );

        let mut app = Self {
            window: s.window,
            webview: s.webview,
            web_context: s.web_context,
            proxy: s.proxy.clone(),
            page_warnings: BTreeSet::new(),
            paths: s.paths,
            settings: s.settings,
            lifecycle: Lifecycle::default(),
            hidden: s.start_hidden,
            ghost: false,
            fullscreen: false,
            autostart,
            ext_host,
            local_extensions: Vec::new(),
            extensions: Vec::new(),
            extension_ids,
            extension_problems: Vec::new(),
            extensions_error: None,
            tray,
            hotkeys,
            agents: s.agents,
            current_agent: s.start_agent,
            home_state: s.home_state,
            overlay: platform::has_uiaccess(),
            media: MediaState::default(),
            mini: Mini::new(s.mini_window, bar_shift),
            bar_dismissed: false,
            last_double_click: None,
            store: s.store,
            keys: s.keys,
            legacy_efs: s.legacy_efs,
            lock_view: LockView::default(),
            locked: false,
            flow: None,
            hello_busy: false,
            pin_busy: false,
            pin_wait: None,
            lock_error: None,
            hidden_since: None,
            allowed_extensions: s.allowed_extensions,
            last_external: None,
            approvals_pending: false,
            pin_reset_pending: false,
            music: s.music,
            together,
            panel: Panel::new(s.together_window),
            together_shown: None,
            cast: Cast::new(s.cast_window),
            remote: Remote::default(),
            menu_open: false,
            page_viewport: None,
            stream_return: None,
            using_notices: BTreeMap::new(),
            busy_notices: BTreeMap::new(),
            blocked_notice: None,
            settings_panel: SettingsPanel::new(s.settings_window),
            settings_shown: RefCell::new(String::new()),
            webview_version: s.webview_version,
            gpu_at_start: false,
        };
        app.gpu_at_start = app.settings.hardware_acceleration;
        app.mini.set_shortcuts(bar_shortcuts);

        // "Always on top" is applied after the webview exists: with
        // UIAccess, WebView2 does not initialize if the window is already
        // topmost (WebView2Feedback #486).
        app.window.set_always_on_top(app.settings.always_on_top);

        // The lock first: so nobody approves extensions without unlocking.
        if app.settings.lock != LockMode::Off {
            app.lock_now();
        }
        app.reload_extensions();
        // Without a tray there is no way to show a hidden window again.
        if app.hidden && app.tray.is_none() {
            app.hidden = false;
        }
        if app.hidden {
            app.lifecycle.update(&app.webview, true);
        } else {
            app.window.set_visible(true);
        }
        app.apply_effects();
        // Keeps the player above borderless fullscreen games (and says what
        // to change in a game it could not stay above).
        let hint_proxy = app.proxy.clone();
        topmost::start(
            app.hwnd(),
            Box::new(move |hint| {
                let _ = hint_proxy.send_event(UserEvent::Command(Command::GameHint(hint)));
            }),
        );
        app.update_guard();

        // Win+L or sleep: lock (if the lock is on).
        let lock_proxy = app.proxy.clone();
        security::on_session_lock(app.hwnd(), move || {
            let _ = lock_proxy.send_event(UserEvent::Command(Command::LockNow));
        });
        if app.hidden {
            app.hidden_since = Some(Instant::now());
        }
        if s.sessions_cleared {
            app.toast(&t!("app.sessions_cleared"));
        } else if s.vault_reset {
            app.toast(&t!("app.vault_reset"));
        }
        if crate::diag::crashed_last_time() {
            app.toast(&t!("app.crashed_last_time"));
        }
        app
    }

    // ------------------------------------------------------------------
    // Events
    // ------------------------------------------------------------------

    /// Returns `false` if the app must quit.
    pub fn on_user_event(&mut self, event: UserEvent) -> bool {
        match event {
            UserEvent::Command(command) => self.handle(command),
            UserEvent::Extension(event) => {
                self.on_extension(event);
                true
            }
            UserEvent::Music(json) => {
                self.push_music(&json);
                true
            }
            UserEvent::Together(id, event) => {
                let effects = self.together.on_link(id, event);
                self.apply_together(effects);
                true
            }
            UserEvent::Sound(chunk) => {
                self.cast.sound(&chunk);
                true
            }
        }
    }

    /// Sends a message from the music service to its page. The address check
    /// is in the script itself: if the window shows another site at that
    /// moment, it gets nothing (and cannot imitate the function).
    fn push_music(&self, json: &str) {
        let script = format!(
            "location.origin === 'https://ytmp.localhost' && location.pathname === '/music' \
             && window.__ytmpMusic && window.__ytmpMusic(\"{}\", {json})",
            self.keys.host
        );
        let _ = self.webview.evaluate_script(&script);
    }

    /// Minimizing and restoring arrive as `Resized`.
    pub fn on_resized(&mut self) {
        self.update_lifecycle();
        self.lock_view.resize(&self.window);
    }

    /// The only things handled while the app is locked.
    fn allowed_while_locked(command: &Command) -> bool {
        matches!(
            command,
            Command::Show
                | Command::Hide
                | Command::ToggleVisible
                | Command::Quit
                | Command::TrayClick(..)
                | Command::TrayDoubleClick
                | Command::Drag
                | Command::Resize(_)
                | Command::Ready
                | Command::MediaReport(_)
                | Command::Navigate(_)
                | Command::Blocked(_)
                | Command::LockNow
                | Command::LockReady
                | Command::LockHello
                | Command::LockPin(_)
                | Command::LockNewPin(_)
                | Command::LockCancel
                | Command::LockForgot
                | Command::ForgotResult(_)
                | Command::BrowsingDataCleared(_)
                | Command::HelloResult(_)
                | Command::PinChecked(_)
                | Command::PinHashed(_)
                | Command::SecurityStatus
                | Command::MiniHide
                | Command::MiniHover(_)
                | Command::MiniReady
                // Reveals nothing, and the lock screen must be readable.
                | Command::SetLanguage(_)
                | Command::TogetherClose
                // Sending stops on locking; these only tidy up after it.
                | Command::CastReady
                | Command::CastCaps(_)
                | Command::CastSound(..)
                | Command::CastState(..)
                | Command::StreamState(_)
                | Command::MenuOpen(_)
                | Command::Viewport(..)
                | Command::SettingsClose
        )
    }

    /// Returns `false` if the app must quit.
    pub fn handle(&mut self, command: Command) -> bool {
        if self.locked && !Self::allowed_while_locked(&command) {
            return true;
        }
        match command {
            // A guest's press on the shared window never moves or resizes
            // it (the page asks for that in answer to a press).
            Command::Drag | Command::Resize(_) if self.remote.pressing(Instant::now()) => {}
            Command::Drag => {
                if !self.fullscreen {
                    let _ = self.window.drag_window();
                }
            }
            Command::Resize(edge) => {
                if !self.fullscreen {
                    let _ = self.window.drag_resize_window(resize_direction(edge));
                }
            }
            Command::Ready => {
                // A full navigation leaves the page's fullscreen; the window
                // must follow it.
                if self.fullscreen {
                    self.set_fullscreen(false);
                }
                self.push_locale();
                self.push_state();
                // A new page: the guests see where the host is now.
                let url = self.current_url();
                let mut effects = self.together.on_media(&self.media, &url);
                // A guest that left the host's window for another page.
                if !is_stream_url(&url) {
                    effects.extend(self.together.stream_left());
                }
                self.apply_together(effects);
            }
            Command::TogglePin => {
                self.settings.always_on_top = !self.settings.always_on_top;
                self.window.set_always_on_top(self.settings.always_on_top);
                self.apply_effects();
                self.save_settings();
                self.toast(&if self.settings.always_on_top { t!("app.always_on_top_on") } else { t!("app.always_on_top_off") });
                self.state_changed();
            }
            Command::SetFullscreen(on) => self.set_fullscreen(on),
            Command::SetOpacity(value) => self.set_opacity(value),
            Command::AdjustOpacity(delta) => {
                let value = (i16::from(self.settings.opacity) + i16::from(delta))
                    .clamp(i16::from(MIN_OPACITY), 100) as u8;
                self.set_opacity(value);
                self.toast(&t!("app.opacity", value = value));
            }
            Command::ResetOpacity => {
                self.set_opacity(100);
                self.toast(&t!("app.opacity", value = 100));
            }
            Command::ToggleGhost => {
                self.ghost = !self.ghost;
                self.apply_effects();
                if self.ghost {
                    // If it was hidden, show it: otherwise the mode would have
                    // no visible effect, which would be confusing.
                    if self.hidden {
                        self.show();
                    }
                    let message = match self.hotkeys.keys_for(Action::Ghost) {
                        Some(keys) => t!("app.ghost_on", keys = keys),
                        None => t!("app.ghost_on_tray"),
                    };
                    self.toast(&message);
                } else {
                    self.toast(&t!("app.ghost_off"));
                }
                self.state_changed();
            }
            Command::Size(preset) => {
                if !self.fullscreen {
                    let (w, h) = preset.logical_size();
                    self.window.set_inner_size(LogicalSize::new(w, h));
                    self.keep_inside_work_area();
                }
            }
            Command::Corner(corner) => {
                if !self.fullscreen {
                    if let Some(area) = platform::work_area(self.hwnd()) {
                        let size = self.window.outer_size();
                        let margin = (CORNER_MARGIN * self.window.scale_factor()).round() as i32;
                        let (x, y) = platform::corner_position(
                            area,
                            (size.width as i32, size.height as i32),
                            margin,
                            corner,
                        );
                        self.window.set_outer_position(PhysicalPosition::new(x, y));
                    }
                }
            }
            Command::Home => {
                self.navigate(services::HOME_URL, Agent::Chrome);
            }
            Command::OpenService(id) => match services::by_id(&id) {
                Some(service) => self.open_service(service),
                None => self.toast(&t!("app.no_such_service")),
            },
            Command::OpenUrl(text) => match services::parse_user_url(&text) {
                Some(url) => self.open_url(url),
                None => self.toast(&t!("app.address_hint")),
            },
            Command::SetStartWithLast(on) => {
                self.settings.start_with_last = on;
                self.save_settings();
                self.sync_home_state();
                self.push_state();
            }
            Command::Show => self.show(),
            Command::Hide => self.hide(),
            Command::ToggleVisible => {
                if self.hidden || self.window.is_minimized() {
                    self.show();
                } else {
                    self.hide();
                }
            }
            Command::Quit => {
                crate::diag::note("quit (asked for)");
                self.save_on_exit();
                return false;
            }
            Command::ToggleVideoOnly => {
                self.settings.video_only = !self.settings.video_only;
                self.save_settings();
                self.push_state();
            }
            Command::OpenClipboard => {
                let text = platform::read_clipboard_text().unwrap_or_default();
                // YouTube links are normalized (youtu.be, bare ids); the other
                // catalog services are taken as they are.
                let url = links::normalize(&text).and_then(|u| url::Url::parse(&u).ok()).or_else(|| {
                    services::parse_user_url(&text).filter(|u| services::for_url(u).is_some())
                });
                match url {
                    Some(url) => {
                        if self.hidden {
                            self.show();
                        }
                        self.open_url(url);
                    }
                    None => self.toast(&t!("app.clipboard_no_link")),
                }
            }
            Command::Media(action) => self.media_action(action),
            Command::MediaReport(state) => {
                let changed_for_tray = self.media.title != state.title
                    || self.media.playing != state.playing
                    || self.media.has_media != state.has_media
                    || self.media.can_next != state.can_next
                    || self.media.can_previous != state.can_previous;
                if self.media.site != state.site {
                    self.bar_dismissed = false;
                }
                self.media = *state;
                if changed_for_tray {
                    self.update_tray();
                }
                self.refresh_bar();
                if self.together.is_active() {
                    let url = self.current_url();
                    let effects = self.together.on_media(&self.media, &url);
                    self.apply_together(effects);
                }
            }
            Command::TrayClick(x, y) => {
                if self.last_double_click.is_some_and(|t| t.elapsed() < DOUBLE_CLICK_GRACE) {
                    return true;
                }
                if self.locked && (self.hidden || self.window.is_minimized()) {
                    // Locked: the click shows the lock screen.
                    self.show();
                } else if self.hidden || self.window.is_minimized() {
                    // Hidden: the click opens (or closes) the controls.
                    if self.mini.is_visible() {
                        self.close_bar();
                    } else {
                        self.open_bar(mini::STAY_AFTER_CLICK, Some((x, y)));
                    }
                } else {
                    self.hide();
                }
            }
            Command::TrayDoubleClick => {
                self.last_double_click = Some(Instant::now());
                self.show();
            }
            Command::MiniShow => self.open_bar(mini::STAY_AFTER_CLICK, None),
            Command::MiniHide => self.close_bar(),
            Command::MiniHover(hovered) => {
                self.mini.set_hovered(hovered);
                self.mini.update(Instant::now());
                self.sync_watch();
            }
            Command::MiniReady => self.mini.push(),
            Command::MiniDrag(phase) => {
                if let Some(shift) = self.mini.drag(phase) {
                    self.settings.bar_shift = shift;
                    self.save_settings();
                }
            }
            Command::ToggleTaskbarBar => {
                self.settings.taskbar_bar = !self.settings.taskbar_bar;
                self.bar_dismissed = false;
                self.save_settings();
                self.toast(&if self.settings.taskbar_bar { t!("app.taskbar_bar_on") } else { t!("app.taskbar_bar_off") });
                self.update_tray();
                self.refresh_bar();
            }
            Command::ExtensionsFolder => platform::open_path(&self.paths.extensions),
            Command::ExtensionsReload => {
                // "Reload" asks again about the rejected ones.
                self.settings.rejected_extensions.clear();
                self.reload_extensions();
                self.toast(&t!("app.extensions_reloaded"));
            }
            Command::ExtensionToggle(id) => {
                if let Some(status) = self.extensions.iter().find(|s| s.id.as_deref() == Some(id.as_str())) {
                    self.ext_host.set_enabled(id, !status.enabled);
                }
            }
            Command::ExtensionOptions(id) => {
                let page = self
                    .extensions
                    .iter()
                    .find(|s| s.id.as_deref() == Some(id.as_str()))
                    .and_then(|s| self.local_extensions.iter().find(|l| l.folder == s.folder))
                    .and_then(|l| l.page.clone());
                match page {
                    Some(page) => {
                        let _ = self.webview.load_url(&format!("chrome-extension://{id}/{page}"));
                        // The options open in the player: Settings steps aside.
                        self.settings_panel.hide();
                        if self.hidden {
                            self.show();
                        }
                    }
                    None => self.toast(&t!("app.extension_no_options")),
                }
            }
            Command::Navigate(url) => match url::Url::parse(&url) {
                Ok(url) => self.open_url(url),
                Err(_) => self.toast(&t!("app.invalid_address")),
            },
            Command::OpenExternal(url) => {
                if self.last_external.is_some_and(|t| t.elapsed() < EXTERNAL_INTERVAL) {
                    return true;
                }
                // Not in answer to a guest's click on the shared window.
                let now = Instant::now();
                if self.remote.pressing(now) {
                    self.toast(&t!("app.remote_external_blocked"));
                    return true;
                }
                self.last_external = Some(Instant::now());
                security::open_in_browser(&url);
                let host = url::Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default();
                self.toast(&t!("app.opened_in_browser", host = host));
            }
            Command::Blocked(reason) => self.toast(&t!("app.blocked", reason = i18n::text(&reason))),
            Command::LockNow => self.lock_now(),
            Command::SetLock(mode) => self.begin_change(mode),
            Command::SignOutAll => self.sign_out_all(),
            Command::SecurityStatus => self.show_security_status(),
            Command::LockReady => self.push_lock_screen(),
            Command::LockHello => self.start_hello(),
            Command::LockPin(pin) => self.check_pin(zeroize::Zeroizing::new(pin)),
            Command::LockNewPin(pin) => self.set_new_pin(zeroize::Zeroizing::new(pin)),
            Command::LockCancel => {
                if self.flow.is_some_and(|f| f.goal != Goal::Unlock) {
                    self.end_flow();
                }
            }
            Command::LockForgot => self.forgot_pin(),
            Command::ForgotResult(result) => {
                self.hello_busy = false;
                match result {
                    Ok(true) => {
                        self.unlock();
                        self.apply_lock(LockMode::Off, None);
                        self.toast(&t!("app.pin_removed_with_hello"));
                    }
                    Ok(false) => self.lock_failed(&t!("lock.hello_not_confirmed")),
                    Err(_) => self.reset_forgotten_pin(),
                }
            }
            Command::HelloResult(result) => {
                self.hello_busy = false;
                match result {
                    Ok(true) => self.verified(),
                    Ok(false) => self.lock_failed(&t!("lock.hello_try_again")),
                    Err(err) => self.lock_failed(&HelloError::text(&err)),
                }
            }
            Command::PinChecked(ok) => {
                self.pin_busy = false;
                if ok {
                    self.settings.lock_failures = 0;
                    self.settings.lock_blocked_until = 0;
                    self.pin_wait = None;
                    self.save_settings();
                    self.verified();
                } else {
                    self.settings.lock_failures = self.settings.lock_failures.saturating_add(1);
                    let wait = lock::delay_after(self.settings.lock_failures);
                    self.settings.lock_blocked_until = if wait > 0 { lock::unix_now() + wait } else { 0 };
                    self.pin_wait = (wait > 0).then(|| (Instant::now(), wait));
                    self.save_settings();
                    self.lock_failed(&t!("lock.pin_wrong"));
                }
            }
            Command::PinHashed(result) => {
                self.pin_busy = false;
                match result {
                    Ok(hash) => self.apply_lock(LockMode::Pin, Some(hash)),
                    Err(err) => self.lock_failed(&t!("lock.pin_save_failed", error = err)),
                }
            }
            Command::BrowsingDataCleared(ok) => {
                if std::mem::take(&mut self.pin_reset_pending) {
                    if ok {
                        self.unlock();
                        self.apply_lock(LockMode::Off, None);
                    } else {
                        self.lock_failed(&t!("lock.sessions_not_cleared"));
                        return true;
                    }
                }
                if ok {
                    self.navigate(services::HOME_URL, Agent::Chrome);
                    self.toast(&t!("app.signed_out"));
                } else {
                    self.toast(&t!("app.clear_failed"));
                }
            }
            Command::Music(request) => self.on_music(request),
            Command::ToggleAutostart => {
                let wanted = !self.autostart;
                match platform::set_autostart(wanted) {
                    Ok(()) => {
                        self.autostart = wanted;
                        self.toast(&if wanted { t!("app.autostart_on") } else { t!("app.autostart_off") });
                    }
                    Err(err) => self.toast(&t!("app.autostart_failed", error = err)),
                }
                self.state_changed();
            }
            Command::SetLanguage(lang) => self.set_language(lang),
            Command::TogetherOpen => {
                if !self.panel.show(&self.webview, &mut self.web_context, &self.proxy, &self.home_state) {
                    self.toast(&t!("app.together_failed"));
                }
            }
            Command::TogetherReady => self.panel.push(&self.together.view_json()),
            Command::TogetherClose => self.panel.hide(),
            Command::TogetherStart(name) => {
                crate::diag::note(format!("together: hosting ({})", if self.settings.together_share { "share" } else { "sync" }));
                self.remember_together_name(&name);
                let effects = self.together.start(&name);
                self.apply_together(effects);
            }
            Command::TogetherJoin(code, name) => {
                crate::diag::note("together: joining");
                self.remember_together_name(&name);
                let effects = self.together.join(&code, &name);
                self.apply_together(effects);
            }
            Command::TogetherLeave => {
                let effects = self.together.leave();
                self.apply_together(effects);
            }
            Command::TogetherDismiss => {
                let effects = self.together.dismiss();
                self.apply_together(effects);
            }
            Command::TogetherKick(member) => {
                let effects = self.together.kick(&member);
                self.apply_together(effects);
            }
            Command::TogetherNewCode => {
                let effects = self.together.new_code();
                self.apply_together(effects);
            }
            Command::TogetherCopy => {
                if let Some(code) = self.together.code_text() {
                    if platform::write_clipboard_text(&code) {
                        self.panel.copied();
                    }
                }
            }
            Command::TogetherMode(mode) => {
                let share = mode == Mode::Share;
                crate::diag::note(format!("together: mode {}", if share { "share" } else { "sync" }));
                if self.settings.together_share != share {
                    self.settings.together_share = share;
                    self.save_settings();
                }
                let effects = self.together.set_mode(mode);
                self.apply_together(effects);
            }
            Command::TogetherMemberControl(member, on) => {
                if !on {
                    self.release_remote(&member);
                }
                let effects = self.together.set_member_control(&member, on);
                self.apply_together(effects);
            }
            Command::TogetherDismissAsk(member) => {
                let effects = self.together.dismiss_ask(&member);
                self.apply_together(effects);
            }
            Command::TogetherWatch => {
                if self.together.wants_window() {
                    self.show_stream();
                }
            }
            Command::TogetherResync => {
                let effects = self.together.resync();
                self.apply_together(effects);
            }
            Command::TogetherOpenSite => {
                if let Some(url) = self.together.site_to_open() {
                    self.open_url(url);
                    if self.hidden {
                        self.show();
                    }
                }
            }
            Command::TogetherRelay(text) => {
                let (taken, effects) = self.together.set_relay_text(&text);
                if taken {
                    self.settings.together_relay = self.together.relay().map(|u| u.as_str().to_owned());
                    self.save_settings();
                }
                self.apply_together(effects);
            }
            Command::CastReady => {
                if self.cast.on_ready() {
                    // It reloaded: its connections are gone.
                    let effects = self.together.cast_reset();
                    self.apply_together(effects);
                }
            }
            Command::CastCaps(sound) => {
                crate::diag::note(format!("cast: page can send sound: {sound}"));
                let sound = self.cast.set_page_sound(sound);
                let effects = self.together.set_sound(sound);
                self.apply_together(effects);
            }
            Command::CastSound(id, ok) => {
                crate::diag::note(format!("cast: sound {}", if ok { "on" } else { "off" }));
                if let Some(sound) = self.cast.set_capture_sound(id, ok) {
                    let effects = self.together.set_sound(sound);
                    self.apply_together(effects);
                }
            }
            Command::GameHint(hint) => {
                // No focus taken: the user may be anywhere by now.
                self.toast(&match hint {
                    topmost::GameHint::Minecraft => t!("app.game_exclusive_minecraft"),
                    topmost::GameHint::Other(game) if game.is_empty() => t!("app.game_exclusive", game = t!("app.a_game")),
                    topmost::GameHint::Other(game) => t!("app.game_exclusive", game = game),
                });
            }
            Command::PageCheck(check) => {
                crate::diag::note(check.line());
                // Once per session and service: the diary has the rest.
                if let Some(service) = check.broken() {
                    if self.page_warnings.insert(service) {
                        self.toast(&match *check {
                            crate::diag::PageCheck::GoogleSignInRefused => t!("app.sign_in_refused"),
                            _ => t!("app.page_changed", service = service),
                        });
                    }
                }
            }
            Command::TogetherSound(whole_pc) => {
                let source = if whole_pc { crate::loopback::Source::Pc } else { crate::loopback::Source::App };
                crate::diag::note(format!("share: the host chose {}", source.label()));
                if let Some(sound) = self.cast.set_source(source, &self.proxy) {
                    let effects = self.together.set_sound(sound);
                    self.apply_together(effects);
                }
            }
            Command::CastOffer(member, sdp) => {
                crate::diag::note(format!("cast: offer for {member} ({} bytes)", sdp.len()));
                let effects = self.together.cast_offer(&member, &sdp);
                self.apply_together(effects);
            }
            Command::CastState(member, state) => {
                crate::diag::note(format!("cast: {member} {state}"));
                let Some(state) = StreamState::from_report(&state) else { return true };
                if state == StreamState::Failed {
                    self.release_remote(&member);
                }
                let effects = self.together.cast_state(&member, state);
                self.apply_together(effects);
            }
            Command::CastInput(member, json) => self.remote_input(&member, &json),
            Command::CastStats(json) => {
                for line in crate::diag::cast_stats(&json) {
                    crate::diag::note(line);
                }
                // Whether the shared page is making any sound right now (if
                // it is and the guests hear nothing, the capture is at fault).
                if let Some(playing) = devtools::playing_audio(&self.webview) {
                    crate::diag::note(format!("cast: the page {} sound", if playing { "plays" } else { "plays no" }));
                }
            }
            Command::StreamStats(json) => {
                if let Some(line) = crate::diag::stream_stats(&json) {
                    crate::diag::note(line);
                }
            }
            Command::StreamReady => {
                crate::diag::note("watch: page ready");
                let effects = self.together.stream_ready();
                self.apply_together(effects);
            }
            Command::StreamAnswer(sdp) => {
                crate::diag::note(format!("watch: answer ({} bytes)", sdp.len()));
                let effects = self.together.stream_answer(&sdp);
                self.apply_together(effects);
            }
            Command::StreamState(state) => {
                crate::diag::note(format!("watch: {state}"));
                let effects = self.together.stream_state(&state);
                self.apply_together(effects);
            }
            Command::StreamAsk => {
                let effects = self.together.ask();
                self.apply_together(effects);
            }
            Command::StreamLeave => {
                let effects = self.together.leave();
                self.apply_together(effects);
            }
            Command::MenuOpen(open) => self.menu_open = open,
            Command::Viewport(width, height) => {
                self.page_viewport = Some(remote::Viewport { width: f64::from(width), height: f64::from(height) });
            }
            Command::ToggleHardwareAcceleration => self.toggle_hardware_acceleration(),
            Command::SettingsOpen(section) => {
                if self.settings_panel.show(section, &self.webview, &mut self.web_context, &self.proxy, &self.home_state) {
                    // Already loaded: straight to the section asked for.
                    self.push_settings(true);
                } else {
                    self.toast(&t!("app.settings_failed"));
                }
            }
            Command::SettingsReady => {
                self.settings_panel.set_ready();
                self.push_settings(true);
            }
            Command::SettingsClose => self.settings_panel.hide(),
            Command::Set(setting) => self.set(setting),
            Command::SettingsLock(mode) => {
                let current = self.settings.lock;
                if mode != current {
                    // The lock screen asks in the player.
                    self.settings_panel.hide();
                    // Picking the mode already set turns it off.
                    self.begin_change(if mode == LockMode::Off { current } else { mode });
                }
                self.push_settings(false);
            }
            Command::TogetherName(name) => {
                self.remember_together_name(&name);
                let effects = self.together.set_name(&self.settings.together_name);
                self.apply_together(effects);
                self.push_settings(false);
            }
            Command::ShowWelcome => {
                if let Ok(mut state) = self.home_state.lock() {
                    state.welcome = true;
                }
                self.settings_panel.hide();
                self.navigate(services::HOME_URL, Agent::Chrome);
                if self.hidden || self.window.is_minimized() {
                    self.show();
                }
            }
            Command::WelcomeDone => {
                if !self.settings.welcome_seen {
                    self.settings.welcome_seen = true;
                    self.save_settings();
                }
                if let Ok(mut state) = self.home_state.lock() {
                    state.welcome = false;
                }
            }
            Command::ShowHelp => {
                self.settings_panel.hide();
                if self.hidden || self.window.is_minimized() {
                    self.show();
                }
                let _ = self
                    .webview
                    .evaluate_script(&format!("window.__ytmpHelp && window.__ytmpHelp(\"{}\")", self.keys.host));
            }
            Command::OpenDataFolder => platform::open_path(&self.paths.root),
            Command::Restart => self.restart(),
        }
        true
    }

    // ------------------------------------------------------------------
    // Sharing the window
    // ------------------------------------------------------------------

    /// A guest's mouse or key on the shared window.
    fn remote_input(&mut self, member: &str, json: &str) {
        if !self.together.may_control(member) || self.locked {
            return;
        }
        let now = Instant::now();
        if !remote::page_allows_input(&self.current_url()) {
            if self.blocked_notice.is_none_or(|at| now.duration_since(at) >= BLOCKED_NOTICE) {
                self.blocked_notice = Some(now);
                self.toast(&t!("together.toast_remote_blocked"));
            }
            return;
        }
        let viewport = self.page_viewport.or_else(|| self.cast.viewport());
        let (Some(input), Some(viewport)) = (remote::Input::parse(json), viewport) else { return };
        let before = self.remote.active(now).map(str::to_owned);
        match self.remote.translate(member, &input, viewport, self.menu_open, now) {
            Outcome::Dispatch(calls) => {
                for (method, params) in calls {
                    devtools::send(&self.webview, method, &params.to_string());
                }
                let fresh = before.as_deref() != Some(member);
                let due = self.using_notices.get(member).is_none_or(|at| now.duration_since(*at) >= USING_NOTICE);
                if fresh && due {
                    self.using_notices.insert(member.to_owned(), now);
                    if let Some(name) = self.together.member_name(member) {
                        self.toast(&t!("together.toast_using", name = name));
                    }
                }
            }
            Outcome::Busy(other) => {
                if self.busy_notices.get(member).is_none_or(|at| now.duration_since(*at) >= BUSY_NOTICE) {
                    self.busy_notices.insert(member.to_owned(), now);
                    let by = self.together.member_name(&other).unwrap_or_default();
                    self.cast.send(&serde_json::json!({ "t": "busy", "id": member, "by": by }).to_string());
                }
            }
            Outcome::Dropped => {}
        }
    }

    /// Whatever a guest holds down on the window is let go.
    fn release_remote(&mut self, member: &str) {
        for (method, params) in self.remote.release(member) {
            devtools::send(&self.webview, method, &params.to_string());
        }
    }

    /// Guest: the host's window takes the player's place (the page it was
    /// on comes back afterwards).
    fn show_stream(&mut self) {
        let url = self.current_url();
        if !is_stream_url(&url) {
            if !url.is_empty() {
                self.stream_return = Some(url);
            }
            self.navigate(app_webview::STREAM_URL, Agent::Chrome);
        }
        if self.hidden || self.window.is_minimized() {
            self.show();
        }
    }

    fn leave_stream(&mut self) {
        let back = self.stream_return.take();
        if !is_stream_url(&self.current_url()) {
            return;
        }
        match back.and_then(|u| url::Url::parse(&u).ok()) {
            Some(url) => self.open_url(url),
            None => self.navigate(services::HOME_URL, Agent::Chrome),
        }
    }

    fn push_stream(&self, json: &str) {
        let script = format!(
            "location.origin === 'https://ytmp.localhost' && location.pathname === '/stream' \
             && window.__ytmpStream && window.__ytmpStream(\"{}\", {json})",
            self.keys.host
        );
        let _ = self.webview.evaluate_script(&script);
    }

    // ------------------------------------------------------------------
    // Hardware acceleration
    // ------------------------------------------------------------------

    /// WebView2 takes it only when it starts: offers to restart now.
    fn toggle_hardware_acceleration(&mut self) {
        self.settings.hardware_acceleration = !self.settings.hardware_acceleration;
        self.save_settings();
        self.state_changed();
        let question = Question {
            title: &t!("dialog.restart_title"),
            heading: &if self.settings.hardware_acceleration {
                t!("dialog.restart_gpu_on")
            } else {
                t!("dialog.restart_gpu_off")
            },
            text: &t!("dialog.restart_text"),
            yes: &t!("dialog.restart_now"),
            no: &t!("dialog.restart_later"),
        };
        if !security::ask(self.dialog_owner(), &question) {
            self.toast(&t!("app.applies_on_restart"));
            return;
        }
        self.restart();
    }

    /// Opens a new copy (which waits for this one) and quits.
    fn restart(&mut self) {
        match platform::relaunch() {
            Ok(()) => {
                let _ = self.proxy.send_event(UserEvent::Command(Command::Quit));
            }
            Err(err) => self.toast(&t!("app.restart_failed", error = err)),
        }
    }

    /// A setting set from the Settings window: only what differs changes
    /// (the same paths as the side bar and the tray, with their notices).
    fn set(&mut self, setting: Setting) {
        let command = match setting {
            Setting::AlwaysOnTop(on) => (self.settings.always_on_top != on).then_some(Command::TogglePin),
            Setting::Ghost(on) => (self.ghost != on).then_some(Command::ToggleGhost),
            Setting::VideoOnly(on) => (self.settings.video_only != on).then_some(Command::ToggleVideoOnly),
            Setting::FullscreenInWindow(on) => {
                if self.settings.fullscreen_in_window != on {
                    self.settings.fullscreen_in_window = on;
                    self.save_settings();
                    self.push_state();
                }
                None
            }
            Setting::TaskbarBar(on) => (self.settings.taskbar_bar != on).then_some(Command::ToggleTaskbarBar),
            Setting::Autostart(on) => (self.autostart != on).then_some(Command::ToggleAutostart),
            Setting::StartWithLast(on) => Some(Command::SetStartWithLast(on)),
            Setting::HardwareAcceleration(on) => {
                // The window says it applies on restart and offers to do
                // it (no dialog on top of it).
                if self.settings.hardware_acceleration != on {
                    self.settings.hardware_acceleration = on;
                    self.save_settings();
                    self.state_changed();
                }
                None
            }
        };
        if let Some(command) = command {
            self.handle(command);
        }
        // Also when nothing changed: the window shows the real state.
        self.push_settings(false);
    }

    /// Dialogs open above the Settings window when it is showing.
    fn dialog_owner(&self) -> isize {
        #[cfg(windows)]
        {
            if self.settings_panel.visible() {
                return self.settings_panel.hwnd();
            }
        }
        self.hwnd()
    }

    /// The Settings window's state (only if it is showing, unless `force`,
    /// and only when something changed).
    fn push_settings(&self, force: bool) {
        if !self.settings_panel.loaded() || (!force && !self.settings_panel.visible()) {
            return;
        }
        let section = self.settings_panel.take_section();
        let view = SettingsView {
            version: env!("CARGO_PKG_VERSION"),
            webview: &self.webview_version,
            section: section.map(|s| s.id()),
            pinned: self.settings.always_on_top,
            opacity: self.settings.opacity,
            ghost: self.ghost,
            video_only: self.settings.video_only,
            fullscreen_in_window: self.settings.fullscreen_in_window,
            taskbar_bar: self.settings.taskbar_bar,
            autostart: self.autostart,
            start_with_last: self.settings.start_with_last,
            tray: self.tray.is_some(),
            hardware_acceleration: self.settings.hardware_acceleration,
            hardware_acceleration_now: self.gpu_at_start,
            lock: self.settings.lock,
            overlay: self.overlay,
            together_name: &self.settings.together_name,
            together_mode: if self.settings.together_share { Mode::Share } else { Mode::Sync },
            together_active: self.together.is_active(),
            extensions: &self.extensions,
            extension_problems: &self.extension_problems,
            extensions_error: self.extensions_error.as_deref(),
            hotkeys: self.hotkeys.views(),
        };
        let Ok(json) = serde_json::to_string(&view) else { return };
        if !force && section.is_none() && *self.settings_shown.borrow() == json {
            return;
        }
        self.settings_panel.push(&json);
        *self.settings_shown.borrow_mut() = json;
    }

    // ------------------------------------------------------------------
    // Watch together
    // ------------------------------------------------------------------

    /// The address the player is on (the webview's own, not the page's say).
    fn current_url(&self) -> String {
        self.webview.url().unwrap_or_default()
    }

    fn remember_together_name(&mut self, name: &str) {
        let name = together::protocol::clean_name(name);
        if self.settings.together_name != name {
            self.settings.together_name = name;
            self.save_settings();
        }
    }

    fn apply_together(&mut self, effects: Vec<together::Effect>) {
        for effect in effects {
            match effect {
                together::Effect::Media(action) => {
                    let _ = self.webview.evaluate_script(&action.script());
                }
                together::Effect::Navigate(url) => {
                    if !self.locked {
                        self.open_url(url);
                    }
                }
                together::Effect::Toast(text) => self.toast(&text),
                together::Effect::Panel(json) => self.panel.push(&json),
                together::Effect::Share(true) => {
                    crate::diag::note("share: on");
                    if !self.cast.start(&self.webview, &mut self.web_context, &self.proxy, &self.home_state) {
                        self.toast(&t!("together.share_failed"));
                    }
                    self.update_lifecycle();
                }
                together::Effect::Share(false) => {
                    crate::diag::note("share: off");
                    for (method, params) in self.remote.release_all() {
                        devtools::send(&self.webview, method, &params.to_string());
                    }
                    self.cast.stop();
                    self.update_lifecycle();
                }
                together::Effect::Cast(json) => self.cast.send(&json),
                together::Effect::Watch(true) => {
                    crate::diag::note("watch: showing the host's window");
                    self.show_stream();
                }
                together::Effect::Watch(false) => {
                    crate::diag::note("watch: back to the own page");
                    self.leave_stream();
                }
                together::Effect::Stream(json) => self.push_stream(&json),
            }
        }
        // A session started or ended: the page reports every second (or
        // stops), and the side bar and tray say so.
        self.sync_watch();
        let shown = self.together.is_active().then(|| if self.together.is_host() { "host" } else { "guest" });
        if shown != self.together_shown {
            self.together_shown = shown;
            self.push_state();
            self.update_tray();
        }
    }

    // ------------------------------------------------------------------
    // Language
    // ------------------------------------------------------------------

    /// Switches the whole interface: tray, side bar, the app's pages (which
    /// redraw in place, so My music keeps playing), the music bar and the
    /// lock screen. The choice is remembered.
    fn set_language(&mut self, lang: Lang) {
        let changed = i18n::current() != lang;
        let not_approved = t!("app.extension_not_approved");
        i18n::set(lang);
        if self.settings.language.as_deref() != Some(lang.code()) {
            self.settings.language = Some(lang.code().to_owned());
            self.save_settings();
        }
        // Also puts the tray's check mark back if the same language was picked.
        self.update_tray();
        if changed {
            self.relocalize_extensions(&not_approved);
            self.mini.relocalize();
            self.lock_view.relocalize();
            self.panel.relocalize();
            self.settings_panel.relocalize();
            self.push_locale();
            self.push_state();
        }
    }

    /// The extension texts the app wrote (problems, names taken from
    /// `_locales`, "not approved") in the new language. Errors that come from
    /// WebView2 keep its wording.
    fn relocalize_extensions(&mut self, old_not_approved: &str) {
        // Only names and problems are needed: hashing every extension again
        // here would freeze the window for as long as that takes.
        let scan = extensions::scan_names(&self.paths.extensions);
        self.extension_problems = scan.problems;
        let not_approved = t!("app.extension_not_approved");
        for status in &mut self.extensions {
            if let Some(ext) = scan.extensions.iter().find(|e| e.folder == status.folder) {
                status.name = ext.name.clone();
            }
            if status.error.as_deref() == Some(old_not_approved) {
                status.error = Some(not_approved.clone());
            }
        }
    }

    /// Strings for the scripts in the main webview: the side bar's, plus the
    /// page's own when it is the start screen or My music.
    fn push_locale(&self) {
        let path = self
            .webview
            .url()
            .ok()
            .and_then(|u| url::Url::parse(&u).ok())
            .filter(|u| u.host_str() == Some(services::HOME_HOST))
            .map(|u| u.path().to_owned());
        let locale = Locale {
            info: i18n::language_info(),
            ui: i18n::page_strings(crate::home::UI_STRINGS),
            page: path.as_deref().and_then(crate::home::strings_for_path),
        };
        if let Ok(json) = serde_json::to_string(&locale) {
            let _ = self
                .webview
                .evaluate_script(&format!("window.__ytmpLocale && window.__ytmpLocale(\"{}\", {json})", self.keys.host));
        }
    }

    // ------------------------------------------------------------------
    // Services
    // ------------------------------------------------------------------

    /// Navigates with the given User-Agent (changed only if needed).
    fn navigate(&mut self, url: &str, agent: Agent) {
        if self.fullscreen {
            self.set_fullscreen(false);
        }
        if agent != self.current_agent {
            if let Some(ua) = self.agents.for_agent(agent) {
                app_webview::set_user_agent(&self.webview, ua);
            }
            self.current_agent = agent;
        }
        if let Err(err) = self.webview.load_url(url) {
            self.toast(&t!("app.open_failed", error = err));
        }
        // What was playing leaves with the page; the new one will report
        // (the start screen reports nothing).
        self.media = MediaState::default();
        self.update_tray();
        self.refresh_bar();
    }

    fn open_service(&mut self, service: &'static Service) {
        self.remember_service(service);
        self.navigate(service.url, service.agent);
    }

    /// A specific address: if it belongs to a service, with its User-Agent
    /// and remembering it; otherwise as Edge, which is what the engine is.
    fn open_url(&mut self, url: url::Url) {
        let agent = match services::for_url(&url) {
            Some(service) => {
                self.remember_service(service);
                service.agent
            }
            None => Agent::Edge,
        };
        self.navigate(url.as_str(), agent);
    }

    fn remember_service(&mut self, service: &Service) {
        if self.settings.last_service.as_deref() != Some(service.id) {
            self.settings.last_service = Some(service.id.to_owned());
            self.save_settings();
            self.sync_home_state();
        }
    }

    fn sync_home_state(&self) {
        if let Ok(mut state) = self.home_state.lock() {
            state.last_service = self.settings.last_service.clone();
            state.start_with_last = self.settings.start_with_last;
        }
    }

    // ------------------------------------------------------------------
    // Window
    // ------------------------------------------------------------------

    #[cfg(windows)]
    fn hwnd(&self) -> isize {
        use tao::platform::windows::WindowExtWindows;
        self.window.hwnd()
    }

    #[cfg(not(windows))]
    fn hwnd(&self) -> isize {
        0
    }

    fn effective_opacity(&self) -> u8 {
        if self.ghost {
            self.settings.opacity.min(GHOST_MAX_OPACITY)
        } else {
            self.settings.opacity
        }
    }

    /// Re-applies opacity and ghost mode (tao clears them when it changes styles).
    fn apply_effects(&self) {
        platform::apply_window_effects(self.hwnd(), self.effective_opacity(), self.ghost);
    }

    fn set_opacity(&mut self, value: u8) {
        self.settings.opacity = value.clamp(MIN_OPACITY, 100);
        self.apply_effects();
        self.save_settings();
        self.push_state();
    }

    fn set_fullscreen(&mut self, on: bool) {
        if on == self.fullscreen {
            return;
        }
        self.fullscreen = on;
        // `None` = the monitor the window is on.
        self.window.set_fullscreen(on.then_some(Fullscreen::Borderless(None)));
        self.apply_effects();
        self.push_state();
    }

    fn show(&mut self) {
        // The bar shown on request goes away; a music service's bar stays.
        self.mini.end_transient();
        self.refresh_bar();
        self.hidden = false;
        self.window.set_visible(true);
        if self.window.is_minimized() {
            self.window.set_minimized(false);
        }
        self.window.set_focus();
        self.apply_effects();
        self.update_lifecycle();
        self.update_tray();
        self.hidden_since = None;
        if self.locked {
            // Brought to the front while locked: ask for Windows Hello right away.
            self.show_lock_view();
            if self.lock_method() == LockMode::Hello {
                self.start_hello();
            }
        }
    }

    fn hide(&mut self) {
        if self.tray.is_none() {
            // Without a tray, hiding would leave the window unreachable.
            self.window.set_minimized(true);
            return;
        }
        if self.fullscreen {
            self.set_fullscreen(false);
        }
        self.hidden = true;
        self.hidden_since = Some(Instant::now());
        self.window.set_visible(false);
        self.update_lifecycle();
        self.update_tray();
    }

    fn update_lifecycle(&mut self) {
        // While the window is shared it keeps drawing, even minimized.
        let background = (self.hidden || self.window.is_minimized()) && !self.cast.active();
        self.lifecycle.update(&self.webview, background);
    }

    /// After a resize, puts the window back on the screen.
    fn keep_inside_work_area(&self) {
        let (Some(area), Ok(pos)) = (platform::work_area(self.hwnd()), self.window.outer_position()) else {
            return;
        };
        let size = self.window.outer_size();
        let x = pos.x.min(area.right - size.width as i32).max(area.left);
        let y = pos.y.min(area.bottom - size.height as i32).max(area.top);
        if (x, y) != (pos.x, pos.y) {
            self.window.set_outer_position(PhysicalPosition::new(x, y));
        }
    }

    // ------------------------------------------------------------------
    // Extensions
    // ------------------------------------------------------------------

    fn reload_extensions(&mut self) {
        let scan = extensions::scan(&self.paths.extensions);
        let approved = self.approve_extensions(&scan.extensions);
        self.extensions = scan
            .extensions
            .iter()
            .map(|ext| {
                let mut status = ExtensionStatus::pending(ext);
                if !approved.iter().any(|a| a.folder == ext.folder) {
                    status.pending = false;
                    status.error = Some(t!("app.extension_not_approved"));
                }
                status
            })
            .collect();
        self.extension_problems = scan.problems;
        self.local_extensions = scan.extensions.clone();
        if let Some(error) = &self.extensions_error {
            for status in &mut self.extensions {
                status.pending = false;
                status.error = Some(error.clone());
            }
        } else {
            // The ones not approved stay out (and are uninstalled if they were in).
            self.ext_host.sync(approved, self.extension_ids.clone());
        }
        self.push_state();
    }

    /// Only extensions the user approved, with their exact fingerprint, are
    /// loaded. New or changed ones are asked about in a system dialog (which
    /// the page can neither draw nor press).
    fn approve_extensions(&mut self, found: &[LocalExtension]) -> Vec<LocalExtension> {
        let mut changed = false;
        // Folders that no longer exist are forgotten.
        let folders: Vec<&str> = found.iter().map(|e| e.folder.as_str()).collect();
        for list in [&mut self.settings.approved_extensions, &mut self.settings.rejected_extensions] {
            let before = list.len();
            list.retain(|folder, _| folders.contains(&folder.as_str()));
            changed |= list.len() != before;
        }
        let mut approved = Vec::new();
        for ext in found {
            if self.settings.approved_extensions.get(&ext.folder) == Some(&ext.fingerprint) {
                approved.push(ext.clone());
                continue;
            }
            if self.settings.rejected_extensions.get(&ext.folder) == Some(&ext.fingerprint) {
                continue;
            }
            if self.locked {
                // Asked when unlocked.
                self.approvals_pending = true;
                continue;
            }
            let was_approved = self.settings.approved_extensions.contains_key(&ext.folder);
            let (heading, text) = extension_question(ext, was_approved);
            let question = Question {
                title: &t!("dialog.extension_title"),
                heading: &heading,
                text: &text,
                yes: &t!("dialog.extension_load"),
                no: &t!("dialog.extension_skip"),
            };
            if security::ask(self.hwnd(), &question) {
                self.settings.rejected_extensions.remove(&ext.folder);
                self.settings.approved_extensions.insert(ext.folder.clone(), ext.fingerprint.clone());
                approved.push(ext.clone());
            } else {
                self.settings.approved_extensions.remove(&ext.folder);
                self.settings.rejected_extensions.insert(ext.folder.clone(), ext.fingerprint.clone());
            }
            changed = true;
        }
        if changed {
            self.save_settings();
        }
        approved
    }

    fn on_extension(&mut self, event: ExtensionEvent) {
        match event {
            ExtensionEvent::Installed { folder, id, enabled } => {
                if let Some(status) = self.extensions.iter_mut().find(|s| s.folder == folder) {
                    status.id = Some(id.clone());
                    status.enabled = enabled;
                    status.pending = false;
                    status.error = None;
                }
                if self.extension_ids.get(&folder) != Some(&id) {
                    self.extension_ids.insert(folder, id);
                    self.save_extension_ids();
                }
            }
            ExtensionEvent::Failed { folder, error } => {
                match self.extensions.iter_mut().find(|s| s.folder == folder && !folder.is_empty()) {
                    Some(status) => {
                        status.pending = false;
                        status.error = Some(error);
                    }
                    None => self.toast(&t!("app.extension_failed", error = error)),
                }
            }
            ExtensionEvent::Toggled { id, enabled } => {
                if let Some(status) = self.extensions.iter_mut().find(|s| s.id.as_deref() == Some(id.as_str())) {
                    status.enabled = enabled;
                    let message = if enabled {
                        t!("app.extension_on", name = status.name)
                    } else {
                        t!("app.extension_off", name = status.name)
                    };
                    self.toast(&message);
                }
            }
            ExtensionEvent::Removed { id } => {
                let before = self.extension_ids.len();
                self.extension_ids.retain(|_, v| *v != id);
                if self.extension_ids.len() != before {
                    self.save_extension_ids();
                }
            }
            ExtensionEvent::Unsupported(error) => {
                let message = t!("app.extensions_unsupported", error = error);
                for status in &mut self.extensions {
                    status.pending = false;
                    status.error = Some(message.clone());
                }
                self.extensions_error = Some(message);
            }
        }
        self.push_state();
    }

    // ------------------------------------------------------------------
    // Persistence and page
    // ------------------------------------------------------------------

    fn save_settings(&self) {
        if let Err(err) = self.settings.save(&self.store, &self.paths.settings) {
            eprintln!("could not save the settings: {err}");
        }
    }

    fn save_extension_ids(&self) {
        if let Err(err) = config::save_extension_ids(&self.store, &self.paths.extension_ids, &self.extension_ids) {
            eprintln!("could not save extensions.enc: {err}");
        }
        if let Ok(mut allowed) = self.allowed_extensions.lock() {
            *allowed = self.extension_ids.values().cloned().collect();
        }
    }

    fn on_music(&mut self, request: crate::music::Request) {
        use crate::music::{os, Request};
        let Some(music) = &self.music else {
            self.toast(&t!("app.music_unavailable"));
            return;
        };
        match request {
            // The system dialog runs on the window's thread (modal).
            Request::PickFolder => {
                if let Some(path) = os::pick_folder(self.hwnd(), &t!("music.pick_folder_title")) {
                    music.request(Request::Import(path));
                }
            }
            other => music.request(other),
        }
    }

    /// tao ends the process with `exit()` without running destructors, so
    /// everything is saved here and the tray icon removed (otherwise a ghost
    /// icon would stay until the mouse passes over it).
    fn save_on_exit(&mut self) {
        // The others are told (the host's guests see "ended" at once).
        self.together.shutdown();
        self.cast.stop();
        // The library is saved (and downloads stopped) before quitting.
        if let Some(music) = &self.music {
            music.shutdown(Duration::from_secs(3));
        }
        if !self.fullscreen {
            if let Some(geometry) = app_window::current_geometry(&self.window) {
                if let Err(err) = geometry.save(&self.store, &self.paths.window_state) {
                    eprintln!("could not save the window geometry: {err}");
                }
            }
        }
        self.save_settings();
        topmost::set_active(false);
        self.mini.close();
        self.tray = None;
        crate::diag::finish();
    }

    /// The guard only works with "always on top" on and the window showing.
    fn update_guard(&self) {
        topmost::set_active(self.settings.always_on_top && !self.hidden);
    }

    fn update_tray(&self) {
        self.update_guard();
        if let Some(tray) = &self.tray {
            // While locked, the tray does not tell what is playing.
            let media = if self.locked { MediaState::default() } else { self.media.clone() };
            tray.update(&TrayState {
                visible: !self.hidden,
                pinned: self.settings.always_on_top,
                taskbar_bar: self.settings.taskbar_bar,
                lock: self.settings.lock,
                locked: self.locked,
                ghost: self.ghost,
                autostart: self.autostart,
                has_media: media.has_media,
                playing: media.playing,
                can_next: media.can_next,
                can_previous: media.can_previous,
                now_playing: crate::media::truncate(&media.title, crate::tray::MENU_TITLE_CHARS),
                tooltip: media.tooltip(),
                language: i18n::current(),
                together: self.together.is_active(),
                hardware_acceleration: self.settings.hardware_acceleration,
            });
        }
        self.push_settings(false);
    }

    // ------------------------------------------------------------------
    // Playback and music bar
    // ------------------------------------------------------------------

    fn media_action(&mut self, action: MediaAction) {
        let _ = self.webview.evaluate_script(&action.script());
        if self.hidden || self.window.is_minimized() {
            // Hidden: the bar appears for a few seconds as feedback, without
            // taking the focus from anything.
            self.open_bar(mini::STAY_AFTER_HOTKEY, None);
            return;
        }
        // Showing: a short notice for what is not obvious at a glance.
        let percent = |delta: i16| (i16::from(self.media.volume_percent()) + delta).clamp(0, 100);
        let message = match action {
            MediaAction::VolumeBy(delta) => Some(t!("app.volume", value = percent(i16::from(delta)))),
            MediaAction::ToggleMute if self.media.muted => Some(t!("app.unmuted")),
            MediaAction::ToggleMute => Some(t!("app.muted")),
            MediaAction::SeekBy(s) if s > 0 => Some(t!("app.seek_forward", seconds = s)),
            MediaAction::SeekBy(s) => Some(t!("app.seek_back", seconds = s.unsigned_abs())),
            _ => None,
        };
        if let Some(message) = message {
            if self.media.has_media {
                self.toast(&message);
            }
        }
    }

    /// Creates the bar's webview if it does not exist yet.
    fn prepare_bar(&mut self) -> bool {
        self.mini.ensure_webview(&self.webview, &mut self.web_context, &self.proxy, &self.home_state)
    }

    /// Works out the bar: pinned if a music service is open (and it was not
    /// closed by hand), and where and whether it shows.
    fn refresh_bar(&mut self) {
        let source = services::music_source(&self.media.site);
        // Locked: no bar (it would show what is playing).
        if self.locked {
            self.mini.set_pinned(false);
            self.mini.close();
            self.sync_watch();
            return;
        }
        let mut pinned = self.settings.taskbar_bar && source.is_some() && !self.bar_dismissed;
        if pinned && !self.prepare_bar() {
            pinned = false;
        }
        self.mini.set_pinned(pinned);
        // Outside the catalog and the known music sites, the bar shows the
        // domain: so it cannot pass itself off as something else.
        let known = source.is_some()
            || url::Url::parse(&format!("https://{}/", self.media.site)).ok().and_then(|u| services::for_url(&u)).is_some();
        let origin = (!known && !self.media.site.is_empty()).then(|| self.media.site.clone());
        self.mini.set_media(&self.media, source, origin);
        self.mini.update(Instant::now());
        self.sync_watch();
    }

    /// Shows it on request for a few seconds (tray click, shortcuts).
    fn open_bar(&mut self, stay: Duration, anchor: Option<(i32, i32)>) {
        if !self.prepare_bar() {
            self.toast(&t!("app.bar_failed"));
            return;
        }
        self.bar_dismissed = false;
        self.mini.show_for(stay, anchor);
        self.refresh_bar();
    }

    /// Right click on the bar (or a tray click with the bar open).
    fn close_bar(&mut self) {
        if self.mini.is_pinned() {
            self.bar_dismissed = true;
        }
        self.mini.close();
        self.refresh_bar();
    }

    /// While the bar shows, the page reports the position every second. The
    /// report says whether the page is in that mode: if it does not match
    /// (new page, reload), it is asked again.
    fn sync_watch(&mut self) {
        // Watch together needs the position every second too.
        let wanted = self.mini.is_visible() || self.together.is_active();
        if self.media.watching != wanted {
            self.media.watching = wanted;
            let script = format!("window.__ytmpMediaWatch && window.__ytmpMediaWatch({wanted})");
            let _ = self.webview.evaluate_script(&script);
        }
    }

    /// Next moment there is something to do (check the bar).
    pub fn deadline(&self) -> Option<Instant> {
        [self.mini.deadline(), self.auto_lock_at(), self.together.deadline(), self.cast.sound_deadline()]
            .into_iter()
            .flatten()
            .min()
    }

    /// When it locks itself for having been hidden a long time.
    fn auto_lock_at(&self) -> Option<Instant> {
        let enabled = self.settings.lock != LockMode::Off && !self.locked;
        self.hidden_since.filter(|_| enabled).map(|since| since + LOCK_AFTER_HIDDEN)
    }

    pub fn on_timer(&mut self) {
        let now = Instant::now();
        self.mini.on_timer(now);
        if self.together.deadline().is_some_and(|at| now >= at) {
            let effects = self.together.on_timer();
            self.apply_together(effects);
        }
        if self.cast.sound_deadline().is_some_and(|at| now >= at) {
            // Is the shared sound heard while the page plays?
            let webview = &self.webview;
            if let Some(sound) = self.cast.check_sound(now, || devtools::playing_audio(webview)) {
                let effects = self.together.set_sound(sound);
                self.apply_together(effects);
            }
        }
        self.sync_watch();
        if self.auto_lock_at().is_some_and(|at| now >= at) {
            self.lock_now();
        }
    }

    pub fn main_window_id(&self) -> tao::window::WindowId {
        self.window.id()
    }

    pub fn mini_window_id(&self) -> tao::window::WindowId {
        self.mini.id()
    }

    pub fn together_window_id(&self) -> tao::window::WindowId {
        self.panel.id()
    }

    pub fn settings_window_id(&self) -> tao::window::WindowId {
        self.settings_panel.id()
    }

    fn state_changed(&self) {
        self.update_tray();
        self.push_state();
    }

    fn push_state(&self) {
        let state = PageState {
            version: env!("CARGO_PKG_VERSION"),
            pinned: self.settings.always_on_top,
            video_only: self.settings.video_only,
            fullscreen_in_window: self.settings.fullscreen_in_window,
            opacity: self.settings.opacity,
            ghost: self.ghost,
            fullscreen: self.fullscreen,
            autostart: self.autostart,
            tray: self.tray.is_some(),
            extensions: &self.extensions,
            extension_problems: &self.extension_problems,
            extensions_error: self.extensions_error.as_deref(),
            hotkeys: self.hotkeys.views(),
            last_service: self.settings.last_service.as_deref(),
            start_with_last: self.settings.start_with_last,
            overlay: self.overlay,
            together: self.together_shown,
            hardware_acceleration: self.settings.hardware_acceleration,
        };
        if let Ok(json) = serde_json::to_string(&state) {
            let _ = self
                .webview
                .evaluate_script(&format!("window.__ytmpSync && window.__ytmpSync(\"{}\", {json})", self.keys.host));
        }
        self.push_settings(false);
    }

    fn toast(&self, message: &str) {
        if let Ok(json) = serde_json::to_string(message) {
            let _ = self
                .webview
                .evaluate_script(&format!("window.__ytmpToast && window.__ytmpToast(\"{}\", {json})", self.keys.host));
        }
    }

    // ------------------------------------------------------------------
    // App lock
    // ------------------------------------------------------------------

    /// The method used to verify now (at the current step).
    fn lock_method(&self) -> LockMode {
        match self.flow.map(|f| f.stage) {
            Some(Stage::NewPin) => LockMode::Pin,
            Some(Stage::ConfirmHello) => LockMode::Hello,
            _ => self.settings.lock,
        }
    }

    fn lock_now(&mut self) {
        if self.settings.lock == LockMode::Off || self.locked {
            return;
        }
        self.locked = true;
        self.flow = Some(Flow { goal: Goal::Unlock, stage: Stage::Verify });
        self.lock_error = None;
        // Nothing keeps playing or stays on screen (watch together included:
        // nothing plays behind the lock screen, and its window hides).
        let _ = self.webview.evaluate_script(&MediaAction::Pause.script());
        self.panel.hide();
        self.settings_panel.hide();
        let effects = self.together.set_locked(true);
        self.apply_together(effects);
        if self.ghost {
            self.ghost = false;
            self.apply_effects();
        }
        if self.fullscreen {
            self.set_fullscreen(false);
        }
        let _ = self.webview.set_visible(false);
        self.refresh_bar();
        if !self.show_lock_view() {
            // Without a lock screen there would be no way to unlock: rather
            // than leaving the app unusable, it does not lock and says so.
            self.locked = false;
            self.flow = None;
            let _ = self.webview.set_visible(true);
            self.toast(&t!("app.lock_screen_failed"));
        }
        self.update_tray();
    }

    fn show_lock_view(&mut self) -> bool {
        let ready = self.lock_view.ensure(&self.window, &self.webview, &mut self.web_context, &self.proxy, &self.home_state);
        if ready {
            let _ = self.webview.set_visible(false);
            self.lock_view.show(&self.window);
            self.push_lock_screen();
        }
        ready
    }

    /// Forgotten PIN: it can be removed by verifying with Windows Hello
    /// (Windows confirms the identity). Without Windows Hello, the only way
    /// out is clearing the sessions: whoever does not know the PIN takes
    /// nothing away.
    fn forgot_pin(&mut self) {
        if !self.locked || self.settings.lock != LockMode::Pin || self.hello_busy || self.pin_busy {
            return;
        }
        self.hello_busy = true;
        self.lock_error = None;
        self.push_lock_screen();
        let hwnd = self.hwnd();
        let proxy = self.proxy.clone();
        let message = t!("lock.hello_prompt_forgot");
        std::thread::spawn(move || {
            let result = lock::hello_verify(hwnd, &message);
            let _ = proxy.send_event(UserEvent::Command(Command::ForgotResult(result)));
        });
    }

    fn reset_forgotten_pin(&mut self) {
        let question = Question {
            title: &t!("dialog.forgot_title"),
            heading: &t!("dialog.forgot_heading"),
            text: &t!("dialog.forgot_text"),
            yes: &t!("dialog.forgot_confirm"),
            no: &t!("common.cancel"),
        };
        if !security::ask(self.hwnd(), &question) {
            self.push_lock_screen();
            return;
        }
        // The PIN is only removed once clearing has succeeded.
        self.pin_reset_pending = true;
        let proxy = self.proxy.clone();
        app_webview::clear_browsing_data(&self.webview, move |ok| {
            let _ = proxy.send_event(UserEvent::Command(Command::BrowsingDataCleared(ok)));
        });
    }

    fn push_lock_screen(&self) {
        let Some(flow) = self.flow else { return };
        let method = self.lock_method();
        let stage = match (flow.goal, flow.stage) {
            (_, Stage::NewPin) => "new-pin",
            (Goal::Unlock, _) => "unlock",
            _ => "verify",
        };
        let wait = if method == LockMode::Pin && flow.stage == Stage::Verify {
            self.pin_wait_left()
        } else {
            0
        };
        self.lock_view.push(&Screen {
            stage,
            method: if method == LockMode::Pin { "pin" } else { "hello" },
            busy: self.hello_busy || self.pin_busy,
            error: self.lock_error.clone(),
            wait,
            can_cancel: flow.goal != Goal::Unlock,
        });
    }

    /// Change the lock mode (from the tray). Picking the mode already set
    /// turns it off. If there is a lock, the user must verify first.
    fn begin_change(&mut self, chosen: LockMode) {
        let target = if chosen == self.settings.lock { LockMode::Off } else { chosen };
        if target == LockMode::Off && self.settings.lock == LockMode::Off {
            self.update_tray();
            return;
        }
        let stage = if self.settings.lock != LockMode::Off {
            Stage::Verify
        } else if target == LockMode::Pin {
            Stage::NewPin
        } else {
            Stage::ConfirmHello
        };
        self.flow = Some(Flow { goal: Goal::Change(target), stage });
        self.lock_error = None;
        if self.hidden || self.window.is_minimized() {
            self.show();
        }
        self.show_lock_view();
        if self.lock_method() == LockMode::Hello {
            self.start_hello();
        }
        self.update_tray();
    }

    fn start_hello(&mut self) {
        let Some(flow) = self.flow else { return };
        if self.hello_busy || self.lock_method() != LockMode::Hello || flow.stage == Stage::NewPin {
            return;
        }
        self.hello_busy = true;
        self.lock_error = None;
        self.push_lock_screen();
        let hwnd = self.hwnd();
        let proxy = self.proxy.clone();
        let message = if flow.goal == Goal::Unlock { t!("lock.hello_prompt_unlock") } else { t!("lock.hello_prompt_change") };
        std::thread::spawn(move || {
            let result = lock::hello_verify(hwnd, &message);
            let _ = proxy.send_event(UserEvent::Command(Command::HelloResult(result)));
        });
    }

    /// Seconds before another PIN may be tried: the saved moment (it
    /// survives a restart) and, in this run, the monotonic clock (moving the
    /// system clock forward does not skip the wait). The larger one counts.
    fn pin_wait_left(&self) -> u64 {
        let saved = lock::remaining(self.settings.lock_blocked_until, lock::unix_now());
        let here = self.pin_wait.map_or(0, |(since, secs)| secs.saturating_sub(since.elapsed().as_secs()));
        saved.max(here)
    }

    fn check_pin(&mut self, pin: zeroize::Zeroizing<String>) {
        let verifying = self.flow.is_some_and(|f| f.stage == Stage::Verify) && self.settings.lock == LockMode::Pin;
        if !verifying || self.pin_busy {
            return;
        }
        if self.pin_wait_left() > 0 {
            self.push_lock_screen();
            return;
        }
        let Some(hash) = self.settings.pin_hash.clone() else { return };
        self.pin_busy = true;
        self.lock_error = None;
        self.push_lock_screen();
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let ok = lock::verify_pin(&pin, &hash);
            let _ = proxy.send_event(UserEvent::Command(Command::PinChecked(ok)));
        });
    }

    fn set_new_pin(&mut self, pin: zeroize::Zeroizing<String>) {
        if self.flow.map(|f| f.stage) != Some(Stage::NewPin) || self.pin_busy {
            return;
        }
        if let Err(reason) = lock::check_pin(&pin) {
            self.lock_failed(&i18n::text(reason));
            return;
        }
        self.pin_busy = true;
        self.lock_error = None;
        self.push_lock_screen();
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let result = lock::hash_pin(&pin);
            let _ = proxy.send_event(UserEvent::Command(Command::PinHashed(result)));
        });
    }

    /// Verification succeeded: on to the next step.
    fn verified(&mut self) {
        let Some(flow) = self.flow else { return };
        self.lock_error = None;
        match (flow.goal, flow.stage) {
            (Goal::Unlock, _) => self.unlock(),
            (Goal::Change(target), Stage::Verify) => match target {
                LockMode::Off => self.apply_lock(LockMode::Off, None),
                LockMode::Pin => {
                    self.flow = Some(Flow { goal: flow.goal, stage: Stage::NewPin });
                    self.push_lock_screen();
                }
                LockMode::Hello => {
                    self.flow = Some(Flow { goal: flow.goal, stage: Stage::ConfirmHello });
                    self.push_lock_screen();
                    self.start_hello();
                }
            },
            (Goal::Change(_), Stage::ConfirmHello) => self.apply_lock(LockMode::Hello, None),
            (Goal::Change(_), Stage::NewPin) => {}
        }
    }

    fn lock_failed(&mut self, message: &str) {
        self.lock_error = Some(message.to_owned());
        self.push_lock_screen();
    }

    fn apply_lock(&mut self, mode: LockMode, pin_hash: Option<String>) {
        self.settings.lock = mode;
        self.settings.pin_hash = pin_hash;
        self.settings.lock_failures = 0;
        self.settings.lock_blocked_until = 0;
        self.pin_wait = None;
        // A new marker: settings from before (a backup with the lock off)
        // no longer match the profile.
        if mode != LockMode::Off {
            if let Err(err) = crate::config::bind_profile(&self.paths.profile, &mut self.settings) {
                eprintln!("could not renew the profile marker: {err}");
            }
        }
        self.save_settings();
        if !self.locked {
            self.end_flow();
        }
        let keys = self.hotkeys.keys_for(Action::Lock);
        self.toast(&match (mode, keys) {
            (LockMode::Off, _) => t!("app.lock_off"),
            (LockMode::Hello, Some(keys)) => t!("app.lock_hello_on", keys = keys),
            (LockMode::Hello, None) => t!("app.lock_hello_on_tray"),
            (LockMode::Pin, Some(keys)) => t!("app.lock_pin_on", keys = keys),
            (LockMode::Pin, None) => t!("app.lock_pin_on_tray"),
        });
    }

    fn end_flow(&mut self) {
        if self.locked {
            return;
        }
        self.flow = None;
        self.lock_error = None;
        self.lock_view.hide();
        let _ = self.webview.set_visible(true);
        let _ = self.webview.focus();
        self.update_tray();
    }

    fn unlock(&mut self) {
        self.locked = false;
        let effects = self.together.set_locked(false);
        self.apply_together(effects);
        self.end_flow();
        self.hidden_since = if self.hidden { Some(Instant::now()) } else { None };
        self.refresh_bar();
        self.update_tray();
        if std::mem::take(&mut self.approvals_pending) {
            self.reload_extensions();
        }
    }

    // ------------------------------------------------------------------
    // Accounts and protection status
    // ------------------------------------------------------------------

    /// Signs out of every service: clears cookies, storage and the profile
    /// cache (after confirming in a system dialog).
    fn sign_out_all(&mut self) {
        let question = Question {
            title: &t!("dialog.sign_out_title"),
            heading: &t!("dialog.sign_out_heading"),
            text: &t!("dialog.sign_out_text"),
            yes: &t!("dialog.sign_out_confirm"),
            no: &t!("common.cancel"),
        };
        if !security::ask(self.dialog_owner(), &question) {
            return;
        }
        let proxy = self.proxy.clone();
        app_webview::clear_browsing_data(&self.webview, move |ok| {
            let _ = proxy.send_event(UserEvent::Command(Command::BrowsingDataCleared(ok)));
        });
    }

    fn show_security_status(&self) {
        let key = match crate::vault::key_protection(&self.paths.vault_key) {
            Some(crate::vault::KeyProtection::DeviceAndAccount) => t!("security.key_tpm"),
            _ if crate::vault::hardware_protector().is_some() => t!("security.key_tpm_pending"),
            _ => t!("security.key_no_tpm"),
        };
        let mut paragraphs = vec![
            t!("security.passwords"),
            t!("security.cookies"),
            if self.music.is_some() { t!("security.app_data_music", key = key) } else { t!("security.app_data", key = key) },
            t!("security.disk"),
        ];
        paragraphs.extend(self.legacy_efs.describe());
        paragraphs.push(match self.settings.lock {
            LockMode::Off => t!("security.lock_off"),
            LockMode::Hello => t!("security.lock_hello"),
            LockMode::Pin => t!("security.lock_pin"),
        });
        paragraphs.push(if self.overlay { t!("security.browsing_overlay") } else { t!("security.browsing") });
        paragraphs.push(t!("security.extensions", count = self.settings.approved_extensions.len()));
        security::inform(self.dialog_owner(), &t!("dialog.security_title"), &t!("dialog.security_heading"), &paragraphs.join("\n\n"));
    }
}

/// Heading and text of the approval dialog for an extension.
fn extension_question(ext: &LocalExtension, changed: bool) -> (String, String) {
    let heading = if changed {
        t!("dialog.extension_changed", name = ext.name, version = ext.version)
    } else {
        t!("dialog.extension_new", name = ext.name, version = ext.version)
    };
    let mut text = String::new();
    if ext.all_sites {
        text.push_str(&format!("⚠ {}\n\n", t!("dialog.extension_all_sites")));
    }
    if ext.permissions.is_empty() {
        text.push_str(&t!("dialog.extension_no_permissions"));
        text.push('\n');
    } else {
        text.push_str(&t!("dialog.extension_permissions"));
        text.push('\n');
        for permission in ext.permissions.iter().take(12) {
            text.push_str(&format!("  • {permission}\n"));
        }
        if ext.permissions.len() > 12 {
            text.push_str(&format!("  • {}\n", t!("dialog.extension_more", count = ext.permissions.len() - 12)));
        }
    }
    text.push('\n');
    text.push_str(&t!("dialog.extension_folder", folder = format!("extensions\\{}", ext.folder)));
    text.push('\n');
    text.push_str(&t!("dialog.extension_fingerprint", fingerprint = format!("{}…", &ext.fingerprint[..16])));
    text.push_str("\n\n");
    text.push_str(&t!("dialog.extension_trust"));
    (heading, text)
}

/// The page with the host's window?
fn is_stream_url(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| u.host_str() == Some(services::HOME_HOST) && u.path() == "/stream")
}

fn resize_direction(edge: Edge) -> ResizeDirection {
    match edge {
        Edge::North => ResizeDirection::North,
        Edge::South => ResizeDirection::South,
        Edge::East => ResizeDirection::East,
        Edge::West => ResizeDirection::West,
        Edge::NorthEast => ResizeDirection::NorthEast,
        Edge::NorthWest => ResizeDirection::NorthWest,
        Edge::SouthEast => ResizeDirection::SouthEast,
        Edge::SouthWest => ResizeDirection::SouthWest,
    }
}
