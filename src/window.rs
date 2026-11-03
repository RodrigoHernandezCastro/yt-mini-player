//! Native window: no decorations, always on top, 16:9 by default.

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
/// 640 × 360 logical: 16:9, comfortable as a thumbnail at 1080p and 1440p.
const DEFAULT_SIZE: (f64, f64) = (640.0, 360.0);
/// Below 256 × 144 YouTube's interface is no longer usable.
const MIN_SIZE: (f64, f64) = (256.0, 144.0);

/// Creates the window hidden; it is shown once the webview exists, to
/// avoid a white flash at startup.
pub fn build<T: 'static>(
    target: &EventLoopWindowTarget<T>,
    saved: Option<WindowGeometry>,
) -> Result<Window, OsError> {
    let mut builder = WindowBuilder::new()
        .with_title(TITLE)
        .with_window_icon(crate::icon::window_icon())
        .with_decorations(false)
        // Never topmost when created: App applies it after creating the webview.
        .with_always_on_top(false)
        .with_resizable(true)
        .with_visible(false)
        .with_min_inner_size(LogicalSize::new(MIN_SIZE.0, MIN_SIZE.1))
        .with_inner_size(LogicalSize::new(DEFAULT_SIZE.0, DEFAULT_SIZE.1));

    #[cfg(windows)]
    {
        // File drag-and-drop is not used: avoids registering an OLE
        // IDropTarget that is useless here.
        builder = builder
            .with_drag_and_drop(false)
            .with_taskbar_icon(crate::icon::window_icon())
            // Stable class name: tools like PowerToys use it.
            .with_window_classname("YtMiniPlayerWindow");
    }

    if let Some(geometry) = saved.filter(|g| is_reachable(target, g)) {
        builder = builder
            .with_position(PhysicalPosition::new(geometry.x, geometry.y))
            .with_inner_size(PhysicalSize::new(geometry.width, geometry.height));
    }

    builder.build(target)
}

/// Current geometry to save on exit. `None` if the window is minimized or
/// maximized (Win+↑), because those are not positions the user wants back.
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

/// The saved geometry is only used if it still falls on a connected
/// monitor (e.g. not if the second monitor was unplugged).
fn is_reachable<T: 'static>(target: &EventLoopWindowTarget<T>, geometry: &WindowGeometry) -> bool {
    target.available_monitors().any(|monitor| {
        let position = monitor.position();
        let size = monitor.size();
        geometry.overlaps((position.x, position.y, size.width, size.height))
    })
}
