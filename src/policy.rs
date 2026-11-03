//! Security rules of the embedded browser (pure logic, tested).
//!
//! - Which navigations are allowed (schemes, http only on the local
//!   network, maximum overlay mode limited to the catalog).
//! - What to do with new windows (`window.open`, `target=_blank`).
//! - Authenticating IPC messages with the session token.

use std::net::IpAddr;

use url::{Host, Url};

use crate::services::{self, HOME_HOST};

/// Identity providers the catalog services use to sign in. Also allowed in
/// maximum overlay mode and as new windows.
const IDENTITY_DOMAINS: &[&str] = &[
    "accounts.google.com",
    "accounts.youtube.com",
    "google.com",
    "appleid.apple.com",
    "facebook.com",
    "login.live.com",
    "login.microsoftonline.com",
    "amazon.com",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Navigation {
    Allow,
    /// This one is blocked and the https version is opened instead.
    Upgrade(String),
    /// Blocked; the i18n key of the reason (for the notice).
    Block(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NewWindow {
    /// Opens in the window itself (services and signing in).
    SameWindow(String),
    /// Opens in the default browser.
    External(String),
    Deny,
}

fn host_in(host: &str, domains: &[&str]) -> bool {
    domains.iter().any(|d| host == *d || host.ends_with(&format!(".{d}")))
}

/// A catalog or sign-in domain (or our start screen).
pub fn is_trusted_host(host: &str) -> bool {
    let host = host.to_ascii_lowercase();
    host == HOME_HOST
        || host_in(&host, IDENTITY_DOMAINS)
        || services::SERVICES.iter().any(|s| host_in(&host, s.domains))
}

/// `localhost`, `*.local` or a private/link-local IP: home servers (Plex,
/// Jellyfin) that usually have no https.
fn is_local(url: &Url) -> bool {
    match url.host() {
        Some(Host::Domain(d)) => {
            let d = d.to_ascii_lowercase();
            d == "localhost" || d.ends_with(".local") || d.ends_with(".localhost")
        }
        Some(Host::Ipv4(ip)) => {
            let ip = IpAddr::V4(ip);
            ip.is_loopback() || matches!(ip, IpAddr::V4(v4) if v4.is_private() || v4.is_link_local())
        }
        Some(Host::Ipv6(ip)) => ip.is_loopback() || (ip.segments()[0] & 0xffc0) == 0xfe80 || (ip.segments()[0] & 0xfe00) == 0xfc00,
        None => false,
    }
}

/// May the main window navigate to `url`?
///
/// `overlay`: maximum overlay (UIAccess). That process can send keys and
/// clicks to administrator windows, so it only opens the catalog and its
/// sign-in pages.
pub fn navigation(url: &str, overlay: bool, extension_ids: &[String]) -> Navigation {
    let Ok(parsed) = Url::parse(url) else { return Navigation::Block("blocked.invalid_address") };
    match parsed.scheme() {
        "https" => {
            let host = parsed.host_str().unwrap_or_default();
            if overlay && !is_trusted_host(host) {
                Navigation::Block("blocked.overlay")
            } else {
                Navigation::Allow
            }
        }
        "http" if is_local(&parsed) && !overlay => Navigation::Allow,
        "http" => {
            let mut secure = parsed.clone();
            if secure.set_scheme("https").is_ok() && secure.port() == Some(80) {
                let _ = secure.set_port(None);
            }
            match navigation(secure.as_str(), overlay, extension_ids) {
                Navigation::Allow => Navigation::Upgrade(secure.into()),
                other => other,
            }
        }
        "about" if parsed.path() == "blank" => Navigation::Allow,
        "chrome-extension" => {
            let id = parsed.host_str().unwrap_or_default();
            if !overlay && extension_ids.iter().any(|e| e == id) {
                Navigation::Allow
            } else {
                Navigation::Block("blocked.extension")
            }
        }
        _ => Navigation::Block("blocked.scheme"),
    }
}

/// What to do with a new window the page asks for?
pub fn new_window(url: &str, overlay: bool) -> NewWindow {
    let Ok(parsed) = Url::parse(url) else { return NewWindow::Deny };
    if !matches!(parsed.scheme(), "http" | "https") {
        return NewWindow::Deny;
    }
    let host = parsed.host_str().unwrap_or_default();
    if is_trusted_host(host) {
        return match navigation(url, overlay, &[]) {
            Navigation::Allow => NewWindow::SameWindow(url.to_owned()),
            Navigation::Upgrade(secure) => NewWindow::SameWindow(secure),
            Navigation::Block(_) => NewWindow::Deny,
        };
    }
    if overlay {
        NewWindow::Deny
    } else {
        NewWindow::External(parsed.into())
    }
}

/// Constant-time comparison (timing does not reveal how many characters of
/// the token a guesser got right).
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Authenticated IPC message: `<token>|<message>`. Returns the message if
/// the token is this session's.
pub fn authenticate<'a>(body: &'a str, token: &str) -> Option<&'a str> {
    let (given, message) = body.split_once('|')?;
    constant_time_eq(given.as_bytes(), token.as_bytes()).then_some(message)
}

/// Session token in hex (128 bits from the system's generator).
pub fn new_token() -> std::io::Result<String> {
    let bytes = crate::vault::random_bytes::<16>()?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn https_is_allowed_and_other_schemes_are_not() {
        assert_eq!(navigation("https://www.youtube.com/watch?v=x", false, &[]), Navigation::Allow);
        assert_eq!(navigation("https://ytmp.localhost/", false, &[]), Navigation::Allow);
        assert_eq!(navigation("https://any-site.example/", false, &[]), Navigation::Allow);
        assert_eq!(navigation("about:blank", false, &[]), Navigation::Allow);
        for bad in [
            "file:///C:/Windows/win.ini",
            "javascript:alert(1)",
            "data:text/html,<script>1</script>",
            "ms-settings:privacy",
            "search-ms:query=x",
            "mailto:a@b.c",
            "edge://settings",
            "chrome://settings",
            "blob:https://x.com/123",
            "about:settings",
            "not a url",
        ] {
            assert!(matches!(navigation(bad, false, &[]), Navigation::Block(_)), "{bad}");
        }
    }

    #[test]
    fn plain_http_is_upgraded_except_on_the_local_network() {
        assert_eq!(navigation("http://www.netflix.com/browse", false, &[]), Navigation::Upgrade("https://www.netflix.com/browse".into()));
        assert_eq!(navigation("http://example.com:80/a?b=1", false, &[]), Navigation::Upgrade("https://example.com/a?b=1".into()));
        for local in [
            "http://localhost:32400/web",
            "http://192.168.1.20:8096/",
            "http://10.0.0.5/",
            "http://172.20.1.1/",
            "http://127.0.0.1:8080/",
            "http://nas.local/",
            "http://[::1]:8096/",
            "http://169.254.10.10/",
        ] {
            assert_eq!(navigation(local, false, &[]), Navigation::Allow, "{local}");
        }
        // 172.32 is no longer private.
        assert!(matches!(navigation("http://172.32.0.1/", false, &[]), Navigation::Upgrade(_)));
    }

    #[test]
    fn overlay_mode_only_opens_the_catalog_and_sign_in() {
        for ok in [
            "https://www.youtube.com/",
            "https://music.youtube.com/",
            "https://accounts.google.com/signin",
            "https://www.netflix.com/login",
            "https://sso.crunchyroll.com/login",
            "https://www.amazon.com/ap/signin",
            "https://ytmp.localhost/",
        ] {
            assert_eq!(navigation(ok, true, &[]), Navigation::Allow, "{ok}");
        }
        for bad in ["https://evil.example/", "http://192.168.1.20:8096/", "https://netflix.com.evil.example/"] {
            assert!(matches!(navigation(bad, true, &[]), Navigation::Block(_)), "{bad}");
        }
        let ext = vec!["abcdefghijklmnopabcdefghijklmnop".to_owned()];
        assert!(matches!(navigation("chrome-extension://abcdefghijklmnopabcdefghijklmnop/o.html", true, &ext), Navigation::Block(_)));
    }

    #[test]
    fn only_installed_extension_pages() {
        let ext = vec!["abcdefghijklmnopabcdefghijklmnop".to_owned()];
        assert_eq!(navigation("chrome-extension://abcdefghijklmnopabcdefghijklmnop/options.html", false, &ext), Navigation::Allow);
        assert!(matches!(navigation("chrome-extension://zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz/x.html", false, &ext), Navigation::Block(_)));
    }

    #[test]
    fn new_windows() {
        assert_eq!(new_window("https://accounts.google.com/o/oauth2", false), NewWindow::SameWindow("https://accounts.google.com/o/oauth2".into()));
        assert_eq!(new_window("http://www.youtube.com/watch?v=x", false), NewWindow::SameWindow("https://www.youtube.com/watch?v=x".into()));
        assert_eq!(new_window("https://shop.example/offer", false), NewWindow::External("https://shop.example/offer".into()));
        assert_eq!(new_window("https://shop.example/offer", true), NewWindow::Deny);
        for bad in ["javascript:alert(1)", "file:///C:/x", "ms-settings:", "about:blank", "x"] {
            assert_eq!(new_window(bad, false), NewWindow::Deny, "{bad}");
        }
    }

    #[test]
    fn ipc_messages_need_the_session_token() {
        let token = new_token().unwrap();
        assert_eq!(token.len(), 32);
        assert_ne!(token, new_token().unwrap());
        assert_eq!(authenticate(&format!("{token}|autostart"), &token), Some("autostart"));
        assert_eq!(authenticate(&format!("{token}|media-state:{{\"a\":\"x|y\"}}"), &token), Some("media-state:{\"a\":\"x|y\"}"));
        for forged in ["autostart", "|autostart", "0000|autostart", &format!("{}|quit", &token[..31]), &format!("{token}x|quit")] {
            assert_eq!(authenticate(forged, &token), None, "{forged}");
        }
    }
}

#[cfg(test)]
#[path = "tests/policy_cases.rs"]
mod cases;
