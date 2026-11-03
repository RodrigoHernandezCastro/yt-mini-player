//! Webview lifecycle following the window state.
//!
//! On minimize:
//! - `IsVisible = false` on the controller: WebView2 stops composing frames
//!   and the page goes to `visibilityState = "hidden"`. Audio keeps
//!   playing, like in a background tab.
//! - `MemoryUsageTargetLevel = Low`: WebView2 trims caches and working
//!   memory. Needs runtime 114.0.1823.32 or later; on older versions the
//!   call does nothing.
//!
//! On restore it is undone in reverse order: first the memory level, then
//! the visibility, so the first frame is not painted with the reduced
//! profile.

use wry::WebView;
#[cfg(windows)]
use wry::{MemoryUsageLevel, WebViewExtWindows};

/// The state applied now, so COM calls are not repeated on every `Resized`
/// (which arrives many times during a resize).
#[derive(Debug, Default)]
pub struct Lifecycle {
    background: bool,
}

impl Lifecycle {
    /// Call on every `WindowEvent::Resized`; it only acts if the minimized
    /// state changed.
    pub fn update(&mut self, webview: &WebView, minimized: bool) {
        if minimized == self.background {
            return;
        }
        self.background = minimized;

        if minimized {
            let _ = webview.set_visible(false);
            set_memory_level(webview, true);
        } else {
            set_memory_level(webview, false);
            let _ = webview.set_visible(true);
        }
    }
}

#[cfg(windows)]
fn set_memory_level(webview: &WebView, low: bool) {
    let level = if low {
        MemoryUsageLevel::Low
    } else {
        MemoryUsageLevel::Normal
    };
    if let Err(err) = webview.set_memory_usage_level(level) {
        eprintln!("could not change the memory level: {err}");
    }
}

#[cfg(not(windows))]
fn set_memory_level(_webview: &WebView, _low: bool) {}
