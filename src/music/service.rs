//! Servicio de música: un hilo dueño de la biblioteca que atiende a la
//! página "Mi música", lanza el trabajo pesado en hilos aparte (importar,
//! buscar, descargar, analizar duplicados, licencias) y guarda todo cifrado.

use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};

use super::{
    archive::{self, Hit, Item, RemoteTrack},
    download::{self, Downloader, Job, Progress},
    fingerprint,
    library::{self, Candidate, DuplicateGroup, Existing, Library, Playlist, Quality, Source, Track},
    os, scan, serve,
};
use crate::{
    config::Store,
    license::{self, LicenseError, LicenseRecord, Plan, StoreConfig},
    net::Http,
};

const LIBRARY: &str = "music-library";
const FINGERPRINTS: &str = "music-fingerprints";
const LICENSE: &str = "music-license";
const SAVE_EVERY: Duration = Duration::from_secs(2);
const MAX_JOBS_SHOWN: usize = 300;
const MAX_ITEMS_CACHED: usize = 30;
/// Canciones por tanda al importar (para que la página vaya viendo).
const SCAN_BATCH: usize = 150;
pub const DOWNLOADS_FOLDER: &str = "YT Mini Player";

// ---------------------------------------------------------------------------
// Órdenes de la página
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    Ready,
    /// Elegir carpeta: lo hace la app (diálogo del sistema) y luego `Import`.
    PickFolder,
    Import(PathBuf),
    Rescan,
    RemoveFolder(String),
    Search { query: String, page: u32, popular: bool },
    Item(String),
    Download { item: String, files: Vec<String>, playlist: bool },
    Cancel(Option<u64>),
    ClearJobs,
    SavePlaylist { id: Option<String>, name: String, tracks: Vec<String> },
    DeletePlaylist(String),
    FindDuplicates,
    IgnoreDuplicates(Vec<String>),
    Remove { ids: Vec<String>, trash: bool },
    Reveal(String),
    OpenFolder,
    Activate(String),
    Deactivate,
    RefreshLicense,
    Buy(Plan),
    ManageSubscription,
    SetQuality(Quality),
}

#[derive(Deserialize)]
struct SearchArgs {
    q: String,
    #[serde(default)]
    page: u32,
    #[serde(default)]
    popular: bool,
}

#[derive(Deserialize)]
struct DownloadArgs {
    item: String,
    files: Vec<String>,
    #[serde(default)]
    playlist: bool,
}

#[derive(Deserialize)]
struct PlaylistArgs {
    id: Option<String>,
    name: String,
    tracks: Vec<String>,
}

#[derive(Deserialize)]
struct RemoveArgs {
    ids: Vec<String>,
    #[serde(default)]
    trash: bool,
}

fn is_track_id(id: &str) -> bool {
    id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
}

fn is_playlist_id(id: &str) -> bool {
    (8..=32).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric())
}

impl Request {
    /// Mensaje de la página (sin el prefijo `music:`). Lista cerrada y
    /// validada: nombres, ids y tamaños acotados.
    pub fn parse(message: &str) -> Option<Self> {
        let (head, arg) = match message.split_once(':') {
            Some((h, a)) => (h, Some(a)),
            None => (message, None),
        };
        fn json(a: Option<&str>) -> Option<&str> {
            a.filter(|a| a.len() <= 512 * 1024)
        }
        Some(match (head, arg) {
            ("ready", None) => Request::Ready,
            ("pick-folder", None) => Request::PickFolder,
            ("rescan", None) => Request::Rescan,
            ("folder-remove", Some(path)) if !path.is_empty() && path.len() <= 1024 => Request::RemoveFolder(path.to_owned()),
            ("search", a) => {
                let args: SearchArgs = serde_json::from_str(json(a)?).ok()?;
                let query = args.q.trim().to_owned();
                if query.is_empty() || query.chars().count() > 200 {
                    return None;
                }
                Request::Search { query, page: args.page.clamp(1, 100), popular: args.popular }
            }
            ("item", Some(id)) if archive::is_identifier(id) => Request::Item(id.to_owned()),
            ("download", a) => {
                let args: DownloadArgs = serde_json::from_str(json(a)?).ok()?;
                if !archive::is_identifier(&args.item) || args.files.is_empty() || args.files.len() > 500 {
                    return None;
                }
                if args.files.iter().any(|f| f.is_empty() || f.len() > 500 || f.contains("..")) {
                    return None;
                }
                Request::Download { item: args.item, files: args.files, playlist: args.playlist }
            }
            ("cancel", Some("all")) => Request::Cancel(None),
            ("cancel", Some(id)) => Request::Cancel(Some(id.parse().ok()?)),
            ("clear-jobs", None) => Request::ClearJobs,
            ("playlist-save", a) => {
                let args: PlaylistArgs = serde_json::from_str(json(a)?).ok()?;
                let name: String = args.name.chars().filter(|c| !c.is_control()).collect::<String>().trim().to_owned();
                if name.is_empty() || name.chars().count() > 100 || args.tracks.len() > 20_000 {
                    return None;
                }
                if args.id.as_deref().is_some_and(|id| !is_playlist_id(id)) || !args.tracks.iter().all(|t| is_track_id(t)) {
                    return None;
                }
                Request::SavePlaylist { id: args.id, name, tracks: args.tracks }
            }
            ("playlist-delete", Some(id)) if is_playlist_id(id) => Request::DeletePlaylist(id.to_owned()),
            ("dupes", None) => Request::FindDuplicates,
            ("dupes-ignore", a) => {
                let ids: Vec<String> = serde_json::from_str(json(a)?).ok()?;
                (ids.len() >= 2 && ids.len() <= 1000 && ids.iter().all(|t| is_track_id(t))).then_some(Request::IgnoreDuplicates(ids))?
            }
            ("remove", a) => {
                let args: RemoveArgs = serde_json::from_str(json(a)?).ok()?;
                if args.ids.is_empty() || args.ids.len() > 20_000 || !args.ids.iter().all(|t| is_track_id(t)) {
                    return None;
                }
                Request::Remove { ids: args.ids, trash: args.trash }
            }
            ("reveal", Some(id)) if is_track_id(id) => Request::Reveal(id.to_owned()),
            ("open-folder", None) => Request::OpenFolder,
            ("license-activate", Some(key)) if !key.is_empty() && key.len() <= 200 => Request::Activate(key.trim().to_owned()),
            ("license-deactivate", None) => Request::Deactivate,
            ("license-refresh", None) => Request::RefreshLicense,
            ("buy", Some("downloads")) => Request::Buy(Plan::Downloads),
            ("buy", Some("lifetime")) => Request::Buy(Plan::Lifetime),
            ("manage", None) => Request::ManageSubscription,
            ("quality", Some("compact")) => Request::SetQuality(Quality::Compact),
            ("quality", Some("best")) => Request::SetQuality(Quality::Best),
            _ => return None,
        })
    }
}

// ---------------------------------------------------------------------------
// Avisos a la página
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct TrackView<'a> {
    id: &'a str,
    title: &'a str,
    artist: &'a str,
    album: &'a str,
    track_no: Option<u32>,
    year: Option<u32>,
    duration: f64,
    size: u64,
    format: &'a str,
    bitrate: Option<u32>,
    cover: Option<&'a str>,
    source: &'static str,
    license: &'a str,
    item: &'a str,
    path: String,
    added: u64,
}

impl<'a> TrackView<'a> {
    fn new(t: &'a Track) -> Self {
        let (source, license, item) = match &t.source {
            Source::Local => ("local", "", ""),
            Source::Archive { license, item, .. } => ("archive", license.as_str(), item.as_str()),
        };
        Self {
            id: &t.id,
            title: &t.title,
            artist: &t.artist,
            album: &t.album,
            track_no: t.track_no,
            year: t.year,
            duration: t.duration,
            size: t.size,
            format: &t.format,
            bitrate: t.bitrate,
            cover: t.cover.as_deref(),
            source,
            license,
            item,
            path: t.path.to_string_lossy().into_owned(),
            added: t.added,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct JobView {
    pub id: u64,
    pub item: String,
    pub file: String,
    pub title: String,
    pub artist: String,
    /// queued, downloading, processing, done, skipped, failed, cancelled
    pub state: &'static str,
    pub received: u64,
    pub size: u64,
    pub message: String,
    pub track_id: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ExistingView<'a> {
    reason: Existing,
    text: &'static str,
    track_id: &'a str,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct RemoteView<'a> {
    #[serde(flatten)]
    track: &'a RemoteTrack,
    existing: Option<ExistingView<'a>>,
    queued: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
struct PlanView {
    plan: Plan,
    configured: bool,
    dev: bool,
    entitlements: Option<license::Entitlements>,
    /// Playlists que hay ahora (para "2 de 5").
    playlists: usize,
    email: String,
    key: String,
    product: String,
    status: String,
    expires_at: Option<String>,
    validated_at: u64,
    busy: bool,
    message: Option<String>,
    error: bool,
    can_buy_downloads: bool,
    can_buy_lifetime: bool,
    can_manage: bool,
}

// ---------------------------------------------------------------------------
// Servicio
// ---------------------------------------------------------------------------

pub struct Config {
    /// `%LOCALAPPDATA%\yt_mini_player\music`
    pub dir: PathBuf,
    /// Carpeta de las descargas (`Música\YT Mini Player`).
    pub downloads: Option<PathBuf>,
    pub store: Store,
    pub archive: String,
    pub license_api: String,
    pub shop: Option<StoreConfig>,
    pub dev_unlock: bool,
}

enum Msg {
    Request(Request),
    ScanBatch(Vec<Track>),
    ScanProgress { done: usize, total: usize },
    ScanDone { roots: Vec<PathBuf>, present: HashSet<String>, failed: usize },
    Searched { query: String, page: u32, result: Result<(u64, Vec<Hit>), String> },
    ItemLoaded { id: String, result: Result<Item, String> },
    ItemCover { item: String, cover: String },
    Download(Progress),
    DupesProgress { done: usize, total: usize },
    DupesDone { groups: Vec<DuplicateGroup>, fingerprints: Vec<(String, String)> },
    License { action: LicenseAction, result: Result<Option<LicenseRecord>, LicenseError> },
    Shutdown(Sender<()>),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LicenseAction {
    Activate,
    Validate,
    Deactivate,
}

pub struct Handle {
    tx: Sender<Msg>,
}

impl Handle {
    pub fn request(&self, request: Request) {
        let _ = self.tx.send(Msg::Request(request));
    }

    /// Guarda y para (espera como mucho `wait`).
    pub fn shutdown(&self, wait: Duration) {
        let (ack, done) = mpsc::channel();
        if self.tx.send(Msg::Shutdown(ack)).is_ok() {
            let _ = done.recv_timeout(wait);
        }
    }
}

/// Lo que espera una descarga para terminar: a qué playlist (y en qué
/// posición) va y de qué álbum viene.
struct PendingJob {
    item: String,
    remote: RemoteTrack,
    license: String,
    creator: String,
    playlist: Option<(String, usize)>,
}

struct Service {
    cfg: Config,
    tx: Sender<Msg>,
    emit: Box<dyn Fn(String) + Send>,
    http: Option<Arc<Http>>,
    library: Library,
    fingerprints: HashMap<String, String>,
    license: Option<LicenseRecord>,
    license_busy: bool,
    license_message: Option<(String, bool)>,
    downloader: Option<Downloader>,
    jobs: BTreeMap<u64, JobView>,
    pending: HashMap<u64, PendingJob>,
    /// Playlists en construcción: id → posiciones (álbum en orden).
    slots: HashMap<String, Vec<Option<String>>>,
    next_job: u64,
    items: Vec<Item>,
    item_covers: HashMap<String, String>,
    scanning: Option<Arc<AtomicBool>>,
    scan_queue: Vec<PathBuf>,
    scan_added: usize,
    dupes_running: bool,
    library_dirty: bool,
    fingerprints_dirty: bool,
    jobs_dirty: bool,
    last_save: Instant,
    last_jobs_emit: Instant,
    /// Última revisión de la licencia (None = aún ninguna). Nunca se resta
    /// tiempo a `Instant::now()`: en Windows cuenta desde el arranque del
    /// equipo y restar más de lo que lleva encendido abortaría la app.
    last_license_check: Option<Instant>,
    ready_once: bool,
}

fn now() -> u64 {
    crate::lock::unix_now()
}

pub fn start(cfg: Config, emit: impl Fn(String) + Send + 'static) -> Handle {
    // La clave de las canciones existe antes de que la página la pida.
    #[cfg(not(test))]
    if let Ok(key) = crate::policy::new_token() {
        serve::init(key, cfg.dir.join("covers"));
    }
    let (tx, rx) = mpsc::channel();
    let handle = Handle { tx: tx.clone() };
    let _ = thread::Builder::new().name("musica".into()).spawn(move || {
        let service = Service::new(cfg, tx, Box::new(emit));
        service.run(rx);
    });
    handle
}

impl Service {
    fn new(cfg: Config, tx: Sender<Msg>, emit: Box<dyn Fn(String) + Send>) -> Self {
        let _ = fs::create_dir_all(cfg.dir.join("covers"));
        let library: Library = cfg.store.load(&cfg.dir.join("library.enc"), LIBRARY).unwrap_or_default();
        let fingerprints = cfg.store.load(&cfg.dir.join("fingerprints.enc"), FINGERPRINTS).unwrap_or_default();
        let license = cfg.store.load(&cfg.dir.join("license.enc"), LICENSE);
        let http = Http::new().map(Arc::new).ok();
        Self {
            cfg,
            tx,
            emit,
            http,
            library,
            fingerprints,
            license,
            license_busy: false,
            license_message: None,
            downloader: None,
            jobs: BTreeMap::new(),
            pending: HashMap::new(),
            slots: HashMap::new(),
            next_job: 1,
            items: Vec::new(),
            item_covers: HashMap::new(),
            scanning: None,
            scan_queue: Vec::new(),
            scan_added: 0,
            dupes_running: false,
            library_dirty: false,
            fingerprints_dirty: false,
            jobs_dirty: false,
            last_save: Instant::now(),
            last_jobs_emit: Instant::now(),
            last_license_check: None,
            ready_once: false,
        }
    }

    fn run(mut self, rx: Receiver<Msg>) {
        os::init_thread();
        self.publish_tracks();
        loop {
            match rx.recv_timeout(Duration::from_millis(400)) {
                Ok(Msg::Shutdown(ack)) => {
                    self.stop();
                    let _ = ack.send(());
                    return;
                }
                Ok(msg) => self.handle(msg),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    self.stop();
                    return;
                }
            }
            self.tick();
        }
    }

    fn stop(&mut self) {
        if let Some(scan) = &self.scanning {
            scan.store(true, Ordering::Relaxed);
        }
        if let Some(d) = &self.downloader {
            d.shutdown();
        }
        self.save(true);
    }

    fn tick(&mut self) {
        self.save(false);
        if self.jobs_dirty && self.last_jobs_emit.elapsed() >= Duration::from_millis(350) {
            self.emit_jobs();
        }
        if self.last_license_check.is_none_or(|t| t.elapsed() >= Duration::from_secs(3600)) {
            self.last_license_check = Some(Instant::now());
            if self.license.as_ref().is_some_and(|r| r.needs_validation(now()) && !r.key.is_empty()) {
                self.validate_license();
            }
        }
    }

    fn save(&mut self, force: bool) {
        if !force && self.last_save.elapsed() < SAVE_EVERY {
            return;
        }
        if self.library_dirty {
            if let Err(err) = self.cfg.store.save(&self.cfg.dir.join("library.enc"), LIBRARY, &self.library) {
                eprintln!("no se pudo guardar la biblioteca: {err}");
            }
            self.library_dirty = false;
        }
        if self.fingerprints_dirty {
            let _ = self.cfg.store.save(&self.cfg.dir.join("fingerprints.enc"), FINGERPRINTS, &self.fingerprints);
            self.fingerprints_dirty = false;
        }
        self.last_save = Instant::now();
    }

    fn send(&self, value: serde_json::Value) {
        if let Ok(json) = serde_json::to_string(&value) {
            (self.emit)(json);
        }
    }

    fn toast(&self, text: &str) {
        self.send(serde_json::json!({ "type": "toast", "text": text }));
    }

    /// Un límite del plan: la página lo explica en línea, sin ventanas.
    fn notice(&self, feature: &str, text: &str) {
        self.send(serde_json::json!({ "type": "notice", "feature": feature, "text": text }));
    }

    fn playlist_limit_text(&self) -> String {
        let plan = self.plan();
        let max = plan.entitlements().max_playlists.unwrap_or(0);
        match plan {
            Plan::Free => format!(
                "Tienes {max} playlists, el máximo del plan Gratis. Con Descargas puedes tener {}; con De por vida, sin límite.",
                license::DOWNLOADS_PLAYLISTS
            ),
            _ => format!("Tienes {max} playlists, el máximo de tu plan. Con De por vida no hay límite."),
        }
    }

    // -- Plan -----------------------------------------------------------------

    fn plan(&self) -> Plan {
        if self.cfg.dev_unlock {
            return Plan::Lifetime;
        }
        self.license.as_ref().map_or(Plan::Free, |r| r.effective_plan(now()))
    }

    fn emit_plan(&self) {
        let plan = self.plan();
        let record = self.license.as_ref();
        let shop = self.cfg.shop.as_ref();
        let view = PlanView {
            plan,
            configured: shop.is_some(),
            dev: self.cfg.dev_unlock,
            entitlements: Some(plan.entitlements()),
            playlists: self.library.playlists.len(),
            email: record.map(|r| license::mask_email(&r.customer_email)).unwrap_or_default(),
            key: record.filter(|r| !r.key.is_empty()).map(|r| license::mask_key(&r.key)).unwrap_or_default(),
            product: record.map(|r| r.product_name.clone()).unwrap_or_default(),
            status: record.map(|r| r.status.clone()).unwrap_or_default(),
            expires_at: record.and_then(|r| r.expires_at.clone()),
            validated_at: record.map_or(0, |r| r.validated_at),
            busy: self.license_busy,
            message: self.license_message.as_ref().map(|(m, _)| m.clone()),
            error: self.license_message.as_ref().is_some_and(|(_, e)| *e),
            can_buy_downloads: shop.is_some_and(|s| s.checkout_downloads.is_some()),
            can_buy_lifetime: shop.is_some_and(|s| s.checkout_lifetime.is_some()),
            can_manage: shop.is_some_and(|s| s.portal.is_some()),
        };
        if let Ok(mut value) = serde_json::to_value(&view) {
            value["type"] = "plan".into();
            self.send(value);
        }
    }

    fn save_license(&self) {
        let path = self.cfg.dir.join("license.enc");
        match &self.license {
            Some(record) => {
                if let Err(err) = self.cfg.store.save(&path, LICENSE, record) {
                    eprintln!("no se pudo guardar la licencia: {err}");
                }
            }
            None => {
                let _ = fs::remove_file(path);
            }
        }
    }

    fn license_job(&mut self, action: LicenseAction, work: impl FnOnce(&Http, &str, &StoreConfig) -> Result<Option<LicenseRecord>, LicenseError> + Send + 'static) {
        let (Some(http), Some(shop)) = (self.http.clone(), self.cfg.shop.clone()) else {
            self.license_message = Some(("Esta compilación no tiene la tienda configurada.".into(), true));
            self.emit_plan();
            return;
        };
        self.license_busy = true;
        self.emit_plan();
        let api = self.cfg.license_api.clone();
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = work(&http, &api, &shop);
            let _ = tx.send(Msg::License { action, result });
        });
    }

    fn validate_license(&mut self) {
        let Some(record) = self.license.clone().filter(|r| !r.key.is_empty()) else { return };
        if self.license_busy || self.cfg.shop.is_none() {
            return;
        }
        self.license_job(LicenseAction::Validate, move |http, api, shop| {
            license::validate(http, api, shop, &record, now()).map(Some)
        });
    }

    fn on_license(&mut self, action: LicenseAction, result: Result<Option<LicenseRecord>, LicenseError>) {
        self.license_busy = false;
        match (action, result) {
            (LicenseAction::Activate, Ok(Some(record))) => {
                let plan = record.plan;
                self.license = Some(record);
                self.save_license();
                let name = if plan == Plan::Lifetime { "De por vida" } else { "Descargas" };
                self.license_message = Some((format!("¡Listo! Plan {name} activado en este equipo."), false));
            }
            (LicenseAction::Validate, Ok(Some(record))) => {
                self.license = Some(record);
                self.save_license();
                self.license_message = None;
            }
            (LicenseAction::Deactivate, Ok(_)) => {
                self.license = None;
                self.save_license();
                self.license_message = Some(("Licencia desactivada en este equipo.".into(), false));
            }
            (LicenseAction::Validate, Err(LicenseError::Rejected(message))) => {
                // Ya no vale: se conserva la clave para reactivar, sin plan.
                if let Some(record) = &mut self.license {
                    record.status = "invalid".into();
                }
                self.save_license();
                self.license_message = Some((message, true));
            }
            (LicenseAction::Validate, Err(LicenseError::Offline(_))) => {
                let days = self.license.as_ref().map_or(0, |r| now().saturating_sub(r.validated_at) / 86_400);
                if days >= 1 {
                    self.license_message =
                        Some((format!("Sin conexión con la tienda desde hace {days} día(s); se usa la última validación."), true));
                }
            }
            (_, Err(err)) => self.license_message = Some((err.to_string(), true)),
            (_, Ok(None)) => {}
        }
        self.emit_plan();
        self.emit_library();
    }

    // -- Biblioteca ---------------------------------------------------------------

    fn publish_tracks(&self) {
        if let Some(shared) = serve::shared() {
            let map = self.library.tracks.iter().map(|t| (t.id.clone(), (t.path.clone(), t.format.clone()))).collect();
            shared.set_tracks(map);
        }
    }

    fn emit_library(&self) {
        let tracks: Vec<TrackView> = self.library.tracks.iter().map(TrackView::new).collect();
        let value = serde_json::json!({
            "type": "library",
            "tracks": tracks,
            "playlists": self.library.playlists,
            "folders": self.library.folders.iter().map(|f| f.to_string_lossy()).collect::<Vec<_>>(),
            "quality": self.library.quality,
            "downloads": self.cfg.downloads.as_ref().map(|d| d.to_string_lossy().into_owned()),
            "mediaKey": serve::shared().map(|s| s.key().to_owned()),
        });
        self.send(value);
    }

    fn library_changed(&mut self) {
        self.library_dirty = true;
        self.publish_tracks();
        self.emit_library();
    }

    fn handle(&mut self, msg: Msg) {
        match msg {
            Msg::Request(request) => self.request(request),
            Msg::ScanBatch(tracks) => {
                for track in tracks {
                    if self.library.track(&track.id).is_none() {
                        self.scan_added += 1;
                    }
                    self.library.upsert(track);
                }
                self.library_changed();
            }
            Msg::ScanProgress { done, total } => {
                self.send(serde_json::json!({ "type": "scan", "running": true, "done": done, "total": total, "added": self.scan_added }));
            }
            Msg::ScanDone { roots, present, failed } => {
                let missing: Vec<String> = self
                    .library
                    .tracks
                    .iter()
                    .filter(|t| roots.iter().any(|r| t.path.starts_with(r)) && !present.contains(&t.id))
                    .map(|t| t.id.clone())
                    .collect();
                self.library.remove(&missing);
                self.scanning = None;
                let added = std::mem::take(&mut self.scan_added);
                let mut message = match added {
                    0 => "Biblioteca al día.".to_owned(),
                    1 => "1 canción nueva.".to_owned(),
                    n => format!("{n} canciones nuevas."),
                };
                if failed > 0 {
                    message.push_str(&format!(" {failed} archivo(s) no se pudieron leer."));
                }
                self.send(serde_json::json!({ "type": "scan", "running": false, "added": added, "removed": missing.len(), "message": message }));
                self.library_changed();
                if !self.scan_queue.is_empty() {
                    let roots = std::mem::take(&mut self.scan_queue);
                    self.start_scan(roots);
                }
            }
            Msg::Searched { query, page, result } => {
                let value = match result {
                    Ok((total, hits)) => serde_json::json!({ "type": "search", "query": query, "page": page, "total": total, "hits": hits }),
                    Err(error) => serde_json::json!({ "type": "search", "query": query, "page": page, "error": error }),
                };
                self.send(value);
            }
            Msg::ItemLoaded { id, result } => match result {
                Ok(item) => {
                    self.items.retain(|i| i.id != item.id);
                    self.items.push(item);
                    if self.items.len() > MAX_ITEMS_CACHED {
                        self.items.remove(0);
                    }
                    self.emit_item(&id);
                }
                Err(error) => self.send(serde_json::json!({ "type": "item", "id": id, "error": error })),
            },
            Msg::ItemCover { item, cover } => {
                self.item_covers.insert(item.clone(), cover.clone());
                let mut changed = false;
                for t in &mut self.library.tracks {
                    if t.cover.is_none() && matches!(&t.source, Source::Archive { item: i, .. } if *i == item) {
                        t.cover = Some(cover.clone());
                        changed = true;
                    }
                }
                if changed {
                    self.library_changed();
                }
            }
            Msg::Download(progress) => self.on_download(progress),
            Msg::DupesProgress { done, total } => {
                self.send(serde_json::json!({ "type": "dupes", "running": true, "done": done, "total": total }));
            }
            Msg::DupesDone { groups, fingerprints } => {
                self.dupes_running = false;
                if !fingerprints.is_empty() {
                    self.fingerprints.extend(fingerprints);
                    self.fingerprints_dirty = true;
                }
                let wasted: u64 = groups.iter().map(|g| g.wasted).sum();
                self.send(serde_json::json!({ "type": "dupes", "running": false, "groups": groups, "wasted": wasted }));
            }
            Msg::License { action, result } => self.on_license(action, result),
            Msg::Shutdown(_) => {}
        }
    }

    fn request(&mut self, request: Request) {
        match request {
            Request::Ready => {
                self.emit_plan();
                self.emit_library();
                self.emit_jobs();
                if !self.ready_once {
                    self.ready_once = true;
                    // Al abrir: revisar las carpetas (archivos nuevos o borrados)
                    // y validar la licencia si toca.
                    let roots = self.library.folders.clone();
                    if !roots.is_empty() {
                        self.start_scan(roots);
                    }
                    if self.license.as_ref().is_some_and(|r| r.needs_validation(now())) {
                        self.validate_license();
                    }
                }
            }
            // La app la convierte en Import tras el diálogo.
            Request::PickFolder => {}
            Request::Import(path) => self.import(path),
            Request::Rescan => {
                let roots = self.library.folders.clone();
                if roots.is_empty() {
                    self.toast("Primero añade una carpeta con tu música.");
                } else {
                    self.start_scan(roots);
                }
            }
            Request::RemoveFolder(path) => {
                let path = PathBuf::from(path);
                if let Some(pos) = self.library.folders.iter().position(|f| *f == path) {
                    self.library.folders.remove(pos);
                    let ids: Vec<String> = self
                        .library
                        .tracks
                        .iter()
                        .filter(|t| t.path.starts_with(&path) && matches!(t.source, Source::Local))
                        .map(|t| t.id.clone())
                        .collect();
                    self.library.remove(&ids);
                    self.toast("Carpeta quitada de la biblioteca (los archivos siguen en su sitio).");
                    self.library_changed();
                }
            }
            Request::Search { query, page, popular } => self.search(query, page, popular),
            Request::Item(id) => {
                if self.items.iter().any(|i| i.id == id) {
                    self.emit_item(&id);
                } else {
                    self.load_item(id);
                }
            }
            Request::Download { item, files, playlist } => self.download(&item, &files, playlist),
            Request::Cancel(id) => {
                if let Some(d) = &self.downloader {
                    for removed in d.cancel(id) {
                        self.finish_job(removed, "cancelled", "Cancelada".into(), None);
                    }
                }
            }
            Request::ClearJobs => {
                self.jobs.retain(|_, j| matches!(j.state, "queued" | "downloading" | "processing"));
                self.emit_jobs();
            }
            Request::SavePlaylist { id, name, tracks } => {
                // Editar una que ya existe siempre se puede (aunque haya más
                // de las del plan); crear una nueva, dentro del límite.
                let exists = id.as_deref().is_some_and(|id| self.library.playlists.iter().any(|p| p.id == id));
                if !exists && !self.plan().entitlements().can_add_playlist(self.library.playlists.len()) {
                    self.notice("playlists", &self.playlist_limit_text());
                    return;
                }
                let tracks: Vec<String> = tracks.into_iter().filter(|t| self.library.track(t).is_some()).collect();
                let stamp = now();
                match id.as_deref().and_then(|id| self.library.playlist_mut(id)) {
                    Some(p) => {
                        p.name = name;
                        p.tracks = tracks;
                        p.updated = stamp;
                    }
                    None => {
                        let id = new_id();
                        self.library.playlists.push(Playlist { id, name, tracks, created: stamp, updated: stamp, origin: None });
                    }
                }
                self.library_changed();
                self.emit_plan();
            }
            Request::DeletePlaylist(id) => {
                self.library.playlists.retain(|p| p.id != id);
                self.slots.remove(&id);
                self.library_changed();
                self.emit_plan();
            }
            Request::FindDuplicates => self.find_duplicates(),
            Request::IgnoreDuplicates(mut ids) => {
                ids.sort();
                if !self.library.ignored_duplicates.contains(&ids) {
                    self.library.ignored_duplicates.push(ids);
                    self.library_dirty = true;
                }
            }
            Request::Remove { ids, trash } => self.remove(ids, trash),
            Request::Reveal(id) => {
                if let Some(t) = self.library.track(&id) {
                    os::reveal(&t.path);
                }
            }
            Request::OpenFolder => {
                if let Some(dir) = &self.cfg.downloads {
                    let _ = fs::create_dir_all(dir);
                    crate::platform::open_path(dir);
                }
            }
            Request::Activate(key) => {
                if !license::plausible_key(&key) {
                    self.license_message = Some(("Esa clave no tiene el formato de una licencia.".into(), true));
                    self.emit_plan();
                    return;
                }
                // Si había otra activada en este equipo, se libera antes.
                let previous = self.license.clone().filter(|r| !r.key.is_empty() && r.key != key && !r.instance_id.is_empty());
                self.license_job(LicenseAction::Activate, move |http, api, shop| {
                    if let Some(previous) = &previous {
                        let _ = license::deactivate(http, api, previous);
                    }
                    license::activate(http, api, shop, &key, &license::instance_name(), now()).map(Some)
                });
            }
            Request::Deactivate => {
                let Some(record) = self.license.clone() else { return };
                self.license_job(LicenseAction::Deactivate, move |http, api, _| match license::deactivate(http, api, &record) {
                    // Si la tienda ya no la conoce, igualmente se olvida aquí.
                    Ok(()) | Err(LicenseError::Rejected(_)) => Ok(None),
                    Err(err) => Err(err),
                });
            }
            Request::RefreshLicense => {
                self.license_message = None;
                self.validate_license();
            }
            Request::Buy(plan) => {
                let url = self.cfg.shop.as_ref().and_then(|s| match plan {
                    Plan::Lifetime => s.checkout_lifetime.clone(),
                    _ => s.checkout_downloads.clone(),
                });
                match url {
                    Some(url) => {
                        crate::security::open_in_browser(&url);
                        self.license_message = Some((
                            "Se abrió la página de pago en tu navegador. Al terminar recibirás la clave por correo: pégala aquí.".into(),
                            false,
                        ));
                    }
                    None => self.license_message = Some(("Esta compilación no tiene la tienda configurada.".into(), true)),
                }
                self.emit_plan();
            }
            Request::ManageSubscription => {
                if let Some(url) = self.cfg.shop.as_ref().and_then(|s| s.portal.clone()) {
                    crate::security::open_in_browser(&url);
                }
            }
            Request::SetQuality(quality) => {
                if self.library.quality != quality {
                    self.library.quality = quality;
                    // Los álbumes abiertos cambian de formato preferido.
                    self.items.clear();
                    self.library_changed();
                }
            }
        }
    }

    // -- Importar ---------------------------------------------------------------

    fn import(&mut self, path: PathBuf) {
        if !path.is_dir() {
            self.toast("Esa carpeta no existe.");
            return;
        }
        if self.library.folders.iter().any(|f| path.starts_with(f)) {
            self.toast("Esa carpeta ya está en la biblioteca; se revisa de nuevo.");
        } else {
            // Si la nueva contiene alguna de las anteriores, la reemplaza.
            self.library.folders.retain(|f| !f.starts_with(&path));
            self.library.folders.push(path.clone());
            self.library_dirty = true;
        }
        self.start_scan(vec![path]);
    }

    fn start_scan(&mut self, roots: Vec<PathBuf>) {
        if self.scanning.is_some() {
            for r in roots {
                if !self.scan_queue.contains(&r) {
                    self.scan_queue.push(r);
                }
            }
            return;
        }
        let cancel = Arc::new(AtomicBool::new(false));
        self.scanning = Some(cancel.clone());
        self.scan_added = 0;
        let known: HashMap<String, (u64, u64)> = self.library.tracks.iter().map(|t| (t.id.clone(), (t.size, t.modified))).collect();
        let covers = self.cfg.dir.join("covers");
        let tx = self.tx.clone();
        self.send(serde_json::json!({ "type": "scan", "running": true, "done": 0, "total": 0, "added": 0 }));
        thread::spawn(move || {
            let mut files = Vec::new();
            for root in &roots {
                files.extend(scan::audio_files(root, &cancel));
            }
            let total = files.len();
            let mut present = HashSet::new();
            let mut batch = Vec::new();
            let mut failed = 0;
            let stamp = now();
            for (i, path) in files.iter().enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    return;
                }
                let id = scan::track_id(path);
                let unchanged = scan::stamp(path).ok().is_some_and(|s| known.get(&id) == Some(&s));
                if unchanged {
                    present.insert(id);
                } else {
                    match scan::read_track(path, &covers, stamp) {
                        Ok(track) => {
                            present.insert(track.id.clone());
                            batch.push(track);
                        }
                        Err(_) => failed += 1,
                    }
                }
                if batch.len() >= SCAN_BATCH {
                    let _ = tx.send(Msg::ScanBatch(std::mem::take(&mut batch)));
                }
                if i.is_multiple_of(25) {
                    let _ = tx.send(Msg::ScanProgress { done: i, total });
                }
            }
            if !batch.is_empty() {
                let _ = tx.send(Msg::ScanBatch(batch));
            }
            let _ = tx.send(Msg::ScanDone { roots, present, failed });
        });
    }

    fn remove(&mut self, ids: Vec<String>, trash: bool) {
        let paths: Vec<PathBuf> = ids.iter().filter_map(|id| self.library.track(id)).map(|t| t.path.clone()).collect();
        if trash {
            let existing: Vec<PathBuf> = paths.iter().filter(|p| p.exists()).cloned().collect();
            if !os::to_recycle_bin(&existing) {
                self.toast("No se pudieron mover a la Papelera (¿están en uso?).");
                return;
            }
            let n = ids.len();
            self.library.remove(&ids);
            self.toast(&if n == 1 { "Enviada a la Papelera.".to_owned() } else { format!("{n} canciones enviadas a la Papelera.") });
        } else {
            self.library.remove(&ids);
        }
        self.library_changed();
    }

    // -- Internet Archive -----------------------------------------------------------

    fn search(&mut self, query: String, page: u32, popular: bool) {
        let Some(http) = self.http.clone() else {
            self.send(serde_json::json!({ "type": "search", "query": query, "page": page, "error": "Sin conexión segura disponible." }));
            return;
        };
        let Some(url) = archive::search_url(&self.cfg.archive, &query, page, popular) else { return };
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = http.get_json::<serde_json::Value>(&url).map(|json| archive::parse_search(&json)).map_err(|e| e.to_string());
            let _ = tx.send(Msg::Searched { query, page, result });
        });
    }

    fn load_item(&mut self, id: String) {
        let Some(http) = self.http.clone() else { return };
        let url = archive::metadata_url(&self.cfg.archive, &id);
        let best = self.library.quality == Quality::Best;
        let tx = self.tx.clone();
        thread::spawn(move || {
            let result = http
                .get_json::<serde_json::Value>(&url)
                .map_err(|e| e.to_string())
                .and_then(|json| archive::parse_item(&json, best));
            let _ = tx.send(Msg::ItemLoaded { id, result });
        });
    }

    fn queued(&self, item: &str, file: &str) -> bool {
        self.pending.values().any(|p| p.item == item && p.remote.file == file)
    }

    fn emit_item(&self, id: &str) {
        let Some(item) = self.items.iter().find(|i| i.id == id) else { return };
        let tracks: Vec<RemoteView> = item
            .tracks
            .iter()
            .map(|t| {
                let candidate = Candidate {
                    hash: None,
                    archive: Some((&item.id, &t.file)),
                    artist: &t.artist,
                    title: &t.title,
                    duration: t.duration,
                };
                let existing = self.library.find_existing(&candidate).map(|(reason, track)| ExistingView {
                    reason,
                    text: reason.describe(),
                    track_id: &track.id,
                });
                RemoteView { track: t, existing, queued: self.queued(&item.id, &t.file) }
            })
            .collect();
        self.send(serde_json::json!({
            "type": "item",
            "id": item.id,
            "item": {
                "id": item.id, "title": item.title, "creator": item.creator, "license": item.license_label,
                "licenseTerms": item.license_terms, "licenseUrl": item.license_url,
                "details": archive::details_url(&item.id),
                "thumbnail": archive::thumbnail_url(&self.cfg.archive, &item.id),
                "tracks": tracks,
            }
        }));
    }

    fn ensure_downloader(&mut self) -> Option<Downloader> {
        if self.downloader.is_none() {
            let http = self.http.clone()?;
            let tx = self.tx.clone();
            let tx = std::sync::Mutex::new(tx);
            self.downloader = Some(Downloader::start(http, download::WORKERS, move |p| {
                if let Ok(tx) = tx.lock() {
                    let _ = tx.send(Msg::Download(p));
                }
            }));
        }
        self.downloader.clone()
    }

    fn download(&mut self, item_id: &str, files: &[String], as_playlist: bool) {
        let rights = self.plan().entitlements();
        if !rights.download {
            self.notice("download", "Descargar necesita el plan Descargas o De por vida. Escuchar la vista previa es gratis.");
            return;
        }
        let Some(item) = self.items.iter().find(|i| i.id == item_id).cloned() else {
            self.toast("Vuelve a abrir el álbum e inténtalo de nuevo.");
            return;
        };
        let Some(root) = self.cfg.downloads.clone() else {
            self.toast("No se encontró tu carpeta de Música.");
            return;
        };
        let wanted: Vec<&RemoteTrack> = item.tracks.iter().filter(|t| files.contains(&t.file)).collect();
        if wanted.is_empty() {
            return;
        }
        // Espacio libre (con margen).
        let needed: u64 = wanted.iter().map(|t| t.size).sum();
        if let Some(free) = os::free_space(&root) {
            if needed + 200 * 1024 * 1024 > free {
                self.toast("No hay espacio suficiente en el disco para esta descarga.");
                return;
            }
        }
        let Some(downloader) = self.ensure_downloader() else {
            self.toast("No se pudo iniciar la descarga (sin conexión segura).");
            return;
        };
        if !self.library.folders.iter().any(|f| root.starts_with(f)) {
            self.library.folders.retain(|f| !f.starts_with(&root));
            self.library.folders.push(root.clone());
            self.library_dirty = true;
        }
        // Playlist del álbum (en el orden del álbum). Si ya no caben más en
        // el plan, se descarga igual, sin crearla.
        let existing = self.library.playlists.iter().find(|p| p.origin.as_deref() == Some(item.id.as_str())).map(|p| p.id.clone());
        let mut playlist_skipped = false;
        let as_playlist = as_playlist && {
            let fits = existing.is_some() || rights.can_add_playlist(self.library.playlists.len());
            playlist_skipped = !fits;
            fits
        };
        let playlist_id = if as_playlist {
            let id = existing.unwrap_or_else(|| {
                let id = new_id();
                let stamp = now();
                self.library.playlists.push(Playlist {
                    id: id.clone(),
                    name: if item.title.is_empty() { item.id.clone() } else { item.title.clone() },
                    tracks: Vec::new(),
                    created: stamp,
                    updated: stamp,
                    origin: Some(item.id.clone()),
                });
                id
            });
            self.slots.insert(id.clone(), vec![None; wanted.len()]);
            Some(id)
        } else {
            None
        };
        let artist_dir = library::safe_file_name(&item.creator, "Varios");
        let album_dir = library::safe_file_name(&item.title, &item.id);
        let mut seen_in_batch: HashSet<String> = HashSet::new();
        let mut queued = 0;
        let mut skipped = 0;
        for (slot, remote) in wanted.iter().enumerate() {
            let candidate = Candidate {
                hash: None,
                archive: Some((&item.id, &remote.file)),
                artist: &remote.artist,
                title: &remote.title,
                duration: remote.duration,
            };
            let key = library::meta_key(&remote.artist, &remote.title).unwrap_or_else(|| remote.file.clone());
            let existing = self.library.find_existing(&candidate).map(|(why, t)| (why, t.id.clone()));
            if let Some((why, track_id)) = existing {
                skipped += 1;
                self.fill_slot(playlist_id.as_deref(), slot, Some(track_id.clone()));
                let id = self.next_job_id();
                self.jobs.insert(id, job_view(id, &item.id, remote, "skipped", why.describe(), Some(track_id)));
                continue;
            }
            if self.queued(&item.id, &remote.file) || !seen_in_batch.insert(key) {
                skipped += 1;
                continue;
            }
            let id = self.next_job_id();
            let number = remote.track_no.map(|n| format!("{n:02} - ")).unwrap_or_default();
            let name = library::safe_file_name(&format!("{number}{}", remote.title), &remote.file);
            let dest = root.join(&artist_dir).join(&album_dir).join(format!("{name}.{}", remote.format));
            self.pending.insert(
                id,
                PendingJob {
                    item: item.id.clone(),
                    remote: (*remote).clone(),
                    license: item.license_label.to_owned(),
                    creator: item.creator.clone(),
                    playlist: playlist_id.clone().map(|p| (p, slot)),
                },
            );
            self.jobs.insert(id, job_view(id, &item.id, remote, "queued", "", None));
            downloader.push(Job {
                id,
                url: archive::download_url(&self.cfg.archive, &item.id, &remote.file),
                dest,
                size: remote.size,
                sha1: remote.sha1.clone(),
            });
            queued += 1;
        }
        self.trim_jobs();
        self.emit_jobs();
        self.emit_item(&item.id);
        if playlist_id.is_some() {
            self.library_changed();
        }
        let text = match (queued, skipped) {
            (0, _) => "Ya tienes todo lo de este álbum: no se descarga nada.".to_owned(),
            (q, 0) => format!("{q} descarga(s) en camino."),
            (q, s) => format!("{q} descarga(s) en camino; {s} ya las tenías (no ocupan más espacio)."),
        };
        self.toast(&text);
        if playlist_skipped {
            self.notice("playlists", &format!("Las canciones se descargan, pero sin crear la playlist del álbum. {}", self.playlist_limit_text()));
        }
        if playlist_id.is_some() {
            self.emit_plan();
        }
        // Portada del álbum (una vez).
        if !self.item_covers.contains_key(&item.id) {
            self.fetch_item_cover(&item);
        }
    }

    fn fetch_item_cover(&mut self, item: &Item) {
        let Some(http) = self.http.clone() else { return };
        let Some(file) = item.cover_file.clone() else { return };
        let url = archive::download_url(&self.cfg.archive, &item.id, &file);
        let covers = self.cfg.dir.join("covers");
        let tmp = covers.join(format!("{}.part", scan::track_id(Path::new(&url))));
        let tx = self.tx.clone();
        let item_id = item.id.clone();
        self.item_covers.insert(item.id.clone(), String::new());
        thread::spawn(move || {
            let cancel = AtomicBool::new(false);
            if http.download(&url, &tmp, &cancel, &mut |_| {}).is_ok() {
                if let Ok(bytes) = fs::read(&tmp) {
                    if bytes.len() < 8 * 1024 * 1024 {
                        if let Some(ext) = scan::sniff_image(&bytes) {
                            if let Some(name) = scan::save_cover_bytes(&bytes, ext, &covers) {
                                let _ = tx.send(Msg::ItemCover { item: item_id, cover: name });
                            }
                        }
                    }
                }
            }
            let _ = fs::remove_file(&tmp);
        });
    }

    fn next_job_id(&mut self) -> u64 {
        let id = self.next_job;
        self.next_job += 1;
        id
    }

    fn trim_jobs(&mut self) {
        while self.jobs.len() > MAX_JOBS_SHOWN {
            let Some(oldest) = self.jobs.iter().find(|(_, j)| !matches!(j.state, "queued" | "downloading" | "processing")).map(|(id, _)| *id)
            else {
                break;
            };
            self.jobs.remove(&oldest);
        }
    }

    fn emit_jobs(&mut self) {
        self.jobs_dirty = false;
        self.last_jobs_emit = Instant::now();
        let jobs: Vec<&JobView> = self.jobs.values().collect();
        self.send(serde_json::json!({ "type": "downloads", "jobs": jobs }));
    }

    fn fill_slot(&mut self, playlist: Option<&str>, slot: usize, track: Option<String>) {
        let Some(playlist) = playlist else { return };
        let Some(slots) = self.slots.get_mut(playlist) else { return };
        if let Some(s) = slots.get_mut(slot) {
            *s = track;
        }
        // En el orden del álbum; lo que ya tenía la playlist (de una
        // descarga anterior) se conserva al final.
        let mut tracks: Vec<String> = slots.iter().flatten().cloned().collect();
        if let Some(p) = self.library.playlist_mut(playlist) {
            for t in &p.tracks {
                if !tracks.contains(t) {
                    tracks.push(t.clone());
                }
            }
            p.tracks = tracks;
            p.updated = now();
        }
        self.library_dirty = true;
    }

    fn finish_job(&mut self, id: u64, state: &'static str, message: String, track: Option<String>) {
        if let Some(job) = self.jobs.get_mut(&id) {
            job.state = state;
            job.message = message;
            job.track_id = track.clone();
            if state == "done" {
                job.received = job.size.max(job.received);
            }
        }
        if let Some(pending) = self.pending.remove(&id) {
            if let Some((playlist, slot)) = pending.playlist {
                self.fill_slot(Some(&playlist), slot, track);
            }
            if self.items.iter().any(|i| i.id == pending.item) {
                self.emit_item(&pending.item);
            }
        }
        self.jobs_dirty = true;
    }

    fn on_download(&mut self, progress: Progress) {
        match progress {
            Progress::Started(id) => {
                if let Some(j) = self.jobs.get_mut(&id) {
                    j.state = "downloading";
                }
                self.jobs_dirty = true;
            }
            Progress::Bytes { id, received } => {
                if let Some(j) = self.jobs.get_mut(&id) {
                    j.received = received;
                }
                self.jobs_dirty = true;
            }
            Progress::Failed { id, error } => self.finish_job(id, "failed", error, None),
            Progress::Cancelled(id) => self.finish_job(id, "cancelled", "Cancelada".into(), None),
            Progress::Finished { id, path } => {
                if let Some(j) = self.jobs.get_mut(&id) {
                    j.state = "processing";
                }
                let (state, message, track) = self.adopt_download(id, &path);
                self.finish_job(id, state, message, track);
                self.library_changed();
            }
        }
    }

    /// Una descarga terminó: se lee, se compara con lo que ya hay (por si
    /// era la misma canción con otro nombre o formato) y se añade.
    fn adopt_download(&mut self, id: u64, path: &Path) -> (&'static str, String, Option<String>) {
        let Some(pending) = self.pending.get(&id) else { return ("failed", "descarga desconocida".into(), None) };
        let covers = self.cfg.dir.join("covers");
        let mut track = match scan::read_track(path, &covers, now()) {
            Ok(t) => t,
            Err(err) => {
                let _ = fs::remove_file(path);
                return ("failed", format!("el archivo no se puede leer: {err}"), None);
            }
        };
        let remote = &pending.remote;
        // Etiquetas del Archive donde el archivo no traía.
        let from_file_name = track.title == scan::title_from_file_name(path);
        if from_file_name && !remote.title.is_empty() {
            track.title = remote.title.clone();
        }
        if track.artist.is_empty() {
            track.artist = remote.artist.clone();
        }
        if track.album.is_empty() {
            track.album = remote.album.clone();
        }
        if track.track_no.is_none() {
            track.track_no = remote.track_no;
        }
        if track.duration <= 0.0 {
            track.duration = remote.duration;
        }
        if track.cover.is_none() {
            track.cover = self.item_covers.get(&pending.item).filter(|c| !c.is_empty()).cloned();
        }
        track.source = Source::Archive {
            item: pending.item.clone(),
            file: remote.file.clone(),
            license: pending.license.clone(),
            creator: pending.creator.clone(),
        };

        // ¿Ya estaba? (mismo contenido o misma canción por etiquetas).
        let candidate = Candidate { hash: Some(&track.hash), archive: None, artist: &track.artist, title: &track.title, duration: track.duration };
        let found = self.library.find_existing(&candidate).filter(|(_, t)| t.id != track.id).map(|(why, t)| (why, t.id.clone()));
        if let Some((why, existing)) = found {
            let _ = fs::remove_file(path);
            self.remember_alias(id, &existing, why);
            return ("skipped", why.describe().to_owned(), Some(existing));
        }
        // ¿Mismo audio? Solo contra canciones de duración parecida.
        if let Ok(print) = fingerprint::compute(path) {
            let nearby: Vec<(String, String, PathBuf)> = self
                .library
                .tracks
                .iter()
                .filter(|t| t.id != track.id && (t.duration - track.duration).abs() <= library::SAME_AUDIO_SECONDS)
                .map(|t| (t.id.clone(), t.hash.clone(), t.path.clone()))
                .collect();
            for (other_id, hash, other_path) in nearby {
                let other = match self.fingerprints.get(&hash).and_then(|f| fingerprint::decode(f)) {
                    Some(f) => f,
                    None => match fingerprint::compute(&other_path) {
                        Ok(f) => {
                            self.fingerprints.insert(hash.clone(), fingerprint::encode(&f));
                            self.fingerprints_dirty = true;
                            f
                        }
                        Err(_) => continue,
                    },
                };
                if fingerprint::same_audio(&print, &other) {
                    let _ = fs::remove_file(path);
                    self.remember_alias(id, &other_id, Existing::SameAudio);
                    return ("skipped", Existing::SameAudio.describe().to_owned(), Some(other_id));
                }
            }
            self.fingerprints.insert(track.hash.clone(), fingerprint::encode(&print));
            self.fingerprints_dirty = true;
        }
        let track_id = track.id.clone();
        self.library.upsert(track);
        ("done", String::new(), Some(track_id))
    }

    /// Ese archivo del Archive es una canción que ya estaba: se recuerda para
    /// no volver a descargarlo.
    fn remember_alias(&mut self, job: u64, track: &str, reason: Existing) {
        let Some(pending) = self.pending.get(&job) else { return };
        let (item, file) = (pending.item.clone(), pending.remote.file.clone());
        self.library.aliases.retain(|a| !(a.item == item && a.file == file));
        self.library.aliases.push(library::Alias { item, file, track: track.to_owned(), reason: Some(reason) });
        if self.library.aliases.len() > 5000 {
            self.library.aliases.remove(0);
        }
        self.library_dirty = true;
    }

    // -- Duplicados -------------------------------------------------------------------

    fn find_duplicates(&mut self) {
        if self.dupes_running {
            return;
        }
        self.dupes_running = true;
        let library = self.library.clone();
        let known = self.fingerprints.clone();
        let tx = self.tx.clone();
        self.send(serde_json::json!({ "type": "dupes", "running": true, "done": 0, "total": 0 }));
        thread::spawn(move || {
            let (groups, computed) = duplicates_job(&library, &known, &|done, total| {
                let _ = tx.send(Msg::DupesProgress { done, total });
            });
            let _ = tx.send(Msg::DupesDone { groups, fingerprints: computed });
        });
    }
}

/// Calcula las huellas que faltan (solo de canciones que tienen otra de
/// duración parecida) y agrupa los duplicados.
pub fn duplicates_job(
    library: &Library,
    known: &HashMap<String, String>,
    progress: &(dyn Fn(usize, usize) + Sync),
) -> (Vec<DuplicateGroup>, Vec<(String, String)>) {
    let tracks = &library.tracks;
    let mut order: Vec<&Track> = tracks.iter().filter(|t| t.duration > 0.0).collect();
    order.sort_by(|a, b| a.duration.total_cmp(&b.duration));
    let mut needs: Vec<&Track> = Vec::new();
    for (i, t) in order.iter().enumerate() {
        let near = (i > 0 && t.duration - order[i - 1].duration <= library::SAME_AUDIO_SECONDS)
            || (i + 1 < order.len() && order[i + 1].duration - t.duration <= library::SAME_AUDIO_SECONDS);
        if near && !known.contains_key(&t.hash) && !needs.iter().any(|n| n.hash == t.hash) {
            needs.push(t);
        }
    }
    let total = needs.len();
    let computed = std::sync::Mutex::new(Vec::new());
    let next = std::sync::atomic::AtomicUsize::new(0);
    let done = std::sync::atomic::AtomicUsize::new(0);
    let workers = thread::available_parallelism().map_or(2, |n| n.get().saturating_sub(1)).clamp(1, 4);
    thread::scope(|scope| {
        for _ in 0..workers {
            scope.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                let Some(t) = needs.get(i) else { break };
                if let Ok(fp) = fingerprint::compute(&t.path) {
                    if let Ok(mut c) = computed.lock() {
                        c.push((t.hash.clone(), fingerprint::encode(&fp)));
                    }
                }
                let d = done.fetch_add(1, Ordering::Relaxed) + 1;
                if d.is_multiple_of(5) || d == total {
                    progress(d, total);
                }
            });
        }
    });
    let computed = computed.into_inner().unwrap_or_default();
    let mut prints: HashMap<&str, Vec<u32>> = HashMap::new();
    for (hash, fp) in known.iter().map(|(h, f)| (h.as_str(), f.as_str())).chain(computed.iter().map(|(h, f)| (h.as_str(), f.as_str()))) {
        if let Some(fp) = fingerprint::decode(fp) {
            prints.insert(hash, fp);
        }
    }
    let groups = library::find_duplicates(library, |a, b| match (prints.get(a.hash.as_str()), prints.get(b.hash.as_str())) {
        (Some(x), Some(y)) => fingerprint::same_audio(x, y),
        _ => false,
    });
    (groups, computed)
}

fn job_view(id: u64, item: &str, remote: &RemoteTrack, state: &'static str, message: &str, track: Option<String>) -> JobView {
    JobView {
        id,
        item: item.to_owned(),
        file: remote.file.clone(),
        title: remote.title.clone(),
        artist: remote.artist.clone(),
        state,
        received: 0,
        size: remote.size,
        message: message.to_owned(),
        track_id: track,
    }
}

fn new_id() -> String {
    crate::vault::random_bytes::<8>().map(|b| scan::hex(&b)).unwrap_or_else(|_| format!("{:016x}", now()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        music::scan::tests::{temp_dir, wav},
        net::testserver::{serve as http_serve, Reply, Seen},
        vault::{tests::TestProtector, Vault},
    };
    use std::sync::Mutex;

    #[test]
    fn page_messages_are_validated() {
        assert_eq!(Request::parse("ready"), Some(Request::Ready));
        assert_eq!(
            Request::parse(r#"search:{"q":"  ambient  ","page":0}"#),
            Some(Request::Search { query: "ambient".into(), page: 1, popular: false })
        );
        assert_eq!(Request::parse("item:TAM033-Ronan_Dec"), Some(Request::Item("TAM033-Ronan_Dec".into())));
        assert_eq!(
            Request::parse(r#"download:{"item":"TAM033","files":["a.mp3"],"playlist":true}"#),
            Some(Request::Download { item: "TAM033".into(), files: vec!["a.mp3".into()], playlist: true })
        );
        assert_eq!(Request::parse("cancel:all"), Some(Request::Cancel(None)));
        assert_eq!(Request::parse("cancel:7"), Some(Request::Cancel(Some(7))));
        assert_eq!(Request::parse("buy:lifetime"), Some(Request::Buy(Plan::Lifetime)));
        assert_eq!(Request::parse("quality:best"), Some(Request::SetQuality(Quality::Best)));
        assert_eq!(
            Request::parse(r#"playlist-save:{"name":" Fiesta ","tracks":["0123456789abcdef"]}"#),
            Some(Request::SavePlaylist { id: None, name: "Fiesta".into(), tracks: vec!["0123456789abcdef".into()] })
        );
        for bad in [
            "",
            "search:",
            r#"search:{"q":""}"#,
            "item:../x",
            r#"download:{"item":"x","files":[]}"#,
            r#"download:{"item":"x","files":["../../a"]}"#,
            r#"download:{"item":"a b","files":["a"]}"#,
            "cancel:x",
            "buy:gratis",
            r#"playlist-save:{"name":"","tracks":[]}"#,
            r#"playlist-save:{"name":"x","tracks":["C:\\Windows"]}"#,
            r#"remove:{"ids":["no-es-un-id"]}"#,
            "reveal:../../x",
            "license-activate:",
            "quality:max",
        ] {
            assert_eq!(Request::parse(bad), None, "{bad}");
        }
    }

    struct Harness {
        handle: Handle,
        events: Arc<Mutex<Vec<serde_json::Value>>>,
        dir: PathBuf,
    }

    impl Harness {
        fn new(name: &str, archive: &str, shop: Option<StoreConfig>, dev: bool) -> Self {
            let dir = temp_dir(name);
            let (vault, _) = Vault::open(&dir.join("vault.key"), &TestProtector(0x5a)).unwrap();
            let events = Arc::new(Mutex::new(Vec::new()));
            let sink = events.clone();
            let handle = start(
                Config {
                    dir: dir.join("music"),
                    downloads: Some(dir.join("Música").join(DOWNLOADS_FOLDER)),
                    store: Store::new(vault),
                    archive: archive.to_owned(),
                    license_api: archive.to_owned(),
                    shop,
                    dev_unlock: dev,
                },
                move |json| sink.lock().unwrap().push(serde_json::from_str(&json).unwrap()),
            );
            Self { handle, events, dir }
        }

        /// Espera un aviso que cumpla `pred` y lo devuelve.
        fn wait(&self, what: &str, pred: impl Fn(&serde_json::Value) -> bool) -> serde_json::Value {
            let deadline = Instant::now() + Duration::from_secs(30);
            loop {
                if let Some(v) = self.events.lock().unwrap().iter().rev().find(|v| pred(v)) {
                    return v.clone();
                }
                assert!(Instant::now() < deadline, "no llegó: {what}\n{:#?}", self.events.lock().unwrap().iter().map(|v| v["type"].clone()).collect::<Vec<_>>());
                thread::sleep(Duration::from_millis(30));
            }
        }

        fn clear(&self) {
            self.events.lock().unwrap().clear();
        }
    }

    fn of_type<'a>(t: &'a str) -> impl Fn(&serde_json::Value) -> bool + 'a {
        move |v| v["type"] == t
    }

    #[test]
    fn imports_rescans_and_finds_duplicates() {
        let h = Harness::new("svc-import", "http://127.0.0.1:9", None, false);
        let music = h.dir.join("Mi música");
        fs::create_dir_all(music.join("Álbum")).unwrap();
        let melody = [261.6, 329.6, 392.0, 523.3, 440.0, 349.2, 293.7, 246.9];
        fs::write(music.join("Álbum/01 - Uno.wav"), wav(12.0, &melody, 22050)).unwrap();
        // Copia exacta en otra carpeta y la misma melodía a otra calidad.
        fs::write(music.join("copia.wav"), wav(12.0, &melody, 22050)).unwrap();
        fs::write(music.join("Uno (otra calidad).wav"), wav(12.0, &melody, 44100)).unwrap();
        fs::write(music.join("distinta.wav"), wav(12.0, &[196.0, 155.6, 370.0, 207.7], 22050)).unwrap();
        fs::write(music.join("notas.txt"), "x").unwrap();

        h.handle.request(Request::Ready);
        h.wait("plan inicial", |v| v["type"] == "plan" && v["plan"] == "free" && v["configured"] == false);
        h.handle.request(Request::Import(music.clone()));
        let done = h.wait("fin de importación", |v| v["type"] == "scan" && v["running"] == false);
        assert_eq!(done["added"], 4, "{done}");
        let lib = h.wait("biblioteca", |v| v["type"] == "library" && v["tracks"].as_array().is_some_and(|t| t.len() == 4));
        assert_eq!(lib["folders"][0], music.to_string_lossy().as_ref());
        assert!(lib["tracks"].as_array().unwrap().iter().any(|t| t["title"] == "Uno"));

        h.clear();
        h.handle.request(Request::FindDuplicates);
        let dupes = h.wait("duplicados", |v| v["type"] == "dupes" && v["running"] == false);
        let groups = dupes["groups"].as_array().unwrap();
        assert_eq!(groups.len(), 1, "{dupes}");
        assert_eq!(groups[0]["tracks"].as_array().unwrap().len(), 3, "copia exacta + misma grabación");
        assert_eq!(groups[0]["reason"], "same-file");

        // Borrar un archivo y volver a revisar: desaparece.
        fs::remove_file(music.join("distinta.wav")).unwrap();
        h.clear();
        h.handle.request(Request::Rescan);
        let done = h.wait("re-escaneo", |v| v["type"] == "scan" && v["running"] == false);
        assert_eq!((done["added"].as_u64(), done["removed"].as_u64()), (Some(0), Some(1)));

        // Plan Gratis: 2 playlists; la tercera se explica (sin ventanas) y
        // no se crea. Editar las que hay sigue funcionando.
        let first_track = lib["tracks"][0]["id"].as_str().unwrap().to_owned();
        for name in ["Una", "Dos"] {
            h.handle.request(Request::SavePlaylist { id: None, name: name.into(), tracks: vec![first_track.clone()] });
        }
        let lib = h.wait("dos playlists", |v| v["type"] == "library" && v["playlists"].as_array().is_some_and(|p| p.len() == 2));
        let plan = h.wait("contador", |v| v["type"] == "plan" && v["playlists"] == 2);
        assert_eq!(plan["entitlements"]["maxPlaylists"], 2);
        h.clear();
        h.handle.request(Request::SavePlaylist { id: None, name: "Tres".into(), tracks: vec![] });
        let notice = h.wait("límite", of_type("notice"));
        assert_eq!(notice["feature"], "playlists");
        assert!(notice["text"].as_str().unwrap().contains("máximo del plan Gratis"), "{notice}");
        let existing = lib["playlists"][0]["id"].as_str().unwrap().to_owned();
        h.handle.request(Request::SavePlaylist { id: Some(existing), name: "Una (renombrada)".into(), tracks: vec![] });
        h.wait("editada", |v| {
            v["type"] == "library" && v["playlists"].as_array().is_some_and(|p| p.len() == 2 && p.iter().any(|p| p["name"] == "Una (renombrada)"))
        });

        // Lo guardado se recupera al reabrir.
        h.handle.shutdown(Duration::from_secs(5));
        let (vault, _) = Vault::open(&h.dir.join("vault.key"), &TestProtector(0x5a)).unwrap();
        let store = Store::new(vault);
        let saved: Library = store.load(&h.dir.join("music/library.enc"), LIBRARY).unwrap();
        assert_eq!(saved.tracks.len(), 3);
        assert!(!fs::read(h.dir.join("music/library.enc")).unwrap().windows(4).any(|w| w == b"Uno\""), "cifrada");
        let _ = fs::remove_dir_all(&h.dir);
    }

    fn album_json() -> String {
        let mut item = crate::music::archive::tests::sample_item();
        // Tamaños y sumas reales de lo que sirve el servidor de pruebas.
        let a = wav(8.0, &[261.6, 329.6, 392.0, 523.3], 22050);
        let b = wav(8.0, &[196.0, 155.6, 370.0, 207.7], 22050);
        let sha = |d: &[u8]| {
            use sha1::{Digest, Sha1};
            scan::hex(&Sha1::digest(d))
        };
        let files = item["files"].as_array_mut().unwrap();
        files.retain(|f| f["name"].as_str().is_some_and(|n| !n.contains("..")));
        for f in files.iter_mut() {
            let name = f["name"].as_str().unwrap().to_owned();
            if name == "TAM033-Ronan_Dec-1-Supernova.mp3" {
                f["size"] = a.len().to_string().into();
                f["sha1"] = sha(&a).into();
                f["length"] = "8.0".into();
            } else if name == "TAM033-Ronan_Dec-2-Waehrend_du_schliefst.mp3" {
                f["size"] = b.len().to_string().into();
                f["sha1"] = sha(&b).into();
                f["length"] = "8.0".into();
            } else if name == "TAM033-Ronan_Dec-10-Final.mp3" {
                // Mismo audio que la 1 (otro "archivo"): se descarga y se descarta.
                let c = wav(8.0, &[261.6, 329.6, 392.0, 523.3], 44100);
                f["size"] = c.len().to_string().into();
                f["sha1"] = sha(&c).into();
                f["length"] = "8.0".into();
            }
        }
        item.to_string()
    }

    #[test]
    fn downloads_an_album_as_playlist_skipping_what_it_has() {
        let a = wav(8.0, &[261.6, 329.6, 392.0, 523.3], 22050);
        let b = wav(8.0, &[196.0, 155.6, 370.0, 207.7], 22050);
        let c = wav(8.0, &[261.6, 329.6, 392.0, 523.3], 44100);
        let metadata = album_json();
        let (base, log) = http_serve(Arc::new(move |s: &Seen| {
            let path = s.path.as_str();
            if path.starts_with("/advancedsearch.php") {
                Reply::json(200, r#"{"response":{"numFound":1,"docs":[{"identifier":"TAM033-Ronan_Dec","title":"Hirnwald EP","creator":"Ronan Dec.","licenseurl":"http://creativecommons.org/licenses/by-sa/2.0/de/","downloads":5}]}}"#)
            } else if path == "/metadata/TAM033-Ronan_Dec" {
                Reply::json(200, &metadata)
            } else if path.ends_with("1-Supernova.mp3") {
                Reply { status: 200, headers: vec![], body: a.clone() }
            } else if path.ends_with("2-Waehrend_du_schliefst.mp3") {
                Reply { status: 200, headers: vec![], body: b.clone() }
            } else if path.ends_with("10-Final.mp3") {
                Reply { status: 200, headers: vec![], body: c.clone() }
            } else if path.ends_with("Cover.jpg") {
                Reply { status: 200, headers: vec![], body: vec![0xFF, 0xD8, 0xFF, 0xE0, 1, 2, 3] }
            } else {
                Reply::json(404, "{}")
            }
        }));
        // Sin plan: no se puede descargar.
        let free = Harness::new("svc-free", &base, None, false);
        free.handle.request(Request::Item("TAM033-Ronan_Dec".into()));
        free.wait("álbum", |v| v["type"] == "item" && v["item"]["tracks"].as_array().is_some());
        free.handle.request(Request::Download { item: "TAM033-Ronan_Dec".into(), files: vec!["TAM033-Ronan_Dec-1-Supernova.mp3".into()], playlist: false });
        assert_eq!(free.wait("límite", of_type("notice"))["feature"], "download");
        free.handle.shutdown(Duration::from_secs(2));

        let h = Harness::new("svc-download", &base, None, true);
        h.handle.request(Request::Ready);
        h.wait("plan dev", |v| v["type"] == "plan" && v["plan"] == "lifetime" && v["dev"] == true);
        h.handle.request(Request::Search { query: "ronan".into(), page: 1, popular: true });
        let found = h.wait("búsqueda", of_type("search"));
        assert_eq!(found["hits"][0]["id"], "TAM033-Ronan_Dec");
        h.handle.request(Request::Item("TAM033-Ronan_Dec".into()));
        let item = h.wait("álbum", |v| v["type"] == "item" && v["item"]["tracks"].as_array().is_some());
        let files: Vec<String> = item["item"]["tracks"].as_array().unwrap().iter().map(|t| t["file"].as_str().unwrap().to_owned()).collect();
        assert_eq!(files.len(), 3);

        h.clear();
        h.handle.request(Request::Download { item: "TAM033-Ronan_Dec".into(), files: files.clone(), playlist: true });
        let finished = |v: &serde_json::Value| {
            v["type"] == "downloads"
                && v["jobs"].as_array().is_some_and(|j| j.len() == 3 && j.iter().all(|j| !matches!(j["state"].as_str(), Some("queued" | "downloading" | "processing"))))
        };
        let jobs = h.wait("descargas terminadas", finished);
        let states: Vec<&str> = jobs["jobs"].as_array().unwrap().iter().map(|j| j["state"].as_str().unwrap()).collect();
        assert_eq!(states.iter().filter(|s| **s == "done").count(), 2, "{jobs}");
        let skipped = jobs["jobs"].as_array().unwrap().iter().find(|j| j["state"] == "skipped").unwrap();
        assert_eq!(skipped["message"], "ya la tienes (mismo audio)", "{skipped}");
        let lib = h.wait("biblioteca con el álbum", |v| {
            v["type"] == "library" && v["playlists"].as_array().is_some_and(|p| p.len() == 1 && p[0]["tracks"].as_array().is_some_and(|t| t.len() == 3))
        });
        assert_eq!(lib["tracks"].as_array().unwrap().len(), 2, "el duplicado no ocupa espacio");
        let playlist = &lib["playlists"][0];
        assert_eq!(playlist["origin"], "TAM033-Ronan_Dec");
        // La playlist tiene las 3 posiciones: la repetida apunta a la que ya había.
        let ids: Vec<&str> = playlist["tracks"].as_array().unwrap().iter().map(|t| t.as_str().unwrap()).collect();
        assert_eq!(ids[0], ids[2], "{ids:?}");
        let saved_dir = h.dir.join("Música").join(DOWNLOADS_FOLDER).join("Ronan Dec").join("Ronan Dec. - Hirnwald EP [TAM033]");
        let mut on_disk: Vec<String> = fs::read_dir(&saved_dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        on_disk.sort();
        assert_eq!(on_disk.len(), 2, "{on_disk:?}");
        let track = lib["tracks"].as_array().unwrap().iter().find(|t| t["license"] == "CC BY-SA").unwrap();
        assert_eq!(track["source"], "archive");

        // Volver a pedir el álbum: todo se salta, no se descarga nada.
        h.clear();
        h.handle.request(Request::Download { item: "TAM033-Ronan_Dec".into(), files, playlist: true });
        let toast = h.wait("aviso", of_type("toast"));
        assert_eq!(toast["text"], "Ya tienes todo lo de este álbum: no se descarga nada.");
        let downloads = log.lock().unwrap().iter().filter(|s| s.path.starts_with("/download/") && s.path.ends_with(".mp3")).count();
        assert_eq!(downloads, 3, "cada canción se pidió una sola vez");
        h.handle.shutdown(Duration::from_secs(5));
        let _ = fs::remove_dir_all(&h.dir);
    }
}
