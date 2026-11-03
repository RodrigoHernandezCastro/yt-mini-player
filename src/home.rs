//! The app's own pages, served by the `ytmp` protocol (on Windows,
//! `https://ytmp.localhost/`):
//! - `/`      start screen (pick a service),
//! - `/music` "My music" (and its songs and covers, see music::serve),
//! - `/mini`  music bar, `/lock` lock screen,
//! - `/together` the "Watch together" window, `/settings` the Settings
//!   window,
//! - `/stream` the host's window, in a guest's player, and `/cast` the
//!   hidden page that sends it (cast.rs).
//!
//! Their data and their strings in the current language are embedded when
//! they are served, so they are drawn complete from the first frame.

use std::{
    borrow::Cow,
    sync::{Arc, Mutex, OnceLock},
};

use http::{header, Request, Response, StatusCode};
use serde::Serialize;

use crate::{
    i18n,
    services::{Service, MUSIC, SERVICES},
};

const TEMPLATE: &str = include_str!("home.html");
/// Music bar (see mini.rs).
const MINI: &str = include_str!("mini.html");
/// Lock screen (see lock.rs).
const LOCK: &str = include_str!("lock.html");
/// My music (see music/).
const MUSIC_PAGE: &str = include_str!("music.html");
/// Watch together (see together/).
const TOGETHER: &str = include_str!("together.html");
/// The Settings window (see settings_panel.rs).
const SETTINGS: &str = include_str!("settings.html");
/// The host's shared window, as a guest sees it.
const STREAM: &str = include_str!("stream.html");
/// The hidden page that sends the host's window (no text of its own).
const CAST: &str = include_str!("cast.html");
const PLACEHOLDER: &str = "__YTMP_DATA__";
const LANG_PLACEHOLDER: &str = "__YTMP_LANG__";

/// Strings each page gets (key prefixes).
const HOME_STRINGS: &[&str] = &["home.", "service.", "lang.", "common."];
const MUSIC_STRINGS: &[&str] = &["music.", "net.", "common."];
const MINI_STRINGS: &[&str] = &["mini.", "common."];
const LOCK_STRINGS: &[&str] = &["lock.", "common."];
const TOGETHER_STRINGS: &[&str] = &["together.", "common."];
const STREAM_STRINGS: &[&str] = &["stream.", "common."];
const SETTINGS_STRINGS: &[&str] = &["settings.", "lang.", "common."];
/// The side bar and menu (ui.js) on every page.
pub const UI_STRINGS: &[&str] = &["ui.", "key.", "lang."];

/// Page security policy: nothing external, only its own script and inline
/// styles. The scripts the app injects do not depend on it.
const CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; \
                   img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";
/// The music bar also loads the video thumbnail (https).
const MINI_CSP: &str = "default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; \
                        img-src https: data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'";

/// What the start screen needs to know about the settings. Shared by the
/// app (which updates it) and the protocol handler (which reads it).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HomeState {
    pub last_service: Option<String>,
    pub start_with_last: bool,
    /// Show the first-run welcome (not seen yet, or asked for again).
    pub welcome: bool,
}

pub type SharedHomeState = Arc<Mutex<HomeState>>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ServiceView {
    id: &'static str,
    name: String,
    /// Key of the name when it is translated (My music), so the page can
    /// show it again after a language change.
    name_key: Option<&'static str>,
    caption: String,
    caption_key: &'static str,
    accent: &'static str,
    drm: bool,
}

/// Language data every page gets.
#[derive(Serialize)]
struct PageI18n {
    #[serde(flatten)]
    info: i18n::LanguageInfo,
    strings: serde_json::Map<String, serde_json::Value>,
}

fn page_i18n(prefixes: &[&str]) -> PageI18n {
    PageI18n { info: i18n::language_info(), strings: i18n::page_strings(prefixes) }
}

/// The strings of the app page at `path`, for a live language change.
pub fn strings_for_path(path: &str) -> Option<serde_json::Map<String, serde_json::Value>> {
    match path {
        "/" | "" => Some(i18n::page_strings(HOME_STRINGS)),
        "/music" => Some(i18n::page_strings(MUSIC_STRINGS)),
        "/stream" => Some(i18n::page_strings(STREAM_STRINGS)),
        _ => None,
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HomeData<'a> {
    version: &'static str,
    services: Vec<ServiceView>,
    music: ServiceView,
    last_service: Option<&'a str>,
    start_with_last: bool,
    welcome: bool,
    i18n: PageI18n,
}

/// "My music": its inline script is allowed by its hash (stricter than
/// 'unsafe-inline': no other inline script could run even if one slipped
/// into the page). Songs and covers: from the host itself; previews and
/// thumbnails: from the Internet Archive.
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

/// `sha256-…` of the page's last inline `<script>`.
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
    i18n: PageI18n,
}

#[derive(Serialize)]
struct StringsOnly {
    i18n: PageI18n,
}

/// Fills in a template: the data and the `lang` of the document.
fn fill<T: Serialize>(template: &str, data: &T) -> String {
    template.replacen(LANG_PLACEHOLDER, i18n::current().code(), 1).replacen(PLACEHOLDER, &json_for_script(data), 1)
}

pub fn render_music() -> String {
    let data = MusicData {
        version: env!("CARGO_PKG_VERSION"),
        media_key: crate::music::serve::shared().map(|s| s.key().to_owned()),
        i18n: page_i18n(MUSIC_STRINGS),
    };
    fill(MUSIC_PAGE, &data)
}

fn service_view(service: &Service, translated_name: bool) -> ServiceView {
    ServiceView {
        id: service.id,
        name: if translated_name { i18n::text(service.name) } else { service.name.to_owned() },
        name_key: translated_name.then_some(service.name),
        caption: i18n::text(service.caption),
        caption_key: service.caption,
        accent: service.accent,
        drm: service.drm,
    }
}

/// Final HTML with the data in a `<script type="application/json">`.
pub fn render(state: &HomeState) -> String {
    let data = HomeData {
        version: env!("CARGO_PKG_VERSION"),
        services: SERVICES
            .iter()
            .map(|s| service_view(s, false))
            .collect(),
        music: service_view(&MUSIC, true),
        last_service: state.last_service.as_deref(),
        start_with_last: state.start_with_last,
        welcome: state.welcome,
        i18n: page_i18n(HOME_STRINGS),
    };
    fill(TEMPLATE, &data)
}

/// JSON that is safe inside `<script>`: no literal `<`, `>` or `&` (so
/// `</script>` cannot close the tag) nor Unicode line separators.
fn json_for_script<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value)
        .unwrap_or_else(|_| "{}".into())
        .replace('<', "\\u003c")
        .replace('>', "\\u003e")
        .replace('&', "\\u0026")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// The `ytmp` protocol handler: the app's own pages and, for "My music",
/// its songs and covers. Nothing else.
pub fn respond(request: &Request<Vec<u8>>, state: &SharedHomeState) -> Response<Cow<'static, [u8]>> {
    let uri = request.uri();
    if let Some(response) = crate::music::serve::respond(uri.path(), uri.query(), request.headers()) {
        return response;
    }
    let (body, csp) = match uri.path() {
        "/" | "" => {
            let snapshot = state.lock().map(|s| s.clone()).unwrap_or_default();
            (render(&snapshot), CSP)
        }
        "/music" => (render_music(), music_csp()),
        "/mini" => (fill(MINI, &StringsOnly { i18n: page_i18n(MINI_STRINGS) }), MINI_CSP),
        "/lock" => (fill(LOCK, &StringsOnly { i18n: page_i18n(LOCK_STRINGS) }), CSP),
        "/together" => (fill(TOGETHER, &StringsOnly { i18n: page_i18n(TOGETHER_STRINGS) }), CSP),
        "/stream" => (fill(STREAM, &StringsOnly { i18n: page_i18n(STREAM_STRINGS) }), CSP),
        "/settings" => (fill(SETTINGS, &StringsOnly { i18n: page_i18n(SETTINGS_STRINGS) }), CSP),
        "/cast" => (CAST.to_owned(), CSP),
        _ => {
            return Response::builder()
                .status(StatusCode::NOT_FOUND)
                .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
                .body(Cow::Borrowed(&b"Not found"[..]))
                .expect("valid 404 response");
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
        .body(Cow::Owned(body.into_bytes()))
        .expect("valid 200 response")
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
    fn embeds_catalog_settings_and_strings() {
        let html = render(&HomeState { last_service: Some("netflix".into()), start_with_last: true, welcome: true });
        assert!(!html.contains(PLACEHOLDER) && !html.contains(LANG_PLACEHOLDER));
        let data = extract_json(&html);
        assert_eq!(data["services"].as_array().unwrap().len(), SERVICES.len());
        assert_eq!(data["services"][2]["id"], "netflix");
        assert_eq!(data["services"][2]["drm"], true);
        assert_eq!(data["lastService"], "netflix");
        assert_eq!(data["startWithLast"], true);
        assert_eq!(data["welcome"], true);
        assert_eq!(extract_json(&render(&HomeState::default()))["welcome"], false, "only when asked");
        assert_eq!(data["music"]["id"], "music");
        assert!(!data["music"]["name"].as_str().unwrap().contains('.'), "a text, not a key");
        let i18n = &data["i18n"];
        assert_eq!(i18n["languages"].as_array().unwrap().len(), 5);
        assert!(i18n["strings"]["home.title"].is_string());
        assert!(i18n["strings"].as_object().unwrap().keys().all(|k| HOME_STRINGS.iter().any(|p| k.starts_with(p))));
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
        // The embedded data does not change the script the hash allows.
        assert_eq!(script_hash(&html), script_hash(MUSIC_PAGE));
        assert!(extract_json(&html)["i18n"]["strings"]["music.tab.library"].is_string());
        // The hash is the real script's.
        assert!(script_hash("<p><script>abc</script>").ends_with(&{
            use base64::{engine::general_purpose::STANDARD, Engine};
            use sha2::{Digest, Sha256};
            STANDARD.encode(Sha256::digest(b"abc"))
        }));
        // Songs without the key: forbidden (or no service in the tests).
        let media = respond(&Request::get("ytmp://localhost/media/0123456789abcdef").body(Vec::new()).unwrap(), &state);
        assert!(matches!(media.status(), StatusCode::FORBIDDEN | StatusCode::NOT_FOUND));
    }

    #[test]
    fn script_breakout_is_impossible() {
        // A corrupt id in the settings cannot close the tag.
        let html = render(&HomeState {
            last_service: Some("</script><script>alert(1)</script>&\u{2028}".into()),
            start_with_last: false,
            welcome: false,
        });
        assert_eq!(html.matches("</script>").count(), 2, "only the page's own two tags");
        assert_eq!(extract_json(&html)["lastService"], "</script><script>alert(1)</script>&\u{2028}");
    }

    #[test]
    fn serves_only_its_pages_with_strict_headers() {
        let state = SharedHomeState::default();
        let ok = respond(&Request::get("ytmp://localhost/").body(Vec::new()).unwrap(), &state);
        assert_eq!(ok.status(), StatusCode::OK);
        assert!(ok.headers()["content-security-policy"].to_str().unwrap().contains("default-src 'none'"));
        let home = String::from_utf8_lossy(ok.body()).into_owned();
        assert!(home.contains(&format!("<html lang=\"{}\">", i18n::current().code())));

        let mini = respond(&Request::get("ytmp://localhost/mini").body(Vec::new()).unwrap(), &state);
        assert_eq!(mini.status(), StatusCode::OK);
        assert!(mini.headers()["content-security-policy"].to_str().unwrap().contains("img-src https: data:"));
        let mini = String::from_utf8_lossy(mini.body()).into_owned();
        assert!(mini.contains("__ytmpMini") && extract_json(&mini)["i18n"]["strings"]["mini.nothing_playing"].is_string());

        let lock = respond(&Request::get("ytmp://localhost/lock").body(Vec::new()).unwrap(), &state);
        assert_eq!(lock.status(), StatusCode::OK);
        let csp = lock.headers()["content-security-policy"].to_str().unwrap();
        assert!(csp.contains("default-src 'none'") && csp.contains("img-src data:") && !csp.contains("https:"));
        let lock_html = String::from_utf8_lossy(lock.body()).into_owned();
        assert!(lock_html.contains("__ytmpLock") && extract_json(&lock_html)["i18n"]["strings"]["lock.unlock"].is_string());
        assert_eq!(lock.headers()["referrer-policy"], "no-referrer");

        let together = respond(&Request::get("ytmp://localhost/together").body(Vec::new()).unwrap(), &state);
        assert_eq!(together.status(), StatusCode::OK);
        let csp = together.headers()["content-security-policy"].to_str().unwrap();
        assert!(csp.contains("default-src 'none'") && !csp.contains("https:"), "nothing external: {csp}");
        let together_html = String::from_utf8_lossy(together.body()).into_owned();
        let strings = extract_json(&together_html)["i18n"]["strings"].clone();
        assert!(together_html.contains("__ytmpTogether") && strings["together.start"].is_string());
        assert!(strings.as_object().unwrap().keys().all(|k| TOGETHER_STRINGS.iter().any(|p| k.starts_with(p))));

        for (path, marker) in [("ytmp://localhost/stream", "ytmp:stream"), ("ytmp://localhost/cast", "cast:offer:")] {
            let res = respond(&Request::get(path).body(Vec::new()).unwrap(), &state);
            assert_eq!(res.status(), StatusCode::OK, "{path}");
            let csp = res.headers()["content-security-policy"].to_str().unwrap();
            assert!(csp.contains("default-src 'none'") && !csp.contains("https:"), "nothing external: {csp}");
            let html = String::from_utf8_lossy(res.body()).into_owned();
            assert!(html.contains(marker) && !html.contains(PLACEHOLDER), "{path}");
        }
        let settings = respond(&Request::get("ytmp://localhost/settings").body(Vec::new()).unwrap(), &state);
        assert_eq!(settings.status(), StatusCode::OK);
        let csp = settings.headers()["content-security-policy"].to_str().unwrap();
        assert!(csp.contains("default-src 'none'") && !csp.contains("https:"), "nothing external: {csp}");
        assert_eq!(settings.headers()["x-frame-options"], "DENY");
        let settings_html = String::from_utf8_lossy(settings.body()).into_owned();
        let strings = extract_json(&settings_html)["i18n"]["strings"].clone();
        assert!(settings_html.contains("__ytmpSettings") && !settings_html.contains(PLACEHOLDER));
        assert!(strings["settings.title"].is_string() && strings["lang.label"].is_string());
        assert!(strings.as_object().unwrap().keys().all(|k| SETTINGS_STRINGS.iter().any(|p| k.starts_with(p))));
        assert!(strings_for_path("/settings").is_none(), "its own window reloads instead");

        let stream = respond(&Request::get("ytmp://localhost/stream").body(Vec::new()).unwrap(), &state);
        let strings = extract_json(&String::from_utf8_lossy(stream.body()))["i18n"]["strings"].clone();
        assert!(strings["stream.connecting"].is_string());
        assert!(strings.as_object().unwrap().keys().all(|k| STREAM_STRINGS.iter().any(|p| k.starts_with(p))));
        assert!(strings_for_path("/stream").is_some());

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
        assert!(strings_for_path("/music").is_some() && strings_for_path("/lock").is_none());
    }
}
