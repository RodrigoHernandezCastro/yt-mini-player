//! Pantalla de inicio: elegir servicio.
//!
//! Se sirve desde el protocolo propio `ytmp` (en Windows,
//! `https://ytmp.localhost/`) con los datos incrustados al servirla, así que
//! se dibuja completa desde el primer momento.

use std::{
    borrow::Cow,
    sync::{Arc, Mutex},
};

use serde::Serialize;
use http::{header, Request, Response, StatusCode};

use crate::services::SERVICES;

const TEMPLATE: &str = include_str!("home.html");
/// Barra de música (ver mini.rs).
const MINI: &str = include_str!("mini.html");
/// Pantalla de bloqueo (ver lock.rs).
const LOCK: &str = include_str!("lock.html");
const PLACEHOLDER: &str = "__YTMP_DATA__";

/// Política de seguridad de la página: nada externo, solo su propio script
/// y estilos en línea. Los scripts que inyecta la app no dependen de ella.
const CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; \
                   img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
/// La barra de música además carga la miniatura del video (https).
const MINI_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; \
                        img-src https: data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// Lo que la pantalla de inicio necesita saber de los ajustes. Lo comparten
/// la app (que lo actualiza) y el manejador del protocolo (que lo lee).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HomeState {
    pub last_service: Option<String>,
    pub start_with_last: bool,
}

pub type SharedHomeState = Arc<Mutex<HomeState>>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceView {
    id: &'static str,
    name: &'static str,
    caption: &'static str,
    accent: &'static str,
    drm: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HomeData<'a> {
    version: &'static str,
    services: Vec<ServiceView>,
    last_service: Option<&'a str>,
    start_with_last: bool,
}

/// HTML final con los datos en un `<script type="application/json">`.
pub fn render(state: &HomeState) -> String {
    let data = HomeData {
        version: env!("CARGO_PKG_VERSION"),
        services: SERVICES
            .iter()
            .map(|s| ServiceView { id: s.id, name: s.name, caption: s.caption, accent: s.accent, drm: s.drm })
            .collect(),
        last_service: state.last_service.as_deref(),
        start_with_last: state.start_with_last,
    };
    TEMPLATE.replacen(PLACEHOLDER, &json_for_script(&data), 1)
}

/// JSON seguro dentro de `<script>`: sin `<`, `>` ni `&` literales (evita
/// cerrar la etiqueta con `</script>`) ni separadores de línea Unicode.
fn json_for_script<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "{}".into())
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// Manejador del protocolo `ytmp`: la pantalla de inicio (`/`) y el mini
/// controlador (`/mini`). Nada más.
pub fn respond(request: &Request<Vec<u8>>, state: &SharedHomeState) -> Response<Cow<'static, [u8]>> {
    let (body, csp) = match request.uri().path() {
        "/" | "" => {
            let snapshot = state.lock().map(|s| s.clone()).unwrap_or_default();
            (Cow::Owned(render(&snapshot).into_bytes()), CSP)
        }
        "/mini" => (Cow::Borrowed(MINI.as_bytes()), MINI_CSP),
        "/lock" => (Cow::Borrowed(LOCK.as_bytes()), CSP),
        _ => {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .body(Cow::Borrowed(&b"No encontrado"[..]))
                .expect("respuesta 404 válida");
        }
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .header("Content-Security-Policy", csp)
        .header("X-Content-Type-Options", "nosniff")
        .header("Referrer-Policy", "no-referrer")
        .header("Cross-Origin-Opener-Policy", "same-origin")
        .body(body)
        .expect("respuesta 200 válida")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn extract_json(html: &str) -> serde_json::Value {
        let start = html.find(r#"<script id="ytmp-data" type="application/json">"#).unwrap();
        let rest = &html[start..];
        let open = rest.find('>').unwrap() + 1;
        let close = rest.find("</script>").unwrap();
        serde_json::from_str(&rest[open..close]).unwrap()
    }

    #[test]
    fn embeds_catalog_and_settings() {
        let html = render(&HomeState { last_service: Some("netflix".into()), start_with_last: true });
        assert!(!html.contains(PLACEHOLDER));
        let data = extract_json(&html);
        assert_eq!(data["services"].as_array().unwrap().len(), SERVICES.len());
        assert_eq!(data["services"][2]["id"], "netflix");
        assert_eq!(data["services"][2]["drm"], true);
        assert_eq!(data["lastService"], "netflix");
        assert_eq!(data["startWithLast"], true);
    }

    #[test]
    fn script_breakout_is_impossible() {
        // Un id corrupto en settings.json no puede cerrar la etiqueta.
        let html = render(&HomeState {
            last_service: Some("</script><script>alert(1)</script>&\u{2028}".into()),
            start_with_last: false,
        });
        assert_eq!(html.matches("</script>").count(), 2, "solo las dos etiquetas propias");
        assert_eq!(extract_json(&html)["lastService"], "</script><script>alert(1)</script>&\u{2028}");
    }

    #[test]
    fn serves_only_the_root_with_strict_headers() {
        let state = SharedHomeState::default();
        let ok = respond(&Request::get("ytmp://localhost/").body(Vec::new()).unwrap(), &state);
        assert_eq!(ok.status(), StatusCode::OK);
        assert!(ok.headers()["content-security-policy"].to_str().unwrap().contains("default-src 'none'"));
        assert!(String::from_utf8_lossy(ok.body()).contains("¿Qué vemos?"));

        let mini = respond(&Request::get("ytmp://localhost/mini").body(Vec::new()).unwrap(), &state);
        assert_eq!(mini.status(), StatusCode::OK);
        assert!(mini.headers()["content-security-policy"].to_str().unwrap().contains("img-src https: data:"));
        assert!(String::from_utf8_lossy(mini.body()).contains("__ytmpMini"));

        let lock = respond(&Request::get("ytmp://localhost/lock").body(Vec::new()).unwrap(), &state);
        assert_eq!(lock.status(), StatusCode::OK);
        let csp = lock.headers()["content-security-policy"].to_str().unwrap();
        assert!(csp.contains("default-src 'none'") && csp.contains("img-src data:") && !csp.contains("https:"));
        assert!(String::from_utf8_lossy(lock.body()).contains("__ytmpLock"));
        assert_eq!(lock.headers()["referrer-policy"], "no-referrer");

        for path in [
            "ytmp://localhost/../etc/passwd",
            "ytmp://localhost/settings.json",
            "ytmp://localhost/settings.enc",
            "ytmp://localhost/vault.key",
            "ytmp://localhost/x",
            "ytmp://localhost/mini/x",
        ] {
            let res = respond(&Request::get(path).body(Vec::new()).unwrap(), &state);
            assert_eq!(res.status(), StatusCode::NOT_FOUND, "{path}");
        }
    }
}
