//! Protocol between the page and the host, and the app's internal commands.
//!
//! The page can only send the messages in [`parse`]: a closed list.
//! Anything else is ignored, so YouTube (or any script on the page) cannot
//! trigger actions outside this set. Commands that must only come from the
//! system (tray, global shortcuts) have no message.

/// Edge or corner a resize starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    North,
    South,
    East,
    West,
    NorthEast,
    NorthWest,
    SouthEast,
    SouthWest,
}

/// Corner of the monitor's work area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

/// Preset window sizes (16:9, in logical pixels).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizePreset {
    Small,
    Medium,
    Large,
}

impl SizePreset {
    pub fn logical_size(self) -> (f64, f64) {
        match self {
            SizePreset::Small => (480.0, 270.0),
            SizePreset::Medium => (640.0, 360.0),
            SizePreset::Large => (960.0, 540.0),
        }
    }
}

pub use crate::media::{MediaAction, MediaState};

/// Phases of dragging the music bar along the taskbar. The host reads the
/// position from the cursor (the page does not send it).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DragPhase {
    Start,
    Move,
    End,
}

/// A section of the Settings window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsSection {
    General,
    Window,
    Together,
    Privacy,
    Extensions,
    Advanced,
}

impl SettingsSection {
    pub const ALL: [SettingsSection; 6] = [
        SettingsSection::General,
        SettingsSection::Window,
        SettingsSection::Together,
        SettingsSection::Privacy,
        SettingsSection::Extensions,
        SettingsSection::Advanced,
    ];

    /// The id the pages use (`settings:extensions`, `#extensions`).
    pub fn id(self) -> &'static str {
        match self {
            SettingsSection::General => "general",
            SettingsSection::Window => "window",
            SettingsSection::Together => "together",
            SettingsSection::Privacy => "privacy",
            SettingsSection::Extensions => "extensions",
            SettingsSection::Advanced => "advanced",
        }
    }

    pub fn parse(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|s| s.id() == id)
    }
}

/// A setting and the value wanted (not a toggle: two quick clicks in the
/// Settings window cannot end up the other way round).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Setting {
    AlwaysOnTop(bool),
    Ghost(bool),
    VideoOnly(bool),
    FullscreenInWindow(bool),
    TaskbarBar(bool),
    Autostart(bool),
    StartWithLast(bool),
    HardwareAcceleration(bool),
}

/// Everything the app can do. Comes from the page (IPC), the system tray,
/// the music bar or the global shortcuts.
#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    // Window
    Drag,
    Resize(Edge),
    TogglePin,
    SetFullscreen(bool),
    SetOpacity(u8),
    AdjustOpacity(i8),
    ResetOpacity,
    ToggleGhost,
    Size(SizePreset),
    Corner(Corner),
    // Visibility and lifecycle
    Show,
    Hide,
    ToggleVisible,
    Quit,
    // Services
    Home,
    OpenService(String),
    OpenUrl(String),
    SetStartWithLast(bool),
    // Page
    Ready,
    ToggleVideoOnly,
    OpenClipboard,
    Media(MediaAction),
    /// What is playing now (media.js report).
    MediaReport(Box<MediaState>),
    // Tray and music bar
    /// Left click on the icon; the icon's position in physical pixels.
    TrayClick(i32, i32),
    TrayDoubleClick,
    /// "Show the controls" in the tray menu.
    MiniShow,
    MiniHide,
    MiniHover(bool),
    MiniReady,
    MiniDrag(DragPhase),
    /// Setting: music bar in the taskbar while a music service is open.
    ToggleTaskbarBar,
    // Extensions
    ExtensionsFolder,
    ExtensionsReload,
    ExtensionToggle(String),
    ExtensionOptions(String),
    // System
    ToggleAutostart,
    /// Interface language (start screen, side bar menu, tray).
    SetLanguage(crate::i18n::Lang),
    // Navigation (from webview.rs's filters; the page cannot ask for these)
    /// Navigate the main window (http upgraded to https, a new window of a
    /// catalog service).
    Navigate(String),
    /// Open in the default browser (a new window of another site).
    OpenExternal(String),
    /// Something was blocked; the i18n key of the reason, for the notice.
    Blocked(String),
    // Security (tray and lock screen; never from the page)
    LockNow,
    /// Pick the lock mode (the one already set = turn it off).
    SetLock(crate::config::LockMode),
    SignOutAll,
    SecurityStatus,
    LockReady,
    LockHello,
    LockPin(String),
    LockNewPin(String),
    LockCancel,
    /// "Forgot your PIN?" on the lock screen.
    LockForgot,
    /// Result of verifying with Windows Hello to remove a forgotten PIN.
    ForgotResult(Result<bool, crate::lock::HelloError>),
    /// Results from worker threads.
    HelloResult(Result<bool, crate::lock::HelloError>),
    PinChecked(bool),
    PinHashed(Result<String, String>),
    BrowsingDataCleared(bool),
    // My music (only from the https://ytmp.localhost/music page)
    Music(crate::music::Request),
    // Watch together. Opening it comes from the side bar or the tray; the
    // rest only from its own window (`parse_together`).
    TogetherOpen,
    TogetherReady,
    /// Start a session as host, with this display name.
    TogetherStart(String),
    /// Join with a code (as typed) and a display name.
    TogetherJoin(String, String),
    /// Leave (guest) or end the session (host).
    TogetherLeave,
    /// "OK" on the "session over" screen.
    TogetherDismiss,
    TogetherKick(String),
    TogetherNewCode,
    TogetherCopy,
    /// "Share my window" or "Sync players" (remembered for next time).
    TogetherMode(crate::together::protocol::Mode),
    /// Host: let a guest use the window (or pause and seek), or not.
    TogetherMemberControl(String, bool),
    /// Host: "Not now" on a guest's request to use the window.
    TogetherDismissAsk(String),
    /// Guest: back to the host's window (after leaving its page).
    TogetherWatch,
    TogetherResync,
    /// Open the host's page by hand (a site outside the catalog).
    TogetherOpenSite,
    /// The relay's address (when the build has none).
    TogetherRelay(String),
    /// A game in exclusive full screen left the front (topmost.rs): what to
    /// change in it so the player can stay above it.
    GameHint(crate::topmost::GameHint),
    /// What a service page's self-check found (diary; a warning once if
    /// something the app relies on is missing).
    PageCheck(Box<crate::diag::PageCheck>),
    /// Host: share all of the PC's sound (`true`) or only the app's, for
    /// this sharing session.
    TogetherSound(bool),
    TogetherClose,
    // Sending the window (only from its hidden page, `parse_cast`; the
    // sound's state comes from the capture thread)
    CastReady,
    /// The page can send sound.
    CastCaps(bool),
    /// Capture number N of the window's sound works (or could not start).
    CastSound(u32, bool),
    /// The connection offer for a guest (member, SDP).
    CastOffer(String, String),
    /// How a guest's connection is going (member, `connectionState`).
    CastState(String, String),
    /// A guest's mouse or key (member, JSON), checked in remote.rs.
    CastInput(String, String),
    /// How sending goes (JSON: pictures, sound, each connection), for the
    /// diary every few seconds.
    CastStats(String),
    // Seeing the host's window (only from https://ytmp.localhost/stream)
    StreamReady,
    StreamAnswer(String),
    /// `connecting`, `watching` or `failed`.
    StreamState(String),
    StreamAsk,
    StreamLeave,
    /// How watching goes (JSON: pictures, sound, connection), for the diary.
    StreamStats(String),
    // Side bar
    /// The app's menu (or help) is open or closed: guests cannot click on it.
    MenuOpen(bool),
    /// The page's size in CSS px (ui.js, on load and on every resize).
    Viewport(u32, u32),
    /// Setting: let WebView2 use the graphics card (applies on restart).
    ToggleHardwareAcceleration,
    // Settings window. Opening it comes from the start screen, the side
    // bar, the tray or Ctrl+,; the rest only from its own page
    // (`parse_settings`).
    SettingsOpen(Option<SettingsSection>),
    SettingsReady,
    SettingsClose,
    /// A setting set to a value.
    Set(Setting),
    /// The lock mode picked in Settings (the lock screen asks to verify).
    SettingsLock(crate::config::LockMode),
    /// The name the others see in Watch together.
    TogetherName(String),
    /// The first-run welcome again, on the start screen.
    ShowWelcome,
    /// The welcome was finished or skipped (start screen).
    WelcomeDone,
    /// The keyboard shortcuts (the help in the player).
    ShowHelp,
    /// The app's data folder (where the session diary is).
    OpenDataFolder,
    /// Restart now (a setting that applies on restart, from Settings).
    Restart,
}

/// Longest statistics report a page may send (diary only).
pub const MAX_STATS: usize = 4096;

/// Minimum opacity: below it the window is practically invisible and the
/// user could lose it.
pub const MIN_OPACITY: u8 = 20;

/// Turns a message from the page into a command.
pub fn parse(message: &str) -> Option<Command> {
    let (head, arg) = match message.split_once(':') {
        Some((head, arg)) => (head, Some(arg)),
        None => (message, None),
    };
    let command = match (head, arg) {
        ("drag", None) => Command::Drag,
        ("resize", Some(dir)) => Command::Resize(parse_edge(dir)?),
        ("ready", None) => Command::Ready,
        ("pin", None) => Command::TogglePin,
        ("video-only", None) => Command::ToggleVideoOnly,
        ("ghost", None) => Command::ToggleGhost,
        ("autostart", None) => Command::ToggleAutostart,
        ("hide", None) => Command::Hide,
        ("quit", None) => Command::Quit,
        ("open-clipboard", None) => Command::OpenClipboard,
        ("media-state", Some(json)) => Command::MediaReport(Box::new(MediaState::from_report(json)?)),
        // A service page's self-check (ui.js): which pieces were found.
        ("page-check", Some(json)) => Command::PageCheck(Box::new(crate::diag::PageCheck::from_report(json)?)),
        ("home", None) => Command::Home,
        ("service", Some(id)) if crate::services::is_service_id(id) => Command::OpenService(id.to_owned()),
        // The address is validated when the command runs (services::parse_user_url).
        ("open-url", Some(url)) if !url.is_empty() && url.len() <= 2048 => Command::OpenUrl(url.to_owned()),
        ("start-with-last", Some("1")) => Command::SetStartWithLast(true),
        ("start-with-last", Some("0")) => Command::SetStartWithLast(false),
        ("fullscreen", Some("1")) => Command::SetFullscreen(true),
        ("fullscreen", Some("0")) => Command::SetFullscreen(false),
        ("opacity", Some(value)) => Command::SetOpacity(parse_opacity(value)?),
        ("size", Some(size)) => Command::Size(parse_size(size)?),
        ("corner", Some(corner)) => Command::Corner(parse_corner(corner)?),
        ("ext-folder", None) => Command::ExtensionsFolder,
        ("ext-reload", None) => Command::ExtensionsReload,
        ("ext-toggle", Some(id)) if is_extension_id(id) => Command::ExtensionToggle(id.to_owned()),
        ("ext-options", Some(id)) if is_extension_id(id) => {
            Command::ExtensionOptions(id.to_owned())
        }
        ("together", None) => Command::TogetherOpen,
        // The page's size in CSS px (to place a guest's clicks exactly).
        ("viewport", Some(size)) => {
            let (w, h) = size.split_once('x')?;
            let (w, h): (u32, u32) = (w.parse().ok()?, h.parse().ok()?);
            ((1..=20_000).contains(&w) && (1..=20_000).contains(&h)).then_some(Command::Viewport(w, h))?
        }
        ("menu-open", Some("1")) => Command::MenuOpen(true),
        ("menu-open", Some("0")) => Command::MenuOpen(false),
        ("hardware-acceleration", None) => Command::ToggleHardwareAcceleration,
        // Exactly one of the codes the app has ("pl", not "pl-PL").
        ("lang", Some(code)) => Command::SetLanguage(parse_lang(code)?),
        ("settings", None) => Command::SettingsOpen(None),
        ("settings", Some(section)) => Command::SettingsOpen(Some(SettingsSection::parse(section)?)),
        ("welcome-done", None) => Command::WelcomeDone,
        _ => return None,
    };
    Some(command)
}

fn parse_opacity(value: &str) -> Option<u8> {
    let value: u8 = value.parse().ok()?;
    (MIN_OPACITY..=100).contains(&value).then_some(value)
}

fn parse_size(size: &str) -> Option<SizePreset> {
    Some(match size {
        "s" => SizePreset::Small,
        "m" => SizePreset::Medium,
        "l" => SizePreset::Large,
        _ => return None,
    })
}

fn parse_corner(corner: &str) -> Option<Corner> {
    Some(match corner {
        "nw" => Corner::TopLeft,
        "ne" => Corner::TopRight,
        "sw" => Corner::BottomLeft,
        "se" => Corner::BottomRight,
        _ => return None,
    })
}

fn parse_lang(code: &str) -> Option<crate::i18n::Lang> {
    crate::i18n::Lang::ALL.into_iter().find(|l| l.code() == code)
}

/// Messages from the Settings window (another closed list, in its own
/// webview pinned to https://ytmp.localhost/settings).
pub fn parse_settings(message: &str) -> Option<Command> {
    use crate::config::LockMode;
    let rest = message.strip_prefix("settings:")?;
    let (head, arg) = match rest.split_once(':') {
        Some((head, arg)) => (head, Some(arg)),
        None => (rest, None),
    };
    let on = || match arg {
        Some("1") => Some(true),
        Some("0") => Some(false),
        _ => None,
    };
    Some(match (head, arg) {
        ("ready", None) => Command::SettingsReady,
        ("close", None) => Command::SettingsClose,
        ("pin", _) => Command::Set(Setting::AlwaysOnTop(on()?)),
        ("ghost", _) => Command::Set(Setting::Ghost(on()?)),
        ("video-only", _) => Command::Set(Setting::VideoOnly(on()?)),
        ("fullscreen-in-window", _) => Command::Set(Setting::FullscreenInWindow(on()?)),
        ("taskbar-bar", _) => Command::Set(Setting::TaskbarBar(on()?)),
        ("autostart", _) => Command::Set(Setting::Autostart(on()?)),
        ("start-with-last", _) => Command::Set(Setting::StartWithLast(on()?)),
        ("hardware-acceleration", _) => Command::Set(Setting::HardwareAcceleration(on()?)),
        ("opacity", Some(value)) => Command::SetOpacity(parse_opacity(value)?),
        ("size", Some(size)) => Command::Size(parse_size(size)?),
        ("corner", Some(corner)) => Command::Corner(parse_corner(corner)?),
        ("lang", Some(code)) => Command::SetLanguage(parse_lang(code)?),
        ("lock", Some("off")) => Command::SettingsLock(LockMode::Off),
        ("lock", Some("hello")) => Command::SettingsLock(LockMode::Hello),
        ("lock", Some("pin")) => Command::SettingsLock(LockMode::Pin),
        ("lock-now", None) => Command::LockNow,
        ("sign-out", None) => Command::SignOutAll,
        ("security", None) => Command::SecurityStatus,
        ("ext-toggle", Some(id)) if is_extension_id(id) => Command::ExtensionToggle(id.to_owned()),
        ("ext-options", Some(id)) if is_extension_id(id) => Command::ExtensionOptions(id.to_owned()),
        ("ext-folder", None) => Command::ExtensionsFolder,
        ("ext-reload", None) => Command::ExtensionsReload,
        ("together", None) => Command::TogetherOpen,
        // Cleaned when it is used (together::protocol::clean_name).
        ("together-name", Some(name)) if name.chars().count() <= 64 => Command::TogetherName(name.to_owned()),
        ("together-mode", Some("share")) => Command::TogetherMode(crate::together::protocol::Mode::Share),
        ("together-mode", Some("sync")) => Command::TogetherMode(crate::together::protocol::Mode::Sync),
        ("welcome", None) => Command::ShowWelcome,
        ("help", None) => Command::ShowHelp,
        ("open-folder", None) => Command::OpenDataFolder,
        ("restart", None) => Command::Restart,
        _ => return None,
    })
}

/// Messages from the music bar (another closed list, in another window).
pub fn parse_mini(message: &str) -> Option<Command> {
    if let Some(action) = message.strip_prefix("media:") {
        return MediaAction::parse(action).map(Command::Media);
    }
    Some(match message {
        "player:show" => Command::Show,
        "mini:close" => Command::MiniHide,
        "mini:hover:1" => Command::MiniHover(true),
        "mini:hover:0" => Command::MiniHover(false),
        "mini:ready" => Command::MiniReady,
        "mini:drag:start" => Command::MiniDrag(DragPhase::Start),
        "mini:drag:move" => Command::MiniDrag(DragPhase::Move),
        "mini:drag:end" => Command::MiniDrag(DragPhase::End),
        _ => return None,
    })
}

/// Messages from the "My music" page (`music:...`). The webview only accepts
/// them from that page (see webview.rs).
pub fn parse_music(message: &str) -> Option<Command> {
    crate::music::Request::parse(message.strip_prefix("music:")?).map(Command::Music)
}

/// Messages from the lock screen (another closed list, another webview).
pub fn parse_lock(message: &str) -> Option<Command> {
    let pin = |p: &str| (!p.is_empty() && p.chars().count() <= crate::lock::PIN_MAX).then(|| p.to_owned());
    Some(match message.split_once(':') {
        Some(("lock", "ready")) => Command::LockReady,
        Some(("lock", "hello")) => Command::LockHello,
        Some(("lock", "cancel")) => Command::LockCancel,
        Some(("lock", "forgot")) => Command::LockForgot,
        Some(("lock", rest)) => match rest.split_once(':') {
            Some(("pin", p)) => Command::LockPin(pin(p)?),
            Some(("new-pin", p)) => Command::LockNewPin(pin(p)?),
            _ => return None,
        },
        _ => return None,
    })
}

/// Messages from the "Watch together" window (another closed list, in its
/// own webview).
pub fn parse_together(message: &str) -> Option<Command> {
    let rest = message.strip_prefix("together:")?;
    let name = |n: &str| (n.chars().count() <= 64).then(|| n.to_owned());
    let (head, arg) = match rest.split_once(':') {
        Some((head, arg)) => (head, Some(arg)),
        None => (rest, None),
    };
    Some(match (head, arg) {
        ("ready", None) => Command::TogetherReady,
        ("start", None) => Command::TogetherStart(String::new()),
        ("start", Some(n)) => Command::TogetherStart(name(n)?),
        // "join:<code>" or "join:<code>:<name>" (a name may contain ':').
        ("join", Some(arg)) => {
            let (code, n) = arg.split_once(':').unwrap_or((arg, ""));
            if code.is_empty() || code.chars().count() > 32 {
                return None;
            }
            Command::TogetherJoin(code.to_owned(), name(n)?)
        }
        ("leave", None) => Command::TogetherLeave,
        ("dismiss", None) => Command::TogetherDismiss,
        ("kick", Some(id)) if crate::together::protocol::is_member_id(id) && id != "host" => Command::TogetherKick(id.to_owned()),
        ("new-code", None) => Command::TogetherNewCode,
        ("copy", None) => Command::TogetherCopy,
        ("mode", Some("share")) => Command::TogetherMode(crate::together::protocol::Mode::Share),
        ("mode", Some("sync")) => Command::TogetherMode(crate::together::protocol::Mode::Sync),
        ("member-control", Some(arg)) => {
            let (id, on) = arg.split_once(':')?;
            let on = match on {
                "1" => true,
                "0" => false,
                _ => return None,
            };
            guest_id(id)?;
            Command::TogetherMemberControl(id.to_owned(), on)
        }
        ("dismiss-ask", Some(id)) => Command::TogetherDismissAsk(guest_id(id)?),
        ("watch", None) => Command::TogetherWatch,
        // "Ask to use it" (guest): the same as on the window's page.
        ("ask", None) => Command::StreamAsk,
        ("resync", None) => Command::TogetherResync,
        ("open-site", None) => Command::TogetherOpenSite,
        ("relay", Some(url)) if url.len() <= 512 => Command::TogetherRelay(url.to_owned()),
        ("sound", Some("pc")) => Command::TogetherSound(true),
        ("sound", Some("app")) => Command::TogetherSound(false),
        ("close", None) => Command::TogetherClose,
        _ => return None,
    })
}

/// A guest's member id (`g1`, `g2`…).
fn guest_id(id: &str) -> Option<String> {
    (crate::together::protocol::is_member_id(id) && id != "host").then(|| id.to_owned())
}

/// Messages from the page that sends the host's window (its own hidden
/// webview, another closed list).
pub fn parse_cast(message: &str) -> Option<Command> {
    use crate::together::protocol::MAX_SDP;
    let rest = message.strip_prefix("cast:")?;
    let (head, arg) = match rest.split_once(':') {
        Some((head, arg)) => (head, Some(arg)),
        None => (rest, None),
    };
    Some(match (head, arg) {
        ("ready", None) => Command::CastReady,
        ("caps", Some(json)) if json.len() <= 256 => {
            let caps: serde_json::Value = serde_json::from_str(json).ok()?;
            Command::CastCaps(caps["audio"].as_bool().unwrap_or(false))
        }
        ("offer", Some(arg)) => {
            let (id, sdp) = arg.split_once(':')?;
            (sdp.len() <= MAX_SDP).then_some(())?;
            Command::CastOffer(guest_id(id)?, sdp.to_owned())
        }
        ("state", Some(arg)) => {
            let (id, state) = arg.split_once(':')?;
            crate::together::session::StreamState::from_report(state)?;
            Command::CastState(guest_id(id)?, state.to_owned())
        }
        ("input", Some(arg)) => {
            let (id, json) = arg.split_once(':')?;
            (json.len() <= 512).then_some(())?;
            Command::CastInput(guest_id(id)?, json.to_owned())
        }
        ("stats", Some(json)) if json.len() <= MAX_STATS && json.starts_with('{') => Command::CastStats(json.to_owned()),
        _ => return None,
    })
}

/// Messages from the page that shows the host's window (the main webview;
/// only accepted from https://ytmp.localhost/stream, see webview.rs).
pub fn parse_stream(message: &str) -> Option<Command> {
    use crate::together::protocol::MAX_SDP;
    let rest = message.strip_prefix("stream:")?;
    Some(match rest.split_once(':') {
        None => match rest {
            "ready" => Command::StreamReady,
            "ask" => Command::StreamAsk,
            "leave" => Command::StreamLeave,
            _ => return None,
        },
        Some(("answer", sdp)) if sdp.len() <= MAX_SDP => Command::StreamAnswer(sdp.to_owned()),
        Some(("stats", json)) if json.len() <= MAX_STATS && json.starts_with('{') => Command::StreamStats(json.to_owned()),
        Some(("state", state @ ("connecting" | "watching" | "failed"))) => Command::StreamState(state.to_owned()),
        _ => return None,
    })
}

fn parse_edge(dir: &str) -> Option<Edge> {
    Some(match dir {
        "n" => Edge::North,
        "s" => Edge::South,
        "e" => Edge::East,
        "w" => Edge::West,
        "ne" => Edge::NorthEast,
        "nw" => Edge::NorthWest,
        "se" => Edge::SouthEast,
        "sw" => Edge::SouthWest,
        _ => return None,
    })
}

/// Chromium extension ids are 32 letters from `a` to `p`.
pub fn is_extension_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b))
}

/// Events that reach the main loop from other threads or callbacks.
#[derive(Debug, Clone, PartialEq)]
pub enum UserEvent {
    Command(Command),
    Extension(crate::extensions::ExtensionEvent),
    /// A message from the music service for its page (JSON).
    Music(String),
    /// From the watch together connection (which connection, what happened).
    Together(u64, crate::together::link::LinkEvent),
    /// A piece of the app's sound, for the shared window (loopback.rs).
    Sound(Vec<u8>),
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "cjpalhdlnbpafiamejdnhcphjbkeiagm";

    #[test]
    fn parses_window_messages() {
        assert_eq!(parse("drag"), Some(Command::Drag));
        assert_eq!(parse("resize:se"), Some(Command::Resize(Edge::SouthEast)));
        assert_eq!(parse("pin"), Some(Command::TogglePin));
        assert_eq!(parse("fullscreen:1"), Some(Command::SetFullscreen(true)));
        assert_eq!(parse("fullscreen:0"), Some(Command::SetFullscreen(false)));
        assert_eq!(parse("size:l"), Some(Command::Size(SizePreset::Large)));
        assert_eq!(parse("corner:sw"), Some(Command::Corner(Corner::BottomLeft)));
        assert_eq!(parse("hide"), Some(Command::Hide));
        assert_eq!(parse("quit"), Some(Command::Quit));
    }

    #[test]
    fn parses_service_messages() {
        assert_eq!(parse("home"), Some(Command::Home));
        assert_eq!(parse("service:netflix"), Some(Command::OpenService("netflix".into())));
        assert_eq!(parse("open-url:https://plex.tv/web"), Some(Command::OpenUrl("https://plex.tv/web".into())));
        assert_eq!(parse("start-with-last:1"), Some(Command::SetStartWithLast(true)));
        assert_eq!(parse("start-with-last:0"), Some(Command::SetStartWithLast(false)));
        for bad in ["home:1", "service:", "service:Netflix", "service:../x", "open-url:", "start-with-last:2"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
        assert_eq!(parse(&format!("open-url:{}", "a".repeat(3000))), None);
    }

    #[test]
    fn parses_media_reports() {
        match parse(r#"media-state:{"hasMedia":true,"title":"Hello","playing":true}"#) {
            Some(Command::MediaReport(state)) => assert!(state.playing && state.title == "Hello"),
            other => panic!("{other:?}"),
        }
        assert_eq!(parse("media-state:not json"), None);
        assert_eq!(parse("media-state:"), None);
    }

    #[test]
    fn parses_page_checks() {
        assert_eq!(
            parse(r#"page-check:{"site":"google","page":"sign-in-refused"}"#),
            Some(Command::PageCheck(Box::new(crate::diag::PageCheck::GoogleSignInRefused)))
        );
        assert_eq!(parse(r#"page-check:{"site":"google","page":"inbox"}"#), None);
        assert_eq!(parse("page-check"), None);
        // Only the services' pages speak it.
        assert_eq!(parse_settings(r#"page-check:{"site":"google","page":"sign-in-refused"}"#), None);
    }

    #[test]
    fn mini_protocol_is_separate_and_closed() {
        assert_eq!(parse_mini("media:next"), Some(Command::Media(MediaAction::Next)));
        assert_eq!(parse_mini("media:volume:30"), Some(Command::Media(MediaAction::Volume(30))));
        assert_eq!(parse_mini("player:show"), Some(Command::Show));
        assert_eq!(parse_mini("mini:hover:1"), Some(Command::MiniHover(true)));
        assert_eq!(parse_mini("mini:close"), Some(Command::MiniHide));
        assert_eq!(parse_mini("mini:drag:start"), Some(Command::MiniDrag(DragPhase::Start)));
        assert_eq!(parse_mini("mini:drag:move"), Some(Command::MiniDrag(DragPhase::Move)));
        assert_eq!(parse_mini("mini:drag:end"), Some(Command::MiniDrag(DragPhase::End)));
        assert_eq!(parse_mini("mini:drag:200"), None, "the page does not decide the position");
        // The music bar cannot ask for anything of the main page...
        for msg in ["quit", "drag", "pin", "service:netflix", "open-url:x.com", "media:delete", "mini:hover:2", "lang:ja"] {
            assert_eq!(parse_mini(msg), None, "{msg}");
        }
        // ...and the page cannot use the music bar's protocol.
        for msg in ["media:next", "player:show", "mini:close", "mini:drag:start"] {
            assert_eq!(parse(msg), None, "{msg}");
        }
    }

    #[test]
    fn lock_protocol_is_separate_and_closed() {
        assert_eq!(parse_lock("lock:ready"), Some(Command::LockReady));
        assert_eq!(parse_lock("lock:hello"), Some(Command::LockHello));
        assert_eq!(parse_lock("lock:forgot"), Some(Command::LockForgot));
        assert_eq!(parse_lock("lock:pin:482915"), Some(Command::LockPin("482915".into())));
        assert_eq!(parse_lock("lock:pin:a:b"), Some(Command::LockPin("a:b".into())), "a PIN may contain ':'");
        assert_eq!(parse_lock("lock:new-pin:long passphrase"), Some(Command::LockNewPin("long passphrase".into())));
        let long = format!("lock:pin:{}", "1".repeat(65));
        for bad in ["lock:pin:", "lock:off", "lock", "quit", "media:next", "mini:close", long.as_str()] {
            assert_eq!(parse_lock(bad), None, "{bad}");
        }
        // Neither the page nor the bar can use the lock protocol.
        for msg in ["lock:hello", "lock:pin:482915", "lock:cancel"] {
            assert_eq!(parse(msg), None, "{msg}");
            assert_eq!(parse_mini(msg), None, "{msg}");
        }
    }

    #[test]
    fn music_protocol_is_separate() {
        assert_eq!(parse_music("music:ready"), Some(Command::Music(crate::music::Request::Ready)));
        assert_eq!(parse_music("ready"), None);
        assert_eq!(parse_music("music:delete-everything"), None);
        // The normal page, the bar and the lock screen do not understand it.
        for msg in ["music:ready", "music:pick-folder", "music:buy:lifetime"] {
            assert_eq!(parse(msg), None, "{msg}");
            assert_eq!(parse_mini(msg), None, "{msg}");
            assert_eq!(parse_lock(msg), None, "{msg}");
        }
    }

    #[test]
    fn together_protocol_is_separate_and_closed() {
        assert_eq!(parse("together"), Some(Command::TogetherOpen), "the side bar opens the window");
        assert_eq!(parse_together("together:ready"), Some(Command::TogetherReady));
        assert_eq!(parse_together("together:start"), Some(Command::TogetherStart(String::new())));
        assert_eq!(parse_together("together:start:Ana"), Some(Command::TogetherStart("Ana".into())));
        assert_eq!(
            parse_together("together:join:k7q4 9xm2 pf3t:Ana: the best"),
            Some(Command::TogetherJoin("k7q4 9xm2 pf3t".into(), "Ana: the best".into()))
        );
        assert_eq!(parse_together("together:join:K7Q4-9XM2-PF3T"), Some(Command::TogetherJoin("K7Q4-9XM2-PF3T".into(), String::new())));
        assert_eq!(parse_together("together:kick:g2"), Some(Command::TogetherKick("g2".into())));
        assert_eq!(parse_together("together:mode:share"), Some(Command::TogetherMode(crate::together::protocol::Mode::Share)));
        assert_eq!(parse_together("together:member-control:g2:1"), Some(Command::TogetherMemberControl("g2".into(), true)));
        assert_eq!(parse_together("together:dismiss-ask:g3"), Some(Command::TogetherDismissAsk("g3".into())));
        assert_eq!(parse_together("together:watch"), Some(Command::TogetherWatch));
        assert_eq!(parse_together("together:ask"), Some(Command::StreamAsk));
        assert_eq!(parse_together("together:relay:wss://x.workers.dev"), Some(Command::TogetherRelay("wss://x.workers.dev".into())));
        assert_eq!(parse_together("together:sound:pc"), Some(Command::TogetherSound(true)));
        assert_eq!(parse_together("together:sound:app"), Some(Command::TogetherSound(false)));
        let long_name = format!("together:start:{}", "x".repeat(65));
        let long_code = format!("together:join:{}", "1".repeat(33));
        for bad in [
            "together:kick:host",
            "together:kick:../x",
            "together:join:",
            "together:control:1",
            "together:mode:screen",
            "together:member-control:host:1",
            "together:member-control:g1:2",
            "together:member-control:g1",
            "together:dismiss-ask:../x",
            "together:sound",
            "together:sound:device",
            "together:sound:PC",
            "together:quit",
            "together",
            "ready",
            long_name.as_str(),
            long_code.as_str(),
        ] {
            assert_eq!(parse_together(bad), None, "{bad}");
        }
        // Nobody else speaks it, and it speaks nobody else's.
        for msg in ["together:start", "together:kick:g1", "together:copy"] {
            assert_eq!(parse(msg), None, "{msg}");
            assert_eq!(parse_mini(msg), None, "{msg}");
            assert_eq!(parse_lock(msg), None, "{msg}");
        }
        for msg in ["quit", "media:next", "lock:hello", "music:ready", "together"] {
            assert_eq!(parse_together(msg), None, "{msg}");
        }
    }

    #[test]
    fn cast_and_stream_protocols_are_separate_and_closed() {
        let sdp = "v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\n";
        assert_eq!(parse_cast("cast:ready"), Some(Command::CastReady));
        assert_eq!(parse_cast(r#"cast:caps:{"audio":true,"generator":true}"#), Some(Command::CastCaps(true)));
        assert_eq!(parse_cast(&format!("cast:offer:g1:{sdp}")), Some(Command::CastOffer("g1".into(), sdp.into())));
        assert_eq!(parse_cast("cast:state:g2:connected"), Some(Command::CastState("g2".into(), "connected".into())));
        assert_eq!(
            parse_cast(r#"cast:input:g1:{"t":"m","e":"move","x":0.5,"y":0.5}"#),
            Some(Command::CastInput("g1".into(), r#"{"t":"m","e":"move","x":0.5,"y":0.5}"#.into()))
        );
        let long_input = format!("cast:input:g1:{}", "x".repeat(600));
        let long_offer = format!("cast:offer:g1:{}", "x".repeat(20_000));
        for bad in [
            "cast:offer:host:v=0",
            "cast:state:g1:weird",
            "cast:input:../x:{}",
            "cast:quit",
            "cast:sound:1",
            long_input.as_str(),
            long_offer.as_str(),
        ] {
            assert_eq!(parse_cast(bad), None, "{bad}");
        }
        assert_eq!(parse_stream("stream:ready"), Some(Command::StreamReady));
        assert_eq!(parse_stream(&format!("stream:answer:{sdp}")), Some(Command::StreamAnswer(sdp.into())));
        assert_eq!(parse_stream("stream:state:watching"), Some(Command::StreamState("watching".into())));
        assert_eq!(parse_stream("stream:ask"), Some(Command::StreamAsk));
        assert_eq!(parse_stream("stream:leave"), Some(Command::StreamLeave));
        // Statistics (only for the diary): a JSON object, not too big.
        let stats = r#"{"fps":29.5,"w":1280,"h":720}"#;
        assert_eq!(parse_stream(&format!("stream:stats:{stats}")), Some(Command::StreamStats(stats.into())));
        assert_eq!(parse_cast(&format!("cast:stats:{stats}")), Some(Command::CastStats(stats.into())));
        let huge = format!("{{\"x\":\"{}\"}}", "y".repeat(MAX_STATS));
        for bad in [format!("stream:stats:{huge}"), "stream:stats:[1]".into(), "stream:stats:".into()] {
            assert_eq!(parse_stream(&bad), None, "{bad:.40}");
        }
        for bad in [format!("cast:stats:{huge}"), "cast:stats:42".into(), "cast:stats".into()] {
            assert_eq!(parse_cast(&bad), None, "{bad:.40}");
        }
        assert_eq!(parse(&format!("stream:stats:{stats}")), None);
        for bad in ["stream:state:hacked", "stream:offer:v=0", "stream", "stream:ready:1"] {
            assert_eq!(parse_stream(bad), None, "{bad}");
        }
        // Nobody else speaks them.
        for msg in ["cast:ready", "stream:ready", "stream:ask", "cast:input:g1:{}"] {
            assert_eq!(parse(msg), None, "{msg}");
            assert_eq!(parse_mini(msg), None, "{msg}");
            assert_eq!(parse_lock(msg), None, "{msg}");
            assert_eq!(parse_together(msg), None, "{msg}");
        }
        assert_eq!(parse_cast("stream:ready"), None);
        assert_eq!(parse_stream("cast:ready"), None);
        assert_eq!(parse("menu-open:1"), Some(Command::MenuOpen(true)));
        assert_eq!(parse("hardware-acceleration"), Some(Command::ToggleHardwareAcceleration));
        assert_eq!(parse("menu-open:2"), None);
        assert_eq!(parse("viewport:1280x720"), Some(Command::Viewport(1280, 720)));
        for bad in ["viewport:0x720", "viewport:1280", "viewport:axb", "viewport:99999x10", "viewport:-1x5"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn parses_every_edge() {
        for dir in ["n", "s", "e", "w", "ne", "nw", "se", "sw"] {
            assert!(matches!(parse(&format!("resize:{dir}")), Some(Command::Resize(_))), "{dir}");
        }
    }

    #[test]
    fn opacity_is_bounded() {
        assert_eq!(parse("opacity:85"), Some(Command::SetOpacity(85)));
        assert_eq!(parse("opacity:100"), Some(Command::SetOpacity(100)));
        assert_eq!(parse("opacity:20"), Some(Command::SetOpacity(20)));
        for bad in ["opacity:19", "opacity:0", "opacity:101", "opacity:-5", "opacity:", "opacity:x"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn extension_ids_are_validated() {
        assert_eq!(parse(&format!("ext-toggle:{ID}")), Some(Command::ExtensionToggle(ID.into())));
        assert_eq!(parse(&format!("ext-options:{ID}")), Some(Command::ExtensionOptions(ID.into())));
        assert_eq!(parse("ext-toggle:../../etc"), None);
        assert_eq!(parse("ext-toggle:CJPALHDLNBPAFIAMEJDNHCPHJBKEIAGM"), None);
        assert_eq!(parse("ext-options:abc"), None);
    }

    #[test]
    fn page_cannot_send_system_only_commands() {
        // Show, play or reset the opacity only come from the tray or the
        // global shortcuts.
        for msg in ["show", "toggle-visible", "media:next", "reset-opacity", "adjust-opacity:10"] {
            assert_eq!(parse(msg), None, "{msg}");
        }
    }

    #[test]
    fn language_codes_are_exact() {
        assert_eq!(parse("lang:ja"), Some(Command::SetLanguage(crate::i18n::Lang::Ja)));
        assert_eq!(parse("lang:pl"), Some(Command::SetLanguage(crate::i18n::Lang::Pl)));
        for bad in ["lang:", "lang:pl-PL", "lang:PL", "lang:de", "lang", "lang:ja:x"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn opens_settings_from_the_player_and_the_start_screen() {
        assert_eq!(parse("settings"), Some(Command::SettingsOpen(None)));
        assert_eq!(parse("settings:extensions"), Some(Command::SettingsOpen(Some(SettingsSection::Extensions))));
        assert_eq!(parse("welcome-done"), Some(Command::WelcomeDone));
        for section in SettingsSection::ALL {
            assert_eq!(SettingsSection::parse(section.id()), Some(section));
            assert_eq!(parse(&format!("settings:{}", section.id())), Some(Command::SettingsOpen(Some(section))));
        }
        // The player cannot change settings directly through this name.
        for bad in ["settings:", "settings:Extensions", "settings:pin:1", "settings:lock:off", "welcome-done:1", "welcome"] {
            assert_eq!(parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn parses_the_settings_window() {
        use crate::config::LockMode;
        let cases = [
            ("settings:ready", Command::SettingsReady),
            ("settings:close", Command::SettingsClose),
            ("settings:pin:1", Command::Set(Setting::AlwaysOnTop(true))),
            ("settings:pin:0", Command::Set(Setting::AlwaysOnTop(false))),
            ("settings:ghost:1", Command::Set(Setting::Ghost(true))),
            ("settings:video-only:0", Command::Set(Setting::VideoOnly(false))),
            ("settings:fullscreen-in-window:1", Command::Set(Setting::FullscreenInWindow(true))),
            ("settings:taskbar-bar:1", Command::Set(Setting::TaskbarBar(true))),
            ("settings:autostart:1", Command::Set(Setting::Autostart(true))),
            ("settings:start-with-last:0", Command::Set(Setting::StartWithLast(false))),
            ("settings:hardware-acceleration:0", Command::Set(Setting::HardwareAcceleration(false))),
            ("settings:opacity:55", Command::SetOpacity(55)),
            ("settings:size:m", Command::Size(SizePreset::Medium)),
            ("settings:corner:se", Command::Corner(Corner::BottomRight)),
            ("settings:lang:ja", Command::SetLanguage(crate::i18n::Lang::Ja)),
            ("settings:lock:off", Command::SettingsLock(LockMode::Off)),
            ("settings:lock:hello", Command::SettingsLock(LockMode::Hello)),
            ("settings:lock:pin", Command::SettingsLock(LockMode::Pin)),
            ("settings:lock-now", Command::LockNow),
            ("settings:sign-out", Command::SignOutAll),
            ("settings:security", Command::SecurityStatus),
            (&format!("settings:ext-toggle:{ID}"), Command::ExtensionToggle(ID.into())),
            (&format!("settings:ext-options:{ID}"), Command::ExtensionOptions(ID.into())),
            ("settings:ext-folder", Command::ExtensionsFolder),
            ("settings:ext-reload", Command::ExtensionsReload),
            ("settings:together", Command::TogetherOpen),
            ("settings:together-name:Ana: la de 2ºB", Command::TogetherName("Ana: la de 2ºB".into())),
            ("settings:together-name:", Command::TogetherName(String::new())),
            ("settings:together-mode:sync", Command::TogetherMode(crate::together::protocol::Mode::Sync)),
            ("settings:together-mode:share", Command::TogetherMode(crate::together::protocol::Mode::Share)),
            ("settings:welcome", Command::ShowWelcome),
            ("settings:help", Command::ShowHelp),
            ("settings:open-folder", Command::OpenDataFolder),
            ("settings:restart", Command::Restart),
        ];
        for (message, command) in cases {
            assert_eq!(parse_settings(message), Some(command), "{message}");
        }
        let long_name = format!("settings:together-name:{}", "n".repeat(65));
        for bad in [
            "pin:1", "settings", "settings:", "settings:pin", "settings:pin:2", "settings:pin:true", "settings:ghost:",
            "settings:opacity:19", "settings:opacity:101", "settings:opacity:-5", "settings:opacity:5x",
            "settings:size:xl", "settings:corner:n", "settings:lang:de", "settings:lang:PL",
            "settings:lock:", "settings:lock:Off", "settings:lock:face", "settings:lock-now:1",
            "settings:ext-toggle:abc", "settings:ext-options:", "settings:together-mode:watch",
            long_name.as_str(), "settings:quit", "settings:drag", "settings:service:netflix", "settings:open-url:x",
            "settings:LockNow", "together:start", "Settings:ready", "settings:restart:1",
        ] {
            assert_eq!(parse_settings(bad), None, "{bad}");
        }
        // The other windows' messages are not settings, and the other way round.
        assert_eq!(parse("settings:pin:1"), None);
        assert_eq!(parse_together("settings:ready"), None);
        assert_eq!(parse_settings("together:ready"), None);
    }

    #[test]
    fn rejects_unknown_or_malformed() {
        for msg in ["", "Drag", "drag:1", "resize:", "resize:x", "pin:1", "size:xl", "corner:n", "close", "drag "] {
            assert_eq!(parse(msg), None, "{msg:?}");
        }
    }
}

#[cfg(test)]
#[path = "tests/ipc_cases.rs"]
mod cases;
