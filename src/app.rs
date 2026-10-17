//! Estado de la aplicación y ejecución de órdenes.
//!
//! Todas las acciones llegan aquí como [`Command`], vengan de la página
//! (barra lateral, atajos locales), de la bandeja o de un atajo global.
//!
//! Con la app bloqueada solo pasan las órdenes de la lista
//! `allowed_while_locked`; el resto se descarta.

use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use serde::Serialize;
use tao::{
    dpi::{LogicalSize, PhysicalPosition},
    event_loop::EventLoopProxy,
    window::{Fullscreen, ResizeDirection, Window},
};
use wry::{WebContext, WebView};

use crate::{
    config::{self, ExtensionIds, LockMode, Paths, Settings, Store},
    extensions::{self, ExtensionEvent, ExtensionHost, ExtensionStatus, LocalExtension},
    hotkeys::{self, Hotkeys, ShortcutInfo},
    home::SharedHomeState,
    ipc::{Command, Edge, UserEvent, MIN_OPACITY},
    lifecycle::Lifecycle,
    links,
    lock,
    lockview::{LockView, Screen},
    media::{MediaAction, MediaState},
    mini::{self, Mini},
    platform,
    security::{self, LegacyEfs},
    services::{self, Agent, Service},
    topmost,
    tray::{Tray, TrayState},
    user_agent::Agents,
    webview::{self as app_webview, Keys},
    window as app_window,
};

/// Opacidad máxima en modo fantasma: así se nota que está activo.
const GHOST_MAX_OPACITY: u8 = 70;
/// Separación de los bordes al colocar en una esquina (px lógicos).
const CORNER_MARGIN: f64 = 12.0;
/// Con el bloqueo activo, se bloquea sola tras este tiempo oculta.
const LOCK_AFTER_HIDDEN: Duration = Duration::from_secs(10 * 60);
/// Como mucho una pestaña nueva en el navegador cada tanto (contra spam).
const EXTERNAL_INTERVAL: Duration = Duration::from_secs(2);

/// Para qué está abierta la pantalla de bloqueo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Goal {
    Unlock,
    /// Cambiar el modo de bloqueo a este.
    Change(LockMode),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    /// Verificar con el método actual.
    Verify,
    /// Elegir un PIN nuevo.
    NewPin,
    /// Probar Windows Hello antes de activarlo.
    ConfirmHello,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Flow {
    goal: Goal,
    stage: Stage,
}

pub struct App {
    window: Window,
    webview: WebView,
    web_context: WebContext,
    proxy: EventLoopProxy<UserEvent>,
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

    agents: Agents,
    /// User-Agent con el que se hizo la última navegación.
    current_agent: Agent,
    home_state: SharedHomeState,
    /// ¿Corre con UIAccess (modo de superposición máxima)?
    overlay: bool,

    /// Lo que suena ahora (informe de media.js).
    media: MediaState,
    /// Barra de música (dentro de la barra de tareas).
    mini: Mini,
    /// El usuario cerró la barra de este servicio de música (clic derecho):
    /// no vuelve sola hasta cambiar de sitio.
    bar_dismissed: bool,
    /// Para ignorar el clic suelto que Windows envía tras un doble clic.
    last_double_click: Option<Instant>,

    // Seguridad
    store: Store,
    keys: Keys,
    legacy_efs: LegacyEfs,
    lock_view: LockView,
    locked: bool,
    flow: Option<Flow>,
    hello_busy: bool,
    pin_busy: bool,
    lock_error: Option<String>,
    hidden_since: Option<Instant>,
    /// Ids de extensiones instaladas, compartidos con el filtro de navegación.
    allowed_extensions: Arc<Mutex<Vec<String>>>,
    last_external: Option<Instant>,
    /// Hay extensiones por aprobar que esperan al desbloqueo (nadie puede
    /// aprobar nada con la app bloqueada).
    approvals_pending: bool,
    /// PIN olvidado sin Windows Hello: se quita cuando el borrado de las
    /// sesiones termine bien (no antes).
    pin_reset_pending: bool,
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
    last_service: Option<&'a str>,
    start_with_last: bool,
    overlay: bool,
}

pub struct Startup {
    pub window: Window,
    pub webview: WebView,
    pub web_context: WebContext,
    pub paths: Paths,
    pub settings: Settings,
    pub proxy: EventLoopProxy<UserEvent>,
    pub start_hidden: bool,
    pub agents: Agents,
    pub start_agent: Agent,
    pub home_state: SharedHomeState,
    pub mini_window: Window,
    pub store: Store,
    pub keys: Keys,
    /// Qué pasó con el cifrado EFS de la 0.6.0 al arrancar.
    pub legacy_efs: LegacyEfs,
    /// La clave de la bóveda no se pudo abrir y se creó otra.
    pub vault_reset: bool,
    pub extension_ids: ExtensionIds,
    pub allowed_extensions: Arc<Mutex<Vec<String>>>,
}

/// Tras un doble clic en la bandeja llega otro clic suelto: se ignora.
const DOUBLE_CLICK_GRACE: Duration = Duration::from_millis(600);

impl App {
    pub fn new(s: Startup) -> Self {
        let autostart = platform::autostart_enabled();
        let hotkeys = hotkeys::register(s.proxy.clone());
        let ext_host = ExtensionHost::new(&s.webview, s.proxy.clone());
        let extension_ids = s.extension_ids;

        let tray_state = TrayState {
            visible: !s.start_hidden,
            pinned: s.settings.always_on_top,
            taskbar_bar: s.settings.taskbar_bar,
            lock: s.settings.lock,
            autostart,
            tooltip: platform::APP_NAME.to_owned(),
            ..TrayState::default()
        };
        let bar_shift = s.settings.bar_shift;
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
            web_context: s.web_context,
            proxy: s.proxy.clone(),
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
            agents: s.agents,
            current_agent: s.start_agent,
            home_state: s.home_state,
            overlay: platform::has_uiaccess(),
            media: MediaState::default(),
            mini: Mini::new(s.mini_window, bar_shift),
            bar_dismissed: false,
            last_double_click: None,
            store: s.store,
            keys: s.keys,
            legacy_efs: s.legacy_efs,
            lock_view: LockView::default(),
            locked: false,
            flow: None,
            hello_busy: false,
            pin_busy: false,
            lock_error: None,
            hidden_since: None,
            allowed_extensions: s.allowed_extensions,
            last_external: None,
            approvals_pending: false,
            pin_reset_pending: false,
        };

        // "Siempre visible" se aplica después de crear el webview: con
        // UIAccess, WebView2 no se inicializa si la ventana ya es topmost
        // (WebView2Feedback #486).
        app.window.set_always_on_top(app.settings.always_on_top);

        // Primero el bloqueo: así nadie aprueba extensiones sin desbloquear.
        if app.settings.lock != LockMode::Off {
            app.lock_now();
        }
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
        // Mantiene el reproductor sobre juegos en pantalla completa sin bordes.
        topmost::start(app.hwnd());
        app.update_guard();

        // Win+L o suspender: se bloquea (si el bloqueo está activado).
        let lock_proxy = app.proxy.clone();
        security::on_session_lock(app.hwnd(), move || {
            let _ = lock_proxy.send_event(UserEvent::Command(Command::LockNow));
        });
        if app.hidden {
            app.hidden_since = Some(Instant::now());
        }
        if s.vault_reset {
            app.toast("Los ajustes cifrados no se pudieron abrir (¿perfil de Windows restablecido o chip de seguridad borrado?) y se empezó de cero.");
        }
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
        self.lock_view.resize(&self.window);
    }

    /// Lo único que se atiende con la app bloqueada.
    fn allowed_while_locked(command: &Command) -> bool {
        matches!(
            command,
            Command::Show
                | Command::Hide
                | Command::ToggleVisible
                | Command::Quit
                | Command::TrayClick(..)
                | Command::TrayDoubleClick
                | Command::Drag
                | Command::Resize(_)
                | Command::Ready
                | Command::MediaReport(_)
                | Command::Navigate(_)
                | Command::Blocked(_)
                | Command::LockNow
                | Command::LockReady
                | Command::LockHello
                | Command::LockPin(_)
                | Command::LockNewPin(_)
                | Command::LockCancel
                | Command::LockForgot
                | Command::ForgotResult(_)
                | Command::BrowsingDataCleared(_)
                | Command::HelloResult(_)
                | Command::PinChecked(_)
                | Command::PinHashed(_)
                | Command::SecurityStatus
                | Command::MiniHide
                | Command::MiniHover(_)
                | Command::MiniReady
        )
    }

    /// Devuelve `false` si la aplicación debe terminar.
    pub fn handle(&mut self, command: Command) -> bool {
        if self.locked && !Self::allowed_while_locked(&command) {
            return true;
        }
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
            Command::Home => {
                self.navigate(services::HOME_URL, Agent::Chrome);
            }
            Command::OpenService(id) => match services::by_id(&id) {
                Some(service) => self.open_service(service),
                None => self.toast("Ese servicio no existe"),
            },
            Command::OpenUrl(text) => match services::parse_user_url(&text) {
                Some(url) => self.open_url(url),
                None => self.toast("Esa dirección no es válida: escribe algo como plex.tv/web"),
            },
            Command::SetStartWithLast(on) => {
                self.settings.start_with_last = on;
                self.save_settings();
                self.sync_home_state();
                self.push_state();
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
            Command::OpenClipboard => {
                let text = platform::read_clipboard_text().unwrap_or_default();
                // YouTube se normaliza (youtu.be, ids sueltos); el resto de
                // servicios del catálogo se aceptan tal cual.
                let url = links::normalize(&text).and_then(|u| url::Url::parse(&u).ok()).or_else(|| {
                    services::parse_user_url(&text).filter(|u| services::for_url(u).is_some())
                });
                match url {
                    Some(url) => {
                        if self.hidden {
                            self.show();
                        }
                        self.open_url(url);
                    }
                    None => self.toast("El portapapeles no contiene un enlace de un servicio de la lista"),
                }
            }
            Command::Media(action) => self.media_action(action),
            Command::MediaReport(state) => {
                let changed_for_tray = self.media.title != state.title
                    || self.media.playing != state.playing
                    || self.media.has_media != state.has_media
                    || self.media.can_next != state.can_next
                    || self.media.can_previous != state.can_previous;
                if self.media.site != state.site {
                    self.bar_dismissed = false;
                }
                self.media = *state;
                if changed_for_tray {
                    self.update_tray();
                }
                self.refresh_bar();
            }
            Command::TrayClick(x, y) => {
                if self.last_double_click.is_some_and(|t| t.elapsed() < DOUBLE_CLICK_GRACE) {
                    return true;
                }
                if self.locked && (self.hidden || self.window.is_minimized()) {
                    // Bloqueada: el clic muestra la pantalla de bloqueo.
                    self.show();
                } else if self.hidden || self.window.is_minimized() {
                    // Oculto: el clic abre (o cierra) la barra de controles.
                    if self.mini.is_visible() {
                        self.close_bar();
                    } else {
                        self.open_bar(mini::STAY_AFTER_CLICK, Some((x, y)));
                    }
                } else {
                    self.hide();
                }
            }
            Command::TrayDoubleClick => {
                self.last_double_click = Some(Instant::now());
                self.show();
            }
            Command::MiniShow => self.open_bar(mini::STAY_AFTER_CLICK, None),
            Command::MiniHide => self.close_bar(),
            Command::MiniHover(hovered) => {
                self.mini.set_hovered(hovered);
                self.mini.update(Instant::now());
                self.sync_watch();
            }
            Command::MiniReady => self.mini.push(),
            Command::MiniDrag(phase) => {
                if let Some(shift) = self.mini.drag(phase) {
                    self.settings.bar_shift = shift;
                    self.save_settings();
                }
            }
            Command::ToggleTaskbarBar => {
                self.settings.taskbar_bar = !self.settings.taskbar_bar;
                self.bar_dismissed = false;
                self.save_settings();
                self.toast(if self.settings.taskbar_bar {
                    "Barra de música en la barra de tareas: activada"
                } else {
                    "Barra de música en la barra de tareas: desactivada"
                });
                self.update_tray();
                self.refresh_bar();
            }
            Command::ExtensionsFolder => platform::open_path(&self.paths.extensions),
            Command::ExtensionsReload => {
                // "Recargar" vuelve a preguntar por las que se rechazaron.
                self.settings.rejected_extensions.clear();
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
            Command::Navigate(url) => match url::Url::parse(&url) {
                Ok(url) => self.open_url(url),
                Err(_) => self.toast("Dirección no válida"),
            },
            Command::OpenExternal(url) => {
                if self.last_external.is_some_and(|t| t.elapsed() < EXTERNAL_INTERVAL) {
                    return true;
                }
                self.last_external = Some(Instant::now());
                security::open_in_browser(&url);
                let host = url::Url::parse(&url).ok().and_then(|u| u.host_str().map(str::to_owned)).unwrap_or_default();
                self.toast(&format!("Abierto en tu navegador: {host}"));
            }
            Command::Blocked(reason) => self.toast(&format!("Bloqueado: {reason}")),
            Command::LockNow => self.lock_now(),
            Command::SetLock(mode) => self.begin_change(mode),
            Command::SignOutAll => self.sign_out_all(),
            Command::SecurityStatus => self.show_security_status(),
            Command::LockReady => self.push_lock_screen(),
            Command::LockHello => self.start_hello(),
            Command::LockPin(pin) => self.check_pin(zeroize::Zeroizing::new(pin)),
            Command::LockNewPin(pin) => self.set_new_pin(zeroize::Zeroizing::new(pin)),
            Command::LockCancel => {
                if self.flow.is_some_and(|f| f.goal != Goal::Unlock) {
                    self.end_flow();
                }
            }
            Command::LockForgot => self.forgot_pin(),
            Command::ForgotResult(result) => {
                self.hello_busy = false;
                match result {
                    Ok(true) => {
                        self.unlock();
                        self.apply_lock(LockMode::Off, None);
                        self.toast("PIN quitado tras verificarte con Windows Hello. Puedes crear otro en la bandeja → Seguridad.");
                    }
                    Ok(false) => self.lock_failed("Windows Hello no confirmó tu identidad."),
                    Err(_) => self.reset_forgotten_pin(),
                }
            }
            Command::HelloResult(result) => {
                self.hello_busy = false;
                match result {
                    Ok(true) => self.verified(),
                    Ok(false) => self.lock_failed("Windows Hello no confirmó tu identidad. Vuelve a intentarlo."),
                    Err(err) => self.lock_failed(&format!("Windows Hello {err}.")),
                }
            }
            Command::PinChecked(ok) => {
                self.pin_busy = false;
                if ok {
                    self.settings.lock_failures = 0;
                    self.settings.lock_blocked_until = 0;
                    self.save_settings();
                    self.verified();
                } else {
                    self.settings.lock_failures = self.settings.lock_failures.saturating_add(1);
                    let wait = lock::delay_after(self.settings.lock_failures);
                    self.settings.lock_blocked_until = if wait > 0 { lock::unix_now() + wait } else { 0 };
                    self.save_settings();
                    self.lock_failed("PIN incorrecto");
                }
            }
            Command::PinHashed(result) => {
                self.pin_busy = false;
                match result {
                    Ok(hash) => self.apply_lock(LockMode::Pin, Some(hash)),
                    Err(err) => self.lock_failed(&format!("No se pudo guardar el PIN ({err})")),
                }
            }
            Command::BrowsingDataCleared(ok) => {
                if std::mem::take(&mut self.pin_reset_pending) {
                    if ok {
                        self.unlock();
                        self.apply_lock(LockMode::Off, None);
                    } else {
                        self.lock_failed("No se pudieron borrar las sesiones, así que el PIN sigue puesto.");
                        return true;
                    }
                }
                if ok {
                    self.navigate(services::HOME_URL, Agent::Chrome);
                    self.toast("Sesiones cerradas y datos de navegación borrados en todos los servicios");
                } else {
                    self.toast("No se pudieron borrar los datos (actualiza Microsoft Edge WebView2)");
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
    // Servicios
    // ------------------------------------------------------------------

    /// Navega con el User-Agent indicado (solo se cambia si hace falta).
    fn navigate(&mut self, url: &str, agent: Agent) {
        if self.fullscreen {
            self.set_fullscreen(false);
        }
        if agent != self.current_agent {
            if let Some(ua) = self.agents.for_agent(agent) {
                app_webview::set_user_agent(&self.webview, ua);
            }
            self.current_agent = agent;
        }
        if let Err(err) = self.webview.load_url(url) {
            self.toast(&format!("No se pudo abrir la página: {err}"));
        }
        // Lo que sonaba se va con la página; la nueva lo informará (la
        // pantalla de inicio no informa nada).
        self.media = MediaState::default();
        self.update_tray();
        self.refresh_bar();
    }

    fn open_service(&mut self, service: &'static Service) {
        self.remember_service(service);
        self.navigate(service.url, service.agent);
    }

    /// Una dirección concreta: si es de un servicio, con su User-Agent y
    /// recordándolo; si no, como Edge, que es lo que el motor es.
    fn open_url(&mut self, url: url::Url) {
        let agent = match services::for_url(&url) {
            Some(service) => {
                self.remember_service(service);
                service.agent
            }
            None => Agent::Edge,
        };
        self.navigate(url.as_str(), agent);
    }

    fn remember_service(&mut self, service: &Service) {
        if self.settings.last_service.as_deref() != Some(service.id) {
            self.settings.last_service = Some(service.id.to_owned());
            self.save_settings();
            self.sync_home_state();
        }
    }

    fn sync_home_state(&self) {
        if let Ok(mut state) = self.home_state.lock() {
            state.last_service = self.settings.last_service.clone();
            state.start_with_last = self.settings.start_with_last;
        }
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
        // La barra pedida a petición se va; la de un servicio de música sigue.
        self.mini.end_transient();
        self.refresh_bar();
        self.hidden = false;
        self.window.set_visible(true);
        if self.window.is_minimized() {
            self.window.set_minimized(false);
        }
        self.window.set_focus();
        self.apply_effects();
        self.update_lifecycle();
        self.update_tray();
        self.hidden_since = None;
        if self.locked {
            // Al traerla al frente bloqueada, se pide Windows Hello directamente.
            self.show_lock_view();
            if self.lock_method() == LockMode::Hello {
                self.start_hello();
            }
        }
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
        self.hidden_since = Some(Instant::now());
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
        let approved = self.approve_extensions(&scan.extensions);
        self.extensions = scan
            .extensions
            .iter()
            .map(|ext| {
                let mut status = ExtensionStatus::pending(ext);
                if !approved.iter().any(|a| a.folder == ext.folder) {
                    status.pending = false;
                    status.error = Some("No aprobada: no se carga. Pulsa «Recargar» para volver a decidir.".into());
                }
                status
            })
            .collect();
        self.extension_problems = scan.problems;
        self.local_extensions = scan.extensions.clone();
        if let Some(error) = &self.extensions_error {
            for status in &mut self.extensions {
                status.pending = false;
                status.error = Some(error.clone());
            }
        } else {
            // Las no aprobadas quedan fuera (y se desinstalan si estaban).
            self.ext_host.sync(approved, self.extension_ids.clone());
        }
        self.push_state();
    }

    /// Solo se cargan extensiones aprobadas por el usuario con su huella
    /// exacta. Las nuevas o modificadas se preguntan en un diálogo del
    /// sistema (que la página no puede dibujar ni pulsar).
    fn approve_extensions(&mut self, found: &[LocalExtension]) -> Vec<LocalExtension> {
        let mut changed = false;
        // Se olvidan las carpetas que ya no existen.
        let folders: Vec<&str> = found.iter().map(|e| e.folder.as_str()).collect();
        for list in [&mut self.settings.approved_extensions, &mut self.settings.rejected_extensions] {
            let before = list.len();
            list.retain(|folder, _| folders.contains(&folder.as_str()));
            changed |= list.len() != before;
        }
        let mut approved = Vec::new();
        for ext in found {
            if self.settings.approved_extensions.get(&ext.folder) == Some(&ext.fingerprint) {
                approved.push(ext.clone());
                continue;
            }
            if self.settings.rejected_extensions.get(&ext.folder) == Some(&ext.fingerprint) {
                continue;
            }
            if self.locked {
                // Se pregunta al desbloquear.
                self.approvals_pending = true;
                continue;
            }
            let was_approved = self.settings.approved_extensions.contains_key(&ext.folder);
            if security::confirm(self.hwnd(), "Extensión nueva o modificada", &extension_question(ext, was_approved)) {
                self.settings.rejected_extensions.remove(&ext.folder);
                self.settings.approved_extensions.insert(ext.folder.clone(), ext.fingerprint.clone());
                approved.push(ext.clone());
            } else {
                self.settings.approved_extensions.remove(&ext.folder);
                self.settings.rejected_extensions.insert(ext.folder.clone(), ext.fingerprint.clone());
            }
            changed = true;
        }
        if changed {
            self.save_settings();
        }
        approved
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
        if let Err(err) = self.settings.save(&self.store, &self.paths.settings) {
            eprintln!("no se pudieron guardar los ajustes: {err}");
        }
    }

    fn save_extension_ids(&self) {
        if let Err(err) = config::save_extension_ids(&self.store, &self.paths.extension_ids, &self.extension_ids) {
            eprintln!("no se pudo guardar extensions.enc: {err}");
        }
        if let Ok(mut allowed) = self.allowed_extensions.lock() {
            *allowed = self.extension_ids.values().cloned().collect();
        }
    }

    /// tao termina el proceso con `exit()` sin ejecutar destructores, así
    /// que aquí se guarda todo y se quita el icono de la bandeja (si no,
    /// quedaría un icono fantasma hasta pasar el ratón por encima).
    fn save_on_exit(&mut self) {
        if !self.fullscreen {
            if let Some(geometry) = app_window::current_geometry(&self.window) {
                if let Err(err) = geometry.save(&self.store, &self.paths.window_state) {
                    eprintln!("no se pudo guardar la geometría: {err}");
                }
            }
        }
        self.save_settings();
        topmost::set_active(false);
        self.mini.close();
        self.tray = None;
    }

    /// El vigilante solo trabaja con "siempre visible" activo y la ventana
    /// a la vista.
    fn update_guard(&self) {
        topmost::set_active(self.settings.always_on_top && !self.hidden);
    }

    fn update_tray(&self) {
        self.update_guard();
        if let Some(tray) = &self.tray {
            // Bloqueada, la bandeja no cuenta qué suena.
            let media = if self.locked { MediaState::default() } else { self.media.clone() };
            tray.update(&TrayState {
                visible: !self.hidden,
                pinned: self.settings.always_on_top,
                taskbar_bar: self.settings.taskbar_bar,
                lock: self.settings.lock,
                locked: self.locked,
                ghost: self.ghost,
                autostart: self.autostart,
                has_media: media.has_media,
                playing: media.playing,
                can_next: media.can_next,
                can_previous: media.can_previous,
                now_playing: crate::media::truncate(&media.title, crate::tray::MENU_TITLE_CHARS),
                tooltip: media.tooltip(),
            });
        }
    }

    // ------------------------------------------------------------------
    // Reproducción y barra de música
    // ------------------------------------------------------------------

    fn media_action(&mut self, action: MediaAction) {
        let _ = self.webview.evaluate_script(&action.script());
        if self.hidden || self.window.is_minimized() {
            // Oculto: la barra aparece unos segundos como aviso, sin quitarle
            // el foco a nada.
            self.open_bar(mini::STAY_AFTER_HOTKEY, None);
            return;
        }
        // Visible: un aviso breve para lo que no se ve a simple vista.
        let percent = |delta: i16| (i16::from(self.media.volume_percent()) + delta).clamp(0, 100);
        let message = match action {
            MediaAction::VolumeBy(delta) => Some(format!("Volumen {} %", percent(i16::from(delta)))),
            MediaAction::ToggleMute if self.media.muted => Some("Sonido activado".to_owned()),
            MediaAction::ToggleMute => Some("Silenciado".to_owned()),
            MediaAction::SeekBy(s) if s > 0 => Some(format!("+{s} s")),
            MediaAction::SeekBy(s) => Some(format!("−{} s", s.unsigned_abs())),
            _ => None,
        };
        if let Some(message) = message {
            if self.media.has_media {
                self.toast(&message);
            }
        }
    }

    /// Crea el webview de la barra si aún no existe.
    fn prepare_bar(&mut self) -> bool {
        self.mini.ensure_webview(&self.webview, &mut self.web_context, &self.proxy, &self.home_state)
    }

    /// Recalcula la barra: fija si hay un servicio de música abierto (y no
    /// se cerró a mano), y dónde y si se ve.
    fn refresh_bar(&mut self) {
        let source = services::music_source(&self.media.site);
        // Bloqueada: nada de barra (mostraría lo que suena).
        if self.locked {
            self.mini.set_pinned(false);
            self.mini.close();
            self.sync_watch();
            return;
        }
        let mut pinned = self.settings.taskbar_bar && source.is_some() && !self.bar_dismissed;
        if pinned && !self.prepare_bar() {
            pinned = false;
        }
        self.mini.set_pinned(pinned);
        // Fuera del catálogo y de los sitios de música conocidos, la barra
        // muestra el dominio: así no puede hacerse pasar por otra cosa.
        let known = source.is_some()
            || url::Url::parse(&format!("https://{}/", self.media.site)).ok().and_then(|u| services::for_url(&u)).is_some();
        let origin = (!known && !self.media.site.is_empty()).then(|| self.media.site.clone());
        self.mini.set_media(&self.media, source, origin);
        self.mini.update(Instant::now());
        self.sync_watch();
    }

    /// La muestra a petición unos segundos (clic en la bandeja, atajos).
    fn open_bar(&mut self, stay: Duration, anchor: Option<(i32, i32)>) {
        if !self.prepare_bar() {
            self.toast("No se pudo abrir la barra de controles");
            return;
        }
        self.bar_dismissed = false;
        self.mini.show_for(stay, anchor);
        self.refresh_bar();
    }

    /// Clic derecho en la barra (o clic en la bandeja con la barra abierta).
    fn close_bar(&mut self) {
        if self.mini.is_pinned() {
            self.bar_dismissed = true;
        }
        self.mini.close();
        self.refresh_bar();
    }

    /// Mientras la barra se ve, la página informa la posición cada segundo.
    /// El informe dice si la página está en ese modo: si no coincide (página
    /// nueva, recarga), se le vuelve a pedir.
    fn sync_watch(&mut self) {
        let wanted = self.mini.is_visible();
        if self.media.watching != wanted {
            self.media.watching = wanted;
            let script = format!("window.__ytmpMediaWatch && window.__ytmpMediaWatch({wanted})");
            let _ = self.webview.evaluate_script(&script);
        }
    }

    /// Próximo momento en que hay algo que hacer (revisar la barra).
    pub fn deadline(&self) -> Option<Instant> {
        let auto_lock = self.auto_lock_at();
        match (self.mini.deadline(), auto_lock) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        }
    }

    /// Cuándo se bloquea sola por llevar oculta mucho tiempo.
    fn auto_lock_at(&self) -> Option<Instant> {
        let enabled = self.settings.lock != LockMode::Off && !self.locked;
        self.hidden_since.filter(|_| enabled).map(|since| since + LOCK_AFTER_HIDDEN)
    }

    pub fn on_timer(&mut self) {
        let now = Instant::now();
        self.mini.on_timer(now);
        self.sync_watch();
        if self.auto_lock_at().is_some_and(|at| now >= at) {
            self.lock_now();
        }
    }

    pub fn main_window_id(&self) -> tao::window::WindowId {
        self.window.id()
    }

    pub fn mini_window_id(&self) -> tao::window::WindowId {
        self.mini.id()
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
            last_service: self.settings.last_service.as_deref(),
            start_with_last: self.settings.start_with_last,
            overlay: self.overlay,
        };
        if let Ok(json) = serde_json::to_string(&state) {
            let _ = self
                .webview
                .evaluate_script(&format!("window.__ytmpSync && window.__ytmpSync(\"{}\", {json})", self.keys.host));
        }
    }

    fn toast(&self, message: &str) {
        if let Ok(json) = serde_json::to_string(message) {
            let _ = self
                .webview
                .evaluate_script(&format!("window.__ytmpToast && window.__ytmpToast(\"{}\", {json})", self.keys.host));
        }
    }

    // ------------------------------------------------------------------
    // Bloqueo de la app
    // ------------------------------------------------------------------

    /// Método con el que se verifica ahora (en el paso actual).
    fn lock_method(&self) -> LockMode {
        match self.flow.map(|f| f.stage) {
            Some(Stage::NewPin) => LockMode::Pin,
            Some(Stage::ConfirmHello) => LockMode::Hello,
            _ => self.settings.lock,
        }
    }

    fn lock_now(&mut self) {
        if self.settings.lock == LockMode::Off || self.locked {
            return;
        }
        self.locked = true;
        self.flow = Some(Flow { goal: Goal::Unlock, stage: Stage::Verify });
        self.lock_error = None;
        // Que no siga sonando ni se vea lo que había.
        let _ = self.webview.evaluate_script(&MediaAction::Pause.script());
        if self.ghost {
            self.ghost = false;
            self.apply_effects();
        }
        if self.fullscreen {
            self.set_fullscreen(false);
        }
        let _ = self.webview.set_visible(false);
        self.refresh_bar();
        if !self.show_lock_view() {
            // Sin pantalla de bloqueo no habría forma de desbloquear: antes
            // que dejar la app inutilizable, no se bloquea y se avisa.
            self.locked = false;
            self.flow = None;
            let _ = self.webview.set_visible(true);
            self.toast("No se pudo mostrar la pantalla de bloqueo; la app sigue desbloqueada.");
        }
        self.update_tray();
    }

    fn show_lock_view(&mut self) -> bool {
        let ready = self.lock_view.ensure(&self.window, &self.webview, &mut self.web_context, &self.proxy, &self.home_state);
        if ready {
            let _ = self.webview.set_visible(false);
            self.lock_view.show(&self.window);
            self.push_lock_screen();
        }
        ready
    }

    /// PIN olvidado: se puede quitar verificándose con Windows Hello (la
    /// identidad la confirma Windows). Si Windows Hello no está, la única
    /// salida es borrar las sesiones: quien no sepa el PIN no se lleva nada.
    fn forgot_pin(&mut self) {
        if !self.locked || self.settings.lock != LockMode::Pin || self.hello_busy || self.pin_busy {
            return;
        }
        self.hello_busy = true;
        self.lock_error = None;
        self.push_lock_screen();
        let hwnd = self.hwnd();
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let result = lock::hello_verify(hwnd, "Verifícate para quitar el PIN de YT Mini Player");
            let _ = proxy.send_event(UserEvent::Command(Command::ForgotResult(result)));
        });
    }

    fn reset_forgotten_pin(&mut self) {
        let confirmed = security::confirm(
            self.hwnd(),
            "PIN olvidado",
            "Windows Hello no está disponible para verificarte.\n\n\
             Para quitar el PIN hay que cerrar la sesión en todos los servicios (se borran cookies, \
             datos de los sitios, caché e historial). Así nadie que no sepa el PIN se lleva tus cuentas.\n\n\
             ¿Borrar las sesiones y quitar el PIN?",
        );
        if !confirmed {
            self.push_lock_screen();
            return;
        }
        // El PIN se quita solo cuando el borrado termine bien.
        self.pin_reset_pending = true;
        let proxy = self.proxy.clone();
        app_webview::clear_browsing_data(&self.webview, move |ok| {
            let _ = proxy.send_event(UserEvent::Command(Command::BrowsingDataCleared(ok)));
        });
    }

    fn push_lock_screen(&self) {
        let Some(flow) = self.flow else { return };
        let method = self.lock_method();
        let stage = match (flow.goal, flow.stage) {
            (_, Stage::NewPin) => "new-pin",
            (Goal::Unlock, _) => "unlock",
            _ => "verify",
        };
        let wait = if method == LockMode::Pin && flow.stage == Stage::Verify {
            lock::remaining(self.settings.lock_blocked_until, lock::unix_now())
        } else {
            0
        };
        self.lock_view.push(&Screen {
            stage,
            method: if method == LockMode::Pin { "pin" } else { "hello" },
            busy: self.hello_busy || self.pin_busy,
            error: self.lock_error.clone(),
            wait,
            can_cancel: flow.goal != Goal::Unlock,
        });
    }

    /// Cambiar el modo de bloqueo (desde la bandeja). Elegir el modo que ya
    /// está lo desactiva. Si hay bloqueo, primero hay que verificarse.
    fn begin_change(&mut self, chosen: LockMode) {
        let target = if chosen == self.settings.lock { LockMode::Off } else { chosen };
        if target == LockMode::Off && self.settings.lock == LockMode::Off {
            self.update_tray();
            return;
        }
        let stage = if self.settings.lock != LockMode::Off {
            Stage::Verify
        } else if target == LockMode::Pin {
            Stage::NewPin
        } else {
            Stage::ConfirmHello
        };
        self.flow = Some(Flow { goal: Goal::Change(target), stage });
        self.lock_error = None;
        if self.hidden || self.window.is_minimized() {
            self.show();
        }
        self.show_lock_view();
        if self.lock_method() == LockMode::Hello {
            self.start_hello();
        }
        self.update_tray();
    }

    fn start_hello(&mut self) {
        let Some(flow) = self.flow else { return };
        if self.hello_busy || self.lock_method() != LockMode::Hello || flow.stage == Stage::NewPin {
            return;
        }
        self.hello_busy = true;
        self.lock_error = None;
        self.push_lock_screen();
        let hwnd = self.hwnd();
        let proxy = self.proxy.clone();
        let message = if flow.goal == Goal::Unlock {
            "Desbloquear YT Mini Player"
        } else {
            "Confirma que eres tú para cambiar el bloqueo de YT Mini Player"
        };
        std::thread::spawn(move || {
            let result = lock::hello_verify(hwnd, message);
            let _ = proxy.send_event(UserEvent::Command(Command::HelloResult(result)));
        });
    }

    fn check_pin(&mut self, pin: zeroize::Zeroizing<String>) {
        let verifying = self.flow.is_some_and(|f| f.stage == Stage::Verify) && self.settings.lock == LockMode::Pin;
        if !verifying || self.pin_busy {
            return;
        }
        if lock::remaining(self.settings.lock_blocked_until, lock::unix_now()) > 0 {
            self.push_lock_screen();
            return;
        }
        let Some(hash) = self.settings.pin_hash.clone() else { return };
        self.pin_busy = true;
        self.lock_error = None;
        self.push_lock_screen();
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let ok = lock::verify_pin(&pin, &hash);
            let _ = proxy.send_event(UserEvent::Command(Command::PinChecked(ok)));
        });
    }

    fn set_new_pin(&mut self, pin: zeroize::Zeroizing<String>) {
        if self.flow.map(|f| f.stage) != Some(Stage::NewPin) || self.pin_busy {
            return;
        }
        if let Err(reason) = lock::check_pin(&pin) {
            self.lock_failed(reason);
            return;
        }
        self.pin_busy = true;
        self.lock_error = None;
        self.push_lock_screen();
        let proxy = self.proxy.clone();
        std::thread::spawn(move || {
            let result = lock::hash_pin(&pin);
            let _ = proxy.send_event(UserEvent::Command(Command::PinHashed(result)));
        });
    }

    /// Verificación correcta: sigue el paso siguiente.
    fn verified(&mut self) {
        let Some(flow) = self.flow else { return };
        self.lock_error = None;
        match (flow.goal, flow.stage) {
            (Goal::Unlock, _) => self.unlock(),
            (Goal::Change(target), Stage::Verify) => match target {
                LockMode::Off => self.apply_lock(LockMode::Off, None),
                LockMode::Pin => {
                    self.flow = Some(Flow { goal: flow.goal, stage: Stage::NewPin });
                    self.push_lock_screen();
                }
                LockMode::Hello => {
                    self.flow = Some(Flow { goal: flow.goal, stage: Stage::ConfirmHello });
                    self.push_lock_screen();
                    self.start_hello();
                }
            },
            (Goal::Change(_), Stage::ConfirmHello) => self.apply_lock(LockMode::Hello, None),
            (Goal::Change(_), Stage::NewPin) => {}
        }
    }

    fn lock_failed(&mut self, message: &str) {
        self.lock_error = Some(message.to_owned());
        self.push_lock_screen();
    }

    fn apply_lock(&mut self, mode: LockMode, pin_hash: Option<String>) {
        self.settings.lock = mode;
        self.settings.pin_hash = pin_hash;
        self.settings.lock_failures = 0;
        self.settings.lock_blocked_until = 0;
        self.save_settings();
        if !self.locked {
            self.end_flow();
        }
        self.toast(match mode {
            LockMode::Off => "Bloqueo desactivado",
            LockMode::Hello => "Bloqueo con Windows Hello activado. Ctrl+Alt+L para bloquear ahora.",
            LockMode::Pin => "Bloqueo con PIN activado. Ctrl+Alt+L para bloquear ahora.",
        });
    }

    fn end_flow(&mut self) {
        if self.locked {
            return;
        }
        self.flow = None;
        self.lock_error = None;
        self.lock_view.hide();
        let _ = self.webview.set_visible(true);
        let _ = self.webview.focus();
        self.update_tray();
    }

    fn unlock(&mut self) {
        self.locked = false;
        self.end_flow();
        self.hidden_since = if self.hidden { Some(Instant::now()) } else { None };
        self.refresh_bar();
        self.update_tray();
        if std::mem::take(&mut self.approvals_pending) {
            self.reload_extensions();
        }
    }

    // ------------------------------------------------------------------
    // Cuentas y estado de la protección
    // ------------------------------------------------------------------

    /// Cierra la sesión en todos los servicios: borra cookies, almacenamiento
    /// y caché del perfil (tras confirmarlo en un diálogo del sistema).
    fn sign_out_all(&mut self) {
        let confirmed = security::confirm(
            self.hwnd(),
            "Cerrar sesión en todos los servicios",
            "Se borrarán las sesiones (cookies), el almacenamiento de los sitios, la caché y el historial \
             del reproductor. Tendrás que volver a iniciar sesión en YouTube, Netflix y el resto.\n\n\
             Los ajustes de la app y las extensiones se conservan.\n\n¿Continuar?",
        );
        if !confirmed {
            return;
        }
        let proxy = self.proxy.clone();
        app_webview::clear_browsing_data(&self.webview, move |ok| {
            let _ = proxy.send_event(UserEvent::Command(Command::BrowsingDataCleared(ok)));
        });
    }

    fn show_security_status(&self) {
        let lock = match self.settings.lock {
            LockMode::Off => "desactivado (bandeja → Ajustes → Seguridad)",
            LockMode::Hello => "Windows Hello; se bloquea al bloquear Windows, al suspender y tras 10 min oculta",
            LockMode::Pin => "PIN (hash Argon2id); se bloquea al bloquear Windows, al suspender y tras 10 min oculta",
        };
        let approved = self.settings.approved_extensions.len();
        let text = format!(
            "Contraseñas: la app no las ve ni las guarda. Inicias sesión en la página de cada servicio y el \
             guardado de contraseñas y el autorrelleno del motor están desactivados.\n\n\
             Sesiones (cookies): cifradas por WebView2 con AES-256-GCM; la clave, protegida por Windows (DPAPI).\n\n\
             Datos de la app (ajustes, PIN, extensiones aprobadas): bóveda AES-256-GCM; su clave está protegida por {key}.\n\n\
             Disco: para cifrar el disco entero (caché incluida), activa BitLocker o el cifrado del dispositivo de Windows.{efs}\n\n\
             Bloqueo: {lock}.\n\n\
             Navegación: SmartScreen activo; descargas, ventanas emergentes y permisos (cámara, micrófono, \
             ubicación…) bloqueados; solo https (http únicamente en tu red local){overlay}.\n\n\
             Extensiones aprobadas: {approved}.",
            key = match crate::vault::key_protection(&self.paths.vault_key) {
                Some(crate::vault::KeyProtection::DeviceAndAccount) => {
                    "el chip de seguridad del equipo (TPM) y tu cuenta de Windows: no se puede abrir en otro equipo"
                }
                _ if crate::vault::hardware_protector().is_some() => {
                    "tu cuenta de Windows (DPAPI); el chip de seguridad (TPM) aún se está preparando o no admite esta función"
                }
                _ => "tu cuenta de Windows (DPAPI); este equipo no tiene chip de seguridad (TPM) disponible",
            },
            efs = self.legacy_efs.describe(),
            overlay = if self.overlay { "; en superposición máxima, solo los servicios del menú" } else { "" },
        );
        security::inform(self.hwnd(), "Protección de tus cuentas y datos", &text);
    }
}

/// Texto del diálogo de aprobación de una extensión.
fn extension_question(ext: &LocalExtension, changed: bool) -> String {
    let what = if changed { "cambió desde que la aprobaste" } else { "es nueva" };
    let mut text = format!("La extensión «{}» {} {what}.\n\n", ext.name, ext.version);
    if ext.all_sites {
        text.push_str(
            "⚠ Puede LEER Y CAMBIAR TODAS LAS PÁGINAS que abras en el reproductor, incluidas las de tus cuentas \
             (YouTube, Netflix…).\n\n",
        );
    }
    if ext.permissions.is_empty() {
        text.push_str("No declara permisos especiales.\n");
    } else {
        text.push_str("Permisos y sitios:\n");
        for permission in ext.permissions.iter().take(12) {
            text.push_str(&format!("  • {permission}\n"));
        }
        if ext.permissions.len() > 12 {
            text.push_str(&format!("  • … y {} más\n", ext.permissions.len() - 12));
        }
    }
    text.push_str(&format!(
        "\nCarpeta: extensions\\{}\nHuella SHA-256: {}…\n\nCárgala solo si viene de una fuente de confianza. ¿Cargarla?",
        ext.folder,
        &ext.fingerprint[..16]
    ));
    text
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
