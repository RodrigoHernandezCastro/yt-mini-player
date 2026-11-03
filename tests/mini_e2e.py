"""End-to-end test of the music bar and of media.js in Chromium.

1. Serves src/mini.html at https://ytmp.localhost/mini with the same CSP as
   the app (and its strings, as home.rs serves them), sends it states the
   way the app does and checks what it sends back. Then again in Russian and
   Japanese, which have longer or wider texts.
2. Loads media.js in a page with a real <audio> (a WAV made in the browser)
   and Chromium's real Media Session: checks that it notes the site's
   controls, that it plays/pauses/seeks/changes the volume and that it
   reports the state.

Usage:  python tests/mini_e2e.py [folder_for_screenshots]
"""

import json
import pathlib
import sys

from playwright.sync_api import sync_playwright

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from bundle import add_app_scripts, fill, i18n, text  # noqa: E402

SHOTS = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else None
MINI_CSP = ("default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; "
            "img-src https: data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'")

# A 16:9 thumbnail served over https (a simple SVG).
THUMB = """<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180">
<defs><linearGradient id="g" x1="0" x2="1" y1="0" y2="1"><stop offset="0" stop-color="#3b4b8f"/><stop offset="1" stop-color="#b0476a"/></linearGradient></defs>
<rect width="320" height="180" fill="url(#g)"/><circle cx="230" cy="70" r="38" fill="#ffd27a"/></svg>"""

IPC_STUB = """
window.__sent = [];
Object.defineProperty(window, 'ipc', { value: Object.freeze({ postMessage: (m) => window.__sent.push(m) }) });
"""

# A page with a real 30 s <audio> of silence and Media Session controls.
MEDIA_PAGE = """<!doctype html><html><head><meta charset="utf-8"><title>Test song - YouTube</title></head><body>
<script>
  function wav(seconds) {
    const rate = 8000, n = rate * seconds, buf = new ArrayBuffer(44 + n * 2), v = new DataView(buf);
    const s = (o, t) => [...t].forEach((c, i) => v.setUint8(o + i, c.charCodeAt(0)));
    s(0, 'RIFF'); v.setUint32(4, 36 + n * 2, true); s(8, 'WAVE'); s(12, 'fmt ');
    v.setUint32(16, 16, true); v.setUint16(20, 1, true); v.setUint16(22, 1, true);
    v.setUint32(24, rate, true); v.setUint32(28, rate * 2, true); v.setUint16(32, 2, true); v.setUint16(34, 16, true);
    s(36, 'data'); v.setUint32(40, n * 2, true);
    return URL.createObjectURL(new Blob([buf], { type: 'audio/wav' }));
  }
  window.audio = new Audio(wav(30));
  document.body.append(window.audio);
  window.siteCalls = [];
  navigator.mediaSession.metadata = new MediaMetadata({
    title: 'Test song', artist: 'Artist',
    artwork: [{ src: 'https://img.example/96.jpg', sizes: '96x96' }, { src: 'https://img.example/512.jpg', sizes: '512x512' }],
  });
  navigator.mediaSession.setActionHandler('nexttrack', () => window.siteCalls.push('next'));
  navigator.mediaSession.setActionHandler('previoustrack', () => window.siteCalls.push('previous'));
</script></body></html>"""


def main():
    failures = []

    def check(cond, name):
        print(("  ok   " if cond else "  FAIL ") + name)
        if not cond:
            failures.append(name)

    with sync_playwright() as p:
        browser = p.chromium.launch(args=["--autoplay-policy=no-user-gesture-required"])

        # ------------------------------------------------------------------
        print("Music bar (docked in the taskbar, 360×40)")
        errors = []
        page = browser.new_page(viewport={"width": 360, "height": 40})
        # The broken thumbnail's 404 is on purpose (the fallback is tested).
        page.on("console", lambda m: m.type == "error" and "404" not in m.text and errors.append(m.text))
        page.on("pageerror", lambda e: errors.append(str(e)))
        lang = {"code": "en"}
        page.route("https://ytmp.localhost/**", lambda r: r.fulfill(
            status=200, content_type="text/html; charset=utf-8",
            headers={"Content-Security-Policy": MINI_CSP},
            body=fill("mini.html", {"i18n": i18n(lang["code"], "mini")}, lang["code"])))
        page.route("https://i.ytimg.com/**", lambda r: r.fulfill(status=200, content_type="image/svg+xml", body=THUMB))
        page.route("https://broken.example/**", lambda r: r.fulfill(status=404, body="no"))
        page.add_init_script(IPC_STUB)
        page.goto("https://ytmp.localhost/mini")
        page.wait_for_timeout(200)

        sent = lambda: page.evaluate("window.__sent.slice()")
        clear = lambda: page.evaluate("window.__sent.length = 0")
        has_class = lambda sel, cls: page.evaluate(f"document.querySelector({json.dumps(sel)}).classList.contains({json.dumps(cls)})")

        def shot(name):
            if SHOTS:
                SHOTS.mkdir(parents=True, exist_ok=True)
                page.screenshot(path=str(SHOTS / f"{name}.png"))

        def push(extra=None, **media):
            payload = {"media": media, "docked": True, "source": "YouTube Music"}
            payload.update(extra or {})
            page.evaluate(f"window.__ytmpMini({json.dumps(payload)})")
            page.wait_for_timeout(80)

        def fill_ratio():
            return page.evaluate("""() => document.getElementById('fill').getBoundingClientRect().width
                / document.getElementById('progress').getBoundingClientRect().width""")

        check(sent()[:1] == ["mini:ready"], f"says it is ready ({sent()})")
        check(page.inner_text("#title") == "Nothing playing", "empty state by default, in English")
        check(page.evaluate("document.documentElement.lang") == "en", "<html lang> says en")
        check(all(page.is_disabled(b) for b in ("#toggle", "#next", "#previous")), "without media, the controls are off")
        check(has_class("#wave", "idle") and has_class("#progress", "disabled"), "wave at rest and progress off")
        push(site="music.youtube.com")
        check(page.inner_text("#artist") == "YouTube Music", "empty on a music service: says which")
        clear()
        page.mouse.move(300, 20)
        page.mouse.wheel(0, -100)
        page.click("#wave")
        page.mouse.click(220, 38)
        check(sent() in ([], ["mini:hover:1"]), f"without media, wheel, wave and progress send nothing ({sent()})")
        shot("1-empty")

        state = dict(hasMedia=True, title="A song with a rather long title that does not fit in the bar and goes on",
                     artist="Sample channel", artwork="https://i.ytimg.com/vi/x/mqdefault.jpg", playing=True,
                     position=83, duration=245, volume=0.6, muted=False, canNext=True, canPrevious=True, canSeek=True,
                     site="music.youtube.com")
        push(**state)
        check(page.inner_text("#artist") == "Sample channel", "shows the artist")
        check(page.is_hidden("#icon-play") and page.is_visible("#icon-pause"), "playing: shows pause")
        check(has_class("#wave", "playing") and not has_class("#wave", "idle"), "the wave moves while it plays")
        check(abs(fill_ratio() - 83 / 245) < 0.01, f"the progress line marks 1:23 of 4:05 ({fill_ratio():.3f})")
        page.wait_for_timeout(200)
        check(page.evaluate("document.getElementById('artwork').naturalWidth > 0"), "loads the https thumbnail")
        shot("2-playing")

        for button, message in [("#toggle", "media:toggle"), ("#next", "media:next"), ("#previous", "media:previous"),
                                ("#art", "player:show"), ("#wave", "media:mute")]:
            clear()
            page.click(button)
            check(message in sent(), f"{button} sends {message} ({sent()})")

        clear()
        page.click("#title", button="right")
        check("mini:close" in sent(), f"right-click closes ({sent()})")

        box = page.locator("#progress").bounding_box()
        y = box["y"] + box["height"] - 2
        clear()
        page.mouse.move(box["x"] + box["width"] * 0.5, y)
        page.wait_for_timeout(50)
        tip = page.locator(".tip")
        check(tip.count() == 1 and tip.inner_text().endswith("/ 4:05"), f"hovering the line shows the time ({tip.all_inner_texts()})")
        page.mouse.down()
        page.mouse.up()
        msgs = [m for m in sent() if m.startswith("media:seek-to:")]
        check(len(msgs) == 1 and 115 <= int(msgs[0].split(":")[-1]) <= 130, f"a click in the middle seeks ~2:02 ({msgs})")

        clear()
        page.mouse.move(box["x"] + box["width"] * 0.2, y)
        page.mouse.down()
        page.mouse.move(box["x"] + box["width"] * 0.5, y - 20, steps=4)
        mid = [m for m in sent() if m.startswith("media:seek-to:")]
        check(not mid and has_class("#progress", "dragging"), "dragging the line does not seek until released")
        check(abs(fill_ratio() - 0.5) < 0.03, f"the line follows the mouse even outside it ({fill_ratio():.3f})")
        page.mouse.move(box["x"] + box["width"] * 0.75, y - 20, steps=4)
        page.mouse.up()
        msgs = [m for m in sent() if m.startswith("media:seek-to:")]
        check(len(msgs) == 1 and 175 <= int(msgs[0].split(":")[-1]) <= 190, f"releasing seeks ~3:04 ({msgs})")
        check(page.locator(".tip").count() == 0 and not has_class("#progress", "dragging"), "and the drag is cleared")
        drag_msgs = [m for m in sent() if m.startswith("mini:drag")]
        check(not drag_msgs, f"seeking on the line does not move the bar ({drag_msgs})")

        # Moving the bar along the taskbar by dragging the text.
        clear()
        tb = page.locator("#text").bounding_box()
        page.mouse.move(tb["x"] + 20, tb["y"] + 10)
        page.mouse.down()
        page.mouse.move(tb["x"] + 22, tb["y"] + 10)
        check(not [m for m in sent() if m.startswith("mini:drag")], "a 2 px tremble does not start a drag")
        page.mouse.move(tb["x"] - 60, tb["y"] + 10, steps=6)
        page.wait_for_timeout(60)
        page.mouse.up()
        drag_msgs = [m for m in sent() if m.startswith("mini:drag")]
        check(drag_msgs[:1] == ["mini:drag:start"] and drag_msgs[-1:] == ["mini:drag:end"] and "mini:drag:move" in drag_msgs,
              f"dragging the text moves the bar: start, move…, end ({drag_msgs[:3]}…{drag_msgs[-1:]})")
        check(not any(m.startswith("media:") or m == "player:show" for m in sent()), "and triggers no control")
        check(page.evaluate("getComputedStyle(document.getElementById('text')).cursor") == "grab", "the text has a hand cursor")
        clear()
        push({"docked": False}, **state)
        page.mouse.move(tb["x"] + 20, tb["y"] + 10)
        page.mouse.down()
        page.mouse.move(tb["x"] - 60, tb["y"] + 10, steps=6)
        page.mouse.up()
        check(not [m for m in sent() if m.startswith("mini:drag")], "floating (no taskbar) does not drag")
        push(**state)

        clear()
        page.mouse.move(150, 20)
        page.mouse.wheel(0, -100)
        check(has_class("#wave", "meter") and page.inner_text("#level") == "Volume 65%"
              and has_class("#text", "show-level"), f"the wheel shows the level ({page.inner_text('#level')})")
        page.mouse.wheel(0, 100)
        vols = [m for m in sent() if m.startswith("media:volume-by:")]
        check(vols == ["media:volume-by:5", "media:volume-by:-5"], f"the wheel turns the volume up and down ({vols})")
        page.wait_for_timeout(1600)
        check(not has_class("#wave", "meter") and not has_class("#text", "show-level"), "the meter goes back to the wave and the artist")

        push(**dict(state, volume=0.35))
        on = page.locator("#wave i.on").count()
        check(has_class("#wave", "meter") and page.inner_text("#level") == "Volume 35%" and on == 6,
              f"a volume change from the shortcut shows too ({page.inner_text('#level')}, {on} bars)")
        shot("3-volume")

        clear()
        page.mouse.move(-10, -10)
        page.mouse.move(100, 20)
        page.mouse.move(500, 500)
        hov = [m for m in sent() if m.startswith("mini:hover")]
        check(hov[-2:] == ["mini:hover:1", "mini:hover:0"], f"reports when the mouse comes in and leaves ({hov})")

        page.wait_for_timeout(1500)
        push(**dict(state, playing=False, muted=True, canNext=False, artwork="https://broken.example/x.jpg"))
        page.wait_for_timeout(250)
        check(page.is_visible("#icon-play"), "paused: shows play")
        check(page.is_disabled("#next") and not page.is_disabled("#previous"), "respects what the site allows")
        check(has_class("#wave", "muted") and not has_class("#wave", "meter") and page.inner_text("#level") == "Muted",
              "muted: overrides the meter and says so")
        check(page.evaluate("getComputedStyle(document.querySelector('.mute-icon')).opacity") == "1", "muted: crossed-out speaker icon")
        check(not has_class("#wave", "playing"), "muted: the wave does not move")
        check(page.is_visible("#empty"), "if the thumbnail fails, shows the icon")
        shot("4-muted")

        push(hasMedia=True, title="Live show", artist="Channel", live=True, duration=0, canSeek=False, playing=True, volume=1)
        check(has_class("#progress", "live") and has_class("#progress", "disabled"), "live: full line, no seeking")
        check(page.inner_text("#artist") == "Live · Channel", f"live: says so next to the artist ({page.inner_text('#artist')})")
        clear()
        page.mouse.click(220, 38)
        check(not [m for m in sent() if m.startswith("media:seek")], "live: clicking the line does not seek")

        clear()
        page.evaluate("""window.__ytmpMini({ media: { hasMedia: true, title: 'x', playing: true, volume: 1 }, docked: true,
          shortcuts: { playPause: 'Ctrl+Alt+P', next: 'Ctrl+Alt+.', toggleVisible: 'Ctrl+Alt+Y' } })""")
        page.wait_for_timeout(80)
        check(page.get_attribute("#next", "title") == "Next (Ctrl+Alt+.)",
              f"tooltips show the shortcut that really works on this PC ({page.get_attribute('#next', 'title')})")

        push(**state)

        def layout(width, height):
            page.set_viewport_size({"width": width, "height": height})
            page.wait_for_timeout(120)
            return page.evaluate("""() => {
                const t = document.getElementById('title');
                const boxes = ['art', 'text', 'previous', 'toggle', 'next', 'wave'].map((id) => {
                    const r = document.getElementById(id).getBoundingClientRect();
                    return [id, r.left, r.right, r.top, r.bottom, r.width > 0]; });
                return { lines: Math.round(t.getBoundingClientRect().height / parseFloat(getComputedStyle(t).lineHeight)),
                         cut: t.scrollWidth > t.clientWidth, boxes,
                         overflow: document.documentElement.scrollWidth > innerWidth || document.documentElement.scrollHeight > innerHeight }; }""")

        # Windows 11 (40), Windows 10 (32), floating (48) and narrow (so as not to cover icons).
        for w, h in [(360, 40), (360, 32), (360, 48), (280, 40), (250, 32), (540, 60)]:
            m = layout(w, h)
            shown = [b for b in m["boxes"] if b[5]]
            inside = all(0 <= l and r <= w and 0 <= t and b <= h for _, l, r, t, b, _ in shown)
            ordered = all(shown[i][2] <= shown[i + 1][1] for i in range(len(shown) - 1))
            text_w = next(b[2] - b[1] for b in m["boxes"] if b[0] == "text")
            wave_shown = next(b[5] for b in m["boxes"] if b[0] == "wave")
            check(inside and ordered and not m["overflow"] and m["lines"] == 1 and text_w >= 60,
                  f"{w}×{h}: all in a row and inside, title on one line, text {text_w:.0f} px wide")
            check(wave_shown == (w > 300), f"{w}×{h}: wave {'shown' if w > 300 else 'hidden to make room'}")
            if (w, h) == (360, 32):
                shot("5-windows10")
            if (w, h) == (280, 40):
                shot("6-narrow")
        layout(360, 40)
        check(page.evaluate("document.getElementById('title').scrollWidth > document.getElementById('title').clientWidth"),
              "the long title is cut with an ellipsis")

        check(not page.evaluate("document.body.classList.contains('css-border')"), "on Windows 11 the system draws the border")
        push({"cssBorder": True}, **state)
        check(page.evaluate("document.body.classList.contains('css-border')"), "on Windows 10 it draws its own border")

        # The app reloads the bar when the language changes (MiniBar::relocalize).
        for code, level in (("ru", "Громкость: 55 %"), ("ja", "音量 55%"), ("pl", "Głośność: 55%")):
            lang["code"] = code
            page.set_viewport_size({"width": 280, "height": 40})
            page.reload()
            page.wait_for_timeout(150)
            check(page.inner_text("#title") == text(code, "mini.nothing_playing"), f"{code}: empty state in that language")
            check(page.evaluate("document.documentElement.lang") == code, f"{code}: <html lang> (fonts for the script)")
            push(**dict(state, title=text(code, "mini.nothing_playing") * 3, volume=0.6))
            page.mouse.move(150, 20)
            page.mouse.wheel(0, 100)
            page.wait_for_timeout(80)
            check(page.inner_text("#level") == level, f"{code}: the level with the language's number style ({page.inner_text('#level')})")
            m = layout(280, 40)
            shot(f"7-{code}")
            check(not m["overflow"] and m["lines"] == 1, f"{code}: fits in 280×40 on one line")
            page.wait_for_timeout(1500)
        check(not errors, f"no console or CSP errors ({errors[:3]})")
        page.close()

        # ------------------------------------------------------------------
        print("media.js with real media")
        errors = []
        page = browser.new_page()
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.route("https://www.youtube.com/**", lambda r: r.fulfill(status=200, content_type="text/html; charset=utf-8", body=MEDIA_PAGE))
        add_app_scripts(page, maintenance=False)
        page.goto("https://www.youtube.com/watch?v=dQw4w9WgXcQ")
        page.wait_for_function("window.audio && window.audio.readyState >= 1")

        states = lambda: [json.loads(m[12:]) for m in page.evaluate("window.__sent.slice()") if m.startswith("media-state:")]

        page.evaluate("window.__ytmpMediaReport()")
        s = states()[-1]
        check(s["title"] == "Test song" and s["artist"] == "Artist", "reads the real Media Session metadata")
        check(s["artwork"] == "https://img.example/512.jpg", "picks the right thumbnail")
        check(abs(s["duration"] - 30) < 0.5 and not s["playing"], f"real duration and paused ({s['duration']})")
        check(s["canNext"] and s["canPrevious"], "detects the site's controls")
        check(s["site"] == "www.youtube.com" and s["watching"] is False, f"reports the site (it decides the music bar) ({s['site']})")

        page.evaluate("window.__ytmpMedia('toggle')")
        page.wait_for_timeout(400)
        check(page.evaluate("!window.audio.paused"), "plays the real audio")
        check(states()[-1]["playing"], "and reports it is playing")

        page.evaluate("window.__ytmpMedia('next'); window.__ytmpMedia('previous')")
        check(page.evaluate("window.siteCalls") == ["next", "previous"], "next/previous use the site's controls")

        page.evaluate("window.__ytmpMedia('seek-to', 12)")
        page.wait_for_timeout(200)
        check(abs(page.evaluate("window.audio.currentTime") - 12) < 0.6, "seeks in the real audio")
        page.evaluate("window.__ytmpMedia('volume', 0.35)")
        page.wait_for_timeout(200)
        check(abs(page.evaluate("window.audio.volume") - 0.35) < 1e-6, "changes the real volume")
        check(abs(states()[-1]["volume"] - 0.35) < 1e-6, "and reports the volume")

        page.evaluate("window.__ytmpMediaWatch(true)")
        before = len(states())
        page.wait_for_timeout(2300)
        check(len(states()) - before >= 2, f"while watched, reports the position every second ({len(states()) - before})")
        check(states()[-1]["watching"] is True, "and says it is being watched (the app checks after reloads)")
        page.evaluate("window.__ytmpMediaWatch(false)")
        page.evaluate("window.__ytmpMedia('toggle')")
        page.wait_for_timeout(300)
        check(page.evaluate("window.audio.paused"), "pauses the real audio")
        check(not errors, f"no errors on the page ({errors[:3]})")
        browser.close()

    print()
    if failures:
        print(f"{len(failures)} test(s) failed")
        sys.exit(1)
    print("mini.html + media.js: all tests pass")


if __name__ == "__main__":
    main()
