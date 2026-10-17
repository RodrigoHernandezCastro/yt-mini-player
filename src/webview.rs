//! Capa web: WebView2 (vía wry) montado sobre la ventana de tao.
//!
//! Seguridad (ver también policy.rs):
//! - **IPC autenticado**: nuestros scripts van en un único envoltorio que
//!   guarda, antes de que corra la página, la función real de envío y un
//!   token de sesión aleatorio. La página puede llamar a `window.ipc`, pero
//!   sin el token la app descarta el mensaje. A su vez, las funciones que la
//!   app llama en la página (`__ytmpSync`, `__ytmpToast`) exigen otra clave.
//! - **Navegación filtrada**: solo https (http en la red local), páginas de
//!   extensiones instaladas y la pantalla de inicio; en superposición máxima,
//!   solo el catálogo.
//! - **Ventanas nuevas**: del catálogo → en esta ventana; el resto → en el
//!   navegador predeterminado.
//! - **Descargas bloqueadas** y **permisos denegados** (cámara, micrófono,
//!   ubicación, notificaciones, portapapeles, archivos...).
//! - **SmartScreen activo** (wry lo apaga por defecto).
//! - **Sin guardado de contraseñas ni autorrelleno** en el perfil.

use std::sync::{Arc, Mutex};

use tao::{event_loop::EventLoopProxy, window::Window};
use wry::{http::Request, NewWindowResponse, PermissionKind, PermissionResponse, WebContext, WebView, WebViewBuilder};

use crate::{
    home::{self, SharedHomeState},
    ipc::{self, Command, UserEvent},
    policy::{self, Navigation, NewWindow},
};

/// Protocolo propio de la pantalla de inicio (ver services::HOME_URL).
const HOME_PROTOCOL: &str = "ytmp";
/// Arrastre y redimensionado (Alt+clic, bordes, arrastrar el video).
const INJECT_JS: &str = include_str!("inject.js");
/// Barra lateral, menú, atajos locales, modo solo video y avisos.
const UI_JS: &str = include_str!("ui.js");
/// Recarga preventiva en sesiones largas; vive en la página y no usa IPC.
const MAINTENANCE_JS: &str = include_str!("maintenance.js");
/// Lectura y control de la reproducción (barra de música, bandeja, atajos).
/// Va primero: debe ejecutarse antes que la página para anotar sus controles.
const MEDIA_JS: &str = include_str!("media.js");

/// Igual que el valor por defecto de wry, pero SIN desactivar SmartScreen
/// (`msSmartScreenProtection`): se pueden abrir sitios arbitrarios.
pub const BROWSER_ARGS: &str = "--disable-features=msWebOOUI,msPdfOOUI";

/// Claves de esta sesión (aleatorias en cada arranque).
#[derive(Clone)]
pub struct Keys {
    /// Va delante de cada mensaje IPC de nuestros scripts.
    pub ipc: String,
    /// Exigida por las funciones que la app llama en la página.
    pub host: String,
}

impl Keys {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self { ipc: policy::new_token()?, host: policy::new_token()? })
    }
}

/// Envuelve nuestros scripts en un único ámbito con el canal autenticado.
///
/// `post_expr` es la expresión que obtiene la función de envío. En la app
/// es la de WebView2 capturada antes de que corra la página (la página
/// podría reemplazar `window.chrome.webview.postMessage` después, pero ya
/// tendremos la original).
pub fn bundle(post_expr: &str, keys: &Keys) -> String {
    format!(
        r#"(() => {{
  'use strict';
  const __ytmpPost = {post_expr};
  const __ytmpSend = (m) => {{ if (__ytmpPost) {{ try {{ __ytmpPost("{ipc}|" + m); }} catch (_) {{}} }} }};
  const __ytmpHostKey = "{host}";
  const __ytmpShared = Object.create(null);
  // La pantalla de inicio (nuestra, con CSP estricta) también envía órdenes.
  if (window.top === window && location.origin === 'https://ytmp.localhost') {{
    Object.defineProperty(window, '__ytmpHomeSend', {{ value: __ytmpSend }});
  }}
{MEDIA_JS}
{INJECT_JS}
{UI_JS}
}})();"#,
        ipc = keys.ipc,
        host = keys.host,
    )
}

/// Captura la función real de envío de WebView2 (solo en el marco principal).
const POST_EXPR: &str = "(() => { try { const w = window.top === window && window.chrome && window.chrome.webview; return w ? w.postMessage.bind(w) : null; } catch (_) { return null; } })()";

pub struct Options<'a> {
    pub start_url: &'a str,
    pub user_agent: Option<&'a str>,
    pub home_state: SharedHomeState,
    pub proxy: EventLoopProxy<UserEvent>,
    pub keys: Keys,
    /// Superposición máxima (UIAccess): navegación limitada al catálogo.
    pub overlay: bool,
    /// Ids de extensiones instaladas (sus páginas se pueden abrir).
    pub extension_ids: Arc<Mutex<Vec<String>>>,
}

/// Crea el webview que ocupa toda el área cliente de la ventana.
///
/// wry subclasifica la ventana padre y ajusta el controlador de WebView2 en
/// cada `WM_SIZE`, así que no hace falta propagar redimensionados a mano.
pub fn build(window: &Window, web_context: &mut WebContext, options: Options) -> wry::Result<WebView> {
    let Options { start_url, user_agent, home_state, proxy, keys, overlay, extension_ids } = options;
    let ipc_proxy = proxy.clone();
    let nav_proxy = proxy.clone();
    let popup_proxy = proxy.clone();
    let download_proxy = proxy.clone();
    let token = keys.ipc.clone();
    let mut builder = WebViewBuilder::new_with_web_context(web_context)
        .with_url(start_url)
        // Negro en lugar de blanco mientras carga la primera página.
        .with_background_color((0, 0, 0, 255))
        // En Windows wry los inyecta también en iframes; el envoltorio y
        // cada script se protegen comprobando `window.top`.
        .with_initialization_script(bundle(POST_EXPR, &keys))
        .with_initialization_script(MAINTENANCE_JS)
        .with_custom_protocol(HOME_PROTOCOL.into(), move |_id, request| home::respond(&request, &home_state))
        .with_ipc_handler(move |request: Request<String>| {
            // Sin el token de esta sesión, el mensaje lo escribió la página.
            let Some(message) = policy::authenticate(request.body(), &token) else { return };
            if let Some(command) = ipc::parse(message) {
                // Solo falla si el bucle de eventos ya terminó.
                let _ = ipc_proxy.send_event(UserEvent::Command(command));
            }
        })
        .with_navigation_handler(move |url| {
            let ids = extension_ids.lock().map(|ids| ids.clone()).unwrap_or_default();
            match policy::navigation(&url, overlay, &ids) {
                Navigation::Allow => true,
                Navigation::Upgrade(secure) => {
                    let _ = nav_proxy.send_event(UserEvent::Command(Command::Navigate(secure)));
                    false
                }
                Navigation::Block(reason) => {
                    let _ = nav_proxy.send_event(UserEvent::Command(Command::Blocked(reason.to_owned())));
                    false
                }
            }
        })
        .with_new_window_req_handler(move |url, _features| {
            let command = match policy::new_window(&url, overlay) {
                NewWindow::SameWindow(url) => Command::Navigate(url),
                NewWindow::External(url) => Command::OpenExternal(url),
                NewWindow::Deny => Command::Blocked("ventana emergente bloqueada".to_owned()),
            };
            let _ = popup_proxy.send_event(UserEvent::Command(command));
            NewWindowResponse::Deny
        })
        .with_download_started_handler(move |_url, _path| {
            let _ = download_proxy.send_event(UserEvent::Command(Command::Blocked(
                "las descargas están desactivadas en el reproductor".to_owned(),
            )));
            false
        })
        .with_permission_handler(permission);

    #[cfg(windows)]
    {
        use wry::WebViewBuilderExtWindows;
        builder = builder
            // https://ytmp.localhost: contexto seguro (necesario para el DRM).
            .with_https_scheme(true)
            .with_additional_browser_args(BROWSER_ARGS)
            // Las extensiones se instalan después, una a una (extensions.rs).
            .with_browser_extensions_enabled(true);
    }

    if let Some(ua) = user_agent {
        builder = builder.with_user_agent(ua);
    }

    let webview = builder.build(window)?;
    harden(&webview);
    Ok(webview)
}

/// Permisos: se deniega todo lo que un reproductor no necesita. El DRM no
/// pasa por aquí como permiso propio; lo desconocido sigue el comportamiento
/// por defecto para no romperlo.
pub fn permission(kind: PermissionKind) -> PermissionResponse {
    match kind {
        PermissionKind::Autoplay => PermissionResponse::Allow,
        PermissionKind::MediaKeySystemAccess | PermissionKind::Other => PermissionResponse::Default,
        _ => PermissionResponse::Deny,
    }
}

/// Ajustes del perfil que wry no expone.
#[cfg(windows)]
pub fn harden(webview: &WebView) {
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        ICoreWebView2Profile3, ICoreWebView2Settings4, ICoreWebView2Settings8, ICoreWebView2_13,
        COREWEBVIEW2_TRACKING_PREVENTION_LEVEL_BALANCED,
    };
    use windows::core::Interface;
    use wry::WebViewExtWindows;

    let core = webview.webview();
    let result = unsafe {
        core.Settings().and_then(|settings| {
            // Nada de objetos del host en la página.
            settings.SetAreHostObjectsAllowed(false)?;
            // Contraseñas y formularios: el perfil no guarda ninguno.
            let s4 = settings.cast::<ICoreWebView2Settings4>()?;
            s4.SetIsPasswordAutosaveEnabled(false)?;
            s4.SetIsGeneralAutofillEnabled(false)?;
            // SmartScreen (si el runtime lo admite).
            if let Ok(s8) = settings.cast::<ICoreWebView2Settings8>() {
                s8.SetIsReputationCheckingRequired(true)?;
            }
            Ok(())
        })
    };
    if let Err(err) = result {
        eprintln!("no se pudieron aplicar los ajustes de seguridad: {err}");
    }
    let tracking = unsafe {
        core.cast::<ICoreWebView2_13>()
            .and_then(|c| c.Profile())
            .and_then(|p| p.cast::<ICoreWebView2Profile3>())
            .and_then(|p| p.SetPreferredTrackingPreventionLevel(COREWEBVIEW2_TRACKING_PREVENTION_LEVEL_BALANCED))
    };
    if let Err(err) = tracking {
        eprintln!("prevención de rastreo no disponible: {err}");
    }
}

#[cfg(not(windows))]
pub fn harden(_: &WebView) {}

/// Borra cookies, almacenamiento, caché, historial y lo que el sitio guardó
/// (cierra la sesión en todos los servicios). `done` recibe si funcionó.
#[cfg(windows)]
pub fn clear_browsing_data(webview: &WebView, done: impl FnOnce(bool) + 'static) {
    use webview2_com::{
        ClearBrowsingDataCompletedHandler,
        Microsoft::Web::WebView2::Win32::{ICoreWebView2Profile2, ICoreWebView2_13},
    };
    use windows::core::Interface;
    use wry::WebViewExtWindows;

    let done = std::rc::Rc::new(std::cell::RefCell::new(Some(done)));
    let finish = {
        let done = done.clone();
        move |ok: bool| {
            if let Some(done) = done.borrow_mut().take() {
                done(ok);
            }
        }
    };
    let on_complete = finish.clone();
    let handler = ClearBrowsingDataCompletedHandler::create(Box::new(move |result| {
        on_complete(result.is_ok());
        Ok(())
    }));
    let started = unsafe {
        webview
            .webview()
            .cast::<ICoreWebView2_13>()
            .and_then(|c| c.Profile())
            .and_then(|p| p.cast::<ICoreWebView2Profile2>())
            .and_then(|p| p.ClearBrowsingDataAll(&handler))
    };
    if started.is_err() {
        finish(false);
    }
}

#[cfg(not(windows))]
pub fn clear_browsing_data(_: &WebView, done: impl FnOnce(bool) + 'static) {
    done(false);
}

/// Cambia el User-Agent; se aplica a la siguiente navegación.
#[cfg(windows)]
pub fn set_user_agent(webview: &WebView, user_agent: &str) {
    use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings2;
    use windows::core::{Interface, HSTRING};
    use wry::WebViewExtWindows;

    let result = unsafe {
        webview
            .webview()
            .Settings()
            .and_then(|settings| settings.cast::<ICoreWebView2Settings2>())
            .and_then(|settings| settings.SetUserAgent(&HSTRING::from(user_agent)))
    };
    if let Err(err) = result {
        eprintln!("no se pudo cambiar el User-Agent: {err}");
    }
}

#[cfg(not(windows))]
pub fn set_user_agent(_: &WebView, _: &str) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bundle_keeps_secrets_out_of_reach() {
        let keys = Keys { ipc: "a".repeat(32), host: "b".repeat(32) };
        let js = bundle("null", &keys);
        assert!(js.starts_with("(() => {"));
        assert!(js.trim_end().ends_with("})();"));
        // Las claves solo aparecen en el envoltorio, nunca en globales.
        assert_eq!(js.matches(&keys.ipc).count(), 1);
        assert_eq!(js.matches(&keys.host).count(), 1);
        assert!(!js.contains("window.ipc.postMessage"), "ningún script usa el canal sin token");
        assert!(!js.contains("window.__ytmpStartDrag"));
    }

    #[test]
    fn permissions_are_denied_by_default() {
        for kind in [
            PermissionKind::Microphone,
            PermissionKind::Camera,
            PermissionKind::Geolocation,
            PermissionKind::Notifications,
            PermissionKind::ClipboardRead,
            PermissionKind::DisplayCapture,
            PermissionKind::FileSystemAccess,
            PermissionKind::LocalFonts,
            PermissionKind::AutomaticDownloads,
        ] {
            assert_eq!(permission(kind), PermissionResponse::Deny, "{kind}");
        }
        assert_eq!(permission(PermissionKind::Autoplay), PermissionResponse::Allow);
        assert_eq!(permission(PermissionKind::MediaKeySystemAccess), PermissionResponse::Default, "no romper el DRM");
    }
}
