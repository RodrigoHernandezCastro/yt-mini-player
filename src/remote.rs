//! A guest using the host's shared window: what its page sends (mouse and
//! keys, as fractions of the picture) becomes input for the host's page
//! (DevTools `Input.dispatchMouseEvent` / `Input.dispatchKeyEvent`).
//!
//! Everything that arrives is someone else's, so only what makes sense on a
//! web page gets through:
//! - only guests the host allowed (checked by the app before this), one at a
//!   time, at most `MAX_PER_SECOND` events each;
//! - the left button and the wheel (no right click: the menu would open on
//!   the host's screen; no middle click: new windows);
//! - never on the app's own side bar (right edge) or the edges that resize
//!   the window, nor while the host has the app's menu open;
//! - keys without Ctrl, Alt or the Windows key (AltGr characters are fine),
//!   no F keys, no browser or media keys;
//! - never on sign-in, account or payment pages, nor on My music (see
//!   [`page_allows_input`]).
//!
//! Whatever is held down is let go when control is taken away or the guest
//! leaves ([`Remote::release`]).

use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

use serde::Deserialize;
use serde_json::{json, Value};

/// CSS px from the right edge where the app's side bar lives (ui.js shows
/// it within 64 px).
pub const SIDE_ZONE: f64 = 72.0;
/// CSS px along every edge where a press would resize the window.
pub const EDGE_ZONE: f64 = 8.0;
pub const MAX_PER_SECOND: u32 = 240;
/// Another guest waits this long after the last one's input.
const TURN: Duration = Duration::from_millis(1500);
/// After a guest's press, the window does not start moving or resizing
/// (the page could ask for it in answer to that press).
const PRESS_SHADOW: Duration = Duration::from_millis(800);

/// What a guest's page sends.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "t")]
pub enum Input {
    #[serde(rename = "m")]
    Mouse {
        e: String,
        x: f64,
        y: f64,
        /// Click count (double click = 2).
        #[serde(default)]
        c: u32,
        #[serde(default)]
        dx: f64,
        #[serde(default)]
        dy: f64,
        /// Shift held.
        #[serde(default)]
        s: bool,
    },
    #[serde(rename = "k")]
    Key {
        e: String,
        key: String,
        #[serde(default)]
        code: String,
        /// Windows virtual-key code (`KeyboardEvent.keyCode`).
        #[serde(default)]
        kc: u32,
        #[serde(default)]
        s: bool,
        #[serde(default)]
        c: bool,
        #[serde(default)]
        a: bool,
        #[serde(default)]
        m: bool,
        #[serde(default)]
        r: bool,
    },
}

impl Input {
    pub fn parse(text: &str) -> Option<Self> {
        if text.len() > 512 {
            return None;
        }
        serde_json::from_str(text).ok()
    }
}

/// The host page's size in CSS px (from the pictures' metadata).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Viewport {
    pub width: f64,
    pub height: f64,
}

/// What became of a guest's event.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// DevTools calls to make: (method, params).
    Dispatch(Vec<(&'static str, Value)>),
    /// Someone else is using the window right now.
    Busy(String),
    /// Not allowed (or meaningless): nothing happens.
    Dropped,
}

#[derive(Debug, Default)]
struct Held {
    button: bool,
    last: Option<(f64, f64)>,
    /// (key, code, virtual key)
    keys: BTreeSet<(String, String, u32)>,
}

#[derive(Debug, Default)]
pub struct Remote {
    held: BTreeMap<String, Held>,
    turn: Option<(String, Instant)>,
    rates: BTreeMap<String, (Instant, u32)>,
    press_shadow: Option<Instant>,
}

/// Keys that never go through: F1–F24, the Windows and menu keys, sleep,
/// the browser, media and launch keys, and the modifiers themselves.
fn blocked_key(vk: u32) -> bool {
    matches!(vk, 16..=18 | 91..=93 | 95 | 112..=135 | 166..=183 | 225..=254) || vk > 254
}

/// Keys that type nothing but do something (Enter types "\r").
fn special_text(key: &str) -> Option<&'static str> {
    match key {
        "Enter" => Some("\r"),
        _ => None,
    }
}

/// The same keys by name (`key` or `code`), whatever virtual key comes
/// with them: the page sees the names, and an injected key is trusted, so
/// `{"key":"F11","kc":0}` would otherwise reach the app's own F11 and F1
/// shortcuts (ui.js) or a site's media keys. Shift alone stays: it does
/// nothing by itself (the modifier travels as a flag).
fn blocked_name(name: &str) -> bool {
    const PREFIXES: &[&str] = &[
        "Meta", "OS", "Super", "Hyper", "Control", "Alt", "ContextMenu", "Fn", "Browser", "Media", "Launch", "Audio",
        "Volume", "Power", "Sleep", "WakeUp", "Eject", "Help", "Print", "Props", "Select", "Open", "Find", "Again",
        "Undo", "Copy", "Paste", "Cut",
    ];
    let function_key = (2..=3).contains(&name.len()) && name.starts_with('F') && name[1..].bytes().all(|b| b.is_ascii_digit());
    function_key || PREFIXES.iter().any(|p| name.starts_with(p))
}

fn valid_code(code: &str) -> bool {
    code.len() <= 24 && code.bytes().all(|b| b.is_ascii_alphanumeric())
}

impl Remote {
    fn allow_rate(&mut self, member: &str, now: Instant) -> bool {
        let entry = self.rates.entry(member.to_owned()).or_insert((now, 0));
        if now.duration_since(entry.0) >= Duration::from_secs(1) {
            *entry = (now, 0);
        }
        entry.1 += 1;
        entry.1 <= MAX_PER_SECOND
    }

    /// Right after a guest's press (or key): what the page asks for now may
    /// be its answer to it (moving the window, opening the browser).
    pub fn pressing(&self, now: Instant) -> bool {
        self.held.values().any(|h| h.button) || self.press_shadow.is_some_and(|until| now < until)
    }

    /// Who is using the window now (input within the last moment).
    pub fn active(&self, now: Instant) -> Option<&str> {
        self.turn.as_ref().filter(|(_, at)| now.duration_since(*at) < TURN).map(|(m, _)| m.as_str())
    }

    /// A guest's event. `menu_open`: the host has the app's menu open.
    pub fn translate(&mut self, member: &str, input: &Input, viewport: Viewport, menu_open: bool, now: Instant) -> Outcome {
        if !self.allow_rate(member, now) {
            return Outcome::Dropped;
        }
        if let Some(other) = self.active(now).filter(|m| *m != member) {
            // Letting go is always fine (it was pressed before).
            let releasing = matches!(input, Input::Mouse { e, .. } | Input::Key { e, .. } if e == "up");
            if !releasing {
                return Outcome::Busy(other.to_owned());
            }
        }
        let calls = match input {
            Input::Mouse { e, x, y, c, dx, dy, s } => self.mouse(member, e, *x, *y, *c, *dx, *dy, *s, viewport, menu_open, now),
            Input::Key { e, key, code, kc, s, c, a, m, r } => {
                self.key(member, e, key, code, *kc, *s, (*c, *a, *m), *r, menu_open, now)
            }
        };
        match calls {
            Some(calls) if !calls.is_empty() => {
                self.turn = Some((member.to_owned(), now));
                Outcome::Dispatch(calls)
            }
            _ => Outcome::Dropped,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn mouse(
        &mut self,
        member: &str,
        kind: &str,
        x: f64,
        y: f64,
        clicks: u32,
        dx: f64,
        dy: f64,
        shift: bool,
        viewport: Viewport,
        menu_open: bool,
        now: Instant,
    ) -> Option<Vec<(&'static str, Value)>> {
        if !(x.is_finite() && y.is_finite() && (0.0..=1.0).contains(&x) && (0.0..=1.0).contains(&y)) {
            return None;
        }
        if !(viewport.width > 0.0 && viewport.height > 0.0) {
            return None;
        }
        let (px, py) = (x * viewport.width, y * viewport.height);
        let inside = !menu_open
            && px >= EDGE_ZONE
            && py >= EDGE_ZONE
            && py <= viewport.height - EDGE_ZONE
            && px <= viewport.width - SIDE_ZONE;
        let held = self.held.entry(member.to_owned()).or_default();
        let modifiers = if shift { 8 } else { 0 };
        let event = |kind: &str, px: f64, py: f64, extra: Value| {
            let mut params = json!({
                "type": kind, "x": (px * 10.0).round() / 10.0, "y": (py * 10.0).round() / 10.0,
                "modifiers": modifiers, "pointerType": "mouse",
            });
            if let (Some(params), Value::Object(extra)) = (params.as_object_mut(), extra) {
                params.extend(extra);
            }
            ("Input.dispatchMouseEvent", params)
        };
        match kind {
            "move" => {
                if !inside {
                    return None;
                }
                held.last = Some((px, py));
                let buttons = if held.button { 1 } else { 0 };
                Some(vec![event("mouseMoved", px, py, json!({ "button": if held.button { "left" } else { "none" }, "buttons": buttons }))])
            }
            "down" => {
                if !inside || held.button {
                    return None;
                }
                held.button = true;
                held.last = Some((px, py));
                self.press_shadow = Some(now + PRESS_SHADOW);
                let clicks = clicks.clamp(1, 3);
                Some(vec![event("mousePressed", px, py, json!({ "button": "left", "buttons": 1, "clickCount": clicks }))])
            }
            "up" => {
                if !held.button {
                    return None;
                }
                held.button = false;
                // Let go where it is, kept inside what may be touched.
                let (px, py) = if inside { (px, py) } else { held.last.unwrap_or((px, py)) };
                self.press_shadow = Some(now + PRESS_SHADOW);
                let clicks = clicks.clamp(1, 3);
                Some(vec![event("mouseReleased", px, py, json!({ "button": "left", "buttons": 0, "clickCount": clicks }))])
            }
            "wheel" => {
                if !inside || !dx.is_finite() || !dy.is_finite() {
                    return None;
                }
                let (dx, dy) = (dx.clamp(-2000.0, 2000.0), dy.clamp(-2000.0, 2000.0));
                Some(vec![event("mouseWheel", px, py, json!({ "button": "none", "deltaX": dx, "deltaY": dy }))])
            }
            _ => None,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn key(
        &mut self,
        member: &str,
        kind: &str,
        key: &str,
        code: &str,
        vk: u32,
        shift: bool,
        (ctrl, alt, meta): (bool, bool, bool),
        repeat: bool,
        menu_open: bool,
        now: Instant,
    ) -> Option<Vec<(&'static str, Value)>> {
        let chars = key.chars().count();
        if chars == 0 || chars > 24 || !valid_code(code) || blocked_key(vk) || blocked_name(code) {
            return None;
        }
        // A printable character: one, not a control character.
        let printable = chars == 1 && !key.chars().any(char::is_control);
        // Named keys ("Enter", "F11"…) are checked by name too.
        if !printable && blocked_name(key) {
            return None;
        }
        // AltGr (Ctrl+Alt on Windows) types characters like @ or € on many
        // keyboards: those go through as the character alone.
        let altgr = ctrl && alt && !meta && printable;
        if (ctrl || alt || meta) && !altgr {
            return None;
        }
        let held = self.held.entry(member.to_owned()).or_default();
        let entry = (key.to_owned(), code.to_owned(), vk);
        let modifiers = if shift && !altgr { 8 } else { 0 };
        match kind {
            "down" => {
                if menu_open {
                    return None;
                }
                held.keys.insert(entry);
                // Enter on a link may open a window, like a click.
                self.press_shadow = Some(now + PRESS_SHADOW);
                let text = if printable { Some(key) } else { special_text(key) };
                let mut params = json!({
                    "type": if text.is_some() { "keyDown" } else { "rawKeyDown" },
                    "key": key, "code": code, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk,
                    "modifiers": modifiers, "autoRepeat": repeat,
                });
                if let (Some(text), Some(map)) = (text, params.as_object_mut()) {
                    map.insert("text".into(), json!(text));
                    map.insert("unmodifiedText".into(), json!(text));
                }
                Some(vec![("Input.dispatchKeyEvent", params)])
            }
            "up" => {
                if !held.keys.remove(&entry) {
                    return None;
                }
                Some(vec![(
                    "Input.dispatchKeyEvent",
                    json!({ "type": "keyUp", "key": key, "code": code, "windowsVirtualKeyCode": vk,
                            "nativeVirtualKeyCode": vk, "modifiers": modifiers }),
                )])
            }
            _ => None,
        }
    }

    /// Lets go of whatever a guest holds (control taken away, it left).
    pub fn release(&mut self, member: &str) -> Vec<(&'static str, Value)> {
        let Some(held) = self.held.remove(member) else { return Vec::new() };
        let mut calls = Vec::new();
        if held.button {
            let (x, y) = held.last.unwrap_or((0.0, 0.0));
            calls.push((
                "Input.dispatchMouseEvent",
                json!({ "type": "mouseReleased", "x": x, "y": y, "button": "left", "buttons": 0, "clickCount": 1, "pointerType": "mouse" }),
            ));
        }
        for (key, code, vk) in held.keys {
            calls.push((
                "Input.dispatchKeyEvent",
                json!({ "type": "keyUp", "key": key, "code": code, "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk, "modifiers": 0 }),
            ));
        }
        if self.turn.as_ref().is_some_and(|(m, _)| m == member) {
            self.turn = None;
        }
        self.rates.remove(member);
        calls
    }

    /// Everyone lets go (sharing stopped).
    pub fn release_all(&mut self) -> Vec<(&'static str, Value)> {
        let members: Vec<String> = self.held.keys().cloned().collect();
        let calls = members.iter().flat_map(|m| self.release(m)).collect();
        self.turn = None;
        self.press_shadow = None;
        calls
    }
}

/// May guests use the host's page at this address? Not on sign-in,
/// account, settings or payment pages, not on the app's My music (the
/// host's own files), and only on https pages or the start screen.
pub fn page_allows_input(url: &str) -> bool {
    let Ok(url) = url::Url::parse(url) else { return false };
    if url.scheme() != "https" {
        return false;
    }
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    if host == crate::services::HOME_HOST {
        return url.path() == "/";
    }
    let path = url.path().to_ascii_lowercase();
    let risky = |text: &str| crate::together::protocol::RISKY.iter().any(|w| text.contains(w));
    let sign_in_host = ["accounts.", "login.", "signin.", "auth.", "id.", "account.", "pay.", "checkout."]
        .iter()
        .any(|p| host.starts_with(p));
    !risky(&path) && !sign_in_host && !host.starts_with("myaccount")
}

#[cfg(test)]
mod tests {
    use super::*;

    const VIEW: Viewport = Viewport { width: 1000.0, height: 500.0 };

    fn mouse(e: &str, x: f64, y: f64) -> Input {
        Input::Mouse { e: e.into(), x, y, c: 1, dx: 0.0, dy: 0.0, s: false }
    }

    fn key(e: &str, key: &str, code: &str, kc: u32) -> Input {
        held(e, key, code, kc, (false, false, false))
    }

    /// With Ctrl, Alt, Windows key held.
    fn held(e: &str, key: &str, code: &str, kc: u32, (c, a, m): (bool, bool, bool)) -> Input {
        Input::Key { e: e.into(), key: key.into(), code: code.into(), kc, s: false, c, a, m, r: false }
    }

    fn calls(outcome: Outcome) -> Vec<(&'static str, Value)> {
        match outcome {
            Outcome::Dispatch(calls) => calls,
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_click_lands_where_the_guest_clicked() {
        let mut remote = Remote::default();
        let now = Instant::now();
        let parsed = Input::parse(r#"{"t":"m","e":"down","x":0.25,"y":0.5,"c":1}"#).unwrap();
        let down = calls(remote.translate("g1", &parsed, VIEW, false, now));
        assert_eq!(down[0].0, "Input.dispatchMouseEvent");
        assert_eq!(down[0].1["type"], "mousePressed");
        assert_eq!((down[0].1["x"].as_f64(), down[0].1["y"].as_f64()), (Some(250.0), Some(250.0)));
        assert_eq!(down[0].1["button"], "left");
        assert!(remote.pressing(now), "the window does not move meanwhile");
        let moved = calls(remote.translate("g1", &mouse("move", 0.3, 0.5), VIEW, false, now));
        assert_eq!(moved[0].1["buttons"], 1, "dragging");
        let up = calls(remote.translate("g1", &mouse("up", 0.3, 0.5), VIEW, false, now));
        assert_eq!(up[0].1["type"], "mouseReleased");
        assert!(remote.pressing(now) && !remote.pressing(now + Duration::from_secs(1)));
        assert_eq!(remote.translate("g1", &mouse("up", 0.3, 0.5), VIEW, false, now), Outcome::Dropped, "not pressed");
        let wheel = Input::Mouse { e: "wheel".into(), x: 0.5, y: 0.5, c: 0, dx: 0.0, dy: 1e9, s: false };
        assert_eq!(calls(remote.translate("g1", &wheel, VIEW, false, now))[0].1["deltaY"], 2000.0);
    }

    #[test]
    fn the_side_bar_edges_and_menu_are_out_of_reach() {
        let mut remote = Remote::default();
        let now = Instant::now();
        for (x, y) in [(0.99, 0.5), (0.95, 0.5), (0.001, 0.5), (0.5, 0.001), (0.5, 0.999)] {
            assert_eq!(remote.translate("g1", &mouse("down", x, y), VIEW, false, now), Outcome::Dropped, "{x} {y}");
            assert_eq!(remote.translate("g1", &mouse("move", x, y), VIEW, false, now), Outcome::Dropped, "{x} {y}");
        }
        assert_eq!(remote.translate("g1", &mouse("down", 0.5, 0.5), VIEW, true, now), Outcome::Dropped, "menu open");
        for bad in [f64::NAN, -0.1, 1.5] {
            assert_eq!(remote.translate("g1", &mouse("down", bad, 0.5), VIEW, false, now), Outcome::Dropped);
        }
        assert_eq!(remote.translate("g1", &mouse("down", 0.5, 0.5), Viewport { width: 0.0, height: 0.0 }, false, now), Outcome::Dropped);
        // A press that leaves the area is let go inside it.
        calls(remote.translate("g1", &mouse("down", 0.5, 0.5), VIEW, false, now));
        let up = calls(remote.translate("g1", &mouse("up", 0.99, 0.5), VIEW, false, now));
        assert_eq!(up[0].1["x"], 500.0);
        // Other buttons and kinds do not exist.
        assert_eq!(Input::parse(r#"{"t":"m","e":"contextmenu","x":0.5,"y":0.5}"#).map(|i| remote.translate("g1", &i, VIEW, false, now)), Some(Outcome::Dropped));
        assert!(Input::parse(r#"{"t":"x"}"#).is_none());
        assert!(Input::parse(&format!(r#"{{"t":"k","e":"down","key":"{}"}}"#, "a".repeat(600))).is_none());
    }

    #[test]
    fn keys_type_but_shortcuts_stay_home() {
        let mut remote = Remote::default();
        let now = Instant::now();
        let typed = calls(remote.translate("g1", &key("down", "a", "KeyA", 65), VIEW, false, now));
        assert_eq!(typed[0].1["type"], "keyDown");
        assert_eq!(typed[0].1["text"], "a");
        assert!(remote.pressing(now), "a key, like a click, may make the page open something");
        let enter = calls(remote.translate("g1", &key("down", "Enter", "Enter", 13), VIEW, false, now));
        assert_eq!(enter[0].1["text"], "\r");
        let arrow = calls(remote.translate("g1", &key("down", "ArrowLeft", "ArrowLeft", 37), VIEW, false, now));
        assert_eq!(arrow[0].1["type"], "rawKeyDown");
        assert!(arrow[0].1.get("text").is_none());
        let up = calls(remote.translate("g1", &key("up", "a", "KeyA", 65), VIEW, false, now));
        assert_eq!(up[0].1["type"], "keyUp");
        // Ctrl+W, Alt+F4, Win+R, F5, F12, the browser's Back key...
        let ctrl_w = held("down", "w", "KeyW", 87, (true, false, false));
        let alt_f4 = held("down", "F4", "F4", 115, (false, true, false));
        let win_r = held("down", "r", "KeyR", 82, (false, false, true));
        for blocked in [ctrl_w, alt_f4, win_r, key("down", "F5", "F5", 116), key("down", "F12", "F12", 123), key("down", "BrowserBack", "BrowserBack", 166), key("down", "Meta", "MetaLeft", 91)] {
            assert_eq!(remote.translate("g1", &blocked, VIEW, false, now), Outcome::Dropped, "{blocked:?}");
        }
        // The names count too, whatever virtual key comes with them (the
        // page, and the app's own F11 / F1 shortcuts, see the names).
        for (k, code) in [
            ("F11", "F11"), ("F1", ""), ("a", "F5"), ("MediaPlayPause", ""), ("BrowserBack", ""), ("AudioVolumeMute", ""),
            ("Meta", ""), ("ContextMenu", "ContextMenu"), ("Control", "ControlLeft"), ("Alt", "AltLeft"), ("x", "MetaLeft"),
        ] {
            assert_eq!(remote.translate("g1", &key("down", k, code, 0), VIEW, false, now), Outcome::Dropped, "{k} {code}");
        }
        for (k, code, vk) in [("F", "KeyF", 70), ("Escape", "Escape", 27), ("Home", "Home", 36), ("Delete", "Delete", 46), ("Tab", "Tab", 9)] {
            assert!(matches!(remote.translate("g1", &key("down", k, code, vk), VIEW, false, now), Outcome::Dispatch(_)), "{k}");
            calls(remote.translate("g1", &key("up", k, code, vk), VIEW, false, now));
        }
        // AltGr+Q = @ on a Spanish or German keyboard.
        let at = held("down", "@", "KeyQ", 81, (true, true, false));
        let typed = calls(remote.translate("g1", &at, VIEW, false, now));
        assert_eq!((typed[0].1["text"].as_str(), typed[0].1["modifiers"].as_i64()), (Some("@"), Some(0)));
        assert_eq!(remote.translate("g1", &key("down", "b", "Key B", 66), VIEW, false, now), Outcome::Dropped, "odd code");
        assert_eq!(remote.translate("g1", &key("up", "z", "KeyZ", 90), VIEW, false, now), Outcome::Dropped, "never pressed");
    }

    #[test]
    fn one_at_a_time_and_not_too_fast() {
        let mut remote = Remote::default();
        let now = Instant::now();
        calls(remote.translate("g1", &mouse("move", 0.5, 0.5), VIEW, false, now));
        assert_eq!(remote.translate("g2", &mouse("down", 0.5, 0.5), VIEW, false, now), Outcome::Busy("g1".into()));
        assert_eq!(remote.active(now), Some("g1"));
        calls(remote.translate("g2", &mouse("move", 0.5, 0.5), VIEW, false, now + Duration::from_secs(2)));
        let later = now + Duration::from_secs(10);
        let accepted = (0..MAX_PER_SECOND + 50)
            .filter(|_| matches!(remote.translate("g3", &mouse("move", 0.4, 0.4), VIEW, false, later), Outcome::Dispatch(_)))
            .count();
        assert_eq!(accepted as u32, MAX_PER_SECOND);
    }

    #[test]
    fn whatever_is_held_is_let_go() {
        let mut remote = Remote::default();
        let now = Instant::now();
        calls(remote.translate("g1", &mouse("down", 0.5, 0.5), VIEW, false, now));
        calls(remote.translate("g1", &key("down", "Shift", "ShiftLeft", 0), VIEW, false, now));
        calls(remote.translate("g1", &key("down", "x", "KeyX", 88), VIEW, false, now));
        let released = remote.release("g1");
        let kinds: Vec<&str> = released.iter().map(|(_, p)| p["type"].as_str().unwrap()).collect();
        assert_eq!(kinds, vec!["mouseReleased", "keyUp", "keyUp"]);
        assert!(remote.release("g1").is_empty());
        assert_eq!(remote.active(now), None);
    }

    #[test]
    fn some_pages_are_off_limits() {
        for ok in [
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ",
            "https://www.youtube.com/",
            "https://www.twitch.tv/somechannel",
            "https://ytmp.localhost/",
        ] {
            assert!(page_allows_input(ok), "{ok}");
        }
        for off in [
            "https://accounts.google.com/signin",
            "https://www.youtube.com/account",
            "https://www.netflix.com/YourAccount",
            "https://www.youtube.com/paid_memberships?ybp=1",
            "https://www.primevideo.com/settings",
            "https://login.microsoftonline.com/",
            "https://ytmp.localhost/music",
            "http://example.com/",
            "chrome-extension://abc/options.html",
            "not a url",
        ] {
            assert!(!page_allows_input(off), "{off}");
        }
    }
}
