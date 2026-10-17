//! Leer archivos de audio: etiquetas, duración, portada y huella del
//! contenido.

use std::{
    fs::{self, File},
    io::{self, Read},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::UNIX_EPOCH,
};

use lofty::{picture::MimeType, prelude::*};
use sha1::{Digest, Sha1};
use walkdir::WalkDir;

use super::library::{Source, Track};

/// Formatos que el motor (WebView2) reproduce.
pub const AUDIO_EXTENSIONS: &[&str] = &["mp3", "flac", "m4a", "aac", "ogg", "oga", "opus", "wav"];
/// Más allá de esto no es una canción (y no se lee entera para el hash).
const MAX_AUDIO: u64 = 2 * 1024 * 1024 * 1024;
const MAX_COVER: usize = 8 * 1024 * 1024;
const MAX_DEPTH: usize = 16;

pub fn extension(path: &Path) -> Option<String> {
    let ext = path.extension()?.to_str()?.to_ascii_lowercase();
    AUDIO_EXTENSIONS.contains(&ext.as_str()).then_some(ext)
}

/// Id estable de una ruta (no depende de mayúsculas, como Windows).
pub fn track_id(path: &Path) -> String {
    let normalized = path.to_string_lossy().replace('/', "\\").to_lowercase();
    let digest = Sha1::digest(normalized.as_bytes());
    hex(&digest[..8])
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Archivos de audio bajo `root` (sin seguir enlaces ni entrar en carpetas
/// ocultas como `$RECYCLE.BIN`).
pub fn audio_files(root: &Path, cancel: &AtomicBool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let walker = WalkDir::new(root).follow_links(false).max_depth(MAX_DEPTH).into_iter().filter_entry(|e| {
        e.depth() == 0 || !e.file_name().to_string_lossy().starts_with(['.', '$'])
    });
    for entry in walker.flatten() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if entry.file_type().is_file() && extension(entry.path()).is_some() {
            out.push(entry.into_path());
        }
    }
    out
}

/// Trozos que se leen para la huella rápida del contenido.
const HASH_CHUNK: usize = 64 * 1024;

/// Huella del contenido: SHA-1 del tamaño y de tres trozos (principio,
/// medio y final). Leer canciones enteras para importar miles de ellas
/// sería lentísimo; dos archivos de audio distintos no coinciden en el
/// tamaño exacto *y* en esos 192 KB. Los archivos pequeños se leen enteros.
pub fn content_hash(path: &Path) -> io::Result<String> {
    use std::io::{Seek, SeekFrom};
    let mut file = File::open(path)?;
    let size = file.metadata()?.len();
    let mut hasher = Sha1::new();
    hasher.update(size.to_le_bytes());
    let mut buffer = vec![0u8; HASH_CHUNK];
    if size <= (HASH_CHUNK as u64) * 3 {
        loop {
            let n = file.read(&mut buffer)?;
            if n == 0 {
                break;
            }
            hasher.update(&buffer[..n]);
        }
    } else {
        for offset in [0, size / 2 - HASH_CHUNK as u64 / 2, size - HASH_CHUNK as u64] {
            file.seek(SeekFrom::Start(offset))?;
            file.read_exact(&mut buffer)?;
            hasher.update(&buffer);
        }
    }
    Ok(hex(&hasher.finalize()[..12]))
}

/// SHA-1 completo (para verificar descargas contra el Archive).
pub fn sha1_file(path: &Path) -> io::Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha1::new();
    let mut buffer = vec![0u8; 256 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
    }
    Ok(hex(&hasher.finalize()))
}

/// Tamaño y fecha de modificación (para saber si hay que volver a leerlo).
pub fn stamp(path: &Path) -> io::Result<(u64, u64)> {
    let meta = fs::metadata(path)?;
    let modified = meta.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map_or(0, |d| d.as_secs());
    Ok((meta.len(), modified))
}

/// Título a partir del nombre del archivo: "03 - Mi tema.mp3" → "Mi tema".
pub fn title_from_file_name(path: &Path) -> String {
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let stem = stem.replace('_', " ");
    let trimmed = stem.trim_start_matches(|c: char| c.is_ascii_digit() || c == ' ' || c == '.' || c == '-');
    let title = if trimmed.trim().is_empty() { stem.trim() } else { trimmed.trim() };
    title.to_owned()
}

fn clean(text: Option<std::borrow::Cow<'_, str>>, max: usize) -> String {
    text.map(|t| t.chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_owned()).unwrap_or_default()
}

/// Lee un archivo de audio. La portada (si trae) se guarda en `covers`
/// con su hash como nombre, así un álbum entero comparte un solo archivo.
pub fn read_track(path: &Path, covers: &Path, now: u64) -> Result<Track, String> {
    let format = extension(path).ok_or("formato no admitido")?;
    let (size, modified) = stamp(path).map_err(|e| e.to_string())?;
    if size == 0 || size > MAX_AUDIO {
        return Err("archivo vacío o demasiado grande".into());
    }
    let hash = content_hash(path).map_err(|e| e.to_string())?;
    let mut track = Track {
        id: track_id(path),
        path: path.to_path_buf(),
        title: title_from_file_name(path),
        size,
        modified,
        format,
        hash,
        source: Source::Local,
        added: now,
        ..Default::default()
    };
    // Sin etiquetas legibles la canción sigue valiendo (nombre del archivo).
    if let Ok(tagged) = lofty::read_from_path(path) {
        let props = tagged.properties();
        track.duration = props.duration().as_secs_f64();
        track.bitrate = props.audio_bitrate().or(props.overall_bitrate()).filter(|&b| b > 0);
        if let Some(tag) = tagged.primary_tag().or_else(|| tagged.first_tag()) {
            let title = clean(tag.title(), 200);
            if !title.is_empty() {
                track.title = title;
            }
            track.artist = clean(tag.artist(), 120);
            track.album = clean(tag.album(), 120);
            track.track_no = tag.track().filter(|&n| n > 0 && n < 1000);
            track.year = tag.year().filter(|&y| (1000..3000).contains(&y));
            track.cover = save_cover(tag.pictures(), covers);
        }
    }
    Ok(track)
}

/// Guarda la portada (la delantera si hay varias). Devuelve su nombre.
fn save_cover(pictures: &[lofty::picture::Picture], covers: &Path) -> Option<String> {
    use lofty::picture::PictureType;
    let picture = pictures
        .iter()
        .find(|p| p.pic_type() == PictureType::CoverFront)
        .or_else(|| pictures.first())?;
    let data = picture.data();
    if data.is_empty() || data.len() > MAX_COVER {
        return None;
    }
    let ext = match picture.mime_type() {
        Some(MimeType::Png) => "png",
        Some(MimeType::Jpeg) => "jpg",
        _ => sniff_image(data)?,
    };
    save_cover_bytes(data, ext, covers)
}

pub fn sniff_image(data: &[u8]) -> Option<&'static str> {
    if data.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if data.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("png")
    } else if data.len() > 12 && &data[..4] == b"RIFF" && &data[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

pub fn save_cover_bytes(data: &[u8], ext: &str, covers: &Path) -> Option<String> {
    let name = format!("{}.{ext}", hex(&Sha1::digest(data)[..10]));
    let path = covers.join(&name);
    if !path.exists() {
        fs::create_dir_all(covers).ok()?;
        crate::vault::write_atomic(&path, data).ok()?;
    }
    Some(name)
}

/// Nombre de portada válido (lo que llega en una URL).
pub fn is_cover_name(name: &str) -> bool {
    let Some((stem, ext)) = name.split_once('.') else { return false };
    stem.len() == 20 && stem.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()) && matches!(ext, "jpg" | "png" | "webp")
}

#[cfg(test)]
pub mod tests {
    use super::*;

    /// WAV PCM de 16 bits con un tono (para pruebas).
    pub fn wav(seconds: f32, freqs: &[f32], rate: u32) -> Vec<u8> {
        let samples = (seconds * rate as f32) as usize;
        let mut data = Vec::with_capacity(samples * 2);
        for i in 0..samples {
            let t = i as f32 / rate as f32;
            // Melodía: cambia de nota cada medio segundo.
            let f = freqs[(t * 2.0) as usize % freqs.len()];
            let v = (t * f * std::f32::consts::TAU).sin() * 0.5 + (t * f * 2.0 * std::f32::consts::TAU).sin() * 0.2;
            data.extend_from_slice(&((v * 20000.0) as i16).to_le_bytes());
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * 2).to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&16u16.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        out
    }

    pub fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("ytmp-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn walks_only_audio_and_skips_hidden() {
        let dir = temp_dir("walk");
        fs::create_dir_all(dir.join("Álbum/Disco 1")).unwrap();
        fs::create_dir_all(dir.join("$RECYCLE.BIN")).unwrap();
        fs::create_dir_all(dir.join(".oculta")).unwrap();
        for f in ["Álbum/Disco 1/01 - Uno.mp3", "Álbum/cover.jpg", "Álbum/Dos.FLAC", "$RECYCLE.BIN/x.mp3", ".oculta/y.ogg", "notas.txt", "tres.opus"] {
            fs::write(dir.join(f), b"x").unwrap();
        }
        let mut found: Vec<String> = audio_files(&dir, &AtomicBool::new(false))
            .iter()
            .map(|p| p.strip_prefix(&dir).unwrap().to_string_lossy().replace('\\', "/"))
            .collect();
        found.sort();
        assert_eq!(found, ["tres.opus", "Álbum/Disco 1/01 - Uno.mp3", "Álbum/Dos.FLAC"]);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn ids_names_and_titles() {
        assert_eq!(track_id(Path::new("C:\\Música\\A.mp3")), track_id(Path::new("c:/música/a.MP3")));
        assert_ne!(track_id(Path::new("C:\\a.mp3")), track_id(Path::new("C:\\b.mp3")));
        assert_eq!(track_id(Path::new("x")).len(), 16);
        assert_eq!(title_from_file_name(Path::new("03 - Mi tema.mp3")), "Mi tema");
        assert_eq!(title_from_file_name(Path::new("TAM033-Ronan_Dec-1-Supernova.mp3")), "TAM033-Ronan Dec-1-Supernova");
        assert_eq!(title_from_file_name(Path::new("1999.mp3")), "1999");
        assert!(is_cover_name("0123456789abcdef0123.jpg"));
        for bad in ["../x.jpg", "0123456789abcdef0123.exe", "0123456789ABCDEF0123.jpg", "x.jpg", "0123456789abcdef0123"] {
            assert!(!is_cover_name(bad), "{bad}");
        }
        assert_eq!(sniff_image(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(sniff_image(b"GIF89a"), None);
    }

    #[test]
    fn reads_a_wav_file() {
        let dir = temp_dir("read");
        let path = dir.join("07 - Tono de prueba.wav");
        fs::write(&path, wav(3.0, &[440.0], 22050)).unwrap();
        let track = read_track(&path, &dir.join("covers"), 1234).unwrap();
        assert_eq!(track.title, "Tono de prueba");
        assert_eq!(track.format, "wav");
        assert!((track.duration - 3.0).abs() < 0.05, "{}", track.duration);
        assert_eq!(track.hash, content_hash(&path).unwrap());
        assert_eq!(track.hash.len(), 24);
        // Un byte distinto en medio cambia la huella.
        let mut bytes = fs::read(&path).unwrap();
        let mid = bytes.len() / 2;
        bytes[mid] ^= 1;
        let other = dir.join("otro.wav");
        fs::write(&other, &bytes).unwrap();
        assert_ne!(content_hash(&other).unwrap(), track.hash);
        assert_eq!(sha1_file(&path).unwrap().len(), 40);
        assert_eq!(track.added, 1234);
        assert!(read_track(&dir.join("no.txt"), &dir, 0).is_err());
        let _ = fs::remove_dir_all(dir);
    }
}
