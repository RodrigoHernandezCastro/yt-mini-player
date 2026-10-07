//! Atajos globales: funcionan aunque la ventana no tenga el foco (o esté
//! oculta en la bandeja). Todos usan Ctrl+Alt para no chocar con YouTube.
//!
//! Se evitan Ctrl+Alt+flechas porque algunos controladores Intel los usan
//! para girar la pantalla, y teclas que con AltGr (= Ctrl+Alt) escriben
//! símbolos en las distribuciones española y latinoamericana.

use serde::Serialize;

use crate::ipc::{Command, Media};

/// Descripción de un atajo para la ayuda de la página.
#[derive(Debug, Clone, Serialize)]
pub struct ShortcutInfo {
    pub keys: &'static str,
    pub label: &'static str,
    /// `false` si Windows no permitió registrarlo (otro programa lo usa).
    pub available: bool,
}

#[cfg_attr(not(windows), allow(dead_code))]
#[derive(Clone, Copy)]
enum Key {
    Y,
    P,
    N,
    B,
    G,
    PageUp,
    PageDown,
    Digit0,
}

struct Definition {
    keys: &'static str,
    label: &'static str,
    #[cfg_attr(not(windows), allow(dead_code))]
    key: Key,
    command: fn() -> Command,
}

const DEFINITIONS: &[Definition] = &[
    Definition { keys: "Ctrl+Alt+Y", label: "Mostrar u ocultar el reproductor", key: Key::Y, command: || Command::ToggleVisible },
    Definition { keys: "Ctrl+Alt+P", label: "Reproducir / pausar", key: Key::P, command: || Command::Media(Media::PlayPause) },
    Definition { keys: "Ctrl+Alt+N", label: "Siguiente video", key: Key::N, command: || Command::Media(Media::Next) },
    Definition { keys: "Ctrl+Alt+B", label: "Video anterior", key: Key::B, command: || Command::Media(Media::Previous) },
    Definition { keys: "Ctrl+Alt+G", label: "Modo fantasma (los clics atraviesan la ventana)", key: Key::G, command: || Command::ToggleGhost },
    Definition { keys: "Ctrl+Alt+RePág", label: "Más opacidad", key: Key::PageUp, command: || Command::AdjustOpacity(10) },
    Definition { keys: "Ctrl+Alt+AvPág", label: "Menos opacidad", key: Key::PageDown, command: || Command::AdjustOpacity(-10) },
    Definition { keys: "Ctrl+Alt+0", label: "Opacidad al 100 %", key: Key::Digit0, command: || Command::ResetOpacity },
];

pub struct Hotkeys {
    #[cfg(windows)]
    _manager: Option<global_hotkey::GlobalHotKeyManager>,
    pub shortcuts: Vec<ShortcutInfo>,
}

#[cfg(windows)]
pub fn register(proxy: tao::event_loop::EventLoopProxy<crate::ipc::UserEvent>) -> Hotkeys {
    use std::collections::HashMap;

    use global_hotkey::{
        hotkey::{Code, HotKey, Modifiers},
        GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState,
    };

    use crate::ipc::UserEvent;

    let manager = match GlobalHotKeyManager::new() {
        Ok(manager) => manager,
        Err(err) => {
            eprintln!("atajos globales no disponibles: {err}");
            return Hotkeys {
                _manager: None,
                shortcuts: DEFINITIONS
                    .iter()
                    .map(|d| ShortcutInfo { keys: d.keys, label: d.label, available: false })
                    .collect(),
            };
        }
    };

    let mut commands: HashMap<u32, Command> = HashMap::new();
    let mut shortcuts = Vec::new();
    for def in DEFINITIONS {
        let code = match def.key {
            Key::Y => Code::KeyY,
            Key::P => Code::KeyP,
            Key::N => Code::KeyN,
            Key::B => Code::KeyB,
            Key::G => Code::KeyG,
            Key::PageUp => Code::PageUp,
            Key::PageDown => Code::PageDown,
            Key::Digit0 => Code::Digit0,
        };
        let hotkey = HotKey::new(Some(Modifiers::CONTROL | Modifiers::ALT), code);
        let available = match manager.register(hotkey) {
            Ok(()) => {
                commands.insert(hotkey.id(), (def.command)());
                true
            }
            Err(err) => {
                eprintln!("no se pudo registrar {}: {err}", def.keys);
                false
            }
        };
        shortcuts.push(ShortcutInfo { keys: def.keys, label: def.label, available });
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
    let _ = DEFINITIONS.iter().map(|d| ((d.command)(), d.key)).count();
    Hotkeys {
        shortcuts: DEFINITIONS
            .iter()
            .map(|d| ShortcutInfo { keys: d.keys, label: d.label, available: false })
            .collect(),
    }
}
