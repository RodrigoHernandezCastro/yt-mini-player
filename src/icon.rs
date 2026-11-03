//! The app icon as RGBA, generated from `assets/` (the executable's `.ico`
//! is embedded separately, in `build.rs`).

#[cfg(windows)]
const ICON_32: &[u8] = include_bytes!("../assets/icon-32.rgba");
const ICON_64: &[u8] = include_bytes!("../assets/icon-64.rgba");

/// Icon of the window and the taskbar.
pub fn window_icon() -> Option<tao::window::Icon> {
    tao::window::Icon::from_rgba(ICON_64.to_vec(), 64, 64).ok()
}

/// System tray icon (Windows scales it by DPI).
#[cfg(windows)]
pub fn tray_icon() -> Option<tray_icon::Icon> {
    tray_icon::Icon::from_rgba(ICON_32.to_vec(), 32, 32).ok()
}
