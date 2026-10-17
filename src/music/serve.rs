//! Servir canciones y portadas a la página de música por el protocolo
//! propio (`https://ytmp.localhost/media/<id>` y `/cover/<nombre>`).
//!
//! - Solo canciones de la biblioteca (por id; nunca una ruta que llegue en
//!   la URL) y portadas de la caché.
//! - Exige la clave de esta sesión (`?k=`), que solo conoce nuestra página:
//!   otra web abierta en el reproductor no puede pedir tus archivos.
//! - Rechaza peticiones de otros sitios (`Sec-Fetch-Site: cross-site`).
//! - Admite `Range` (206) por trozos, para que el audio empiece enseguida y
//!   se pueda saltar a cualquier punto sin leer el archivo entero.

use std::{
    borrow::Cow,
    collections::HashMap,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
    sync::{OnceLock, RwLock},
};

use http::{header, HeaderMap, Response, StatusCode};

/// Trozo máximo por respuesta.
const CHUNK: u64 = 1024 * 1024;
const MAX_COVER: u64 = 8 * 1024 * 1024;

pub struct Shared {
    key: String,
    covers: PathBuf,
    tracks: RwLock<HashMap<String, (PathBuf, String)>>,
}

static SHARED: OnceLock<Shared> = OnceLock::new();

/// Se llama una vez al arrancar el servicio de música.
pub fn init(key: String, covers: PathBuf) -> &'static Shared {
    SHARED.get_or_init(|| Shared { key, covers, tracks: RwLock::new(HashMap::new()) })
}

pub fn shared() -> Option<&'static Shared> {
    SHARED.get()
}

impl Shared {
    pub fn key(&self) -> &str {
        &self.key
    }

    /// Reemplaza la tabla id → (ruta, formato).
    pub fn set_tracks(&self, tracks: HashMap<String, (PathBuf, String)>) {
        if let Ok(mut map) = self.tracks.write() {
            *map = tracks;
        }
    }

    fn lookup(&self, id: &str) -> Option<(PathBuf, String)> {
        self.tracks.read().ok()?.get(id).cloned()
    }
}

fn constant_time_eq(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub fn mime(format: &str) -> &'static str {
    match format {
        "mp3" => "audio/mpeg",
        "flac" => "audio/flac",
        "m4a" | "aac" => "audio/mp4",
        "ogg" | "oga" => "audio/ogg",
        "opus" => "audio/ogg; codecs=opus",
        "wav" => "audio/wav",
        "jpg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

/// `bytes=inicio-[fin]` → (inicio, fin incluido), acotado al tamaño.
pub fn parse_range(value: &str, size: u64) -> Option<(u64, u64)> {
    let spec = value.trim().strip_prefix("bytes=")?;
    if spec.contains(',') || size == 0 {
        return None;
    }
    let (start, end) = spec.split_once('-')?;
    let (start, end) = if start.is_empty() {
        // Sufijo: los últimos N bytes.
        let n: u64 = end.parse().ok()?;
        (size.saturating_sub(n), size - 1)
    } else {
        let start: u64 = start.parse().ok()?;
        let end = if end.is_empty() { size - 1 } else { end.parse::<u64>().ok()?.min(size - 1) };
        (start, end)
    };
    (start <= end && start < size).then_some((start, end))
}

type Reply = Response<Cow<'static, [u8]>>;

fn status(code: StatusCode) -> Reply {
    Response::builder()
        .status(code)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, "no-store")
        .body(Cow::Borrowed(&b""[..]))
        .expect("respuesta válida")
}

fn query_key(query: Option<&str>) -> Option<String> {
    url::form_urlencoded::parse(query?.as_bytes()).find(|(k, _)| k == "k").map(|(_, v)| v.into_owned())
}

/// ¿Es una ruta de música? Si lo es, la respuesta.
pub fn respond(path: &str, query: Option<&str>, headers: &HeaderMap) -> Option<Reply> {
    let (kind, name) = match path.strip_prefix("/media/") {
        Some(id) => ("media", id),
        None => ("cover", path.strip_prefix("/cover/")?),
    };
    let Some(shared) = shared() else { return Some(status(StatusCode::NOT_FOUND)) };
    let authorized = query_key(query).is_some_and(|k| constant_time_eq(&k, &shared.key));
    let cross_site = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()).is_some_and(|v| v == "cross-site");
    if !authorized || cross_site {
        return Some(status(StatusCode::FORBIDDEN));
    }
    Some(match kind {
        "media" => media(shared, name, headers),
        _ => cover(shared, name),
    })
}

fn media(shared: &Shared, id: &str, headers: &HeaderMap) -> Reply {
    if id.len() != 16 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return status(StatusCode::NOT_FOUND);
    }
    let Some((path, format)) = shared.lookup(id) else { return status(StatusCode::NOT_FOUND) };
    let Ok(mut file) = File::open(&path) else { return status(StatusCode::NOT_FOUND) };
    let size = file.metadata().map(|m| m.len()).unwrap_or(0);
    let range = headers.get(header::RANGE).and_then(|v| v.to_str().ok());
    let (start, end, partial) = match range {
        Some(r) => match parse_range(r, size) {
            Some((s, e)) => (s, e.min(s + CHUNK - 1), true),
            None => {
                return Response::builder()
                    .status(StatusCode::RANGE_NOT_SATISFIABLE)
                    .header(header::CONTENT_RANGE, format!("bytes */{size}"))
                    .body(Cow::Borrowed(&b""[..]))
                    .expect("respuesta válida");
            }
        },
        // Sin Range: el primer trozo como 206 (el motor pide el resto).
        None if size > CHUNK => (0, CHUNK - 1, true),
        None => (0, size.saturating_sub(1), false),
    };
    let len = if size == 0 { 0 } else { end - start + 1 };
    let mut body = vec![0u8; len as usize];
    if file.seek(SeekFrom::Start(start)).is_err() || file.read_exact(&mut body).is_err() {
        return status(StatusCode::INTERNAL_SERVER_ERROR);
    }
    let mut builder = Response::builder()
        .status(if partial { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK })
        .header(header::CONTENT_TYPE, mime(&format))
        .header(header::ACCEPT_RANGES, "bytes")
        .header(header::CONTENT_LENGTH, len.to_string())
        .header(header::CACHE_CONTROL, "no-store")
        .header("X-Content-Type-Options", "nosniff");
    if partial {
        builder = builder.header(header::CONTENT_RANGE, format!("bytes {start}-{end}/{size}"));
    }
    builder.body(Cow::Owned(body)).expect("respuesta válida")
}

fn cover(shared: &Shared, name: &str) -> Reply {
    if !super::scan::is_cover_name(name) {
        return status(StatusCode::NOT_FOUND);
    }
    let path = shared.covers.join(name);
    let Ok(meta) = std::fs::metadata(&path) else { return status(StatusCode::NOT_FOUND) };
    if meta.len() > MAX_COVER {
        return status(StatusCode::NOT_FOUND);
    }
    let Ok(bytes) = std::fs::read(&path) else { return status(StatusCode::NOT_FOUND) };
    let ext = name.rsplit('.').next().unwrap_or_default();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime(ext))
        .header(header::CACHE_CONTROL, "private, max-age=86400")
        .header("X-Content-Type-Options", "nosniff")
        .body(Cow::Owned(bytes))
        .expect("respuesta válida")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::music::scan::tests::temp_dir;

    #[test]
    fn ranges() {
        assert_eq!(parse_range("bytes=0-", 100), Some((0, 99)));
        assert_eq!(parse_range("bytes=10-19", 100), Some((10, 19)));
        assert_eq!(parse_range("bytes=90-500", 100), Some((90, 99)));
        assert_eq!(parse_range("bytes=-10", 100), Some((90, 99)));
        for bad in ["bytes=100-", "bytes=5-2", "items=0-1", "bytes=0-1,5-6", "bytes=x-"] {
            assert_eq!(parse_range(bad, 100), None, "{bad}");
        }
        assert_eq!(parse_range("bytes=0-", 0), None);
    }

    #[test]
    fn serves_only_known_tracks_with_the_session_key() {
        let dir = temp_dir("serve");
        let song = dir.join("canción.mp3");
        let content: Vec<u8> = (0..(CHUNK as usize * 2 + 500)).map(|i| (i % 253) as u8).collect();
        std::fs::write(&song, &content).unwrap();
        let covers = dir.join("covers");
        std::fs::create_dir_all(&covers).unwrap();
        std::fs::write(covers.join("0123456789abcdef0123.jpg"), [0xFF, 0xD8, 0xFF, 0xE0]).unwrap();
        let shared = init("clave-de-sesion".into(), covers);
        let id = "00112233aabbccdd";
        shared.set_tracks(HashMap::from([(id.to_owned(), (song.clone(), "mp3".to_owned()))]));
        let key = Some("k=clave-de-sesion");
        let none = HeaderMap::new();

        assert!(respond("/", key, &none).is_none(), "no es una ruta de música");
        assert_eq!(respond(&format!("/media/{id}"), None, &none).unwrap().status(), StatusCode::FORBIDDEN);
        assert_eq!(respond(&format!("/media/{id}"), Some("k=otra"), &none).unwrap().status(), StatusCode::FORBIDDEN);
        let mut cross = HeaderMap::new();
        cross.insert("sec-fetch-site", "cross-site".parse().unwrap());
        assert_eq!(respond(&format!("/media/{id}"), key, &cross).unwrap().status(), StatusCode::FORBIDDEN);
        for bad in ["/media/../../etc/passwd", "/media/C:%5CWindows", "/media/ffffffffffffffff", "/cover/../vault.key"] {
            assert_eq!(respond(bad, key, &none).unwrap().status(), StatusCode::NOT_FOUND, "{bad}");
        }

        // Sin Range: primer trozo como 206.
        let first = respond(&format!("/media/{id}"), key, &none).unwrap();
        assert_eq!(first.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(first.headers()["content-range"], format!("bytes 0-{}/{}", CHUNK - 1, content.len()));
        assert_eq!(first.headers()["content-type"], "audio/mpeg");
        assert_eq!(first.body().as_ref(), &content[..CHUNK as usize]);
        // Range hacia el final.
        let mut range = HeaderMap::new();
        range.insert("range", format!("bytes={}-", CHUNK * 2).parse().unwrap());
        let tail = respond(&format!("/media/{id}"), key, &range).unwrap();
        assert_eq!(tail.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(tail.body().as_ref(), &content[(CHUNK * 2) as usize..]);
        assert_eq!(tail.headers()["accept-ranges"], "bytes");
        range.insert("range", "bytes=999999999-".parse().unwrap());
        assert_eq!(respond(&format!("/media/{id}"), key, &range).unwrap().status(), StatusCode::RANGE_NOT_SATISFIABLE);

        let cover = respond("/cover/0123456789abcdef0123.jpg", key, &none).unwrap();
        assert_eq!((cover.status(), cover.headers()["content-type"].to_str().unwrap()), (StatusCode::OK, "image/jpeg"));
        assert_eq!(respond("/cover/0123456789abcdef9999.jpg", key, &none).unwrap().status(), StatusCode::NOT_FOUND);
        let _ = std::fs::remove_dir_all(dir);
    }
}
