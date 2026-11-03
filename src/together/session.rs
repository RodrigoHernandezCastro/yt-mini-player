//! Watch together: the host's and the guests' side of a session.
//!
//! Pure logic: messages from the relay, the player's reports and the time
//! go in; frames for the relay, actions on the player and notices come out
//! ([`Out`]). The connection itself is link.rs; the glue with the app,
//! mod.rs.

use serde_json::{json, Value};
use url::Url;

use super::{
    code::{self, Code},
    crypto::{self, Ephemeral, Key},
    link::Dial,
    protocol::{
        aad_hello, aad_rekey, aad_room, aad_welcome, clean_name, default_ice_servers, is_member_id, out, page_for,
        sanitize_ice_servers, valid_sdp, Envelope, HelloBody, HostState, IceServer, KeyBody, Member, Mode, RelayIn,
        Request, RoomMsg, Sealed, MAX_PEOPLE,
    },
    sync::{Action, Follower, Own},
};
use crate::media::{MediaAction, MediaState};

/// A guest that connected must say hello (with the code) within this time.
const HELLO_TIMEOUT_MS: u64 = 15_000;
/// The host's state goes out at most this often…
const MIN_STATE_INTERVAL_MS: u64 = 250;
/// …and at least this often.
const HEARTBEAT_MS: u64 = 10_000;
/// After a guest's request, the host's next changes carry its name.
const BY_WINDOW_MS: u64 = 3_000;
/// The previous room key still opens messages for a moment after a change.
const OLD_KEY_MS: u64 = 10_000;
/// Clock pings: a few quick ones, then this often.
const PING_EVERY_MS: u64 = 15_000;
const QUICK_PINGS: u32 = 4;
const QUICK_PING_MS: u64 = 500;
/// 2: window sharing and per-member control (0.10). Older apps are turned
/// away with a notice for the host: they would not see the window.
const PROTOCOL: u32 = 2;
/// A guest's "may I use the window?" shows this long, and is not repeated
/// sooner than `ASK_AGAIN_MS`.
const ASK_SHOWN_MS: u64 = 60_000;
const ASK_AGAIN_MS: u64 = 15_000;

/// The time, as the session sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Now {
    /// A monotonic clock, ms.
    pub ms: u64,
    /// Wall clock, Unix seconds (the code changes with it).
    pub unix: u64,
}

/// Why a session ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EndReason {
    /// The host ended it.
    Ended,
    /// The host was gone too long.
    HostGone,
    /// The host removed you.
    Removed,
    /// No room has that code (or it expired).
    BadCode,
    /// Host and two guests already.
    Full,
    /// The host is reconnecting; try again in a moment.
    HostAway,
    /// The host's app did not answer.
    NoAnswer,
    /// The relay could not be reached.
    Unreachable,
    /// The connection dropped and did not come back.
    Lost,
    /// This session was opened again somewhere else.
    Replaced,
}

impl EndReason {
    /// Its text (i18n key).
    pub fn key(self) -> &'static str {
        match self {
            EndReason::Ended => "together.ended_by_host",
            EndReason::HostGone => "together.ended_host_gone",
            EndReason::Removed => "together.ended_removed",
            EndReason::BadCode => "together.ended_bad_code",
            EndReason::Full => "together.ended_full",
            EndReason::HostAway => "together.ended_host_away",
            EndReason::NoAnswer => "together.ended_no_answer",
            EndReason::Unreachable => "together.ended_unreachable",
            EndReason::Lost => "together.ended_lost",
            EndReason::Replaced => "together.ended_replaced",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Out {
    /// A frame for the relay.
    Relay(String),
    /// Where to reconnect from now on.
    Redial(Dial),
    /// Close the connection (after what was sent).
    Close,
    Media(MediaAction),
    Navigate(Url),
    /// A notice, already in the interface's language.
    Toast(String),
    Over(EndReason),
    /// Host: a message for the page that sends the window (JSON).
    Cast(String),
    /// Guest: a message for the page that shows the host's window (JSON).
    Stream(String),
}

/// How a guest's connection to the shared window is going (host side, as
/// its page reports it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum StreamState {
    /// Not watching.
    #[default]
    None,
    Connecting,
    Watching,
    Failed,
}

impl StreamState {
    /// From the page's report (RTCPeerConnection's `connectionState`).
    pub fn from_report(state: &str) -> Option<Self> {
        Some(match state {
            "new" | "connecting" | "disconnected" => StreamState::Connecting,
            "connected" => StreamState::Watching,
            "failed" | "closed" => StreamState::Failed,
            _ => return None,
        })
    }
}

/// Most upload per guest for the window (bits/s): the host sends one copy
/// to each, so the more there are, the less each gets (6 Mbit/s at most in
/// all). Each connection measures its own path and sends less if it must;
/// with one or two guests there is room for 720p with movement.
pub fn bitrate_for(viewers: usize) -> u32 {
    match viewers {
        0 | 1 => 4_000_000,
        2 => 3_000_000,
        3 => 2_000_000,
        4 => 1_500_000,
        _ => 1_200_000,
    }
}

fn hex_id(bytes: usize) -> String {
    code::hex(&crypto::random32()[..bytes])
}

/// "Ana", or "Guest 2" if they gave no name.
pub fn display_name(id: &str, name: &str) -> String {
    if !name.is_empty() {
        name.to_owned()
    } else if id == "host" {
        t!("together.default_host")
    } else {
        t!("together.default_guest", number = id.trim_start_matches('g'))
    }
}

fn format_time(seconds: f64) -> String {
    let total = seconds.max(0.0) as u64;
    let (h, m, s) = (total / 3600, total / 60 % 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

fn to_value<T: serde::Serialize>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

// ===========================================================================
// Host
// ===========================================================================

struct Seat {
    id: String,
    name: String,
    /// Key of the handshake with this guest; `None` until its hello.
    key: Option<Key>,
    connected: bool,
    since: u64,
    last_seq: u64,
    /// May pause and seek (sync) or use the window (share).
    control: bool,
    /// When it last asked to use the window.
    asked: Option<u64>,
    /// Its app shows the host's window (it said `watch`).
    watching: bool,
    /// The sending page has a connection for it.
    peer: bool,
    stream: StreamState,
}

impl Seat {
    fn new(id: String, now: u64) -> Self {
        Self {
            id,
            name: String::new(),
            key: None,
            connected: true,
            since: now,
            last_seq: 0,
            control: false,
            asked: None,
            watching: false,
            peer: false,
            stream: StreamState::None,
        }
    }

    fn admitted(&self) -> bool {
        self.key.is_some()
    }

    /// Should have a connection to the window now?
    fn wants_peer(&self) -> bool {
        self.admitted() && self.connected && self.watching
    }
}

pub struct Host {
    room: String,
    token: String,
    secret: Key,
    window: u64,
    code: Code,
    /// The previous window's code, still good until (Unix seconds).
    previous: Option<(Code, u64)>,
    name: String,
    room_key: Key,
    epoch: u32,
    old_key: Option<(Key, u32, u64)>,
    seats: Vec<Seat>,
    mode: Mode,
    seq: u64,
    connected: bool,
    locked: bool,
    state: Option<HostState>,
    sent: Option<(HostState, u64)>,
    send_due: Option<u64>,
    by: Option<(String, u64)>,
    collisions: u32,
    ice: Vec<IceServer>,
    ice_asked: bool,
    /// The window is being captured and sent (the app follows this).
    capturing: bool,
    bitrate: u32,
}

/// What the panel shows of a member.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct MemberView {
    pub id: String,
    pub name: String,
    pub you: bool,
    pub online: bool,
    pub control: bool,
    /// Asked to use the window (host side, for a while).
    pub asking: bool,
    pub stream: StreamState,
}

impl Host {
    pub fn new(name: &str, mode: Mode, now: Now) -> Self {
        let secret = crypto::random_key();
        let window = code::window(now.unix);
        let code = Code::for_window(&secret, window);
        Self {
            room: hex_id(16),
            token: hex_id(32),
            secret,
            window,
            code,
            previous: None,
            name: clean_name(name),
            room_key: crypto::random_key(),
            epoch: 0,
            old_key: None,
            seats: Vec::new(),
            mode,
            seq: 0,
            connected: false,
            locked: false,
            state: None,
            sent: None,
            send_due: None,
            by: None,
            collisions: 0,
            ice: Vec::new(),
            ice_asked: false,
            capturing: false,
            bitrate: 0,
        }
    }

    pub fn dial(&self) -> Dial {
        Dial { path: format!("/v1/host/{}", self.room), headers: vec![("X-Ytmp-Host", self.token.clone())] }
    }

    pub fn code(&self) -> &Code {
        &self.code
    }

    /// Seconds until the code changes.
    pub fn code_left(&self, now: Now) -> u64 {
        code::window_end(self.window).saturating_sub(now.unix)
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn connected(&self) -> bool {
        self.connected
    }

    /// The window is being sent (the app captures it while this is true).
    pub fn capturing(&self) -> bool {
        self.capturing
    }

    pub fn members(&self, now: Now) -> Vec<MemberView> {
        let mut list = vec![MemberView {
            id: "host".into(),
            name: display_name("host", &self.name),
            you: true,
            online: true,
            control: true,
            asking: false,
            stream: StreamState::None,
        }];
        list.extend(self.seats.iter().filter(|s| s.admitted()).map(|s| MemberView {
            id: s.id.clone(),
            name: display_name(&s.id, &s.name),
            you: false,
            online: s.connected,
            control: s.control,
            asking: !s.control && s.asked.is_some_and(|at| now.ms < at + ASK_SHOWN_MS),
            stream: if s.watching && self.mode == Mode::Share { s.stream } else { StreamState::None },
        }));
        list
    }

    /// Locked: nothing is sent (the guests see "paused") and no one controls
    /// anything.
    pub fn set_locked(&mut self, locked: bool) -> Vec<Out> {
        if self.locked == locked {
            return Vec::new();
        }
        self.locked = locked;
        let mut outs = self.broadcast_members();
        outs.extend(self.reconcile());
        outs
    }

    /// May this guest use the window right now?
    pub fn may_control(&self, member: &str) -> bool {
        self.mode == Mode::Share
            && !self.locked
            && self.seats.iter().any(|s| s.id == member && s.control && s.wants_peer())
    }

    /// The name others see for a member.
    pub fn member_name(&self, member: &str) -> Option<String> {
        self.seats.iter().find(|s| s.id == member && s.admitted()).map(|s| display_name(&s.id, &s.name))
    }

    fn ice_servers(&self) -> Vec<IceServer> {
        if self.ice.is_empty() {
            default_ice_servers()
        } else {
            self.ice.clone()
        }
    }

    /// Brings the sending page and the capture in line with who watches:
    /// a connection for each guest that should have one, none for the rest.
    fn reconcile(&mut self) -> Vec<Out> {
        let capture = self.mode == Mode::Share && !self.locked && self.seats.iter().any(Seat::wants_peer);
        self.capturing = capture;
        let mut outs = Vec::new();
        let ice = self.ice_servers();
        let viewers = if capture { self.seats.iter().filter(|s| s.wants_peer()).count() } else { 0 };
        let bitrate = bitrate_for(viewers);
        for seat in &mut self.seats {
            let wanted = capture && seat.wants_peer();
            if seat.peer && !wanted {
                seat.peer = false;
                outs.push(Out::Cast(json!({ "t": "close", "id": seat.id }).to_string()));
            } else if wanted && !seat.peer {
                seat.peer = true;
                seat.stream = StreamState::Connecting;
                outs.push(Out::Cast(json!({ "t": "peer", "id": seat.id, "ice": ice, "bitrate": bitrate }).to_string()));
            }
        }
        if capture && bitrate != self.bitrate {
            outs.push(Out::Cast(json!({ "t": "bitrate", "bitrate": bitrate }).to_string()));
        }
        self.bitrate = if capture { bitrate } else { 0 };
        outs
    }

    /// "Share my window" or "Sync players".
    pub fn set_mode(&mut self, mode: Mode, now: Now) -> Vec<Out> {
        if self.mode == mode {
            return Vec::new();
        }
        self.mode = mode;
        for seat in &mut self.seats {
            seat.watching = false;
            seat.stream = StreamState::None;
        }
        let mut outs = self.reconcile();
        outs.extend(self.broadcast_members());
        // The guests' players pick up where the host is.
        outs.extend(self.send_state(now));
        outs
    }

    /// Lets a guest use the window (or pause and seek), or not.
    pub fn set_control(&mut self, member: &str, control: bool) -> Vec<Out> {
        let Some(seat) = self.seats.iter_mut().find(|s| s.id == member && s.admitted()) else { return Vec::new() };
        if seat.control == control {
            seat.asked = None;
            return Vec::new();
        }
        seat.control = control;
        seat.asked = None;
        let mut outs = self.broadcast_members();
        if !control {
            // Whatever it was pressing is let go.
            outs.push(Out::Cast(json!({ "t": "revoked", "id": member }).to_string()));
        }
        outs
    }

    /// Turns down a guest's request (the notice goes away).
    pub fn dismiss_ask(&mut self, member: &str) {
        if let Some(seat) = self.seats.iter_mut().find(|s| s.id == member) {
            seat.asked = None;
        }
    }

    /// The sending page made a connection offer for a guest.
    pub fn cast_offer(&mut self, member: &str, sdp: &str) -> Vec<Out> {
        let wanted = self.capturing && self.seats.iter().any(|s| s.id == member && s.peer && s.wants_peer());
        if !wanted || !valid_sdp(sdp) {
            return Vec::new();
        }
        let data = self.seal(&RoomMsg::Offer { sdp: sdp.to_owned() });
        vec![Out::Relay(out::send_to(member, &data))]
    }

    /// The sending page says how a guest's connection is going.
    pub fn cast_state(&mut self, member: &str, state: StreamState) {
        if let Some(seat) = self.seats.iter_mut().find(|s| s.id == member && s.peer) {
            seat.stream = state;
        }
    }

    /// The sending page (re)loaded: every connection is made again.
    pub fn cast_reset(&mut self) -> Vec<Out> {
        for seat in &mut self.seats {
            seat.peer = false;
        }
        self.bitrate = 0;
        self.reconcile()
    }

    fn ttl(&self, now: Now) -> u64 {
        (code::window_end(self.window) + code::GRACE_SECS).saturating_sub(now.unix).max(1)
    }

    fn advertise(&self, now: Now) -> Vec<Out> {
        let mut outs = vec![Out::Relay(out::advertise(&self.code.lookup(), self.ttl(now)))];
        if let Some((previous, until)) = &self.previous {
            if *until > now.unix {
                outs.push(Out::Relay(out::advertise(&previous.lookup(), until - now.unix)));
            }
        }
        outs
    }

    pub fn on_link_down(&mut self) {
        self.connected = false;
    }

    pub fn on_relay(&mut self, msg: RelayIn, now: Now) -> Vec<Out> {
        match msg {
            RelayIn::Ready { guests } => {
                self.connected = true;
                self.seats.retain(|s| s.key.is_some() || guests.contains(&s.id));
                for seat in &mut self.seats {
                    seat.connected = guests.contains(&seat.id);
                }
                let mut outs = self.advertise(now);
                if !std::mem::replace(&mut self.ice_asked, true) {
                    outs.push(Out::Relay(out::ice()));
                }
                outs.extend(self.broadcast_members());
                outs.extend(self.send_state(now));
                outs.extend(self.reconcile());
                outs
            }
            RelayIn::Ice { servers } => {
                self.ice = sanitize_ice_servers(servers);
                Vec::new()
            }
            RelayIn::Joined { member, resumed } => {
                if !is_member_id(&member) || member == "host" {
                    return Vec::new();
                }
                if let Some(seat) = self.seats.iter_mut().find(|s| s.id == member) {
                    seat.connected = true;
                    if resumed && seat.key.is_some() {
                        // Back after a dropped connection: the current key
                        // (it may have changed meanwhile) and where things are.
                        let mut outs: Vec<Out> = self.rekey_for(&member).into_iter().collect();
                        outs.extend(self.broadcast_members());
                        outs.extend(self.send_state(now));
                        return outs;
                    }
                    seat.since = now.ms;
                    return Vec::new();
                }
                self.seats.push(Seat::new(member, now.ms));
                Vec::new()
            }
            RelayIn::Left { member, reason } => {
                if reason == "dropped" {
                    if let Some(seat) = self.seats.iter_mut().find(|s| s.id == member) {
                        seat.connected = false;
                        // Its window connection broke too; it asks again when back.
                        seat.watching = false;
                    }
                    let mut outs = self.reconcile();
                    outs.extend(self.broadcast_members());
                    return outs;
                }
                let Some(index) = self.seats.iter().position(|s| s.id == member) else { return Vec::new() };
                let seat = self.seats.remove(index);
                let mut outs = Vec::new();
                if seat.peer {
                    outs.push(Out::Cast(json!({ "t": "close", "id": seat.id }).to_string()));
                }
                outs.extend(self.reconcile());
                if seat.key.is_none() {
                    return outs;
                }
                outs.push(Out::Toast(t!("together.toast_left", name = display_name(&seat.id, &seat.name))));
                outs.extend(self.broadcast_members());
                outs
            }
            RelayIn::Msg { from, data } => self.on_message(&from, data, now),
            RelayIn::Advertised { ok: false, .. } => {
                // Another room has that lookup (as likely as guessing a
                // 55-bit code): a new secret, a new code.
                self.collisions += 1;
                if self.collisions > 3 {
                    return Vec::new();
                }
                self.rotate(now)
            }
            RelayIn::Ended { reason } => {
                vec![Out::Over(if reason == "host-gone" { EndReason::HostGone } else { EndReason::Ended })]
            }
            _ => Vec::new(),
        }
    }

    fn on_message(&mut self, from: &str, data: Value, now: Now) -> Vec<Out> {
        let Ok(envelope) = serde_json::from_value::<Envelope>(data) else { return Vec::new() };
        match envelope {
            Envelope::Hello { eph, sealed } => self.on_hello(from, &eph, &sealed, now),
            Envelope::Room { e, sealed } => match self.open_room(from, e, &sealed, now) {
                Some(msg) => self.on_room(from, msg, now),
                None => Vec::new(),
            },
            _ => Vec::new(),
        }
    }

    fn on_hello(&mut self, from: &str, eph: &str, sealed: &str, now: Now) -> Vec<Out> {
        let Some(index) = self.seats.iter().position(|s| s.id == from && s.key.is_none()) else { return Vec::new() };
        // The current code, or the previous one during its grace.
        let mut codes = vec![self.code.clone()];
        if let Some((previous, until)) = &self.previous {
            if *until > now.unix {
                codes.push(previous.clone());
            }
        }
        let opened = codes.iter().find_map(|code| {
            let join = code.join_key();
            let plain = crypto::open(&join, &aad_hello(from), sealed)?;
            let hello: HelloBody = serde_json::from_slice(&plain).ok()?;
            Some((join, hello))
        });
        let ephemeral = Ephemeral::generate();
        let shared = opened.as_ref().and_then(|_| ephemeral.agree(eph));
        let (Some((join, hello)), Some(shared)) = (opened, shared) else {
            // Not from someone with the code: out.
            self.seats.remove(index);
            return vec![Out::Relay(out::kick(from))];
        };
        if hello.v < PROTOCOL {
            // It had the code, but its app is too old to see the window.
            self.seats.remove(index);
            let name = display_name(from, &clean_name(&hello.name));
            return vec![Out::Relay(out::kick(from)), Out::Toast(t!("together.toast_old_version", name = name))];
        }
        let key = crypto::welcome_key(&shared, &join, from);
        let body = KeyBody { room: crypto::b64(&*self.room_key), epoch: self.epoch, host: self.name.clone() };
        let welcome = Envelope::Welcome {
            eph: ephemeral.public_b64(),
            sealed: crypto::seal(&key, &aad_welcome(from), &serde_json::to_vec(&body).unwrap_or_default()),
        };
        let seat = &mut self.seats[index];
        seat.key = Some(key);
        seat.name = clean_name(&hello.name);
        seat.connected = true;
        let name = display_name(&seat.id, &seat.name);
        let mut outs = vec![Out::Relay(out::send_to(from, &to_value(&welcome))), Out::Toast(t!("together.toast_joined", name = name))];
        outs.extend(self.broadcast_members());
        outs.extend(self.send_state(now));
        outs
    }

    fn open_room(&mut self, from: &str, epoch: u32, sealed: &str, now: Now) -> Option<RoomMsg> {
        let key = if epoch == self.epoch {
            &self.room_key
        } else {
            match &self.old_key {
                Some((key, old, until)) if *old == epoch && now.ms < *until => key,
                _ => return None,
            }
        };
        let plain = crypto::open(key, &aad_room(from, epoch), sealed)?;
        let message: Sealed = serde_json::from_slice(&plain).ok()?;
        let seat = self.seats.iter_mut().find(|s| s.id == from && s.key.is_some())?;
        if message.s <= seat.last_seq {
            return None;
        }
        seat.last_seq = message.s;
        Some(message.msg)
    }

    fn on_room(&mut self, from: &str, msg: RoomMsg, now: Now) -> Vec<Out> {
        match msg {
            RoomMsg::Ping { t0 } => {
                let data = self.seal(&RoomMsg::Pong { t0, t1: now.ms });
                vec![Out::Relay(out::send_to(from, &data))]
            }
            RoomMsg::Request { action, pos } => {
                let allowed = self.seats.iter().any(|s| s.id == from && s.control);
                if !allowed || self.locked || self.mode != Mode::Sync {
                    return Vec::new();
                }
                let pos = if pos.is_finite() { pos.clamp(0.0, 1e7) } else { return Vec::new() };
                let name = self.seats.iter().find(|s| s.id == from).map(|s| display_name(&s.id, &s.name)).unwrap_or_default();
                self.by = Some((from.to_owned(), now.ms + BY_WINDOW_MS));
                let (action, toast) = match action {
                    Request::Play => (MediaAction::Play, t!("together.toast_played", name = name)),
                    Request::Pause => (MediaAction::Pause, t!("together.toast_paused", name = name)),
                    Request::Seek => (
                        MediaAction::SeekToMs((pos * 1000.0).round() as u64),
                        t!("together.toast_moved", name = name, time = format_time(pos)),
                    ),
                };
                vec![Out::Media(action), Out::Toast(toast)]
            }
            RoomMsg::Watch => {
                let Some(seat) = self.seats.iter_mut().find(|s| s.id == from) else { return Vec::new() };
                if self.mode != Mode::Share {
                    return Vec::new();
                }
                seat.watching = true;
                // Asked again (its page reloaded or the connection failed):
                // a new connection.
                let mut outs = Vec::new();
                if seat.peer {
                    seat.peer = false;
                    outs.push(Out::Cast(json!({ "t": "close", "id": seat.id }).to_string()));
                }
                outs.extend(self.reconcile());
                outs
            }
            RoomMsg::Unwatch => {
                if let Some(seat) = self.seats.iter_mut().find(|s| s.id == from) {
                    seat.watching = false;
                    seat.stream = StreamState::None;
                }
                self.reconcile()
            }
            RoomMsg::Answer { sdp } => {
                let ok = self.capturing && valid_sdp(&sdp) && self.seats.iter().any(|s| s.id == from && s.peer);
                if !ok {
                    return Vec::new();
                }
                vec![Out::Cast(json!({ "t": "answer", "id": from, "sdp": sdp }).to_string())]
            }
            RoomMsg::Ask => {
                let Some(seat) = self.seats.iter_mut().find(|s| s.id == from) else { return Vec::new() };
                if seat.control || self.mode != Mode::Share || seat.asked.is_some_and(|at| now.ms < at + ASK_AGAIN_MS) {
                    return Vec::new();
                }
                seat.asked = Some(now.ms);
                vec![Out::Toast(t!("together.toast_asks", name = display_name(&seat.id, &seat.name)))]
            }
            _ => Vec::new(),
        }
    }

    /// A room message, sealed as coming from the host.
    fn seal(&mut self, msg: &RoomMsg) -> Value {
        self.seq += 1;
        let body = serde_json::to_vec(&Sealed { s: self.seq, msg: msg.clone() }).unwrap_or_default();
        let sealed = crypto::seal(&self.room_key, &aad_room("host", self.epoch), &body);
        to_value(&Envelope::Room { e: self.epoch, sealed })
    }

    /// To every guest that has the room key.
    fn broadcast(&mut self, msg: &RoomMsg) -> Vec<Out> {
        if !self.connected {
            return Vec::new();
        }
        let ready: Vec<String> = self.seats.iter().filter(|s| s.key.is_some() && s.connected).map(|s| s.id.clone()).collect();
        if ready.is_empty() {
            return Vec::new();
        }
        let all = self.seats.iter().filter(|s| s.connected).all(|s| s.key.is_some());
        let data = self.seal(msg);
        if all {
            vec![Out::Relay(out::send_all(&data))]
        } else {
            ready.iter().map(|id| Out::Relay(out::send_to(id, &data))).collect()
        }
    }

    fn broadcast_members(&mut self) -> Vec<Out> {
        let mut list = vec![Member { id: "host".into(), name: self.name.clone(), control: true }];
        list.extend(
            self.seats
                .iter()
                .filter(|s| s.admitted())
                .map(|s| Member { id: s.id.clone(), name: s.name.clone(), control: s.control }),
        );
        let (mode, paused) = (self.mode, self.locked && self.mode == Mode::Share);
        self.broadcast(&RoomMsg::Members { list, control: false, mode, paused })
    }

    /// The room key for one guest, sealed with its handshake key.
    fn rekey_for(&self, member: &str) -> Option<Out> {
        let seat = self.seats.iter().find(|s| s.id == member)?;
        let key = seat.key.as_ref()?;
        let body = KeyBody { room: crypto::b64(&*self.room_key), epoch: self.epoch, host: self.name.clone() };
        let sealed = crypto::seal(key, &aad_rekey(member, self.epoch), &serde_json::to_vec(&body).unwrap_or_default());
        Some(Out::Relay(out::send_to(member, &to_value(&Envelope::Rekey { e: self.epoch, sealed }))))
    }

    /// What the host's player reports (and the page it is on).
    pub fn on_media(&mut self, media: &MediaState, url: &str, now: Now) -> Vec<Out> {
        let state = HostState {
            at: now.ms,
            page: page_for(url),
            title: media.title.clone(),
            site: media.site.clone(),
            media: media.has_media,
            playing: media.playing,
            pos: media.position,
            dur: media.duration,
            live: media.live,
            rate: if media.rate > 0.0 { media.rate } else { 1.0 },
            ad: media.ad,
            by: None,
        }
        .sanitized();
        let significant = match &self.sent {
            None => true,
            Some((old, _)) => {
                old.page != state.page
                    || old.media != state.media
                    || old.playing != state.playing
                    || old.ad != state.ad
                    || old.live != state.live
                    || old.title != state.title
                    || (old.rate - state.rate).abs() > 0.01
                    || (old.position_after(state.at as i64 - old.at as i64) - state.pos).abs() > 1.0
            }
        };
        self.state = Some(state);
        if significant {
            // Now, or a moment after the last one (no flood while scrubbing).
            let earliest = self.sent.as_ref().map_or(now.ms, |(_, at)| at + MIN_STATE_INTERVAL_MS).max(now.ms);
            self.send_due = Some(self.send_due.map_or(earliest, |due| due.min(earliest)));
        }
        self.flush(now)
    }

    fn flush(&mut self, now: Now) -> Vec<Out> {
        let due = self.send_due.is_some_and(|at| now.ms >= at);
        let stale = self.sent.as_ref().is_some_and(|(_, at)| now.ms >= at + HEARTBEAT_MS);
        if due || stale {
            self.send_state(now)
        } else {
            Vec::new()
        }
    }

    fn send_state(&mut self, now: Now) -> Vec<Out> {
        let Some(mut state) = self.state.clone() else { return Vec::new() };
        self.send_due = None;
        state.by = self.by.as_ref().filter(|(_, until)| now.ms < *until).map(|(id, _)| id.clone());
        self.sent = Some((state.clone(), now.ms));
        self.broadcast(&RoomMsg::State(state))
    }

    pub fn deadline(&self) -> Option<u64> {
        self.send_due
    }

    pub fn tick(&mut self, now: Now) -> Vec<Out> {
        let mut outs = Vec::new();
        let window = code::window(now.unix);
        if window != self.window {
            self.previous = Some((self.code.clone(), code::window_end(self.window) + code::GRACE_SECS));
            self.window = window;
            self.code = Code::for_window(&self.secret, window);
            if self.connected {
                outs.push(Out::Relay(out::advertise(&self.code.lookup(), self.ttl(now))));
            }
        }
        if self.previous.as_ref().is_some_and(|(_, until)| *until <= now.unix) {
            self.previous = None;
        }
        if self.old_key.as_ref().is_some_and(|(_, _, until)| now.ms >= *until) {
            self.old_key = None;
        }
        // Connected but silent: out.
        let silent: Vec<String> = self
            .seats
            .iter()
            .filter(|s| s.key.is_none() && s.connected && now.ms >= s.since + HELLO_TIMEOUT_MS)
            .map(|s| s.id.clone())
            .collect();
        for id in silent {
            self.seats.retain(|s| s.id != id);
            outs.push(Out::Relay(out::kick(&id)));
        }
        // Requests to use the window go away after a while.
        for seat in &mut self.seats {
            if seat.asked.is_some_and(|at| now.ms >= at + ASK_SHOWN_MS) {
                seat.asked = None;
            }
        }
        outs.extend(self.flush(now));
        outs
    }

    /// A new code right now (the old ones stop working).
    fn rotate(&mut self, now: Now) -> Vec<Out> {
        let mut outs = vec![Out::Relay(out::withdraw(&self.code.lookup()))];
        if let Some((previous, _)) = self.previous.take() {
            outs.push(Out::Relay(out::withdraw(&previous.lookup())));
        }
        self.secret = crypto::random_key();
        self.window = code::window(now.unix);
        self.code = Code::for_window(&self.secret, self.window);
        if self.connected {
            outs.push(Out::Relay(out::advertise(&self.code.lookup(), self.ttl(now))));
        }
        outs
    }

    pub fn new_code(&mut self, now: Now) -> Vec<Out> {
        self.rotate(now)
    }

    /// Removes a guest: it is disconnected, the code changes (it knew it)
    /// and the others get a new room key (it had the old one).
    pub fn kick(&mut self, member: &str, now: Now) -> Vec<Out> {
        let Some(index) = self.seats.iter().position(|s| s.id == member) else { return Vec::new() };
        let seat = self.seats.remove(index);
        let mut outs = vec![Out::Relay(out::kick(member))];
        if seat.peer {
            outs.push(Out::Cast(json!({ "t": "close", "id": seat.id }).to_string()));
        }
        outs.extend(self.reconcile());
        outs.extend(self.rotate(now));
        let old = std::mem::replace(&mut self.room_key, crypto::random_key());
        self.old_key = Some((old, self.epoch, now.ms + OLD_KEY_MS));
        self.epoch += 1;
        let others: Vec<String> = self.seats.iter().filter(|s| s.key.is_some() && s.connected).map(|s| s.id.clone()).collect();
        outs.extend(others.iter().filter_map(|id| self.rekey_for(id)));
        outs.extend(self.broadcast_members());
        if seat.key.is_some() {
            outs.push(Out::Toast(t!("together.toast_removed", name = display_name(&seat.id, &seat.name))));
        }
        outs
    }

    pub fn end(&mut self) -> Vec<Out> {
        vec![Out::Relay(out::end()), Out::Close]
    }
}

// ===========================================================================
// Guest
// ===========================================================================

pub struct Guest {
    code: Code,
    name: String,
    member: Option<String>,
    eph: Ephemeral,
    welcome_key: Option<Key>,
    room: Option<(Key, u32)>,
    host_name: String,
    members: Vec<Member>,
    seq: u64,
    host_seq: u64,
    pub follower: Follower,
    hello_at: Option<u64>,
    next_ping: u64,
    pings: u32,
    host_away: bool,
    locked: bool,
    mode: Mode,
    paused: bool,
    /// May pause and seek (sync) or use the window (share).
    control: bool,
    /// Its window page is open and said it is ready.
    watching: bool,
    asked: Option<u64>,
    ice: Vec<IceServer>,
    ice_asked: bool,
}

impl Guest {
    pub fn new(code: Code, name: &str) -> Self {
        Self {
            code,
            name: clean_name(name),
            member: None,
            eph: Ephemeral::generate(),
            welcome_key: None,
            room: None,
            host_name: String::new(),
            members: Vec::new(),
            seq: 0,
            host_seq: 0,
            follower: Follower::default(),
            hello_at: None,
            next_ping: 0,
            pings: 0,
            host_away: false,
            locked: false,
            mode: Mode::Sync,
            paused: false,
            control: false,
            watching: false,
            asked: None,
            ice: Vec::new(),
            ice_asked: false,
        }
    }

    pub fn dial(&self) -> Dial {
        Dial { path: format!("/v1/join/{}", self.code.lookup()), headers: Vec::new() }
    }

    pub fn mode(&self) -> Mode {
        self.mode
    }

    pub fn paused(&self) -> bool {
        self.paused
    }

    pub fn control(&self) -> bool {
        self.control
    }

    /// Its window page is open.
    pub fn watching(&self) -> bool {
        self.watching
    }

    /// Should the app show the host's window (instead of its own page)?
    pub fn wants_window(&self) -> bool {
        self.admitted() && self.mode == Mode::Share && !self.locked
    }

    fn ice_servers(&self) -> Vec<IceServer> {
        if self.ice.is_empty() {
            default_ice_servers()
        } else {
            self.ice.clone()
        }
    }

    /// What the window page shows besides the picture (JSON).
    fn stream_state(&self) -> Out {
        Out::Stream(
            json!({
                "t": "state",
                "host": self.host_name(),
                "control": self.control,
                "paused": self.paused,
                "hostAway": self.host_away,
            })
            .to_string(),
        )
    }

    /// The window page loaded (or wants a new connection): the host is
    /// asked for the window.
    pub fn stream_ready(&mut self) -> Vec<Out> {
        if !self.wants_window() {
            return Vec::new();
        }
        self.watching = true;
        let mut outs = vec![self.stream_state()];
        if let Some(data) = self.seal(&RoomMsg::Watch) {
            outs.push(Out::Relay(out::send(&data)));
        }
        outs
    }

    /// The window page answered the host's offer.
    pub fn stream_answer(&mut self, sdp: &str) -> Vec<Out> {
        if !self.watching || !valid_sdp(sdp) {
            return Vec::new();
        }
        match self.seal(&RoomMsg::Answer { sdp: sdp.to_owned() }) {
            Some(data) => vec![Out::Relay(out::send(&data))],
            None => Vec::new(),
        }
    }

    /// The app left the window page (another page, the lock screen…).
    pub fn stream_left(&mut self) -> Vec<Out> {
        if !std::mem::take(&mut self.watching) {
            return Vec::new();
        }
        match self.seal(&RoomMsg::Unwatch) {
            Some(data) => vec![Out::Relay(out::send(&data))],
            None => Vec::new(),
        }
    }

    /// "Ask to use the window".
    pub fn ask(&mut self, now: Now) -> Vec<Out> {
        if self.control || self.mode != Mode::Share || self.asked.is_some_and(|at| now.ms < at + ASK_AGAIN_MS) {
            return Vec::new();
        }
        self.asked = Some(now.ms);
        let mut outs = Vec::new();
        if let Some(data) = self.seal(&RoomMsg::Ask) {
            outs.push(Out::Relay(out::send(&data)));
            outs.push(Out::Toast(t!("together.toast_asked", name = self.host_name())));
        }
        outs
    }

    pub fn admitted(&self) -> bool {
        self.room.is_some()
    }

    pub fn host_name(&self) -> String {
        display_name("host", &self.host_name)
    }

    pub fn host_away(&self) -> bool {
        self.host_away
    }

    pub fn members(&self) -> Vec<MemberView> {
        self.members
            .iter()
            .map(|m| MemberView {
                id: m.id.clone(),
                name: display_name(&m.id, &m.name),
                you: self.member.as_deref() == Some(m.id.as_str()),
                online: true,
                control: m.control,
                asking: false,
                stream: StreamState::None,
            })
            .collect()
    }

    /// Locked: its window page closes (nothing plays behind the lock), and
    /// opens again once unlocked.
    pub fn set_locked(&mut self, locked: bool) -> Vec<Out> {
        if self.locked && !locked {
            self.follower.reset();
        }
        self.locked = locked;
        if locked {
            self.stream_left()
        } else {
            Vec::new()
        }
    }

    fn seal(&mut self, msg: &RoomMsg) -> Option<Value> {
        let (key, epoch) = self.room.as_ref()?;
        let member = self.member.as_deref()?;
        self.seq += 1;
        let body = serde_json::to_vec(&Sealed { s: self.seq, msg: msg.clone() }).ok()?;
        let sealed = crypto::seal(key, &aad_room(member, *epoch), &body);
        Some(to_value(&Envelope::Room { e: *epoch, sealed }))
    }

    pub fn on_relay(&mut self, msg: RelayIn, now: Now) -> Vec<Out> {
        match msg {
            RelayIn::Ice { servers } => {
                self.ice = sanitize_ice_servers(servers);
                Vec::new()
            }
            RelayIn::You { member, ticket, room } => {
                let valid = is_member_id(&member)
                    && member != "host"
                    && ticket.len() == 64
                    && ticket.bytes().all(|b| b.is_ascii_hexdigit())
                    && room.len() == 32
                    && room.bytes().all(|b| b.is_ascii_hexdigit());
                if !valid || self.member.as_deref().is_some_and(|m| m != member) {
                    return vec![Out::Over(EndReason::Lost)];
                }
                let redial = Dial {
                    path: format!("/v1/rejoin/{room}"),
                    headers: vec![("X-Ytmp-Member", member.clone()), ("X-Ytmp-Ticket", ticket)],
                };
                self.member = Some(member.clone());
                let mut outs = vec![Out::Redial(redial)];
                if !std::mem::replace(&mut self.ice_asked, true) {
                    outs.push(Out::Relay(out::ice()));
                }
                if self.room.is_none() {
                    let hello = HelloBody { v: PROTOCOL, name: self.name.clone() };
                    let sealed = crypto::seal(&self.code.join_key(), &aad_hello(&member), &serde_json::to_vec(&hello).unwrap_or_default());
                    let envelope = Envelope::Hello { eph: self.eph.public_b64(), sealed };
                    outs.push(Out::Relay(out::send(&to_value(&envelope))));
                    self.hello_at = Some(now.ms);
                }
                outs
            }
            RelayIn::Msg { from, data } if from == "host" => self.on_message(data, now),
            RelayIn::HostAway => {
                self.host_away = true;
                if self.watching {
                    return vec![self.stream_state()];
                }
                Vec::new()
            }
            RelayIn::HostBack => {
                self.host_away = false;
                if self.watching {
                    // Its connection to the host's window may have gone with it.
                    return self.stream_ready();
                }
                Vec::new()
            }
            RelayIn::Ended { reason } => {
                vec![Out::Over(if reason == "host-gone" { EndReason::HostGone } else { EndReason::Ended })]
            }
            _ => Vec::new(),
        }
    }

    fn on_message(&mut self, data: Value, now: Now) -> Vec<Out> {
        let Some(member) = self.member.clone() else { return Vec::new() };
        let Ok(envelope) = serde_json::from_value::<Envelope>(data) else { return Vec::new() };
        match envelope {
            Envelope::Welcome { eph, sealed } if self.room.is_none() => {
                let Some(shared) = self.eph.agree(&eph) else { return Vec::new() };
                let key = crypto::welcome_key(&shared, &self.code.join_key(), &member);
                let Some(body) = crypto::open(&key, &aad_welcome(&member), &sealed)
                    .and_then(|plain| serde_json::from_slice::<KeyBody>(&plain).ok())
                else {
                    return Vec::new();
                };
                let Some(room_key) = crypto::key_from_b64(&body.room) else { return Vec::new() };
                self.room = Some((room_key, body.epoch));
                self.host_name = clean_name(&body.host);
                self.welcome_key = Some(key);
                self.hello_at = None;
                self.next_ping = now.ms;
                self.pings = 0;
                self.tick(now)
            }
            Envelope::Rekey { e, sealed } => {
                let Some(key) = &self.welcome_key else { return Vec::new() };
                let body = crypto::open(key, &aad_rekey(&member, e), &sealed)
                    .and_then(|plain| serde_json::from_slice::<KeyBody>(&plain).ok())
                    .filter(|body| body.epoch == e);
                if let Some(room_key) = body.and_then(|b| crypto::key_from_b64(&b.room)) {
                    self.room = Some((room_key, e));
                }
                Vec::new()
            }
            Envelope::Room { e, sealed } => {
                let Some((key, epoch)) = &self.room else { return Vec::new() };
                if e != *epoch {
                    return Vec::new();
                }
                let Some(message) =
                    crypto::open(key, &aad_room("host", e), &sealed).and_then(|plain| serde_json::from_slice::<Sealed>(&plain).ok())
                else {
                    return Vec::new();
                };
                if message.s <= self.host_seq {
                    return Vec::new();
                }
                self.host_seq = message.s;
                self.on_room(message.msg, now)
            }
            _ => Vec::new(),
        }
    }

    fn on_room(&mut self, msg: RoomMsg, now: Now) -> Vec<Out> {
        match msg {
            RoomMsg::State(state) => {
                let state = state.sanitized();
                let mut outs = Vec::new();
                // Another guest paused or moved it: say who.
                if let (Some(by), Some(old)) = (state.by.as_deref(), self.follower.host()) {
                    if Some(by) != self.member.as_deref() && (old.playing != state.playing || (old.pos - state.pos).abs() > 3.0) {
                        let name = self.members.iter().find(|m| m.id == by).map(|m| display_name(&m.id, &m.name));
                        if let Some(name) = name {
                            let text = if old.playing != state.playing {
                                if state.playing { t!("together.toast_played", name = name) } else { t!("together.toast_paused", name = name) }
                            } else {
                                t!("together.toast_moved", name = name, time = format_time(state.pos))
                            };
                            outs.push(Out::Toast(text));
                        }
                    }
                }
                let actions = self.follower.on_host_state(state, now.ms);
                // Sharing: the window shows the host's video, the own player
                // is not touched.
                if self.mode == Mode::Sync {
                    outs.extend(self.act(actions));
                }
                outs
            }
            RoomMsg::Members { list, control, mode, paused } => {
                self.members = list
                    .into_iter()
                    .filter(|m| is_member_id(&m.id))
                    .take(MAX_PEOPLE + 2)
                    .map(|m| Member { id: m.id, name: clean_name(&m.name), control: m.control })
                    .collect();
                if let Some(host) = self.members.iter().find(|m| m.id == "host") {
                    self.host_name = host.name.clone();
                }
                let own = self.members.iter().any(|m| Some(m.id.as_str()) == self.member.as_deref() && m.control);
                let mut outs = Vec::new();
                let had = self.control;
                self.control = own || control;
                self.follower.control = self.control;
                if mode == Mode::Share && self.mode == Mode::Share && had != self.control {
                    outs.push(Out::Toast(if self.control {
                        t!("together.toast_control_on", name = self.host_name())
                    } else {
                        t!("together.toast_control_off")
                    }));
                }
                if self.control {
                    self.asked = None;
                }
                if mode != self.mode {
                    self.mode = mode;
                    if mode == Mode::Sync {
                        // Back to its own player: it follows the host from scratch.
                        self.watching = false;
                        self.follower.reset();
                        let actions = self.follower.evaluate(now.ms);
                        outs.extend(self.act(actions));
                    }
                }
                self.paused = paused;
                if self.watching {
                    outs.push(self.stream_state());
                }
                outs
            }
            RoomMsg::Offer { sdp } => {
                if !self.watching || !valid_sdp(&sdp) {
                    return Vec::new();
                }
                vec![Out::Stream(json!({ "t": "offer", "sdp": sdp, "ice": self.ice_servers() }).to_string())]
            }
            RoomMsg::Pong { t0, t1 } => {
                self.follower.on_pong(t0, t1, now.ms);
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn act(&mut self, actions: Vec<Action>) -> Vec<Out> {
        let mut outs = Vec::new();
        for action in actions {
            match action {
                Action::Media(media) if !self.locked => outs.push(Out::Media(media)),
                Action::Navigate(url) if !self.locked => outs.push(Out::Navigate(url)),
                Action::Request(request, pos) => {
                    if let Some(data) = self.seal(&RoomMsg::Request { action: request, pos }) {
                        outs.push(Out::Relay(out::send(&data)));
                    }
                }
                Action::NoControl => outs.push(Out::Toast(t!("together.only_host"))),
                _ => {}
            }
        }
        outs
    }

    pub fn on_media(&mut self, media: &MediaState, url: &str, now: Now) -> Vec<Out> {
        if !self.admitted() || self.mode == Mode::Share {
            return Vec::new();
        }
        let own = Own {
            media: media.has_media,
            playing: media.playing,
            pos: media.position,
            live: media.live,
            rate: if media.rate > 0.0 { media.rate } else { 1.0 },
            ad: media.ad,
            identity: Url::parse(url).ok().map(|u| super::protocol::identity(&u)),
        };
        let actions = self.follower.on_own(own, now.ms);
        self.act(actions)
    }

    pub fn tick(&mut self, now: Now) -> Vec<Out> {
        if self.hello_at.is_some_and(|at| now.ms >= at + HELLO_TIMEOUT_MS) {
            return vec![Out::Over(EndReason::NoAnswer), Out::Close];
        }
        if !self.admitted() {
            return Vec::new();
        }
        let mut outs = Vec::new();
        if now.ms >= self.next_ping && !self.host_away {
            if let Some(data) = self.seal(&RoomMsg::Ping { t0: now.ms }) {
                outs.push(Out::Relay(out::send(&data)));
            }
            self.pings += 1;
            self.next_ping = now.ms + if self.pings < QUICK_PINGS { QUICK_PING_MS } else { PING_EVERY_MS };
        }
        if self.mode == Mode::Sync {
            let actions = self.follower.evaluate(now.ms);
            outs.extend(self.act(actions));
        }
        outs
    }

    pub fn deadline(&self) -> Option<u64> {
        self.admitted().then_some(self.next_ping)
    }

    pub fn resync(&mut self, now: Now) -> Vec<Out> {
        if self.mode == Mode::Share {
            // A new connection to the host's window.
            return self.stream_ready();
        }
        self.follower.reset();
        let actions = self.follower.evaluate(now.ms);
        self.act(actions)
    }

    pub fn leave(&mut self) -> Vec<Out> {
        vec![Out::Relay(out::leave()), Out::Close]
    }
}

// ===========================================================================
// Tests: a host and two guests through an in-memory relay
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::together::protocol::{parse_relay, Page};
    use serde_json::json;

    const VIDEO: &str = "https://www.youtube.com/watch?v=dQw4w9WgXcQ";

    /// What the relay would do with each frame (no network).
    struct Relay {
        next: u32,
        lookups: Vec<String>,
        kicked: Vec<String>,
        ended: bool,
    }

    fn now(ms: u64) -> Now {
        Now { ms, unix: 1_800_000_000 + ms / 1000 }
    }

    fn relays(outs: &[Out]) -> Vec<Value> {
        outs.iter()
            .filter_map(|o| match o {
                Out::Relay(text) => Some(serde_json::from_str(text).unwrap()),
                _ => None,
            })
            .collect()
    }

    fn msg(from: &str, data: &Value) -> RelayIn {
        parse_relay(&json!({"op": "msg", "from": from, "data": data}).to_string()).unwrap()
    }

    impl Relay {
        fn new() -> Self {
            Self { next: 1, lookups: Vec::new(), kicked: Vec::new(), ended: false }
        }

        /// Delivers the host's frames to the guests (by member id).
        fn deliver(&mut self, outs: &[Out], guests: &mut [(&str, &mut Guest)], t: Now) -> Vec<Out> {
            let mut back = Vec::new();
            for frame in relays(outs) {
                match frame["op"].as_str().unwrap() {
                    "advertise" => self.lookups.push(frame["lookup"].as_str().unwrap().to_owned()),
                    "withdraw" => self.lookups.retain(|l| l != frame["lookup"].as_str().unwrap()),
                    "kick" => self.kicked.push(frame["member"].as_str().unwrap().to_owned()),
                    "end" => self.ended = true,
                    "ice" => {}
                    "send" => {
                        for (id, guest) in guests.iter_mut() {
                            if frame["to"] == "*" || frame["to"] == *id {
                                back.extend(guest.on_relay(msg("host", &frame["data"]), t));
                            }
                        }
                    }
                    other => panic!("{other}"),
                }
            }
            back
        }

        /// A guest connects with its code: its id and its hello.
        fn join(&mut self, host: &mut Host, guest: &mut Guest, t: Now) -> (String, Vec<Out>) {
            let lookup = guest.code.lookup();
            assert!(self.lookups.contains(&lookup), "the code is advertised");
            let member = format!("g{}", self.next);
            self.next += 1;
            host.on_relay(RelayIn::Joined { member: member.clone(), resumed: false }, t);
            let outs = guest.on_relay(RelayIn::You { member: member.clone(), ticket: "a".repeat(64), room: "b".repeat(32) }, t);
            assert!(matches!(outs[0], Out::Redial(_)));
            (member, outs)
        }
    }

    /// Guest frames to the host; the host's answers back to the guests.
    fn guest_to_host(relay: &mut Relay, host: &mut Host, from: &str, outs: &[Out], guests: &mut [(&str, &mut Guest)], t: Now) -> Vec<Out> {
        let mut answers = Vec::new();
        for frame in relays(outs) {
            if frame["op"] == "ice" {
                continue;
            }
            assert_eq!(frame["op"], "send");
            answers.extend(host.on_relay(msg(from, &frame["data"]), t));
        }
        let back = relay.deliver(&answers, guests, t);
        answers.extend(back);
        answers
    }

    fn playing(position: f64) -> MediaState {
        MediaState { has_media: true, playing: true, position, duration: 600.0, rate: 1.0, title: "Song".into(), site: "www.youtube.com".into(), ..Default::default() }
    }

    #[test]
    fn a_host_two_guests_a_removal_and_a_new_key() {
        let mut relay = Relay::new();
        let mut host = Host::new("Rodrigo", Mode::Sync, now(0));
        let outs = host.on_relay(RelayIn::Ready { guests: vec![] }, now(0));
        relay.deliver(&outs, &mut [], now(0));
        assert_eq!(relay.lookups, vec![host.code().lookup()]);

        // Ana types the code with spaces and in lower case.
        let typed = host.code().display().to_lowercase().replace('-', " ");
        let mut ana = Guest::new(Code::parse(&typed).unwrap(), "Ana");
        let (ana_id, hello) = relay.join(&mut host, &mut ana, now(100));
        let answers = guest_to_host(&mut relay, &mut host, &ana_id, &hello, &mut [(&ana_id, &mut ana)], now(120));
        assert!(ana.admitted(), "welcomed");
        assert!(answers.iter().any(|o| matches!(o, Out::Toast(t) if t.contains("Ana"))));
        assert_eq!(ana.host_name(), "Rodrigo");
        assert_eq!(ana.members().len(), 2);

        // Bruno with the same code.
        let mut bruno = Guest::new(host.code().clone(), "");
        let (bruno_id, hello) = relay.join(&mut host, &mut bruno, now(200));
        guest_to_host(&mut relay, &mut host, &bruno_id, &hello, &mut [(&ana_id, &mut ana), (&bruno_id, &mut bruno)], now(220));
        assert!(bruno.admitted());
        assert_eq!(host.members(now(220)).len(), 3);
        assert_eq!(ana.members().len(), 3, "everyone gets the new list");
        // Ana may pause and seek; Bruno may not.
        let outs = host.set_control(&ana_id, true);
        relay.deliver(&outs, &mut [(&ana_id, &mut ana), (&bruno_id, &mut bruno)], now(230));
        assert!(ana.control() && !bruno.control());

        // The host plays a video: both guests open it.
        let outs = host.on_media(&playing(30.0), VIDEO, now(1_000));
        let back = relay.deliver(&outs, &mut [(&ana_id, &mut ana), (&bruno_id, &mut bruno)], now(1_010));
        let navigations = back.iter().filter(|o| matches!(o, Out::Navigate(u) if u.as_str() == VIDEO)).count();
        assert_eq!(navigations, 2);

        // Ana's player gets there and is behind: it seeks.
        let outs = ana.on_media(&MediaState { position: 5.0, ..playing(5.0) }, VIDEO, now(2_000));
        assert!(outs.iter().any(|o| matches!(o, Out::Media(MediaAction::SeekToMs(ms)) if (30_900..=31_500).contains(ms))), "{outs:?}");

        // Ana pauses on her side: she has control, so it is a request.
        ana.on_media(&playing(33.0), VIDEO, now(5_000));
        let outs = ana.on_media(&MediaState { playing: false, ..playing(34.0) }, VIDEO, now(6_000));
        let back = guest_to_host(&mut relay, &mut host, &ana_id, &outs, &mut [], now(6_050));
        assert!(back.contains(&Out::Media(MediaAction::Pause)), "the host's player pauses: {back:?}");
        assert!(back.iter().any(|o| matches!(o, Out::Toast(t) if t.contains("Ana"))));
        // The host's report goes out with "by Ana": Bruno is told who.
        let outs = host.on_media(&MediaState { playing: false, ..playing(34.0) }, VIDEO, now(6_400));
        let back = relay.deliver(&outs, &mut [(&bruno_id, &mut bruno)], now(6_410));
        assert!(back.iter().any(|o| matches!(o, Out::Toast(t) if t.contains("Ana"))), "{back:?}");

        // Bruno is removed: kicked, new code, new key for Ana.
        let old_lookup = host.code().lookup();
        let outs = host.kick(&bruno_id, now(7_000));
        relay.deliver(&outs, &mut [(&ana_id, &mut ana)], now(7_000));
        assert_eq!(relay.kicked, vec![bruno_id.clone()]);
        assert!(!relay.lookups.contains(&old_lookup), "the old code no longer works");
        assert!(relay.lookups.contains(&host.code().lookup()));
        assert_eq!(ana.room.as_ref().unwrap().1, 1, "Ana has epoch 1");
        // Bruno, with the old key, cannot read what follows.
        let outs = host.on_media(&playing(40.0), VIDEO, now(8_000));
        let before = bruno.host_seq;
        relay.deliver(&outs, &mut [(&bruno_id, &mut bruno)], now(8_000));
        assert_eq!(bruno.host_seq, before);
        let before = ana.host_seq;
        relay.deliver(&outs, &mut [(&ana_id, &mut ana)], now(8_000));
        assert!(ana.host_seq > before, "Ana can");

        // The end.
        let outs = host.end();
        relay.deliver(&outs, &mut [], now(9_000));
        assert!(relay.ended && outs.contains(&Out::Close));
    }

    #[test]
    fn a_wrong_or_forged_hello_is_turned_away() {
        let mut relay = Relay::new();
        let mut host = Host::new("", Mode::Sync, now(0));
        relay.deliver(&host.on_relay(RelayIn::Ready { guests: vec![] }, now(0)), &mut [], now(0));
        // Someone who reached the room but has another code.
        let other = Code::for_window(&[9; 32], 5);
        let mut eve = Guest::new(other, "Eve");
        let id = "g1".to_owned();
        host.on_relay(RelayIn::Joined { member: id.clone(), resumed: false }, now(0));
        let hello = eve.on_relay(RelayIn::You { member: id.clone(), ticket: "a".repeat(64), room: "b".repeat(32) }, now(0));
        let answers = guest_to_host(&mut relay, &mut host, &id, &hello, &mut [], now(10));
        assert_eq!(relays(&answers), vec![json!({"op": "kick", "member": "g1"})]);
        assert_eq!(host.members(now(10)).len(), 1);
        // Someone silent is removed after a while.
        host.on_relay(RelayIn::Joined { member: "g2".into(), resumed: false }, now(1_000));
        assert!(relays(&host.tick(now(2_000))).is_empty());
        assert_eq!(relays(&host.tick(now(1_000 + HELLO_TIMEOUT_MS))), vec![json!({"op": "kick", "member": "g2"})]);
    }

    #[test]
    fn requests_need_control_and_replays_are_dropped() {
        let mut relay = Relay::new();
        let mut host = Host::new("", Mode::Sync, now(0));
        relay.deliver(&host.on_relay(RelayIn::Ready { guests: vec![] }, now(0)), &mut [], now(0));
        let mut ana = Guest::new(host.code().clone(), "");
        let (id, hello) = relay.join(&mut host, &mut ana, now(0));
        guest_to_host(&mut relay, &mut host, &id, &hello, &mut [(&id, &mut ana)], now(0));
        let request = ana.seal(&RoomMsg::Request { action: Request::Pause, pos: 3.0 }).unwrap();
        assert!(host.on_relay(msg(&id, &request), now(10)).is_empty(), "control is off");
        host.set_control(&id, true);
        let request = ana.seal(&RoomMsg::Request { action: Request::Pause, pos: 3.0 }).unwrap();
        assert!(host.on_relay(msg(&id, &request), now(20)).contains(&Out::Media(MediaAction::Pause)));
        assert!(host.on_relay(msg(&id, &request), now(30)).is_empty(), "the same message again");
        // Pretending to be another member does not open.
        let forged = ana.seal(&RoomMsg::Request { action: Request::Play, pos: 0.0 }).unwrap();
        assert!(host.on_relay(msg("g9", &forged), now(40)).is_empty());
        // Locked host: nothing plays behind the lock screen.
        let _ = host.set_locked(true);
        let request = ana.seal(&RoomMsg::Request { action: Request::Play, pos: 0.0 }).unwrap();
        assert!(host.on_relay(msg(&id, &request), now(50)).is_empty());
    }

    #[test]
    fn the_code_changes_every_thirty_minutes_and_the_old_one_has_a_grace() {
        let start = Now { ms: 0, unix: 1_800_000_000 - 60 };
        let mut relay = Relay::new();
        let mut host = Host::new("", Mode::Sync, start);
        relay.deliver(&host.on_relay(RelayIn::Ready { guests: vec![] }, start), &mut [], start);
        let first = host.code().clone();
        assert_eq!(host.code_left(start), 60);
        let later = Now { ms: 61_000, unix: 1_800_000_001 };
        relay.deliver(&host.tick(later), &mut [], later);
        assert_ne!(host.code(), &first);
        assert!(relay.lookups.contains(&host.code().lookup()));
        // A guest with the old code, two minutes later, still gets in.
        let mut late = Guest::new(first.clone(), "");
        let (id, hello) = relay.join(&mut host, &mut late, later);
        let t = Now { ms: 120_000, unix: 1_800_000_120 };
        guest_to_host(&mut relay, &mut host, &id, &hello, &mut [(&id, &mut late)], t);
        assert!(late.admitted());
        // After the grace, no longer.
        let t = Now { ms: 400_000, unix: 1_800_000_000 + code::GRACE_SECS + 1 };
        host.tick(t);
        let mut too_late = Guest::new(first, "");
        host.on_relay(RelayIn::Joined { member: "g9".into(), resumed: false }, t);
        let hello = too_late.on_relay(RelayIn::You { member: "g9".into(), ticket: "a".repeat(64), room: "b".repeat(32) }, t);
        let answers = guest_to_host(&mut relay, &mut host, "g9", &hello, &mut [], t);
        assert_eq!(relays(&answers), vec![json!({"op": "kick", "member": "g9"})]);
    }

    #[test]
    fn a_dropped_guest_comes_back_with_the_current_key() {
        let mut relay = Relay::new();
        let mut host = Host::new("", Mode::Sync, now(0));
        relay.deliver(&host.on_relay(RelayIn::Ready { guests: vec![] }, now(0)), &mut [], now(0));
        let mut a = Guest::new(host.code().clone(), "A");
        let mut b = Guest::new(host.code().clone(), "B");
        let (a_id, hello) = relay.join(&mut host, &mut a, now(0));
        guest_to_host(&mut relay, &mut host, &a_id, &hello, &mut [(&a_id, &mut a)], now(0));
        let (b_id, hello) = relay.join(&mut host, &mut b, now(0));
        guest_to_host(&mut relay, &mut host, &b_id, &hello, &mut [(&a_id, &mut a), (&b_id, &mut b)], now(0));
        // A drops; meanwhile B is removed (new key A did not get).
        host.on_relay(RelayIn::Left { member: a_id.clone(), reason: "dropped".into() }, now(100));
        relay.deliver(&host.kick(&b_id, now(200)), &mut [], now(200));
        assert_eq!(a.room.as_ref().unwrap().1, 0);
        // A comes back: the current key, the members and the state.
        let outs = host.on_relay(RelayIn::Joined { member: a_id.clone(), resumed: true }, now(300));
        let again = a.on_relay(RelayIn::You { member: a_id.clone(), ticket: "a".repeat(64), room: "b".repeat(32) }, now(300));
        assert!(relays(&again).is_empty(), "no second hello");
        relay.deliver(&outs, &mut [(&a_id, &mut a)], now(300));
        assert_eq!(a.room.as_ref().unwrap().1, 1);
        assert_eq!(a.members().len(), 2);
    }

    #[test]
    fn the_host_shares_pages_and_sends_little() {
        let mut host = Host::new("", Mode::Sync, now(0));
        host.on_relay(RelayIn::Ready { guests: vec![] }, now(0));
        host.on_relay(RelayIn::Joined { member: "g1".into(), resumed: false }, now(0));
        host.seats[0].key = Some(crypto::random_key());
        let sent = |outs: &[Out]| relays(outs).iter().filter(|f| f["op"] == "send").count();
        assert_eq!(sent(&host.on_media(&playing(10.0), VIDEO, now(1_000))), 1, "first report");
        // Every second while playing: nothing new to say.
        for s in 1..4u64 {
            assert_eq!(sent(&host.on_media(&playing(10.0 + s as f64), VIDEO, now(1_000 + s * 1000))), 0);
        }
        // Paused: at once.
        assert_eq!(sent(&host.on_media(&MediaState { playing: false, ..playing(13.2) }, VIDEO, now(4_200))), 1);
        // A jump right after: waits a moment (no flood while scrubbing)...
        assert_eq!(sent(&host.on_media(&MediaState { playing: false, ..playing(100.0) }, VIDEO, now(4_300))), 0);
        assert_eq!(host.deadline(), Some(4_450));
        assert_eq!(sent(&host.tick(now(4_450))), 1);
        // ...and a heartbeat now and then.
        assert_eq!(sent(&host.tick(now(4_450 + HEARTBEAT_MS))), 1);
        // On the start screen: the guests see where the host is.
        host.on_media(&MediaState::default(), "https://ytmp.localhost/", now(20_000));
        assert_eq!(host.state.as_ref().unwrap().page, Page::Home);
    }

    fn casts(outs: &[Out]) -> Vec<Value> {
        outs.iter()
            .filter_map(|o| match o {
                Out::Cast(text) => Some(serde_json::from_str(text).unwrap()),
                _ => None,
            })
            .collect()
    }

    fn streams(outs: &[Out]) -> Vec<Value> {
        outs.iter()
            .filter_map(|o| match o {
                Out::Stream(text) => Some(serde_json::from_str(text).unwrap()),
                _ => None,
            })
            .collect()
    }

    const SDP: &str = "v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\ns=-\r\nt=0 0\r\n";

    #[test]
    fn sharing_the_window_with_three_guests() {
        let mut relay = Relay::new();
        let mut host = Host::new("Rodrigo", Mode::Share, now(0));
        let outs = host.on_relay(RelayIn::Ready { guests: vec![] }, now(0));
        assert!(relays(&outs).iter().any(|f| f["op"] == "ice"), "asks the relay for its servers");
        relay.deliver(&outs, &mut [], now(0));
        let turn = IceServer {
            urls: vec!["turn:turn.example:3478".into()],
            username: Some("u".into()),
            credential: Some("c".into()),
        };
        host.on_relay(RelayIn::Ice { servers: vec![turn.clone()] }, now(0));

        let mut ana = Guest::new(host.code().clone(), "Ana");
        let mut bruno = Guest::new(host.code().clone(), "Bruno");
        let mut carla = Guest::new(host.code().clone(), "Carla");
        let mut ids = Vec::new();
        for (t, guest) in [&mut ana, &mut bruno, &mut carla].into_iter().enumerate() {
            let t = now(100 + t as u64 * 100);
            let (id, hello) = relay.join(&mut host, guest, t);
            guest_to_host(&mut relay, &mut host, &id, &hello, &mut [(&id, guest)], t);
            assert!(guest.wants_window() && guest.mode() == Mode::Share, "{id} shows the host's window");
            ids.push(id);
        }
        let (a, b, c) = (ids[0].clone(), ids[1].clone(), ids[2].clone());
        assert!(!host.capturing(), "nobody watches yet: nothing is captured");

        // Ana's window page is ready: the host's page makes her a connection.
        let outs = ana.stream_ready();
        assert_eq!(streams(&outs)[0]["t"], "state");
        assert_eq!(streams(&outs)[0]["host"], "Rodrigo");
        let back = guest_to_host(&mut relay, &mut host, &a, &outs, &mut [], now(1_000));
        let peer = &casts(&back)[0];
        assert_eq!((peer["t"].as_str(), peer["id"].as_str()), (Some("peer"), Some(a.as_str())));
        assert_eq!(peer["ice"][0]["urls"][0], "turn:turn.example:3478");
        assert_eq!(peer["bitrate"], 4_000_000);
        assert!(host.capturing());

        // Its offer reaches Ana's page (with Ana's own servers); her answer,
        // the host's page.
        let outs = host.cast_offer(&a, SDP);
        let back = relay.deliver(&outs, &mut [(&a, &mut ana)], now(1_100));
        let offer = &streams(&back)[0];
        assert_eq!((offer["t"].as_str(), offer["sdp"].as_str()), (Some("offer"), Some(SDP)));
        assert!(offer["ice"][0]["urls"][0].as_str().unwrap().starts_with("stun:"), "Ana had no servers from the relay");
        let outs = ana.stream_answer(SDP);
        let back = guest_to_host(&mut relay, &mut host, &a, &outs, &mut [], now(1_200));
        assert_eq!(casts(&back), vec![json!({"t": "answer", "id": a, "sdp": SDP})]);
        host.cast_state(&a, StreamState::Watching);
        assert_eq!(host.members(now(1_200))[1].stream, StreamState::Watching);

        // Two more: each copy gets less upload.
        let outs = bruno.stream_ready();
        guest_to_host(&mut relay, &mut host, &b, &outs, &mut [], now(1_300));
        let outs = carla.stream_ready();
        let back = guest_to_host(&mut relay, &mut host, &c, &outs, &mut [], now(1_400));
        assert!(casts(&back).contains(&json!({"t": "bitrate", "bitrate": 2_000_000})), "{back:?}");

        // Nobody may use the window until the host says so; Bruno asks.
        assert!(!host.may_control(&a));
        let outs = bruno.ask(now(2_000));
        assert!(outs.iter().any(|o| matches!(o, Out::Toast(t) if t.contains("Rodrigo"))));
        let back = guest_to_host(&mut relay, &mut host, &b, &outs, &mut [], now(2_000));
        assert!(back.iter().any(|o| matches!(o, Out::Toast(t) if t.contains("Bruno"))));
        assert!(host.members(now(2_000))[2].asking);
        assert!(bruno.ask(now(3_000)).is_empty(), "not again so soon");
        let outs = host.set_control(&b, true);
        let back = relay.deliver(&outs, &mut [(&a, &mut ana), (&b, &mut bruno), (&c, &mut carla)], now(2_100));
        assert!(back.iter().any(|o| matches!(o, Out::Toast(t) if t.contains("Rodrigo"))), "Bruno is told");
        assert!(bruno.control() && !ana.control());
        assert!(host.may_control(&b) && !host.may_control(&a));
        assert!(!host.members(now(2_100))[2].asking);
        // Taken back: whatever Bruno held is let go.
        let outs = host.set_control(&b, false);
        assert!(casts(&outs).contains(&json!({"t": "revoked", "id": b})));
        relay.deliver(&outs, &mut [(&b, &mut bruno)], now(2_200));
        assert!(!bruno.control());

        // The host locks the app: nothing is sent, the guests see "paused".
        let outs = host.set_locked(true);
        assert!(!host.capturing());
        assert_eq!(casts(&outs).iter().filter(|m| m["t"] == "close").count(), 3);
        let back = relay.deliver(&outs, &mut [(&a, &mut ana)], now(3_000));
        assert!(ana.paused() && streams(&back).iter().any(|m| m["paused"] == true));
        let outs = host.set_locked(false);
        assert_eq!(casts(&outs).iter().filter(|m| m["t"] == "peer").count(), 3, "everyone again");
        relay.deliver(&outs, &mut [(&a, &mut ana), (&b, &mut bruno), (&c, &mut carla)], now(3_100));
        assert!(!ana.paused());

        // Carla is removed: her connection closes and the others get more.
        let outs = host.kick(&c, now(4_000));
        let cast = casts(&outs);
        assert!(cast.contains(&json!({"t": "close", "id": c})));
        assert!(cast.contains(&json!({"t": "bitrate", "bitrate": 3_000_000})), "{cast:?}");
        relay.deliver(&outs, &mut [(&a, &mut ana), (&b, &mut bruno)], now(4_000));

        // Ana leaves the window page: her connection closes.
        let outs = ana.stream_left();
        let back = guest_to_host(&mut relay, &mut host, &a, &outs, &mut [], now(5_000));
        assert_eq!(casts(&back), vec![json!({"t": "close", "id": a}), json!({"t": "bitrate", "bitrate": 4_000_000})]);
        assert!(ana.stream_left().is_empty(), "once");

        // Back to synced players: nothing is captured, and the guests follow
        // the host's video again.
        host.on_media(&playing(30.0), VIDEO, now(6_000));
        let outs = host.set_mode(Mode::Sync, now(6_100));
        assert!(!host.capturing());
        let back = relay.deliver(&outs, &mut [(&a, &mut ana), (&b, &mut bruno)], now(6_200));
        assert!(!ana.wants_window() && ana.mode() == Mode::Sync);
        assert!(back.iter().any(|o| matches!(o, Out::Navigate(u) if u.as_str() == VIDEO)), "{back:?}");
    }

    #[test]
    fn sharing_guests_keep_their_own_player_still() {
        let mut relay = Relay::new();
        let mut host = Host::new("", Mode::Share, now(0));
        relay.deliver(&host.on_relay(RelayIn::Ready { guests: vec![] }, now(0)), &mut [], now(0));
        let mut ana = Guest::new(host.code().clone(), "");
        let (id, hello) = relay.join(&mut host, &mut ana, now(0));
        guest_to_host(&mut relay, &mut host, &id, &hello, &mut [(&id, &mut ana)], now(0));
        let outs = host.on_media(&playing(30.0), VIDEO, now(1_000));
        let back = relay.deliver(&outs, &mut [(&id, &mut ana)], now(1_000));
        assert!(!back.iter().any(|o| matches!(o, Out::Navigate(_) | Out::Media(_))), "{back:?}");
        assert!(ana.on_media(&playing(2.0), VIDEO, now(2_000)).is_empty());
        assert!(ana.tick(now(3_000)).iter().all(|o| matches!(o, Out::Relay(_))));
        // Locked guest: its window page closes, and nothing is asked.
        ana.stream_ready();
        let outs = ana.set_locked(true);
        assert!(!ana.wants_window() && !ana.watching());
        assert_eq!(relays(&outs).len(), 1, "unwatch");
        assert!(ana.stream_ready().is_empty());
        ana.set_locked(false);
        assert!(ana.wants_window());
    }

    #[test]
    fn descriptions_and_requests_are_checked() {
        let mut relay = Relay::new();
        let mut host = Host::new("", Mode::Share, now(0));
        relay.deliver(&host.on_relay(RelayIn::Ready { guests: vec![] }, now(0)), &mut [], now(0));
        let mut ana = Guest::new(host.code().clone(), "");
        let (id, hello) = relay.join(&mut host, &mut ana, now(0));
        guest_to_host(&mut relay, &mut host, &id, &hello, &mut [(&id, &mut ana)], now(0));
        // Not watching yet: no offer goes out, an answer is dropped.
        assert!(host.cast_offer(&id, SDP).is_empty());
        let answer = ana.seal(&RoomMsg::Answer { sdp: SDP.into() }).unwrap();
        assert!(host.on_relay(msg(&id, &answer), now(10)).is_empty());
        let outs = ana.stream_ready();
        guest_to_host(&mut relay, &mut host, &id, &outs, &mut [], now(20));
        // Something that is not a description never reaches a page.
        assert!(host.cast_offer(&id, "<script>alert(1)</script>").is_empty());
        assert!(ana.stream_answer("v=0\r\n\u{0}").is_empty());
        let forged = ana.seal(&RoomMsg::Answer { sdp: "javascript:1".into() }).unwrap();
        assert!(host.on_relay(msg(&id, &forged), now(30)).is_empty());
        // A request to play only counts in synced mode, and with control.
        host.set_control(&id, true);
        let request = ana.seal(&RoomMsg::Request { action: Request::Play, pos: 0.0 }).unwrap();
        assert!(host.on_relay(msg(&id, &request), now(40)).is_empty(), "sharing: the window is used instead");
        // Members that are not in the room cannot be given control.
        assert!(host.set_control("g9", true).is_empty());
        assert!(!host.may_control("g9"));
    }

    #[test]
    fn an_older_app_is_turned_away_with_a_notice() {
        let mut relay = Relay::new();
        let mut host = Host::new("", Mode::Share, now(0));
        relay.deliver(&host.on_relay(RelayIn::Ready { guests: vec![] }, now(0)), &mut [], now(0));
        host.on_relay(RelayIn::Joined { member: "g1".into(), resumed: false }, now(0));
        let eph = Ephemeral::generate();
        let body = serde_json::to_vec(&HelloBody { v: 1, name: "Ana".into() }).unwrap();
        let sealed = crypto::seal(&host.code().join_key(), &aad_hello("g1"), &body);
        let hello = to_value(&Envelope::Hello { eph: eph.public_b64(), sealed });
        let outs = host.on_relay(msg("g1", &hello), now(10));
        assert_eq!(relays(&outs), vec![json!({"op": "kick", "member": "g1"})]);
        assert!(outs.iter().any(|o| matches!(o, Out::Toast(t) if t.contains("Ana"))));
        assert_eq!(host.members(now(10)).len(), 1);
    }

    #[test]
    fn upload_is_shared_between_guests() {
        assert_eq!(bitrate_for(1), 4_000_000);
        for viewers in 1..=5u32 {
            assert!(bitrate_for(viewers as usize) * viewers <= 6_000_000, "{viewers} guests fit in a common upload");
        }
        assert!((1..5).all(|n| bitrate_for(n) > bitrate_for(n + 1)), "fewer guests, more each");
        assert_eq!(StreamState::from_report("connected"), Some(StreamState::Watching));
        assert_eq!(StreamState::from_report("failed"), Some(StreamState::Failed));
        assert_eq!(StreamState::from_report("bogus"), None);
    }

    #[test]
    fn names_and_times() {
        assert_eq!(format_time(61.9), "1:01");
        assert_eq!(format_time(3_725.0), "1:02:05");
        assert_eq!(display_name("g2", "Ana"), "Ana");
        assert!(display_name("g2", "").contains('2'));
    }
}
