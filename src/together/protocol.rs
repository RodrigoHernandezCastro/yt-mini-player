//! Watch together: what travels between the apps and the relay.
//!
//! Three layers:
//! - **Relay operations** (`op`): plain JSON the relay understands (who is
//!   in the room, send this to that member, remove someone).
//! - **Envelopes** (`k`): what the apps put inside `data`. The relay cannot
//!   read them: `hello` and `welcome` carry the handshake (crypto.rs) and
//!   `room` carries a [`RoomMsg`] sealed with the room key.
//! - **Room messages** (`t`): the host's playback state, the member list,
//!   clock pings, the guests' requests and, when the host shares the
//!   window, the WebRTC handshake (offer and answer, with their ICE
//!   candidates inside).
//!
//! Everything that arrives is untrusted: bounded sizes, finite numbers,
//! pages that a guest may follow only if they are catalog services, and
//! connection descriptions and ICE servers checked before they reach a page.

use serde::{Deserialize, Serialize};
use url::Url;

/// Longest display name.
pub const MAX_NAME: usize = 24;
/// Largest message taken from the relay (it checks too).
pub const MAX_FRAME: usize = 64 * 1024;
/// The host and up to five guests (the relay holds the same limit).
pub const MAX_PEOPLE: usize = 6;
/// Longest connection description (SDP) passed on: the app keeps them
/// small (few codecs) so a sealed one fits in the 16 KiB older relays take.
pub const MAX_SDP: usize = 10_000;

// ---------------------------------------------------------------------------
// Relay operations
// ---------------------------------------------------------------------------

/// From the relay.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum RelayIn {
    /// To the host on connecting: the guests that are connected.
    Ready {
        #[serde(default)]
        guests: Vec<String>,
    },
    /// To a guest on connecting: its place in the room and the ticket to
    /// come back (to `room`) if the connection drops.
    You { member: String, ticket: String, room: String },
    /// To the host.
    Joined {
        member: String,
        #[serde(default)]
        resumed: bool,
    },
    /// To the host: `left` (said goodbye), `kicked`, `dropped` (connection
    /// lost; the seat is kept for a minute) or `gone` (did not come back).
    Left { member: String, reason: String },
    Msg { from: String, data: serde_json::Value },
    /// To the guests: the host's connection dropped (or came back).
    HostAway,
    HostBack,
    /// The room is over: `ended` (by the host), `host-gone`.
    Ended { reason: String },
    /// Answer to `advertise`.
    Advertised { lookup: String, ok: bool },
    /// Answer to `ice`: the STUN (and TURN, if the relay has it) servers
    /// for window sharing.
    Ice {
        #[serde(default)]
        servers: Vec<IceServer>,
    },
    Error { error: String },
}

pub fn parse_relay(text: &str) -> Option<RelayIn> {
    if text.len() > MAX_FRAME {
        return None;
    }
    serde_json::from_str(text).ok()
}

/// To the relay.
pub mod out {
    use serde_json::{json, Value};

    pub fn send_all(data: &Value) -> String {
        json!({ "op": "send", "to": "*", "data": data }).to_string()
    }

    pub fn send_to(member: &str, data: &Value) -> String {
        json!({ "op": "send", "to": member, "data": data }).to_string()
    }

    /// From a guest (always to the host).
    pub fn send(data: &Value) -> String {
        json!({ "op": "send", "data": data }).to_string()
    }

    pub fn kick(member: &str) -> String {
        json!({ "op": "kick", "member": member }).to_string()
    }

    /// The room can be found by this lookup for `ttl` seconds.
    pub fn advertise(lookup: &str, ttl: u64) -> String {
        json!({ "op": "advertise", "lookup": lookup, "ttl": ttl }).to_string()
    }

    pub fn withdraw(lookup: &str) -> String {
        json!({ "op": "withdraw", "lookup": lookup }).to_string()
    }

    pub fn end() -> String {
        json!({ "op": "end" }).to_string()
    }

    pub fn leave() -> String {
        json!({ "op": "leave" }).to_string()
    }

    /// The servers for window sharing (any member may ask).
    pub fn ice() -> String {
        json!({ "op": "ice" }).to_string()
    }
}

// ---------------------------------------------------------------------------
// Envelopes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "k", rename_all = "kebab-case")]
pub enum Envelope {
    /// Guest → host, sealed with the code's key.
    Hello { eph: String, #[serde(rename = "box")] sealed: String },
    /// Host → guest, sealed with the handshake key.
    Welcome { eph: String, #[serde(rename = "box")] sealed: String },
    /// Host → guest: the room key of epoch `e` (someone was removed, or
    /// the guest came back after a dropped connection).
    Rekey { e: u32, #[serde(rename = "box")] sealed: String },
    /// Anyone, sealed with the room key of epoch `e`.
    Room { e: u32, #[serde(rename = "box")] sealed: String },
}

/// Inside `hello`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HelloBody {
    pub v: u32,
    #[serde(default)]
    pub name: String,
}

/// Inside `welcome` and `rekey`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct KeyBody {
    pub room: String,
    pub epoch: u32,
    #[serde(default)]
    pub host: String,
}

pub fn aad_hello(member: &str) -> String {
    format!("hello|{member}")
}

pub fn aad_welcome(member: &str) -> String {
    format!("welcome|{member}")
}

pub fn aad_rekey(member: &str, epoch: u32) -> String {
    format!("rekey|{member}|{epoch}")
}

pub fn aad_room(from: &str, epoch: u32) -> String {
    format!("room|{from}|{epoch}")
}

// ---------------------------------------------------------------------------
// Room messages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sealed {
    /// Per-sender sequence number: a message seen twice is dropped.
    pub s: u64,
    #[serde(flatten)]
    pub msg: RoomMsg,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "kebab-case")]
pub enum RoomMsg {
    State(HostState),
    Members {
        list: Vec<Member>,
        /// 0.9: everyone may pause and seek. Now per member (`Member`).
        #[serde(default)]
        control: bool,
        #[serde(default)]
        mode: Mode,
        /// Sharing, but the picture is stopped (the host's app is locked).
        #[serde(default)]
        paused: bool,
    },
    /// Guest → host: what time is it for you? (`t0`: guest clock, ms).
    Ping { t0: u64 },
    /// Host → guest (`t1`: host clock, ms).
    Pong { t0: u64, t1: u64 },
    /// Guest → host: the guest played, paused or moved the video.
    Request { action: Request, #[serde(default)] pos: f64 },
    /// Guest → host: send me the window (again: a new connection).
    Watch,
    /// Guest → host: I stopped watching it.
    Unwatch,
    /// Host → guest: the connection for the window (WebRTC offer).
    Offer { sdp: String },
    /// Guest → host: its answer.
    Answer { sdp: String },
    /// Guest → host: may I use the window?
    Ask,
}

/// How the room watches.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum Mode {
    /// Each one plays in their own app (with their own account); the
    /// players follow the host's.
    #[default]
    Sync,
    /// The host's window (picture and sound) goes to everyone.
    Share,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Request {
    Play,
    Pause,
    Seek,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Member {
    pub id: String,
    pub name: String,
    /// May pause and seek (sync) or use the window (share).
    #[serde(default)]
    pub control: bool,
}

/// A STUN or TURN server, as WebRTC takes it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IceServer {
    #[serde(deserialize_with = "one_or_many")]
    pub urls: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
}

fn one_or_many<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    #[derive(Deserialize)]
    #[serde(untagged)]
    enum OneOrMany {
        One(String),
        Many(Vec<String>),
    }
    Ok(match OneOrMany::deserialize(d)? {
        OneOrMany::One(url) => vec![url],
        OneOrMany::Many(urls) => urls,
    })
}

/// Public STUN servers, when the relay gives none.
pub fn default_ice_servers() -> Vec<IceServer> {
    vec![IceServer {
        urls: vec!["stun:stun.cloudflare.com:3478".into(), "stun:stun.l.google.com:19302".into()],
        username: None,
        credential: None,
    }]
}

/// Only `stun:`, `turn:` and `turns:` addresses, bounded, without port 53
/// (browsers give up on it slowly); a TURN server needs its credentials.
pub fn sanitize_ice_servers(servers: Vec<IceServer>) -> Vec<IceServer> {
    let printable = |s: &str, max: usize| s.len() <= max && s.bytes().all(|b| (0x21..0x7f).contains(&b));
    servers
        .into_iter()
        .filter_map(|server| {
            let urls: Vec<String> = server
                .urls
                .into_iter()
                .filter(|u| {
                    printable(u, 256)
                        && ["stun:", "turn:", "turns:"].iter().any(|p| u.starts_with(p))
                        && !u.split('?').next().unwrap_or_default().ends_with(":53")
                })
                .take(8)
                .collect();
            let username = server.username.filter(|u| printable(u, 512));
            let credential = server.credential.filter(|c| printable(c, 512));
            let turn = urls.iter().any(|u| u.starts_with("turn"));
            if urls.is_empty() || (turn && (username.is_none() || credential.is_none())) {
                return None;
            }
            Some(IceServer { urls, username, credential })
        })
        .take(4)
        .collect()
}

/// A connection description that may reach a page: an SDP of printable
/// ASCII lines, not too long.
pub fn valid_sdp(sdp: &str) -> bool {
    sdp.len() <= MAX_SDP
        && sdp.starts_with("v=0")
        && sdp.bytes().all(|b| (0x20..0x7f).contains(&b) || b == b'\r' || b == b'\n')
}

/// What the host is watching, and where it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct HostState {
    /// Host clock (ms) when `pos` was measured.
    pub at: u64,
    pub page: Page,
    pub title: String,
    pub site: String,
    pub media: bool,
    pub playing: bool,
    /// Seconds.
    pub pos: f64,
    pub dur: f64,
    pub live: bool,
    pub rate: f64,
    /// An advert is playing on the host's side: the others wait.
    pub ad: bool,
    /// Who made the last change, if it was a guest.
    pub by: Option<String>,
}

impl HostState {
    /// Clean numbers and texts (it comes from someone else's app).
    pub fn sanitized(mut self) -> Self {
        let secs = |n: f64| if n.is_finite() { n.clamp(0.0, 1e7) } else { 0.0 };
        self.pos = secs(self.pos);
        self.dur = secs(self.dur);
        if self.dur > 0.0 {
            self.pos = self.pos.min(self.dur);
        }
        self.rate = if self.rate.is_finite() && (0.25..=4.0).contains(&self.rate) { self.rate } else { 1.0 };
        self.title = clean(&self.title, 200);
        self.site = clean(&self.site, 253);
        if self.by.as_deref().is_some_and(|b| !is_member_id(b)) {
            self.by = None;
        }
        self.page = self.page.sanitized();
        self
    }

    /// Where the host is now, in seconds, `elapsed_ms` after `at`.
    pub fn position_after(&self, elapsed_ms: i64) -> f64 {
        let mut pos = self.pos;
        if self.playing && !self.ad && !self.live {
            pos += elapsed_ms.max(0) as f64 / 1000.0 * self.rate;
        }
        if self.dur > 0.0 {
            pos = pos.min(self.dur);
        }
        pos
    }
}

/// The host's page, as much as is safe to share.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Page {
    #[default]
    None,
    /// The start screen.
    Home,
    /// My music: the host's own files; nobody can follow.
    Music,
    /// A catalog service: guests open the same address.
    Service { url: String },
    /// Another site: guests may open it by hand.
    Site { url: String },
}

impl Page {
    fn sanitized(self) -> Self {
        match self {
            Page::Service { url } => match follow_url(&url) {
                Some(url) => Page::Service { url: url.into() },
                None => Page::None,
            },
            Page::Site { url } => match Url::parse(&url) {
                Ok(u) if u.scheme() == "https" && u.host_str().is_some() && url.len() <= 2048 => Page::Site { url: u.into() },
                _ => Page::None,
            },
            other => other,
        }
    }

    /// Same video or page (not just the same address: YouTube adds `t`,
    /// `pp`, `si`...).
    pub fn identity(&self) -> Option<String> {
        match self {
            Page::Service { url } | Page::Site { url } => Url::parse(url).ok().map(|u| identity(&u)),
            Page::Home => Some("home".into()),
            Page::Music => Some("music".into()),
            Page::None => None,
        }
    }
}

fn is_youtube(host: &str) -> bool {
    matches!(host, "www.youtube.com" | "youtube.com" | "m.youtube.com" | "music.youtube.com" | "youtu.be")
}

/// What identifies a page: the video on YouTube, origin and path elsewhere.
pub fn identity(url: &Url) -> String {
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if is_youtube(&host) {
        if let Some(id) = youtube_video(url) {
            return format!("yt:{id}");
        }
        if let Some((_, list)) = url.query_pairs().find(|(k, _)| k == "list") {
            return format!("yt-list:{list}");
        }
    }
    format!("{host}{}", url.path().trim_end_matches('/'))
}

fn youtube_video(url: &Url) -> Option<String> {
    let host = url.host_str()?.to_ascii_lowercase();
    let valid = |id: &str| id.len() == 11 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    let id = if host == "youtu.be" {
        url.path().trim_start_matches('/').to_owned()
    } else if url.path() == "/watch" {
        url.query_pairs().find(|(k, _)| k == "v").map(|(_, v)| v.into_owned())?
    } else {
        let mut parts = url.path_segments()?;
        match (parts.next(), parts.next()) {
            (Some("shorts" | "live" | "embed"), Some(id)) => id.to_owned(),
            _ => return None,
        }
    };
    valid(&id).then_some(id)
}

/// The host's page as it is shared (host side). Query strings and
/// fragments may carry session data, so only what identifies the content is
/// kept.
pub fn page_for(url: &str) -> Page {
    let Ok(url) = Url::parse(url) else { return Page::None };
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if host == crate::services::HOME_HOST {
        return if url.path() == "/music" { Page::Music } else { Page::Home };
    }
    if url.scheme() != "https" {
        return Page::None;
    }
    if is_youtube(&host) {
        let target = if host == "music.youtube.com" { "music.youtube.com" } else { "www.youtube.com" };
        if let Some(id) = youtube_video(&url) {
            let mut out = format!("https://{target}/watch?v={id}");
            if let Some((_, list)) = url.query_pairs().find(|(k, _)| k == "list") {
                if list.len() <= 64 && list.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
                    out.push_str(&format!("&list={list}"));
                }
            }
            return Page::Service { url: out };
        }
    }
    let mut bare = url.clone();
    bare.set_query(None);
    bare.set_fragment(None);
    if crate::services::for_url(&bare).is_some() {
        match follow_url(bare.as_str()) {
            Some(safe) => Page::Service { url: safe.into() },
            // An account or sign-out page: nobody is sent there.
            None => Page::Site { url: format!("https://{host}/") },
        }
    } else {
        Page::Site { url: bare.into() }
    }
}

/// Words that never appear in an address a guest is sent to: a host cannot
/// sign the others out or open their account pages. Guests using the host's
/// window cannot touch such pages either (remote.rs).
pub const RISKY: &[&str] = &[
    "logout", "log-out", "signout", "sign-out", "login", "signin", "sign-in", "signup", "sign-up", "account",
    "settings", "billing", "payment", "subscription", "membership", "purchase", "checkout", "wallet", "password",
    "delete", "profile", "manage", "redeem", "gift",
];

/// The address a guest may open by itself: https, a catalog service, no
/// query except YouTube's `v` and `list`, and no account pages.
pub fn follow_url(text: &str) -> Option<Url> {
    if text.len() > 2048 {
        return None;
    }
    let url = Url::parse(text).ok()?;
    if url.scheme() != "https" || url.port().is_some() || !url.username().is_empty() || url.password().is_some() {
        return None;
    }
    crate::services::for_url(&url)?;
    let path = url.path().to_ascii_lowercase();
    if RISKY.iter().any(|w| path.contains(w)) || url.fragment().is_some() {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    if is_youtube(&host) {
        if url.path() != "/watch" || youtube_video(&url).is_none() {
            return None;
        }
        if url.query_pairs().any(|(k, _)| k != "v" && k != "list") {
            return None;
        }
    } else if url.query().is_some() {
        return None;
    }
    Some(url)
}

/// A member id the relay gives out: `host` or `g1`, `g2`…
pub fn is_member_id(id: &str) -> bool {
    id == "host" || (id.len() <= 8 && id.starts_with('g') && id.len() > 1 && id[1..].bytes().all(|b| b.is_ascii_digit()))
}

/// A display name: no control characters, collapsed spaces, at most
/// [`MAX_NAME`] characters.
pub fn clean_name(name: &str) -> String {
    clean(name, MAX_NAME)
}

fn clean(text: &str, max: usize) -> String {
    let collapsed = text
        .chars()
        .map(|c| if c.is_control() || matches!(c, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}') { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    collapsed.chars().take(max).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relay_operations_parse() {
        assert_eq!(parse_relay(r#"{"op":"ready","guests":["g1"]}"#), Some(RelayIn::Ready { guests: vec!["g1".into()] }));
        assert_eq!(
            parse_relay(r#"{"op":"you","member":"g2","ticket":"ab","room":"cd"}"#),
            Some(RelayIn::You { member: "g2".into(), ticket: "ab".into(), room: "cd".into() })
        );
        assert_eq!(parse_relay(r#"{"op":"host-away"}"#), Some(RelayIn::HostAway));
        assert_eq!(
            parse_relay(r#"{"op":"joined","member":"g1"}"#),
            Some(RelayIn::Joined { member: "g1".into(), resumed: false })
        );
        assert!(matches!(parse_relay(r#"{"op":"msg","from":"host","data":{"k":"room"}}"#), Some(RelayIn::Msg { .. })));
        assert_eq!(parse_relay(r#"{"op":"nope"}"#), None);
        assert_eq!(parse_relay(&format!(r#"{{"op":"error","error":"{}"}}"#, "x".repeat(MAX_FRAME))), None);
    }

    #[test]
    fn envelopes_round_trip() {
        let hello = Envelope::Hello { eph: "AAA".into(), sealed: "BBB".into() };
        let json = serde_json::to_value(&hello).unwrap();
        assert_eq!(json, serde_json::json!({"k": "hello", "eph": "AAA", "box": "BBB"}));
        assert_eq!(serde_json::from_value::<Envelope>(json).unwrap(), hello);
        let sealed = Sealed { s: 7, msg: RoomMsg::Request { action: Request::Seek, pos: 61.5 } };
        let json = serde_json::to_string(&sealed).unwrap();
        assert_eq!(json, r#"{"s":7,"t":"request","action":"seek","pos":61.5}"#);
        assert_eq!(serde_json::from_str::<Sealed>(&json).unwrap(), sealed);
        let state = Sealed { s: 1, msg: RoomMsg::State(HostState { page: Page::Home, ..Default::default() }) };
        let back: Sealed = serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert_eq!(back, state);
    }

    #[test]
    fn host_pages_keep_only_what_identifies_the_content() {
        assert_eq!(
            page_for("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=42s&pp=xyz&list=PL1234_-ab"),
            Page::Service { url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PL1234_-ab".into() }
        );
        assert_eq!(page_for("https://youtu.be/dQw4w9WgXcQ?si=abc"), Page::Service { url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".into() });
        assert_eq!(page_for("https://www.youtube.com/shorts/dQw4w9WgXcQ"), Page::Service { url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".into() });
        assert_eq!(
            page_for("https://music.youtube.com/watch?v=dQw4w9WgXcQ&feature=share"),
            Page::Service { url: "https://music.youtube.com/watch?v=dQw4w9WgXcQ".into() }
        );
        assert_eq!(
            page_for("https://www.netflix.com/watch/81234567?trackId=1&tctx=secret"),
            Page::Service { url: "https://www.netflix.com/watch/81234567".into() }
        );
        assert_eq!(page_for("https://www.netflix.com/YourAccount"), Page::Site { url: "https://www.netflix.com/".into() });
        assert_eq!(page_for("https://ytmp.localhost/"), Page::Home);
        assert_eq!(page_for("https://ytmp.localhost/music"), Page::Music);
        assert_eq!(page_for("https://example.com/a/b?token=1#x"), Page::Site { url: "https://example.com/a/b".into() });
        assert_eq!(page_for("http://example.com/"), Page::None);
        assert_eq!(page_for("not a url"), Page::None);
    }

    #[test]
    fn guests_only_follow_safe_catalog_addresses() {
        for ok in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&list=PL1",
            "https://www.netflix.com/watch/81234567",
            "https://www.twitch.tv/somechannel",
            "https://www.crunchyroll.com/watch/GXXXX/slug",
        ] {
            assert!(follow_url(ok).is_some(), "{ok}");
        }
        for bad in [
            "http://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com/logout",
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ&action=x",
            "https://www.youtube.com/watch?v=short",
            "https://www.netflix.com/SignOut",
            "https://www.netflix.com/YourAccount",
            "https://www.primevideo.com/settings",
            "https://www.netflix.com/watch/1?x=1",
            "https://www.netflix.com/watch/1#frag",
            "https://user@www.netflix.com/watch/1",
            "https://www.netflix.com:8443/watch/1",
            "https://evil.example/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com.evil.example/watch?v=dQw4w9WgXcQ",
            "javascript:alert(1)",
            "https://ytmp.localhost/",
        ] {
            assert!(follow_url(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn identities_ignore_noise() {
        let a = Page::Service { url: "https://www.youtube.com/watch?v=dQw4w9WgXcQ".into() };
        let b = Url::parse("https://www.youtube.com/watch?v=dQw4w9WgXcQ&t=10&pp=1").unwrap();
        assert_eq!(a.identity(), Some(identity(&b)));
        let music = Url::parse("https://music.youtube.com/watch?v=dQw4w9WgXcQ").unwrap();
        assert_eq!(identity(&music), identity(&b), "the same video on YouTube Music");
        let n1 = Url::parse("https://www.netflix.com/watch/81234567?trackId=1").unwrap();
        let n2 = Url::parse("https://www.netflix.com/watch/81234567/").unwrap();
        assert_eq!(identity(&n1), identity(&n2));
        assert_ne!(identity(&n1), identity(&Url::parse("https://www.netflix.com/watch/7").unwrap()));
    }

    #[test]
    fn foreign_state_is_cleaned() {
        let state = HostState {
            pos: f64::NAN,
            dur: 100.0,
            rate: 50.0,
            title: "a\u{202e}b\nc".into(),
            by: Some("<script>".into()),
            page: Page::Service { url: "https://www.youtube.com/logout".into() },
            ..Default::default()
        }
        .sanitized();
        assert_eq!((state.pos, state.rate), (0.0, 1.0));
        assert_eq!(state.title, "a b c");
        assert_eq!(state.by, None);
        assert_eq!(state.page, Page::None);
        let playing = HostState { pos: 10.0, dur: 11.0, playing: true, rate: 1.0, ..Default::default() };
        assert_eq!(playing.position_after(500), 10.5);
        assert_eq!(playing.position_after(5000), 11.0, "never past the end");
        assert_eq!(HostState { ad: true, ..playing.clone() }.position_after(5000), 10.0);
        assert_eq!(HostState { playing: false, ..playing }.position_after(5000), 10.0);
    }

    #[test]
    fn share_messages_round_trip_and_old_ones_still_parse() {
        let members = Sealed {
            s: 3,
            msg: RoomMsg::Members {
                list: vec![Member { id: "g1".into(), name: "Ana".into(), control: true }],
                control: false,
                mode: Mode::Share,
                paused: false,
            },
        };
        let json = serde_json::to_string(&members).unwrap();
        assert!(json.contains(r#""mode":"share""#), "{json}");
        assert_eq!(serde_json::from_str::<Sealed>(&json).unwrap(), members);
        // From a 0.9 host: no mode, no per-member control.
        let old: Sealed = serde_json::from_str(r#"{"s":1,"t":"members","list":[{"id":"host","name":"R"}],"control":true}"#).unwrap();
        assert_eq!(
            old.msg,
            RoomMsg::Members { list: vec![Member { id: "host".into(), name: "R".into(), control: false }], control: true, mode: Mode::Sync, paused: false }
        );
        for (msg, text) in [
            (RoomMsg::Watch, r#"{"s":1,"t":"watch"}"#),
            (RoomMsg::Unwatch, r#"{"s":1,"t":"unwatch"}"#),
            (RoomMsg::Ask, r#"{"s":1,"t":"ask"}"#),
            (RoomMsg::Offer { sdp: "v=0".into() }, r#"{"s":1,"t":"offer","sdp":"v=0"}"#),
            (RoomMsg::Answer { sdp: "v=0".into() }, r#"{"s":1,"t":"answer","sdp":"v=0"}"#),
        ] {
            assert_eq!(serde_json::to_string(&Sealed { s: 1, msg: msg.clone() }).unwrap(), text);
            assert_eq!(serde_json::from_str::<Sealed>(text).unwrap().msg, msg);
        }
    }

    #[test]
    fn ice_servers_from_the_relay_are_checked() {
        let parsed = parse_relay(
            r#"{"op":"ice","servers":[{"urls":"stun:stun.cloudflare.com:3478"},
               {"urls":["turn:turn.cloudflare.com:3478?transport=udp","turn:turn.cloudflare.com:53?transport=udp",
                        "turns:turn.cloudflare.com:443?transport=tcp","https://evil.example/"],
                "username":"u","credential":"c"},
               {"urls":["turn:nocreds.example:3478"]},
               {"urls":["javascript:alert(1)"]}]}"#,
        );
        let Some(RelayIn::Ice { servers }) = parsed else { panic!("{parsed:?}") };
        let clean = sanitize_ice_servers(servers);
        assert_eq!(clean.len(), 2, "{clean:?}");
        assert_eq!(clean[0].urls, vec!["stun:stun.cloudflare.com:3478"]);
        assert_eq!(
            clean[1].urls,
            vec!["turn:turn.cloudflare.com:3478?transport=udp", "turns:turn.cloudflare.com:443?transport=tcp"],
            "no port 53, nothing but stun/turn"
        );
        assert_eq!(clean[1].username.as_deref(), Some("u"));
        assert_eq!(parse_relay(r#"{"op":"ice"}"#), Some(RelayIn::Ice { servers: vec![] }));
        assert_eq!(sanitize_ice_servers(default_ice_servers()), default_ice_servers());
        // Serialized as RTCPeerConnection takes it.
        let json = serde_json::to_value(default_ice_servers()).unwrap();
        assert!(json[0]["urls"].is_array() && json[0].get("username").is_none());
    }

    #[test]
    fn descriptions_are_plain_sdp() {
        assert!(valid_sdp("v=0\r\no=- 1 2 IN IP4 127.0.0.1\r\ns=-\r\n"));
        assert!(!valid_sdp("<script>"));
        assert!(!valid_sdp("v=0\r\na=x\u{0}"));
        assert!(!valid_sdp("v=0\r\na=é"));
        assert!(!valid_sdp(&format!("v=0\r\n{}", "a".repeat(MAX_SDP))));
    }

    #[test]
    fn names_and_members() {
        assert_eq!(clean_name("  Ana\t\u{0}María  "), "Ana María");
        assert_eq!(clean_name(&"x".repeat(40)).chars().count(), MAX_NAME);
        assert!(is_member_id("host") && is_member_id("g1") && is_member_id("g12"));
        for bad in ["g", "gx", "G1", "guest", "", "g123456789"] {
            assert!(!is_member_id(bad), "{bad}");
        }
    }
}

#[cfg(test)]
#[path = "../tests/protocol_cases.rs"]
mod cases;
