//! A small diary of what the app does, to find out why it closed.
//!
//! `session.log` (in the app's folder) gets one line per notable step:
//! window sharing starting and stopping, the sending page, the screencast,
//! the sound capture, a guest's connection… never what anyone watches,
//! names, codes or addresses. On the next start the previous diary becomes
//! `previous-session.log`.
//!
//! If the app crashes, the last lines say where: a Rust panic is written
//! with its file and line, a Windows exception with its code and the
//! module it happened in, and a WebView2 process that dies with its kind
//! and reason.
//!
//! The sound capture is native code that talks to the audio service: if a
//! diary ends while it was starting, the next session does not try it again
//! (the window is still shared, without sound) — see [`sound_crashed`].

use std::{
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
    sync::Mutex,
    time::Instant,
};

const LOG: &str = "session.log";
const PREVIOUS: &str = "previous-session.log";
/// Lines a diary keeps at most (it is not a recording).
const MAX_LINES: usize = 2000;
/// Written once the sound capture runs.
pub const SOUND_STARTED: &str = "sound: started";

/// Written right before the sound capture is requested (with the version:
/// a crash in an older version does not turn it off in a newer one).
pub fn sound_starting() -> String {
    format!("sound: starting (v{})", env!("CARGO_PKG_VERSION"))
}

struct Diary {
    file: Option<File>,
    lines: usize,
    start: Instant,
}

static DIARY: Mutex<Option<Diary>> = Mutex::new(None);
/// The last lines of the previous diary (read once at start).
static PREVIOUS_TAIL: Mutex<Vec<String>> = Mutex::new(Vec::new());

/// Opens a new diary in `dir` (keeping the last one as the previous).
pub fn start(dir: &Path, webview_version: &str) {
    let log = dir.join(LOG);
    let previous = dir.join(PREVIOUS);
    let tail: Vec<String> = fs::read_to_string(&log)
        .map(|text| text.lines().rev().take(40).map(str::to_owned).collect::<Vec<_>>().into_iter().rev().collect())
        .unwrap_or_default();
    if let Ok(mut previous) = PREVIOUS_TAIL.lock() {
        *previous = tail;
    }
    let _ = fs::rename(&log, &previous);
    let file = OpenOptions::new().create(true).write(true).truncate(true).open(&log).ok();
    if let Ok(mut diary) = DIARY.lock() {
        *diary = Some(Diary { file, lines: 0, start: Instant::now() });
    }
    install_crash_handlers();
    note(format!("YT Mini Player {} on {} (WebView2 {webview_version})", env!("CARGO_PKG_VERSION"), windows_version()));
}

/// One line in the diary.
pub fn note(text: impl AsRef<str>) {
    let Ok(mut guard) = DIARY.try_lock() else { return };
    let Some(diary) = guard.as_mut() else { return };
    if diary.lines >= MAX_LINES {
        return;
    }
    diary.lines += 1;
    let ms = diary.start.elapsed().as_millis();
    let thread = std::thread::current();
    let name = thread.name().unwrap_or("?");
    if let Some(file) = diary.file.as_mut() {
        let _ = writeln!(file, "{:>8}.{:03} [{name}] {}", ms / 1000, ms % 1000, text.as_ref());
        let _ = file.flush();
    }
}

/// The app is quitting normally.
pub fn finish() {
    note("exit");
}

/// The previous session ended badly: a crash, a WebView2 process that
/// died, or it stopped in the middle of sharing or watching a window.
pub fn crashed_last_time() -> bool {
    let tail = PREVIOUS_TAIL.lock().map(|t| t.clone()).unwrap_or_default();
    if tail.is_empty() || tail.last().is_some_and(|l| l.ends_with(" exit")) {
        return false;
    }
    let failed = tail.iter().any(|l| l.contains("] CRASH") || l.contains("] PANIC") || l.contains("process failed"));
    let busy = tail
        .iter()
        .rev()
        .find(|l| l.contains("] share: ") || l.contains("] watch: showing") || l.contains("] watch: back"))
        .is_some_and(|l| l.ends_with("share: on") || l.contains("watch: showing"));
    failed || busy
}

/// The previous session ended (crashed) while starting the sound capture
/// (between "starting" and "started").
pub fn sound_crashed() -> bool {
    let tail = PREVIOUS_TAIL.lock().map(|t| t.clone()).unwrap_or_default();
    let last_sound = tail.iter().rev().find(|l| l.contains("] sound: "));
    let ended_cleanly = tail.last().is_some_and(|l| l.ends_with(" exit"));
    // From "starting" (this version) until "started": the request, the
    // answer, setting it up.
    let started_here = tail.iter().any(|l| l.ends_with(&sound_starting()));
    let starting = ["sound: starting", "sound: requested", "sound: activated", "sound: initialized"];
    !ended_cleanly && started_here && last_sound.is_some_and(|l| starting.iter().any(|s| l.contains(s)))
}

/// How often the diary gets the screencast's numbers.
const REPORT_EVERY: std::time::Duration = std::time::Duration::from_secs(10);

/// The screencast's pictures that arrived and went on since the last report.
#[derive(Debug, Clone, Copy)]
pub struct PictureMeter {
    since: std::time::Instant,
    got: u32,
    sent: u32,
    bytes: u64,
    size: (u32, u32),
}

impl PictureMeter {
    pub fn new(now: std::time::Instant) -> Self {
        Self { since: now, got: 0, sent: 0, bytes: 0, size: (0, 0) }
    }

    /// One picture (`base64` long), sent on or skipped. Returns the
    /// diary's line when it is time for one.
    pub fn picture(&mut self, now: std::time::Instant, base64: usize, size: (u32, u32), sent: bool) -> Option<String> {
        self.got += 1;
        self.sent += u32::from(sent);
        self.bytes += base64 as u64 * 3 / 4;
        self.size = size;
        let elapsed = now.duration_since(self.since);
        if elapsed < REPORT_EVERY {
            return None;
        }
        let seconds = elapsed.as_secs_f64();
        let line = format!(
            "screencast: {:.1} pictures/s in, {:.1} out, {} KB each, {}x{}",
            f64::from(self.got) / seconds,
            f64::from(self.sent) / seconds,
            self.bytes / u64::from(self.got.max(1)) / 1024,
            self.size.0,
            self.size.1
        );
        *self = Self::new(now);
        Some(line)
    }
}


// --- Statistics from the sharing pages -----------------------------------------
//
// The pages (cast.html for the host, stream.html for a guest) send their
// numbers every few seconds; only known fields reach the diary, as numbers
// or short plain words (never names, codes or addresses).

fn number(object: &serde_json::Value, key: &str) -> Option<f64> {
    object.get(key)?.as_f64().filter(|v| v.is_finite() && (0.0..1e9).contains(v))
}

fn word(object: &serde_json::Value, key: &str) -> Option<String> {
    let text: String = object
        .get(key)?
        .as_str()?
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.' | '/' | '(' | ')'))
        .take(48)
        .collect();
    (!text.trim().is_empty()).then(|| text.trim().to_owned())
}

/// "29.5 fps 1280x720, 2310 kbps" and so on, from the fields that came.
fn video_part(o: &serde_json::Value, parts: &mut Vec<String>) {
    if let Some(fps) = number(o, "fps") {
        let size = match (number(o, "w"), number(o, "h")) {
            (Some(w), Some(h)) => format!(" {w:.0}x{h:.0}"),
            _ => String::new(),
        };
        parts.push(format!("{fps:.1} fps{size}"));
    }
    if let Some(kbps) = number(o, "kbps") {
        parts.push(format!("{kbps:.0} kbps"));
    }
}

fn sound_part(o: &serde_json::Value, parts: &mut Vec<String>) {
    let mut sound = Vec::new();
    if let Some(kbps) = number(o, "audioKbps") {
        sound.push(format!("{kbps:.0} kbps"));
    }
    if let Some(level) = number(o, "level") {
        sound.push(format!("level {level:.2}"));
    }
    if !sound.is_empty() {
        parts.push(format!("sound {}", sound.join(" ")));
    }
}

/// The host's page: one line for the page, one per connection.
pub fn cast_stats(json: &str) -> Vec<String> {
    let Ok(stats) = serde_json::from_str::<serde_json::Value>(json) else { return Vec::new() };
    let mut lines = Vec::new();
    let mut page = Vec::new();
    if let Some(pictures) = number(&stats, "in") {
        page.push(format!("{pictures:.1} pictures/s in"));
    }
    if let Some(out) = number(&stats, "out") {
        page.push(format!("{out:.1} to the tracks"));
    }
    if let Some(ms) = number(&stats, "decodeMs") {
        page.push(format!("{ms:.1} ms to decode"));
    }
    if let Some(sound) = number(&stats, "sound") {
        page.push(format!("{sound:.0} sound pieces/s"));
    }
    if !page.is_empty() {
        lines.push(format!("cast: {}", page.join(", ")));
    }
    let peers = stats.get("peers").and_then(|p| p.as_array()).map(Vec::as_slice).unwrap_or_default();
    for peer in peers.iter().take(8) {
        let Some(id) = peer.get("id").and_then(|i| i.as_str()) else { continue };
        if !crate::together::protocol::is_member_id(id) {
            continue;
        }
        let mut parts = Vec::new();
        if let Some(state) = word(peer, "state") {
            parts.push(state);
        }
        video_part(peer, &mut parts);
        if let Some(codec) = word(peer, "codec") {
            parts.push(match word(peer, "enc") {
                Some(encoder) => format!("{codec} ({encoder})"),
                None => codec,
            });
        }
        if let Some(limit) = word(peer, "limit").filter(|l| l != "none") {
            parts.push(format!("limited by {limit}"));
        }
        if let Some(rtt) = number(peer, "rtt") {
            parts.push(format!("rtt {rtt:.0} ms"));
        }
        if let Some(lost) = number(peer, "lost") {
            parts.push(format!("{lost:.1}% lost"));
        }
        if let Some(route) = word(peer, "route") {
            parts.push(format!("via {route}"));
        }
        sound_part(peer, &mut parts);
        lines.push(format!("cast {id}: {}", parts.join(", ")));
    }
    lines
}

/// A guest's page: one line.
pub fn stream_stats(json: &str) -> Option<String> {
    let stats = serde_json::from_str::<serde_json::Value>(json).ok().filter(|v| v.is_object())?;
    let mut parts = Vec::new();
    video_part(&stats, &mut parts);
    if let Some(dropped) = number(&stats, "dropped") {
        parts.push(format!("{dropped:.0} dropped"));
    }
    if let Some(freezes) = number(&stats, "freezes") {
        parts.push(format!("{freezes:.0} freezes"));
    }
    if let Some(jitter) = number(&stats, "jitter") {
        parts.push(format!("jitter {jitter:.0} ms"));
    }
    if let Some(decoder) = word(&stats, "dec") {
        parts.push(format!("decoder {decoder}"));
    }
    if let Some(rtt) = number(&stats, "rtt") {
        parts.push(format!("rtt {rtt:.0} ms"));
    }
    if let Some(route) = word(&stats, "route") {
        parts.push(format!("via {route}"));
    }
    sound_part(&stats, &mut parts);
    if let Some(concealed) = number(&stats, "concealed") {
        parts.push(format!("{concealed:.1}% sound filled in"));
    }
    if stats.get("muted").and_then(|m| m.as_bool()) == Some(true) {
        parts.push("MUTED (waiting for a click)".into());
    }
    if stats.get("paused").and_then(|m| m.as_bool()) == Some(true) {
        parts.push("paused".into());
    }
    (!parts.is_empty()).then(|| format!("watch: {}", parts.join(", ")))
}

/// What a service page's self-check found (ui.js, "page-check:<json>"):
/// which of the pieces the app relies on are there. A closed set of values:
/// anything else is refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PageCheck {
    /// YouTube's watch page.
    YouTubeWatch {
        /// How the player was found: `main` (its usual name), `fallback`
        /// (another of its names), `video` (only as the box around the
        /// video) or `none`.
        player: &'static str,
        api: bool,
        video: bool,
        fullscreen_button: bool,
    },
    /// Google refused the sign-in (its "rejected" page).
    GoogleSignInRefused,
}

impl PageCheck {
    pub fn from_report(json: &str) -> Option<Self> {
        if json.len() > 512 {
            return None;
        }
        let value: serde_json::Value = serde_json::from_str(json).ok()?;
        let text = |key: &str| value.get(key).and_then(|v| v.as_str());
        let flag = |key: &str| value.get(key).and_then(|v| v.as_bool());
        match (text("site")?, text("page")?) {
            ("youtube", "watch") => Some(Self::YouTubeWatch {
                player: ["main", "fallback", "video", "none"].into_iter().find(|p| Some(*p) == text("player"))?,
                api: flag("api")?,
                video: flag("video")?,
                fullscreen_button: flag("fullscreenButton")?,
            }),
            ("google", "sign-in-refused") => Some(Self::GoogleSignInRefused),
            _ => None,
        }
    }

    /// The diary's line.
    pub fn line(&self) -> String {
        let yes = |b: bool| if b { "yes" } else { "MISSING" };
        match self {
            Self::YouTubeWatch { player, api, video, fullscreen_button } => format!(
                "page check: youtube watch: player {}, api {}, video {}, full-screen button {}",
                if *player == "main" { "found" } else { player },
                yes(*api),
                yes(*video),
                yes(*fullscreen_button)
            ),
            Self::GoogleSignInRefused => "page check: Google refused the sign-in (its \"rejected\" page)".into(),
        }
    }

    /// Something the app relies on is missing: which service (for the
    /// warning, given once per session and service).
    pub fn broken(&self) -> Option<&'static str> {
        match self {
            Self::YouTubeWatch { player, api, video, fullscreen_button } => {
                (*player != "main" || !api || !video || !fullscreen_button).then_some("YouTube")
            }
            Self::GoogleSignInRefused => Some("Google"),
        }
    }
}

fn install_crash_handlers() {
    let default = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_owned())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_default();
        let place = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
        note(format!("PANIC at {place}: {message}"));
        default(info);
    }));
    #[cfg(windows)]
    win::install();
}

#[cfg(windows)]
fn windows_version() -> String {
    // The real build number (GetVersionEx lies without a manifest entry).
    let key = r"SOFTWARE\Microsoft\Windows NT\CurrentVersion";
    let read = |name: &str| win::registry_string(key, name).unwrap_or_default();
    format!("Windows build {}.{} ({})", read("CurrentBuild"), win::registry_dword(key, "UBR").unwrap_or(0), read("DisplayVersion"))
}

#[cfg(not(windows))]
fn windows_version() -> String {
    std::env::consts::OS.to_owned()
}

#[cfg(windows)]
mod win {
    use std::ffi::c_void;

    use windows_sys::Win32::{
        Foundation::ERROR_SUCCESS,
        System::{
            Diagnostics::Debug::{SetUnhandledExceptionFilter, EXCEPTION_POINTERS},
            LibraryLoader::{
                GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
                GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            },
            Registry::{RegGetValueW, HKEY_LOCAL_MACHINE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ},
        },
    };

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    pub fn registry_string(key: &str, name: &str) -> Option<String> {
        let (key, name) = (wide(key), wide(name));
        let mut buffer = [0u16; 128];
        let mut size = (buffer.len() * 2) as u32;
        let status = unsafe {
            RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), name.as_ptr(), RRF_RT_REG_SZ, std::ptr::null_mut(), buffer.as_mut_ptr().cast(), &mut size)
        };
        (status == ERROR_SUCCESS).then(|| {
            let len = (size as usize / 2).saturating_sub(1).min(buffer.len());
            String::from_utf16_lossy(&buffer[..len])
        })
    }

    pub fn registry_dword(key: &str, name: &str) -> Option<u32> {
        let (key, name) = (wide(key), wide(name));
        let mut value = 0u32;
        let mut size = 4u32;
        let status = unsafe {
            RegGetValueW(HKEY_LOCAL_MACHINE, key.as_ptr(), name.as_ptr(), RRF_RT_REG_DWORD, std::ptr::null_mut(), (&mut value as *mut u32).cast(), &mut size)
        };
        (status == ERROR_SUCCESS).then_some(value)
    }

    /// The module (exe or DLL) an address belongs to, and the offset in it.
    fn module_of(address: *const c_void) -> String {
        unsafe {
            let mut module = std::ptr::null_mut();
            let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
            if GetModuleHandleExW(flags, address.cast(), &mut module) == 0 {
                return format!("{address:p}");
            }
            let mut name = [0u16; 260];
            let len = GetModuleFileNameW(module, name.as_mut_ptr(), name.len() as u32) as usize;
            let path = String::from_utf16_lossy(&name[..len.min(name.len())]);
            let file = path.rsplit('\\').next().unwrap_or_default().to_owned();
            format!("{file}+0x{:x}", address as usize - module as usize)
        }
    }

    unsafe extern "system" fn on_exception(info: *const EXCEPTION_POINTERS) -> i32 {
        if let Some(record) = unsafe { info.as_ref() }.and_then(|i| unsafe { i.ExceptionRecord.as_ref() }) {
            super::note(format!(
                "CRASH: exception 0x{:08x} at {}",
                record.ExceptionCode as u32,
                module_of(record.ExceptionAddress)
            ));
        }
        // EXCEPTION_CONTINUE_SEARCH: Windows goes on as usual.
        0
    }

    pub fn install() {
        unsafe {
            SetUnhandledExceptionFilter(Some(on_exception));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_checks_are_read_strictly_and_tell_what_is_missing() {
        let healthy = r#"{"site":"youtube","page":"watch","player":"main","api":true,"video":true,"fullscreenButton":true}"#;
        let check = PageCheck::from_report(healthy).unwrap();
        assert_eq!(check.line(), "page check: youtube watch: player found, api yes, video yes, full-screen button yes");
        assert_eq!(check.broken(), None);
        let renamed = r#"{"site":"youtube","page":"watch","player":"video","api":false,"video":true,"fullscreenButton":false}"#;
        let check = PageCheck::from_report(renamed).unwrap();
        assert_eq!(check.line(), "page check: youtube watch: player video, api MISSING, video yes, full-screen button MISSING");
        assert_eq!(check.broken(), Some("YouTube"));
        let no_button = healthy.replace(r#""fullscreenButton":true"#, r#""fullscreenButton":false"#);
        assert_eq!(PageCheck::from_report(&no_button).unwrap().broken(), Some("YouTube"));
        let refused = PageCheck::from_report(r#"{"site":"google","page":"sign-in-refused"}"#).unwrap();
        assert_eq!(refused.broken(), Some("Google"));
        for bad in [
            r#"{"site":"youtube","page":"watch","player":"<b>","api":true,"video":true,"fullscreenButton":true}"#,
            r#"{"site":"youtube","page":"watch","player":"main","api":"yes","video":true,"fullscreenButton":true}"#,
            r#"{"site":"youtube","page":"watch","player":"main"}"#,
            r#"{"site":"vimeo","page":"watch"}"#,
            r#"{"site":"google","page":"anything"}"#,
            "not json",
        ] {
            assert_eq!(PageCheck::from_report(bad), None, "{bad}");
        }
        let huge = format!(r#"{{"site":"google","page":"sign-in-refused","x":"{}"}}"#, "y".repeat(600));
        assert_eq!(PageCheck::from_report(&huge), None, "too long");
    }

    #[test]
    fn statistics_become_plain_lines() {
        let cast = r#"{"in":29.6,"out":29.1,"decodeMs":5.12,"sound":50,"peers":[
            {"id":"g1","state":"connected","fps":28.94,"w":1280,"h":720,"kbps":2310.4,"codec":"H264",
             "enc":"MediaFoundationVideoEncodeAccelerator","limit":"none","rtt":45.2,"lost":0.25,"route":"srflx-host",
             "audioKbps":96,"level":0.214},
            {"id":"../../x","fps":1},
            {"id":"g2","state":"connecting<script>","limit":"bandwidth","codec":"VP8"}]}"#;
        let lines = cast_stats(cast);
        assert_eq!(lines[0], "cast: 29.6 pictures/s in, 29.1 to the tracks, 5.1 ms to decode, 50 sound pieces/s");
        assert_eq!(
            lines[1],
            "cast g1: connected, 28.9 fps 1280x720, 2310 kbps, H264 (MediaFoundationVideoEncodeAccelerator), \
             rtt 45 ms, 0.2% lost, via srflx-host, sound 96 kbps level 0.21"
        );
        assert_eq!(lines[2], "cast g2: connectingscript, VP8, limited by bandwidth", "no markup, odd ids dropped");
        assert_eq!(lines.len(), 3);
        assert!(cast_stats("not json").is_empty() && cast_stats("[]").is_empty());

        let watch = r#"{"fps":24,"w":960,"h":540,"kbps":1800,"dropped":3,"freezes":1,"jitter":12.4,
            "dec":"D3D11VideoDecoder","rtt":40,"audioKbps":90,"level":0,"concealed":2.5,"muted":true,"paused":false,
            "secret":"K7Q4-9XM2"}"#;
        assert_eq!(
            stream_stats(watch).unwrap(),
            "watch: 24.0 fps 960x540, 1800 kbps, 3 dropped, 1 freezes, jitter 12 ms, decoder D3D11VideoDecoder, \
             rtt 40 ms, sound 90 kbps level 0.00, 2.5% sound filled in, MUTED (waiting for a click)"
        );
        assert_eq!(stream_stats(r#"{"fps":-5,"w":"x","kbps":1e12}"#), None, "nonsense is left out");
        assert_eq!(stream_stats("[1,2]"), None);
    }

    #[test]
    fn the_picture_meter_reports_every_ten_seconds() {
        let start = Instant::now();
        let mut meter = PictureMeter::new(start);
        let mut line = None;
        for i in 0..300u32 {
            let at = start + std::time::Duration::from_millis(u64::from(i) * 34);
            line = meter.picture(at, 81_920, (1280, 720), i % 2 == 0).or(line);
        }
        assert_eq!(line.as_deref(), Some("screencast: 29.5 pictures/s in, 14.8 out, 60 KB each, 1280x720"));
    }

    #[test]
    fn a_crash_while_starting_the_sound_is_remembered() {
        let dir = std::env::temp_dir().join(format!("ytmp-diag-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let log = dir.join(LOG);
        let older = "   1.000 [main] YT Mini Player\n   2.000 [main] share: on\n   2.100 [main] sound: starting (v0.10.1)\n";
        fs::write(&log, older).unwrap();
        start(&dir, "test");
        assert!(!sound_crashed(), "a crash in another version does not count");
        fs::write(&log, format!("   1.000 [main] YT Mini Player\n   2.000 [main] share: on\n   2.100 [main] {}\n   2.110 [main] sound: requested\n", sound_starting())).unwrap();
        start(&dir, "test");
        assert!(sound_crashed(), "it died while starting the sound");
        assert!(crashed_last_time(), "in the middle of sharing");
        assert!(dir.join(PREVIOUS).exists());
        note("share: on");
        finish();
        let text = fs::read_to_string(&log).unwrap();
        assert!(text.contains("share: on") && text.trim_end().ends_with("exit"));
        fs::remove_dir_all(&dir).unwrap();
    }
}

#[cfg(test)]
#[path = "tests/diag_cases.rs"]
mod cases;
