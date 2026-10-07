//! Extensiones de Chrome cargadas en WebView2.
//!
//! WebView2 no tiene Chrome Web Store ni barra de extensiones: solo carga
//! extensiones *descomprimidas* desde una carpeta. Este módulo:
//!
//! 1. Descomprime los `.crx` y `.zip` que el usuario deja en la carpeta de
//!    extensiones (y aparta el archivo original a `.archivos\`).
//! 2. Valida cada subcarpeta (debe tener `manifest.json`) y lee su nombre.
//! 3. En Windows, sincroniza con el perfil de WebView2: instala las nuevas,
//!    desinstala las que el usuario borró y permite activarlas o no.
//!
//! No se usa `with_extensions_path` de wry porque intenta instalar *cualquier*
//! entrada de la carpeta y, si una falla, aborta la creación del webview.

use std::{
    fs,
    io::{self, Cursor},
    path::{Path, PathBuf},
};

use serde::Serialize;
use serde_json::Value;

/// Carpeta (dentro de la de extensiones) donde se apartan los `.crx`/`.zip`
/// ya descomprimidos. Empieza por punto para que el escaneo la ignore.
const ARCHIVE_DIR: &str = ".archivos";

/// Extensión válida encontrada en disco.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalExtension {
    /// Nombre de la subcarpeta; es la clave estable de la extensión.
    pub folder: String,
    /// Carpeta que contiene `manifest.json` (puede ser una subcarpeta).
    pub path: PathBuf,
    pub name: String,
    pub version: String,
    /// Página de opciones o, si no hay, la ventana emergente del botón.
    pub page: Option<String>,
}

/// Resultado de preparar la carpeta de extensiones.
#[derive(Debug, Default)]
pub struct Scan {
    pub extensions: Vec<LocalExtension>,
    /// Mensajes legibles para el usuario (archivo corrupto, sin manifest...).
    pub problems: Vec<String>,
}

/// Estado de una extensión tal como lo ve la página.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionStatus {
    pub folder: String,
    pub name: String,
    pub version: String,
    pub id: Option<String>,
    pub enabled: bool,
    pub has_page: bool,
    pub error: Option<String>,
    /// `true` mientras WebView2 aún no ha confirmado la instalación.
    pub pending: bool,
}

impl ExtensionStatus {
    pub fn pending(ext: &LocalExtension) -> Self {
        Self {
            folder: ext.folder.clone(),
            name: ext.name.clone(),
            version: ext.version.clone(),
            id: None,
            enabled: false,
            has_page: ext.page.is_some(),
            error: None,
            pending: true,
        }
    }
}

/// Resultados asíncronos de WebView2, enviados al bucle de eventos.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionEvent {
    Installed { folder: String, id: String, enabled: bool },
    Failed { folder: String, error: String },
    Toggled { id: String, enabled: bool },
    Removed { id: String },
    /// El runtime no soporta extensiones (anterior a la versión 118, aprox.).
    Unsupported(String),
}

/// Descomprime archivos pendientes y lista las extensiones válidas.
pub fn scan(dir: &Path) -> Scan {
    let mut scan = Scan::default();
    unpack_archives(dir, &mut scan.problems);

    let Ok(entries) = fs::read_dir(dir) else {
        scan.problems.push(format!("No se pudo leer {}", dir.display()));
        return scan;
    };
    let mut folders: Vec<_> = entries
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|name| !name.starts_with('.'))
        .collect();
    folders.sort_by_key(|f| f.to_lowercase());

    for folder in folders {
        let base = dir.join(&folder);
        let Some(root) = manifest_root(&base) else {
            scan.problems.push(format!("«{folder}» no tiene manifest.json"));
            continue;
        };
        match read_manifest(&root) {
            Ok((name, version, page)) => {
                scan.extensions.push(LocalExtension { folder, path: root, name, version, page })
            }
            Err(err) => scan.problems.push(format!("«{folder}»: {err}")),
        }
    }
    scan
}

/// Descomprime cada `.crx` y `.zip` de la raíz en una carpeta con su nombre
/// (reemplazándola si existía: así se actualiza una extensión) y aparta el
/// archivo a `.archivos\`.
fn unpack_archives(dir: &Path, problems: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(dir) else { return };
    for entry in entries.filter_map(Result::ok) {
        let path = entry.path();
        let is_archive = path.is_file()
            && path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("crx") || e.eq_ignore_ascii_case("zip"));
        if !is_archive {
            continue;
        }
        let file_name = entry.file_name().to_string_lossy().into_owned();
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let folder = sanitize_folder_name(&stem);
        let dest = dir.join(&folder);

        let result = fs::read(&path)
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                let payload = crx_payload(&bytes)?;
                // Se descomprime en una carpeta temporal y solo se reemplaza
                // la anterior si todo salió bien.
                let tmp = dir.join(format!(".tmp-{folder}"));
                let _ = fs::remove_dir_all(&tmp);
                extract_zip(payload, &tmp).inspect_err(|_| {
                    let _ = fs::remove_dir_all(&tmp);
                })?;
                if manifest_root(&tmp).is_none() {
                    let _ = fs::remove_dir_all(&tmp);
                    return Err("el archivo no contiene manifest.json".into());
                }
                if dest.exists() {
                    fs::remove_dir_all(&dest).map_err(|e| e.to_string())?;
                }
                fs::rename(&tmp, &dest).map_err(|e| e.to_string())
            });

        match result {
            Ok(()) => {
                let archive_dir = dir.join(ARCHIVE_DIR);
                let moved = fs::create_dir_all(&archive_dir)
                    .and_then(|_| fs::rename(&path, archive_dir.join(&file_name)));
                if moved.is_err() {
                    // Si no se puede apartar, se borra para no reinstalarlo
                    // en cada arranque.
                    let _ = fs::remove_file(&path);
                }
            }
            Err(err) => problems.push(format!("«{file_name}»: {err}")),
        }
    }
}

/// Devuelve el ZIP contenido en un `.crx` (versiones 2 y 3), o los mismos
/// bytes si ya es un ZIP.
pub fn crx_payload(bytes: &[u8]) -> Result<&[u8], String> {
    if bytes.starts_with(b"PK\x03\x04") {
        return Ok(bytes);
    }
    if !bytes.starts_with(b"Cr24") {
        return Err("no es un .crx ni un .zip válido".into());
    }
    let u32_at = |offset: usize| -> Result<usize, String> {
        bytes
            .get(offset..offset + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
            .ok_or_else(|| "cabecera .crx truncada".to_string())
    };
    let start = match u32_at(4)? {
        // Cr24 | versión | longitud de la cabecera | cabecera | zip
        3 => 12usize.checked_add(u32_at(8)?),
        // Cr24 | versión | long. clave | long. firma | clave | firma | zip
        2 => 16usize.checked_add(u32_at(8)?).and_then(|n| n.checked_add(u32_at(12).ok()?)),
        v => return Err(format!("versión de .crx no soportada ({v})")),
    }
    .ok_or("cabecera .crx inválida")?;
    let payload = bytes.get(start..).ok_or("cabecera .crx inválida")?;
    if !payload.starts_with(b"PK") {
        return Err("el .crx no contiene un ZIP".into());
    }
    Ok(payload)
}

/// Extrae un ZIP en `dest`, rechazando rutas que escapen de la carpeta.
fn extract_zip(bytes: &[u8], dest: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| format!("ZIP inválido: {e}"))?;
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| format!("ZIP inválido: {e}"))?;
        // `enclosed_name` descarta rutas absolutas y con `..`.
        let Some(relative) = file.enclosed_name() else {
            return Err(format!("ruta no permitida en el ZIP: {}", file.name()));
        };
        let out = dest.join(relative);
        if file.is_dir() {
            fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut writer = fs::File::create(&out).map_err(|e| e.to_string())?;
            io::copy(&mut file, &mut writer).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// Carpeta con `manifest.json`: la propia carpeta o, si no lo tiene, su
/// única subcarpeta (los ZIP de GitHub suelen traer una carpeta envolvente).
fn manifest_root(dir: &Path) -> Option<PathBuf> {
    if dir.join("manifest.json").is_file() {
        return Some(dir.to_path_buf());
    }
    let subdirs: Vec<_> = fs::read_dir(dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    match subdirs.as_slice() {
        [only] if only.join("manifest.json").is_file() => Some(only.clone()),
        _ => None,
    }
}

/// Lee nombre, versión y página de opciones/emergente del manifest.
fn read_manifest(root: &Path) -> Result<(String, String, Option<String>), String> {
    let text = fs::read_to_string(root.join("manifest.json")).map_err(|e| e.to_string())?;
    // Algunos manifest traen BOM.
    let manifest: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| format!("manifest.json inválido: {e}"))?;

    let raw_name = manifest.get("name").and_then(Value::as_str).ok_or("manifest.json sin «name»")?;
    let name = localize(root, &manifest, raw_name);
    let version = manifest.get("version").and_then(Value::as_str).unwrap_or("").to_owned();

    let page = [
        manifest.pointer("/options_ui/page"),
        manifest.get("options_page"),
        manifest.pointer("/action/default_popup"),
        manifest.pointer("/browser_action/default_popup"),
    ]
    .into_iter()
    .flatten()
    .filter_map(Value::as_str)
    .map(|p| p.trim_start_matches('/'))
    .find(|p| !p.is_empty() && !p.contains("..") && !p.contains("://"))
    .map(str::to_owned);

    Ok((name, version, page))
}

/// Resuelve `__MSG_clave__` con `_locales/<default_locale>/messages.json`.
fn localize(root: &Path, manifest: &Value, value: &str) -> String {
    let Some(key) = value.strip_prefix("__MSG_").and_then(|v| v.strip_suffix("__")) else {
        return value.to_owned();
    };
    let locale = manifest.get("default_locale").and_then(Value::as_str).unwrap_or("en");
    let lookup = |locale: &str| -> Option<String> {
        let path = root.join("_locales").join(locale).join("messages.json");
        let messages: Value = serde_json::from_str(
            fs::read_to_string(path).ok()?.trim_start_matches('\u{feff}'),
        )
        .ok()?;
        // Las claves no distinguen mayúsculas en Chromium.
        let entry = messages
            .as_object()?
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))?
            .1;
        entry.get("message")?.as_str().map(str::to_owned)
    };
    ["es", locale, "en"].into_iter().find_map(lookup).unwrap_or_else(|| key.to_owned())
}

/// Nombre de carpeta seguro a partir del nombre del archivo.
fn sanitize_folder_name(stem: &str) -> String {
    let cleaned: String = stem
        .chars()
        .map(|c| if c.is_alphanumeric() || matches!(c, '-' | '_' | '.' | ' ') { c } else { '_' })
        .collect();
    let cleaned = cleaned.trim().trim_start_matches('.').trim();
    if cleaned.is_empty() {
        "extension".into()
    } else {
        cleaned.chars().take(64).collect()
    }
}

// ---------------------------------------------------------------------------
// Integración con WebView2 (solo Windows)
// ---------------------------------------------------------------------------

#[cfg(windows)]
pub use self::host::ExtensionHost;

#[cfg(windows)]
mod host {
    use std::collections::HashMap;

    use tao::event_loop::EventLoopProxy;
    use webview2_com::{
        take_pwstr, BrowserExtensionEnableCompletedHandler, BrowserExtensionRemoveCompletedHandler,
        Microsoft::Web::WebView2::Win32::{
            ICoreWebView2BrowserExtension, ICoreWebView2BrowserExtensionList, ICoreWebView2Profile7,
            ICoreWebView2_13,
        },
        ProfileAddBrowserExtensionCompletedHandler, ProfileGetBrowserExtensionsCompletedHandler,
    };
    use windows::core::{Interface, BOOL, HSTRING, PWSTR};
    use wry::{WebView, WebViewExtWindows};

    use super::{ExtensionEvent, LocalExtension};
    use crate::{config::ExtensionIds, ipc::UserEvent};

    type Proxy = EventLoopProxy<UserEvent>;

    pub struct ExtensionHost {
        profile: Option<ICoreWebView2Profile7>,
        proxy: Proxy,
    }

    fn describe(err: &windows::core::Error) -> String {
        let message = err.message();
        let message = message.trim();
        if message.is_empty() {
            format!("error 0x{:08X}", err.code().0)
        } else {
            format!("{message} (0x{:08X})", err.code().0)
        }
    }

    struct Installed {
        id: String,
        enabled: bool,
        extension: ICoreWebView2BrowserExtension,
    }

    unsafe fn read_list(list: &ICoreWebView2BrowserExtensionList) -> windows::core::Result<Vec<Installed>> {
        let mut count = 0u32;
        list.Count(&mut count)?;
        let mut out = Vec::with_capacity(count as usize);
        for i in 0..count {
            let extension = list.GetValueAtIndex(i)?;
            let mut id = PWSTR::null();
            extension.Id(&mut id)?;
            let mut enabled = BOOL::default();
            extension.IsEnabled(&mut enabled)?;
            out.push(Installed { id: take_pwstr(id), enabled: enabled.as_bool(), extension });
        }
        Ok(out)
    }

    impl ExtensionHost {
        pub fn new(webview: &WebView, proxy: Proxy) -> Self {
            let profile = unsafe {
                webview
                    .webview()
                    .cast::<ICoreWebView2_13>()
                    .and_then(|w| w.Profile())
                    .and_then(|p| p.cast::<ICoreWebView2Profile7>())
            };
            if let Err(err) = &profile {
                let _ = proxy.send_event(UserEvent::Extension(ExtensionEvent::Unsupported(
                    describe(err),
                )));
            }
            Self { profile: profile.ok(), proxy }
        }

        /// Lista lo instalado y llama a `f` con ello en el hilo de la UI.
        fn with_installed(&self, f: impl FnOnce(Vec<Installed>, &Proxy) + 'static) {
            let Some(profile) = &self.profile else { return };
            let proxy = self.proxy.clone();
            let handler = ProfileGetBrowserExtensionsCompletedHandler::create(Box::new(
                move |result, list| {
                    let installed = result
                        .and_then(|_| list.ok_or_else(|| windows::core::Error::from(windows::Win32::Foundation::E_POINTER)))
                        .and_then(|list| unsafe { read_list(&list) });
                    match installed {
                        Ok(installed) => f(installed, &proxy),
                        Err(err) => {
                            let _ = proxy.send_event(UserEvent::Extension(
                                ExtensionEvent::Unsupported(describe(&err)),
                            ));
                        }
                    }
                    Ok(())
                },
            ));
            if let Err(err) = unsafe { profile.GetBrowserExtensions(&handler) } {
                let _ = self
                    .proxy
                    .send_event(UserEvent::Extension(ExtensionEvent::Unsupported(describe(&err))));
            }
        }

        /// Instala las extensiones nuevas, informa de las ya instaladas y
        /// desinstala las que se instalaron antes pero ya no están en disco.
        pub fn sync(&self, local: Vec<LocalExtension>, known: ExtensionIds) {
            let Some(profile) = self.profile.clone() else { return };
            self.with_installed(move |installed, proxy| {
                let by_id: HashMap<&str, &Installed> =
                    installed.iter().map(|i| (i.id.as_str(), i)).collect();

                // 1. Desinstalar lo que instalamos y ya no existe en disco.
                let wanted: Vec<&str> = local
                    .iter()
                    .filter_map(|ext| known.get(&ext.folder).map(String::as_str))
                    .collect();
                for (folder, id) in &known {
                    let still_local = local.iter().any(|e| &e.folder == folder);
                    if still_local || wanted.contains(&id.as_str()) {
                        continue;
                    }
                    if let Some(item) = by_id.get(id.as_str()) {
                        let proxy = proxy.clone();
                        let id = id.clone();
                        let handler = BrowserExtensionRemoveCompletedHandler::create(Box::new(
                            move |result| {
                                if result.is_ok() {
                                    let _ = proxy.send_event(UserEvent::Extension(
                                        ExtensionEvent::Removed { id },
                                    ));
                                }
                                Ok(())
                            },
                        ));
                        let _ = unsafe { item.extension.Remove(&handler) };
                    }
                }

                // 2. Informar de las instaladas e instalar las nuevas.
                for ext in local {
                    if let Some(item) = known.get(&ext.folder).and_then(|id| by_id.get(id.as_str())) {
                        let _ = proxy.send_event(UserEvent::Extension(ExtensionEvent::Installed {
                            folder: ext.folder.clone(),
                            id: item.id.clone(),
                            enabled: item.enabled,
                        }));
                        continue;
                    }
                    let folder = ext.folder.clone();
                    let reply = proxy.clone();
                    let handler = ProfileAddBrowserExtensionCompletedHandler::create(Box::new(
                        move |result, extension| {
                            let event = match result.and_then(|_| {
                                extension.ok_or_else(|| {
                                    windows::core::Error::from(windows::Win32::Foundation::E_POINTER)
                                })
                            }) {
                                Ok(extension) => unsafe {
                                    let mut id = PWSTR::null();
                                    let mut enabled = BOOL::default();
                                    match extension.Id(&mut id).and_then(|_| extension.IsEnabled(&mut enabled)) {
                                        Ok(()) => ExtensionEvent::Installed {
                                            folder,
                                            id: take_pwstr(id),
                                            enabled: enabled.as_bool(),
                                        },
                                        Err(err) => ExtensionEvent::Failed { folder, error: describe(&err) },
                                    }
                                },
                                Err(err) => ExtensionEvent::Failed { folder, error: describe(&err) },
                            };
                            let _ = reply.send_event(UserEvent::Extension(event));
                            Ok(())
                        },
                    ));
                    let path = HSTRING::from(ext.path.as_path());
                    if let Err(err) = unsafe { profile.AddBrowserExtension(&path, &handler) } {
                        let _ = proxy.send_event(UserEvent::Extension(ExtensionEvent::Failed {
                            folder: ext.folder,
                            error: describe(&err),
                        }));
                    }
                }
            });
        }

        pub fn set_enabled(&self, id: String, enabled: bool) {
            self.with_installed(move |installed, proxy| {
                let Some(item) = installed.iter().find(|i| i.id == id) else { return };
                let reply = proxy.clone();
                let handler = BrowserExtensionEnableCompletedHandler::create(Box::new(move |result| {
                    let event = match result {
                        Ok(()) => ExtensionEvent::Toggled { id, enabled },
                        Err(err) => ExtensionEvent::Failed { folder: String::new(), error: describe(&err) },
                    };
                    let _ = reply.send_event(UserEvent::Extension(event));
                    Ok(())
                }));
                let _ = unsafe { item.extension.Enable(enabled, &handler) };
            });
        }
    }
}

/// Fuera de Windows no hay WebView2: el host no hace nada.
#[cfg(not(windows))]
pub struct ExtensionHost;

#[cfg(not(windows))]
impl ExtensionHost {
    pub fn new<W, P>(_: &W, _: P) -> Self {
        Self
    }
    pub fn sync(&self, _: Vec<LocalExtension>, _: crate::config::ExtensionIds) {}
    pub fn set_enabled(&self, _: String, _: bool) {}
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ytmp-ext-{tag}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn zip_bytes(files: &[(&str, &str)]) -> Vec<u8> {
        let mut buf = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            let opts = zip::write::SimpleFileOptions::default();
            for (name, content) in files {
                zip.start_file(*name, opts).unwrap();
                zip.write_all(content.as_bytes()).unwrap();
            }
            zip.finish().unwrap();
        }
        buf.into_inner()
    }

    fn crx3(zip: &[u8]) -> Vec<u8> {
        let header = b"cabecera-protobuf-falsa";
        let mut out = b"Cr24".to_vec();
        out.extend(3u32.to_le_bytes());
        out.extend((header.len() as u32).to_le_bytes());
        out.extend(header);
        out.extend(zip);
        out
    }

    fn crx2(zip: &[u8]) -> Vec<u8> {
        let mut out = b"Cr24".to_vec();
        out.extend(2u32.to_le_bytes());
        out.extend(3u32.to_le_bytes());
        out.extend(2u32.to_le_bytes());
        out.extend(b"KEYSG");
        out.extend(zip);
        out
    }

    const MANIFEST: &str = r#"{"manifest_version":3,"name":"Demo","version":"1.2.3","options_ui":{"page":"options.html"}}"#;

    #[test]
    fn crx_headers() {
        let zip = zip_bytes(&[("manifest.json", MANIFEST)]);
        assert_eq!(crx_payload(&zip).unwrap(), zip.as_slice());
        assert_eq!(crx_payload(&crx3(&zip)).unwrap(), zip.as_slice());
        assert_eq!(crx_payload(&crx2(&zip)).unwrap(), zip.as_slice());
        assert!(crx_payload(b"hola").is_err());
        assert!(crx_payload(b"Cr24\x03\x00\x00\x00\xff\xff\xff\x7f").is_err());
        assert!(crx_payload(b"Cr24\x09\x00\x00\x00").is_err());
        assert!(crx_payload(b"Cr24").is_err());
    }

    #[test]
    fn scans_unpacked_and_archives() {
        let dir = temp_dir("scan");
        // Carpeta descomprimida a mano.
        fs::create_dir_all(dir.join("manual")).unwrap();
        fs::write(dir.join("manual/manifest.json"), MANIFEST).unwrap();
        // ZIP con carpeta envolvente (como los de GitHub).
        fs::write(
            dir.join("uBOLite.chromium.zip"),
            zip_bytes(&[("uBOLite/manifest.json", r#"{"name":"uBO Lite","version":"2025.1","action":{"default_popup":"/popup.html"}}"#)]),
        )
        .unwrap();
        // CRX3 con nombre localizado.
        fs::write(
            dir.join("Sponsor Block!.crx"),
            crx3(&zip_bytes(&[
                ("manifest.json", r#"{"name":"__MSG_fullName__","version":"5.0","default_locale":"en"}"#),
                ("_locales/en/messages.json", r#"{"fullName":{"message":"SponsorBlock"}}"#),
                ("_locales/es/messages.json", r#"{"FULLNAME":{"message":"SponsorBlock (es)"}}"#),
            ])),
        )
        .unwrap();
        // Basura.
        fs::create_dir_all(dir.join("vacia")).unwrap();
        fs::write(dir.join("roto.crx"), b"Cr24\x03\x00\x00\x00").unwrap();
        fs::write(dir.join("LEEME.txt"), "hola").unwrap();

        let scan = scan(&dir);
        let names: Vec<_> = scan.extensions.iter().map(|e| (e.folder.as_str(), e.name.as_str())).collect();
        assert_eq!(
            names,
            [("manual", "Demo"), ("Sponsor Block_", "SponsorBlock (es)"), ("uBOLite.chromium", "uBO Lite")]
        );
        let manual = &scan.extensions[0];
        assert_eq!(manual.version, "1.2.3");
        assert_eq!(manual.page.as_deref(), Some("options.html"));
        assert_eq!(scan.extensions[2].page.as_deref(), Some("popup.html"));
        assert!(scan.extensions[2].path.ends_with("uBOLite"));

        // Archivos apartados; el roto se queda y se informa.
        assert!(dir.join(".archivos/uBOLite.chromium.zip").is_file());
        assert!(!dir.join("uBOLite.chromium.zip").exists());
        assert!(dir.join("roto.crx").is_file());
        assert_eq!(scan.problems.len(), 2, "{:?}", scan.problems);
        assert!(scan.problems.iter().any(|p| p.contains("vacia")));
        assert!(scan.problems.iter().any(|p| p.contains("roto.crx")));

        // Un segundo escaneo no duplica ni vuelve a descomprimir.
        let again = super::scan(&dir);
        assert_eq!(again.extensions.len(), 3);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn archive_replaces_previous_version() {
        let dir = temp_dir("update");
        fs::write(dir.join("demo.zip"), zip_bytes(&[("manifest.json", MANIFEST)])).unwrap();
        assert_eq!(scan(&dir).extensions[0].version, "1.2.3");
        fs::write(
            dir.join("demo.zip"),
            zip_bytes(&[("manifest.json", r#"{"name":"Demo","version":"2.0"}"#)]),
        )
        .unwrap();
        let scan = scan(&dir);
        assert_eq!(scan.extensions.len(), 1);
        assert_eq!(scan.extensions[0].version, "2.0");
        assert_eq!(scan.extensions[0].page, None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn zip_slip_is_rejected() {
        let dir = temp_dir("slip");
        fs::write(dir.join("malo.zip"), zip_bytes(&[("../../fuera.txt", "x"), ("manifest.json", MANIFEST)])).unwrap();
        let scan = scan(&dir);
        assert!(scan.extensions.is_empty());
        assert!(scan.problems[0].contains("ruta no permitida"), "{:?}", scan.problems);
        assert!(!dir.parent().unwrap().join("fuera.txt").exists());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unsafe_pages_are_ignored() {
        let dir = temp_dir("pages");
        fs::write(
            dir.join("manifest.json"),
            r#"{"name":"X","version":"1","options_page":"https://evil.com/x","action":{"default_popup":"../up.html"}}"#,
        )
        .unwrap();
        assert_eq!(read_manifest(&dir).unwrap().2, None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn folder_names_are_sanitized() {
        assert_eq!(sanitize_folder_name("uBlock Origin Lite"), "uBlock Origin Lite");
        assert_eq!(sanitize_folder_name("..\\..\\x"), "_.._x");
        assert_eq!(sanitize_folder_name("..."), "extension");
        assert_eq!(sanitize_folder_name("a/b:c*?"), "a_b_c__");
    }
}
