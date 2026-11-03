"""More generated Rust cases (run through tests/gen/rust_cases.py)."""
import json

from rust_cases import FUZZ_HELPER, File, rs


# ---------------------------------------------------------------------------
# together/: the room code and the protocol's small pieces
# ---------------------------------------------------------------------------

def code():
    f = File("code_cases.rs", "use super::*;\n")
    for seed in range(20):
        f.test(f"code round trip for secret {seed}", f"""
let secret = [{seed}u8; 32];
for window in [0u64, 1, 991_234, 123_456_789, u64::MAX / 1800] {{
    let code = Code::for_window(&secret, window);
    let shown = code.display();
    assert_eq!(shown.len(), CODE_CHARS + 2, "{{shown}}");
    assert_eq!(Code::parse(&shown), Ok(code.clone()));
    assert_eq!(Code::parse(&shown.to_lowercase()), Ok(code.clone()));
    assert_eq!(Code::parse(&shown.replace('-', " ")), Ok(code.clone()));
    assert_eq!(Code::parse(&shown.replace('-', "")), Ok(code.clone()));
    assert_eq!(code.lookup().len(), 64);
}}""")
    for seed in range(12):
        f.test(f"every single wrong character is caught, secret {seed}", f"""
let code = Code::for_window(&[{seed * 17 % 256}u8; 32], {seed * 1000 + 7});
let plain: Vec<u8> = code.display().replace('-', "").into_bytes();
for position in 0..plain.len() {{
    for &other in ALPHABET.iter() {{
        if other == plain[position] {{
            continue;
        }}
        let mut typo = plain.clone();
        typo[position] = other;
        assert_eq!(Code::parse(std::str::from_utf8(&typo).unwrap()), Err(CodeError::Check), "position {{position}}");
    }}
}}""")
    f.test("codes change every window and differ between secrets", """
let a = Code::for_window(&[1; 32], 10);
assert_ne!(a, Code::for_window(&[1; 32], 11));
assert_ne!(a, Code::for_window(&[2; 32], 10));
assert_eq!(a, Code::for_window(&[1; 32], 10));
assert_ne!(a.join_key(), Code::for_window(&[1; 32], 11).join_key());
assert_ne!(a.lookup(), Code::for_window(&[1; 32], 11).lookup());""")
    f.test("look-alike letters are read as digits", """
let code = Code::for_window(&[9; 32], 4242);
let shown = code.display();
let swapped: String = shown.chars().map(|c| match c { '0' => 'O', '1' => 'l', c => c }).collect();
assert_eq!(Code::parse(&swapped), Ok(code.clone()));
let swapped_i: String = shown.chars().map(|c| if c == '1' { 'I' } else { c }).collect();
assert_eq!(Code::parse(&swapped_i), Ok(code));""")
    f.test("separators anywhere are ignored", """
let code = Code::for_window(&[3; 32], 77);
let plain = code.display().replace('-', "");
let dotted: String = plain.chars().flat_map(|c| [c, '.']).collect();
assert_eq!(Code::parse(&dotted), Ok(code.clone()));
let spaced: String = plain.chars().flat_map(|c| ['_', c, ' ']).collect();
assert_eq!(Code::parse(&spaced), Ok(code));""")
    for text, err in [("", "Length"), ("   ", "Length"), ("K7Q4", "Length"), ("0" * 11, "Length"), ("0" * 13, "Length"),
                      ("0" * 40, "Length"), ("0000-0000-00U0", "Character"), ("0000-0000-000!", "Character"),
                      ("ÑÑÑÑÑÑÑÑÑÑÑÑ", "Character"), ("0000/0000/0000", "Character"), ("🙂", "Character")]:
        f.test(f"code {text or 'empty'} is a {err} error", f"assert_eq!(Code::parse({rs(text)}), Err(CodeError::{err}));")
    f.test("twelve zeros are a valid code", """assert!(Code::parse("0000-0000-0000").is_ok());""")
    for unix, w in [(0, 0), (1, 0), (1799, 0), (1800, 1), (3599, 1), (3600, 2), (1_700_000_000, 944_444)]:
        f.test(f"window of {unix} is {w}", f"""
assert_eq!(window({unix}), {w});
assert!(window_end({w}) > {unix});
assert_eq!(window_end({w}), {(w + 1) * 1800});""")
    f.test("the grace period is shorter than a period", "let grace = std::hint::black_box(GRACE_SECS);\nassert!(grace < PERIOD_SECS && grace > 0);")
    f.test("hex is lowercase and two digits a byte", """assert_eq!(hex(&[0, 1, 15, 16, 255]), "00010f10ff");""")
    f.test("a code never shows whole in debug output", """
let code = Code::for_window(&[5; 32], 5);
let debug = format!("{code:?}");
assert!(debug.len() < 12, "{debug}");
assert!(!debug.contains(&code.display()[5..]));""")
    return f.write()


def protocol():
    f = File("protocol_cases.rs", "use super::*;\n")
    for mid, ok in [("host", True), ("g1", True), ("g2", True), ("g10", True), ("g0", True), ("g1234567", True),
                    ("g", False), ("g12345678", False), ("h1", False), ("G1", False), ("g1 ", False), (" g1", False),
                    ("g-1", False), ("g1a", False), ("", False), ("HOST", False), ("hosts", False), ("g\u0663", False),
                    ("ｇ１", False), ("g+1", False), ("g01", True), ("g 1", False)]:
        f.test(f"is_member_id {mid!r} {ok}", f"assert_eq!(is_member_id({rs(mid)}), {str(ok).lower()});")
    for raw, clean in [("Ana", "Ana"), ("  Ana   María ", "Ana María"), ("Ana\nMaría", "Ana María"),
                       ("\u202eevil", "evil"), ("a\u2066b\u2069c", "a b c"), ("tab\there", "tab here"),
                       ("", ""), ("\n\t\r", ""), ("a" * 30, "a" * 24), ("名前" * 20, "名前" * 12),
                       ("🙂" * 30, "🙂" * 24), ("x\u0000y", "x y"), ("  ", ""), ("a b c", "a b c"),
                       ("Zoë", "Zoë"), ("x" * 24, "x" * 24), ("a\u00a0b", "a b"), ("  " + "b" * 26, "b" * 24)]:
        f.test(f"clean_name {raw[:12]!r}", f"assert_eq!(clean_name({rs(raw)}), {rs(clean)});")
    f.test("clean names never hold control or direction characters", """
for raw in ["a\\u{202a}b", "\\u{202b}x", "y\\u{202c}", "\\u{202d}z\\u{2067}", "q\\u{2068}w", "\\u{7}bell", "del\\u{7f}"] {
    let name = clean_name(raw);
    let odd = |c: char| c.is_control() || ('\\u{202a}'..='\\u{202e}').contains(&c) || ('\\u{2066}'..='\\u{2069}').contains(&c);
    assert!(!name.chars().any(odd), "{raw:?}");
    assert!(name.chars().count() <= MAX_NAME);
}""")
    f.test("limits are sane", "let (name, people, sdp) = std::hint::black_box((MAX_NAME, MAX_PEOPLE, MAX_SDP));\nassert!(name >= 16 && people == 6 && sdp >= 4096);")
    return f.write()


# ---------------------------------------------------------------------------
# media.rs: actions and the page's report
# ---------------------------------------------------------------------------

def media():
    f = File("media_cases.rs", "use super::*;\n" + FUZZ_HELPER)
    actions = {"toggle": "Some(MediaAction::PlayPause)", "next": "Some(MediaAction::Next)",
               "previous": "Some(MediaAction::Previous)", "mute": "Some(MediaAction::ToggleMute)",
               "seek-by:0": "Some(MediaAction::SeekBy(0))", "seek-by:+5": "Some(MediaAction::SeekBy(5))",
               "seek-by:-1": "Some(MediaAction::SeekBy(-1))", "seek-to:59": "Some(MediaAction::SeekTo(59))",
               "volume:50": "Some(MediaAction::Volume(50))", "volume-by:-5": "Some(MediaAction::VolumeBy(-5))",
               "volume-by:0": "Some(MediaAction::VolumeBy(0))", "seek-by:abc": "None", "seek-to:": "None",
               "volume:": "None", "volume": "None", "seek-by": "None", "TOGGLE": "None", "toggle:": "None",
               "next:1": "None", "seek-to:1.5": "None", "volume:0x10": "None", "seek-by:99999": "None",
               "volume-by:128": "None", "rate:100": "None", "play": "None", "pause": "None", "": "None"}
    for text, expected in actions.items():
        f.test(f"MediaAction::parse {text or 'empty'}", f"assert_eq!(MediaAction::parse({rs(text)}), {expected});")

    def report(body):
        return f"MediaState::from_report({rs(json.dumps(body, ensure_ascii=False))}).unwrap()"

    cases = [
        ({"hasMedia": True, "title": "  a \n b  ", "playing": True}, 's.title == "a b" && s.playing'),
        ({"hasMedia": False, "playing": True}, "!s.playing"),
        ({"hasMedia": True, "position": -5}, "s.position == 0.0"),
        ({"hasMedia": True, "position": 90, "duration": 60}, "s.position == 60.0 && s.duration == 60.0"),
        ({"hasMedia": True, "position": 90, "duration": 0}, "s.position == 90.0"),
        ({"hasMedia": True, "position": 1e9}, "s.position == 1e7"),
        ({"hasMedia": True, "duration": -1}, "s.duration == 0.0"),
        ({"volume": 1.5}, "s.volume == 1.0"), ({"volume": -1}, "s.volume == 0.0"), ({"volume": 0.25}, "s.volume == 0.25"),
        ({"rate": 0}, "s.rate == 1.0"), ({"rate": 20}, "s.rate == 1.0"), ({"rate": 0.5}, "s.rate == 0.5"),
        ({"rate": 16}, "s.rate == 16.0"), ({"rate": 0.0625}, "s.rate == 0.0625"), ({"rate": 0.06}, "s.rate == 1.0"),
        ({"site": "Music.YouTube.com"}, 's.site == "music.youtube.com"'), ({"site": "evil.com/<x>"}, "s.site.is_empty()"),
        ({"site": "a b.com"}, "s.site.is_empty()"), ({"site": "x" * 254}, "s.site.is_empty()"),
        ({"site": "x" * 253}, "s.site.len() == 253"),
        ({"artwork": "https://i.ytimg.com/vi/x/mq.jpg"}, 's.artwork == "https://i.ytimg.com/vi/x/mq.jpg"'),
        ({"artwork": "javascript:alert(1)"}, "s.artwork.is_empty()"), ({"artwork": "http://x.com/a.jpg"}, "s.artwork.is_empty()"),
        ({"artwork": "data:image/png;base64,AA"}, "s.artwork.is_empty()"),
        ({"artwork": "https://" + "a" * 2050}, "s.artwork.is_empty()"), ({"artwork": "file:///C:/x.jpg"}, "s.artwork.is_empty()"),
        ({"title": "t" * 250}, "s.title.chars().count() == 200 && s.title.ends_with('…')"),
        ({"title": "t" * 200}, "s.title.chars().count() == 200 && !s.title.ends_with('…')"),
        ({"artist": "a" * 130}, "s.artist.chars().count() == 120 && s.artist.ends_with('…')"),
        ({"title": "a\u0000b\u0007c"}, 's.title == "a b c"'), ({"unknownField": 1, "hasMedia": True}, "s.has_media"),
        ({}, "s == MediaState::default().sanitized()"),
    ]
    for body, check in cases:
        f.test(f"from_report {json.dumps(body)[:40]}", f"let s = {report(body)};\nassert!({check}, \"{{s:?}}\");")
    for text in ["", "null", "not json", '{"title": 5}', '{"hasMedia": "yes"}', '{"position": "1"}', '{"volume": true}']:
        f.test(f"from_report refuses {text or 'empty'}", f"assert_eq!(MediaState::from_report({rs(text)}), None);")
    f.test("from_report refuses a report past its limit", """
let big = format!("{{\\"title\\":\\"{}\\"}}", "x".repeat(MAX_REPORT_BYTES));
assert_eq!(MediaState::from_report(&big), None);""")
    for volume, muted, percent in [(0.5, False, 50), (0.5, True, 0), (1.0, False, 100), (0.0, False, 0), (0.556, False, 56),
                                   (0.004, False, 0), (0.996, False, 100)]:
        f.test(f"volume_percent {volume} muted {muted}",
               f"assert_eq!(MediaState {{ volume: {volume}, muted: {str(muted).lower()}, ..Default::default() }}.volume_percent(), {percent});")
    for text, max_len, expected in [("abc", 5, "abc"), ("abcdef", 5, "abcd…"), ("abcde", 5, "abcde"), ("", 3, ""),
                                    ("ñandú", 4, "ñan…"), ("🙂🙂🙂", 2, "🙂…"), ("abc", 1, "…"), ("abc", 0, "…")]:
        f.test(f"truncate {text or 'empty'} to {max_len}", f"assert_eq!(truncate({rs(text)}, {max_len}), {rs(expected)});")
    f.test("the tooltip without media is the app's name", "assert_eq!(MediaState::default().tooltip(), crate::platform::APP_NAME);")
    f.test("the tooltip shows playing and paused", """
let mut s = MediaState { has_media: true, title: "Song".into(), playing: true, ..Default::default() };
assert_eq!(s.tooltip(), format!("▶ Song\\n{}", crate::platform::APP_NAME));
s.playing = false;
assert_eq!(s.tooltip(), format!("⏸ Song\\n{}", crate::platform::APP_NAME));""")
    f.test("the tooltip fits Windows' 127 characters", """
let s = MediaState { has_media: true, title: "x".repeat(400), playing: true, ..Default::default() };
assert!(s.tooltip().chars().count() <= 127);""")
    for seed in range(1, 9):
        f.test(f"from_report fuzz seed {seed}: never panics, always sane", f"""
let mut rng = Rng::new({seed * 4099});
let keys = ["title", "artist", "artwork", "site", "position", "duration", "volume", "rate", "playing", "hasMedia"];
for _ in 0..1500 {{
    let key = keys[rng.below(keys.len())];
    let value = match rng.below(4) {{
        0 => format!("{{}}", rng.next() as i64 as f64 / 1e6),
        1 => serde_json::to_string(&rng.text(&[])).unwrap(),
        2 => (rng.below(2) == 0).to_string(),
        _ => "null".to_owned(),
    }};
    let json = format!("{{{{\\"{{key}}\\":{{value}},\\"hasMedia\\":true}}}}");
    if let Some(s) = MediaState::from_report(&json) {{
        assert!(s.position.is_finite() && (0.0..=1e7).contains(&s.position), "{{json}}");
        assert!((0.0..=1.0).contains(&s.volume), "{{json}}");
        assert!((0.0625..=16.0).contains(&s.rate), "{{json}}");
        assert!(s.title.chars().count() <= 200 && !s.title.chars().any(char::is_control), "{{json}}");
        assert!(s.artwork.is_empty() || s.artwork.starts_with("https://"), "{{json}}");
    }}
}}""")
    return f.write()


# ---------------------------------------------------------------------------
# loopback.rs, pacer.rs: sound and picture pacing
# ---------------------------------------------------------------------------

LOOPBACK_HELPERS = """
/// One input frame of `channels` samples, all at `value` (from -1 to 1), in `sample`'s encoding.
fn frame(sample: Sample, channels: usize, value: f32) -> Vec<u8> {
    let one: Vec<u8> = match sample {
        Sample::F32 => value.to_le_bytes().to_vec(),
        Sample::S16 => ((value * 32_768.0) as i16).to_le_bytes().to_vec(),
        Sample::S24 => ((value * 8_388_608.0) as i32).to_le_bytes()[..3].to_vec(),
        Sample::S32 => ((f64::from(value) * 2_147_483_648.0) as i32).to_le_bytes().to_vec(),
    };
    one.repeat(channels)
}

fn samples(bytes: &[u8]) -> Vec<i16> {
    bytes.chunks_exact(2).map(|s| i16::from_le_bytes([s[0], s[1]])).collect()
}

/// What a constant level becomes after folding `channels` into stereo.
fn folded(channels: usize, value: f32) -> f32 {
    const HALF: f32 = std::f32::consts::FRAC_1_SQRT_2;
    match channels {
        1 | 2 => value,
        n => {
            let extra = (4..n).filter(|i| i % 2 == 0).count() as f32;
            value + value * HALF + extra * value * HALF
        }
    }
}
"""


def loopback():
    f = File("loopback_cases.rs", "use super::*;\nuse std::time::{Duration, Instant};\n" + LOOPBACK_HELPERS)
    for sample in ["F32", "S16", "S24", "S32"]:
        for channels in [1, 2, 6, 8]:
            for rate in [8000, 11025, 16000, 22050, 24000, 32000, 44100, 48000, 88200, 96000, 192000]:
                f.test(f"converter {sample} {channels} ch {rate} Hz keeps level, length and silence", f"""
let mut converter = Converter::new(Sample::{sample}, {channels}, {rate}).unwrap();
let frames = {rate} / 10;
let input = frame(Sample::{sample}, {channels}, 0.25).repeat(frames);
let out = converter.convert(Some(&input), frames);
let got = out.len() / 4;
let expected = frames as f64 * 48_000.0 / {rate}.0;
let slack = 48_000.0 / {rate}.0 + 2.0;
assert!((got as f64 - expected).abs() <= slack, "{{got}} frames, {{expected}} expected");
let level = (folded({channels}, 0.25).clamp(-1.0, 1.0) * 32_767.0).round() as i32;
for s in samples(&out) {{
    assert!((i32::from(s) - level).abs() <= 2, "{{s}} vs {{level}}");
}}
let mut fresh = Converter::new(Sample::{sample}, {channels}, {rate}).unwrap();
let silence = fresh.convert(None, frames);
assert!(!silence.is_empty() && silence.iter().all(|b| *b == 0));""")
    for sizes in [[3840], [3836, 4], [1, 1, 1, 1], [7680, 3840], [100] * 50, [3839], [3841], [0], [4] * 960,
                  [3840 * 5 + 12], [2000, 2000, 2000, 2000], [960 * 4 - 4, 8]]:
        sizes_rs = "&[" + ", ".join(str(x) for x in sizes) + "]"
        total = sum(sizes)
        f.test(f"chunker cuts {len(sizes)} pushes of {total} bytes into 20 ms pieces", f"""
let out = std::cell::RefCell::new(Vec::new());
let sink = |piece: Vec<u8>| out.borrow_mut().push(piece);
let mut chunker = Chunker::default();
let sizes: &[usize] = {sizes_rs};
for size in sizes {{
    chunker.push(&vec![7u8; *size], &sink);
}}
let pieces = out.borrow();
assert_eq!(pieces.len(), {total // 3840});
assert!(pieces.iter().all(|p| p.len() == CHUNK_FRAMES * BYTES_PER_FRAME));
assert_eq!(chunker.buffer.len(), {total % 3840});""")
    loud, quiet, nothing = "Heard { frames: 240_000, peak: 9_000 }", "Heard { frames: 240_000, peak: 0 }", "Heard::default()"
    yes, no, unknown = "Some(true)", "Some(false)", "None"
    sequences = {
        "three quiet checks while playing": [(quiet, yes, "None"), (quiet, yes, "None"), (quiet, yes, "Some(true)")],
        "quiet but nothing plays": [(quiet, no, "None")] * 6,
        "unknown playing state": [(nothing, unknown, "None")] * 6,
        "sound resets the count": [(quiet, yes, "None"), (quiet, yes, "None"), (loud, yes, "None"),
                                   (quiet, yes, "None"), (quiet, yes, "None"), (quiet, yes, "Some(true)")],
        "deaf then heard": [(nothing, yes, "None")] * 2 + [(nothing, yes, "Some(true)"), (loud, no, "Some(false)")],
        "a pause keeps the count": [(quiet, yes, "None"), (quiet, no, "None"), (quiet, yes, "None"), (quiet, yes, "Some(true)")],
        "said once": [(quiet, yes, "None")] * 2 + [(quiet, yes, "Some(true)")] + [(quiet, yes, "None")] * 5,
        "loud all along": [(loud, yes, "None")] * 10,
        "a peak of one is sound": [("Heard { frames: 10, peak: 1 }", yes, "None")] * 5,
        "deaf stays deaf while nothing plays": [(quiet, yes, "None")] * 2 + [(quiet, yes, "Some(true)")] + [(nothing, no, "None")] * 4,
    }
    for name, steps in sequences.items():
        lines = ["let mut watch = SoundWatch::default();"]
        for heard, playing, out in steps:
            lines.append(f"assert_eq!(watch.check({heard}, {playing}), {out});")
        f.test(f"sound watch: {name}", "\n".join(lines))
    for adds, frames, peak in [([(960, 100)], 960, 100), ([(960, 100), (960, 300)], 1920, 300), ([], 0, 0),
                               ([(1, 65535)], 1, 65535), ([(5, 5)] * 10, 50, 5)]:
        body = "let levels = Levels::default();\n" + "\n".join(f"levels.add({a}, {b});" for a, b in adds)
        body += f"\nassert_eq!(levels.take(), Heard {{ frames: {frames}, peak: {peak} }});\nassert_eq!(levels.take(), Heard::default());"
        f.test(f"levels {len(adds)} adds of {frames} frames", body)
    for bytes_, expected in [("&[]", 0), ("&[0, 0, 0, 0]", 0), ("&[1, 0]", 1), ("&[0xff, 0x7f]", 32767), ("&[0x00, 0x80]", 32768),
                             ("&[0xff, 0xff]", 1), ("&[0x00, 0x40, 0x00, 0xc0]", 16384), ("&[5]", 0)]:
        f.test(f"loudest of {bytes_}", f"assert_eq!(loudest({bytes_}), {expected});")
    f.test("perf: converting a minute of 44.1 kHz surround stays fast", """
let mut converter = Converter::new(Sample::F32, 6, 44_100).unwrap();
let packet = frame(Sample::F32, 6, 0.1).repeat(441);
let start = Instant::now();
let mut out = 0;
for _ in 0..6000 {
    out += converter.convert(Some(&packet), 441).len();
}
let budget = Duration::from_millis(if cfg!(debug_assertions) { 60_000 } else { 800 });
assert!(start.elapsed() < budget, "{:?}", start.elapsed());
assert!(out > 0);""")
    f.test("perf: the meter and chunker keep up with an hour of sound", """
let mut meter = SoundMeter::new(Instant::now());
let mut chunker = Chunker::default();
let piece = vec![1u8; CHUNK_FRAMES * BYTES_PER_FRAME];
let start = Instant::now();
let sink = |_: Vec<u8>| {};
for _ in 0..(3600 * 50 / 10) {
    meter.packet(Some(&piece), CHUNK_FRAMES);
    chunker.push(&piece, &sink);
}
let budget = Duration::from_millis(if cfg!(debug_assertions) { 30_000 } else { 1_500 });
assert!(start.elapsed() < budget, "{:?}", start.elapsed());""")
    return f.write()


def pacer():
    f = File("pacer_cases.rs", "use super::*;\nuse std::time::Duration;\n")
    for rate in [1, 10, 24, 30, 60]:
        for gap in [5, 10, 16, 17, 33, 34, 50, 100, 250, 999]:
            if gap * rate == 1000:
                continue
            arrivals = 10_000 // gap
            target = min(arrivals, rate * 10)
            f.test(f"pacer {rate} a second with a picture every {gap} ms", f"""
let mut pacer = Pacer::new({rate});
let start = Instant::now();
let mut sent = 0i64;
for i in 1..={arrivals}u64 {{
    sent += i64::from(pacer.take(start + Duration::from_millis({gap} * i)));
}}
assert!(sent <= {rate * 10 + 3}, "{{sent}}");
assert!((sent - {target}).abs() <= 3, "{{sent}} vs {target}");""")
    f.test("pacer of zero behaves as one a second", """
let mut pacer = Pacer::new(0);
let start = Instant::now();
let sent = (1..=100u64).filter(|i| pacer.take(start + Duration::from_millis(100 * i))).count();
assert!((8..=13).contains(&sent), "{sent}");""")
    f.test("a picture back in time is not a burst", """
let mut pacer = Pacer::new(30);
let start = Instant::now() + Duration::from_secs(10);
assert!(pacer.take(start));
assert!(!pacer.take(start - Duration::from_secs(5)));""")
    return f.write()


# ---------------------------------------------------------------------------
# topmost.rs, diag.rs, e2e.rs, config.rs
# ---------------------------------------------------------------------------

def topmost():
    f = File("topmost_cases.rs", "use super::*;\nuse std::time::{Duration, Instant};\n")
    monitor = (0, 0, 1920, 1080)

    def rect(r):
        return f"Rect {{ left: {r[0]}, top: {r[1]}, right: {r[2]}, bottom: {r[3]} }}"

    for r, ok in [((0, 0, 1920, 1080), True), ((-8, -8, 1928, 1088), True), ((0, 0, 1920, 1040), False),
                  ((1, 0, 1920, 1080), False), ((0, 1, 1920, 1080), False), ((0, 0, 1919, 1080), False),
                  ((0, 0, 1920, 1079), False), ((-1920, 0, 1920, 1080), True), ((0, 0, 3840, 2160), True),
                  ((100, 100, 900, 700), False), ((1920, 0, 3840, 1080), False), ((0, 0, 0, 0), False)]:
        f.test(f"covers {r} {ok}", f"assert_eq!(covers({rect(r)}, {rect(monitor)}), {str(ok).lower()});")
    flags = [("TOOLWINDOW", 0x80), ("NOACTIVATE", 0x0800_0000), ("LAYERED", 0x80000), ("TRANSPARENT", 0x20)]
    for mask in range(16):
        style = sum(v for i, (_, v) in enumerate(flags) if mask >> i & 1)
        names = [n for i, (n, _) in enumerate(flags) if mask >> i & 1] or ["none"]
        overlay = bool(mask & 1) or bool(mask & 2) or (bool(mask & 4) and bool(mask & 8))
        f.test(f"is_overlay {'+'.join(names)}", f"assert_eq!(is_overlay({style:#x}), {str(overlay).lower()});")
    for exe, protected in [("explorer.exe", True), ("EXPLORER.EXE", True), ("ShellExperienceHost.exe", True),
                           ("StartMenuExperienceHost.exe", True), ("SearchHost.exe", True), ("SearchApp.exe", True),
                           ("ScreenClippingHost.exe", True), ("SnippingTool.exe", True), ("LockApp.exe", True),
                           ("LogonUI.exe", True), ("TextInputHost.exe", True), ("ShellHost.exe", True), ("dwm.exe", True),
                           ("Magnify.exe", True), ("ShareX.exe", True), ("Greenshot.exe", True), ("Lightshot.exe", True),
                           ("flameshot.exe", True), ("PowerToys.ScreenRuler.exe", True), ("powertoys.exe", True),
                           ("Ribbons.scr", True), ("javaw.exe", False), ("Game-Win64-Shipping.exe", False),
                           ("chrome.exe", False), ("vlc.exe", False), ("", False), ("explorer.exe.bak", False),
                           ("myexplorer.exe", False), ("screensaver.scrx", False)]:
        f.test(f"is_protected_process {exe or 'empty'} {protected}", f"assert_eq!(is_protected_process({rs(exe)}), {str(protected).lower()});")
    for state in range(0, 9):
        for changed in [False, True]:
            expected = "Some(Exclusive::Direct3D)" if state == 3 else ("Some(Exclusive::DisplayMode)" if changed else "None")
            f.test(f"exclusive_signs state {state} mode changed {changed}",
                   f"assert_eq!(exclusive_signs({state}, {str(changed).lower()}), {expected});")
        f.test(f"notification_label {state}", f"assert!(!notification_label({state}).is_empty());")
    for exe, hint in [("javaw.exe", "GameHint::Minecraft"), ("java.exe", "GameHint::Minecraft"), ("JavaW.EXE", "GameHint::Minecraft"),
                      ("RobloxPlayerBeta.exe", 'GameHint::Other("RobloxPlayerBeta".into())'),
                      ("eldenring.EXE", 'GameHint::Other("eldenring".into())'), ("game", 'GameHint::Other("game".into())'),
                      ("", "GameHint::Other(String::new())"), ("javaw", 'GameHint::Other("javaw".into())')]:
        f.test(f"game_hint {exe or 'empty'}", f"assert_eq!(game_hint({rs(exe)}), {hint});")
    for raised, minimized, ok in [(0, 0, True), (0, 100, True), (0, 2999, True), (0, 3000, False), (500, 0, False),
                                  (1000, 3999, True), (1000, 4000, False), (0, 10_000, False)]:
        f.test(f"exclusive raised {raised} minimized {minimized}", f"""
let t = Instant::now();
assert_eq!(exclusive(t + Duration::from_millis({raised}), t + Duration::from_millis({minimized}), Duration::from_secs(3)), {str(ok).lower()});""")
    for band, label in [(0, "default"), (1, "normal windows"), (2, "UIAccess (maximum overlay)"), (3, "a system band"), (17, "a system band")]:
        f.test(f"band_label {band}", f"assert_eq!(band_label({band}), {rs(label)});")
    for exe, style, cloaked, ok in [("game.exe", 0, False, True), ("game.exe", 0, True, False), ("explorer.exe", 0, False, False),
                                    ("game.exe", 0x80, False, False), ("game.exe", 0x80000, False, True),
                                    ("game.exe", 0x80020, False, False), ("", 0, False, True)]:
        f.test(f"coverable {exe or 'empty'} {style:#x} cloaked {cloaked}", f"""
let fg = Foreground {{ rect: {rect(monitor)}, ex_style: {style:#x}, cloaked: {str(cloaked).lower()}, above_us: true, exe: {rs(exe)} }};
assert_eq!(coverable(&fg), {str(ok).lower()});""")
    return f.write()


def diag():
    f = File("diag_cases.rs", "use super::*;\n")
    healthy = {"site": "youtube", "page": "watch", "player": "main", "api": True, "video": True, "fullscreenButton": True}
    for player in ["main", "fallback", "video", "none"]:
        for api in [True, False]:
            body = dict(healthy, player=player, api=api)
            broken = player != "main" or not api
            f.test(f"page check player {player} api {api}", f"""
let check = PageCheck::from_report({rs(json.dumps(body))}).unwrap();
assert_eq!(check.broken(), {'Some("YouTube")' if broken else 'None'});
assert!(check.line().starts_with("page check: youtube watch: player "));""")
    for field in ["video", "fullscreenButton"]:
        body = dict(healthy, **{field: False})
        f.test(f"page check without {field}", f"""
let check = PageCheck::from_report({rs(json.dumps(body))}).unwrap();
assert_eq!(check.broken(), Some("YouTube"));
assert!(check.line().contains("MISSING"));""")
    for bad in [dict(healthy, player="Main"), dict(healthy, api=1), {k: v for k, v in healthy.items() if k != "video"},
                dict(healthy, site="vimeo"), dict(healthy, page="home"), {"site": "google", "page": "signin"},
                {"site": "google"}, {"page": "watch"}, [], "text", None, 5]:
        f.test(f"page check refuses {json.dumps(bad)[:40]}", f"assert_eq!(PageCheck::from_report({rs(json.dumps(bad))}), None);")
    f.test("page check accepts Google's refusal", """
let check = PageCheck::from_report(r#"{"site":"google","page":"sign-in-refused"}"#).unwrap();
assert_eq!(check, PageCheck::GoogleSignInRefused);
assert_eq!(check.broken(), Some("Google"));""")
    casts = [
        ({"in": 30, "out": 29.6}, ["cast: 30.0 pictures/s in, 29.6 to the tracks"]),
        ({"decodeMs": 2, "sound": 50}, ["cast: 2.0 ms to decode, 50 sound pieces/s"]),
        ({"in": -1}, []), ({"in": 2e9}, []), ({"in": "30"}, []), ({}, []),
        ({"peers": [{"id": "g1", "state": "connected", "fps": 30, "w": 1280, "h": 720, "kbps": 2310}]},
         ["cast g1: connected, 30.0 fps 1280x720, 2310 kbps"]),
        ({"peers": [{"id": "g2", "codec": "H264", "enc": "MediaFoundation"}]}, ["cast g2: H264 (MediaFoundation)"]),
        ({"peers": [{"id": "g3", "limit": "bandwidth", "rtt": 45, "lost": 0.5, "route": "srflx-host"}]},
         ["cast g3: limited by bandwidth, rtt 45 ms, 0.5% lost, via srflx-host"]),
        ({"peers": [{"id": "g4", "limit": "none"}]}, ["cast g4: "]),
        ({"peers": [{"id": "../x", "fps": 30}]}, []), ({"peers": [{"fps": 30}]}, []),
        ({"peers": [{"id": "g5", "audioKbps": 96, "level": 0.21}]}, ["cast g5: sound 96 kbps level 0.21"]),
        ({"peers": [{"id": "g6", "state": "<script>alert(1)</script>"}]}, ["cast g6: scriptalert(1)/script"]),
        ({"peers": [{"id": "g7", "route": "1.2.3.4:5678"}]}, ["cast g7: via 1.2.3.45678"]),
    ]
    for body, lines in casts:
        lines_rs = "vec![" + ", ".join(f"{rs(line)}.to_owned()" for line in lines) + "]"
        f.test(f"cast_stats {json.dumps(body)[:40]}", f"let expected: Vec<String> = {lines_rs};\nassert_eq!(cast_stats({rs(json.dumps(body))}), expected);")
    f.test("cast_stats keeps at most eight peers", """
let peers: Vec<String> = (1..=12).map(|i| format!("{{\\"id\\":\\"g{i}\\",\\"fps\\":30}}")).collect();
let json = format!("{{\\"peers\\":[{}]}}", peers.join(","));
assert_eq!(cast_stats(&json).len(), 8);""")
    f.test("cast_stats ignores what is not JSON", """assert!(cast_stats("not json").is_empty() && cast_stats("").is_empty());""")
    streams = [
        ({"fps": 24, "w": 1920, "h": 1080}, "watch: 24.0 fps 1920x1080"),
        ({"dropped": 3, "freezes": 1, "jitter": 12}, "watch: 3 dropped, 1 freezes, jitter 12 ms"),
        ({"dec": "D3D11VideoDecoder", "rtt": 80, "route": "relay"}, "watch: decoder D3D11VideoDecoder, rtt 80 ms, via relay"),
        ({"concealed": 2.5, "muted": True, "paused": True}, "watch: 2.5% sound filled in, MUTED (waiting for a click), paused"),
        ({"muted": False, "paused": False}, None), ({}, None), ({"fps": "30"}, None),
        ({"audioKbps": 64}, "watch: sound 64 kbps"),
    ]
    for body, line in streams:
        expected = f"Some({rs(line)}.to_owned())" if line else "None"
        f.test(f"stream_stats {json.dumps(body)[:40]}", f"assert_eq!(stream_stats({rs(json.dumps(body))}), {expected});")
    for text in ["[]", "1", "\"x\"", "null", "", "{"]:
        f.test(f"stream_stats refuses {text or 'empty'}", f"assert_eq!(stream_stats({rs(text)}), None);")
    f.test("the crash marker names this version", """
assert!(sound_starting().contains(env!("CARGO_PKG_VERSION")));
assert!(SOUND_STARTED.starts_with("sound: "));""")
    return f.write()


def e2e():
    f = File("e2e_cases.rs", "use super::*;\n")
    for text, port in [("9333", "Some(9333)"), (" 9333 ", "Some(9333)"), ("1024", "Some(1024)"), ("65535", "Some(65535)"),
                       ("1023", "None"), ("80", "None"), ("0", "None"), ("65536", "None"), ("-1", "None"), ("", "None"),
                       ("x", "None"), ("9333x", "None"), ("9 333", "None"), ("0x2405", "None")]:
        f.test(f"parse_port {text.strip() or 'empty'}", f"assert_eq!(parse_port({rs(text)}), {port});")
    for text, ok in [("/tmp/ytmp-a", True), ("  /tmp/a  ", True), ("relative/folder", False), ("", False), ("   ", False),
                     ("./here", False), ("../up", False)]:
        f.test(f"parse_dir {text.strip() or 'blank'} {ok}",
               f"assert!(parse_dir({rs(text)}).{'is_some' if ok else 'is_none'}());")
    for a, b, same in [("/tmp/a", "/tmp/a", True), ("/tmp/a", "/TMP/A", True), ("/tmp/a", "/tmp/b", False),
                       ("/x", "/x/", False), ("/data/host", "/data/guest", False)]:
        f.test(f"instance {a} vs {b}",
               f"assert_{'eq' if same else 'ne'}!(instance_for(std::path::Path::new({rs(a)})), "
               f"instance_for(std::path::Path::new({rs(b)})));")
    f.test("instance names are safe for an event name", """
for dir in ["/tmp/a", "/tmp/ü ñ", "/tmp/a:b\\\\c"] {
    let name = instance_for(std::path::Path::new(dir));
    assert!(name.starts_with("e2e-") && name.len() == 20);
    assert!(name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
}""")
    return f.write()


def config():
    f = File("config_cases.rs", "use super::*;\n")
    f.test("defaults: on top, opaque, menu first", """
let s = Settings::default();
assert!(s.always_on_top && s.opacity == 100 && !s.video_only && !s.fullscreen_in_window);
assert!(s.taskbar_bar && s.hardware_acceleration && s.together_share && !s.welcome_seen);
assert_eq!(s.lock, LockMode::Off);
assert!(s.language.is_none() && s.last_service.is_none() && s.pin_hash.is_none());""")
    for body, check in [("{}", "s == Settings::default()"), ('{"opacity": 55}', "s.opacity == 55 && s.always_on_top"),
                        ('{"unknown": true, "video_only": true}', "s.video_only"), ('{"lock": "pin"}', "s.lock == LockMode::Pin"),
                        ('{"lock": "hello"}', "s.lock == LockMode::Hello"), ('{"lock": "off"}', "s.lock == LockMode::Off"),
                        ('{"fullscreen_in_window": true}', "s.fullscreen_in_window"),
                        ('{"language": "pl"}', 's.language.as_deref() == Some("pl")'),
                        ('{"together_name": "Ana"}', 's.together_name == "Ana"'),
                        ('{"approved_extensions": {"dark": "ab"}}', 's.approved_extensions.get("dark").map(String::as_str) == Some("ab")')]:
        f.test(f"settings from {body}", f"let s: Settings = serde_json::from_str({rs(body)}).unwrap();\nassert!({check}, \"{{s:?}}\");")
    for body in ['{"lock": "PIN"}', '{"opacity": "high"}', '{"opacity": 300}', '{"opacity": -1}', 'nope']:
        f.test(f"settings refuse {body}", f"assert!(serde_json::from_str::<Settings>({rs(body)}).is_err());")
    f.test("settings round trip through JSON", """
let s = Settings { opacity: 42, fullscreen_in_window: true, lock: LockMode::Hello, language: Some("ja".into()), ..Default::default() };
let back: Settings = serde_json::from_str(&serde_json::to_string(&s).unwrap()).unwrap();
assert_eq!(back, s);""")
    for mode, text in [("Off", "off"), ("Hello", "hello"), ("Pin", "pin")]:
        f.test(f"lock mode {text} is written in lowercase",
               f"assert_eq!(serde_json::to_string(&LockMode::{mode}).unwrap(), {rs(json.dumps(text))});")
    return f.write()


GENERATORS = [code, protocol, media, loopback, pacer, topmost, diag, e2e, config]
