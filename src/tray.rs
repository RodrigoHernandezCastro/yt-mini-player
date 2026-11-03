//! System tray icon.
//!
//! - Left click with the player hidden: opens (or closes) the controls in
//!   the taskbar. With the player showing: hides it.
//! - Double click: shows the player.
//! - Right click: what is playing, playback controls and options.
//! - The tooltip shows the title that is playing.

use crate::{config::LockMode, hotkeys::Action, i18n::Lang, ipc::Command, media::MediaAction};

/// What the tray menu reflects.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrayState {
    pub visible: bool,
    pub pinned: bool,
    /// Music bar in the taskbar while a music service is open.
    pub taskbar_bar: bool,
    pub lock: LockMode,
    /// Locked: the menu neither tells what is playing nor controls it.
    pub locked: bool,
    pub ghost: bool,
    pub autostart: bool,
    pub has_media: bool,
    pub playing: bool,
    pub can_next: bool,
    pub can_previous: bool,
    /// Title for the first line of the menu (already shortened).
    pub now_playing: String,
    pub tooltip: String,
    pub language: Lang,
    /// A watch together session is on.
    pub together: bool,
    pub hardware_acceleration: bool,
}

/// Longest title shown in the menu.
pub const MENU_TITLE_CHARS: usize = 48;

/// Shortcut text of an action, if its global shortcut works.
pub type Accelerators = Vec<(Action, String)>;

#[cfg_attr(not(windows), allow(dead_code))]
fn command_for(id: &str) -> Option<Command> {
    if let Some(code) = id.strip_prefix("lang-") {
        return Lang::ALL.into_iter().find(|l| l.code() == code).map(Command::SetLanguage);
    }
    Some(match id {
        "toggle-visible" => Command::ToggleVisible,
        "home" => Command::Home,
        "media-toggle" => Command::Media(MediaAction::PlayPause),
        "media-next" => Command::Media(MediaAction::Next),
        "media-previous" => Command::Media(MediaAction::Previous),
        "mini" => Command::MiniShow,
        "together" => Command::TogetherOpen,
        "settings" => Command::SettingsOpen(None),
        "pin" => Command::TogglePin,
        "taskbar-bar" => Command::ToggleTaskbarBar,
        "lock-hello" => Command::SetLock(LockMode::Hello),
        "lock-pin" => Command::SetLock(LockMode::Pin),
        "lock-now" => Command::LockNow,
        "sign-out" => Command::SignOutAll,
        "security" => Command::SecurityStatus,
        "ghost" => Command::ToggleGhost,
        "reset-opacity" => Command::ResetOpacity,
        "autostart" => Command::ToggleAutostart,
        "hardware-acceleration" => Command::ToggleHardwareAcceleration,
        "ext-folder" => Command::ExtensionsFolder,
        "quit" => Command::Quit,
        _ => return None,
    })
}

/// "Label<TAB>Ctrl+Alt+X" when the action has a working shortcut.
#[cfg_attr(not(windows), allow(dead_code))]
fn with_accelerator(label: String, action: Action, accelerators: &Accelerators) -> String {
    match accelerators.iter().find(|(a, _)| *a == action) {
        Some((_, keys)) => format!("{label}\t{keys}"),
        None => label,
    }
}

#[cfg(windows)]
pub use win::Tray;

#[cfg(windows)]
mod win {
    use tao::event_loop::EventLoopProxy;
    use tray_icon::{
        menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
        MouseButton, MouseButtonState, Rect, TrayIcon, TrayIconBuilder, TrayIconEvent,
    };

    use super::{command_for, with_accelerator, Accelerators, TrayState};
    use crate::{
        config::LockMode,
        hotkeys::Action,
        i18n::Lang,
        icon,
        ipc::{Command, UserEvent},
        platform::APP_NAME,
    };

    pub struct Tray {
        icon: TrayIcon,
        accelerators: Accelerators,
        now_playing: MenuItem,
        toggle: MenuItem,
        next: MenuItem,
        previous: MenuItem,
        mini: MenuItem,
        together: MenuItem,
        visible: MenuItem,
        home: MenuItem,
        open_settings: MenuItem,
        settings: Submenu,
        pin: CheckMenuItem,
        ghost: CheckMenuItem,
        reset: MenuItem,
        taskbar_bar: CheckMenuItem,
        autostart: CheckMenuItem,
        hardware: CheckMenuItem,
        folder: MenuItem,
        language: Submenu,
        languages: Vec<(Lang, CheckMenuItem)>,
        security: Submenu,
        lock_hello: CheckMenuItem,
        lock_pin: CheckMenuItem,
        lock_now: MenuItem,
        sign_out: MenuItem,
        status: MenuItem,
        quit: MenuItem,
        tooltip: std::cell::RefCell<String>,
        language_shown: std::cell::Cell<Option<Lang>>,
    }

    fn now_playing_text(state: &TrayState) -> String {
        if state.locked {
            format!("🔒 {}", t!("tray.locked"))
        } else if state.has_media && !state.now_playing.is_empty() {
            // "&" would be a menu mnemonic: doubled to show it as is.
            format!("{} {}", if state.playing { "▶" } else { "⏸" }, state.now_playing.replace('&', "&&"))
        } else {
            t!("tray.nothing_playing")
        }
    }

    /// Position of the icon (for the floating bar): horizontal centre, top
    /// edge (in physical pixels).
    fn anchor(rect: &Rect) -> (i32, i32) {
        (
            (rect.position.x + f64::from(rect.size.width) / 2.0).round() as i32,
            rect.position.y.round() as i32,
        )
    }

    impl Tray {
        pub fn new(proxy: EventLoopProxy<UserEvent>, state: TrayState, accelerators: Accelerators) -> Result<Self, String> {
            // Labels are set by `relabel`, right below.
            let item = |id: &str, enabled: bool| MenuItem::with_id(id, "", enabled, None);
            let check = |id: &str, checked: bool| CheckMenuItem::with_id(id, "", true, checked, None);
            let now_playing = item("now-playing", false);
            let toggle = item("media-toggle", state.has_media);
            let next = item("media-next", state.can_next);
            let previous = item("media-previous", state.can_previous);
            let mini = item("mini", true);
            let together = item("together", true);
            let visible = item("toggle-visible", true);
            let home = item("home", true);
            let open_settings = item("settings", true);
            let pin = check("pin", state.pinned);
            let taskbar_bar = check("taskbar-bar", state.taskbar_bar);
            let ghost = check("ghost", state.ghost);
            let reset = item("reset-opacity", true);
            let autostart = check("autostart", state.autostart);
            let hardware = check("hardware-acceleration", state.hardware_acceleration);
            let folder = item("ext-folder", true);
            let quit = item("quit", true);
            let lock_hello = check("lock-hello", state.lock == LockMode::Hello);
            let lock_pin = check("lock-pin", state.lock == LockMode::Pin);
            let lock_now = item("lock-now", state.lock != LockMode::Off);
            let sign_out = item("sign-out", true);
            let status = item("security", true);
            // Each language in its own name, so anyone can find theirs.
            let languages: Vec<(Lang, CheckMenuItem)> = Lang::ALL
                .iter()
                .map(|&l| (l, CheckMenuItem::with_id(format!("lang-{}", l.code()), l.native_name(), true, l == state.language, None)))
                .collect();
            let language_items: Vec<&dyn tray_icon::menu::IsMenuItem> =
                languages.iter().map(|(_, item)| item as &dyn tray_icon::menu::IsMenuItem).collect();
            let language = Submenu::with_items("", true, &language_items).map_err(|e| e.to_string())?;
            let security = Submenu::with_items(
                "",
                true,
                &[&lock_hello, &lock_pin, &lock_now, &PredefinedMenuItem::separator(), &sign_out, &status],
            )
            .map_err(|e| e.to_string())?;

            // Everyday things at the top; the options, in a submenu.
            let settings = Submenu::with_items(
                "",
                true,
                &[
                    &open_settings,
                    &PredefinedMenuItem::separator(),
                    &pin,
                    &ghost,
                    &reset,
                    &PredefinedMenuItem::separator(),
                    &taskbar_bar,
                    &autostart,
                    &hardware,
                    &folder,
                ],
            )
            .map_err(|e| e.to_string())?;
            let menu = Menu::new();
            menu.append_items(&[
                &now_playing,
                &PredefinedMenuItem::separator(),
                &toggle,
                &previous,
                &next,
                &mini,
                &PredefinedMenuItem::separator(),
                &together,
                &PredefinedMenuItem::separator(),
                &visible,
                &home,
                &settings,
                &security,
                &language,
                &PredefinedMenuItem::separator(),
                &quit,
            ])
            .map_err(|e| e.to_string())?;

            let icon = TrayIconBuilder::new()
                .with_tooltip(APP_NAME)
                .with_icon(icon::tray_icon().ok_or("invalid icon")?)
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .build()
                .map_err(|e| e.to_string())?;

            let menu_proxy = proxy.clone();
            MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
                if let Some(command) = command_for(event.id.0.as_str()) {
                    let _ = menu_proxy.send_event(UserEvent::Command(command));
                }
            }));
            TrayIconEvent::set_event_handler(Some(move |event: TrayIconEvent| {
                let command = match event {
                    TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, rect, .. } => {
                        let (x, y) = anchor(&rect);
                        Command::TrayClick(x, y)
                    }
                    TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } => Command::TrayDoubleClick,
                    _ => return,
                };
                let _ = proxy.send_event(UserEvent::Command(command));
            }));

            let tray = Self {
                icon,
                accelerators,
                now_playing,
                toggle,
                next,
                previous,
                mini,
                together,
                visible,
                home,
                open_settings,
                settings,
                pin,
                ghost,
                reset,
                taskbar_bar,
                autostart,
                hardware,
                folder,
                language,
                languages,
                security,
                lock_hello,
                lock_pin,
                lock_now,
                sign_out,
                status,
                quit,
                tooltip: std::cell::RefCell::new(String::new()),
                language_shown: std::cell::Cell::new(None),
            };
            tray.update(&state);
            Ok(tray)
        }

        /// Texts that only change with the language.
        fn relabel(&self) {
            let accel = |label: String, action: Action| with_accelerator(label, action, &self.accelerators);
            self.next.set_text(accel(t!("tray.next"), Action::Next));
            self.previous.set_text(accel(t!("tray.previous"), Action::Previous));
            self.mini.set_text(t!("tray.show_controls"));
            self.home.set_text(t!("tray.home"));
            self.settings.set_text(t!("tray.settings"));
            self.open_settings.set_text(t!("tray.open_settings"));
            self.pin.set_text(t!("tray.always_on_top"));
            self.ghost.set_text(accel(t!("tray.ghost"), Action::Ghost));
            self.reset.set_text(accel(t!("tray.reset_opacity"), Action::OpacityReset));
            self.taskbar_bar.set_text(t!("tray.taskbar_bar"));
            self.autostart.set_text(t!("tray.autostart"));
            self.hardware.set_text(t!("tray.hardware_acceleration"));
            self.folder.set_text(t!("tray.extensions_folder"));
            self.language.set_text(t!("tray.language"));
            self.security.set_text(t!("tray.security"));
            self.lock_hello.set_text(t!("tray.lock_hello"));
            self.lock_pin.set_text(t!("tray.lock_pin"));
            self.lock_now.set_text(accel(t!("tray.lock_now"), Action::Lock));
            self.sign_out.set_text(t!("tray.sign_out"));
            self.status.set_text(t!("tray.security_status"));
            self.quit.set_text(t!("tray.quit"));
        }

        pub fn update(&self, state: &TrayState) {
            if self.language_shown.get() != Some(state.language) {
                self.language_shown.set(Some(state.language));
                self.relabel();
                for (lang, item) in &self.languages {
                    item.set_checked(*lang == state.language);
                }
            }
            let accel = |label: String, action: Action| with_accelerator(label, action, &self.accelerators);
            self.now_playing.set_text(now_playing_text(state));
            let toggle = if state.playing { t!("tray.pause") } else { t!("tray.play") };
            self.toggle.set_text(accel(toggle, Action::PlayPause));
            self.toggle.set_enabled(state.has_media);
            self.next.set_enabled(state.has_media && state.can_next);
            self.previous.set_enabled(state.has_media && state.can_previous);
            let visible = if state.visible { t!("tray.hide_player") } else { t!("tray.show_player") };
            self.visible.set_text(accel(visible, Action::ToggleVisible));
            self.pin.set_checked(state.pinned);
            self.taskbar_bar.set_checked(state.taskbar_bar);
            self.lock_hello.set_checked(state.lock == LockMode::Hello);
            self.lock_pin.set_checked(state.lock == LockMode::Pin);
            // While locked nothing about security changes and the bar stays shut.
            self.lock_hello.set_enabled(!state.locked);
            self.lock_pin.set_enabled(!state.locked);
            self.sign_out.set_enabled(!state.locked);
            self.mini.set_enabled(!state.locked);
            self.open_settings.set_enabled(!state.locked);
            // A dot while a session is on.
            let together = t!("tray.together");
            self.together.set_text(if state.together { format!("● {together}") } else { together });
            self.together.set_enabled(!state.locked);
            self.lock_now.set_enabled(state.lock != LockMode::Off && !state.locked);
            self.ghost.set_checked(state.ghost);
            self.autostart.set_checked(state.autostart);
            self.hardware.set_checked(state.hardware_acceleration);
            // A language item that was clicked toggles itself: put it back.
            for (lang, item) in &self.languages {
                if item.is_checked() != (*lang == state.language) {
                    item.set_checked(*lang == state.language);
                }
            }
            // The tooltip is only touched when it changes (avoids flicker).
            if *self.tooltip.borrow() != state.tooltip {
                let _ = self.icon.set_tooltip(Some(&state.tooltip));
                *self.tooltip.borrow_mut() = state.tooltip.clone();
            }
        }
    }
}

#[cfg(not(windows))]
pub struct Tray;

#[cfg(not(windows))]
impl Tray {
    pub fn new<P>(_: P, _: TrayState, accelerators: Accelerators) -> Result<Self, String> {
        let _ = (command_for("quit"), with_accelerator(String::new(), Action::Next, &accelerators));
        Err("no tray outside Windows".into())
    }
    pub fn update(&self, _: &TrayState) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn menu_ids_map_to_commands() {
        assert_eq!(command_for("media-next"), Some(Command::Media(MediaAction::Next)));
        assert_eq!(command_for("mini"), Some(Command::MiniShow));
        assert_eq!(command_for("together"), Some(Command::TogetherOpen));
        assert_eq!(command_for("settings"), Some(Command::SettingsOpen(None)));
        assert_eq!(command_for("taskbar-bar"), Some(Command::ToggleTaskbarBar));
        assert_eq!(command_for("lock-hello"), Some(Command::SetLock(LockMode::Hello)));
        assert_eq!(command_for("lock-now"), Some(Command::LockNow));
        assert_eq!(command_for("sign-out"), Some(Command::SignOutAll));
        assert_eq!(command_for("lang-ja"), Some(Command::SetLanguage(Lang::Ja)));
        assert_eq!(command_for("hardware-acceleration"), Some(Command::ToggleHardwareAcceleration));
        assert_eq!(command_for("lang-xx"), None);
        assert_eq!(command_for("now-playing"), None, "the title line does nothing");
        assert_eq!(command_for("unknown"), None);
    }

    #[test]
    fn accelerators_only_when_the_shortcut_works() {
        let accelerators = vec![(Action::Next, "Ctrl+Alt+.".to_owned())];
        assert_eq!(with_accelerator("Next".into(), Action::Next, &accelerators), "Next\tCtrl+Alt+.");
        assert_eq!(with_accelerator("Lock now".into(), Action::Lock, &accelerators), "Lock now");
    }
}
