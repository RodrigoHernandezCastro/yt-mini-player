//! User interface languages: English, Spanish, Russian, Polish and Japanese.
//!
//! Every text the user can see lives in `src/locales/<code>.json`, one file
//! per language with the same keys. The texts are written for each language
//! (not word-for-word translations): each file follows the conventions of
//! Windows in that language (terms, quotation marks, the `%` sign, polite
//! forms), and plurals use the CLDR categories of the language
//! (`one`/`other` in English and Spanish, `one`/`few`/`many` in Russian and
//! Polish, a single form in Japanese).
//!
//! - Rust code uses [`t!`] for a text right now and [`msg!`] for a
//!   [`Message`] that a page renders later (so it follows a language change).
//! - Pages get the strings they need embedded when they are served
//!   ([`page_strings`]) and format them with the same rules in JavaScript
//!   (`Intl.PluralRules`).
//! - The language follows the Windows display language until the user picks
//!   one (start screen, tray menu or the side bar menu).

use std::{
    collections::HashMap,
    fmt::Display,
    sync::{
        atomic::{AtomicU8, Ordering},
        OnceLock,
    },
};

use serde::{ser::SerializeMap, Serialize, Serializer};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Lang {
    #[default]
    En,
    Es,
    Ru,
    Pl,
    Ja,
}

impl Lang {
    pub const ALL: [Lang; 5] = [Lang::En, Lang::Es, Lang::Ru, Lang::Pl, Lang::Ja];

    pub fn code(self) -> &'static str {
        match self {
            Lang::En => "en",
            Lang::Es => "es",
            Lang::Ru => "ru",
            Lang::Pl => "pl",
            Lang::Ja => "ja",
        }
    }

    /// The name of the language in that language (what pickers show, so
    /// anyone can find their own language whatever the current one is).
    pub fn native_name(self) -> &'static str {
        match self {
            Lang::En => "English",
            Lang::Es => "Español",
            Lang::Ru => "Русский",
            Lang::Pl => "Polski",
            Lang::Ja => "日本語",
        }
    }

    /// A supported language from a BCP 47 tag (`pl-PL`, `es_419`, `ja`).
    pub fn from_tag(tag: &str) -> Option<Lang> {
        let primary = tag.trim().split(['-', '_']).next()?.to_ascii_lowercase();
        Some(match primary.as_str() {
            "en" => Lang::En,
            "es" => Lang::Es,
            "ru" => Lang::Ru,
            "pl" => Lang::Pl,
            "ja" => Lang::Ja,
            _ => return None,
        })
    }

    /// Closest supported language for people whose own language is not
    /// available: Spanish for the other languages of Spain, Russian where it
    /// is the common second language. (Ukrainian deliberately has none: it
    /// falls through to the next Windows language, or English.)
    fn fallback_for(tag: &str) -> Option<Lang> {
        let primary = tag.trim().split(['-', '_']).next()?.to_ascii_lowercase();
        match primary.as_str() {
            "ca" | "gl" | "eu" | "ast" => Some(Lang::Es),
            "be" | "kk" | "ky" | "tg" => Some(Lang::Ru),
            _ => None,
        }
    }

    /// CLDR plural category of a whole number.
    pub fn plural(self, n: u64) -> &'static str {
        let (n10, n100) = (n % 10, n % 100);
        match self {
            Lang::En | Lang::Es => {
                if n == 1 {
                    "one"
                } else {
                    "other"
                }
            }
            Lang::Ru => {
                if n10 == 1 && n100 != 11 {
                    "one"
                } else if (2..=4).contains(&n10) && !(12..=14).contains(&n100) {
                    "few"
                } else {
                    "many"
                }
            }
            Lang::Pl => {
                if n == 1 {
                    "one"
                } else if (2..=4).contains(&n10) && !(12..=14).contains(&n100) {
                    "few"
                } else {
                    "many"
                }
            }
            Lang::Ja => "other",
        }
    }

    fn index(self) -> usize {
        match self {
            Lang::En => 0,
            Lang::Es => 1,
            Lang::Ru => 2,
            Lang::Pl => 3,
            Lang::Ja => 4,
        }
    }

    fn from_index(index: u8) -> Lang {
        Lang::ALL.get(usize::from(index)).copied().unwrap_or_default()
    }
}

/// First supported language in the user's list of Windows display
/// languages (most preferred first); English if there is none.
pub fn pick(preferred: &[String]) -> Lang {
    preferred
        .iter()
        .find_map(|tag| Lang::from_tag(tag))
        .or_else(|| preferred.iter().find_map(|tag| Lang::fallback_for(tag)))
        .unwrap_or_default()
}

/// The Windows display language (or the closest one the app has).
pub fn detect() -> Lang {
    pick(&system_languages())
}

#[cfg(windows)]
fn system_languages() -> Vec<String> {
    use windows_sys::Win32::Globalization::{GetUserDefaultLocaleName, GetUserPreferredUILanguages, MUI_LANGUAGE_NAME};
    let mut languages = Vec::new();
    unsafe {
        let (mut count, mut len) = (0u32, 0u32);
        if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, std::ptr::null_mut(), &mut len) != 0 && len > 0 {
            let mut buffer = vec![0u16; len as usize];
            if GetUserPreferredUILanguages(MUI_LANGUAGE_NAME, &mut count, buffer.as_mut_ptr(), &mut len) != 0 {
                // A double-NUL terminated list of names.
                languages.extend(
                    buffer.split(|&c| c == 0).filter(|s| !s.is_empty()).map(String::from_utf16_lossy),
                );
            }
        }
        // Then the regional format, in case the display language is one the
        // app does not have but the region is.
        let mut locale = [0u16; 85];
        let n = GetUserDefaultLocaleName(locale.as_mut_ptr(), locale.len() as i32);
        if n > 1 {
            languages.push(String::from_utf16_lossy(&locale[..n as usize - 1]));
        }
    }
    languages
}

#[cfg(not(windows))]
fn system_languages() -> Vec<String> {
    ["LC_ALL", "LC_MESSAGES", "LANG"].iter().filter_map(|v| std::env::var(v).ok()).filter(|v| !v.is_empty()).collect()
}

static CURRENT: AtomicU8 = AtomicU8::new(0);

pub fn current() -> Lang {
    Lang::from_index(CURRENT.load(Ordering::Relaxed))
}

pub fn set(lang: Lang) {
    CURRENT.store(lang.index() as u8, Ordering::Relaxed);
}

// ---------------------------------------------------------------------------
// Catalogs
// ---------------------------------------------------------------------------

const SOURCES: [&str; 5] = [
    include_str!("locales/en.json"),
    include_str!("locales/es.json"),
    include_str!("locales/ru.json"),
    include_str!("locales/pl.json"),
    include_str!("locales/ja.json"),
];

#[derive(Debug, Clone, PartialEq)]
enum Entry {
    Text(String),
    /// CLDR category → text.
    Plural(Vec<(String, String)>),
}

type Catalog = HashMap<String, Entry>;

fn parse(source: &str) -> Catalog {
    let Ok(serde_json::Value::Object(map)) = serde_json::from_str(source) else {
        return Catalog::new();
    };
    map.into_iter()
        .filter_map(|(key, value)| {
            let entry = match value {
                serde_json::Value::String(text) => Entry::Text(text),
                serde_json::Value::Object(forms) => Entry::Plural(
                    forms.into_iter().filter_map(|(k, v)| v.as_str().map(|s| (k, s.to_owned()))).collect(),
                ),
                _ => return None,
            };
            Some((key, entry))
        })
        .collect()
}

fn catalogs() -> &'static [Catalog; 5] {
    static CATALOGS: OnceLock<[Catalog; 5]> = OnceLock::new();
    CATALOGS.get_or_init(|| SOURCES.map(parse))
}

/// The entry in `lang`, or in English if that language lacks it.
fn entry(lang: Lang, key: &str) -> Option<&'static Entry> {
    let all = catalogs();
    all[lang.index()].get(key).or_else(|| all[Lang::En.index()].get(key))
}

fn plural_form(lang: Lang, forms: &[(String, String)], count: Option<u64>) -> String {
    let category = count.map_or("other", |n| lang.plural(n));
    let find = |c: &str| forms.iter().find(|(k, _)| k == c).map(|(_, v)| v.clone());
    find(category)
        .or_else(|| find("other"))
        .or_else(|| find("many"))
        .or_else(|| forms.first().map(|(_, v)| v.clone()))
        .unwrap_or_default()
}

/// Digit grouping for counts the way each language writes them.
pub fn format_int(lang: Lang, n: i64) -> String {
    let digits = n.unsigned_abs().to_string();
    // Spanish style books leave four-digit numbers ungrouped (1500).
    let min = if lang == Lang::Es { 5 } else { 4 };
    if digits.len() < min {
        return n.to_string();
    }
    let separator = match lang {
        Lang::En | Lang::Ja => ",",
        Lang::Es => ".",
        // A no-break space, so the number never splits across lines.
        Lang::Ru | Lang::Pl => "\u{a0}",
    };
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push_str(separator);
        }
        out.push(c);
    }
    if n < 0 {
        out.insert(0, '-');
    }
    out
}

/// Replaces `{name}` with the argument of that name. Unknown names stay as
/// they are (a visible sign of a mistake rather than a silent gap).
fn substitute(template: &str, lang: Lang, args: &[(&str, ArgValue)]) -> String {
    if args.is_empty() || !template.contains('{') {
        return template.to_owned();
    }
    let mut out = String::with_capacity(template.len() + 16);
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('}') {
            Some(end) if after[..end].bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') => {
                let name = &after[..end];
                match args.iter().find(|(n, _)| *n == name) {
                    Some((_, ArgValue::Int(v))) => out.push_str(&format_int(lang, *v)),
                    Some((_, ArgValue::Text(v))) => out.push_str(v),
                    None => out.push_str(&rest[start..start + end + 2]),
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push('{');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// The text of `key` in `lang`. A missing key shows the key itself.
pub fn render(lang: Lang, key: &str, args: &[(&str, ArgValue)]) -> String {
    let template = match entry(lang, key) {
        Some(Entry::Text(text)) => text.clone(),
        Some(Entry::Plural(forms)) => {
            let count = args.iter().find(|(n, _)| *n == "count").and_then(|(_, v)| match v {
                ArgValue::Int(n) => u64::try_from(*n).ok(),
                ArgValue::Text(t) => t.parse().ok(),
            });
            plural_form(lang, forms, count)
        }
        None => return key.to_owned(),
    };
    substitute(&template, lang, args)
}

/// Text in the current language.
pub fn text(key: &str) -> String {
    render(current(), key, &[])
}

/// Text in the current language with arguments (see [`t!`]).
pub fn format(key: &str, args: &[(&str, &dyn Display)]) -> String {
    let args: Vec<(&str, ArgValue)> = args.iter().map(|(name, value)| (*name, ArgValue::from_display(*value))).collect();
    render(current(), key, &args)
}

/// Strings for a page: every key that starts with one of `prefixes`, in the
/// current language (English where a language lacks one). Plurals go as
/// objects for the page's own `Intl.PluralRules`.
pub fn page_strings(prefixes: &[&str]) -> serde_json::Map<String, serde_json::Value> {
    strings_for(current(), prefixes)
}

fn strings_for(lang: Lang, prefixes: &[&str]) -> serde_json::Map<String, serde_json::Value> {
    let all = catalogs();
    let mut out = serde_json::Map::new();
    for key in all[Lang::En.index()].keys().filter(|k| prefixes.iter().any(|p| k.starts_with(p))) {
        let value = match entry(lang, key) {
            Some(Entry::Text(text)) => serde_json::Value::String(text.clone()),
            Some(Entry::Plural(forms)) => serde_json::Value::Object(
                forms.iter().map(|(k, v)| (k.clone(), serde_json::Value::String(v.clone()))).collect(),
            ),
            None => continue,
        };
        out.insert(key.clone(), value);
    }
    out
}

/// What pages need to know about languages: the current one and the list
/// for the language picker.
#[derive(Debug, Clone, Serialize)]
pub struct LanguageInfo {
    pub lang: &'static str,
    pub languages: Vec<LanguageChoice>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LanguageChoice {
    pub code: &'static str,
    pub name: &'static str,
}

pub fn language_info() -> LanguageInfo {
    LanguageInfo {
        lang: current().code(),
        languages: Lang::ALL.iter().map(|l| LanguageChoice { code: l.code(), name: l.native_name() }).collect(),
    }
}

// ---------------------------------------------------------------------------
// Messages for pages
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgValue {
    Int(i64),
    Text(String),
}

impl ArgValue {
    fn from_display(value: &dyn Display) -> Self {
        let text = value.to_string();
        match text.parse::<i64>() {
            Ok(n) => ArgValue::Int(n),
            Err(_) => ArgValue::Text(text),
        }
    }
}

macro_rules! arg_from_int {
    ($($t:ty),*) => {$(
        impl From<$t> for ArgValue {
            fn from(n: $t) -> Self {
                ArgValue::Int(i64::try_from(n).unwrap_or(i64::MAX))
            }
        }
    )*};
}
arg_from_int!(u8, u16, u32, u64, usize, i32, i64);

impl From<&str> for ArgValue {
    fn from(s: &str) -> Self {
        ArgValue::Text(s.to_owned())
    }
}

impl From<String> for ArgValue {
    fn from(s: String) -> Self {
        ArgValue::Text(s)
    }
}

impl From<&String> for ArgValue {
    fn from(s: &String) -> Self {
        ArgValue::Text(s.clone())
    }
}

impl Serialize for ArgValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            ArgValue::Int(n) => serializer.serialize_i64(*n),
            ArgValue::Text(t) => serializer.serialize_str(t),
        }
    }
}

/// A text the page shows in its own (current) language: a key and its
/// arguments, serialized as `{"key": "...", "args": {...}}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    pub key: &'static str,
    pub args: Vec<(&'static str, ArgValue)>,
}

impl Message {
    pub fn new(key: &'static str) -> Self {
        Self { key, args: Vec::new() }
    }

    pub fn arg(mut self, name: &'static str, value: impl Into<ArgValue>) -> Self {
        self.args.push((name, value.into()));
        self
    }
}

impl Serialize for Message {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        struct Args<'a>(&'a [(&'static str, ArgValue)]);
        impl Serialize for Args<'_> {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                let mut map = serializer.serialize_map(Some(self.0.len()))?;
                for (name, value) in self.0 {
                    map.serialize_entry(name, value)?;
                }
                map.end()
            }
        }
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("key", self.key)?;
        if !self.args.is_empty() {
            map.serialize_entry("args", &Args(&self.args))?;
        }
        map.end()
    }
}

/// Text in the current language: `t!("tray.quit")`,
/// `t!("app.opacity", value = 70)`. A `count` argument picks the plural form.
macro_rules! t {
    ($key:literal) => {
        $crate::i18n::text($key)
    };
    ($key:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::format($key, &[$((stringify!($name), &$value as &dyn ::std::fmt::Display)),+])
    };
}

/// A [`Message`] for a page: `msg!("music.scan.new", count = 3)`.
macro_rules! msg {
    ($key:literal) => {
        $crate::i18n::Message::new($key)
    };
    ($key:literal, $($name:ident = $value:expr),+ $(,)?) => {
        $crate::i18n::Message::new($key)$(.arg(stringify!($name), $value))+
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(lang: Lang) -> Vec<String> {
        let mut keys: Vec<String> = catalogs()[lang.index()].keys().cloned().collect();
        keys.sort();
        keys
    }

    fn placeholders(text: &str) -> Vec<String> {
        let mut found: Vec<String> = text
            .split('{')
            .skip(1)
            .filter_map(|part| part.split_once('}').map(|(name, _)| name.to_owned()))
            .filter(|name| !name.is_empty() && name.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
            .collect();
        found.sort();
        found.dedup();
        found
    }

    #[test]
    fn every_language_has_every_text_with_the_same_placeholders() {
        let english = keys(Lang::En);
        assert!(english.len() > 100, "the English catalog loads");
        for lang in Lang::ALL {
            assert_eq!(keys(lang), english, "{lang:?} has the same keys as English");
            for key in &english {
                let (en, other) = (&catalogs()[0][key], &catalogs()[lang.index()][key]);
                let texts = |e: &Entry| match e {
                    Entry::Text(t) => vec![t.clone()],
                    Entry::Plural(forms) => forms.iter().map(|(_, v)| v.clone()).collect(),
                };
                let expected: Vec<String> = texts(en).iter().flat_map(|t| placeholders(t)).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
                for text in texts(other) {
                    assert!(!text.trim().is_empty(), "{lang:?} {key} is empty");
                    for name in placeholders(&text) {
                        assert!(expected.contains(&name), "{lang:?} {key} uses {{{name}}}, which English does not have");
                    }
                }
                // Every argument English uses appears in some form.
                let all_other: String = texts(other).join(" ");
                for name in &expected {
                    if name != "count" {
                        assert!(placeholders(&all_other).contains(name), "{lang:?} {key} lacks {{{name}}}");
                    }
                }
                if let Entry::Plural(forms) = other {
                    let needed: &[&str] = match lang {
                        Lang::En | Lang::Es => &["one", "other"],
                        Lang::Ru | Lang::Pl => &["one", "few", "many"],
                        Lang::Ja => &["other"],
                    };
                    for category in needed {
                        assert!(forms.iter().any(|(k, _)| k == category), "{lang:?} {key} lacks the {category} form");
                    }
                }
                assert_eq!(matches!(en, Entry::Plural(_)), matches!(other, Entry::Plural(_)), "{lang:?} {key}: plural in both or neither");
            }
        }
    }

    #[test]
    fn plural_categories() {
        let cat = |l: Lang, n: u64| l.plural(n);
        assert_eq!([0, 1, 2, 5].map(|n| cat(Lang::En, n)), ["other", "one", "other", "other"]);
        assert_eq!([1, 2, 4, 5, 11, 12, 21, 22, 25, 101, 111].map(|n| cat(Lang::Ru, n)),
                   ["one", "few", "few", "many", "many", "many", "one", "few", "many", "one", "many"]);
        assert_eq!([1, 2, 4, 5, 12, 21, 22, 25, 102, 112].map(|n| cat(Lang::Pl, n)),
                   ["one", "few", "few", "many", "many", "many", "few", "many", "few", "many"]);
        assert_eq!([0, 1, 7].map(|n| cat(Lang::Ja, n)), ["other", "other", "other"]);
    }

    #[test]
    fn language_from_windows_list() {
        let list = |tags: &[&str]| tags.iter().map(|t| t.to_string()).collect::<Vec<_>>();
        assert_eq!(pick(&list(&["pl-PL", "en-US"])), Lang::Pl);
        assert_eq!(pick(&list(&["de-DE", "ja-JP"])), Lang::Ja, "first supported one");
        assert_eq!(pick(&list(&["es-419"])), Lang::Es);
        assert_eq!(pick(&list(&["ca-ES"])), Lang::Es, "Catalan falls back to Spanish");
        assert_eq!(pick(&list(&["kk-KZ"])), Lang::Ru);
        assert_eq!(pick(&list(&["uk-UA", "en-GB"])), Lang::En, "Ukrainian never becomes Russian");
        assert_eq!(pick(&list(&["ca-ES", "en-US"])), Lang::En, "a supported language beats a fallback");
        assert_eq!(pick(&list(&["fr-FR"])), Lang::En);
        assert_eq!(pick(&[]), Lang::En);
        assert_eq!(Lang::from_tag("JA_jp"), Some(Lang::Ja));
        assert_eq!(Lang::from_tag(""), None);
    }

    #[test]
    fn formatting() {
        let args = [("count", ArgValue::Int(3)), ("name", ArgValue::Text("Ana".into()))];
        assert_eq!(substitute("{name} has {count} {missing}", Lang::En, &args), "Ana has 3 {missing}");
        assert_eq!(substitute("no braces", Lang::En, &args), "no braces");
        assert_eq!(substitute("odd { brace } {", Lang::En, &args), "odd { brace } {");
        assert_eq!(format_int(Lang::En, 1234567), "1,234,567");
        assert_eq!(format_int(Lang::Es, 1500), "1500");
        assert_eq!(format_int(Lang::Es, 15000), "15.000");
        assert_eq!(format_int(Lang::Ru, 2500), "2\u{a0}500");
        assert_eq!(format_int(Lang::Ja, -1000), "-1,000");
        let plural = vec![("one".to_owned(), "{count} song".to_owned()), ("other".to_owned(), "{count} songs".to_owned())];
        assert_eq!(plural_form(Lang::En, &plural, Some(1)), "{count} song");
        assert_eq!(plural_form(Lang::Ru, &plural, Some(5)), "{count} songs", "a missing category uses other");
    }

    #[test]
    fn messages_render_and_serialize() {
        let m = msg!("music.scan.new", count = 3usize);
        assert_eq!(serde_json::to_value(&m).unwrap(), serde_json::json!({"key": "music.scan.new", "args": {"count": 3}}));
        assert_eq!(serde_json::to_value(msg!("music.scan.up_to_date")).unwrap(), serde_json::json!({"key": "music.scan.up_to_date"}));
        assert_eq!(render(Lang::En, "no.such.key", &[]), "no.such.key");
        assert!(!render(Lang::Ja, "tray.quit", &[]).is_ascii());
        let strings = strings_for(Lang::Pl, &["lock."]);
        assert!(!strings.is_empty() && strings.keys().all(|k| k.starts_with("lock.")));
        assert_ne!(strings, strings_for(Lang::En, &["lock."]), "Polish texts, not English");
    }
}

#[cfg(test)]
#[path = "tests/i18n_cases.rs"]
mod cases;
