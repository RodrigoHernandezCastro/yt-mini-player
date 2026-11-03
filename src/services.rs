//! Catalog of streaming services and address validation.
//!
//! Each service declares the User-Agent it needs:
//! - `Chrome`: YouTube and YouTube Music. Google blocks signing in on
//!   embedded browsers that identify themselves as such.
//! - `Edge`: the rest. WebView2 *is* Edge, and saying so lets DRM services
//!   pick the protection system the engine offers.

use url::Url;

/// Address of the start screen. On Windows wry serves the `ytmp` protocol
/// as `https://ytmp.<path>`; https makes it a secure context (needed to
/// check DRM from the page).
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
    /// Brand name (never translated), or an i18n key for the app's own page.
    pub name: &'static str,
    /// i18n key of a line that says what is there.
    pub caption: &'static str,
    pub url: &'static str,
    /// Domains that belong to the service (subdomains included).
    pub domains: &'static [&'static str],
    /// Does it need DRM (Widevine or PlayReady) to play?
    pub drm: bool,
    pub agent: Agent,
    /// Colour of the thin mark next to the name.
    pub accent: &'static str,
    /// Music service: while it is open, the music bar lives in the taskbar.
    /// Marking a service here is all it takes to add one.
    pub music: bool,
}

/// In the order they appear; the position is also the key (1–8).
pub const SERVICES: &[Service] = &[
    Service {
        id: "youtube",
        name: "YouTube",
        caption: "service.videos_live",
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
        caption: "service.music_podcasts",
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
        caption: "service.shows_movies",
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
        caption: "service.anime",
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
        caption: "service.shows_movies",
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
        caption: "service.shows_movies",
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
        caption: "service.shows_movies",
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
        caption: "service.live",
        url: "https://www.twitch.tv/",
        domains: &["twitch.tv"],
        drm: false,
        agent: Agent::Edge,
        accent: "#a970ff",
        music: false,
    },
];

/// "My music": the user's own library (an app page, not a site).
pub const MUSIC_ID: &str = "music";
pub const MUSIC_URL: &str = "https://ytmp.localhost/music";
pub const MUSIC: Service = Service {
    id: MUSIC_ID,
    name: "service.music_name",
    caption: "service.music_caption",
    url: MUSIC_URL,
    domains: &[],
    drm: false,
    agent: Agent::Chrome,
    accent: "#c2b3ff",
    music: true,
};

/// Music sites that are not in the menu but can be opened with "Another
/// site": they get the music bar too.
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

/// If `host` is a music service, its name (in the current language for
/// the app's own page).
pub fn music_source(host: &str) -> Option<String> {
    let host = host.trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() {
        return None;
    }
    // On our host only the music page reports media.
    if host == HOME_HOST {
        return Some(crate::i18n::text(MUSIC.name));
    }
    let url = Url::parse(&format!("https://{host}/")).ok()?;
    match for_url(&url) {
        Some(service) if service.music => Some(service.name.to_owned()),
        // A catalog service that is not about music (youtube.com) wins over
        // the list below.
        Some(_) => None,
        None => OTHER_MUSIC_SITES.iter().find(|(domain, _)| host_matches(&host, domain)).map(|(_, name)| (*name).to_owned()),
    }
}

pub fn by_id(id: &str) -> Option<&'static Service> {
    SERVICES.iter().find(|s| s.id == id).or((id == MUSIC_ID).then_some(&MUSIC))
}

/// The service a URL belongs to. The most specific domain wins
/// (music.youtube.com before youtube.com).
pub fn for_url(url: &Url) -> Option<&'static Service> {
    let host = url.host_str()?.to_ascii_lowercase();
    SERVICES
        .iter()
        .flat_map(|s| s.domains.iter().map(move |d| (s, *d)))
        .filter(|(_, domain)| host_matches(&host, domain))
        .max_by_key(|(_, domain)| domain.len())
        .map(|(service, _)| service)
}

/// Validates an address typed by the user ("Another site"). Accepts
/// http(s) and adds https:// if missing. Rejects everything else
/// (javascript:, file:, the start screen itself...).
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
    // Without a dot it is not a site (avoids "https://hello"); never our host.
    if host == HOME_HOST || (!host.contains('.') && host != "localhost") {
        return None;
    }
    Some(url)
}

/// Service to start with, or `None` to show the menu.
pub fn startup_service(start_with_last: bool, last: Option<&str>) -> Option<&'static Service> {
    if start_with_last {
        last.and_then(by_id)
    } else {
        None
    }
}

/// A valid id in the IPC protocol (before looking it up).
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
        assert_eq!(SERVICES.len(), 8, "the start screen assigns keys 1–8");
        for (i, s) in SERVICES.iter().enumerate() {
            assert!(is_service_id(s.id), "{}", s.id);
            assert!(SERVICES[..i].iter().all(|o| o.id != s.id), "repeated id {}", s.id);
            assert!(s.caption.starts_with("service."), "{}", s.id);
            let parsed = url(s.url);
            assert_eq!(parsed.scheme(), "https", "{}", s.id);
            assert_eq!(for_url(&parsed).map(|f| f.id), Some(s.id), "the URL of {} is its own", s.id);
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
        let name = |s: &str| music_source(s);
        assert_eq!(name("music.youtube.com").as_deref(), Some("YouTube Music"));
        assert_eq!(name("MUSIC.YOUTUBE.COM.").as_deref(), Some("YouTube Music"));
        assert_eq!(name("open.spotify.com").as_deref(), Some("Spotify"));
        assert_eq!(name("m.soundcloud.com").as_deref(), Some("SoundCloud"));
        assert_eq!(name("www.deezer.com").as_deref(), Some("Deezer"));
        assert_eq!(name("ytmp.localhost"), Some(crate::i18n::text("service.music_name")));
        for not_music in ["www.youtube.com", "youtube.com", "www.netflix.com", "www.twitch.tv", "spotify.com.evil.com",
                          "notspotify.com", "", "ytmp.localhost.evil.com", "with spaces"] {
            assert_eq!(music_source(not_music), None, "{not_music}");
        }
        // In the catalog, only YouTube Music for now.
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
            "hello",
            "hello world.com",
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
        assert_eq!(startup_service(true, Some("deleted")).map(|s| s.id), None);
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
        assert_eq!(by_id("music").map(|s| s.url), Some(MUSIC_URL));
        assert!(is_service_id(MUSIC_ID));
        assert!(SERVICES.iter().all(|s| s.id != MUSIC_ID));
    }
}

#[cfg(test)]
#[path = "tests/services_cases.rs"]
mod cases;
