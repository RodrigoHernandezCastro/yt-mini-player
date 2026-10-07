//! Estado de la aplicación y ejecución de órdenes.
//!
//! Todas las acciones llegan aquí como [`Command`], vengan de la página
//! (barra lateral, atajos locales), de la bandeja o de un atajo global.

use serde::Serialize;
use tao::{
    dpi::{LogicalSize, PhysicalPosition},
    event_loop::EventLoopProxy,
    window::{Fullscreen, ResizeDirection, Window},
};
use wry::{WebContext, WebView};

use crate::{
    config::{self, ExtensionIds, Paths, Settings},
    extensions::{self, ExtensionEvent, ExtensionHost, ExtensionStatus, LocalExtension},
    hotkeys::{self, Hotkeys, ShortcutInfo},
    ipc::{Command, Edge, UserEvent, MIN_OPACITY},
    lifecycle::Lifecycle,
    links, platform,
    tray::{Tray, TrayState},
    window as app_window,
};

/// Opacidad máxima en modo fantasma: así se nota que está activo.
const GHOST_MAX_OPACITY: u8 = 70;
/// Separación de los bordes al colocar en una esquina (px lógicos).
const CORNER_MARGIN: f64 = 12.0;

pub struct App {
    window: Window,
    webview: WebView,
    _web_context: WebContext,
    paths: Paths,
    settings: Settings,
    lifecycle: Lifecycle,

    hidden: bool,
    ghost: bool,
    fullscreen: bool,
    autostart: bool,

    ext_host: ExtensionHost,
    local_extensions: Vec<LocalExtension>,
    extensions: Vec<ExtensionStatus>,
    extension_ids: ExtensionIds,
    extension_problems: Vec<String>,
    extensions_error: Option<String>,

    tray: Option<Tray>,
    hotkeys: Hotkeys,
}

/// Estado que se envía a la página (barra lateral y menú).
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PageState<'a> {
    version: &'static str,
    pinned: bool,
    video_only: bool,
    opacity: u8,
    ghost: bool,
    fullscreen: bool,
    autostart: bool,
    tray: bool,
    extensions: &'a [ExtensionStatus],
    extension_problems: &'a [String],
    extensions_error: Option<&'a str>,
    hotkeys: &'a [ShortcutInfo],
}

pub struct Startup {
    pub window: Window,
    pub webview: WebView,
    pub web_context: WebContext,
    pub paths: Paths,
    pub settings: Settings,
    pub proxy: EventLoopProxy<UserEvent>,
    pub start_hidden: bool,
}

impl App {
    pub fn new(s: Startup) -> Self {
        let autostart = platform::autostart_enabled();
        let hotkeys = hotkeys::register(s.proxy.clone());
        let ext_host = ExtensionHost::new(&s.webview, s.proxy.clone());
        let extension_ids = config::load_extension_ids(&s.paths.extension_ids);

        let tray_state = TrayState {
            visible: !s.start_hidden,
            pinned: s.settings.always_on_top,
            ghost: false,
            autostart,
        };
        let tray = match Tray::new(s.proxy.clone(), tray_state) {
            Ok(tray) => Some(tray),
            Err(err) => {
                eprintln!("sin icono en la bandeja: {err}");
                None
            }
        };

        let mut app = Self {
            window: s.window,
            webview: s.webview,
            _web_context: s.web_context,
            paths: s.paths,
            settings: s.settings,
            lifecycle: Lifecycle::default(),
            hidden: s.start_hidden,
            ghost: false,
            fullscreen: false,
            autostart,
            ext_host,
            local_extensions: Vec::new(),
            extensions: Vec::new(),
            extension_ids,
            extension_problems: Vec::new(),
            extensions_error: None,
            tray,
            hotkeys,
        };

        app.reload_extensions();
        // Sin bandeja no hay forma de volver a mostrar una ventana oculta.
        if app.hidden && app.tray.is_none() {
            app.hidden = false;
        }
        if app.hidden {
            app.lifecycle.update(&app.webview, true);
        } else {
            app.window.set_visible(true);
        }
        app.apply_effects();
        app
    }

    // ------------------------------------------------------------------
    // Eventos
    // ------------------------------------------------------------------

    /// Devuelve `false` si la aplicación debe terminar.
    pub fn on_user_event(&mut self, event: UserEvent) -> bool {
        match event {
            UserEvent::Command(command) => self.handle(command),
            UserEvent::Extension(event) => {
                self.on_extension(event);
                true
            }
        }
    }

    /// Minimizar y restaurar llegan como `Resized`.
    pub fn on_resized(&mut self) {
        self.update_lifecycle();
    }

    /// Devuelve `false` si la aplicación debe terminar.
    pub fn handle(&mut self, command: Command) -> bool {
        match command {
            Command::Drag => {
                if !self.fullscreen {
                    let _ = self.window.drag_window();
                }
            }
            Command::Resize(edge) => {
                if !self.fullscreen {
                    let _ = self.window.drag_resize_window(resize_direction(edge));
                }
            }
            Command::Ready => {
                // Una navegación completa sale de la pantalla completa de la
                // página; la ventana debe acompañarla.
                if self.fullscreen {
                    self.set_fullscreen(false);
                }
                self.push_state();
            }
            Command::TogglePin => {
                self.settings.always_on_top = !self.settings.always_on_top;
                self.window.set_always_on_top(self.settings.always_on_top);
                self.apply_effects();
                self.save_settings();
                self.toast(if self.settings.always_on_top {
                    "Siempre visible: activado"
                } else {
                    "Siempre visible: desactivado"
                });
                self.state_changed();
            }
            Command::SetFullscreen(on) => self.set_fullscreen(on),
            Command::SetOpacity(value) => self.set_opacity(value),
            Command::AdjustOpacity(delta) => {
                let value = (i16::from(self.settings.opacity) + i16::from(delta))
                    .clamp(i16::from(MIN_OPACITY), 100) as u8;
                self.set_opacity(value);
                self.toast(&format!("Opacidad {value} %"));
            }
            Command::ResetOpacity => {
                self.set_opacity(100);
                self.toast("Opacidad 100 %");
            }
            Command::ToggleGhost => {
                self.ghost = !self.ghost;
                self.apply_effects();
                if self.ghost {
                    // Si estaba oculta, se muestra: si no, el modo no tendría
                    // efecto visible y sería confuso.
                    if self.hidden {
                        self.show();
                    }
                    self.toast("Modo fantasma: los clics atraviesan la ventana. Ctrl+Alt+G para salir.");
                } else {
                    self.toast("Modo fantasma desactivado");
                }
                self.state_changed();
            }
            Command::Size(preset) => {
                if !self.fullscreen {
                    let (w, h) = preset.logical_size();
                    self.window.set_inner_size(LogicalSize::new(w, h));
                    self.keep_inside_work_area();
                }
            }
            Command::Corner(corner) => {
                if !self.fullscreen {
                    if let Some(area) = platform::work_area(self.hwnd()) {
                        let size = self.window.outer_size();
                        let margin = (CORNER_MARGIN * self.window.scale_factor()).round() as i32;
                        let (x, y) = platform::corner_position(
                            area,
                            (size.width as i32, size.height as i32),
                            margin,
                            corner,
                        );
                        self.window.set_outer_position(PhysicalPosition::new(x, y));
                    }
                }
            }
            Command::Show => self.show(),
            Command::Hide => self.hide(),
            Command::ToggleVisible => {
                if self.hidden || self.window.is_minimized() {
                    self.show();
                } else {
                    self.hide();
                }
            }
            Command::Quit => {
                self.save_on_exit();
                return false;
            }
            Command::ToggleVideoOnly => {
                self.settings.video_only = !self.settings.video_only;
                self.save_settings();
                self.push_state();
            }
            Command::OpenClipboard => match platform::read_clipboard_text().as_deref().and_then(links::normalize) {
                Some(url) => {
                    if self.hidden {
                        self.show();
                    }
                    let _ = self.webview.load_url(&url);
                }
                None => self.toast("El portapapeles no contiene un enlace de YouTube"),
            },
            Command::Media(action) => {
                let _ = self.webview.evaluate_script(&format!(
                    "window.__ytmpMedia && window.__ytmpMedia('{}')",
                    action.js_name()
                ));
            }
            Command::ExtensionsFolder => platform::open_path(&self.paths.extensions),
            Command::ExtensionsReload => {
                self.reload_extensions();
                self.toast("Extensiones recargadas. Recarga la página (F5) para aplicarlas.");
            }
            Command::ExtensionToggle(id) => {
                if let Some(status) = self.extensions.iter().find(|s| s.id.as_deref() == Some(id.as_str())) {
                    self.ext_host.set_enabled(id, !status.enabled);
                }
            }
            Command::ExtensionOptions(id) => {
                let page = self
                    .extensions
                    .iter()
                    .find(|s| s.id.as_deref() == Some(id.as_str()))
                    .and_then(|s| self.local_extensions.iter().find(|l| l.folder == s.folder))
                    .and_then(|l| l.page.clone());
                match page {
                    Some(page) => {
                        let _ = self.webview.load_url(&format!("chrome-extension://{id}/{page}"));
                    }
                    None => self.toast("Esta extensión no tiene página de opciones"),
                }
            }
            Command::ToggleAutostart => {
                let wanted = !self.autostart;
                match platform::set_autostart(wanted) {
                    Ok(()) => {
                        self.autostart = wanted;
                        self.toast(if wanted {
                            "Se iniciará con Windows, oculto en la bandeja"
                        } else {
                            "Ya no se iniciará con Windows"
                        });
                    }
                    Err(err) => self.toast(&format!("No se pudo cambiar el inicio con Windows: {err}")),
                }
                self.state_changed();
            }
        }
        true
    }

    // ------------------------------------------------------------------
    // Ventana
    // ------------------------------------------------------------------

    #[cfg(windows)]
    fn hwnd(&self) -> isize {
        use tao::platform::windows::WindowExtWindows;
        self.window.hwnd()
    }

    #[cfg(not(windows))]
    fn hwnd(&self) -> isize {
        0
    }

    fn effective_opacity(&self) -> u8 {
        if self.ghost {
            self.settings.opacity.min(GHOST_MAX_OPACITY)
        } else {
            self.settings.opacity
        }
    }

    /// Reaplica opacidad y modo fantasma (tao los borra al cambiar estilos).
    fn apply_effects(&self) {
        platform::apply_window_effects(self.hwnd(), self.effective_opacity(), self.ghost);
    }

    fn set_opacity(&mut self, value: u8) {
        self.settings.opacity = value.clamp(MIN_OPACITY, 100);
        self.apply_effects();
        self.save_settings();
        self.push_state();
    }

    fn set_fullscreen(&mut self, on: bool) {
        if on == self.fullscreen {
            return;
        }
        self.fullscreen = on;
        // `None` = el monitor donde está la ventana.
        self.window.set_fullscreen(on.then_some(Fullscreen::Borderless(None)));
        self.apply_effects();
        self.push_state();
    }

    fn show(&mut self) {
        self.hidden = false;
        self.window.set_visible(true);
        if self.window.is_minimized() {
            self.window.set_minimized(false);
        }
        self.window.set_focus();
        self.apply_effects();
        self.update_lifecycle();
        self.update_tray();
    }

    fn hide(&mut self) {
        if self.tray.is_none() {
            // Sin bandeja, ocultar dejaría la ventana inaccesible.
            self.window.set_minimized(true);
            return;
        }
        if self.fullscreen {
            self.set_fullscreen(false);
        }
        self.hidden = true;
        self.window.set_visible(false);
        self.update_lifecycle();
        self.update_tray();
    }

    fn update_lifecycle(&mut self) {
        let background = self.hidden || self.window.is_minimized();
        self.lifecycle.update(&self.webview, background);
    }

    /// Tras cambiar de tamaño, vuelve a meter la ventana en la pantalla.
    fn keep_inside_work_area(&self) {
        let (Some(area), Ok(pos)) = (platform::work_area(self.hwnd()), self.window.outer_position()) else {
            return;
        };
        let size = self.window.outer_size();
        let x = pos.x.min(area.right - size.width as i32).max(area.left);
        let y = pos.y.min(area.bottom - size.height as i32).max(area.top);
        if (x, y) != (pos.x, pos.y) {
            self.window.set_outer_position(PhysicalPosition::new(x, y));
        }
    }

    // ------------------------------------------------------------------
    // Extensiones
    // ------------------------------------------------------------------

    fn reload_extensions(&mut self) {
        let scan = extensions::scan(&self.paths.extensions);
        self.extensions = scan.extensions.iter().map(ExtensionStatus::pending).collect();
        self.extension_problems = scan.problems;
        self.local_extensions = scan.extensions.clone();
        if let Some(error) = &self.extensions_error {
            for status in &mut self.extensions {
                status.pending = false;
                status.error = Some(error.clone());
            }
        } else {
            self.ext_host.sync(scan.extensions, self.extension_ids.clone());
        }
        self.push_state();
    }

    fn on_extension(&mut self, event: ExtensionEvent) {
        match event {
            ExtensionEvent::Installed { folder, id, enabled } => {
                if let Some(status) = self.extensions.iter_mut().find(|s| s.folder == folder) {
                    status.id = Some(id.clone());
                    status.enabled = enabled;
                    status.pending = false;
                    status.error = None;
                }
                if self.extension_ids.get(&folder) != Some(&id) {
                    self.extension_ids.insert(folder, id);
                    self.save_extension_ids();
                }
            }
            ExtensionEvent::Failed { folder, error } => {
                match self.extensions.iter_mut().find(|s| s.folder == folder && !folder.is_empty()) {
                    Some(status) => {
                        status.pending = false;
                        status.error = Some(error);
                    }
                    None => self.toast(&format!("Extensión: {error}")),
                }
            }
            ExtensionEvent::Toggled { id, enabled } => {
                if let Some(status) = self.extensions.iter_mut().find(|s| s.id.as_deref() == Some(id.as_str())) {
                    status.enabled = enabled;
                    let message = format!(
                        "{} {}. Recarga la página (F5) para aplicarlo.",
                        status.name,
                        if enabled { "activada" } else { "desactivada" }
                    );
                    self.toast(&message);
                }
            }
            ExtensionEvent::Removed { id } => {
                let before = self.extension_ids.len();
                self.extension_ids.retain(|_, v| *v != id);
                if self.extension_ids.len() != before {
                    self.save_extension_ids();
                }
            }
            ExtensionEvent::Unsupported(error) => {
                let message = format!(
                    "Este runtime de WebView2 no admite extensiones ({error}). Actualiza Microsoft Edge WebView2."
                );
                for status in &mut self.extensions {
                    status.pending = false;
                    status.error = Some(message.clone());
                }
                self.extensions_error = Some(message);
            }
        }
        self.push_state();
    }

    // ------------------------------------------------------------------
    // Persistencia y página
    // ------------------------------------------------------------------

    fn save_settings(&self) {
        if let Err(err) = self.settings.save(&self.paths.settings) {
            eprintln!("no se pudieron guardar los ajustes: {err}");
        }
    }

    fn save_extension_ids(&self) {
        if let Err(err) = config::save_extension_ids(&self.paths.extension_ids, &self.extension_ids) {
            eprintln!("no se pudo guardar extensions.json: {err}");
        }
    }

    /// tao termina el proceso con `exit()` sin ejecutar destructores, así
    /// que aquí se guarda todo y se quita el icono de la bandeja (si no,
    /// quedaría un icono fantasma hasta pasar el ratón por encima).
    fn save_on_exit(&mut self) {
        if !self.fullscreen {
            if let Some(geometry) = app_window::current_geometry(&self.window) {
                if let Err(err) = geometry.save(&self.paths.window_state) {
                    eprintln!("no se pudo guardar la geometría: {err}");
                }
            }
        }
        self.save_settings();
        self.tray = None;
    }

    fn update_tray(&self) {
        if let Some(tray) = &self.tray {
            tray.update(TrayState {
                visible: !self.hidden,
                pinned: self.settings.always_on_top,
                ghost: self.ghost,
                autostart: self.autostart,
            });
        }
    }

    fn state_changed(&self) {
        self.update_tray();
        self.push_state();
    }

    fn push_state(&self) {
        let state = PageState {
            version: env!("CARGO_PKG_VERSION"),
            pinned: self.settings.always_on_top,
            video_only: self.settings.video_only,
            opacity: self.settings.opacity,
            ghost: self.ghost,
            fullscreen: self.fullscreen,
            autostart: self.autostart,
            tray: self.tray.is_some(),
            extensions: &self.extensions,
            extension_problems: &self.extension_problems,
            extensions_error: self.extensions_error.as_deref(),
            hotkeys: &self.hotkeys.shortcuts,
        };
        if let Ok(json) = serde_json::to_string(&state) {
            let _ = self
                .webview
                .evaluate_script(&format!("window.__ytmpSync && window.__ytmpSync({json})"));
        }
    }

    fn toast(&self, message: &str) {
        if let Ok(json) = serde_json::to_string(message) {
            let _ = self
                .webview
                .evaluate_script(&format!("window.__ytmpToast && window.__ytmpToast({json})"));
        }
    }
}

fn resize_direction(edge: Edge) -> ResizeDirection {
    match edge {
        Edge::North => ResizeDirection::North,
        Edge::South => ResizeDirection::South,
        Edge::East => ResizeDirection::East,
        Edge::West => ResizeDirection::West,
        Edge::NorthEast => ResizeDirection::NorthEast,
        Edge::NorthWest => ResizeDirection::NorthWest,
        Edge::SouthEast => ResizeDirection::SouthEast,
        Edge::SouthWest => ResizeDirection::SouthWest,
    }
}
