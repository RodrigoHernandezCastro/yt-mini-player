//! Music library: songs, playlists and finding duplicates.
//!
//! Everything here is pure (no disk or network) so it can be tested
//! thoroughly.
//!
//! A song counts as **the same** as another if:
//! 1. **Same file**: the content is identical (quick content hash, see
//!    `scan::content_hash`).
//! 2. **Same song**: artist and title match once normalized (case, accents,
//!    Polish letters, "ё", full-width Latin letters, "feat.",
//!    "(Remastered)"...) and the durations differ by less than 3 s.
//! 3. **Same audio**: the acoustic fingerprint (Chromaprint) matches, even if
//!    the tags or the format differ (an MP3 and a FLAC of the same track).

use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
};

use serde::{Deserialize, Serialize};

/// Largest duration difference for "same song" by tags.
pub const SAME_SONG_SECONDS: f64 = 3.0;
/// Fingerprints are only compared between songs of similar duration.
pub const SAME_AUDIO_SECONDS: f64 = 10.0;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Source {
    /// A file of the user's.
    #[default]
    Local,
    /// Downloaded from the Internet Archive (with its license and authors).
    Archive { item: String, file: String, license: String, creator: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Track {
    /// Stable for a path (hash of the path).
    pub id: String,
    pub path: PathBuf,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub track_no: Option<u32>,
    pub year: Option<u32>,
    /// Seconds (0 if unknown).
    pub duration: f64,
    pub size: u64,
    /// The file's modification date (Unix seconds).
    pub modified: u64,
    /// Lowercase extension (`mp3`, `flac`...).
    pub format: String,
    /// kbps.
    pub bitrate: Option<u32>,
    /// Content hash (`scan::content_hash`).
    pub hash: String,
    /// Cover in the cache (file name).
    pub cover: Option<String>,
    pub source: Source,
    /// When it entered the library (Unix seconds).
    pub added: u64,
}

impl Track {
    pub fn lossless(&self) -> bool {
        matches!(self.format.as_str(), "flac" | "wav")
    }

    /// To pick which duplicate to keep: lossless > more kbps > bigger.
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
    /// If it was created by downloading an album: the album's Archive id.
    pub origin: Option<String>,
}

/// An Archive file that turned out to be a song already there (it was
/// downloaded, compared and discarded): it is not downloaded again.
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
    /// MP3 when there is one: it takes much less space.
    #[default]
    Compact,
    /// The lossless original (FLAC) if there is one.
    Best,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct Library {
    pub version: u32,
    /// Imported folders (checked again when opening).
    pub folders: Vec<PathBuf>,
    pub tracks: Vec<Track>,
    pub playlists: Vec<Playlist>,
    pub quality: Quality,
    /// Duplicate groups the user decided to keep (sorted ids).
    pub ignored_duplicates: Vec<Vec<String>>,
    pub aliases: Vec<Alias>,
}

/// Normalizes a text for comparing: lowercase, no accents or punctuation,
/// no "(Remastered 2011)", "[Official Audio]", "feat. X" or track numbers.
pub fn normalize(text: &str) -> String {
    // Full-width Latin letters and digits (common in Japanese tags) first,
    // so "ＡＢＣ" and "ABC" compare equal.
    let mut s: String = text.chars().map(half_width).collect::<String>().to_lowercase();
    // What goes in brackets is versions or decoration ("（Live）" too).
    let mut out = String::with_capacity(s.len());
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '(' | '[' | '{' | '【' | '「' => depth += 1,
            ')' | ']' | '}' | '】' | '」' => depth = (depth - 1).max(0),
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
    // "01 - Title" / "1-02 Title": leading track numbers go.
    let skip = words.iter().take_while(|w| w.len() <= 3 && w.chars().all(|c| c.is_ascii_digit())).count();
    let words = if skip < words.len() { &words[skip..] } else { &words[..] };
    let words: Vec<&str> = words.iter().copied().filter(|w| !matches!(*w, "the" | "and" | "y")).collect();
    words.join(" ")
}

/// Full-width ASCII (U+FF01–FF5E) and the ideographic space to plain ASCII.
fn half_width(c: char) -> char {
    match c {
        '\u{3000}' => ' ',
        '\u{ff01}'..='\u{ff5e}' => char::from_u32(c as u32 - 0xfee0).unwrap_or(c),
        c => c,
    }
}

fn fold(c: char) -> char {
    match c {
        'á' | 'à' | 'ä' | 'â' | 'ã' | 'å' | 'ą' => 'a',
        'é' | 'è' | 'ë' | 'ê' | 'ę' => 'e',
        'í' | 'ì' | 'ï' | 'î' => 'i',
        'ó' | 'ò' | 'ö' | 'ô' | 'õ' | 'ø' => 'o',
        'ú' | 'ù' | 'ü' | 'û' => 'u',
        'ñ' | 'ń' => 'n',
        'ç' | 'ć' => 'c',
        'ł' => 'l',
        'ś' => 's',
        'ź' | 'ż' => 'z',
        // Russian often writes "ё" as "е".
        'ё' => 'е',
        '&' | '+' | '・' => ' ',
        c => c,
    }
}

/// "Same song" key: normalized artist + title. Without a title there is no
/// key (no comparing by tags).
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

/// A song about to be added (before downloading it).
#[derive(Debug, Clone, Default)]
pub struct Candidate<'a> {
    pub hash: Option<&'a str>,
    pub archive: Option<(&'a str, &'a str)>,
    pub artist: &'a str,
    pub title: &'a str,
    pub duration: f64,
}

/// Why it need not be downloaded. (The names say what is "the same"; they
/// go to the page as they are.)
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Existing {
    /// That very file was already downloaded.
    SameDownload,
    /// There is an identical file.
    SameFile,
    /// Same song by tags and duration.
    SameSong,
    /// Same audio by acoustic fingerprint (after downloading).
    SameAudio,
}

impl Existing {
    /// Why, for the page.
    pub fn message(self) -> crate::i18n::Message {
        match self {
            Existing::SameDownload => msg!("music.existing.same_download"),
            Existing::SameFile => msg!("music.existing.same_file"),
            Existing::SameSong => msg!("music.existing.same_song"),
            Existing::SameAudio => msg!("music.existing.same_audio"),
        }
    }
}

impl Library {
    pub fn track(&self, id: &str) -> Option<&Track> {
        self.tracks.iter().find(|t| t.id == id)
    }

    /// Adds or replaces (same id = same path). Keeps when it was added.
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

    /// Removes songs (from the playlists too). Returns the removed ones.
    pub fn remove(&mut self, ids: &[String]) -> Vec<Track> {
        let (gone, keep): (Vec<Track>, Vec<Track>) = std::mem::take(&mut self.tracks).into_iter().partition(|t| ids.contains(&t.id));
        self.tracks = keep;
        for playlist in &mut self.playlists {
            playlist.tracks.retain(|id| !ids.contains(id));
        }
        self.aliases.retain(|a| !ids.contains(&a.track));
        gone
    }

    /// Is it already there? So what the user has is not downloaded.
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

/// A group of repeated songs.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DuplicateGroup {
    /// The group's strongest reason.
    pub reason: Existing,
    /// From best to worst quality; the first is the one suggested to keep.
    pub tracks: Vec<String>,
    /// Bytes freed by removing all but the first.
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

/// Finds repeated songs. `same_audio(a, b)` compares fingerprints (it can
/// be slow: only called for similar durations).
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

    // 1. Same content.
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
    // 2. Same tags and similar duration.
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
    // 3. Acoustic fingerprint, only between similar durations (a sliding
    //    window over the list sorted by duration).
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

/// A file name that is safe on Windows (no forbidden characters, reserved
/// names or trailing dots or spaces) and of reasonable length.
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
            path: PathBuf::from(format!("C:/Music/{id}.mp3")),
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
        assert_eq!(normalize("Señor feat. Someone"), "senor");
        assert_eq!(normalize("The Beatles"), "beatles");
        assert_eq!(normalize("Simon & Garfunkel"), normalize("Simon and Garfunkel"));
        assert_eq!(normalize("1-02 Para"), "para");
        assert_eq!(normalize("1999"), "1999", "a title that is a number is not erased");
        assert_eq!(normalize("  "), "");
        // Polish, Russian and Japanese tags.
        assert_eq!(normalize("Żółć Łąka"), normalize("zolc laka"));
        assert_eq!(normalize("Ёлка"), normalize("елка"));
        assert_eq!(normalize("ＹＯＡＳＯＢＩ　夜に駆ける（Ｌｉｖｅ）"), normalize("YOASOBI 夜に駆ける"));
        assert_eq!(normalize("夜に駆ける【MV】"), "夜に駆ける");
        assert_eq!(meta_key("Ronan Dec.", "Para"), meta_key("RONAN DEC", "para"));
        assert_ne!(meta_key("Ronan Dec.", "Para"), meta_key("Other", "Para"));
        assert_eq!(meta_key("x", ""), None);
    }

    #[test]
    fn finds_existing_before_downloading() {
        let mut lib = Library::default();
        let mut a = track("a", "Ronan Dec.", "Supernova", 263.7);
        a.source = Source::Archive { item: "TAM033".into(), file: "1-Supernova.mp3".into(), license: "by-sa".into(), creator: "Ronan Dec.".into() };
        lib.upsert(a);
        lib.upsert(track("b", "Other", "Song", 200.0));

        let same_download = Candidate { archive: Some(("TAM033", "1-Supernova.mp3")), ..Default::default() };
        assert_eq!(lib.find_existing(&same_download).map(|(e, t)| (e, t.id.as_str())), Some((Existing::SameDownload, "a")));
        let same_file = Candidate { hash: Some("hash-b"), ..Default::default() };
        assert_eq!(lib.find_existing(&same_file).map(|(e, _)| e), Some(Existing::SameFile));
        let same_song = Candidate { artist: "ronan dec", title: "Supernova (Original Mix)", duration: 265.0, ..Default::default() };
        assert_eq!(lib.find_existing(&same_song).map(|(e, _)| e), Some(Existing::SameSong));
        let longer = Candidate { artist: "ronan dec", title: "Supernova", duration: 300.0, ..Default::default() };
        assert_eq!(lib.find_existing(&longer), None, "another version (different length)");
        let unknown_length = Candidate { artist: "ronan dec", title: "Supernova", duration: 0.0, ..Default::default() };
        assert_eq!(lib.find_existing(&unknown_length), None, "without a duration it takes no chances");
    }

    #[test]
    fn upsert_keeps_added_date_and_source() {
        let mut lib = Library::default();
        let mut a = track("a", "x", "y", 10.0);
        a.added = 100;
        a.source = Source::Archive { item: "i".into(), file: "f".into(), license: String::new(), creator: String::new() };
        lib.upsert(a);
        let mut again = track("a", "x", "new title", 10.0);
        again.added = 999;
        lib.upsert(again);
        assert_eq!(lib.tracks.len(), 1);
        assert_eq!((lib.tracks[0].added, lib.tracks[0].title.as_str()), (100, "new title"));
        assert!(matches!(lib.tracks[0].source, Source::Archive { .. }), "reading the file again does not forget where it came from");
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
        // The same file in two folders.
        let a1 = track("a1", "Artist", "One", 200.0);
        let mut a2 = track("a2", "Artist", "One copy", 200.0);
        a2.hash = a1.hash.clone();
        // Same song by tags; the FLAC is better.
        let b1 = track("b1", "Band", "Two (Remastered)", 180.0);
        let mut b2 = track("b2", "BAND", "Two", 181.5);
        b2.format = "flac".into();
        b2.bitrate = Some(900);
        b2.size = 30_000_000;
        // Same audio with different tags.
        let c1 = track("c1", "", "Track 3", 240.0);
        let c2 = track("c2", "Someone", "Three", 243.0);
        // Different: same tags but another length.
        let d1 = track("d1", "Band", "Two", 300.0);
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
        assert_eq!(find("b1").tracks[0], "b2", "keeping the FLAC is suggested");
        assert_eq!(find("b1").wasted, 5_000_000);
        assert_eq!(find("c1").reason, Existing::SameAudio);
        assert!(groups.iter().all(|g| !g.tracks.contains(&"d1".to_owned())));
        // Fingerprints of very different durations are never compared.
        assert!(compared.iter().all(|(a, b)| (lib.track(a).unwrap().duration - lib.track(b).unwrap().duration).abs() <= SAME_AUDIO_SECONDS));

        // A group marked "not duplicates" no longer shows.
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
        assert!(calls < 3000 * 25, "a duration window: {calls} comparisons");
    }

    #[test]
    fn safe_windows_file_names() {
        assert_eq!(safe_file_name("AC/DC: Live?", "x"), "AC_DC_ Live_");
        assert_eq!(safe_file_name("con", "x"), "x");
        assert_eq!(safe_file_name("COM1.mp3", "x"), "x");
        assert_eq!(safe_file_name("Comfort", "x"), "Comfort");
        assert_eq!(safe_file_name("ends with a dot. ", "x"), "ends with a dot");
        assert_eq!(safe_file_name("   ", "x"), "x");
        assert_eq!(safe_file_name("a\u{0}b\nc", "x"), "a_b_c");
        assert_eq!(safe_file_name(&"á".repeat(300), "x").chars().count(), 100);
    }
}
