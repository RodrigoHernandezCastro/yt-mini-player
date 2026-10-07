//! Ciclo de vida del webview según el estado de la ventana (Fase 4).
//!
//! Al minimizar:
//! - `IsVisible = false` en el controlador: WebView2 deja de componer
//!   fotogramas y la página pasa a `visibilityState = "hidden"`. El audio
//!   sigue sonando, como en una pestaña en segundo plano.
//! - `MemoryUsageTargetLevel = Low`: WebView2 recorta cachés y memoria de
//!   trabajo. Requiere runtime 114.0.1823.32 o superior; en versiones
//!   anteriores la llamada no hace nada.
//!
//! Al restaurar se deshace en orden inverso: primero el nivel de memoria,
//! después la visibilidad, para que el primer fotograma no se pinte con el
//! perfil reducido.

use wry::WebView;
#[cfg(windows)]
use wry::{MemoryUsageLevel, WebViewExtWindows};

/// Estado aplicado actualmente, para no repetir llamadas COM en cada
/// `Resized` (que llega muchas veces durante un redimensionado).
#[derive(Debug, Default)]
pub struct Lifecycle {
    background: bool,
}

impl Lifecycle {
    /// Llamar ante cada `WindowEvent::Resized`; solo actúa si el estado de
    /// minimizado cambió.
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
        eprintln!("no se pudo cambiar el nivel de memoria: {err}");
    }
}

#[cfg(not(windows))]
fn set_memory_level(_webview: &WebView, _low: bool) {}
