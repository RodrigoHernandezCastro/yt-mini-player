//! Barra de música: controles multimedia dentro de la barra de tareas.
//!
//! - Aparece sola mientras hay un servicio de música abierto (YouTube Music
//!   y lo que se marque como música en services.rs), esté el reproductor a
//!   la vista u oculto. Con cualquier otro servicio aparece a petición (clic
//!   en el icono de la bandeja, atajos multimedia con el reproductor oculto)
//!   y se esconde sola unos segundos después.
//! - Se coloca dentro de la barra de tareas, a la izquierda del área de
//!   notificación, y se puede arrastrar a lo largo de ella (dock.rs). Con la
//!   barra de tareas vertical, flota junto a la bandeja.
//! - Se esconde si la barra de tareas se esconde (auto-ocultar) o si hay una
//!   app a pantalla completa delante, y vuelve cuando reaparece.
//! - Nunca toma el foco (WS_EX_NOACTIVATE): se puede usar en medio de un
//!   juego sin sacarlo de primer plano. Por eso se muestra y oculta con
//!   Win32 directamente: tao siempre activa la ventana al mostrarla.
//! - Su webview usa el MISMO entorno de WebView2 que el reproductor. Dos
//!   entornos con opciones distintas sobre la misma carpeta de perfil no
//!   pueden convivir: WebView2 rechaza el segundo (por eso, hasta la 0.4.1,
//!   el mini controlador nunca llegaba a aparecer).

use std::time::{Duration, Instant};

use tao::{
    dpi::LogicalSize,
    error::OsError,
    event_loop::{EventLoopProxy, EventLoopWindowTarget},
    window::{Window, WindowBuilder, WindowId},
};
use wry::{http::Request, WebContext, WebView, WebViewBuilder};

use crate::{
    dock::{self, Layout, Placement},
    home::{self, SharedHomeState},
    ipc::{self, DragPhase, UserEvent},
    media::MediaState,
    platform::{self, Rect},
    taskbar,
};

const URL: &str = "https://ytmp.localhost/mini";
/// Borde lavanda (el mismo `--lavender-dim` de mini.html).
const BORDER: (u8, u8, u8) = (0x8f, 0x84, 0xc9);
/// Separación de los bordes cuando flota (px lógicos).
const FLOAT_MARGIN: f64 = 12.0;
/// Cada cuánto se revisa la barra de tareas mientras la barra se ve.
const TICK: Duration = Duration::from_millis(300);
/// Mientras está escondida por una app a pantalla completa (o sin barra de
/// tareas), basta con revisar más despacio.
const SLOW_TICK: Duration = Duration::from_secs(1);

/// Cuánto tiempo sigue visible (si no es un servicio de música).
pub const STAY_AFTER_CLICK: Duration = Duration::from_secs(6);
pub const STAY_AFTER_HOTKEY: Duration = Duration::from_millis(2500);
pub const STAY_AFTER_LEAVE: Duration = Duration::from_millis(1800);

pub struct Mini {
    window: Window,
    webview: Option<WebView>,
    /// Windows 10 no colorea bordes: la página dibuja uno propio.
    css_border: bool,
    /// Hay un servicio de música abierto: se queda mientras tanto.
    pinned: bool,
    /// Mostrada a petición hasta este momento.
    until: Option<Instant>,
    hovered: bool,
    shown: bool,
    docked: bool,
    /// Desplazamiento elegido por el usuario (px lógicos hacia la izquierda).
    shift: i32,
    layout: Option<Layout>,
    /// Arrastre en curso: (x del cursor, x de la ventana) al empezar.
    drag: Option<(i32, i32)>,
    /// Icono de la bandeja, para cuando flota.
    anchor: Option<(i32, i32)>,
    next_tick: Option<Instant>,
    media: MediaState,
    source: Option<&'static str>,
    /// Dominio de la página cuando no es un servicio conocido (se muestra
    /// para que nadie pueda hacerse pasar por otra cosa en la barra).
    origin: Option<String>,
}

pub fn build_window<T: 'static>(target: &EventLoopWindowTarget<T>) -> Result<Window, OsError> {
    let mut builder = WindowBuilder::new()
        .with_title("YT Mini Player: barra de música")
        .with_window_icon(crate::icon::window_icon())
        .with_decorations(false)
        .with_resizable(false)
        .with_visible(false)
        // Topmost se aplica tras crear el webview (UIAccess, ver app.rs).
        .with_always_on_top(false)
        .with_focusable(false)
        .with_inner_size(LogicalSize::new(dock::WIDTH, dock::FLOAT_HEIGHT));
    #[cfg(windows)]
    {
        use tao::platform::windows::WindowBuilderExtWindows;
        builder = builder
            .with_skip_taskbar(true)
            // Sin sombra: tao la consigue dejando un marco invisible alrededor
            // del área cliente, que en esta ventana se veía como un borde
            // blanco de varios píxeles (taskbar::prepare lo quita del todo).
            .with_undecorated_shadow(false)
            .with_drag_and_drop(false)
            .with_window_classname("YtMiniPlayerControls");
    }
    builder.build(target)
}

impl Mini {
    pub fn new(window: Window, shift: i32) -> Self {
        Self {
            window,
            webview: None,
            css_border: false,
            pinned: false,
            until: None,
            hovered: false,
            shown: false,
            docked: false,
            shift,
            layout: None,
            drag: None,
            anchor: None,
            next_tick: None,
            media: MediaState::default(),
            source: None,
            origin: None,
        }
    }

    pub fn id(&self) -> WindowId {
        self.window.id()
    }

    pub fn is_visible(&self) -> bool {
        self.shown
    }

    pub fn is_pinned(&self) -> bool {
        self.pinned
    }

    #[cfg(windows)]
    fn hwnd(&self) -> isize {
        use tao::platform::windows::WindowExtWindows;
        self.window.hwnd()
    }

    #[cfg(not(windows))]
    fn hwnd(&self) -> isize {
        0
    }

    /// Crea el webview la primera vez que hace falta.
    pub fn ensure_webview(
        &mut self,
        main: &WebView,
        web_context: &mut WebContext,
        proxy: &EventLoopProxy<UserEvent>,
        home_state: &SharedHomeState,
    ) -> bool {
        if self.webview.is_some() {
            return true;
        }
        let proxy = proxy.clone();
        let home_state = home_state.clone();
        let mut builder = WebViewBuilder::new_with_web_context(web_context)
            .with_url(URL)
            .with_background_color((16, 21, 42, 255))
            .with_custom_protocol("ytmp".into(), move |_id, request| home::respond(&request, &home_state))
            .with_ipc_handler(move |request: Request<String>| {
                if let Some(command) = ipc::parse_mini(request.body()) {
                    let _ = proxy.send_event(UserEvent::Command(command));
                }
            })
            // Fija la página: ni enlaces, ni ventanas, ni descargas, ni permisos.
            .with_navigation_handler(|url| url == URL)
            .with_new_window_req_handler(|_, _| wry::NewWindowResponse::Deny)
            .with_download_started_handler(|_, _| false)
            .with_permission_handler(|_| wry::PermissionResponse::Deny);
        #[cfg(windows)]
        {
            use wry::{WebViewBuilderExtWindows, WebViewExtWindows};
            builder = builder
                .with_environment(main.environment())
                // Por si acaso: las mismas opciones de entorno que el reproductor.
                .with_browser_extensions_enabled(true)
                .with_additional_browser_args(crate::webview::BROWSER_ARGS)
                .with_https_scheme(true)
                .with_default_context_menus(false);
        }
        #[cfg(not(windows))]
        let _ = main;
        match builder.build(&self.window) {
            Ok(webview) => {
                crate::webview::harden(&webview);
                self.webview = Some(webview);
                // Ahora sí: siempre encima (antes, WebView2 falla con UIAccess).
                // Va antes de `prepare`: tao reescribe los estilos al cambiarlo.
                self.window.set_always_on_top(true);
                let hwnd = self.hwnd();
                taskbar::prepare(hwnd);
                self.css_border = cfg!(windows) && !platform::set_border_color(hwnd, BORDER);
                platform::never_activate_on_click(hwnd);
                taskbar::keep_above(hwnd);
                true
            }
            Err(err) => {
                eprintln!("no se pudo crear la barra de música: {err}");
                false
            }
        }
    }

    pub fn set_pinned(&mut self, pinned: bool) {
        self.pinned = pinned;
    }

    /// La muestra a petición durante `stay` (o más, si ya estaba pedida).
    pub fn show_for(&mut self, stay: Duration, anchor: Option<(i32, i32)>) {
        if anchor.is_some() {
            self.anchor = anchor;
        }
        let at = Instant::now() + stay;
        self.until = Some(self.until.map_or(at, |current| current.max(at)));
    }

    /// Termina lo pedido a petición (la de un servicio de música sigue).
    pub fn end_transient(&mut self) {
        self.until = None;
    }

    pub fn set_hovered(&mut self, hovered: bool) {
        self.hovered = hovered && self.shown;
        if !hovered && self.shown && !self.pinned {
            self.show_for(STAY_AFTER_LEAVE, None);
        }
    }

    fn wanted(&self, now: Instant) -> bool {
        self.pinned || self.until.is_some_and(|at| now < at) || (self.hovered && self.shown) || self.drag.is_some()
    }

    /// Decide si se ve y dónde. Se llama tras cualquier cambio y cada `TICK`
    /// mientras se quiere ver.
    pub fn update(&mut self, now: Instant) {
        if self.until.is_some_and(|at| now >= at) {
            self.until = None;
        }
        if !self.wanted(now) || self.webview.is_none() {
            self.hide_now();
            self.next_tick = None;
            return;
        }
        self.next_tick = Some(now + TICK);
        let hwnd = self.hwnd();
        let Some((taskbar_hwnd, info)) = taskbar::info() else {
            self.next_tick = Some(now + SLOW_TICK);
            self.float();
            return;
        };
        match dock::place(&info, taskbar::fullscreen_in_front(info.monitor)) {
            Placement::Docked(layout) => {
                self.layout = Some(layout);
                // Durante el arrastre manda la posición actual, no la guardada.
                let rect = match (self.drag, taskbar::window_rect(hwnd)) {
                    (Some(_), Some(current)) if self.shown => {
                        let width = current.right - current.left;
                        let (lo, hi) = layout.drag_range(width);
                        let x = current.left.clamp(lo, hi);
                        Rect { left: x, top: layout.y, right: x + width, bottom: layout.y + layout.height }
                    }
                    _ => layout.rect(self.shift),
                };
                self.show_at(rect, true);
                taskbar::raise_if_covered(hwnd, taskbar_hwnd);
                taskbar::set_scanning(true);
            }
            Placement::Hidden => {
                self.next_tick = Some(now + SLOW_TICK);
                self.hide_now();
            }
            Placement::Unavailable => {
                self.next_tick = Some(now + SLOW_TICK);
                self.float();
            }
        }
    }

    /// Sin barra de tareas horizontal: flota junto a la bandeja.
    fn float(&mut self) {
        self.layout = None;
        self.drag = None;
        taskbar::set_scanning(false);
        let area = match self.anchor {
            Some((x, y)) => platform::work_area_at(x, y),
            None => platform::primary_work_area(),
        };
        let scale = self.window.scale_factor();
        let size = ((dock::WIDTH * scale).round() as i32, (dock::FLOAT_HEIGHT * scale).round() as i32);
        let margin = (FLOAT_MARGIN * scale).round() as i32;
        let (x, y) = match area {
            Some(area) => {
                let anchor = self.anchor.unwrap_or((area.right - 1, area.bottom));
                platform::popup_position(area, anchor, size, margin)
            }
            None => (0, 0),
        };
        self.show_at(Rect { left: x, top: y, right: x + size.0, bottom: y + size.1 }, false);
    }

    fn show_at(&mut self, rect: Rect, docked: bool) {
        let hwnd = self.hwnd();
        let current = if self.shown { taskbar::window_rect(hwnd) } else { None };
        if current != Some(rect) {
            taskbar::place(hwnd, rect);
        }
        let changed = !self.shown || docked != self.docked;
        self.shown = true;
        self.docked = docked;
        if changed {
            self.push();
        }
    }

    fn hide_now(&mut self) {
        if self.shown {
            platform::hide_window(self.hwnd());
        }
        taskbar::set_scanning(false);
        self.shown = false;
        self.hovered = false;
        self.drag = None;
    }

    /// Cierre explícito (clic derecho, salir).
    pub fn close(&mut self) {
        self.until = None;
        self.drag = None;
        self.hide_now();
        self.next_tick = None;
    }

    pub fn deadline(&self) -> Option<Instant> {
        self.next_tick
    }

    pub fn on_timer(&mut self, now: Instant) {
        if self.next_tick.is_some_and(|at| now >= at) {
            self.update(now);
        }
    }

    /// Arrastre a lo largo de la barra de tareas. Al soltar devuelve el
    /// nuevo desplazamiento, para guardarlo.
    pub fn drag(&mut self, phase: DragPhase) -> Option<i32> {
        let hwnd = self.hwnd();
        let Some(layout) = self.layout.filter(|_| self.shown && self.docked) else {
            self.drag = None;
            return None;
        };
        match phase {
            DragPhase::Start => {
                if let (Some(cursor), Some(rect)) = (taskbar::cursor_x(), taskbar::window_rect(hwnd)) {
                    self.drag = Some((cursor, rect.left));
                }
                None
            }
            DragPhase::Move => {
                if let (Some((cursor0, x0)), Some(cursor)) = (self.drag, taskbar::cursor_x()) {
                    let width = taskbar::window_rect(hwnd).map_or(layout.full_width, |r| r.right - r.left);
                    let (lo, hi) = layout.drag_range(width);
                    let x = (x0 + cursor - cursor0).clamp(lo, hi);
                    let rect = Rect { left: x, top: layout.y, right: x + width, bottom: layout.y + layout.height };
                    taskbar::place(hwnd, rect);
                }
                None
            }
            DragPhase::End => {
                self.drag.take()?;
                let x = taskbar::window_rect(hwnd).map_or_else(|| layout.rect(self.shift).left, |r| r.left);
                self.shift = layout.shift_for(x);
                Some(self.shift)
            }
        }
    }

    /// Guarda lo que suena y, si se ve, lo manda a la página.
    pub fn set_media(&mut self, media: &MediaState, source: Option<&'static str>, origin: Option<String>) {
        self.media = media.clone();
        self.source = source;
        self.origin = origin;
        if self.shown {
            self.push();
        }
    }

    /// Envía el estado a la página de la barra.
    pub fn push(&self) {
        let Some(webview) = &self.webview else { return };
        #[derive(serde::Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Payload<'a> {
            media: &'a MediaState,
            css_border: bool,
            docked: bool,
            source: Option<&'a str>,
            origin: Option<&'a str>,
        }
        let payload = Payload {
            media: &self.media,
            css_border: self.css_border,
            docked: self.docked,
            source: self.source,
            origin: self.origin.as_deref(),
        };
        if let Ok(json) = serde_json::to_string(&payload) {
            let _ = webview.evaluate_script(&format!("window.__ytmpMini && window.__ytmpMini({json})"));
        }
    }
}
