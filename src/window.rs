//! Ventana nativa: sin decoraciones, siempre visible, 16:9 por defecto.

use tao::{
    dpi::{LogicalSize, PhysicalPosition, PhysicalSize},
    error::OsError,
    event_loop::EventLoopWindowTarget,
    window::{Window, WindowBuilder},
};

#[cfg(windows)]
use tao::platform::windows::WindowBuilderExtWindows;

use crate::config::WindowGeometry;

const TITLE: &str = crate::platform::APP_NAME;
/// 640 × 360 lógicos: 16:9, cómodo como miniatura en 1080p y 1440p.
const DEFAULT_SIZE: (f64, f64) = (640.0, 360.0);
/// Por debajo de 256 × 144 la interfaz de YouTube deja de ser usable.
const MIN_SIZE: (f64, f64) = (256.0, 144.0);

/// Crea la ventana oculta; se muestra cuando el webview ya existe, para
/// evitar un destello blanco durante el arranque.
pub fn build<T: 'static>(
    target: &EventLoopWindowTarget<T>,
    saved: Option<WindowGeometry>,
) -> Result<Window, OsError> {
    let mut builder = WindowBuilder::new()
        .with_title(TITLE)
        .with_window_icon(crate::icon::window_icon())
        .with_decorations(false)
        // Nunca topmost al crearla: App lo aplica tras crear el webview.
        .with_always_on_top(false)
        .with_resizable(true)
        .with_visible(false)
        .with_min_inner_size(LogicalSize::new(MIN_SIZE.0, MIN_SIZE.1))
        .with_inner_size(LogicalSize::new(DEFAULT_SIZE.0, DEFAULT_SIZE.1));

    #[cfg(windows)]
    {
        // No usamos arrastrar-y-soltar de archivos: evita registrar un
        // IDropTarget OLE que no sirve para nada aquí.
        builder = builder
            .with_drag_and_drop(false)
            .with_taskbar_icon(crate::icon::window_icon())
            // Nombre de clase estable: lo usan herramientas como PowerToys.
            .with_window_classname("YtMiniPlayerWindow");
    }

    if let Some(geometry) = saved.filter(|g| is_reachable(target, g)) {
        builder = builder
            .with_position(PhysicalPosition::new(geometry.x, geometry.y))
            .with_inner_size(PhysicalSize::new(geometry.width, geometry.height));
    }

    builder.build(target)
}

/// Geometría actual para guardar al cerrar. Devuelve `None` si la ventana
/// está minimizada o maximizada (Win+↑), porque esas posiciones no son las
/// que el usuario quiere recuperar.
pub fn current_geometry(window: &Window) -> Option<WindowGeometry> {
    if window.is_minimized() || window.is_maximized() {
        return None;
    }
    let position = window.outer_position().ok()?;
    let size = window.inner_size();
    Some(WindowGeometry {
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
    })
}

/// La geometría guardada solo se usa si sigue cayendo en algún monitor
/// conectado (p. ej. no si se desconectó el segundo monitor).
fn is_reachable<T: 'static>(target: &EventLoopWindowTarget<T>, geometry: &WindowGeometry) -> bool {
    target.available_monitors().any(|monitor| {
        let position = monitor.position();
        let size = monitor.size();
        geometry.overlaps((position.x, position.y, size.width, size.height))
    })
}
