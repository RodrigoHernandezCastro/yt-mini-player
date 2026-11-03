"""End-to-end test of the start screen and of other services.

- Serves src/home.html at https://ytmp.localhost/ with the same security
  policy (CSP) the app uses, and injects inject.js and ui.js.
- Serves a page that imitates another service's player (big video with its
  own controls) at https://www.netflix.com/watch/1.
- Changes the language the way the app does (lang:xx → __ytmpLocale).

Usage:  python tests/home_e2e.py [folder_for_screenshots]
"""

import json
import pathlib
import sys

from playwright.sync_api import sync_playwright

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from bundle import HOST_KEY, LANGUAGES, add_app_scripts, catalog, fill, i18n, push_locale, text  # noqa: E402

SHOTS = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else None
CSP = ("default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; "
       "img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'")

SERVICES = [
    ("youtube", "YouTube", "service.videos_live", "#ff4e45", False),
    ("youtube-music", "YouTube Music", "service.music_podcasts", "#ff6f61", False),
    ("netflix", "Netflix", "service.shows_movies", "#e50914", True),
    ("crunchyroll", "Crunchyroll", "service.anime", "#f47521", True),
    ("prime-video", "Prime Video", "service.shows_movies", "#1f9bde", True),
    ("disney-plus", "Disney+", "service.shows_movies", "#2fb5c4", True),
    ("hbo-max", "HBO Max", "service.shows_movies", "#8a5cf6", True),
    ("twitch", "Twitch", "service.live", "#a970ff", False),
]

STREAMING_HTML = """<!doctype html><html><head><meta charset="utf-8"><style>
  body { margin: 0; background: #000; }
  #player { position: fixed; inset: 0; }
  video { width: 100%; height: 100%; background: linear-gradient(135deg,#402020,#000); display: block; }
  .controls { position: absolute; left: 0; right: 0; bottom: 0; height: 48px; display: flex; align-items: center; gap: 8px; padding: 0 12px; background: rgba(0,0,0,.5); }
  .seek { flex: 1; height: 6px; background: #888; }
  .controls button { width: 32px; height: 32px; }
</style></head><body>
  <div id="player">
    <video></video>
    <div class="controls"><button aria-label="Play">▶</button><div class="seek" role="slider" aria-valuenow="10"></div><button aria-label="Full screen">⛶</button></div>
  </div>
  <script>
    window.clicks = 0;
    document.querySelector('video').addEventListener('click', () => window.clicks++);
  </script>
</body></html>"""


def home_html(last_service=None, start_with_last=False, lang="en", welcome=False):
    strings = catalog(lang)
    service = lambda i, n, c, a, d, key=None: {
        "id": i, "name": strings[key] if key else n, "nameKey": key, "caption": strings[c], "captionKey": c, "accent": a, "drm": d,
    }
    data = {
        "version": "0.8.0",
        "services": [service(*s) for s in SERVICES],
        "music": service("music", None, "service.music_caption", "#c2b3ff", False, key="service.music_name"),
        "lastService": last_service,
        "startWithLast": start_with_last,
        "welcome": welcome,
        "i18n": i18n(lang, "home"),
    }
    return fill("home.html", data, lang)


def main():
    failures = []

    def check(cond, name):
        print(("  ok   " if cond else "  FAIL ") + name)
        if not cond:
            failures.append(name)

    with sync_playwright() as p:
        browser = p.chromium.launch()

        def new_page(width=640, height=360, drm_stub=None, last=None, lang="en"):
            errors = []
            page = browser.new_page(viewport={"width": width, "height": height})
            page.on("console", lambda m: m.type == "error" and errors.append(m.text))
            page.on("pageerror", lambda e: errors.append(str(e)))
            page.route("https://ytmp.localhost/**", lambda r: r.fulfill(
                status=200, content_type="text/html; charset=utf-8",
                headers={"Content-Security-Policy": CSP}, body=home_html(last_service=last, lang=lang)))
            page.route("https://www.netflix.com/**", lambda r: r.fulfill(
                status=200, content_type="text/html; charset=utf-8", body=STREAMING_HTML))
            if drm_stub:
                page.add_init_script(drm_stub)
            add_app_scripts(page, maintenance=False)
            return page, errors

        def open_home(page, lang="en"):
            page.goto("https://ytmp.localhost/")
            # The app answers "ready" with the strings (App::push_locale).
            push_locale(page, lang, "home")
            page.wait_for_timeout(300)

        sent = lambda page: page.evaluate("window.__sent.slice()")
        clear = lambda page: page.evaluate("window.__sent.length = 0")

        def shot(page, name):
            if SHOTS:
                SHOTS.mkdir(parents=True, exist_ok=True)
                page.screenshot(path=str(SHOTS / f"{name}.png"))

        # ------------------------------------------------------------------
        print("Start screen")
        page, errors = new_page(last="crunchyroll")
        open_home(page)
        check(page.inner_text("h1") == text("en", "home.title"), f"title in English ({page.inner_text('h1')})")
        check(page.evaluate("document.documentElement.lang") == "en", "<html lang> says en")
        rows = page.locator(".key[data-id]")
        check(rows.count() == 9, f"shows the 8 services and My music ({rows.count()})")
        boxes = [rows.nth(i).bounding_box() for i in range(9)]
        check(all(abs(boxes[i]["y"] - boxes[i - i % 3]["y"]) < 1 for i in range(9))
              and boxes[0]["x"] < boxes[1]["x"] < boxes[2]["x"] and boxes[0]["y"] < boxes[3]["y"] < boxes[6]["y"],
              "laid out like a keypad: 1 2 3 / 4 5 6 / 7 8 9")
        music_row = page.locator(".key[data-id='music']")
        check(music_row.count() == 1 and rows.nth(8).get_attribute("data-id") == "music", "My music, last")
        together = page.locator("#together")
        check(together.count() == 1 and together.get_attribute("aria-label") == text("en", "home.together"), "Watch together, by the address")
        clear(page)
        together.click()
        check(sent(page) == ["together"], f"it opens the Watch together window ({sent(page)})")
        clear(page)
        page.click("#settings")
        check(sent(page) == ["settings"], f"the gear opens Settings ({sent(page)})")
        check(page.get_attribute("#settings", "aria-label") == text("en", "home.settings"), "and is named Settings")
        page.evaluate("scrollTo(0, 0)")
        check(music_row.inner_text().startswith("9") and "My music" in music_row.inner_text(), "with key 9 and its name")
        check(rows.nth(2).inner_text().startswith("3"), "each key carries its number")
        check(rows.nth(2).get_attribute("title") == "Open Netflix (press 3)", f"row tooltip ({rows.nth(2).get_attribute('title')})")
        check(page.locator(".key[data-id='crunchyroll'] .last").count() == 1, "marks the last service used")
        check(page.locator("#welcome").is_hidden(), "no welcome once it was seen")
        shot(page, "1-home")

        clear(page)
        rows.nth(2).click()
        check(sent(page) == ["service:netflix"], f"clicking Netflix sends 'service:netflix' ({sent(page)})")
        check(text("en", "home.opening") in rows.nth(2).inner_text(), "the row says it is opening")

        clear(page)
        page.keyboard.press("4")
        check(sent(page) == ["service:crunchyroll"], f"key 4 opens Crunchyroll ({sent(page)})")

        clear(page)
        page.keyboard.press("9")
        check(sent(page) == ["service:music"], f"key 9 opens My music ({sent(page)})")

        clear(page)
        page.click("#site-url")
        page.keyboard.type("plex.tv/web5")
        check(sent(page) == [], "typing an address does not trigger keys 1–8")
        page.keyboard.press("Enter")
        check(sent(page) == ["open-url:plex.tv/web5"], f"Enter opens the address ({sent(page)})")

        check(page.locator("#startup").count() == 0, "the start option moved to Settings")

        clear(page)
        box = page.locator("h1").bounding_box()
        page.mouse.move(box["x"] + 10, box["y"] + 8)
        page.mouse.down()
        page.mouse.up()
        check(sent(page) == ["drag"], f"the header moves the window ({sent(page)})")

        page.wait_for_timeout(200)
        state = page.locator("#drm").get_attribute("data-state")
        check(state == "missing", f"without Widevine in this Chromium, the DRM state is 'missing' ({state})")
        warn = page.locator(".key[data-id='netflix'] .caption.warn").count()
        check(warn == 1 and page.locator(".key[data-id='netflix'] .needs").count() == 1, "Netflix warns that it needs DRM (and a dot by its name)")
        check(page.locator(".key[data-id='youtube'] .caption.warn, .key[data-id='youtube'] .needs").count() == 0, "YouTube does not warn (it needs none)")
        check(page.locator("#recheck").is_visible(), "offers to check again")
        shot(page, "2-home-no-drm")

        page.evaluate(f"window.__ytmpSync({json.dumps(HOST_KEY)}, {{ startWithLast: false, lastService: 'netflix', tray: true }})")
        page.wait_for_timeout(100)
        check(page.locator(".key[data-id='netflix'] .last").count() == 1, "shows the new last service the app sends")

        page.mouse.move(630, 180)
        page.wait_for_timeout(300)
        bar_buttons = page.evaluate("""() => {
          const hits = new Set();
          for (let y = 0; y < innerHeight; y += 4) {
            const el = document.elementFromPoint(611, y);
            if (el && el.tagName === 'YTMP-UI') hits.add(y);
          }
          return hits.size;
        }""")
        check(bar_buttons > 0, "the app's side bar is on the start screen too")
        shot(page, "3-home-bar")
        check(not errors, f"no console or CSP errors ({errors[:3]})")
        page.close()

        # ------------------------------------------------------------------
        print("Language picker")
        page, errors = new_page()
        open_home(page)
        button = page.locator("#lang")
        check(button.inner_text().strip() == "English", f"the globe button shows the language's own name ({button.inner_text()!r})")
        check(button.get_attribute("title") == "Language", "and says 'Language' on hover")
        button.click()
        items = page.locator("#lang-menu [role='menuitemradio']")
        check(page.locator("#lang-menu").is_visible() and items.count() == len(LANGUAGES), f"opens a list of {len(LANGUAGES)} languages")
        names = [items.nth(i).locator(".native").inner_text() for i in range(items.count())]
        check(names == [l["name"] for l in LANGUAGES], f"each in its own language ({names})")
        check(items.nth(0).get_attribute("aria-checked") == "true", "the current one is checked")
        local = page.locator("#lang-menu [data-code='ja'] .local").inner_text()
        check(local == "Japanese", f"with the name in the current language too ({local!r})")
        check(page.evaluate("document.activeElement.dataset.code") == "en", "focus goes to the current language")
        page.keyboard.press("ArrowDown")
        check(page.evaluate("document.activeElement.dataset.code") == "es", "arrows move through the list")
        clear(page)
        page.keyboard.press("3")
        check(sent(page) == [], "number keys do not open services while the list is open")
        page.keyboard.press("Escape")
        check(page.locator("#lang-menu").is_hidden(), "Escape closes it")
        check(page.evaluate("document.activeElement.id") == "lang", "and gives focus back to the button")
        shot(page, "6-language-closed")

        button.click()
        shot(page, "7-language-open")
        clear(page)
        page.click("#lang-menu [data-code='ja']")
        check(sent(page) == ["lang:ja"], f"choosing Japanese sends 'lang:ja' ({sent(page)})")
        check(page.locator("#lang-menu").is_hidden(), "and closes the list")

        page.fill("#site-url", "plex.tv/web")
        push_locale(page, "ja", "home")
        page.wait_for_timeout(200)
        check(page.inner_text("h1") == text("ja", "home.title"), f"the page switches without reloading ({page.inner_text('h1')})")
        check(page.evaluate("document.documentElement.lang") == "ja", "<html lang> says ja (Japanese fonts)")
        check(button.inner_text().strip() == "日本語", "the button shows 日本語")
        check(button.get_attribute("title") == "言語 / Language", "and keeps 'Language' readable for anyone")
        check(page.input_value("#site-url") == "plex.tv/web", "what was typed stays")
        caption = page.locator(".key[data-id='youtube'] .caption").inner_text()
        check(caption == text("ja", "service.videos_live"), f"captions in Japanese ({caption})")
        music = page.locator(".key[data-id='music'] .name").inner_text()
        check(music == text("ja", "service.music_name"), f"My music in Japanese ({music})")
        title = page.locator(".key[data-id='netflix']").get_attribute("title")
        check(title == text("ja", "home.open_service", name="Netflix", key=3), f"tooltips in Japanese ({title})")
        drm = page.inner_text("#drm .text")
        check(drm == text("ja", "home.drm_missing"), f"the DRM line too ({drm})")
        check(page.get_attribute("#site-url", "placeholder") == text("ja", "home.other_site_placeholder"), "and the address placeholder")
        shot(page, "8-home-ja")
        check(not errors, f"no console errors ({errors[:3]})")
        page.close()

        for lang in ("es", "ru", "pl"):
            page, errors = new_page(lang=lang)
            open_home(page, lang)
            check(page.inner_text("h1") == text(lang, "home.title"), f"{lang}: served in that language ({page.inner_text('h1')})")
            for w, h in [(640, 360), (360, 220)]:
                page.set_viewport_size({"width": w, "height": h})
                page.wait_for_timeout(100)
                overflow = page.evaluate("document.documentElement.scrollWidth > innerWidth")
                check(not overflow, f"{lang} {w}×{h}: no horizontal scrolling")
                shot(page, f"9-home-{lang}-{w}x{h}")
            check(not errors, f"{lang}: no console errors ({errors[:3]})")
            page.close()

        # ------------------------------------------------------------------
        print("With DRM available")
        stub = """
        navigator.requestMediaKeySystemAccess = (ks) => ks === 'com.widevine.alpha' || ks.includes('playready')
          ? Promise.resolve({}) : Promise.reject(new Error('no'));
        """
        page, errors = new_page(drm_stub=stub)
        open_home(page)
        check(page.locator("#drm").get_attribute("data-state") == "ok", "the DRM state is 'ok'")
        check("Widevine and PlayReady" in page.inner_text("#drm .text"), f"names the systems with the language's 'and' ({page.inner_text('#drm .text')})")
        check(page.locator(".caption.warn").count() == 0, "no warnings on the services")
        push_locale(page, "pl", "home")
        page.wait_for_timeout(100)
        check("Widevine i PlayReady" in page.inner_text("#drm .text"), f"in Polish, 'i' ({page.inner_text('#drm .text')})")
        page.close()

        print("Small window")
        for w, h in [(360, 220), (256, 144)]:
            page, errors = new_page(width=w, height=h)
            open_home(page)
            overflow = page.evaluate("document.documentElement.scrollWidth > innerWidth")
            check(not overflow, f"{w}×{h}: no horizontal scrolling")
            shot(page, f"4-home-{w}x{h}")
            page.close()

        # ------------------------------------------------------------------
        print("Another service (generic player)")
        page, errors = new_page()
        page.goto("https://www.netflix.com/watch/1")
        push_locale(page, "en")
        page.wait_for_timeout(300)

        clear(page)
        page.mouse.move(300, 150)
        page.mouse.down()
        page.mouse.move(320, 165, steps=4)
        page.mouse.up()
        check(sent(page) == ["drag"], f"dragging the video moves the window ({sent(page)})")
        check(page.evaluate("window.clicks") == 0, "and the video gets no click")

        clear(page)
        page.mouse.click(300, 150)
        check(sent(page) == [], "a click without moving does not drag")
        check(page.evaluate("window.clicks") == 1, "and reaches the video (pause/play)")

        clear(page)
        page.mouse.move(300, 336)  # progress bar (role=slider)
        page.mouse.down()
        page.mouse.move(340, 336, steps=4)
        page.mouse.up()
        check(sent(page) == [], "dragging the progress bar does not move the window")

        page.mouse.move(630, 180)
        page.wait_for_timeout(300)
        shot(page, "5-service-bar")
        clear(page)
        # The main menu button is the bar's second one (after the handle).
        ys = [y for y in range(0, 360, 2) if page.evaluate(f"document.elementFromPoint(611, {y})?.tagName") == "YTMP-UI"]
        page.mouse.click(611, ys[0] + 5 + 16 + 34)
        check(sent(page) == ["home"], f"the main menu button sends 'home' ({sent(page)})")

        clear(page)
        page.mouse.move(300, 150)
        page.keyboard.press("Alt+Home")
        check(sent(page) == ["home"], f"Alt+Home goes back to the menu ({sent(page)})")

        clear(page)
        page.keyboard.press("Control+m")
        check(sent(page) == [], "Ctrl+M (video only) does nothing outside YouTube")
        check(page.evaluate("typeof window.__ytmpHomeSend") == "undefined", "other sites do not get the start screen's channel")
        check(not errors, f"no console errors ({errors[:3]})")
        browser.close()

    print()
    if failures:
        print(f"{len(failures)} test(s) failed")
        sys.exit(1)
    print("home.html + services: all tests pass")


if __name__ == "__main__":
    main()
