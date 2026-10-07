//! Capa web: WebView2 (vía wry) montado sobre la ventana de tao.

use tao::{event_loop::EventLoopProxy, window::Window};
use wry::{http::Request, WebContext, WebView, WebViewBuilder};

use crate::{
    ipc::{self, UserEvent},
    user_agent,
};

const START_URL: &str = "https://www.youtube.com/";
/// Arrastre y redimensionado (Alt+clic, bordes, arrastrar el video).
const INJECT_JS: &str = include_str!("inject.js");
/// Barra lateral, menú, atajos locales, modo solo video y avisos.
const UI_JS: &str = include_str!("ui.js");
/// Recarga preventiva en sesiones largas; vive en la página y no usa IPC.
const MAINTENANCE_JS: &str = include_str!("maintenance.js");

/// Crea el webview que ocupa toda el área cliente de la ventana.
///
/// wry subclasifica la ventana padre y ajusta el controlador de WebView2 en
/// cada `WM_SIZE`, así que no hace falta propagar redimensionados a mano.
pub fn build(
    window: &Window,
    web_context: &mut WebContext,
    runtime_version: &str,
    proxy: EventLoopProxy<UserEvent>,
) -> wry::Result<WebView> {
    let mut builder = WebViewBuilder::new_with_web_context(web_context)
        .with_url(START_URL)
        // Negro en lugar de blanco mientras carga la primera página.
        .with_background_color((0, 0, 0, 255))
        // En Windows wry los inyecta también en iframes; cada script se
        // protege comprobando `window.top`.
        .with_initialization_script(INJECT_JS)
        .with_initialization_script(UI_JS)
        .with_initialization_script(MAINTENANCE_JS)
        .with_ipc_handler(move |request: Request<String>| {
            if let Some(command) = ipc::parse(request.body()) {
                // Solo falla si el bucle de eventos ya terminó.
                let _ = proxy.send_event(UserEvent::Command(command));
            }
        });

    #[cfg(windows)]
    {
        use wry::WebViewBuilderExtWindows;
        // Las extensiones se instalan después, una a una (extensions.rs).
        builder = builder.with_browser_extensions_enabled(true);
    }

    if let Some(ua) = user_agent::chrome_user_agent(runtime_version) {
        builder = builder.with_user_agent(ua);
    }

    builder.build(window)
}
