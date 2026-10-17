//! Páginas propias de la app, servidas por el protocolo `ytmp` (en Windows,
//! `https://ytmp.localhost/`):
//! - `/`      pantalla de inicio (elegir servicio),
//! - `/music` "Mi música" (y sus canciones y portadas, ver music::serve),
//! - `/mini`  barra de música, `/lock` pantalla de bloqueo.
//!
//! Los datos van incrustados al servirlas, así que se dibujan completas desde
//! el primer momento.

use std::{
    borrow::Cow,
    sync::{Arc, Mutex, OnceLock},
};

use serde::Serialize;
use http::{header, Request, Response, StatusCode};

use crate::services::{MUSIC, SERVICES};

const TEMPLATE: &str = include_str!("home.html");
/// Barra de música (ver mini.rs).
const MINI: &str = include_str!("mini.html");
/// Pantalla de bloqueo (ver lock.rs).
const LOCK: &str = include_str!("lock.html");
/// Mi música (ver music/).
const MUSIC_PAGE: &str = include_str!("music.html");
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
    music: ServiceView,
    last_service: Option<&'a str>,
    start_with_last: bool,
}

/// "Mi música": su script en línea se autoriza por su hash (más estricto que
/// 'unsafe-inline': ningún otro script en línea podría ejecutarse aunque se
/// colara en la página). Canciones y portadas: del propio host; vistas
/// previas y miniaturas: del Internet Archive.
fn music_csp() -> &'static str {
    static CSP: OnceLock<String> = OnceLock::new();
    CSP.get_or_init(|| {
        format!(
            "default-src 'none'; script-src '{}'; style-src 'unsafe-inline'; \
             img-src 'self' data: https://archive.org https://*.archive.org; \
             media-src 'self' https://archive.org https://*.archive.org; \
             base-uri 'none'; form-action 'none'; frame-ancestors 'none'",
            script_hash(MUSIC_PAGE)
        )
    })
}

/// `sha256-…` del último `<script>` en línea de la página.
pub fn script_hash(html: &str) -> String {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use sha2::{Digest, Sha256};
    let start = html.rfind("<script>").map(|i| i + "<script>".len()).unwrap_or(0);
    let end = html.rfind("</script>").filter(|&e| e >= start).unwrap_or(start);
    format!("sha256-{}", STANDARD.encode(Sha256::digest(&html.as_bytes()[start..end])))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct MusicData {
    version: &'static str,
    media_key: Option<String>,
}

pub fn render_music() -> String {
    let data = MusicData {
        version: env!("CARGO_PKG_VERSION"),
        media_key: crate::music::serve::shared().map(|s| s.key().to_owned()),
    };
    MUSIC_PAGE.replacen(PLACEHOLDER, &json_for_script(&data), 1)
}

/// HTML final con los datos en un `<script type="application/json">`.
pub fn render(state: &HomeState) -> String {
    let data = HomeData {
        version: env!("CARGO_PKG_VERSION"),
        services: SERVICES
            .iter()
            .map(|s| ServiceView { id: s.id, name: s.name, caption: s.caption, accent: s.accent, drm: s.drm })
            .collect(),
        music: ServiceView { id: MUSIC.id, name: MUSIC.name, caption: MUSIC.caption, accent: MUSIC.accent, drm: false },
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

/// Manejador del protocolo `ytmp`: las páginas propias y, para "Mi
/// música", sus canciones y portadas. Nada más.
pub fn respond(request: &Request<Vec<u8>>, state: &SharedHomeState) -> Response<Cow<'static, [u8]>> {
    let uri = request.uri();
    if let Some(response) = crate::music::serve::respond(uri.path(), uri.query(), request.headers()) {
        return response;
    }
    let (body, csp) = match uri.path() {
        "/" | "" => {
            let snapshot = state.lock().map(|s| s.clone()).unwrap_or_default();
            (Cow::Owned(render(&snapshot).into_bytes()), CSP)
        }
        "/music" => (Cow::Owned(render_music().into_bytes()), music_csp()),
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
        .header("X-Frame-Options", "DENY")
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
        assert_eq!(data["music"]["id"], "music");
        assert_eq!(data["music"]["name"], "Mi música");
    }

    #[test]
    fn music_page_allows_only_its_own_script() {
        let state = SharedHomeState::default();
        let res = respond(&Request::get("ytmp://localhost/music").body(Vec::new()).unwrap(), &state);
        assert_eq!(res.status(), StatusCode::OK);
        let csp = res.headers()["content-security-policy"].to_str().unwrap();
        assert!(csp.contains(&format!("script-src '{}'", script_hash(MUSIC_PAGE))), "{csp}");
        assert!(!csp.contains("unsafe-inline';") || !csp.contains("script-src 'unsafe-inline'"));
        assert!(csp.contains("media-src 'self' https://archive.org https://*.archive.org"));
        assert!(csp.contains("frame-ancestors 'none'"));
        assert_eq!(res.headers()["x-frame-options"], "DENY");
        let html = String::from_utf8_lossy(res.body()).into_owned();
        assert!(html.contains("ytmp:music") && !html.contains(PLACEHOLDER));
        // El hash es el del script de verdad.
        assert!(script_hash("<p><script>abc</script>").ends_with(&{
            use base64::{engine::general_purpose::STANDARD, Engine};
            use sha2::{Digest, Sha256};
            STANDARD.encode(Sha256::digest(b"abc"))
        }));
        // Canciones sin la clave: prohibido (o no hay servicio en las pruebas).
        let media = respond(&Request::get("ytmp://localhost/media/0123456789abcdef").body(Vec::new()).unwrap(), &state);
        assert!(matches!(media.status(), StatusCode::FORBIDDEN | StatusCode::NOT_FOUND));
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
