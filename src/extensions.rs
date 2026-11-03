//! Chrome extensions loaded into WebView2.
//!
//! WebView2 has no Chrome Web Store or extensions toolbar: it only loads
//! *unpacked* extensions from a folder. This module:
//!
//! 1. Unpacks the `.crx` and `.zip` files the user drops into the extensions
//!    folder (and moves the original file to `.archives\`).
//! 2. Validates each subfolder (it must have a `manifest.json`) and reads
//!    its name.
//! 3. On Windows, syncs with the WebView2 profile: installs new ones,
//!    uninstalls the ones the user deleted and lets them be turned on or off.
//!
//! wry's `with_extensions_path` is not used because it tries to install
//! *any* entry of the folder and, if one fails, aborts creating the webview.
//!
//! Security: any program of the user can write to the folder, and an
//! extension sees the pages (and sessions) it runs on. That is why each
//! extension has a **SHA-256 fingerprint** of all its content and is only
//! loaded if the user approved it in a system dialog that shows its
//! permissions (app.rs). If a single byte changes, the user is asked again.
//! Unpacking limits the number of files and the total size.

use std::{
    fs,
    io::{self, Cursor},
    path::{Path, PathBuf},
};

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

/// Folder (inside the extensions folder) where unpacked `.crx`/`.zip` files
/// are moved. It starts with a dot so the scan ignores it.
const ARCHIVE_DIR: &str = ".archives";
/// Its name up to 0.7.0, renamed on the next scan.
const OLD_ARCHIVE_DIR: &str = ".archivos";
/// Unpacking limits (against "ZIP bombs").
const MAX_ENTRIES: usize = 10_000;
const MAX_UNPACKED: u64 = 256 * 1024 * 1024;

/// A valid extension found on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LocalExtension {
    /// Name of the subfolder; the extension's stable key.
    pub folder: String,
    /// Folder that contains `manifest.json` (it may be a subfolder).
    pub path: PathBuf,
    pub name: String,
    pub version: String,
    /// Options page or, if there is none, the button's popup.
    pub page: Option<String>,
    /// Permissions and sites it accesses (from the manifest).
    pub permissions: Vec<String>,
    /// Can it read and change every page?
    pub all_sites: bool,
    /// SHA-256 (hex) of all its content.
    pub fingerprint: String,
}

/// Result of preparing the extensions folder.
#[derive(Debug, Default)]
pub struct Scan {
    pub extensions: Vec<LocalExtension>,
    /// Messages for the user (corrupt file, no manifest...), already in the current language.
    pub problems: Vec<String>,
}

/// An extension's state as the page sees it.
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
    /// `true` while WebView2 has not confirmed the install yet.
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

/// WebView2's asynchronous results, sent to the event loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExtensionEvent {
    Installed { folder: String, id: String, enabled: bool },
    Failed { folder: String, error: String },
    Toggled { id: String, enabled: bool },
    Removed { id: String },
    /// The runtime does not support extensions (older than about version 118).
    Unsupported(String),
}

/// Unpacks pending archives and lists the valid extensions.
pub fn scan(dir: &Path) -> Scan {
    scan_with(dir, true)
}

/// Like [`scan`], without reading the extensions' content: enough to show
/// their names (and problems) again in another language. The fingerprints
/// are left empty, so it is never used to approve anything.
pub fn scan_names(dir: &Path) -> Scan {
    scan_with(dir, false)
}

fn scan_with(dir: &Path, fingerprints: bool) -> Scan {
    let mut scan = Scan::default();
    let (old, new) = (dir.join(OLD_ARCHIVE_DIR), dir.join(ARCHIVE_DIR));
    if old.is_dir() && !new.exists() {
        let _ = fs::rename(&old, &new);
    }
    unpack_archives(dir, &mut scan.problems);

    let Ok(entries) = fs::read_dir(dir) else {
        scan.problems.push(t!("ext.unreadable_folder", path = dir.display()));
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
            scan.problems.push(t!("ext.no_manifest", folder = folder));
            continue;
        };
        let manifest = read_manifest(&root);
        let fingerprint = if fingerprints { fingerprint(&base) } else { walk(&base).map(|_| String::new()) };
        match (manifest, fingerprint) {
            (Ok(m), Ok(fingerprint)) => scan.extensions.push(LocalExtension {
                folder,
                path: root,
                name: m.name,
                version: m.version,
                page: m.page,
                all_sites: m.permissions.iter().any(|p| is_all_sites(p)),
                permissions: m.permissions,
                fingerprint,
            }),
            (Err(err), _) => scan.problems.push(t!("ext.problem", folder = folder, error = err)),
            (_, Err(err)) => scan.problems.push(t!("ext.unreadable", folder = folder, error = fingerprint_error(&err))),
        }
    }
    scan
}

/// Unpacks each `.crx` and `.zip` at the top into a folder with its name
/// (replacing it if it existed: that is how an extension is updated) and
/// moves the file to `.archives\`.
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
                // Unpacked into a temporary folder; the previous one is only
                // replaced if everything went well.
                let tmp = dir.join(format!(".tmp-{folder}"));
                let _ = fs::remove_dir_all(&tmp);
                extract_zip(payload, &tmp).inspect_err(|_| {
                    let _ = fs::remove_dir_all(&tmp);
                })?;
                if manifest_root(&tmp).is_none() {
                    let _ = fs::remove_dir_all(&tmp);
                    return Err(t!("ext.archive_no_manifest"));
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
                    // If it cannot be moved away, it is deleted so it is not
                    // reinstalled on every start.
                    let _ = fs::remove_file(&path);
                }
            }
            Err(err) => problems.push(t!("ext.problem", folder = file_name, error = err)),
        }
    }
}

/// Returns the ZIP inside a `.crx` (versions 2 and 3), or the same bytes if
/// it already is a ZIP.
pub fn crx_payload(bytes: &[u8]) -> Result<&[u8], String> {
    if bytes.starts_with(b"PK\x03\x04") {
        return Ok(bytes);
    }
    if !bytes.starts_with(b"Cr24") {
        return Err(t!("ext.not_an_archive"));
    }
    let u32_at = |offset: usize| -> Result<usize, String> {
        bytes
            .get(offset..offset + 4)
            .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]) as usize)
            .ok_or_else(|| t!("ext.bad_crx"))
    };
    let start = match u32_at(4)? {
        // Cr24 | version | header length | header | zip
        3 => 12usize.checked_add(u32_at(8)?),
        // Cr24 | version | key length | signature length | key | signature | zip
        2 => 16usize.checked_add(u32_at(8)?).and_then(|n| n.checked_add(u32_at(12).ok()?)),
        v => return Err(t!("ext.crx_version", version = v)),
    }
    .ok_or_else(|| t!("ext.bad_crx"))?;
    let payload = bytes.get(start..).ok_or_else(|| t!("ext.bad_crx"))?;
    if !payload.starts_with(b"PK") {
        return Err(t!("ext.bad_crx"));
    }
    Ok(payload)
}

/// Extracts a ZIP into `dest`, rejecting paths that escape the folder.
fn extract_zip(bytes: &[u8], dest: &Path) -> Result<(), String> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| t!("ext.bad_zip", error = e))?;
    if archive.len() > MAX_ENTRIES {
        return Err(t!("ext.zip_too_many", max = MAX_ENTRIES));
    }
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    // Total budget: what actually comes out is counted (the sizes the ZIP
    // declares can lie).
    let mut budget = MAX_UNPACKED;
    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| t!("ext.bad_zip", error = e))?;
        // `enclosed_name` rejects absolute paths and paths with `..`.
        let Some(relative) = file.enclosed_name() else {
            let name = file.name().map(|n| n.into_owned()).unwrap_or_default();
            return Err(t!("ext.zip_bad_path", name = name));
        };
        let out = dest.join(relative);
        if file.is_dir() {
            fs::create_dir_all(&out).map_err(|e| e.to_string())?;
        } else {
            if let Some(parent) = out.parent() {
                fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
            let mut writer = fs::File::create(&out).map_err(|e| e.to_string())?;
            let written = io::copy(&mut io::Read::take(&mut file, budget + 1), &mut writer).map_err(|e| e.to_string())?;
            if written > budget {
                return Err(t!("ext.zip_too_big", size = MAX_UNPACKED / (1024 * 1024)));
            }
            budget -= written;
        }
    }
    Ok(())
}

/// The folder with `manifest.json`: the folder itself or, if it has none,
/// its only subfolder (GitHub ZIPs usually have a wrapping folder).
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

/// What matters in the manifest.
struct Manifest {
    name: String,
    version: String,
    page: Option<String>,
    permissions: Vec<String>,
}

/// Patterns that give access to every page.
fn is_all_sites(pattern: &str) -> bool {
    matches!(pattern, "<all_urls>" | "*://*/*" | "http://*/*" | "https://*/*" | "*://*/" | "file:///*")
}

/// Every file under `dir`. Refuses symlinks, too many entries (folders
/// count too: a tree of empty folders is not walked forever) and too much
/// content, judged by the sizes before anything is read.
fn walk(dir: &Path) -> io::Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    let mut entries = 0usize;
    let mut total = 0u64;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        for entry in fs::read_dir(&current)? {
            let entry = entry?;
            let kind = entry.file_type()?;
            if kind.is_symlink() {
                return Err(io::Error::other(FINGERPRINT_SYMLINK));
            }
            entries += 1;
            if entries > 2 * MAX_ENTRIES {
                return Err(io::Error::other(FINGERPRINT_TOO_MANY));
            }
            if kind.is_dir() {
                stack.push(entry.path());
            } else {
                total = total.saturating_add(entry.metadata()?.len());
                if total > MAX_UNPACKED {
                    return Err(io::Error::other(FINGERPRINT_TOO_BIG));
                }
                files.push(entry.path());
            }
        }
        if files.len() > MAX_ENTRIES {
            return Err(io::Error::other(FINGERPRINT_TOO_MANY));
        }
    }
    Ok(files)
}

/// SHA-256 fingerprint of a folder: relative paths (sorted, with `/`),
/// sizes and content of each file. It changes if any byte changes.
pub fn fingerprint(dir: &Path) -> io::Result<String> {
    use io::Read as _;
    let mut files = walk(dir)?;
    let relative = |p: &Path| {
        p.strip_prefix(dir).unwrap_or(p).components().map(|c| c.as_os_str().to_string_lossy()).collect::<Vec<_>>().join("/")
    };
    files.sort_by_cached_key(|p| relative(p));
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    for path in files {
        // At most what is left of the budget (and one byte to notice): a
        // file that grew after it was listed is not read in whole.
        let mut bytes = Vec::new();
        fs::File::open(&path)?.take(MAX_UNPACKED - total + 1).read_to_end(&mut bytes)?;
        total += bytes.len() as u64;
        if total > MAX_UNPACKED {
            return Err(io::Error::other(FINGERPRINT_TOO_BIG));
        }
        hasher.update(relative(&path).as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    Ok(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

const FINGERPRINT_SYMLINK: &str = "symlinks";
const FINGERPRINT_TOO_MANY: &str = "too many files";
const FINGERPRINT_TOO_BIG: &str = "too big";

/// The reason a folder could not be fingerprinted, for the user.
fn fingerprint_error(err: &io::Error) -> String {
    match err.to_string().as_str() {
        FINGERPRINT_SYMLINK => t!("ext.symlinks"),
        FINGERPRINT_TOO_MANY => t!("ext.too_many_files"),
        FINGERPRINT_TOO_BIG => t!("ext.too_big"),
        other => other.to_owned(),
    }
}

/// Reads name, version, options/popup page and permissions from the manifest.
fn read_manifest(root: &Path) -> Result<Manifest, String> {
    let text = fs::read_to_string(root.join("manifest.json")).map_err(|e| e.to_string())?;
    // Some manifests have a BOM.
    let manifest: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
        .map_err(|e| t!("ext.bad_manifest", error = e))?;

    let raw_name = manifest.get("name").and_then(Value::as_str).ok_or_else(|| t!("ext.manifest_no_name"))?;
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

    // Permissions (v2 mixes sites and APIs in "permissions"; v3 separates
    // them) and the sites it injects scripts into.
    let mut permissions: Vec<String> = ["/permissions", "/host_permissions", "/optional_host_permissions"]
        .into_iter()
        .filter_map(|p| manifest.pointer(p)?.as_array().cloned())
        .flatten()
        .filter_map(|v| v.as_str().map(str::to_owned))
        .collect();
    if let Some(scripts) = manifest.get("content_scripts").and_then(Value::as_array) {
        for script in scripts {
            if let Some(matches) = script.get("matches").and_then(Value::as_array) {
                permissions.extend(matches.iter().filter_map(|m| m.as_str().map(str::to_owned)));
            }
        }
    }
    permissions.sort();
    permissions.dedup();
    permissions.truncate(64);

    Ok(Manifest { name, version, page, permissions })
}

/// Resolves `__MSG_key__` from `_locales/<locale>/messages.json`: the app's
/// language first, then the extension's default, then English.
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
        // Keys are case-insensitive in Chromium.
        let entry = messages
            .as_object()?
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(key))?
            .1;
        entry.get("message")?.as_str().map(str::to_owned)
    };
    [crate::i18n::current().code(), locale, "en"].into_iter().find_map(lookup).unwrap_or_else(|| key.to_owned())
}

/// A safe folder name from the file name.
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
// WebView2 integration (Windows only)
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

        /// Lists what is installed and calls `f` with it on the UI thread.
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

        /// Installs new extensions, reports the ones already installed and
        /// uninstalls those installed before that are no longer on disk.
        pub fn sync(&self, local: Vec<LocalExtension>, known: ExtensionIds) {
            let Some(profile) = self.profile.clone() else { return };
            self.with_installed(move |installed, proxy| {
                let by_id: HashMap<&str, &Installed> =
                    installed.iter().map(|i| (i.id.as_str(), i)).collect();

                // 1. Uninstall what we installed that no longer exists on disk.
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

                // 2. Report the installed ones and install the new ones.
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

/// Outside Windows there is no WebView2: the host does nothing.
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
        let header = b"fake-protobuf-header";
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
        assert!(crx_payload(b"hello").is_err());
        assert!(crx_payload(b"Cr24\x03\x00\x00\x00\xff\xff\xff\x7f").is_err());
        assert!(crx_payload(b"Cr24\x09\x00\x00\x00").is_err());
        assert!(crx_payload(b"Cr24").is_err());
    }

    #[test]
    fn scans_unpacked_and_archives() {
        let dir = temp_dir("scan");
        // A folder unpacked by hand.
        fs::create_dir_all(dir.join("manual")).unwrap();
        fs::write(dir.join("manual/manifest.json"), MANIFEST).unwrap();
        // A ZIP with a wrapping folder (like GitHub's).
        fs::write(
            dir.join("uBOLite.chromium.zip"),
            zip_bytes(&[("uBOLite/manifest.json", r#"{"name":"uBO Lite","version":"2025.1","action":{"default_popup":"/popup.html"}}"#)]),
        )
        .unwrap();
        // A CRX3 with a localized name.
        fs::write(
            dir.join("Sponsor Block!.crx"),
            crx3(&zip_bytes(&[
                ("manifest.json", r#"{"name":"__MSG_fullName__","version":"5.0","default_locale":"en"}"#),
                // Keys are case-insensitive; the app's language comes first.
                ("_locales/en/messages.json", r#"{"FULLNAME":{"message":"SponsorBlock"}}"#),
                ("_locales/es/messages.json", r#"{"fullName":{"message":"SponsorBlock (es)"}}"#),
            ])),
        )
        .unwrap();
        // Junk.
        fs::create_dir_all(dir.join("empty")).unwrap();
        fs::write(dir.join("broken.crx"), b"Cr24\x03\x00\x00\x00").unwrap();
        fs::write(dir.join("README.txt"), "hello").unwrap();

        let scan = scan(&dir);
        let names: Vec<_> = scan.extensions.iter().map(|e| (e.folder.as_str(), e.name.as_str())).collect();
        assert_eq!(
            names,
            [("manual", "Demo"), ("Sponsor Block_", "SponsorBlock"), ("uBOLite.chromium", "uBO Lite")]
        );
        let manual = &scan.extensions[0];
        assert_eq!(manual.version, "1.2.3");
        assert_eq!(manual.page.as_deref(), Some("options.html"));
        assert_eq!(scan.extensions[2].page.as_deref(), Some("popup.html"));
        assert!(scan.extensions[2].path.ends_with("uBOLite"));

        // Archives moved away; the broken one stays and is reported.
        assert!(dir.join(".archives/uBOLite.chromium.zip").is_file());
        assert!(!dir.join("uBOLite.chromium.zip").exists());
        assert!(dir.join("broken.crx").is_file());
        assert_eq!(scan.problems.len(), 2, "{:?}", scan.problems);
        assert!(scan.problems.iter().any(|p| p.contains("empty")));
        assert!(scan.problems.iter().any(|p| p.contains("broken.crx")));

        // A second scan neither duplicates nor unpacks again.
        let again = super::scan(&dir);
        assert_eq!(again.extensions.len(), 3);

        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn old_archive_folder_is_renamed() {
        let dir = temp_dir("rename");
        fs::create_dir_all(dir.join(".archivos")).unwrap();
        fs::write(dir.join(".archivos/old.zip"), b"x").unwrap();
        let _ = scan(&dir);
        assert!(dir.join(".archives/old.zip").is_file() && !dir.join(".archivos").exists());
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
        fs::write(dir.join("evil.zip"), zip_bytes(&[("../../outside.txt", "x"), ("manifest.json", MANIFEST)])).unwrap();
        let scan = scan(&dir);
        assert!(scan.extensions.is_empty());
        assert!(scan.problems[0].contains("../../outside.txt"), "{:?}", scan.problems);
        assert!(!dir.parent().unwrap().join("outside.txt").exists());
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
        assert_eq!(read_manifest(&dir).unwrap().page, None);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn fingerprint_and_permissions() {
        let dir = temp_dir("fp");
        fs::create_dir_all(dir.join("ext/js")).unwrap();
        fs::write(
            dir.join("ext/manifest.json"),
            r#"{"manifest_version":3,"name":"Reader","version":"1","permissions":["storage","tabs"],
                "host_permissions":["https://*.youtube.com/*"],"content_scripts":[{"matches":["<all_urls>"],"js":["js/a.js"]}]}"#,
        )
        .unwrap();
        fs::write(dir.join("ext/js/a.js"), "console.log(1)").unwrap();
        let first = scan(&dir);
        let ext = &first.extensions[0];
        assert_eq!(ext.fingerprint.len(), 64);
        assert_eq!(ext.permissions, ["<all_urls>", "https://*.youtube.com/*", "storage", "tabs"]);
        assert!(ext.all_sites);
        // Same folder → same fingerprint; one different byte → another fingerprint.
        assert_eq!(fingerprint(&dir.join("ext")).unwrap(), ext.fingerprint);
        fs::write(dir.join("ext/js/a.js"), "console.log(2)").unwrap();
        assert_ne!(fingerprint(&dir.join("ext")).unwrap(), ext.fingerprint);
        // Moving a file elsewhere also changes the fingerprint.
        fs::write(dir.join("ext/js/a.js"), "console.log(1)").unwrap();
        fs::rename(dir.join("ext/js/a.js"), dir.join("ext/a.js")).unwrap();
        assert_ne!(fingerprint(&dir.join("ext")).unwrap(), ext.fingerprint);
        // Names only (a language change): same list, nothing hashed.
        let names = scan_names(&dir);
        assert_eq!(names.extensions[0].name, "Reader");
        assert!(names.extensions[0].fingerprint.is_empty() && names.problems.is_empty());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn fingerprint_is_stable_and_bounded() {
        // The exact bytes hashed are part of the approvals people already
        // gave: this value must never change.
        let dir = temp_dir("fp-stable");
        fs::create_dir_all(dir.join("sub")).unwrap();
        fs::write(dir.join("manifest.json"), "{}").unwrap();
        fs::write(dir.join("sub/b.js"), "b").unwrap();
        let mut hasher = Sha256::new();
        for (path, content) in [("manifest.json", "{}"), ("sub/b.js", "b")] {
            hasher.update(path.as_bytes());
            hasher.update([0]);
            hasher.update((content.len() as u64).to_le_bytes());
            hasher.update(content.as_bytes());
        }
        let expected: String = hasher.finalize().iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(fingerprint(&dir).unwrap(), expected);
        // A file bigger than the whole budget is refused by its size, before
        // it is read (a sparse file: nothing is really written).
        let big = fs::File::create(dir.join("huge.bin")).unwrap();
        big.set_len(MAX_UNPACKED + 1).unwrap();
        drop(big);
        let err = fingerprint(&dir).unwrap_err();
        assert_eq!(fingerprint_error(&err), t!("ext.too_big"));
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn zip_bombs_are_rejected() {
        let dir = temp_dir("bomb");
        // 300 MB of zeros compress to almost nothing.
        let mut buf = Cursor::new(Vec::new());
        {
            let mut zip = zip::ZipWriter::new(&mut buf);
            zip.start_file("manifest.json", zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(MANIFEST.as_bytes()).unwrap();
            zip.start_file("padding.bin", zip::write::SimpleFileOptions::default()).unwrap();
            let chunk = vec![0u8; 1024 * 1024];
            for _ in 0..300 {
                zip.write_all(&chunk).unwrap();
            }
            zip.finish().unwrap();
        }
        let bytes = buf.into_inner();
        assert!(bytes.len() < 2 * 1024 * 1024, "the compressed ZIP is small ({})", bytes.len());
        fs::write(dir.join("bomb.zip"), bytes).unwrap();
        let scan = scan(&dir);
        assert!(scan.extensions.is_empty());
        assert!(scan.problems.iter().any(|p| p.contains("256")), "{:?}", scan.problems);
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
