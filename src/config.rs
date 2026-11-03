//! Application paths and state files.
//!
//! Everything lives under `%LOCALAPPDATA%\yt_mini_player`:
//! - `profile\`        → WebView2 UserDataFolder (cookies, session, cache).
//!   Chromium encrypts cookies with a key protected by DPAPI.
//! - `extensions\`     → unpacked extensions, `.crx` or `.zip`.
//! - `vault.key`       → the vault key, wrapped by the TPM and DPAPI (vault.rs).
//! - `settings.enc`    → preferences, PIN (Argon2id hash), approved extensions.
//! - `window.enc`      → window position and size on exit.
//! - `extensions.enc`  → extension folder → id installed in WebView2.
//!
//! The `.enc` files are encrypted with AES-256-GCM. Older versions kept
//! plain `.json` files: they are migrated and deleted on first start.

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
};

use serde::{de::DeserializeOwned, Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::{ipc::MIN_OPACITY, vault::Vault};

const APP_DIR: &str = "yt_mini_player";

/// Smallest size accepted when restoring (avoids degenerate windows from a
/// corrupt or hand-edited file).
const MIN_RESTORED_WIDTH: u32 = 160;
const MIN_RESTORED_HEIGHT: u32 = 90;

pub const SETTINGS: &str = "settings";
pub const WINDOW: &str = "window";
pub const EXTENSIONS: &str = "extensions";

pub struct Paths {
    pub root: PathBuf,
    pub profile: PathBuf,
    pub extensions: PathBuf,
    pub vault_key: PathBuf,
    pub settings: PathBuf,
    pub window_state: PathBuf,
    pub extension_ids: PathBuf,
}

impl Paths {
    /// Resolves the paths and creates the folders if they don't exist.
    ///
    /// Setting the UserDataFolder is mandatory: without it WebView2 tries to
    /// create the folder next to the executable, which fails under
    /// `Program Files`.
    pub fn resolve() -> io::Result<Self> {
        let base = directories::BaseDirs::new().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "could not resolve %LOCALAPPDATA%")
        })?;
        Self::under(base.data_local_dir().join(APP_DIR))
    }

    pub fn under(root: PathBuf) -> io::Result<Self> {
        let paths = Self {
            profile: root.join("profile"),
            extensions: root.join("extensions"),
            vault_key: root.join("vault.key"),
            settings: root.join("settings.enc"),
            window_state: root.join("window.enc"),
            extension_ids: root.join("extensions.enc"),
            root,
        };
        fs::create_dir_all(&paths.profile)?;
        fs::create_dir_all(&paths.extensions)?;
        Ok(paths)
    }
}

/// Reads a plain JSON file (only to migrate files from older versions).
fn load_legacy_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

/// State files encrypted with the vault. Cloneable: the music service saves
/// its library from its own thread with the same key.
#[derive(Clone)]
pub struct Store {
    vault: Arc<Vault>,
}

impl Store {
    pub fn new(vault: Vault) -> Self {
        Self { vault: Arc::new(vault) }
    }

    /// The vault (to bind its key to the TPM from another thread).
    pub fn vault(&self) -> Arc<Vault> {
        self.vault.clone()
    }

    /// Reads and decrypts. If missing, migrates the plain `.json` of older
    /// versions (and deletes it). Any problem (tampered, another key,
    /// corrupt) returns `None`: the app carries on with defaults.
    pub fn load<T: DeserializeOwned + Serialize>(&self, path: &Path, purpose: &str) -> Option<T> {
        match self.vault.read(path, purpose) {
            Ok(Some(bytes)) => serde_json::from_slice(&bytes).ok(),
            Ok(None) => {
                let legacy = path.with_extension("json");
                let value: T = load_legacy_json(&legacy)?;
                if self.save(path, purpose, &value).is_ok() {
                    let _ = fs::remove_file(&legacy);
                }
                Some(value)
            }
            Err(err) => {
                eprintln!("could not read {}: {err}", path.display());
                None
            }
        }
    }

    pub fn save<T: Serialize>(&self, path: &Path, purpose: &str, value: &T) -> io::Result<()> {
        let json = Zeroizing::new(serde_json::to_vec(value).map_err(io::Error::other)?);
        self.vault.write(path, purpose, &json)
    }
}

/// How the app is unlocked (lock.rs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LockMode {
    #[default]
    Off,
    Hello,
    Pin,
}

/// User preferences. Missing fields take their default value, so adding
/// fields in future versions does not break older files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub always_on_top: bool,
    /// Percent, between `MIN_OPACITY` and 100.
    pub opacity: u8,
    pub video_only: bool,
    /// A video's own full-screen button fills the player window instead of
    /// the screen (ui.js turns it into "full screen in the window").
    pub fullscreen_in_window: bool,
    /// Last service opened (catalog id).
    pub last_service: Option<String>,
    /// On start, open the last service straight away instead of the menu.
    pub start_with_last: bool,
    /// Music bar in the taskbar while a music service is open.
    pub taskbar_bar: bool,
    /// How far the music bar was dragged to the left (logical px).
    pub bar_shift: i32,
    /// App lock.
    pub lock: LockMode,
    /// The PIN as an Argon2id PHC string (never the PIN itself).
    pub pin_hash: Option<String>,
    /// Failed attempts in a row and until when to wait (Unix seconds).
    pub lock_failures: u32,
    pub lock_blocked_until: u64,
    /// Approved or rejected extensions: folder → SHA-256 fingerprint of its
    /// content. If the extension changes, the user is asked again.
    pub approved_extensions: BTreeMap<String, String>,
    pub rejected_extensions: BTreeMap<String, String>,
    /// Interface language the user picked (`en`, `es`, `ru`, `pl`, `ja`);
    /// `None` follows the Windows display language.
    pub language: Option<String>,
    /// Watch together: the name others see, the relay typed by hand (when
    /// the build has none) and whether a session shares the host's window
    /// (or syncs everyone's players).
    pub together_name: String,
    pub together_relay: Option<String>,
    pub together_share: bool,
    /// WebView2 may use the graphics card (applies on the next start).
    pub hardware_acceleration: bool,
    /// The first-run welcome on the start screen was seen (or skipped).
    pub welcome_seen: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            always_on_top: true,
            opacity: 100,
            video_only: false,
            fullscreen_in_window: false,
            last_service: None,
            start_with_last: false,
            taskbar_bar: true,
            bar_shift: 0,
            lock: LockMode::Off,
            pin_hash: None,
            lock_failures: 0,
            lock_blocked_until: 0,
            approved_extensions: BTreeMap::new(),
            rejected_extensions: BTreeMap::new(),
            language: None,
            together_name: String::new(),
            together_relay: None,
            together_share: true,
            hardware_acceleration: true,
            welcome_seen: false,
        }
    }
}

impl Settings {
    pub fn load(store: &Store, path: &Path) -> Self {
        let mut settings: Self = store.load(path, SETTINGS).unwrap_or_default();
        settings.opacity = settings.opacity.clamp(MIN_OPACITY, 100);
        settings.bar_shift = settings.bar_shift.clamp(0, 10_000);
        // An id that is no longer in the catalog is forgotten.
        if settings.last_service.as_deref().is_some_and(|id| crate::services::by_id(id).is_none()) {
            settings.last_service = None;
        }
        // PIN lock without a saved PIN (damaged file): no lock at all would be
        // silently unsafe, so it switches to Windows Hello.
        if settings.lock == LockMode::Pin && settings.pin_hash.is_none() {
            settings.lock = LockMode::Hello;
        }
        settings.together_name = crate::together::protocol::clean_name(&settings.together_name);
        if settings.together_relay.as_deref().is_some_and(|r| crate::together::link::parse_relay(r).is_none()) {
            settings.together_relay = None;
        }
        // A language this version does not have: follow Windows instead.
        if settings.language.as_deref().is_some_and(|code| crate::i18n::Lang::from_tag(code).is_none()) {
            settings.language = None;
        }
        settings
    }

    pub fn save(&self, store: &Store, path: &Path) -> io::Result<()> {
        store.save(path, SETTINGS, self)
    }

    /// The language to show: the one picked, or else the Windows one.
    pub fn language(&self) -> crate::i18n::Lang {
        self.language.as_deref().and_then(crate::i18n::Lang::from_tag).unwrap_or_else(crate::i18n::detect)
    }
}

/// Extension folder → id WebView2 gave it on install.
pub type ExtensionIds = BTreeMap<String, String>;

pub fn load_extension_ids(store: &Store, path: &Path) -> ExtensionIds {
    store.load(path, EXTENSIONS).unwrap_or_default()
}

pub fn save_extension_ids(store: &Store, path: &Path, ids: &ExtensionIds) -> io::Result<()> {
    store.save(path, EXTENSIONS, ids)
}

/// Geometry in physical pixels, as the system reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl WindowGeometry {
    /// `None` if the file is missing, unreadable or its content is not
    /// sensible. Never a fatal error.
    pub fn load(store: &Store, path: &Path) -> Option<Self> {
        let geometry: Self = store.load(path, WINDOW)?;
        (geometry.width >= MIN_RESTORED_WIDTH && geometry.height >= MIN_RESTORED_HEIGHT)
            .then_some(geometry)
    }

    pub fn save(&self, store: &Store, path: &Path) -> io::Result<()> {
        store.save(path, WINDOW, self)
    }

    /// Minimum overlap, in pixels per axis, between the window and a monitor
    /// for the user to be able to reach it with the mouse.
    pub const MIN_VISIBLE: i32 = 64;

    /// Does this geometry overlap the given monitor rectangle (x, y, width,
    /// height) enough?
    pub fn overlaps(&self, monitor: (i32, i32, u32, u32)) -> bool {
        let (mx, my, mw, mh) = monitor;
        let overlap_x = (mx + mw as i32).min(self.x + self.width as i32) - mx.max(self.x);
        let overlap_y = (my + mh as i32).min(self.y + self.height as i32) - my.max(self.y);
        overlap_x >= Self::MIN_VISIBLE && overlap_y >= Self::MIN_VISIBLE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MONITOR: (i32, i32, u32, u32) = (0, 0, 1920, 1080);

    fn geom(x: i32, y: i32) -> WindowGeometry {
        WindowGeometry { x, y, width: 640, height: 360 }
    }

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ytmp-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn store(dir: &Path) -> Store {
        let (vault, _) = Vault::open(&dir.join("vault.key"), &crate::vault::tests::TestProtector(9)).unwrap();
        Store::new(vault)
    }

    #[test]
    fn inside_monitor_overlaps() {
        assert!(geom(100, 100).overlaps(MONITOR));
    }

    #[test]
    fn barely_visible_is_rejected() {
        assert!(!geom(1910, 100).overlaps(MONITOR));
    }

    #[test]
    fn partially_visible_is_accepted() {
        assert!(geom(1720, 100).overlaps(MONITOR));
    }

    #[test]
    fn disconnected_second_monitor_is_rejected() {
        assert!(!geom(2200, 100).overlaps(MONITOR));
    }

    #[test]
    fn negative_coordinates_on_left_monitor() {
        assert!(geom(-1500, 200).overlaps((-1920, 0, 1920, 1080)));
    }

    #[test]
    fn geometry_round_trip_and_validation() {
        let dir = temp_dir("geom");
        let store = store(&dir);
        let path = dir.join("window.enc");

        let g = geom(10, 20);
        g.save(&store, &path).unwrap();
        assert_eq!(WindowGeometry::load(&store, &path), Some(g));
        assert!(!fs::read(&path).unwrap().windows(5).any(|w| w == b"width"), "encrypted on disk");

        store.save(&path, WINDOW, &WindowGeometry { x: 0, y: 0, width: 5, height: 5 }).unwrap();
        assert_eq!(WindowGeometry::load(&store, &path), None);

        fs::write(&path, "not from the vault").unwrap();
        assert_eq!(WindowGeometry::load(&store, &path), None);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn settings_defaults_partial_files_and_clamping() {
        let dir = temp_dir("settings");
        let store = store(&dir);
        let path = dir.join("settings.enc");
        let legacy = dir.join("settings.json");

        assert_eq!(Settings::load(&store, &path), Settings::default());

        // Migration: the plain JSON of older versions is encrypted and deleted.
        fs::write(&legacy, r#"{"video_only":true,"opacity":3}"#).unwrap();
        let s = Settings::load(&store, &path);
        assert!(s.video_only && s.always_on_top && s.opacity == MIN_OPACITY);
        assert!(!legacy.exists(), "the plain file is deleted after migrating");
        assert!(path.exists() && Settings::load(&store, &path).video_only);

        let s = Settings {
            always_on_top: false,
            opacity: 70,
            video_only: true,
            fullscreen_in_window: true,
            last_service: Some("crunchyroll".into()),
            start_with_last: true,
            taskbar_bar: false,
            bar_shift: 120,
            lock: LockMode::Pin,
            pin_hash: Some("$argon2id$v=19$m=19456,t=2,p=1$c2FsdA$aGFzaA".into()),
            lock_failures: 3,
            lock_blocked_until: 1_800_000_000,
            approved_extensions: [("dark".to_owned(), "ab".repeat(32))].into(),
            rejected_extensions: BTreeMap::new(),
            language: Some("pl".into()),
            together_name: "Ana".into(),
            together_relay: Some("wss://ytmp-together.example.workers.dev/".into()),
            together_share: false,
            hardware_acceleration: false,
            welcome_seen: true,
        };
        s.save(&store, &path).unwrap();
        assert_eq!(Settings::load(&store, &path), s);
        // A relay address that is not one is forgotten; a name is cleaned.
        let odd = Settings { together_relay: Some("http://x".into()), together_name: " a\u{0}b ".into(), ..s.clone() };
        odd.save(&store, &path).unwrap();
        let loaded = Settings::load(&store, &path);
        assert_eq!((loaded.together_relay, loaded.together_name.as_str()), (None, "a b"));
        let raw = fs::read(&path).unwrap();
        assert!(!raw.windows(8).any(|w| w == b"argon2id"), "the PIN hash is not readable either");

        // Fields missing in old files: the bar is on, no lock.
        store.save(&path, SETTINGS, &serde_json::json!({"always_on_top": false})).unwrap();
        let s = Settings::load(&store, &path);
        assert!(s.taskbar_bar && s.bar_shift == 0 && s.lock == LockMode::Off);
        assert!(s.hardware_acceleration && s.together_share, "on unless turned off");
        assert!(!s.welcome_seen, "the welcome shows once after updating too");
        // 0.9's setting is simply forgotten.
        store.save(&path, SETTINGS, &serde_json::json!({"together_control": true})).unwrap();
        assert_eq!(Settings::load(&store, &path), Settings::default());
        store.save(&path, SETTINGS, &serde_json::json!({"bar_shift": -40, "last_service": "deleted-service", "language": "xx"})).unwrap();
        let s = Settings::load(&store, &path);
        assert_eq!((s.bar_shift, s.last_service, s.language), (0, None, None));
        // PIN lock without a hash: it does not end up unlocked, it moves to Windows Hello.
        store.save(&path, SETTINGS, &serde_json::json!({"lock": "pin"})).unwrap();
        assert_eq!(Settings::load(&store, &path).lock, LockMode::Hello);

        // Tampered file: ignored (defaults), nothing breaks.
        let mut raw = fs::read(&path).unwrap();
        let last = raw.len() - 1;
        raw[last] ^= 0xff;
        fs::write(&path, raw).unwrap();
        assert_eq!(Settings::load(&store, &path), Settings::default());

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn paths_create_directories() {
        let dir = temp_dir("paths");
        let paths = Paths::under(dir.join("app")).unwrap();
        assert!(paths.profile.is_dir() && paths.extensions.is_dir());
        assert_eq!(paths.settings, dir.join("app").join("settings.enc"));
        assert_eq!(paths.root, dir.join("app"));
        fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
#[path = "tests/config_cases.rs"]
mod cases;
