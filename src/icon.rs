//! Icono de la aplicación en RGBA, generado desde `assets/` (el `.ico` del
//! ejecutable se incrusta aparte, en `build.rs`).

#[cfg(windows)]
const ICON_32: &[u8] = include_bytes!("../assets/icon-32.rgba");
const ICON_64: &[u8] = include_bytes!("../assets/icon-64.rgba");

/// Icono de la ventana y de la barra de tareas.
pub fn window_icon() -> Option<tao::window::Icon> {
    tao::window::Icon::from_rgba(ICON_64.to_vec(), 64, 64).ok()
}

/// Icono de la bandeja del sistema (Windows lo escala según el DPI).
#[cfg(windows)]
pub fn tray_icon() -> Option<tray_icon::Icon> {
    tray_icon::Icon::from_rgba(ICON_32.to_vec(), 32, 32).ok()
}
