//! Internet Archive: búsqueda y descarga de música con licencia libre.
//!
//! Solo se ofrece lo que el Archive marca con una licencia que permite
//! copiar y conservar la obra: Creative Commons BY, BY-SA, BY-ND, CC0 y
//! dominio público. Las licencias *NoComercial* (NC) quedan fuera porque la
//! descarga forma parte de un plan de pago. El filtro va en la consulta y se
//! vuelve a comprobar aquí con cada resultado.
//!
//! Normas de uso que se respetan: User-Agent propio, pocas descargas a la
//! vez, espera ante 429 y nada de rastreos masivos (solo lo que el usuario
//! pide).

use serde::Serialize;
use serde_json::Value;

pub const BASE: &str = "https://archive.org";
pub const ROWS: u32 = 30;

/// Licencias aceptadas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum License {
    PublicDomain,
    Cc0,
    By,
    BySa,
    ByNd,
}

impl License {
    /// Lee la URL de licencia del Archive (`http://creativecommons.org/licenses/by-sa/3.0/us/`).
    pub fn from_url(url: &str) -> Option<Self> {
        let u = url.trim().to_ascii_lowercase();
        let u = u.trim_start_matches("https://").trim_start_matches("http://").trim_start_matches("www.");
        if let Some(rest) = u.strip_prefix("creativecommons.org/licenses/") {
            let kind = rest.split('/').next().unwrap_or_default();
            return match kind {
                "by" => Some(License::By),
                "by-sa" => Some(License::BySa),
                "by-nd" => Some(License::ByNd),
                _ => None,
            };
        }
        if u.starts_with("creativecommons.org/publicdomain/zero/") {
            return Some(License::Cc0);
        }
        if u.starts_with("creativecommons.org/publicdomain/mark/") || u.starts_with("creativecommons.org/licenses/publicdomain") {
            return Some(License::PublicDomain);
        }
        None
    }

    pub fn label(self) -> &'static str {
        match self {
            License::PublicDomain => "Dominio público",
            License::Cc0 => "CC0",
            License::By => "CC BY",
            License::BySa => "CC BY-SA",
            License::ByNd => "CC BY-ND",
        }
    }

    /// Qué hay que hacer al usar la obra.
    pub fn terms(self) -> &'static str {
        match self {
            License::PublicDomain | License::Cc0 => "Sin restricciones.",
            License::By => "Cita a la autoría si la compartes.",
            License::BySa => "Cita a la autoría; si la modificas, comparte con la misma licencia.",
            License::ByNd => "Cita a la autoría; no la distribuyas modificada.",
        }
    }
}

/// Filtro de licencias para la consulta.
const LICENSE_FILTER: &str = "licenseurl:(*creativecommons.org/licenses/by/* OR *creativecommons.org/licenses/by-sa/* \
     OR *creativecommons.org/licenses/by-nd/* OR *creativecommons.org/publicdomain/*)";
/// Fuera audiolibros, radio y podcasts: se busca música.
const NOT_MUSIC: &str = "NOT collection:(librivoxaudio OR audio_bookspoetry OR podcasts OR radioprograms OR oldtimeradio)";

/// Escapa lo que el usuario escribe (sin operadores ni comodines suyos).
pub fn escape_query(text: &str) -> String {
    let words: Vec<String> = text
        .split_whitespace()
        .take(12)
        .map(|w| {
            w.chars()
                .filter(|c| !c.is_control())
                .flat_map(|c| {
                    let special = "+-&|!(){}[]^\"~*?:\\/".contains(c);
                    special.then_some('\\').into_iter().chain(std::iter::once(c))
                })
                .collect::<String>()
        })
        .filter(|w| !w.is_empty() && !matches!(w.as_str(), "AND" | "OR" | "NOT"))
        .collect();
    words.join(" AND ")
}

pub fn encode_component(text: &str) -> String {
    url::form_urlencoded::byte_serialize(text.as_bytes()).collect()
}

/// URL de búsqueda. `popular` ordena por descargas; si no, por relevancia.
pub fn search_url(base: &str, text: &str, page: u32, popular: bool) -> Option<String> {
    let words = escape_query(text);
    if words.is_empty() {
        return None;
    }
    let query = format!("({words}) AND mediatype:audio AND {LICENSE_FILTER} AND {NOT_MUSIC}");
    let mut url = format!(
        "{base}/advancedsearch.php?q={}&fl=identifier,title,creator,licenseurl,downloads,year,date&rows={ROWS}&page={}&output=json",
        encode_component(&query),
        page.clamp(1, 100),
    );
    if popular {
        url.push_str("&sort=downloads+desc");
    }
    Some(url)
}

/// El Archive devuelve a veces texto y a veces una lista de textos.
fn text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "),
        Value::Number(n) => n.to_string(),
        _ => String::new(),
    }
}

fn clean(s: String, max: usize) -> String {
    s.chars().filter(|c| !c.is_control()).take(max).collect::<String>().trim().to_owned()
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub id: String,
    pub title: String,
    pub creator: String,
    pub license: License,
    pub license_label: &'static str,
    pub downloads: u64,
    pub year: String,
}

/// Identificador del Archive: letras, números, `.`, `-` y `_`.
pub fn is_identifier(id: &str) -> bool {
    (1..=100).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'))
}

pub fn parse_search(json: &Value) -> (u64, Vec<Hit>) {
    let total = json["response"]["numFound"].as_u64().unwrap_or(0);
    let hits = json["response"]["docs"]
        .as_array()
        .map(|docs| {
            docs.iter()
                .filter_map(|d| {
                    let id = text(&d["identifier"]);
                    let license = License::from_url(&text(&d["licenseurl"]))?;
                    is_identifier(&id).then(|| Hit {
                        title: clean(text(&d["title"]), 200).chars().take(200).collect::<String>(),
                        creator: clean(text(&d["creator"]), 120),
                        license,
                        license_label: license.label(),
                        downloads: d["downloads"].as_u64().unwrap_or(0),
                        year: clean(
                            if d["year"].is_null() { text(&d["date"]).chars().take(4).collect() } else { text(&d["year"]) },
                            4,
                        ),
                        id,
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    (total, hits)
}

/// Una canción del álbum (ya elegido el formato).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoteTrack {
    /// Nombre del archivo en el Archive.
    pub file: String,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub track_no: Option<u32>,
    pub duration: f64,
    pub size: u64,
    pub sha1: String,
    pub md5: String,
    /// Extensión (`mp3`, `ogg`, `flac`).
    pub format: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Item {
    pub id: String,
    pub title: String,
    pub creator: String,
    pub license: License,
    pub license_label: &'static str,
    pub license_terms: &'static str,
    pub license_url: String,
    pub tracks: Vec<RemoteTrack>,
    /// Imagen de portada del álbum (archivo del propio item), si hay.
    pub cover_file: Option<String>,
}

/// Formatos de audio del Archive que el motor reproduce: (formato, extensión,
/// ¿sin pérdida?).
fn audio_format(format: &str, name: &str) -> Option<(&'static str, bool)> {
    let f = format.to_ascii_lowercase();
    let ext = name.rsplit('.').next().unwrap_or_default().to_ascii_lowercase();
    if f.contains("mp3") || (f.is_empty() && ext == "mp3") {
        Some(("mp3", false))
    } else if f.contains("flac") {
        Some(("flac", true))
    } else if f.contains("ogg vorbis") || f == "ogg" {
        Some(("ogg", false))
    } else if f == "opus" {
        Some(("opus", false))
    } else if f.contains("wave") && ext == "wav" {
        Some(("wav", true))
    } else if (f.contains("aac") || f.contains("m4a") || f.contains("mpeg4 audio")) && matches!(ext.as_str(), "m4a" | "aac") {
        Some(("m4a", false))
    } else {
        None
    }
}

/// Orden de preferencia: compacto = MP3 VBR > MP3 > Ogg > FLAC > WAV;
/// mejor = FLAC > WAV > MP3 VBR > MP3 > Ogg. Menos es mejor.
fn preference(format: &str, ext: &str, best: bool) -> u32 {
    let f = format.to_ascii_lowercase();
    let vbr = f.contains("vbr");
    let low = f.contains("64kbps") || f.contains("32kbps");
    match (best, ext) {
        (true, "flac") => 0,
        (true, "wav") => 1,
        (_, "mp3") if vbr => 2,
        (_, "mp3") if !low => 3,
        (_, "ogg" | "opus" | "m4a") => 4,
        (_, "mp3") => 5,
        (false, "flac") => 6,
        (false, "wav") => 7,
        _ => 9,
    }
}

fn number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => {
            // "263.71" o "4:23"
            if let Some((m, s)) = s.split_once(':') {
                Some(m.trim().parse::<f64>().ok()? * 60.0 + s.trim().parse::<f64>().ok()?)
            } else {
                s.trim().parse().ok()
            }
        }
        _ => None,
    }
    .filter(|n| n.is_finite() && *n >= 0.0)
}

fn track_number(value: &Value) -> Option<u32> {
    // "3", "03", "3/10"
    text(value).split('/').next()?.trim().parse().ok().filter(|&n| n > 0 && n < 1000)
}

/// Interpreta los metadatos de un item: una canción por grabación original,
/// en el formato preferido. `None` si la licencia no es aceptable.
pub fn parse_item(json: &Value, best: bool) -> Result<Item, String> {
    let meta = &json["metadata"];
    let id = text(&meta["identifier"]);
    if !is_identifier(&id) {
        return Err("el Archive no devolvió ese álbum".into());
    }
    let license_url = text(&meta["licenseurl"]);
    let license = License::from_url(&license_url).ok_or("este álbum no tiene una licencia libre que permita descargarlo")?;
    let item_title = clean(text(&meta["title"]), 200);
    let item_creator = clean(text(&meta["creator"]), 120);
    let files = json["files"].as_array().cloned().unwrap_or_default();

    // Agrupa derivados con su original: la clave es el nombre del original.
    let mut groups: Vec<(String, Vec<&Value>)> = Vec::new();
    for f in &files {
        let name = text(&f["name"]);
        if name.is_empty() || name.contains("..") || name.starts_with('/') {
            continue;
        }
        let Some(_) = audio_format(&text(&f["format"]), &name) else { continue };
        let key = match text(&f["source"]).as_str() {
            "derivative" => text(&f["original"]),
            _ => name.clone(),
        };
        match groups.iter_mut().find(|(k, _)| *k == key) {
            Some((_, members)) => members.push(f),
            None => groups.push((key, vec![f])),
        }
    }
    let mut tracks: Vec<RemoteTrack> = groups
        .iter()
        .filter_map(|(original, members)| {
            let chosen = members.iter().min_by_key(|f| {
                let name = text(&f["name"]);
                let (ext, _) = audio_format(&text(&f["format"]), &name).unwrap_or(("", false));
                preference(&text(&f["format"]), ext, best)
            })?;
            let name = text(&chosen["name"]);
            let (ext, _) = audio_format(&text(&chosen["format"]), &name)?;
            // Las etiquetas suelen estar en el original.
            let source = members.iter().find(|f| text(&f["name"]) == *original).copied().unwrap_or(chosen);
            let pick = |key: &str| {
                let v = text(&chosen[key]);
                if v.is_empty() { text(&source[key]) } else { v }
            };
            let title = clean(pick("title"), 200);
            let title = if title.is_empty() {
                crate::music::scan::title_from_file_name(std::path::Path::new(&name))
            } else {
                title
            };
            let artist = clean(if pick("artist").is_empty() { pick("creator") } else { pick("artist") }, 120);
            Some(RemoteTrack {
                file: name.clone(),
                title,
                artist: if artist.is_empty() { item_creator.clone() } else { artist },
                album: { let a = clean(pick("album"), 120); if a.is_empty() { item_title.clone() } else { a } },
                track_no: track_number(&chosen["track"]).or_else(|| track_number(&source["track"])),
                duration: number(&chosen["length"]).or_else(|| number(&source["length"])).unwrap_or(0.0),
                size: number(&chosen["size"]).map(|n| n as u64).unwrap_or(0),
                sha1: text(&chosen["sha1"]).to_ascii_lowercase(),
                md5: text(&chosen["md5"]).to_ascii_lowercase(),
                format: ext.to_owned(),
            })
        })
        .collect();
    // Por número de pista si todas lo tienen; si no, por nombre de archivo
    // (que en los sellos suele empezar por el número).
    if tracks.iter().all(|t| t.track_no.is_some()) {
        tracks.sort_by(|a, b| a.track_no.cmp(&b.track_no).then_with(|| natural_cmp(&a.file, &b.file)));
    } else {
        tracks.sort_by(|a, b| natural_cmp(&a.file, &b.file));
    }

    let cover_file = files
        .iter()
        .filter(|f| matches!(text(&f["format"]).as_str(), "JPEG" | "PNG" | "Item Tile"))
        .filter(|f| number(&f["size"]).is_some_and(|s| s < 8.0 * 1024.0 * 1024.0))
        .min_by_key(|f| {
            let name = text(&f["name"]).to_ascii_lowercase();
            let tile = text(&f["format"]) == "Item Tile";
            (u8::from(!(name.contains("cover") || name.contains("front") || name.contains("folder"))), u8::from(tile))
        })
        .map(|f| text(&f["name"]))
        .filter(|n| !n.contains(".."));

    Ok(Item {
        id,
        title: item_title,
        creator: item_creator,
        license,
        license_label: license.label(),
        license_terms: license.terms(),
        license_url,
        tracks,
        cover_file,
    })
}

/// "Pista 2" antes que "Pista 10".
fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let key = |s: &str| -> Vec<(u8, u64, String)> {
        let mut out = Vec::new();
        let mut digits = String::new();
        let mut letters = String::new();
        for c in s.chars() {
            if c.is_ascii_digit() {
                if !letters.is_empty() {
                    out.push((1, 0, std::mem::take(&mut letters).to_lowercase()));
                }
                digits.push(c);
            } else {
                if !digits.is_empty() {
                    out.push((0, digits.parse().unwrap_or(u64::MAX), String::new()));
                    digits.clear();
                }
                letters.push(c);
            }
        }
        if !digits.is_empty() {
            out.push((0, digits.parse().unwrap_or(u64::MAX), String::new()));
        }
        if !letters.is_empty() {
            out.push((1, 0, letters.to_lowercase()));
        }
        out
    };
    key(a).cmp(&key(b))
}

/// Ruta de descarga de un archivo (cada parte codificada).
pub fn download_url(base: &str, item: &str, file: &str) -> String {
    let path: Vec<String> = file
        .split('/')
        .map(|part| url::form_urlencoded::byte_serialize(part.as_bytes()).collect::<String>().replace('+', "%20"))
        .collect();
    format!("{base}/download/{}/{}", encode_component(item), path.join("/"))
}

pub fn metadata_url(base: &str, item: &str) -> String {
    format!("{base}/metadata/{}", encode_component(item))
}

pub fn thumbnail_url(base: &str, item: &str) -> String {
    format!("{base}/services/img/{}", encode_component(item))
}

pub fn details_url(item: &str) -> String {
    format!("https://archive.org/details/{}", encode_component(item))
}

#[cfg(test)]
pub mod tests {
    use super::*;
    use serde_json::json;

    /// Respuesta real (recortada) de /metadata/TAM033-Ronan_Dec, con
    /// derivados añadidos como los genera el Archive.
    pub fn sample_item() -> Value {
        json!({
          "created": 1791974020, "d1": "ia601500.us.archive.org", "dir": "/28/items/TAM033-Ronan_Dec", "files_count": 12,
          "metadata": {
            "identifier": "TAM033-Ronan_Dec", "title": "Ronan Dec. - Hirnwald EP [TAM033]", "creator": "Ronan Dec.",
            "mediatype": "audio", "collection": ["taktalsmittel", "netlabels"], "licenseurl": "http://creativecommons.org/licenses/by-sa/2.0/de/"
          },
          "files": [
            { "name": "TAM033-Cover.jpg", "source": "original", "format": "JPEG", "size": "1027079", "md5": "e3", "sha1": "96" },
            { "name": "TAM033-Ronan_Dec-1-Supernova.mp3", "source": "original", "title": "TAM033-Ronan_Dec-1-Supernova", "format": "VBR MP3",
              "creator": "ronan dec", "album": "Hirnwald EP", "artist": "ronan dec", "size": "8438638", "length": "263.71",
              "md5": "88c9b7d6d40118ebcba827ff1fdb2a77", "sha1": "918079C0668AE9E1151EB23899555D933DBA6131" },
            { "name": "TAM033-Ronan_Dec-1-Supernova.ogg", "source": "derivative", "original": "TAM033-Ronan_Dec-1-Supernova.mp3",
              "format": "Ogg Vorbis", "size": "6000000", "length": "263.70", "md5": "aa", "sha1": "bb" },
            { "name": "TAM033-Ronan_Dec-1-Supernova_64kb.mp3", "source": "derivative", "original": "TAM033-Ronan_Dec-1-Supernova.mp3",
              "format": "64Kbps MP3", "size": "2100000", "length": "263.71", "md5": "cc", "sha1": "dd" },
            { "name": "TAM033-Ronan_Dec-10-Final.flac", "source": "original", "format": "Flac", "title": "Final", "track": "10/10",
              "size": "40000000", "length": "4:10", "md5": "ee", "sha1": "ff" },
            { "name": "TAM033-Ronan_Dec-10-Final.mp3", "source": "derivative", "original": "TAM033-Ronan_Dec-10-Final.flac",
              "format": "VBR MP3", "size": "9000000", "length": "250.1", "md5": "11", "sha1": "22" },
            { "name": "TAM033-Ronan_Dec-2-Waehrend_du_schliefst.mp3", "source": "original", "format": "VBR MP3",
              "title": "Waehrend du schliefst", "artist": "ronan dec", "size": "12008263", "length": "375.26", "md5": "dc", "sha1": "ec" },
            { "name": "TAM033-Ronan_Dec_archive.torrent", "source": "metadata", "format": "Archive BitTorrent", "size": "4059" },
            { "name": "TAM033-Ronan_Dec_meta.xml", "source": "metadata", "format": "Metadata", "size": "818" },
            { "name": "../../etc/passwd.mp3", "source": "original", "format": "VBR MP3", "size": "1" },
            { "name": "__ia_thumb.jpg", "source": "original", "format": "Item Tile", "size": "27632" }
          ]
        })
    }

    #[test]
    fn licenses() {
        assert_eq!(License::from_url("http://creativecommons.org/licenses/by-sa/2.0/de/"), Some(License::BySa));
        assert_eq!(License::from_url("https://creativecommons.org/licenses/by/4.0/"), Some(License::By));
        assert_eq!(License::from_url("http://creativecommons.org/licenses/by-nd/3.0/"), Some(License::ByNd));
        assert_eq!(License::from_url("http://creativecommons.org/publicdomain/zero/1.0/"), Some(License::Cc0));
        assert_eq!(License::from_url("http://creativecommons.org/publicdomain/mark/1.0/"), Some(License::PublicDomain));
        for nc in [
            "http://creativecommons.org/licenses/by-nc/3.0/",
            "http://creativecommons.org/licenses/by-nc-sa/3.0/us/",
            "http://creativecommons.org/licenses/by-nc-nd/2.5/",
            "",
            "https://example.com/licenses/by/",
            "http://creativecommons.org/licenses/by-nd-nc/1.0/",
        ] {
            assert_eq!(License::from_url(nc), None, "{nc}");
        }
    }

    #[test]
    fn queries_are_escaped_and_filtered() {
        assert_eq!(escape_query("ambient  techno"), "ambient AND techno");
        assert_eq!(escape_query("a:b (c) OR *"), "a\\:b AND \\(c\\) AND \\*");
        assert_eq!(escape_query("   "), "");
        let url = search_url(BASE, "ronan dec", 2, true).unwrap();
        assert!(url.starts_with("https://archive.org/advancedsearch.php?q="));
        let q = url::Url::parse(&url).unwrap();
        let query: String = q.query_pairs().find(|(k, _)| k == "q").unwrap().1.into_owned();
        assert!(query.starts_with("(ronan AND dec) AND mediatype:audio AND licenseurl:("), "{query}");
        assert!(query.contains("NOT collection:(librivoxaudio"));
        assert!(!query.contains("by-nc"));
        assert!(url.contains("&page=2&") && url.ends_with("&sort=downloads+desc"));
        assert!(search_url(BASE, "", 1, false).is_none());
        assert!(!search_url(BASE, "x", 1, false).unwrap().contains("sort="));
    }

    #[test]
    fn parses_search_results_and_drops_bad_ones() {
        let json = json!({ "responseHeader": { "status": 0 }, "response": { "numFound": 2959, "start": 0, "docs": [
            { "identifier": "TAM033-Ronan_Dec", "title": "Ronan Dec. - Hirnwald EP [TAM033]", "creator": "Ronan Dec.",
              "licenseurl": "http://creativecommons.org/licenses/by-sa/2.0/de/", "downloads": 191994 },
            { "identifier": "TFN147", "title": "[TFN147] fabioRosho - Untitled", "creator": ["fabioRosho", "Otro"],
              "licenseurl": "http://creativecommons.org/licenses/by-sa/3.0/us/", "downloads": 148357, "year": "2009" },
            { "identifier": "nc-album", "title": "NC", "licenseurl": "http://creativecommons.org/licenses/by-nc/3.0/" },
            { "identifier": "../malo", "title": "x", "licenseurl": "http://creativecommons.org/licenses/by/3.0/" }
        ]}});
        let (total, hits) = parse_search(&json);
        assert_eq!(total, 2959);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].license_label, "CC BY-SA");
        assert_eq!(hits[1].creator, "fabioRosho, Otro");
        assert_eq!(hits[1].year, "2009");
        assert_eq!(parse_search(&json!({})).1.len(), 0);
    }

    #[test]
    fn picks_one_file_per_recording() {
        let item = parse_item(&sample_item(), false).unwrap();
        assert_eq!(item.license, License::BySa);
        assert_eq!(item.cover_file.as_deref(), Some("TAM033-Cover.jpg"));
        let files: Vec<&str> = item.tracks.iter().map(|t| t.file.as_str()).collect();
        // Orden: por número de pista o natural (1, 2, 10); compacto = MP3.
        assert_eq!(files, ["TAM033-Ronan_Dec-1-Supernova.mp3", "TAM033-Ronan_Dec-2-Waehrend_du_schliefst.mp3", "TAM033-Ronan_Dec-10-Final.mp3"]);
        let first = &item.tracks[0];
        assert_eq!((first.artist.as_str(), first.album.as_str()), ("ronan dec", "Hirnwald EP"));
        assert_eq!(first.sha1, "918079c0668ae9e1151eb23899555d933dba6131");
        assert_eq!((first.size, first.format.as_str()), (8_438_638, "mp3"));
        assert!((first.duration - 263.71).abs() < 0.01);
        let last = &item.tracks[2];
        assert_eq!(last.title, "Final", "el derivado toma las etiquetas del original");
        assert_eq!(last.track_no, Some(10));
        assert_eq!(item.tracks[1].artist, "ronan dec");

        let best = parse_item(&sample_item(), true).unwrap();
        let files: Vec<&str> = best.tracks.iter().map(|t| t.file.as_str()).collect();
        assert!(files.contains(&"TAM033-Ronan_Dec-10-Final.flac"), "mejor calidad = FLAC: {files:?}");
        assert!((best.tracks.iter().find(|t| t.format == "flac").unwrap().duration - 250.0).abs() < 0.01, "4:10");

        let mut nc = sample_item();
        nc["metadata"]["licenseurl"] = "http://creativecommons.org/licenses/by-nc-sa/3.0/".into();
        assert!(parse_item(&nc, false).is_err());
        assert!(parse_item(&json!({"metadata": {}}), false).is_err());
    }

    #[test]
    fn urls() {
        assert_eq!(download_url(BASE, "TAM033-Ronan_Dec", "Disco 1/01 Tema #1.mp3"), "https://archive.org/download/TAM033-Ronan_Dec/Disco%201/01%20Tema%20%231.mp3");
        assert_eq!(metadata_url(BASE, "TFN147"), "https://archive.org/metadata/TFN147");
        assert_eq!(thumbnail_url(BASE, "TFN147"), "https://archive.org/services/img/TFN147");
        assert!(is_identifier("TAM033-Ronan_Dec") && !is_identifier("a/b") && !is_identifier(""));
        assert_eq!(natural_cmp("Pista 2", "Pista 10"), std::cmp::Ordering::Less);
    }
}
