//! Icono en la bandeja del sistema. Clic izquierdo: mostrar u ocultar.
//! Clic derecho: menú con las opciones más usadas y "Salir".

use crate::ipc::Command;

/// Lo que el menú de la bandeja refleja.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TrayState {
    pub visible: bool,
    pub pinned: bool,
    pub ghost: bool,
    pub autostart: bool,
}

#[cfg_attr(not(windows), allow(dead_code))]
fn command_for(id: &str) -> Option<Command> {
    Some(match id {
        "toggle-visible" => Command::ToggleVisible,
        "pin" => Command::TogglePin,
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
        menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
        MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
    };

    use super::{command_for, TrayState};
    use crate::{icon, ipc::UserEvent, platform::APP_NAME};

    pub struct Tray {
        _icon: TrayIcon,
        visible: MenuItem,
        pin: CheckMenuItem,
        ghost: CheckMenuItem,
        autostart: CheckMenuItem,
    }

    fn visible_text(visible: bool) -> &'static str {
        if visible { "Ocultar reproductor\tCtrl+Alt+Y" } else { "Mostrar reproductor\tCtrl+Alt+Y" }
    }

    impl Tray {
        pub fn new(proxy: EventLoopProxy<UserEvent>, state: TrayState) -> Result<Self, String> {
            let visible = MenuItem::with_id("toggle-visible", visible_text(state.visible), true, None);
            let pin = CheckMenuItem::with_id("pin", "Siempre visible", true, state.pinned, None);
            let ghost = CheckMenuItem::with_id("ghost", "Modo fantasma\tCtrl+Alt+G", true, state.ghost, None);
            let reset = MenuItem::with_id("reset-opacity", "Opacidad al 100 %\tCtrl+Alt+0", true, None);
            let autostart = CheckMenuItem::with_id("autostart", "Iniciar con Windows", true, state.autostart, None);
            let folder = MenuItem::with_id("ext-folder", "Abrir carpeta de extensiones", true, None);
            let quit = MenuItem::with_id("quit", "Salir", true, None);

            let menu = Menu::new();
            menu.append_items(&[
                &visible,
                &PredefinedMenuItem::separator(),
                &pin,
                &ghost,
                &reset,
                &PredefinedMenuItem::separator(),
                &autostart,
                &folder,
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
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    let _ = proxy.send_event(UserEvent::Command(crate::ipc::Command::ToggleVisible));
                }
            }));

            Ok(Self { _icon: icon, visible, pin, ghost, autostart })
        }

        pub fn update(&self, state: TrayState) {
            self.visible.set_text(visible_text(state.visible));
            self.pin.set_checked(state.pinned);
            self.ghost.set_checked(state.ghost);
            self.autostart.set_checked(state.autostart);
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
    pub fn update(&self, _: TrayState) {}
}

