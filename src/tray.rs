//! Icono en la bandeja del sistema.
//!
//! - Clic izquierdo con el reproductor oculto: abre (o cierra) la barra de
//!   controles en la barra de tareas. Con el reproductor visible: lo oculta.
//! - Doble clic: muestra el reproductor.
//! - Clic derecho: lo que suena, controles de reproducción y opciones.
//! - El tooltip muestra el título que suena.

use crate::{config::LockMode, ipc::Command, media::MediaAction};

/// Lo que el menú de la bandeja refleja.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct TrayState {
    pub visible: bool,
    pub pinned: bool,
    /// Barra de música en la barra de tareas con servicios de música.
    pub taskbar_bar: bool,
    pub lock: LockMode,
    /// Bloqueada: el menú no dice qué suena ni deja controlarlo.
    pub locked: bool,
    pub ghost: bool,
    pub autostart: bool,
    pub has_media: bool,
    pub playing: bool,
    pub can_next: bool,
    pub can_previous: bool,
    /// Título para la primera línea del menú (ya acortado).
    pub now_playing: String,
    pub tooltip: String,
}

/// Longitud máxima del título en el menú.
pub const MENU_TITLE_CHARS: usize = 48;

#[cfg_attr(not(windows), allow(dead_code))]
fn command_for(id: &str) -> Option<Command> {
    Some(match id {
        "toggle-visible" => Command::ToggleVisible,
        "home" => Command::Home,
        "media-toggle" => Command::Media(MediaAction::PlayPause),
        "media-next" => Command::Media(MediaAction::Next),
        "media-previous" => Command::Media(MediaAction::Previous),
        "mini" => Command::MiniShow,
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
        "ext-folder" => Command::ExtensionsFolder,
        "quit" => Command::Quit,
        _ => return None,
    })
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

    use super::{command_for, TrayState};
    use crate::config::LockMode;
    use crate::{
        icon,
        ipc::{Command, UserEvent},
        platform::APP_NAME,
    };

    pub struct Tray {
        icon: TrayIcon,
        now_playing: MenuItem,
        toggle: MenuItem,
        next: MenuItem,
        previous: MenuItem,
        visible: MenuItem,
        pin: CheckMenuItem,
        taskbar_bar: CheckMenuItem,
        lock_hello: CheckMenuItem,
        lock_pin: CheckMenuItem,
        lock_now: MenuItem,
        sign_out: MenuItem,
        mini: MenuItem,
        ghost: CheckMenuItem,
        autostart: CheckMenuItem,
        tooltip: std::cell::RefCell<String>,
    }

    fn visible_text(visible: bool) -> &'static str {
        if visible { "Ocultar reproductor\tCtrl+Alt+Y" } else { "Mostrar reproductor\tCtrl+Alt+Y" }
    }

    fn now_playing_text(state: &TrayState) -> String {
        if state.locked {
            "🔒 Bloqueado".to_owned()
        } else if state.has_media && !state.now_playing.is_empty() {
            // "&" sería un atajo de menú: se duplica para mostrarlo tal cual.
            format!("{} {}", if state.playing { "▶" } else { "⏸" }, state.now_playing.replace('&', "&&"))
        } else {
            "Nada sonando".to_owned()
        }
    }

    fn toggle_text(playing: bool) -> &'static str {
        if playing { "Pausar\tCtrl+Alt+P" } else { "Reproducir\tCtrl+Alt+P" }
    }

    /// Posición del icono (para la barra flotante): centro horizontal,
    /// borde superior (en píxeles físicos).
    fn anchor(rect: &Rect) -> (i32, i32) {
        (
            (rect.position.x + f64::from(rect.size.width) / 2.0).round() as i32,
            rect.position.y.round() as i32,
        )
    }

    impl Tray {
        pub fn new(proxy: EventLoopProxy<UserEvent>, state: TrayState) -> Result<Self, String> {
            let now_playing = MenuItem::with_id("now-playing", now_playing_text(&state), false, None);
            let toggle = MenuItem::with_id("media-toggle", toggle_text(state.playing), state.has_media, None);
            let next = MenuItem::with_id("media-next", "Siguiente\tCtrl+Alt+N", state.can_next, None);
            let previous = MenuItem::with_id("media-previous", "Anterior\tCtrl+Alt+B", state.can_previous, None);
            let mini = MenuItem::with_id("mini", "Mostrar barra de controles", true, None);
            let visible = MenuItem::with_id("toggle-visible", visible_text(state.visible), true, None);
            let home = MenuItem::with_id("home", "Menú principal", true, None);
            let pin = CheckMenuItem::with_id("pin", "Siempre visible", true, state.pinned, None);
            let taskbar_bar = CheckMenuItem::with_id(
                "taskbar-bar",
                "Barra de música en la barra de tareas",
                true,
                state.taskbar_bar,
                None,
            );
            let ghost = CheckMenuItem::with_id("ghost", "Modo fantasma\tCtrl+Alt+G", true, state.ghost, None);
            let reset = MenuItem::with_id("reset-opacity", "Opacidad al 100 %\tCtrl+Alt+0", true, None);
            let autostart = CheckMenuItem::with_id("autostart", "Iniciar con Windows", true, state.autostart, None);
            let folder = MenuItem::with_id("ext-folder", "Abrir carpeta de extensiones", true, None);
            let quit = MenuItem::with_id("quit", "Salir", true, None);
            let lock_hello =
                CheckMenuItem::with_id("lock-hello", "Bloquear con Windows Hello", true, state.lock == LockMode::Hello, None);
            let lock_pin = CheckMenuItem::with_id("lock-pin", "Bloquear con PIN propio…", true, state.lock == LockMode::Pin, None);
            let lock_now = MenuItem::with_id("lock-now", "Bloquear ahora\tCtrl+Alt+L", state.lock != LockMode::Off, None);
            let sign_out = MenuItem::with_id("sign-out", "Cerrar sesión en todos los servicios…", true, None);
            let security = MenuItem::with_id("security", "Estado de la protección…", true, None);
            let security_menu = Submenu::with_items(
                "Seguridad",
                true,
                &[&lock_hello, &lock_pin, &lock_now, &PredefinedMenuItem::separator(), &sign_out, &security],
            )
            .map_err(|e| e.to_string())?;

            // Lo de todos los días arriba; las opciones, en un submenú.
            let settings = Submenu::with_items(
                "Ajustes",
                true,
                &[
                    &pin,
                    &ghost,
                    &reset,
                    &PredefinedMenuItem::separator(),
                    &taskbar_bar,
                    &autostart,
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
                &visible,
                &home,
                &settings,
                &security_menu,
                &PredefinedMenuItem::separator(),
                &quit,
            ])
            .map_err(|e| e.to_string())?;

            let icon = TrayIconBuilder::new()
                .with_tooltip(APP_NAME)
                .with_icon(icon::tray_icon().ok_or("icono inválido")?)
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
                now_playing,
                toggle,
                next,
                previous,
                visible,
                pin,
                taskbar_bar,
                lock_hello,
                lock_pin,
                lock_now,
                sign_out,
                mini,
                ghost,
                autostart,
                tooltip: std::cell::RefCell::new(String::new()),
            };
            tray.update(&state);
            Ok(tray)
        }

        pub fn update(&self, state: &TrayState) {
            self.now_playing.set_text(now_playing_text(state));
            self.toggle.set_text(toggle_text(state.playing));
            self.toggle.set_enabled(state.has_media);
            self.next.set_enabled(state.has_media && state.can_next);
            self.previous.set_enabled(state.has_media && state.can_previous);
            self.visible.set_text(visible_text(state.visible));
            self.pin.set_checked(state.pinned);
            self.taskbar_bar.set_checked(state.taskbar_bar);
            self.lock_hello.set_checked(state.lock == LockMode::Hello);
            self.lock_pin.set_checked(state.lock == LockMode::Pin);
            // Bloqueada no se cambia nada de seguridad ni se abre la barra.
            self.lock_hello.set_enabled(!state.locked);
            self.lock_pin.set_enabled(!state.locked);
            self.sign_out.set_enabled(!state.locked);
            self.mini.set_enabled(!state.locked);
            self.lock_now.set_enabled(state.lock != LockMode::Off && !state.locked);
            self.ghost.set_checked(state.ghost);
            self.autostart.set_checked(state.autostart);
            // El tooltip solo se toca si cambia (evita parpadeos).
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
    pub fn new<P>(_: P, _: TrayState) -> Result<Self, String> {
        let _ = command_for("quit");
        Err("sin bandeja fuera de Windows".into())
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
        assert_eq!(command_for("taskbar-bar"), Some(Command::ToggleTaskbarBar));
        assert_eq!(command_for("lock-hello"), Some(Command::SetLock(LockMode::Hello)));
        assert_eq!(command_for("lock-now"), Some(Command::LockNow));
        assert_eq!(command_for("sign-out"), Some(Command::SignOutAll));
        assert_eq!(command_for("now-playing"), None, "la línea del título no hace nada");
        assert_eq!(command_for("desconocido"), None);
    }
}
