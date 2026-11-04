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
//! The `.enc` files are encrypted with AES-256-GCM. Plain `.json` files
//! next to them are never read: anyone who can write to the folder could
//! otherwise turn the lock off or approve an extension.
//!
//! The profile and the settings belong together ([`check_profile`]): a
//! random marker sits in both. Settings that cannot be read (deleted, from
//! another key) or a marker that does not match mean someone may be trying
//! to get past the lock, so the saved sign-ins are wiped before any page
//! opens. Whoever does not know the PIN takes nothing away.

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

    /// Reads and decrypts. Missing, or any problem (tampered, another key,
    /// corrupt), returns `None`: the app carries on with defaults.
    pub fn load<T: DeserializeOwned>(&self, path: &Path, purpose: &str) -> Option<T> {
        match self.vault.read(path, purpose) {
            Ok(Some(bytes)) => serde_json::from_slice(&bytes).ok(),
            Ok(None) => None,
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
    /// The same random text as the profile's marker file (see
    /// [`check_profile`]); new whenever the lock is turned on.
    pub profile_marker: Option<String>,
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
            profile_marker: None,
        }
    }
}

impl Settings {
    /// The app starts with [`Settings::load_checked`] (it needs to know).
    #[cfg(test)]
    pub fn load(store: &Store, path: &Path) -> Self {
        Self::load_checked(store, path).0
    }

    /// The settings, and whether they were really read (`false`: missing,
    /// tampered or from another key, so these are the defaults).
    pub fn load_checked(store: &Store, path: &Path) -> (Self, bool) {
        let loaded: Option<Self> = store.load(path, SETTINGS);
        let read = loaded.is_some();
        let mut settings = loaded.unwrap_or_default();
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
        (settings, read)
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

/// The marker file inside the profile folder.
pub const PROFILE_MARKER: &str = "ytmp-profile.id";

/// What to do with the profile at start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProfileCheck {
    /// The profile is the one these settings belong to.
    Matches,
    /// A new marker in both, nothing wiped: an empty profile, or settings
    /// read fine from a version that had no marker yet.
    Bind,
    /// Wipe the profile (every saved sign-in), then a new marker.
    Wipe,
}

/// The rule (pure, tested): `read` says whether the settings were really
/// read, `in_settings` and `in_profile` are the two markers, `has_data`
/// whether the profile holds anything but its marker.
pub fn check_profile(read: bool, in_settings: Option<&str>, in_profile: Option<&str>, has_data: bool) -> ProfileCheck {
    if read && in_settings.is_some() && in_settings == in_profile {
        ProfileCheck::Matches
    } else if !has_data || (read && in_settings.is_none() && in_profile.is_none()) {
        ProfileCheck::Bind
    } else {
        ProfileCheck::Wipe
    }
}

fn read_marker(profile: &Path) -> Option<String> {
    let text = fs::read_to_string(profile.join(PROFILE_MARKER)).ok()?;
    let text = text.trim();
    (text.len() == 32 && text.bytes().all(|b| b.is_ascii_hexdigit())).then(|| text.to_owned())
}

fn has_data(profile: &Path) -> bool {
    fs::read_dir(profile).is_ok_and(|entries| entries.flatten().any(|e| e.file_name() != PROFILE_MARKER))
}

/// A new marker, written to the profile and put in `settings` (the caller
/// saves them).
pub fn bind_profile(profile: &Path, settings: &mut Settings) -> io::Result<()> {
    let marker = crate::policy::new_token()?;
    crate::vault::write_atomic(&profile.join(PROFILE_MARKER), marker.as_bytes())?;
    settings.profile_marker = Some(marker);
    Ok(())
}

/// Checks the profile against the settings before any webview opens it,
/// and wipes it if they do not belong together. Returns what it did; the
/// caller saves the settings unless it was `Matches`. An error means the
/// profile could not be wiped: the app must not start with it.
pub fn secure_profile(profile: &Path, settings: &mut Settings, read: bool) -> io::Result<ProfileCheck> {
    let check = check_profile(read, settings.profile_marker.as_deref(), read_marker(profile).as_deref(), has_data(profile));
    if check == ProfileCheck::Wipe {
        wipe(profile)?;
    }
    if check != ProfileCheck::Matches {
        bind_profile(profile, settings)?;
    }
    Ok(check)
}

/// Deletes the profile folder. WebView2's processes from a copy that just
/// quit can hold files for a moment, so it tries for a few seconds.
fn wipe(profile: &Path) -> io::Result<()> {
    let mut last = None;
    for _ in 0..20 {
        match fs::remove_dir_all(profile) {
            Ok(()) => return fs::create_dir_all(profile),
            Err(err) if err.kind() == io::ErrorKind::NotFound => return fs::create_dir_all(profile),
            Err(err) => last = Some(err),
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    let err = last.unwrap_or_else(|| io::Error::other("unknown"));
    Err(io::Error::new(err.kind(), format!("could not clear the saved sign-ins in {}: {err}", profile.display())))
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

        // A plain JSON next to it is never read: it could turn the lock off.
        fs::write(&legacy, r#"{"video_only":true,"lock":"off","approved_extensions":{"evil":"00"}}"#).unwrap();
        assert_eq!(Settings::load_checked(&store, &path), (Settings::default(), false));
        assert!(!path.exists(), "nothing was adopted");
        Settings { video_only: true, opacity: 3, ..Settings::default() }.save(&store, &path).unwrap();
        let (s, read) = Settings::load_checked(&store, &path);
        assert!(read && s.video_only && s.always_on_top && s.opacity == MIN_OPACITY);

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
            profile_marker: Some("0123456789abcdef0123456789abcdef".into()),
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
        assert_eq!(Settings::load_checked(&store, &path), (Settings::default(), false));

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_profile_belongs_to_the_settings() {
        use ProfileCheck::*;
        let m = Some("a".repeat(32));
        let other = Some("b".repeat(32));
        // The same marker in both.
        assert_eq!(check_profile(true, m.as_deref(), m.as_deref(), true), Matches);
        // An empty profile (first start): nothing to protect.
        assert_eq!(check_profile(false, None, None, false), Bind);
        assert_eq!(check_profile(false, None, other.as_deref(), false), Bind);
        // Settings read fine from a version without markers: kept, bound.
        assert_eq!(check_profile(true, None, None, true), Bind);
        // Settings deleted, damaged or from another key, with sign-ins in
        // the profile: wiped, whatever the markers say.
        assert_eq!(check_profile(false, None, None, true), Wipe);
        assert_eq!(check_profile(false, None, m.as_deref(), true), Wipe);
        // Another profile put in its place, its marker deleted, or older
        // settings put back (a new marker each time the lock goes on).
        assert_eq!(check_profile(true, m.as_deref(), other.as_deref(), true), Wipe);
        assert_eq!(check_profile(true, m.as_deref(), None, true), Wipe);
        assert_eq!(check_profile(true, None, m.as_deref(), true), Wipe);
    }

    #[test]
    fn deleting_the_settings_wipes_the_sign_ins() {
        let dir = temp_dir("profile");
        let store = store(&dir);
        let paths = Paths::under(dir.join("app")).unwrap();
        // First start: an empty profile gets a marker.
        let (mut s, read) = Settings::load_checked(&store, &paths.settings);
        assert_eq!(secure_profile(&paths.profile, &mut s, read).unwrap(), ProfileCheck::Bind);
        s.lock = LockMode::Hello;
        s.save(&store, &paths.settings).unwrap();
        fs::create_dir_all(paths.profile.join("EBWebView/Default")).unwrap();
        fs::write(paths.profile.join("EBWebView/Default/Cookies"), "signed in").unwrap();
        // Next start: it matches, nothing changes.
        let (mut s, read) = Settings::load_checked(&store, &paths.settings);
        assert_eq!(secure_profile(&paths.profile, &mut s, read).unwrap(), ProfileCheck::Matches);
        assert_eq!(s.lock, LockMode::Hello);
        // Someone deletes settings.enc to get past the lock: the sign-ins go.
        fs::remove_file(&paths.settings).unwrap();
        let (mut s, read) = Settings::load_checked(&store, &paths.settings);
        assert_eq!(secure_profile(&paths.profile, &mut s, read).unwrap(), ProfileCheck::Wipe);
        assert!(!paths.profile.join("EBWebView").exists() && paths.profile.join(PROFILE_MARKER).is_file());
        s.save(&store, &paths.settings).unwrap();
        // Putting an old profile (or its cookies) back does not help either.
        fs::create_dir_all(paths.profile.join("EBWebView")).unwrap();
        fs::write(paths.profile.join(PROFILE_MARKER), "c".repeat(32)).unwrap();
        let (mut s, read) = Settings::load_checked(&store, &paths.settings);
        assert_eq!(secure_profile(&paths.profile, &mut s, read).unwrap(), ProfileCheck::Wipe);
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
