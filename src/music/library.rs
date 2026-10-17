//! Biblioteca de música: canciones, playlists y detección de duplicados.
//!
//! Todo lo de aquí es puro (sin disco ni red) para poder probarlo a fondo.
//!
//! Una canción se considera **la misma** que otra si:
//! 1. **Mismo archivo**: el contenido es idéntico (huella rápida del
//!    contenido, ver `scan::content_hash`).
//! 2. **Misma canción**: artista y título coinciden una vez normalizados
//!    (mayúsculas, tildes, "feat.", "(Remastered)"...) y la duración difiere
//!    en menos de 3 s.
//! 3. **Mismo audio**: la huella acústica (Chromaprint) coincide, aunque las
//!    etiquetas o el formato sean distintos (un MP3 y un FLAC del mismo tema).

use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

use serde::{Deserialize, Serialize};

/// Diferencia máxima de duración para "misma canción" por etiquetas.
pub const SAME_SONG_SECONDS: f64 = 3.0;
/// Para comparar huellas solo se miran canciones de duración parecida.
pub const SAME_AUDIO_SECONDS: f64 = 10.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Source {
    /// Un archivo del usuario.
    #[default]
    Local,
    /// Descargada del Internet Archive (con su licencia y autoría).
    Archive { item: String, file: String, license: String, creator: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Track {
    /// Estable para una ruta (hash de la ruta).
    pub id: String,
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub track_no: Option<u32>,
    pub year: Option<u32>,
    /// Segundos (0 si se desconoce).
    pub duration: f64,
    pub size: u64,
    /// Fecha de modificación del archivo (segundos Unix).
    pub modified: u64,
    /// Extensión en minúsculas (`mp3`, `flac`...).
    pub format: String,
    /// kbps.
    pub bitrate: Option<u32>,
    /// Huella del contenido (`scan::content_hash`).
    pub hash: String,
    /// Portada en la caché (nombre del archivo).
    pub cover: Option<String>,
    pub source: Source,
    /// Cuándo entró en la biblioteca (segundos Unix).
    pub added: u64,
}

impl Track {
    pub fn lossless(&self) -> bool {
        matches!(self.format.as_str(), "flac" | "wav")
    }

    /// Para elegir cuál conservar entre duplicados: sin pérdida > más kbps >
    /// más grande.
    pub fn quality(&self) -> u64 {
        let kbps = self.bitrate.map(u64::from).unwrap_or_else(|| {
            if self.duration > 0.0 {
                (self.size as f64 * 8.0 / 1000.0 / self.duration) as u64
            } else {
                0
            }
        });
        (u64::from(self.lossless()) << 40) | (kbps.min(100_000) << 20) | (self.size >> 20).min(0xF_FFFF)
    }

    pub fn meta_key(&self) -> Option<String> {
        meta_key(&self.artist, &self.title)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Playlist {
    pub id: String,
    pub name: String,
    pub tracks: Vec<String>,
    pub created: u64,
    pub updated: u64,
    /// Si se creó al descargar un álbum: su identificador en el Archive.
    pub origin: Option<String>,
}

/// Un archivo del Archive que resultó ser una canción que ya estaba (se
/// descargó, se comparó y se descartó): no se vuelve a descargar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Alias {
    pub item: String,
    pub file: String,
    pub track: String,
    pub reason: Option<Existing>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    /// MP3 cuando existe: ocupa mucho menos.
    #[default]
    Compact,
    /// El original sin pérdida (FLAC) si existe.
    Best,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Library {
    pub version: u32,
    /// Carpetas importadas (se vuelven a revisar al abrir).
    pub folders: Vec<PathBuf>,
    pub tracks: Vec<Track>,
    pub playlists: Vec<Playlist>,
    pub quality: Quality,
    /// Grupos de duplicados que el usuario decidió conservar (ids ordenados).
    pub ignored_duplicates: Vec<Vec<String>>,
    pub aliases: Vec<Alias>,
}

/// Normaliza un texto para comparar: minúsculas, sin tildes ni signos, sin
/// "(Remastered 2011)", "[Official Audio]", "feat. X" ni números de pista.
pub fn normalize(text: &str) -> String {
    let mut s: String = text.to_lowercase();
    // Lo que va entre paréntesis o corchetes son versiones o adornos.
    let mut out = String::with_capacity(s.len());
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '(' | '[' | '{' => depth += 1,
            ')' | ']' | '}' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    s = out;
    for marker in [" feat. ", " feat ", " ft. ", " ft ", " featuring "] {
        if let Some(i) = s.find(marker) {
            s.truncate(i);
        }
    }
    let folded: String = s.chars().map(fold).collect();
    let words: Vec<&str> = folded.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
    // "01 - Título" / "1-02 Título": fuera los números de pista del principio.
    let skip = words.iter().take_while(|w| w.len() <= 3 && w.chars().all(|c| c.is_ascii_digit())).count();
    let words = if skip < words.len() { &words[skip..] } else { &words[..] };
    let words: Vec<&str> = words.iter().copied().filter(|w| !matches!(*w, "the" | "and" | "y")).collect();
    words.join(" ")
}

fn fold(c: char) -> char {
    match c {
        'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' => 'a',
        'é' | 'è' | 'ë' | 'ê' => 'e',
        'í' | 'ì' | 'ï' | 'î' => 'i',
        'ó' | 'ò' | 'ö' | 'ô' | 'õ' | 'ø' => 'o',
        'ú' | 'ù' | 'ü' | 'û' => 'u',
        'ñ' => 'n',
        'ç' => 'c',
        '&' | '+' => ' ',
        c => c,
    }
}

/// Clave de "misma canción": artista + título normalizados. Sin título no
/// hay clave (no se compara por etiquetas).
pub fn meta_key(artist: &str, title: &str) -> Option<String> {
    let title = normalize(title);
    if title.is_empty() {
        return None;
    }
    Some(format!("{}\u{1f}{title}", normalize(artist)))
}

fn close(a: f64, b: f64, tolerance: f64) -> bool {
    a > 0.0 && b > 0.0 && (a - b).abs() <= tolerance
}

/// Una canción que se quiere añadir (antes de descargarla).
#[derive(Debug, Clone, Default)]
pub struct Candidate<'a> {
    pub hash: Option<&'a str>,
    pub archive: Option<(&'a str, &'a str)>,
    pub artist: &'a str,
    pub title: &'a str,
    pub duration: f64,
}

/// Por qué no hace falta descargarla. (Los nombres dicen qué es "lo mismo";
/// van tal cual a la página.)
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Existing {
    /// Ya se descargó ese mismo archivo.
    SameDownload,
    /// Hay un archivo idéntico.
    SameFile,
    /// Misma canción por etiquetas y duración.
    SameSong,
    /// Mismo audio por huella acústica (tras descargar).
    SameAudio,
}

impl Existing {
    pub fn describe(self) -> &'static str {
        match self {
            Existing::SameDownload => "ya la descargaste",
            Existing::SameFile => "ya tienes ese mismo archivo",
            Existing::SameSong => "ya la tienes (misma canción)",
            Existing::SameAudio => "ya la tienes (mismo audio)",
        }
    }
}

impl Library {
    pub fn track(&self, id: &str) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    /// Añade o reemplaza (misma id = misma ruta). Conserva cuándo se añadió.
    pub fn upsert(&mut self, mut track: Track) {
        match self.tracks.iter_mut().find(|t| t.id == track.id) {
            Some(existing) => {
                track.added = existing.added;
                if matches!(track.source, Source::Local) {
                    track.source = existing.source.clone();
                }
                *existing = track;
            }
            None => self.tracks.push(track),
        }
    }

    /// Quita canciones (también de las playlists). Devuelve las quitadas.
    pub fn remove(&mut self, ids: &[String]) -> Vec<Track> {
        let (gone, keep): (Vec<Track>, Vec<Track>) = std::mem::take(&mut self.tracks).into_iter().partition(|t| ids.contains(&t.id));
        self.tracks = keep;
        for playlist in &mut self.playlists {
            playlist.tracks.retain(|id| !ids.contains(id));
        }
        self.aliases.retain(|a| !ids.contains(&a.track));
        gone
    }

    /// ¿Ya está? Para no descargar lo que ya se tiene.
    pub fn find_existing(&self, c: &Candidate) -> Option<(Existing, &Track)> {
        if let Some((item, file)) = c.archive {
            let found = self.tracks.iter().find(|t| {
                matches!(&t.source, Source::Archive { item: i, file: f, .. } if i == item && f == file)
            });
            if let Some(t) = found {
                return Some((Existing::SameDownload, t));
            }
            let alias = self.aliases.iter().filter(|a| a.item == item && a.file == file).find_map(|a| {
                self.track(&a.track).map(|t| (a.reason.unwrap_or(Existing::SameDownload), t))
            });
            if alias.is_some() {
                return alias;
            }
        }
        if let Some(hash) = c.hash.filter(|s| !s.is_empty()) {
            if let Some(t) = self.tracks.iter().find(|t| t.hash == hash) {
                return Some((Existing::SameFile, t));
            }
        }
        let key = meta_key(c.artist, c.title)?;
        self.tracks
            .iter()
            .find(|t| close(t.duration, c.duration, SAME_SONG_SECONDS) && t.meta_key().as_deref() == Some(key.as_str()))
            .map(|t| (Existing::SameSong, t))
    }

    pub fn playlist_mut(&mut self, id: &str) -> Option<&mut Playlist> {
        self.playlists.iter_mut().find(|p| p.id == id)
    }
}

/// Un grupo de canciones repetidas.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGroup {
    /// El motivo más fuerte del grupo.
    pub reason: Existing,
    /// De mejor a peor calidad; la primera es la que se sugiere conservar.
    pub tracks: Vec<String>,
    /// Bytes que se liberan quitando todas menos la primera.
    pub wasted: u64,
}

struct UnionFind {
    parent: Vec<usize>,
}

impl UnionFind {
    fn new(n: usize) -> Self {
        Self { parent: (0..n).collect() }
    }
    fn find(&mut self, x: usize) -> usize {
        let mut root = x;
        while self.parent[root] != root {
            root = self.parent[root];
        }
        let mut x = x;
        while self.parent[x] != root {
            let next = self.parent[x];
            self.parent[x] = root;
            x = next;
        }
        root
    }
    fn union(&mut self, a: usize, b: usize) {
        let (a, b) = (self.find(a), self.find(b));
        if a != b {
            self.parent[b.max(a)] = a.min(b);
        }
    }
}

fn rank(reason: Existing) -> u8 {
    match reason {
        Existing::SameFile | Existing::SameDownload => 3,
        Existing::SameAudio => 2,
        Existing::SameSong => 1,
    }
}

/// Busca canciones repetidas. `same_audio(a, b)` compara huellas (puede ser
/// lento: solo se llama con duraciones parecidas).
pub fn find_duplicates(library: &Library, mut same_audio: impl FnMut(&Track, &Track) -> bool) -> Vec<DuplicateGroup> {
    let tracks = &library.tracks;
    let n = tracks.len();
    let mut uf = UnionFind::new(n);
    let mut reasons: HashMap<(usize, usize), Existing> = HashMap::new();
    let link = |uf: &mut UnionFind, reasons: &mut HashMap<(usize, usize), Existing>, a: usize, b: usize, why: Existing| {
        uf.union(a, b);
        let key = (a.min(b), a.max(b));
        let entry = reasons.entry(key).or_insert(why);
        if rank(why) > rank(*entry) {
            *entry = why;
        }
    };

    // 1. Mismo contenido.
    let mut by_hash: BTreeMap<&str, usize> = BTreeMap::new();
    for (i, t) in tracks.iter().enumerate() {
        if t.hash.is_empty() {
            continue;
        }
        match by_hash.get(t.hash.as_str()) {
            Some(&first) => link(&mut uf, &mut reasons, first, i, Existing::SameFile),
            None => {
                by_hash.insert(&t.hash, i);
            }
        }
    }
    // 2. Mismas etiquetas y duración parecida.
    let mut by_meta: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (i, t) in tracks.iter().enumerate() {
        if let Some(key) = t.meta_key() {
            by_meta.entry(key).or_default().push(i);
        }
    }
    for group in by_meta.values() {
        for (x, &a) in group.iter().enumerate() {
            for &b in &group[x + 1..] {
                if close(tracks[a].duration, tracks[b].duration, SAME_SONG_SECONDS) {
                    link(&mut uf, &mut reasons, a, b, Existing::SameSong);
                }
            }
        }
    }
    // 3. Huella acústica, solo entre duraciones parecidas (ventana deslizante
    //    sobre la lista ordenada por duración).
    let mut order: Vec<usize> = (0..n).filter(|&i| tracks[i].duration > 0.0).collect();
    order.sort_by(|&a, &b| tracks[a].duration.total_cmp(&tracks[b].duration));
    for (x, &a) in order.iter().enumerate() {
        for &b in &order[x + 1..] {
            if tracks[b].duration - tracks[a].duration > SAME_AUDIO_SECONDS {
                break;
            }
            if uf.find(a) == uf.find(b) {
                continue;
            }
            if same_audio(&tracks[a], &tracks[b]) {
                link(&mut uf, &mut reasons, a, b, Existing::SameAudio);
            }
        }
    }

    let mut groups: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for i in 0..n {
        let root = uf.find(i);
        groups.entry(root).or_default().push(i);
    }
    let mut out: Vec<DuplicateGroup> = groups
        .into_values()
        .filter(|g| g.len() > 1)
        .filter_map(|mut members| {
            members.sort_by(|&a, &b| tracks[b].quality().cmp(&tracks[a].quality()).then(tracks[a].added.cmp(&tracks[b].added)));
            let ids: Vec<String> = members.iter().map(|&i| tracks[i].id.clone()).collect();
            let mut sorted_ids = ids.clone();
            sorted_ids.sort();
            if library.ignored_duplicates.contains(&sorted_ids) {
                return None;
            }
            let reason = reasons
                .iter()
                .filter(|((a, b), _)| members.contains(a) && members.contains(b))
                .map(|(_, r)| *r)
                .max_by_key(|r| rank(*r))
                .unwrap_or(Existing::SameSong);
            let wasted = members[1..].iter().map(|&i| tracks[i].size).sum();
            Some(DuplicateGroup { reason, tracks: ids, wasted })
        })
        .collect();
    out.sort_by_key(|g| std::cmp::Reverse(g.wasted));
    out
}

/// Nombre de archivo seguro en Windows (sin caracteres prohibidos, nombres
/// reservados ni puntos o espacios al final) y de longitud razonable.
pub fn safe_file_name(text: &str, fallback: &str) -> String {
    let cleaned: String = text
        .chars()
        .map(|c| if c.is_control() || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*') { '_' } else { c })
        .collect();
    let mut name: String = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    name = name.chars().take(100).collect();
    let name = name.trim_end_matches(['.', ' ']).trim_start().to_owned();
    let stem = name.split('.').next().unwrap_or_default().to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || ((stem.starts_with("COM") || stem.starts_with("LPT")) && stem.len() == 4 && stem.as_bytes()[3].is_ascii_digit());
    if name.is_empty() || reserved || name.chars().all(|c| c == '_') {
        fallback.to_owned()
    } else {
        name
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn track(id: &str, artist: &str, title: &str, duration: f64) -> Track {
        Track {
            id: id.into(),
            path: PathBuf::from(format!("C:/Música/{id}.mp3")),
            artist: artist.into(),
            title: title.into(),
            duration,
            size: 5_000_000,
            format: "mp3".into(),
            bitrate: Some(192),
            hash: format!("hash-{id}"),
            ..Default::default()
        }
    }

    #[test]
    fn normalizes_titles_and_artists() {
        assert_eq!(normalize("Bohemian Rhapsody (Remastered 2011)"), "bohemian rhapsody");
        assert_eq!(normalize("01 - Canción Número Uno [Official Audio]"), "cancion numero uno");
        assert_eq!(normalize("Señor feat. Alguien"), "senor");
        assert_eq!(normalize("The Beatles"), "beatles");
        assert_eq!(normalize("Simon & Garfunkel"), normalize("Simon and Garfunkel"));
        assert_eq!(normalize("1-02 Para"), "para");
        assert_eq!(normalize("1999"), "1999", "un título que es un número no se borra");
        assert_eq!(normalize("  "), "");
        assert_eq!(meta_key("Ronan Dec.", "Para"), meta_key("RONAN DEC", "para"));
        assert_ne!(meta_key("Ronan Dec.", "Para"), meta_key("Otro", "Para"));
        assert_eq!(meta_key("x", ""), None);
    }

    #[test]
    fn finds_existing_before_downloading() {
        let mut lib = Library::default();
        let mut a = track("a", "Ronan Dec.", "Supernova", 263.7);
        a.source = Source::Archive { item: "TAM033".into(), file: "1-Supernova.mp3".into(), license: "by-sa".into(), creator: "Ronan Dec.".into() };
        lib.upsert(a);
        lib.upsert(track("b", "Otro", "Tema", 200.0));

        let same_download = Candidate { archive: Some(("TAM033", "1-Supernova.mp3")), ..Default::default() };
        assert_eq!(lib.find_existing(&same_download).map(|(e, t)| (e, t.id.as_str())), Some((Existing::SameDownload, "a")));
        let same_file = Candidate { hash: Some("hash-b"), ..Default::default() };
        assert_eq!(lib.find_existing(&same_file).map(|(e, _)| e), Some(Existing::SameFile));
        let same_song = Candidate { artist: "ronan dec", title: "Supernova (Original Mix)", duration: 265.0, ..Default::default() };
        assert_eq!(lib.find_existing(&same_song).map(|(e, _)| e), Some(Existing::SameSong));
        let longer = Candidate { artist: "ronan dec", title: "Supernova", duration: 300.0, ..Default::default() };
        assert_eq!(lib.find_existing(&longer), None, "otra versión (dura distinto)");
        let unknown_length = Candidate { artist: "ronan dec", title: "Supernova", duration: 0.0, ..Default::default() };
        assert_eq!(lib.find_existing(&unknown_length), None, "sin duración no se arriesga");
    }

    #[test]
    fn upsert_keeps_added_date_and_source() {
        let mut lib = Library::default();
        let mut a = track("a", "x", "y", 10.0);
        a.added = 100;
        a.source = Source::Archive { item: "i".into(), file: "f".into(), license: String::new(), creator: String::new() };
        lib.upsert(a);
        let mut again = track("a", "x", "nuevo título", 10.0);
        again.added = 999;
        lib.upsert(again);
        assert_eq!(lib.tracks.len(), 1);
        assert_eq!((lib.tracks[0].added, lib.tracks[0].title.as_str()), (100, "nuevo título"));
        assert!(matches!(lib.tracks[0].source, Source::Archive { .. }), "volver a leer el archivo no olvida de dónde vino");
    }

    #[test]
    fn remove_also_cleans_playlists() {
        let mut lib = Library::default();
        lib.upsert(track("a", "x", "1", 1.0));
        lib.upsert(track("b", "x", "2", 1.0));
        lib.playlists.push(Playlist { id: "p".into(), name: "P".into(), tracks: vec!["a".into(), "b".into()], ..Default::default() });
        let gone = lib.remove(&["a".into()]);
        assert_eq!(gone.len(), 1);
        assert_eq!(lib.tracks.len(), 1);
        assert_eq!(lib.playlists[0].tracks, vec!["b".to_owned()]);
    }

    #[test]
    fn groups_duplicates_and_suggests_the_best_copy() {
        let mut lib = Library::default();
        // Mismo archivo en dos carpetas.
        let a1 = track("a1", "Artista", "Uno", 200.0);
        let mut a2 = track("a2", "Artista", "Uno copia", 200.0);
        a2.hash = a1.hash.clone();
        // Misma canción por etiquetas; la FLAC es mejor.
        let b1 = track("b1", "Grupo", "Dos (Remastered)", 180.0);
        let mut b2 = track("b2", "GRUPO", "Dos", 181.5);
        b2.format = "flac".into();
        b2.bitrate = Some(900);
        b2.size = 30_000_000;
        // Mismo audio con etiquetas distintas.
        let c1 = track("c1", "", "Pista 3", 240.0);
        let c2 = track("c2", "Alguien", "Tres", 243.0);
        // Distintas: misma etiqueta pero dura otra cosa.
        let d1 = track("d1", "Grupo", "Dos", 300.0);
        for t in [a1, a2, b1, b2, c1, c2, d1] {
            lib.upsert(t);
        }
        let mut compared = Vec::new();
        let groups = find_duplicates(&lib, |a, b| {
            compared.push((a.id.clone(), b.id.clone()));
            (a.id == "c1" && b.id == "c2") || (a.id == "c2" && b.id == "c1")
        });
        assert_eq!(groups.len(), 3, "{groups:?}");
        let find = |id: &str| groups.iter().find(|g| g.tracks.contains(&id.to_owned())).unwrap();
        assert_eq!(find("a1").reason, Existing::SameFile);
        assert_eq!(find("b1").reason, Existing::SameSong);
        assert_eq!(find("b1").tracks[0], "b2", "se sugiere conservar la FLAC");
        assert_eq!(find("b1").wasted, 5_000_000);
        assert_eq!(find("c1").reason, Existing::SameAudio);
        assert!(groups.iter().all(|g| !g.tracks.contains(&"d1".to_owned())));
        // Nunca se comparan huellas de duraciones muy distintas.
        assert!(compared.iter().all(|(a, b)| (lib.track(a).unwrap().duration - lib.track(b).unwrap().duration).abs() <= SAME_AUDIO_SECONDS));

        // Un grupo marcado como "no son duplicados" ya no sale.
        let mut ignored = find("c1").tracks.clone();
        ignored.sort();
        lib.ignored_duplicates.push(ignored);
        let groups = find_duplicates(&lib, |a, b| (a.id == "c1" && b.id == "c2") || (a.id == "c2" && b.id == "c1"));
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn many_tracks_compare_only_nearby_durations() {
        let mut lib = Library::default();
        for i in 0..3000 {
            lib.upsert(track(&format!("t{i}"), "a", &format!("t{i}"), 60.0 + f64::from(i) * 0.5));
        }
        let mut calls = 0usize;
        let groups = find_duplicates(&lib, |_, _| {
            calls += 1;
            false
        });
        assert!(groups.is_empty());
        assert!(calls < 3000 * 25, "ventana por duración: {calls} comparaciones");
    }

    #[test]
    fn safe_windows_file_names() {
        assert_eq!(safe_file_name("AC/DC: Live?", "x"), "AC_DC_ Live_");
        assert_eq!(safe_file_name("con", "x"), "x");
        assert_eq!(safe_file_name("COM1.mp3", "x"), "x");
        assert_eq!(safe_file_name("Comfort", "x"), "Comfort");
        assert_eq!(safe_file_name("fin con punto. ", "x"), "fin con punto");
        assert_eq!(safe_file_name("   ", "x"), "x");
        assert_eq!(safe_file_name("a\u{0}b\nc", "x"), "a_b_c");
        assert_eq!(safe_file_name(&"á".repeat(300), "x").chars().count(), 100);
    }
}
