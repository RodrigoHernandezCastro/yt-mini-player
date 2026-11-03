//! Global shortcuts: they work even when the window has no focus (or is
//! hidden in the tray). All use Ctrl+Alt so they never clash with YouTube.
//!
//! On many keyboard layouts AltGr *is* Ctrl+Alt and types characters:
//! Polish uses AltGr+L for "ł" and AltGr+N for "ń", Spanish layouts use it
//! for "@", "#" or "€". A global Ctrl+Alt+L would swallow every "ł" the user
//! types in any program. So before registering a key, the app asks every
//! keyboard layout the user has installed what AltGr+key types; if any of
//! them types a character, that key is skipped and the next candidate (if
//! the action has one) is used instead: Ctrl+Alt+. for "next" and
//! Ctrl+Alt+K for "lock" on a Polish keyboard. Ctrl+Alt+arrows are used by
//! some old Intel drivers to rotate the screen: if Windows refuses them, the
//! help shows them as unavailable.

use serde::Serialize;

use crate::ipc::{Command, MediaAction};

#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Y,
    P,
    N,
    B,
    G,
    L,
    K,
    M,
    Period,
    PageUp,
    PageDown,
    Digit0,
    Up,
    Down,
    Left,
    Right,
}

impl Key {
    /// Windows virtual-key code.
    #[cfg_attr(not(windows), allow(dead_code))]
    fn vk(self) -> u16 {
        match self {
            Key::Y => 0x59,
            Key::P => 0x50,
            Key::N => 0x4E,
            Key::B => 0x42,
            Key::G => 0x47,
            Key::L => 0x4C,
            Key::K => 0x4B,
            Key::M => 0x4D,
            Key::Period => 0xBE,
            Key::PageUp => 0x21,
            Key::PageDown => 0x22,
            Key::Digit0 => 0x30,
            Key::Up => 0x26,
            Key::Down => 0x28,
            Key::Left => 0x25,
            Key::Right => 0x27,
        }
    }

    /// Keys that can type a character (the others never clash with AltGr).
    #[cfg_attr(not(windows), allow(dead_code))]
    fn types_text(self) -> bool {
        !matches!(self, Key::PageUp | Key::PageDown | Key::Up | Key::Down | Key::Left | Key::Right)
    }

    /// How the key is written on keyboards in the current language.
    fn label(self) -> String {
        match self {
            Key::Y => "Y".into(),
            Key::P => "P".into(),
            Key::N => "N".into(),
            Key::B => "B".into(),
            Key::G => "G".into(),
            Key::L => "L".into(),
            Key::K => "K".into(),
            Key::M => "M".into(),
            Key::Period => ".".into(),
            Key::PageUp => t!("key.page_up"),
            Key::PageDown => t!("key.page_down"),
            Key::Digit0 => "0".into(),
            Key::Up => "↑".into(),
            Key::Down => "↓".into(),
            Key::Left => "←".into(),
            Key::Right => "→".into(),
        }
    }

    fn combo(self) -> String {
        format!("Ctrl+Alt+{}", self.label())
    }
}

/// What a shortcut does (also how the tray menu finds its shortcut text).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    ToggleVisible,
    PlayPause,
    Next,
    Previous,
    SeekForward,
    SeekBack,
    VolumeUp,
    VolumeDown,
    Mute,
    Ghost,
    OpacityUp,
    OpacityDown,
    OpacityReset,
    Lock,
}

struct Definition {
    action: Action,
    /// i18n key of the description in the help.
    label: &'static str,
    /// Keys to try, in order of preference.
    keys: &'static [Key],
    command: fn() -> Command,
}

const DEFINITIONS: &[Definition] = &[
    Definition { action: Action::ToggleVisible, label: "hotkey.toggle_visible", keys: &[Key::Y], command: || Command::ToggleVisible },
    Definition { action: Action::PlayPause, label: "hotkey.play_pause", keys: &[Key::P], command: || Command::Media(MediaAction::PlayPause) },
    Definition { action: Action::Next, label: "hotkey.next", keys: &[Key::N, Key::Period], command: || Command::Media(MediaAction::Next) },
    Definition { action: Action::Previous, label: "hotkey.previous", keys: &[Key::B], command: || Command::Media(MediaAction::Previous) },
    Definition { action: Action::SeekForward, label: "hotkey.seek_forward", keys: &[Key::Right], command: || Command::Media(MediaAction::SeekBy(10)) },
    Definition { action: Action::SeekBack, label: "hotkey.seek_back", keys: &[Key::Left], command: || Command::Media(MediaAction::SeekBy(-10)) },
    Definition { action: Action::VolumeUp, label: "hotkey.volume_up", keys: &[Key::Up], command: || Command::Media(MediaAction::VolumeBy(10)) },
    Definition { action: Action::VolumeDown, label: "hotkey.volume_down", keys: &[Key::Down], command: || Command::Media(MediaAction::VolumeBy(-10)) },
    Definition { action: Action::Mute, label: "hotkey.mute", keys: &[Key::M], command: || Command::Media(MediaAction::ToggleMute) },
    Definition { action: Action::Ghost, label: "hotkey.ghost", keys: &[Key::G], command: || Command::ToggleGhost },
    Definition { action: Action::OpacityUp, label: "hotkey.opacity_up", keys: &[Key::PageUp], command: || Command::AdjustOpacity(10) },
    Definition { action: Action::OpacityDown, label: "hotkey.opacity_down", keys: &[Key::PageDown], command: || Command::AdjustOpacity(-10) },
    Definition { action: Action::OpacityReset, label: "hotkey.opacity_reset", keys: &[Key::Digit0], command: || Command::ResetOpacity },
    Definition { action: Action::Lock, label: "hotkey.lock", keys: &[Key::L, Key::K], command: || Command::LockNow },
];

/// Why a shortcut could not be registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ready,
    /// Windows refused it: another program already uses it.
    InUse,
    /// A keyboard layout of the user types a character with it (AltGr).
    TypesText,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shortcut {
    action: Action,
    label: &'static str,
    /// The key in use, or the preferred one if none could be used.
    key: Key,
    status: Status,
}

/// A shortcut for the help in the page, in the current language.
#[derive(Debug, Clone, Serialize)]
pub struct ShortcutView {
    pub keys: String,
    pub label: String,
    pub available: bool,
    /// Why it is unavailable.
    pub note: Option<String>,
}

pub struct Hotkeys {
    #[cfg(windows)]
    _manager: Option<global_hotkey::GlobalHotKeyManager>,
    shortcuts: Vec<Shortcut>,
}

impl Hotkeys {
    /// The help table (built on demand, so it follows language changes).
    pub fn views(&self) -> Vec<ShortcutView> {
        self.shortcuts
            .iter()
            .map(|s| ShortcutView {
                keys: s.key.combo(),
                label: crate::i18n::text(s.label),
                available: s.status == Status::Ready,
                note: match s.status {
                    Status::Ready => None,
                    Status::InUse => Some(t!("hotkey.in_use")),
                    Status::TypesText => Some(t!("hotkey.types_text")),
                },
            })
            .collect()
    }

    /// The shortcut of an action, if it works (for the tray menu).
    pub fn keys_for(&self, action: Action) -> Option<String> {
        self.shortcuts.iter().find(|s| s.action == action && s.status == Status::Ready).map(|s| s.key.combo())
    }
}

/// Picks the first candidate that no layout types with and that Windows
/// accepts. Pure, so it can be tested without a keyboard.
fn choose(keys: &[Key], types_text: impl Fn(Key) -> bool, mut register: impl FnMut(Key) -> bool) -> (Key, Status) {
    let mut status = Status::InUse;
    for &key in keys {
        if types_text(key) {
            if status == Status::InUse {
                status = Status::TypesText;
            }
            continue;
        }
        if register(key) {
            return (key, Status::Ready);
        }
        status = Status::InUse;
    }
    (keys[0], status)
}

/// Does AltGr (= Ctrl+Alt) plus this key type something on any installed
/// keyboard layout?
#[cfg(windows)]
fn altgr_types(key: Key, layouts: &[windows_sys::Win32::UI::Input::KeyboardAndMouse::HKL]) -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        MapVirtualKeyExW, ToUnicodeEx, MAPVK_VK_TO_VSC, VK_CONTROL, VK_LCONTROL, VK_MENU, VK_RMENU,
    };
    if !key.types_text() {
        return false;
    }
    let mut state = [0u8; 256];
    for vk in [VK_CONTROL, VK_LCONTROL, VK_MENU, VK_RMENU] {
        state[usize::from(vk)] = 0x80;
    }
    layouts.iter().any(|&layout| unsafe {
        let scan = MapVirtualKeyExW(u32::from(key.vk()), MAPVK_VK_TO_VSC, layout);
        let mut out = [0u16; 8];
        // Flag 4: do not change the keyboard state (no dead key left behind).
        let n = ToUnicodeEx(u32::from(key.vk()), scan, state.as_ptr(), out.as_mut_ptr(), out.len() as i32, 4, layout);
        // A dead key (n < 0) or any printable character.
        n < 0 || out[..n.max(0) as usize].iter().any(|&c| c >= 0x20)
    })
}

#[cfg(windows)]
fn keyboard_layouts() -> Vec<windows_sys::Win32::UI::Input::KeyboardAndMouse::HKL> {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetKeyboardLayoutList;
    unsafe {
        let count = GetKeyboardLayoutList(0, std::ptr::null_mut());
        if count <= 0 {
            return Vec::new();
        }
        let mut layouts = vec![std::ptr::null_mut(); count as usize];
        let got = GetKeyboardLayoutList(count, layouts.as_mut_ptr());
        layouts.truncate(got.max(0) as usize);
        layouts
    }
}

#[cfg(windows)]
pub fn register(proxy: tao::event_loop::EventLoopProxy<crate::ipc::UserEvent>) -> Hotkeys {
    use std::collections::HashMap;

    use global_hotkey::{
        hotkey::{Code, HotKey, Modifiers},
        GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    };

    use crate::ipc::UserEvent;

    let code = |key: Key| match key {
        Key::Y => Code::KeyY,
        Key::P => Code::KeyP,
        Key::N => Code::KeyN,
        Key::B => Code::KeyB,
        Key::G => Code::KeyG,
        Key::L => Code::KeyL,
        Key::K => Code::KeyK,
        Key::M => Code::KeyM,
        Key::Period => Code::Period,
        Key::Up => Code::ArrowUp,
        Key::Down => Code::ArrowDown,
        Key::Left => Code::ArrowLeft,
        Key::Right => Code::ArrowRight,
        Key::PageUp => Code::PageUp,
        Key::PageDown => Code::PageDown,
        Key::Digit0 => Code::Digit0,
    };

    let manager = match GlobalHotKeyManager::new() {
        Ok(manager) => manager,
        Err(err) => {
            eprintln!("global shortcuts unavailable: {err}");
            return Hotkeys {
                _manager: None,
                shortcuts: DEFINITIONS
                    .iter()
                    .map(|d| Shortcut { action: d.action, label: d.label, key: d.keys[0], status: Status::InUse })
                    .collect(),
            };
        }
    };

    let layouts = keyboard_layouts();
    let mut commands: HashMap<u32, Command> = HashMap::new();
    let mut shortcuts = Vec::new();
    for def in DEFINITIONS {
        let (key, status) = choose(
            def.keys,
            |key| altgr_types(key, &layouts),
            |key| {
                let hotkey = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), code(key));
                match manager.register(hotkey) {
                    Ok(()) => {
                        commands.insert(hotkey.id(), (def.command)());
                        true
                    }
                    Err(err) => {
                        eprintln!("could not register {}: {err}", key.combo());
                        false
                    }
                }
            },
        );
        shortcuts.push(Shortcut { action: def.action, label: def.label, key, status });
    }

    GlobalHotKeyEvent::set_event_handler(Some(move |event: GlobalHotKeyEvent| {
        if event.state != HotKeyState::Pressed {
            return;
        }
        if let Some(command) = commands.get(&event.id) {
            let _ = proxy.send_event(UserEvent::Command(command.clone()));
        }
    }));

    Hotkeys { _manager: Some(manager), shortcuts }
}

#[cfg(not(windows))]
pub fn register<P>(_: P) -> Hotkeys {
    // Off Windows nothing is registered; this only keeps the helpers in use.
    for d in DEFINITIONS {
        let _ = ((d.command)(), d.keys[0].vk(), d.keys[0].types_text());
    }
    Hotkeys {
        shortcuts: DEFINITIONS
            .iter()
            .map(|d| Shortcut { action: d.action, label: d.label, key: d.keys[0], status: Status::InUse })
            .collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_that_type_with_altgr_get_the_next_key() {
        // Polish (programmers): AltGr+N = ń, AltGr+L = ł.
        let polish = |key: Key| matches!(key, Key::N | Key::L);
        assert_eq!(choose(&[Key::N, Key::Period], polish, |_| true), (Key::Period, Status::Ready));
        assert_eq!(choose(&[Key::L, Key::K], polish, |_| true), (Key::K, Status::Ready));
        assert_eq!(choose(&[Key::Y], polish, |_| true), (Key::Y, Status::Ready));
        // Typed with AltGr and no alternative: unavailable, and why.
        assert_eq!(choose(&[Key::M], |k| k == Key::M, |_| true), (Key::M, Status::TypesText));
        // Another program has it.
        assert_eq!(choose(&[Key::Up], |_| false, |_| false), (Key::Up, Status::InUse));
        // The first is typed and the alternative is taken: the last reason wins.
        assert_eq!(choose(&[Key::N, Key::Period], polish, |_| false), (Key::N, Status::InUse));
    }

    #[test]
    fn every_action_has_a_label_and_a_key() {
        for def in DEFINITIONS {
            assert!(!def.keys.is_empty() && def.label.starts_with("hotkey."));
        }
        assert!(!Key::Up.types_text() && Key::L.types_text());
        assert_eq!(Key::Right.combo(), "Ctrl+Alt+→");
    }
}
