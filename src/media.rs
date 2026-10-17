//! Estado de la reproducción (lo informa media.js) y acciones sobre ella.
//!
//! El estado viene de la página, así que se trata como dato no confiable:
//! textos acotados y sin caracteres de control, números finitos, y la
//! miniatura solo si es una URL https.

use serde::{Deserialize, Serialize};
use url::Url;

/// Acciones sobre el reproductor. Se ejecutan en la página con
/// `window.__ytmpMedia(nombre, argumento)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaAction {
    PlayPause,
    /// Solo pausa (al bloquear la app). No llega desde la barra.
    Pause,
    Next,
    Previous,
    /// Segundos, positivos o negativos.
    SeekBy(i16),
    /// Segundo exacto.
    SeekTo(u32),
    /// Porcentaje 0–100.
    Volume(u8),
    /// Puntos porcentuales, positivos o negativos.
    VolumeBy(i8),
    ToggleMute,
}

impl MediaAction {
    /// Script que la ejecuta en la página (no hace nada si media.js no está).
    pub fn script(self) -> String {
        let (name, arg) = match self {
            MediaAction::PlayPause => ("toggle", String::new()),
            MediaAction::Pause => ("pause", String::new()),
            MediaAction::Next => ("next", String::new()),
            MediaAction::Previous => ("previous", String::new()),
            MediaAction::SeekBy(s) => ("seek-by", s.to_string()),
            MediaAction::SeekTo(s) => ("seek-to", s.to_string()),
            MediaAction::Volume(v) => ("volume", format!("{:.2}", f64::from(v.min(100)) / 100.0)),
            MediaAction::VolumeBy(d) => ("volume-by", format!("{:.2}", f64::from(d) / 100.0)),
            MediaAction::ToggleMute => ("mute", String::new()),
        };
        let arg = if arg.is_empty() { "undefined".to_owned() } else { arg };
        format!("window.__ytmpMedia && window.__ytmpMedia('{name}', {arg})")
    }

    /// Lee la acción de un mensaje de la barra de música ("toggle", "seek-by:-10"...).
    pub fn parse(text: &str) -> Option<Self> {
        let (name, arg) = match text.split_once(':') {
            Some((name, arg)) => (name, Some(arg)),
            None => (text, None),
        };
        Some(match (name, arg) {
            ("toggle", None) => MediaAction::PlayPause,
            ("next", None) => MediaAction::Next,
            ("previous", None) => MediaAction::Previous,
            ("mute", None) => MediaAction::ToggleMute,
            ("seek-by", Some(a)) => MediaAction::SeekBy(a.parse().ok().filter(|s: &i16| s.unsigned_abs() <= 600)?),
            ("seek-to", Some(a)) => MediaAction::SeekTo(a.parse().ok().filter(|s| *s <= 10_000_000)?),
            ("volume", Some(a)) => MediaAction::Volume(a.parse().ok().filter(|v| *v <= 100)?),
            ("volume-by", Some(a)) => MediaAction::VolumeBy(a.parse().ok().filter(|d: &i8| d.unsigned_abs() <= 100)?),
            _ => return None,
        })
    }
}

/// Lo que suena ahora mismo, tal como lo muestra la app.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MediaState {
    pub has_media: bool,
    pub title: String,
    pub artist: String,
    pub artwork: String,
    pub playing: bool,
    /// Segundos.
    pub position: f64,
    /// Segundos; 0 si se desconoce.
    pub duration: f64,
    pub live: bool,
    /// 0.0–1.0.
    pub volume: f64,
    pub muted: bool,
    pub can_next: bool,
    pub can_previous: bool,
    pub can_seek: bool,
    /// Sitio de la página (`music.youtube.com`): decide si hay barra de música.
    pub site: String,
    /// ¿La página está informando la posición cada segundo?
    pub watching: bool,
}

/// Tope del mensaje "media-state:" (el JSON real ocupa < 1 KB).
pub const MAX_REPORT_BYTES: usize = 16 * 1024;
const MAX_TITLE: usize = 200;
const MAX_ARTIST: usize = 120;
const MAX_URL: usize = 2048;

impl MediaState {
    /// Lee y limpia el informe de la página.
    pub fn from_report(json: &str) -> Option<Self> {
        if json.len() > MAX_REPORT_BYTES {
            return None;
        }
        serde_json::from_str::<Self>(json).ok().map(Self::sanitized)
    }

    pub fn sanitized(mut self) -> Self {
        self.title = clean_text(&self.title, MAX_TITLE);
        self.artist = clean_text(&self.artist, MAX_ARTIST);
        self.artwork = safe_artwork(&self.artwork);
        self.site = clean_host(&self.site);
        let seconds = |n: f64| if n.is_finite() { n.clamp(0.0, 1e7) } else { 0.0 };
        self.position = seconds(self.position);
        self.duration = seconds(self.duration);
        if self.duration > 0.0 {
            self.position = self.position.min(self.duration);
        }
        self.volume = if self.volume.is_finite() { self.volume.clamp(0.0, 1.0) } else { 1.0 };
        if !self.has_media {
            self.playing = false;
        }
        self
    }

    /// Volumen efectivo en porcentaje (0 si está en silencio).
    pub fn volume_percent(&self) -> u8 {
        if self.muted {
            0
        } else {
            (self.volume * 100.0).round() as u8
        }
    }

    /// Texto del icono de la bandeja (Windows corta a 127 caracteres).
    pub fn tooltip(&self) -> String {
        const APP: &str = crate::platform::APP_NAME;
        if !self.has_media || self.title.is_empty() {
            return APP.to_owned();
        }
        let icon = if self.playing { "▶" } else { "⏸" };
        let room = 127 - APP.chars().count() - 3;
        format!("{icon} {}\n{APP}", truncate(&self.title, room))
    }
}

/// Sin caracteres de control, espacios colapsados, como mucho `max` caracteres.
fn clean_text(text: &str, max: usize) -> String {
    let collapsed = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    truncate(&collapsed, max)
}

pub fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut cut: String = text.chars().take(max.saturating_sub(1)).collect();
    cut.push('…');
    cut
}

/// Un nombre de host en minúsculas, o vacío si trae algo raro.
fn clean_host(host: &str) -> String {
    let host = host.to_ascii_lowercase();
    if host.len() <= 253 && host.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-') {
        host
    } else {
        String::new()
    }
}

fn safe_artwork(url: &str) -> String {
    if url.len() > MAX_URL {
        return String::new();
    }
    match Url::parse(url) {
        Ok(parsed) if parsed.scheme() == "https" && parsed.host_str().is_some() => parsed.into(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_round_trip_to_scripts() {
        assert_eq!(MediaAction::PlayPause.script(), "window.__ytmpMedia && window.__ytmpMedia('toggle', undefined)");
        assert_eq!(MediaAction::SeekBy(-10).script(), "window.__ytmpMedia && window.__ytmpMedia('seek-by', -10)");
        assert_eq!(MediaAction::Volume(55).script(), "window.__ytmpMedia && window.__ytmpMedia('volume', 0.55)");
        assert_eq!(MediaAction::VolumeBy(-10).script(), "window.__ytmpMedia && window.__ytmpMedia('volume-by', -0.10)");
        assert_eq!(MediaAction::Volume(250).script(), "window.__ytmpMedia && window.__ytmpMedia('volume', 1.00)");
    }

    #[test]
    fn parses_mini_actions() {
        assert_eq!(MediaAction::parse("toggle"), Some(MediaAction::PlayPause));
        assert_eq!(MediaAction::parse("seek-by:-10"), Some(MediaAction::SeekBy(-10)));
        assert_eq!(MediaAction::parse("seek-to:3600"), Some(MediaAction::SeekTo(3600)));
        assert_eq!(MediaAction::parse("volume:0"), Some(MediaAction::Volume(0)));
        assert_eq!(MediaAction::parse("volume-by:5"), Some(MediaAction::VolumeBy(5)));
        for bad in ["", "toggle:1", "seek-by:", "seek-by:9999", "seek-to:-1", "volume:101", "volume:x", "volume-by:-128", "play();"] {
            assert_eq!(MediaAction::parse(bad), None, "{bad}");
        }
    }

    #[test]
    fn report_is_sanitized() {
        let json = r#"{"hasMedia":true,"title":"  Hola\n\tmundo \u0007 ","artist":"Canal","artwork":"javascript:alert(1)",
            "playing":true,"position":500,"duration":120,"volume":3,"muted":false,"canNext":true,"extra":"ignorado"}"#;
        let state = MediaState::from_report(json).unwrap();
        assert_eq!(state.title, "Hola mundo");
        assert_eq!(state.artwork, "");
        assert_eq!(state.position, 120.0, "no pasa de la duración");
        assert_eq!(state.volume, 1.0);
        assert!(state.can_next && !state.can_seek);
        assert_eq!(state.site, "");

        let site = |s: &str| MediaState::from_report(&format!(r#"{{"site":{}}}"#, serde_json::to_string(s).unwrap())).unwrap().site;
        assert_eq!(site("Music.YouTube.com"), "music.youtube.com");
        for bad in ["music.youtube.com/x", "a b.com", "<script>", "música.com"] {
            assert_eq!(site(bad), "", "{bad}");
        }
        assert_eq!(site(&"a".repeat(300)), "");

        let long = format!(r#"{{"hasMedia":true,"title":"{}"}}"#, "a".repeat(500));
        assert_eq!(MediaState::from_report(&long).unwrap().title.chars().count(), MAX_TITLE);
        assert!(MediaState::from_report(&"x".repeat(MAX_REPORT_BYTES + 1)).is_none());
        assert!(MediaState::from_report("no es json").is_none());

        let ok = MediaState::from_report(r#"{"artwork":"https://i.ytimg.com/vi/x/mqdefault.jpg","playing":true}"#).unwrap();
        assert_eq!(ok.artwork, "https://i.ytimg.com/vi/x/mqdefault.jpg");
        assert!(!ok.playing, "sin medio no puede estar reproduciendo");
        for bad in ["http://i.ytimg.com/x.jpg", "data:image/png;base64,AAAA", "file:///C:/x.jpg", "https://"] {
            assert_eq!(safe_artwork(bad), "", "{bad}");
        }
    }

    #[test]
    fn non_finite_numbers_are_neutralized() {
        let state = MediaState { position: f64::NAN, duration: f64::INFINITY, volume: f64::NAN, ..Default::default() }.sanitized();
        assert_eq!((state.position, state.duration, state.volume), (0.0, 0.0, 1.0));
    }

    #[test]
    fn tooltip_and_volume() {
        let state = MediaState { has_media: true, title: "x".repeat(300), playing: true, volume: 0.42, ..Default::default() };
        let tip = state.tooltip();
        assert!(tip.starts_with("▶ "));
        assert!(tip.ends_with("\nYT Mini Player"));
        assert!(tip.chars().count() <= 127, "{}", tip.chars().count());
        assert_eq!(state.volume_percent(), 42);
        assert_eq!(MediaState { muted: true, ..state }.volume_percent(), 0);
        assert_eq!(MediaState::default().tooltip(), "YT Mini Player");
    }
}
