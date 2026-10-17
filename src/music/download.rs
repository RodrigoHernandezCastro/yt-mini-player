//! Cola de descargas: varias a la vez (pocas, como pide el Archive), cada
//! una a un `.part` que se reanuda, con verificación SHA-1 al terminar.

use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

use crate::net::{Http, NetError};

/// Descargas simultáneas.
pub const WORKERS: usize = 3;
/// Separación mínima entre dos arranques (no golpear al servidor).
const STAGGER: Duration = Duration::from_millis(400);
/// Cada cuánto se informa el progreso de una descarga.
const PROGRESS_EVERY: Duration = Duration::from_millis(300);

#[derive(Debug, Clone, PartialEq)]
pub struct Job {
    pub id: u64,
    pub url: String,
    /// Ruta final deseada (si existe, se usa "Nombre (2).ext").
    pub dest: PathBuf,
    pub size: u64,
    /// SHA-1 esperado (hex, minúsculas) o vacío.
    pub sha1: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Progress {
    Started(u64),
    Bytes { id: u64, received: u64 },
    Finished { id: u64, path: PathBuf },
    Failed { id: u64, error: String },
    Cancelled(u64),
}

#[derive(Default)]
struct Queue {
    waiting: VecDeque<Job>,
    running: Vec<(u64, Arc<AtomicBool>)>,
    last_start: Option<Instant>,
    shutdown: bool,
}

#[derive(Clone)]
pub struct Downloader {
    shared: Arc<(Mutex<Queue>, Condvar)>,
}

impl Downloader {
    pub fn start(http: Arc<Http>, workers: usize, report: impl Fn(Progress) + Send + Sync + 'static) -> Self {
        let shared = Arc::new((Mutex::new(Queue::default()), Condvar::new()));
        let report = Arc::new(report);
        for n in 0..workers.max(1) {
            let shared = shared.clone();
            let http = http.clone();
            let report = report.clone();
            let _ = thread::Builder::new().name(format!("descarga-{n}")).spawn(move || worker(&shared, &http, &*report));
        }
        Self { shared }
    }

    pub fn push(&self, job: Job) {
        let (lock, wake) = &*self.shared;
        lock.lock().unwrap_or_else(|e| e.into_inner()).waiting.push_back(job);
        wake.notify_one();
    }

    /// Cancela una (o todas con `None`). Devuelve las que estaban en espera
    /// (las que ya corrían avisan solas con `Cancelled`).
    pub fn cancel(&self, id: Option<u64>) -> Vec<u64> {
        let (lock, _) = &*self.shared;
        let mut q = lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut removed = Vec::new();
        q.waiting.retain(|j| {
            let hit = id.is_none_or(|id| id == j.id);
            if hit {
                removed.push(j.id);
            }
            !hit
        });
        for (running, flag) in &q.running {
            if id.is_none_or(|id| id == *running) {
                flag.store(true, Ordering::Relaxed);
            }
        }
        removed
    }

    pub fn shutdown(&self) {
        let (lock, wake) = &*self.shared;
        let mut q = lock.lock().unwrap_or_else(|e| e.into_inner());
        q.shutdown = true;
        for (_, flag) in &q.running {
            flag.store(true, Ordering::Relaxed);
        }
        wake.notify_all();
    }
}

fn worker(shared: &(Mutex<Queue>, Condvar), http: &Http, report: &dyn Fn(Progress)) {
    let (lock, wake) = shared;
    loop {
        let (job, cancel) = {
            let mut q = lock.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if q.shutdown {
                    return;
                }
                let ready = q.last_start.is_none_or(|t| t.elapsed() >= STAGGER);
                if ready && !q.waiting.is_empty() {
                    break;
                }
                q = if q.waiting.is_empty() {
                    wake.wait(q).unwrap_or_else(|e| e.into_inner())
                } else {
                    let left = STAGGER.saturating_sub(q.last_start.map_or(Duration::ZERO, |t| t.elapsed()));
                    wake.wait_timeout(q, left).unwrap_or_else(|e| e.into_inner()).0
                };
            }
            let job = q.waiting.pop_front().expect("hay trabajo");
            let cancel = Arc::new(AtomicBool::new(false));
            q.running.push((job.id, cancel.clone()));
            q.last_start = Some(Instant::now());
            (job, cancel)
        };
        report(Progress::Started(job.id));
        let result = run(http, &job, &cancel, report);
        {
            let mut q = lock.lock().unwrap_or_else(|e| e.into_inner());
            q.running.retain(|(id, _)| *id != job.id);
        }
        report(match result {
            Ok(path) => Progress::Finished { id: job.id, path },
            Err(NetError::Cancelled) => Progress::Cancelled(job.id),
            Err(err) => Progress::Failed { id: job.id, error: err.to_string() },
        });
    }
}

/// `Nombre.ext` → `Nombre (2).ext` si ya existe.
pub fn free_name(dest: &Path) -> PathBuf {
    if !dest.exists() {
        return dest.to_path_buf();
    }
    let stem = dest.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let ext = dest.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..1000)
        .map(|n| dest.with_file_name(format!("{stem} ({n}){ext}")))
        .find(|p| !p.exists())
        .unwrap_or_else(|| dest.to_path_buf())
}

pub fn part_path(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().map(|n| n.to_os_string()).unwrap_or_default();
    name.push(".part");
    dest.with_file_name(name)
}

fn run(http: &Http, job: &Job, cancel: &AtomicBool, report: &dyn Fn(Progress)) -> Result<PathBuf, NetError> {
    let dir = job.dest.parent().ok_or_else(|| NetError::Io("ruta sin carpeta".into()))?;
    fs::create_dir_all(dir).map_err(|e| NetError::Io(e.to_string()))?;
    let part = part_path(&job.dest);
    // `Instant` no se puede restar sin riesgo (en Windows cuenta desde el
    // arranque del equipo): "nunca" se representa con None.
    let mut last: Option<Instant> = None;
    let size = http.download(&job.url, &part, cancel, &mut |received| {
        if last.is_none_or(|t| t.elapsed() >= PROGRESS_EVERY) {
            last = Some(Instant::now());
            report(Progress::Bytes { id: job.id, received });
        }
    });
    let size = match size {
        Ok(size) => size,
        Err(NetError::Cancelled) => {
            let _ = fs::remove_file(&part);
            return Err(NetError::Cancelled);
        }
        Err(err) => return Err(err),
    };
    if job.size > 0 && size != job.size {
        let _ = fs::remove_file(&part);
        return Err(NetError::Invalid(format!("tamaño inesperado ({size} en vez de {})", job.size)));
    }
    if !job.sha1.is_empty() {
        let actual = super::scan::sha1_file(&part).map_err(|e| NetError::Io(e.to_string()))?;
        if !actual.eq_ignore_ascii_case(&job.sha1) {
            let _ = fs::remove_file(&part);
            return Err(NetError::Invalid("el archivo llegó dañado (la suma SHA-1 no coincide)".into()));
        }
    }
    let path = free_name(&job.dest);
    fs::rename(&part, &path).map_err(|e| NetError::Io(e.to_string()))?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        music::scan::{hex, tests::temp_dir},
        net::testserver::{serve, Reply, Seen},
    };
    use sha1::{Digest, Sha1};
    use std::sync::mpsc;

    #[test]
    fn names() {
        let dir = temp_dir("names");
        let dest = dir.join("01 - Tema.mp3");
        assert_eq!(free_name(&dest), dest);
        fs::write(&dest, b"x").unwrap();
        assert_eq!(free_name(&dest), dir.join("01 - Tema (2).mp3"));
        assert_eq!(part_path(&dest), dir.join("01 - Tema.mp3.part"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn downloads_in_parallel_verifies_and_cancels() {
        let good: Vec<u8> = (0..300_000u32).map(|i| (i * 7 % 256) as u8).collect();
        let sha = hex(&Sha1::digest(&good));
        let body = good.clone();
        let (base, log) = serve(Arc::new(move |s: &Seen| match s.path.as_str() {
            "/slow.mp3" => {
                thread::sleep(Duration::from_millis(1500));
                Reply { status: 200, headers: vec![], body: body.clone() }
            }
            "/missing.mp3" => Reply { status: 404, headers: vec![], body: vec![] },
            _ => Reply { status: 200, headers: vec![], body: body.clone() },
        }));
        let dir = temp_dir("dl");
        let (tx, rx) = mpsc::channel();
        let tx = Mutex::new(tx);
        let dl = Downloader::start(Arc::new(Http::new().unwrap()), 3, move |p| {
            let _ = tx.lock().unwrap().send(p);
        });
        let job = |id: u64, path: &str, name: &str, sha1: &str| Job {
            id,
            url: format!("{base}{path}"),
            dest: dir.join("Artista").join("Álbum").join(name),
            size: 300_000,
            sha1: sha1.into(),
        };
        dl.push(job(1, "/a.mp3", "01 - Uno.mp3", &sha));
        dl.push(job(2, "/b.mp3", "02 - Dos.mp3", "0000000000000000000000000000000000000000"));
        dl.push(job(3, "/missing.mp3", "03 - Tres.mp3", ""));
        dl.push(job(4, "/slow.mp3", "04 - Cuatro.mp3", &sha));
        dl.push(job(5, "/a.mp3", "05 - Cinco.mp3", ""));
        thread::sleep(Duration::from_millis(100));
        assert_eq!(dl.cancel(Some(5)), vec![5], "la que espera se quita de la cola");

        let mut done = std::collections::BTreeMap::new();
        let deadline = Instant::now() + Duration::from_secs(20);
        let mut cancelled_slow = false;
        while done.len() < 4 && Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_millis(200)) {
                Ok(Progress::Started(4)) if !cancelled_slow => {
                    cancelled_slow = true;
                    dl.cancel(Some(4));
                }
                Ok(p @ (Progress::Finished { .. } | Progress::Failed { .. } | Progress::Cancelled(_))) => {
                    let id = match &p {
                        Progress::Finished { id, .. } | Progress::Failed { id, .. } => *id,
                        Progress::Cancelled(id) => *id,
                        _ => unreachable!(),
                    };
                    done.insert(id, p);
                }
                _ => {}
            }
        }
        let path = match &done[&1] {
            Progress::Finished { path, .. } => path.clone(),
            other => panic!("{other:?}"),
        };
        assert_eq!(fs::read(&path).unwrap(), good);
        assert!(matches!(&done[&2], Progress::Failed { error, .. } if error.contains("SHA-1")), "{:?}", done[&2]);
        assert!(!part_path(&dir.join("Artista/Álbum/02 - Dos.mp3")).exists(), "el dañado no queda");
        assert!(matches!(&done[&3], Progress::Failed { error, .. } if error.contains("404")));
        assert!(matches!(&done[&4], Progress::Cancelled(4)), "{:?}", done.get(&4));
        assert!(!done.contains_key(&5));
        assert!(log.lock().unwrap().iter().all(|s| s.headers.iter().any(|(k, v)| k == "user-agent" && v.contains("YTMiniPlayer"))));
        dl.shutdown();
        let _ = fs::remove_dir_all(dir);
    }
}
