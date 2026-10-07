//! Rutas de la aplicación y archivos de estado.
//!
//! Todo vive bajo `%LOCALAPPDATA%\yt_mini_player`:
//! - `profile\`         → UserDataFolder de WebView2 (cookies, sesión, caché).
//! - `extensions\`      → extensiones descomprimidas, `.crx` o `.zip`.
//! - `window.json`      → posición y tamaño de la ventana al cerrar.
//! - `settings.json`    → preferencias (siempre visible, opacidad, solo video).
//! - `extensions.json`  → carpeta de extensión → id instalado en WebView2.

use std::{
    collections::BTreeMap,
    fs, io,
    path::{Path, PathBuf},
};

use serde::{de::DeserializeOwned, Deserialize, Serialize};

use crate::ipc::MIN_OPACITY;

const APP_DIR: &str = "yt_mini_player";

/// Tamaño mínimo aceptado al restaurar (evita ventanas degeneradas por un
/// `window.json` corrupto o editado a mano).
const MIN_RESTORED_WIDTH: u32 = 160;
const MIN_RESTORED_HEIGHT: u32 = 90;

pub struct Paths {
    pub profile: PathBuf,
    pub extensions: PathBuf,
    pub window_state: PathBuf,
    pub settings: PathBuf,
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
            window_state: root.join("window.json"),
            settings: root.join("settings.json"),
            extension_ids: root.join("extensions.json"),
        };
        fs::create_dir_all(&paths.profile)?;
        fs::create_dir_all(&paths.extensions)?;
        Ok(paths)
    }
}

/// Lee un JSON; cualquier problema (no existe, corrupto) devuelve `None`.
fn load_json<T: DeserializeOwned>(path: &Path) -> Option<T> {
    serde_json::from_str(&fs::read_to_string(path).ok()?).ok()
}

/// Escritura atómica: archivo temporal + rename, para no dejar un JSON
/// truncado si el proceso muere a mitad de la escritura.
fn save_json<T: Serialize>(path: &Path, value: &T) -> io::Result<()> {
    let json = serde_json::to_string_pretty(value).map_err(io::Error::other)?;
    let tmp = path.with_extension("json.tmp");
    fs::write(&tmp, json)?;
    fs::rename(&tmp, path)
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
}

impl Default for Settings {
    fn default() -> Self {
        Self { always_on_top: true, opacity: 100, video_only: false }
    }
}

impl Settings {
    pub fn load(path: &Path) -> Self {
        let mut settings: Self = load_json(path).unwrap_or_default();
        settings.opacity = settings.opacity.clamp(MIN_OPACITY, 100);
        settings
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        save_json(path, self)
    }
}

/// Carpeta de extensión → id que le asignó WebView2 al instalarla.
pub type ExtensionIds = BTreeMap<String, String>;

pub fn load_extension_ids(path: &Path) -> ExtensionIds {
    load_json(path).unwrap_or_default()
}

pub fn save_extension_ids(path: &Path, ids: &ExtensionIds) -> io::Result<()> {
    save_json(path, ids)
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
    pub fn load(path: &Path) -> Option<Self> {
        let geometry: Self = load_json(path)?;
        (geometry.width >= MIN_RESTORED_WIDTH && geometry.height >= MIN_RESTORED_HEIGHT)
            .then_some(geometry)
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        save_json(path, self)
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
        let path = dir.join("window.json");

        let g = geom(10, 20);
        g.save(&path).unwrap();
        assert_eq!(WindowGeometry::load(&path), Some(g));

        fs::write(&path, r#"{"x":0,"y":0,"width":5,"height":5}"#).unwrap();
        assert_eq!(WindowGeometry::load(&path), None);

        fs::write(&path, "no es json").unwrap();
        assert_eq!(WindowGeometry::load(&path), None);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn settings_defaults_partial_files_and_clamping() {
        let dir = temp_dir("settings");
        let path = dir.join("settings.json");

        assert_eq!(Settings::load(&path), Settings::default());

        fs::write(&path, r#"{"video_only":true}"#).unwrap();
        let s = Settings::load(&path);
        assert!(s.video_only && s.always_on_top && s.opacity == 100);

        fs::write(&path, r#"{"opacity":3}"#).unwrap();
        assert_eq!(Settings::load(&path).opacity, MIN_OPACITY);

        let s = Settings { always_on_top: false, opacity: 70, video_only: true };
        s.save(&path).unwrap();
        assert_eq!(Settings::load(&path), s);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn paths_create_directories() {
        let dir = temp_dir("paths");
        let paths = Paths::under(dir.join("app")).unwrap();
        assert!(paths.profile.is_dir() && paths.extensions.is_dir());
        assert_eq!(paths.settings, dir.join("app").join("settings.json"));
        fs::remove_dir_all(&dir).unwrap();
    }
}
