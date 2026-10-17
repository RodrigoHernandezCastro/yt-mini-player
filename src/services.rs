//! Catálogo de servicios de streaming y validación de direcciones.
//!
//! Cada servicio declara qué User-Agent necesita:
//! - `Chrome`: YouTube y YouTube Music. Google bloquea el inicio de sesión
//!   en navegadores embebidos que se identifican como tales.
//! - `Edge`: el resto. WebView2 *es* Edge, y declararlo permite que los
//!   servicios con DRM elijan el sistema de protección que el motor ofrece.

use url::Url;

/// Dirección de la pantalla de inicio. wry sirve el protocolo `ytmp` en
/// Windows como `https://ytmp.<ruta>`; https la convierte en un contexto
/// seguro (necesario para comprobar el DRM desde la página).
pub const HOME_URL: &str = "https://ytmp.localhost/";
pub const HOME_HOST: &str = "ytmp.localhost";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Agent {
    Chrome,
    Edge,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Service {
    pub id: &'static str,
    pub name: &'static str,
    /// Una línea que dice qué hay ahí.
    pub caption: &'static str,
    pub url: &'static str,
    /// Dominios que pertenecen al servicio (incluye subdominios).
    pub domains: &'static [&'static str],
    /// ¿Necesita DRM (Widevine o PlayReady) para reproducir?
    pub drm: bool,
    pub agent: Agent,
    /// Color de la marca fina junto al nombre.
    pub accent: &'static str,
    /// Servicio de música: mientras está abierto, la barra de música vive en
    /// la barra de tareas. Para añadir uno nuevo basta con marcarlo aquí.
    pub music: bool,
}

/// En el orden en que aparecen; la posición es también la tecla (1–8).
pub const SERVICES: &[Service] = &[
    Service {
        id: "youtube",
        name: "YouTube",
        caption: "Videos y directos",
        url: "https://www.youtube.com/",
        domains: &["youtube.com", "youtu.be"],
        drm: false,
        agent: Agent::Chrome,
        accent: "#ff4e45",
        music: false,
    },
    Service {
        id: "youtube-music",
        name: "YouTube Music",
        caption: "Música y podcasts",
        url: "https://music.youtube.com/",
        domains: &["music.youtube.com"],
        drm: false,
        agent: Agent::Chrome,
        accent: "#ff6f61",
        music: true,
    },
    Service {
        id: "netflix",
        name: "Netflix",
        caption: "Series y películas",
        url: "https://www.netflix.com/browse",
        domains: &["netflix.com"],
        drm: true,
        agent: Agent::Edge,
        accent: "#e50914",
        music: false,
    },
    Service {
        id: "crunchyroll",
        name: "Crunchyroll",
        caption: "Anime",
        url: "https://www.crunchyroll.com/",
        domains: &["crunchyroll.com"],
        drm: true,
        agent: Agent::Edge,
        accent: "#f47521",
        music: false,
    },
    Service {
        id: "prime-video",
        name: "Prime Video",
        caption: "Series y películas",
        url: "https://www.primevideo.com/",
        domains: &["primevideo.com"],
        drm: true,
        agent: Agent::Edge,
        accent: "#1f9bde",
        music: false,
    },
    Service {
        id: "disney-plus",
        name: "Disney+",
        caption: "Series y películas",
        url: "https://www.disneyplus.com/",
        domains: &["disneyplus.com"],
        drm: true,
        agent: Agent::Edge,
        accent: "#2fb5c4",
        music: false,
    },
    Service {
        id: "hbo-max",
        name: "HBO Max",
        caption: "Series y películas",
        url: "https://www.hbomax.com/",
        domains: &["hbomax.com", "max.com"],
        drm: true,
        agent: Agent::Edge,
        accent: "#8a5cf6",
        music: false,
    },
    Service {
        id: "twitch",
        name: "Twitch",
        caption: "Directos",
        url: "https://www.twitch.tv/",
        domains: &["twitch.tv"],
        drm: false,
        agent: Agent::Edge,
        accent: "#a970ff",
        music: false,
    },
];

/// Sitios de música que no están en el menú pero se pueden abrir con
/// "Otro sitio": también tienen barra de música.
const OTHER_MUSIC_SITES: &[(&str, &str)] = &[
    ("open.spotify.com", "Spotify"),
    ("soundcloud.com", "SoundCloud"),
    ("music.apple.com", "Apple Music"),
    ("deezer.com", "Deezer"),
    ("listen.tidal.com", "TIDAL"),
    ("music.amazon.com", "Amazon Music"),
];

fn host_matches(host: &str, domain: &str) -> bool {
    host == domain || host.ends_with(&format!(".{domain}"))
}

/// Si `host` es un servicio de música, su nombre.
pub fn music_source(host: &str) -> Option<&'static str> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    let url = Url::parse(&format!("https://{host}/")).ok()?;
    match for_url(&url) {
        Some(service) if service.music => Some(service.name),
        // Un servicio del catálogo que no es de música (youtube.com) manda
        // sobre la lista de abajo.
        Some(_) => None,
        None => OTHER_MUSIC_SITES.iter().find(|(domain, _)| host_matches(&host, domain)).map(|(_, name)| *name),
    }
}

pub fn by_id(id: &str) -> Option<&'static Service> {
    SERVICES.iter().find(|s| s.id == id)
}

/// Servicio al que pertenece una URL. Se elige el dominio más específico
/// (music.youtube.com antes que youtube.com).
pub fn for_url(url: &Url) -> Option<&'static Service> {
    let host = url.host_str()?.to_ascii_lowercase();
    SERVICES
        .iter()
        .flat_map(|s| s.domains.iter().map(move |d| (s, *d)))
        .filter(|(_, domain)| host_matches(&host, domain))
        .max_by_key(|(_, domain)| domain.len())
        .map(|(service, _)| service)
}

/// Valida una dirección escrita por el usuario ("Otro sitio"). Acepta
/// http(s) y añade https:// si falta. Rechaza todo lo demás (javascript:,
/// file:, la propia pantalla de inicio...).
pub fn parse_user_url(text: &str) -> Option<Url> {
    let text = text.trim();
    if text.is_empty() || text.len() > 2048 || text.chars().any(char::is_whitespace) {
        return None;
    }
    let with_scheme;
    let candidate = if text.contains("://") {
        text
    } else {
        with_scheme = format!("https://{text}");
        &with_scheme
    };
    let url = Url::parse(candidate).ok()?;
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    let host = url.host_str()?.to_ascii_lowercase();
    // Sin punto no es un sitio (evita "https://hola"); y nunca nuestro host.
    if host == HOME_HOST || (!host.contains('.') && host != "localhost") {
        return None;
    }
    Some(url)
}

/// Servicio con el que arrancar, o `None` para mostrar el menú.
pub fn startup_service(start_with_last: bool, last: Option<&str>) -> Option<&'static Service> {
    if start_with_last {
        last.and_then(by_id)
    } else {
        None
    }
}

/// Identificador válido en el protocolo IPC (antes de buscarlo).
pub fn is_service_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_lowercase() || b == b'-')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn catalog_is_consistent() {
        assert_eq!(SERVICES.len(), 8, "la pantalla de inicio asigna las teclas 1–8");
        for (i, s) in SERVICES.iter().enumerate() {
            assert!(is_service_id(s.id), "{}", s.id);
            assert!(SERVICES[..i].iter().all(|o| o.id != s.id), "id repetido {}", s.id);
            let parsed = url(s.url);
            assert_eq!(parsed.scheme(), "https", "{}", s.id);
            assert_eq!(for_url(&parsed).map(|f| f.id), Some(s.id), "la URL de {} es suya", s.id);
            assert!(s.accent.starts_with('#') && s.accent.len() == 7);
        }
    }

    #[test]
    fn google_services_use_chrome_agent_and_drm_ones_edge() {
        for s in SERVICES {
            let google = s.domains.iter().any(|d| d.contains("youtube") || *d == "youtu.be");
            assert_eq!(s.agent == Agent::Chrome, google, "{}", s.id);
            if s.drm {
                assert_eq!(s.agent, Agent::Edge, "{}", s.id);
            }
        }
    }

    #[test]
    fn matches_most_specific_domain() {
        assert_eq!(for_url(&url("https://music.youtube.com/watch?v=x")).unwrap().id, "youtube-music");
        assert_eq!(for_url(&url("https://www.youtube.com/watch?v=x")).unwrap().id, "youtube");
        assert_eq!(for_url(&url("https://m.youtube.com/")).unwrap().id, "youtube");
        assert_eq!(for_url(&url("https://www.netflix.com/watch/123")).unwrap().id, "netflix");
        assert_eq!(for_url(&url("https://play.hbomax.com/")).unwrap().id, "hbo-max");
        assert_eq!(for_url(&url("https://play.max.com/")).unwrap().id, "hbo-max");
    }

    #[test]
    fn rejects_lookalike_domains() {
        for bad in [
            "https://netflix.com.evil.com/",
            "https://evilnetflix.com/",
            "https://youtube.co/",
            "https://notcrunchyroll.com/",
        ] {
            assert_eq!(for_url(&url(bad)).map(|s| s.id), None, "{bad}");
        }
    }

    #[test]
    fn music_services_get_the_taskbar_bar() {
        assert_eq!(music_source("music.youtube.com"), Some("YouTube Music"));
        assert_eq!(music_source("MUSIC.YOUTUBE.COM."), Some("YouTube Music"));
        assert_eq!(music_source("open.spotify.com"), Some("Spotify"));
        assert_eq!(music_source("m.soundcloud.com"), Some("SoundCloud"));
        assert_eq!(music_source("www.deezer.com"), Some("Deezer"));
        for not_music in ["www.youtube.com", "youtube.com", "www.netflix.com", "www.twitch.tv", "spotify.com.evil.com",
                          "notspotify.com", "", "ytmp.localhost", "con espacios"] {
            assert_eq!(music_source(not_music), None, "{not_music}");
        }
        // En el catálogo, solo YouTube Music por ahora.
        let music: Vec<_> = SERVICES.iter().filter(|s| s.music).map(|s| s.id).collect();
        assert_eq!(music, ["youtube-music"]);
    }

    #[test]
    fn user_urls() {
        assert_eq!(parse_user_url("plex.tv/web").unwrap().as_str(), "https://plex.tv/web");
        assert_eq!(parse_user_url(" http://192.168.1.20:8096/ ").unwrap().as_str(), "http://192.168.1.20:8096/");
        assert_eq!(parse_user_url("http://localhost:32400/web").unwrap().host_str(), Some("localhost"));
        for bad in [
            "",
            "hola",
            "hola mundo.com",
            "javascript:alert(1)",
            "file:///C:/Windows/win.ini",
            "chrome-extension://abc/x.html",
            "https://ytmp.localhost/",
            "ftp://example.com/",
        ] {
            assert!(parse_user_url(bad).is_none(), "{bad}");
        }
    }

    #[test]
    fn startup_choice() {
        assert_eq!(startup_service(false, Some("netflix")).map(|s| s.id), None);
        assert_eq!(startup_service(true, Some("netflix")).map(|s| s.id), Some("netflix"));
        assert_eq!(startup_service(true, None).map(|s| s.id), None);
        assert_eq!(startup_service(true, Some("borrado")).map(|s| s.id), None);
        assert_eq!(Url::parse(HOME_URL).unwrap().host_str(), Some(HOME_HOST));
    }

    #[test]
    fn service_ids_are_validated() {
        assert!(is_service_id("prime-video"));
        for bad in ["", "Netflix", "net flix", "../x", "a".repeat(40).as_str()] {
            assert!(!is_service_id(bad), "{bad}");
        }
        assert!(by_id("netflix").is_some());
        assert!(by_id("nope").is_none());
    }
}
