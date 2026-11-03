//! Watch together: the host shares a code; up to five friends join with it
//! and either see the host's window (picture and sound, sent straight to
//! each of them with WebRTC) or have their own players follow the host's
//! (see README, "Watch together").
//!
//! - code.rs: the code (HMAC of a secret and the 30-minute window).
//! - crypto.rs: the handshake and the room's encryption.
//! - protocol.rs: what travels, and what may be followed.
//! - sync.rs: keeping a guest's player with the host's.
//! - session.rs: the host's and the guests' logic.
//! - link.rs: the WebSocket to the relay (its own thread).
//! - ../together_panel.rs: the "Watch together" window.
//!
//! [`Together`] ties them to the app: the app hands it commands from the
//! panel, the player's reports and the timer, and applies the
//! [`Effect`]s it returns.

pub mod code;
pub mod crypto;
pub mod link;
pub mod protocol;
pub mod session;
pub mod sync;

use std::{
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use url::Url;

use self::{
    code::{Code, CodeError},
    link::{Failure, Link, LinkEvent},
    protocol::{parse_relay, Mode, Page},
    session::{EndReason, Guest, Host, MemberView, Now, Out, StreamState},
    sync::Status,
};
use crate::media::{MediaAction, MediaState};

/// How long the connection keeps trying after it drops (the relay keeps a
/// guest's seat for 60 s and a host's room for 120 s).
const HOST_RETRY: Duration = Duration::from_secs(110);
const GUEST_RETRY: Duration = Duration::from_secs(55);
/// How often the session looks at the clock while it is on.
const TICK: Duration = Duration::from_millis(500);

/// This build's relay, if it was built with one.
pub fn built_in_relay() -> Option<Url> {
    option_env!("YTMP_TOGETHER_RELAY").and_then(link::parse_relay)
}

/// What the app must do.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Media(MediaAction),
    Navigate(Url),
    Toast(String),
    /// The panel's new state (JSON).
    Panel(String),
    /// Host: capture the window and send it (or stop).
    Share(bool),
    /// Host: a message for the page that sends the window (JSON).
    Cast(String),
    /// Guest: show the host's window instead of the own page (or go back).
    Watch(bool),
    /// Guest: a message for the page that shows the host's window (JSON).
    Stream(String),
}

enum Session {
    Host(Box<Host>),
    Guest(Box<Guest>),
}

/// Events from the connection's thread, tagged with the connection they
/// belong to (an old one's are ignored).
pub type Notify = Arc<dyn Fn(u64, LinkEvent) + Send + Sync>;

pub struct Together {
    notify: Notify,
    session: Option<Session>,
    link: Option<Link>,
    link_id: u64,
    origin: Instant,
    connecting: bool,
    reconnecting: bool,
    ended: Option<EndReason>,
    /// The code typed wrong, to say so in the panel.
    code_error: Option<CodeError>,
    relay: Option<Url>,
    relay_fixed: bool,
    relay_error: bool,
    name: String,
    locked: bool,
    last_view: String,
    next_tick: Option<Instant>,
    /// What the app was last told (it follows the session).
    sharing: bool,
    watching: bool,
    /// The host's page protects its video (it would be black for the others).
    protected_page: bool,
    /// How the window's sound is going out (from the sending side).
    sound: Sound,
    /// "Share my window" or "Sync players" for the next session.
    preferred: Mode,
    /// Guest: how its window page's connection is going.
    stream: Option<&'static str>,
}

/// Host: how the window's sound is going out (from the sending side).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sound {
    /// It goes out (`None` until that is known).
    pub on: Option<bool>,
    /// All of the PC's sound instead of only the app's.
    pub whole_pc: bool,
    /// Why it does not.
    pub problem: Option<SoundProblem>,
    /// The app's own sound could not be captured in this sharing session.
    pub app_unavailable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SoundProblem {
    /// The sending page can't make a sound track.
    Page,
    /// Windows did not give the sound.
    Unavailable,
    /// Nothing is heard while the page plays.
    Silent,
}

impl SoundProblem {
    pub fn key(self) -> &'static str {
        match self {
            Self::Page => "page",
            Self::Unavailable => "unavailable",
            Self::Silent => "silent",
        }
    }
}

impl Sound {
    /// From what the sending page can do (`page`), whether the capture
    /// started (`capture`) and whether it hears nothing while the page
    /// plays (`deaf`); `None` while not known yet.
    pub fn new(page: Option<bool>, capture: Option<bool>, deaf: bool, whole_pc: bool, app_unavailable: bool) -> Self {
        let problem = match (page, capture) {
            (Some(false), _) => Some(SoundProblem::Page),
            (_, Some(false)) => Some(SoundProblem::Unavailable),
            (_, Some(true)) if deaf => Some(SoundProblem::Silent),
            _ => None,
        };
        let on = match (page, capture) {
            _ if problem.is_some() => Some(false),
            (Some(true), Some(true)) => Some(true),
            _ => None,
        };
        Self { on, whole_pc, problem, app_unavailable }
    }
}

/// What the panel shows.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct View<'a> {
    /// `idle`, `connecting`, `host`, `guest` or `ended`.
    phase: &'static str,
    ended: Option<&'static str>,
    code_error: Option<&'static str>,
    name: &'a str,
    relay: Option<String>,
    relay_fixed: bool,
    relay_error: bool,
    reconnecting: bool,
    locked: bool,
    /// `sync` or `share`.
    mode: Mode,
    max_people: usize,
    // Host
    code: Option<String>,
    /// When the code changes (Unix seconds): the panel counts down itself.
    code_ends: u64,
    /// Guest: it may pause and seek or use the window.
    control: bool,
    members: Vec<MemberView>,
    sharing: bool,
    protected_page: bool,
    /// The window's sound goes out (`None` until that is known).
    sound: Option<bool>,
    /// It is all of the PC's sound (the host chose it for this session).
    sound_whole_pc: bool,
    /// Why it does not: `page`, `unavailable` or `silent`.
    sound_problem: Option<&'static str>,
    /// The app's own sound could not be captured in this session (no point
    /// offering to go back to it).
    sound_app_unavailable: bool,
    /// Guest: `connecting`, `watching` or `failed`, from its window page.
    stream: Option<&'static str>,
    // Guest
    host_name: String,
    host_away: bool,
    status: Option<Status>,
    title: String,
    site: String,
    open_url: Option<String>,
    rtt: Option<u64>,
    /// Sharing: the host's window is on screen / paused by the host.
    watching: bool,
    paused: bool,
}

impl Together {
    pub fn new(notify: Notify, relay: Option<Url>, relay_fixed: bool, name: &str, preferred: Mode) -> Self {
        Self {
            notify,
            session: None,
            link: None,
            link_id: 0,
            origin: Instant::now(),
            connecting: false,
            reconnecting: false,
            ended: None,
            code_error: None,
            relay,
            relay_fixed,
            relay_error: false,
            name: protocol::clean_name(name),
            locked: false,
            last_view: String::new(),
            next_tick: None,
            sharing: false,
            watching: false,
            protected_page: false,
            sound: Sound::default(),
            preferred,
            stream: None,
        }
    }

    fn now(&self) -> Now {
        Now {
            ms: self.origin.elapsed().as_millis() as u64,
            unix: SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs()),
        }
    }

    /// A session is on (or connecting): the player reports every second.
    pub fn is_active(&self) -> bool {
        self.session.is_some()
    }

    pub fn is_host(&self) -> bool {
        matches!(self.session, Some(Session::Host(_)))
    }

    /// The relay typed in the panel (when the build has none). `true` if it
    /// was taken (to save it).
    pub fn set_relay_text(&mut self, text: &str) -> (bool, Vec<Effect>) {
        let mut taken = false;
        if !self.relay_fixed && self.session.is_none() {
            match link::parse_relay(text) {
                Some(url) => {
                    self.relay = Some(url);
                    self.relay_error = false;
                    taken = true;
                }
                None => self.relay_error = true,
            }
        }
        (taken, self.finish_effects(Vec::new()))
    }

    pub fn relay(&self) -> Option<&Url> {
        self.relay.as_ref()
    }

    fn connect(&mut self, dial: link::Dial, retry_for: Duration) -> bool {
        let Some(relay) = self.relay.clone() else { return false };
        self.link_id += 1;
        let id = self.link_id;
        let notify = self.notify.clone();
        self.link = Some(Link::start(relay, dial, retry_for, move |event| notify(id, event)));
        self.connecting = true;
        self.reconnecting = false;
        self.ended = None;
        self.code_error = None;
        self.next_tick = Some(Instant::now() + TICK);
        true
    }

    /// "Start a session" (sharing the window or syncing, as last chosen).
    pub fn start(&mut self, name: &str) -> Vec<Effect> {
        if self.session.is_some() || self.relay.is_none() {
            return self.finish_effects(Vec::new());
        }
        self.name = protocol::clean_name(name);
        let host = Host::new(&self.name, self.preferred, self.now());
        if self.connect(host.dial(), HOST_RETRY) {
            let mut host = Box::new(host);
            let _ = host.set_locked(self.locked);
            self.session = Some(Session::Host(host));
        }
        self.finish_effects(Vec::new())
    }

    /// "Join" with what the user typed.
    pub fn join(&mut self, typed: &str, name: &str) -> Vec<Effect> {
        if self.session.is_some() || self.relay.is_none() {
            return self.finish_effects(Vec::new());
        }
        self.name = protocol::clean_name(name);
        match Code::parse(typed) {
            Ok(code) => {
                let mut guest = Box::new(Guest::new(code, &self.name));
                let _ = guest.set_locked(self.locked);
                if self.connect(guest.dial(), GUEST_RETRY) {
                    self.session = Some(Session::Guest(guest));
                }
            }
            Err(err) => {
                self.code_error = Some(err);
                self.ended = None;
            }
        }
        self.finish_effects(Vec::new())
    }

    /// Says goodbye (the host ends the room, a guest leaves it) and closes,
    /// waiting at most `wait` for that to go out.
    fn goodbye(&mut self, wait: Duration) -> Vec<Effect> {
        let outs = match self.session.as_mut() {
            Some(Session::Host(host)) => host.end(),
            Some(Session::Guest(guest)) => guest.leave(),
            None => Vec::new(),
        };
        let effects = self.apply(outs.into_iter().filter(|o| *o != Out::Close).collect());
        self.stop(wait);
        effects
    }

    /// "Leave" (guest) or "End session" (host).
    pub fn leave(&mut self) -> Vec<Effect> {
        let effects = self.goodbye(Duration::ZERO);
        self.ended = None;
        self.finish_effects(effects)
    }

    /// On quitting: the goodbye gets a moment to go out.
    pub fn shutdown(&mut self) {
        if self.session.is_some() {
            self.goodbye(Duration::from_millis(800));
        }
    }

    fn stop(&mut self, wait: Duration) {
        self.session = None;
        self.connecting = false;
        self.reconnecting = false;
        self.next_tick = None;
        self.link_id += 1;
        if let Some(link) = self.link.take() {
            link.close(wait);
        }
    }

    /// "OK" on the "session over" screen.
    pub fn dismiss(&mut self) -> Vec<Effect> {
        self.ended = None;
        self.code_error = None;
        self.finish_effects(Vec::new())
    }

    pub fn kick(&mut self, member: &str) -> Vec<Effect> {
        let now = self.now();
        let outs = match self.session.as_mut() {
            Some(Session::Host(host)) => host.kick(member, now),
            _ => Vec::new(),
        };
        let effects = self.apply(outs);
        self.finish_effects(effects)
    }

    pub fn new_code(&mut self) -> Vec<Effect> {
        let now = self.now();
        let outs = match self.session.as_mut() {
            Some(Session::Host(host)) => host.new_code(now),
            _ => Vec::new(),
        };
        let effects = self.apply(outs);
        self.finish_effects(effects)
    }

    /// The name typed in Settings: the one the next session uses.
    pub fn set_name(&mut self, name: &str) -> Vec<Effect> {
        self.name = protocol::clean_name(name);
        self.finish_effects(Vec::new())
    }

    /// "Share my window" or "Sync players": for this session (host) and the
    /// next ones.
    pub fn set_mode(&mut self, mode: Mode) -> Vec<Effect> {
        self.preferred = mode;
        let now = self.now();
        let outs = match self.session.as_mut() {
            Some(Session::Host(host)) => host.set_mode(mode, now),
            _ => Vec::new(),
        };
        let effects = self.apply(outs);
        self.finish_effects(effects)
    }

    /// Host: lets a guest use the window (or pause and seek), or not.
    pub fn set_member_control(&mut self, member: &str, on: bool) -> Vec<Effect> {
        let outs = match self.session.as_mut() {
            Some(Session::Host(host)) => host.set_control(member, on),
            _ => Vec::new(),
        };
        let effects = self.apply(outs);
        self.finish_effects(effects)
    }

    /// Host: "Not now" on a guest's request.
    pub fn dismiss_ask(&mut self, member: &str) -> Vec<Effect> {
        if let Some(Session::Host(host)) = self.session.as_mut() {
            host.dismiss_ask(member);
        }
        self.finish_effects(Vec::new())
    }

    /// Host: may this guest use the window right now?
    pub fn may_control(&self, member: &str) -> bool {
        matches!(&self.session, Some(Session::Host(host)) if host.may_control(member))
    }

    pub fn member_name(&self, member: &str) -> Option<String> {
        match &self.session {
            Some(Session::Host(host)) => host.member_name(member),
            _ => None,
        }
    }

    /// Host: the sending page's offer for a guest.
    pub fn cast_offer(&mut self, member: &str, sdp: &str) -> Vec<Effect> {
        let outs = match self.session.as_mut() {
            Some(Session::Host(host)) => host.cast_offer(member, sdp),
            _ => Vec::new(),
        };
        let effects = self.apply(outs);
        self.finish_effects(effects)
    }

    /// Host: how a guest's connection is going (its page says).
    pub fn cast_state(&mut self, member: &str, state: StreamState) -> Vec<Effect> {
        if let Some(Session::Host(host)) = self.session.as_mut() {
            host.cast_state(member, state);
        }
        self.finish_effects(Vec::new())
    }

    /// Host: the sending page (re)loaded.
    pub fn cast_reset(&mut self) -> Vec<Effect> {
        let outs = match self.session.as_mut() {
            Some(Session::Host(host)) => host.cast_reset(),
            _ => Vec::new(),
        };
        let effects = self.apply(outs);
        self.finish_effects(effects)
    }

    /// Host: how the window's sound is going out.
    pub fn set_sound(&mut self, sound: Sound) -> Vec<Effect> {
        self.sound = sound;
        self.finish_effects(Vec::new())
    }

    /// Guest: its window page is ready (or wants a new connection).
    pub fn stream_ready(&mut self) -> Vec<Effect> {
        let outs = match self.session.as_mut() {
            Some(Session::Guest(guest)) => guest.stream_ready(),
            _ => Vec::new(),
        };
        let effects = self.apply(outs);
        self.finish_effects(effects)
    }

    pub fn stream_answer(&mut self, sdp: &str) -> Vec<Effect> {
        let outs = match self.session.as_mut() {
            Some(Session::Guest(guest)) => guest.stream_answer(sdp),
            _ => Vec::new(),
        };
        let effects = self.apply(outs);
        self.finish_effects(effects)
    }

    /// Guest: what its window page says about the connection.
    pub fn stream_state(&mut self, state: &str) -> Vec<Effect> {
        self.stream = match state {
            "connecting" => Some("connecting"),
            "watching" => Some("watching"),
            "failed" => Some("failed"),
            _ => None,
        };
        self.finish_effects(Vec::new())
    }

    /// Guest: the app is on another page now.
    pub fn stream_left(&mut self) -> Vec<Effect> {
        self.stream = None;
        let outs = match self.session.as_mut() {
            Some(Session::Guest(guest)) => guest.stream_left(),
            _ => Vec::new(),
        };
        let effects = self.apply(outs);
        self.finish_effects(effects)
    }

    /// Guest: "Ask to use the window".
    pub fn ask(&mut self) -> Vec<Effect> {
        let now = self.now();
        let outs = match self.session.as_mut() {
            Some(Session::Guest(guest)) => guest.ask(now),
            _ => Vec::new(),
        };
        let effects = self.apply(outs);
        self.finish_effects(effects)
    }

    /// Guest: the host's window should be on screen.
    pub fn wants_window(&self) -> bool {
        matches!(&self.session, Some(Session::Guest(guest)) if guest.wants_window())
    }

    pub fn resync(&mut self) -> Vec<Effect> {
        let now = self.now();
        let outs = match self.session.as_mut() {
            Some(Session::Guest(guest)) => guest.resync(now),
            _ => Vec::new(),
        };
        let effects = self.apply(outs);
        self.finish_effects(effects)
    }

    /// The code as shown, for "Copy".
    pub fn code_text(&self) -> Option<String> {
        match &self.session {
            Some(Session::Host(host)) => Some(host.code().display()),
            _ => None,
        }
    }

    /// The other site the host is on (a guest opens it by hand).
    pub fn site_to_open(&self) -> Option<Url> {
        match &self.session {
            Some(Session::Guest(guest)) => match &guest.follower.host()?.page {
                Page::Site { url } => Url::parse(url).ok().filter(|u| u.scheme() == "https"),
                _ => None,
            },
            _ => None,
        }
    }

    pub fn set_locked(&mut self, locked: bool) -> Vec<Effect> {
        self.locked = locked;
        let outs = match self.session.as_mut() {
            Some(Session::Host(host)) => host.set_locked(locked),
            Some(Session::Guest(guest)) => guest.set_locked(locked),
            None => Vec::new(),
        };
        let effects = self.apply(outs);
        self.finish_effects(effects)
    }

    /// Something from the connection.
    pub fn on_link(&mut self, id: u64, event: LinkEvent) -> Vec<Effect> {
        if id != self.link_id || self.session.is_none() {
            return Vec::new();
        }
        let now = self.now();
        let outs = match event {
            LinkEvent::Up => {
                self.connecting = false;
                self.reconnecting = false;
                Vec::new()
            }
            LinkEvent::Frame(text) => match parse_relay(&text) {
                Some(msg) => match self.session.as_mut() {
                    Some(Session::Host(host)) => host.on_relay(msg, now),
                    Some(Session::Guest(guest)) => guest.on_relay(msg, now),
                    None => Vec::new(),
                },
                None => Vec::new(),
            },
            LinkEvent::Down => {
                self.reconnecting = true;
                if let Some(Session::Host(host)) = self.session.as_mut() {
                    host.on_link_down();
                }
                Vec::new()
            }
            LinkEvent::Failed(failure) => vec![Out::Over(self.reason_for(&failure))],
            LinkEvent::Closed => Vec::new(),
        };
        let effects = self.apply(outs);
        self.finish_effects(effects)
    }

    fn reason_for(&self, failure: &Failure) -> EndReason {
        let guest_joining = matches!(&self.session, Some(Session::Guest(g)) if !g.admitted());
        match failure {
            Failure::ClosedBy(4000) => EndReason::Ended,
            Failure::ClosedBy(4001) => EndReason::Removed,
            Failure::ClosedBy(4002) => EndReason::Replaced,
            Failure::Refused(404) if guest_joining => EndReason::BadCode,
            Failure::Refused(409) => EndReason::Full,
            Failure::Refused(503) if guest_joining => EndReason::HostAway,
            Failure::Refused(404) => EndReason::HostGone,
            Failure::Unreachable(_) => EndReason::Unreachable,
            Failure::Refused(_) | Failure::ClosedBy(_) | Failure::Lost => {
                if self.connecting && !self.reconnecting {
                    EndReason::Unreachable
                } else {
                    EndReason::Lost
                }
            }
        }
    }

    /// What the player reports, with the page it is on.
    pub fn on_media(&mut self, media: &MediaState, url: &str) -> Vec<Effect> {
        let now = self.now();
        self.protected_page = Url::parse(url).ok().and_then(|u| crate::services::for_url(&u)).is_some_and(|s| s.drm);
        let outs = match self.session.as_mut() {
            Some(Session::Host(host)) => host.on_media(media, url, now),
            Some(Session::Guest(guest)) => guest.on_media(media, url, now),
            None => Vec::new(),
        };
        let effects = self.apply(outs);
        self.finish_effects(effects)
    }

    pub fn deadline(&self) -> Option<Instant> {
        let session = match &self.session {
            Some(Session::Host(host)) => host.deadline(),
            Some(Session::Guest(guest)) => guest.deadline(),
            None => None,
        }
        .map(|ms| self.origin + Duration::from_millis(ms));
        match (self.next_tick, session) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    pub fn on_timer(&mut self) -> Vec<Effect> {
        if self.session.is_none() {
            self.next_tick = None;
            return Vec::new();
        }
        let now = self.now();
        let outs = match self.session.as_mut() {
            Some(Session::Host(host)) => host.tick(now),
            Some(Session::Guest(guest)) => guest.tick(now),
            None => Vec::new(),
        };
        self.next_tick = Some(Instant::now() + TICK);
        let effects = self.apply(outs);
        self.finish_effects(effects)
    }

    fn apply(&mut self, outs: Vec<Out>) -> Vec<Effect> {
        let mut effects = Vec::new();
        for out in outs {
            match out {
                Out::Relay(text) => {
                    if let Some(link) = &self.link {
                        link.send(text);
                    }
                }
                Out::Redial(dial) => {
                    if let Some(link) = &self.link {
                        link.redial(dial);
                    }
                }
                Out::Close => {
                    if let Some(link) = self.link.take() {
                        link.close(Duration::ZERO);
                    }
                }
                Out::Media(action) => effects.push(Effect::Media(action)),
                Out::Navigate(url) => effects.push(Effect::Navigate(url)),
                Out::Toast(text) => effects.push(Effect::Toast(text)),
                Out::Cast(json) => effects.push(Effect::Cast(json)),
                Out::Stream(json) => effects.push(Effect::Stream(json)),
                Out::Over(reason) => {
                    self.stop(Duration::ZERO);
                    self.ended = Some(reason);
                    effects.push(Effect::Toast(crate::i18n::text(reason.key())));
                }
            }
        }
        effects
    }

    /// The panel's state, if it changed; and whether the app captures the
    /// window (host) or shows the host's (guest), if that changed.
    fn finish_effects(&mut self, mut effects: Vec<Effect>) -> Vec<Effect> {
        let sharing = matches!(&self.session, Some(Session::Host(host)) if host.capturing());
        if sharing != self.sharing {
            self.sharing = sharing;
            if sharing {
                // First: the sending page must exist before its messages.
                effects.insert(0, Effect::Share(true));
            } else {
                self.sound = Sound::default();
                effects.push(Effect::Share(false));
            }
        }
        let watching = self.wants_window();
        if watching != self.watching {
            self.watching = watching;
            if !watching {
                self.stream = None;
            }
            effects.push(Effect::Watch(watching));
        }
        let view = self.view_json();
        if view != self.last_view {
            self.last_view = view.clone();
            effects.push(Effect::Panel(view));
        }
        effects
    }

    /// The panel's state now (when it opens or asks).
    pub fn view_json(&self) -> String {
        let now = self.now();
        let mut view = View {
            phase: "idle",
            ended: self.ended.map(EndReason::key),
            code_error: self.code_error.map(|e| match e {
                CodeError::Length => "together.code_length",
                CodeError::Character => "together.code_character",
                CodeError::Check => "together.code_check",
            }),
            name: &self.name,
            relay: self.relay.as_ref().map(|u| u.as_str().trim_end_matches('/').to_owned()),
            relay_fixed: self.relay_fixed,
            relay_error: self.relay_error,
            reconnecting: self.reconnecting,
            locked: self.locked,
            mode: self.preferred,
            max_people: protocol::MAX_PEOPLE,
            code: None,
            code_ends: 0,
            control: false,
            members: Vec::new(),
            sharing: false,
            protected_page: false,
            sound: None,
            sound_whole_pc: false,
            sound_problem: None,
            sound_app_unavailable: false,
            stream: None,
            host_name: String::new(),
            host_away: false,
            status: None,
            title: String::new(),
            site: String::new(),
            open_url: None,
            rtt: None,
            watching: false,
            paused: false,
        };
        match &self.session {
            Some(Session::Host(host)) => {
                view.phase = if host.connected() || self.reconnecting { "host" } else { "connecting" };
                view.code = Some(host.code().display());
                view.code_ends = now.unix + host.code_left(now);
                view.mode = host.mode();
                view.members = host.members(now);
                view.sharing = host.capturing();
                view.protected_page = host.mode() == Mode::Share && self.protected_page;
                if host.capturing() {
                    view.sound = self.sound.on;
                    view.sound_whole_pc = self.sound.whole_pc;
                    view.sound_problem = self.sound.problem.map(SoundProblem::key);
                    view.sound_app_unavailable = self.sound.app_unavailable;
                }
            }
            Some(Session::Guest(guest)) => {
                view.phase = if guest.admitted() { "guest" } else { "connecting" };
                view.mode = guest.mode();
                view.watching = guest.watching();
                view.paused = guest.paused();
                view.stream = if guest.watching() { self.stream.or(Some("connecting")) } else { None };
                view.members = guest.members();
                view.host_name = guest.host_name();
                view.host_away = guest.host_away();
                view.status = guest.admitted().then(|| guest.follower.status());
                view.rtt = guest.follower.rtt();
                if let Some(state) = guest.follower.host() {
                    view.title = state.title.clone();
                    view.site = state.site.clone();
                    if let Page::Site { url } = &state.page {
                        view.open_url = Some(url.clone());
                        if view.site.is_empty() {
                            view.site = Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default();
                        }
                    }
                }
                view.control = guest.control();
            }
            None => {
                if self.ended.is_some() {
                    view.phase = "ended";
                }
            }
        }
        serde_json::to_string(&view).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    //! Against a real relay: `wrangler dev` in server/together-relay, then
    //! `YTMP_TEST_RELAY=ws://127.0.0.1:8799 cargo test against_a_real_relay`.
    //! Without the variable the test does nothing.

    use super::*;
    use serde_json::Value;
    use std::sync::mpsc::{self, Receiver};

    const VIDEO: &str = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";

    struct App {
        together: Together,
        events: Receiver<(u64, LinkEvent)>,
        effects: Vec<Effect>,
    }

    impl App {
        fn new(relay: &Url) -> Self {
            let (tx, events) = mpsc::channel();
            let notify: Notify = Arc::new(move |id, event| {
                let _ = tx.send((id, event));
            });
            Self { together: Together::new(notify, Some(relay.clone()), false, "", Mode::Sync), events, effects: Vec::new() }
        }

        fn view(&self) -> Value {
            serde_json::from_str(&self.together.view_json()).unwrap()
        }

        /// What the app's loop does: connection events and the timer.
        fn pump(&mut self) {
            while let Ok((id, event)) = self.events.try_recv() {
                let effects = self.together.on_link(id, event);
                self.effects.extend(effects);
            }
            if self.together.deadline().is_some_and(|at| Instant::now() >= at) {
                let effects = self.together.on_timer();
                self.effects.extend(effects);
            }
        }

        fn take(&mut self) -> Vec<Effect> {
            std::mem::take(&mut self.effects)
        }
    }

    /// Runs everyone's loop until `done` (or fails after a few seconds).
    fn until(apps: &mut [&mut App], what: &str, mut done: impl FnMut(&mut [&mut App]) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(8);
        loop {
            for app in apps.iter_mut() {
                app.pump();
            }
            if done(apps) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for: {what}");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn playing(position: f64) -> MediaState {
        MediaState { has_media: true, playing: true, position, duration: 600.0, rate: 1.0, title: "Song".into(), site: "www.youtube.com".into(), ..Default::default() }
    }

    #[test]
    fn the_sound_says_whether_it_goes_out_and_why_not() {
        let sound = |page, capture, deaf| Sound::new(page, capture, deaf, false, false);
        assert_eq!(sound(None, None, false), Sound::default(), "not known yet");
        assert_eq!(sound(Some(true), None, false).on, None, "the capture has not answered");
        assert_eq!(sound(Some(true), Some(true), false).on, Some(true));
        let deaf = sound(Some(true), Some(true), true);
        assert_eq!((deaf.on, deaf.problem), (Some(false), Some(SoundProblem::Silent)));
        assert_eq!(sound(Some(false), Some(true), true).problem, Some(SoundProblem::Page), "the page first");
        assert_eq!(sound(None, Some(false), true).problem, Some(SoundProblem::Unavailable));
        assert_eq!(sound(None, Some(false), false).on, Some(false));
        let pc = Sound::new(Some(true), Some(true), false, true, true);
        assert!(pc.whole_pc && pc.app_unavailable && pc.on == Some(true));
        assert_eq!(
            [SoundProblem::Page, SoundProblem::Unavailable, SoundProblem::Silent].map(SoundProblem::key),
            ["page", "unavailable", "silent"]
        );
        // Not sharing: the panel gets nothing about it.
        let notify: Notify = Arc::new(|_, _| {});
        let mut together = Together::new(notify, None, false, "", Mode::Share);
        let _ = together.set_sound(deaf);
        let view: Value = serde_json::from_str(&together.view_json()).unwrap();
        assert_eq!(view["sound"], Value::Null);
        assert_eq!(view["soundProblem"], Value::Null);
        assert_eq!(view["soundWholePc"], false);
        assert_eq!(view["soundAppUnavailable"], false);
    }

    #[test]
    fn against_a_real_relay() {
        let Some(relay) = std::env::var("YTMP_TEST_RELAY").ok().and_then(|r| link::parse_relay(&r)) else {
            eprintln!("YTMP_TEST_RELAY not set: skipped");
            return;
        };
        let mut host = App::new(&relay);
        let mut ana = App::new(&relay);
        let mut bruno = App::new(&relay);
        let mut carla = App::new(&relay);
        let mut others: Vec<App> = (0..3).map(|_| App::new(&relay)).collect();

        host.together.start("Rodrigo");
        until(&mut [&mut host], "the host's room", |a| a[0].view()["phase"] == "host");
        // The code is registered at the relay right after (a person types
        // it seconds later; the test would beat it).
        let registered = Instant::now() + Duration::from_millis(400);
        until(&mut [&mut host], "the code registered", |_| Instant::now() >= registered);
        let code = host.view()["code"].as_str().unwrap().to_owned();
        assert_eq!(code.len(), 14);

        // A code with a valid check character that nobody has.
        let wrong = Code::for_window(&[1; 32], 1).display();
        carla.together.join(&wrong, "Carla");
        until(&mut [&mut carla], "no such code", |a| a[0].view()["phase"] == "ended");
        assert_eq!(carla.view()["ended"], "together.ended_bad_code");
        carla.together.dismiss();

        ana.together.join(&code.to_lowercase(), "Ana");
        until(&mut [&mut host, &mut ana], "Ana in", |a| a[1].view()["phase"] == "guest");
        bruno.together.join(&code, "Bruno");
        until(&mut [&mut host, &mut ana, &mut bruno], "Bruno in", |a| {
            a[2].view()["phase"] == "guest" && a[0].view()["members"].as_array().unwrap().len() == 3
        });
        assert_eq!(ana.view()["hostName"], "Rodrigo");

        // Host and five guests: a sixth does not fit.
        for (i, other) in others.iter_mut().enumerate() {
            other.together.join(&code, &format!("Guest {i}"));
            until(&mut [&mut host, other], "another guest in", |a| a[1].view()["phase"] == "guest");
        }
        assert_eq!(host.view()["members"].as_array().unwrap().len(), 6);
        carla.together.join(&code, "Carla");
        until(&mut [&mut host, &mut ana, &mut bruno, &mut carla], "full", |a| a[3].view()["phase"] == "ended");
        assert_eq!(carla.view()["ended"], "together.ended_full");
        for other in &mut others {
            other.together.leave();
        }
        until(&mut [&mut host], "the others left", |a| a[0].view()["members"].as_array().unwrap().len() == 3);
        // Ana may pause and seek.
        host.together.set_member_control("g1", true);
        until(&mut [&mut host, &mut ana], "Ana has control", |a| a[1].view()["control"] == true);

        // The host plays: both guests open the same video.
        host.take();
        let effects = host.together.on_media(&playing(30.0), VIDEO);
        host.effects.extend(effects);
        let navigated = |app: &App| app.effects.iter().any(|e| matches!(e, Effect::Navigate(u) if u.as_str() == VIDEO));
        until(&mut [&mut host, &mut ana, &mut bruno], "guests open the video", |a| navigated(a[1]) && navigated(a[2]));

        // Ana's player is behind: it seeks to where the host is.
        ana.take();
        let effects = ana.together.on_media(&playing(2.0), VIDEO);
        assert!(effects.iter().any(|e| matches!(e, Effect::Media(MediaAction::SeekToMs(ms)) if *ms >= 30_000)), "{effects:?}");

        // Ana pauses on her side: the host's player pauses (guests may).
        ana.together.on_media(&playing(31.0), VIDEO);
        let effects = ana.together.on_media(&MediaState { playing: false, ..playing(31.5) }, VIDEO);
        ana.effects.extend(effects);
        until(&mut [&mut host, &mut ana], "the host pauses", |a| a[0].effects.contains(&Effect::Media(MediaAction::Pause)));
        assert!(host.effects.iter().any(|e| matches!(e, Effect::Toast(t) if t.contains("Ana"))));

        // Bruno is removed: his session ends; Ana keeps getting the state
        // (with the new room key).
        let old_code = code.clone();
        host.together.kick("g2");
        until(&mut [&mut host, &mut ana, &mut bruno], "Bruno out", |a| a[2].view()["phase"] == "ended");
        assert_eq!(bruno.view()["ended"], "together.ended_removed");
        let new_code = host.view()["code"].as_str().unwrap().to_owned();
        assert_ne!(new_code, old_code, "the code changed");
        ana.take();
        let effects = host.together.on_media(&playing(200.0), VIDEO);
        host.effects.extend(effects);
        let effects = ana.together.on_media(&MediaState { playing: false, ..playing(31.5) }, VIDEO);
        ana.effects.extend(effects);
        until(&mut [&mut host, &mut ana], "Ana follows after the new key", |a| {
            a[1].effects.iter().any(|e| matches!(e, Effect::Media(MediaAction::SeekToMs(ms)) if *ms >= 199_000))
        });

        // The host shares the window: Ana's app shows it, asks for it, and
        // the connection's offer and answer go through the relay sealed.
        const SDP: &str = "v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n";
        host.take();
        ana.take();
        let effects = host.together.set_mode(Mode::Share);
        host.effects.extend(effects);
        until(&mut [&mut host, &mut ana], "Ana shows the window", |a| a[1].effects.contains(&Effect::Watch(true)));
        let effects = ana.together.stream_ready();
        ana.effects.extend(effects);
        let peer = |app: &App| {
            app.effects.iter().find_map(|e| match e {
                Effect::Cast(json) => serde_json::from_str::<Value>(json).ok().filter(|v| v["t"] == "peer"),
                _ => None,
            })
        };
        until(&mut [&mut host, &mut ana], "the host's page makes a connection", |a| peer(a[0]).is_some());
        let share_at = host.effects.iter().position(|e| *e == Effect::Share(true));
        let peer_at = host.effects.iter().position(|e| matches!(e, Effect::Cast(j) if j.contains("\"peer\"")));
        assert!(share_at.is_some() && share_at < peer_at, "the window is captured, before its page gets messages");
        let peer = peer(&host).unwrap();
        assert_eq!(peer["id"], "g1");
        assert!(peer["ice"][0]["urls"][0].as_str().unwrap().starts_with("stun:"), "{peer}");
        let effects = host.together.cast_offer("g1", SDP);
        host.effects.extend(effects);
        let offered = |app: &App| app.effects.iter().any(|e| matches!(e, Effect::Stream(j) if j.contains("\"offer\"")));
        until(&mut [&mut host, &mut ana], "the offer reaches Ana's page", |a| offered(a[1]));
        let effects = ana.together.stream_answer(SDP);
        ana.effects.extend(effects);
        let answered = |app: &App| app.effects.iter().any(|e| matches!(e, Effect::Cast(j) if j.contains("\"answer\"")));
        until(&mut [&mut host, &mut ana], "the answer reaches the host's page", |a| answered(a[0]));
        let effects = host.together.set_mode(Mode::Sync);
        host.effects.extend(effects);
        assert!(host.effects.contains(&Effect::Share(false)));
        until(&mut [&mut host, &mut ana], "Ana goes back to her page", |a| a[1].effects.contains(&Effect::Watch(false)));

        // The old code no longer opens anything.
        carla.together.dismiss();
        carla.together.join(&old_code, "Carla");
        until(&mut [&mut host, &mut carla], "old code refused", |a| a[1].view()["phase"] == "ended");
        assert_eq!(carla.view()["ended"], "together.ended_bad_code");

        // The end: Ana is told.
        host.together.leave();
        until(&mut [&mut host, &mut ana], "ended for Ana", |a| a[1].view()["phase"] == "ended");
        assert_eq!(ana.view()["ended"], "together.ended_by_host");
        assert_eq!(host.view()["phase"], "idle");
    }
}
