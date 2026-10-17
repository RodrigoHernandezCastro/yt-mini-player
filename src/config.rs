//! Rutas de la aplicación y archivos de estado.
//!
//! Todo vive bajo `%LOCALAPPDATA%\yt_mini_player`:
//! - `profile\`        → UserDataFolder de WebView2 (cookies, sesión, caché).
//!   Chromium cifra cookies y contraseñas con una clave protegida por DPAPI;
//!   además, la carpeta entera se cifra con EFS cuando Windows lo admite
//!   (security.rs).
//! - `extensions\`     → extensiones descomprimidas, `.crx` o `.zip`.
//! - `vault.key`       → clave de la bóveda, envuelta por DPAPI (vault.rs).
//! - `settings.enc`    → preferencias, PIN (hash Argon2id), extensiones aprobadas.
//! - `window.enc`      → posición y tamaño de la ventana al cerrar.
//! - `extensions.enc`  → carpeta de extensión → id instalado en WebView2.
//!
//! Los `.enc` van cifrados con AES-256-GCM. Las versiones anteriores
//! guardaban `.json` en claro: se migran y se borran al primer arranque.

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

/// Tamaño mínimo aceptado al restaurar (evita ventanas degeneradas por un
/// archivo corrupto o editado a mano).
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
    /// Resuelve las rutas y crea los directorios si no existen.
    ///
    /// Fijar el UserDataFolder es obligatorio: sin él, WebView2 intenta crear
    /// la carpeta junto al ejecutable, lo que falla bajo `Program Files`.
    pub fn resolve() -> io::Result<Self> {
        let base = directories::BaseDirs::new().ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, "no se pudo resolver %LOCALAPPDATA%")
        })?;
        Self::under(base.data_local_dir().join(APP_DIR))
    }

    fn under(root: PathBuf) -> io::Result<Self> {
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

/// Lee un JSON en claro (solo para migrar archivos de versiones anteriores).
fn load_legacy_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

/// Archivos de estado cifrados con la bóveda. Se puede clonar: la música
/// guarda su biblioteca desde su propio hilo con la misma clave.
#[derive(Clone)]
pub struct Store {
    vault: Arc<Vault>,
}

impl Store {
    pub fn new(vault: Vault) -> Self {
        Self { vault: Arc::new(vault) }
    }

    /// La bóveda (para atar su clave al TPM desde otro hilo).
    pub fn vault(&self) -> Arc<Vault> {
        self.vault.clone()
    }

    /// Lee y descifra. Si no existe, migra el `.json` en claro de versiones
    /// anteriores (y lo borra). Cualquier problema (modificado, de otra
    /// clave, corrupto) devuelve `None`: la app sigue con valores por defecto.
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
                eprintln!("no se pudo leer {}: {err}", path.display());
                None
            }
        }
    }

    pub fn save<T: Serialize>(&self, path: &Path, purpose: &str, value: &T) -> io::Result<()> {
        let json = Zeroizing::new(serde_json::to_vec(value).map_err(io::Error::other)?);
        self.vault.write(path, purpose, &json)
    }
}

/// Cómo se desbloquea la app (lock.rs).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LockMode {
    #[default]
    Off,
    Hello,
    Pin,
}

/// Preferencias del usuario. Los campos ausentes toman su valor por defecto,
/// así que añadir campos en versiones futuras no rompe archivos antiguos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub always_on_top: bool,
    /// Porcentaje, entre `MIN_OPACITY` y 100.
    pub opacity: u8,
    pub video_only: bool,
    /// Último servicio abierto (id del catálogo).
    pub last_service: Option<String>,
    /// Al arrancar, entrar directo al último servicio en vez del menú.
    pub start_with_last: bool,
    /// Barra de música en la barra de tareas con servicios de música.
    pub taskbar_bar: bool,
    /// Cuánto se arrastró la barra de música hacia la izquierda (px lógicos).
    pub bar_shift: i32,
    /// Bloqueo de la app.
    pub lock: LockMode,
    /// PIN en formato PHC de Argon2id (nunca el PIN).
    pub pin_hash: Option<String>,
    /// Intentos fallidos seguidos y hasta cuándo esperar (segundos Unix).
    pub lock_failures: u32,
    pub lock_blocked_until: u64,
    /// Extensiones aprobadas o rechazadas: carpeta → huella SHA-256 de su
    /// contenido. Si la extensión cambia, se vuelve a preguntar.
    pub approved_extensions: BTreeMap<String, String>,
    pub rejected_extensions: BTreeMap<String, String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            always_on_top: true,
            opacity: 100,
            video_only: false,
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
        }
    }
}

impl Settings {
    pub fn load(store: &Store, path: &Path) -> Self {
        let mut settings: Self = store.load(path, SETTINGS).unwrap_or_default();
        settings.opacity = settings.opacity.clamp(MIN_OPACITY, 100);
        settings.bar_shift = settings.bar_shift.clamp(0, 10_000);
        // Un id que ya no existe en el catálogo se olvida.
        if settings.last_service.as_deref().is_some_and(|id| crate::services::by_id(id).is_none()) {
            settings.last_service = None;
        }
        // Bloqueo por PIN sin PIN guardado (archivo dañado): sin bloqueo
        // sería inseguro en silencio, así que se pasa a Windows Hello.
        if settings.lock == LockMode::Pin && settings.pin_hash.is_none() {
            settings.lock = LockMode::Hello;
        }
        settings
    }

    pub fn save(&self, store: &Store, path: &Path) -> io::Result<()> {
        store.save(path, SETTINGS, self)
    }
}

/// Carpeta de extensión → id que le asignó WebView2 al instalarla.
pub type ExtensionIds = BTreeMap<String, String>;

pub fn load_extension_ids(store: &Store, path: &Path) -> ExtensionIds {
    store.load(path, EXTENSIONS).unwrap_or_default()
}

pub fn save_extension_ids(store: &Store, path: &Path, ids: &ExtensionIds) -> io::Result<()> {
    store.save(path, EXTENSIONS, ids)
}

/// Geometría en píxeles físicos, tal como la reporta el sistema.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WindowGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
}

impl WindowGeometry {
    /// Devuelve `None` si el archivo no existe, no se puede leer o su
    /// contenido no es razonable. Nunca es un error fatal.
    pub fn load(store: &Store, path: &Path) -> Option<Self> {
        let geometry: Self = store.load(path, WINDOW)?;
        (geometry.width >= MIN_RESTORED_WIDTH && geometry.height >= MIN_RESTORED_HEIGHT)
            .then_some(geometry)
    }

    pub fn save(&self, store: &Store, path: &Path) -> io::Result<()> {
        store.save(path, WINDOW, self)
    }

    /// Solape mínimo, en píxeles por eje, entre la ventana y un monitor para
    /// considerar que el usuario puede alcanzarla con el ratón.
    pub const MIN_VISIBLE: i32 = 64;

    /// ¿Esta geometría se solapa lo suficiente con el rectángulo de monitor
    /// dado (x, y, ancho, alto)?
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
        assert!(!fs::read(&path).unwrap().windows(5).any(|w| w == b"width"), "cifrado en disco");

        store.save(&path, WINDOW, &WindowGeometry { x: 0, y: 0, width: 5, height: 5 }).unwrap();
        assert_eq!(WindowGeometry::load(&store, &path), None);

        fs::write(&path, "no es de la bóveda").unwrap();
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

        // Migración: el JSON en claro de versiones anteriores se cifra y se borra.
        fs::write(&legacy, r#"{"video_only":true,"opacity":3}"#).unwrap();
        let s = Settings::load(&store, &path);
        assert!(s.video_only && s.always_on_top && s.opacity == MIN_OPACITY);
        assert!(!legacy.exists(), "el archivo en claro se borra tras migrar");
        assert!(path.exists() && Settings::load(&store, &path).video_only);

        let s = Settings {
            always_on_top: false,
            opacity: 70,
            video_only: true,
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
        };
        s.save(&store, &path).unwrap();
        assert_eq!(Settings::load(&store, &path), s);
        let raw = fs::read(&path).unwrap();
        assert!(!raw.windows(8).any(|w| w == b"argon2id"), "el hash del PIN tampoco queda legible");

        // Campos que faltan en archivos viejos: la barra viene activada, sin bloqueo.
        store.save(&path, SETTINGS, &serde_json::json!({"always_on_top": false})).unwrap();
        let s = Settings::load(&store, &path);
        assert!(s.taskbar_bar && s.bar_shift == 0 && s.lock == LockMode::Off);
        store.save(&path, SETTINGS, &serde_json::json!({"bar_shift": -40, "last_service": "servicio-borrado"})).unwrap();
        let s = Settings::load(&store, &path);
        assert_eq!((s.bar_shift, s.last_service), (0, None));
        // PIN activado sin hash: no se queda sin bloqueo, pasa a Windows Hello.
        store.save(&path, SETTINGS, &serde_json::json!({"lock": "pin"})).unwrap();
        assert_eq!(Settings::load(&store, &path).lock, LockMode::Hello);

        // Archivo manipulado: se ignora (valores por defecto), no se rompe.
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
