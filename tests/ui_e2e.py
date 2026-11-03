"""End-to-end test of inject.js + ui.js in Chromium (Playwright).

Serves a page that imitates the structure of YouTube's player at
https://www.youtube.com/watch, with the Trusted Types policy YouTube uses,
and injects the scripts just like the app. A simulated "host" in Python
answers the IPC messages the way app.rs does, strings included.

Usage:    python tests/ui_e2e.py [folder_for_screenshots]
Requires: pip install playwright && playwright install chromium
"""

import json
import pathlib
import sys

from playwright.sync_api import sync_playwright

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from bundle import HOST_KEY, add_app_scripts, push_locale, text  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
SHOTS = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else None
EXT_ID = "cjpalhdlnbpafiamejdnhcphjbkeiagm"

MOCK_HTML = """<!doctype html>
<html><head><meta charset="utf-8"><title>Video - YouTube</title>
<style>
  /* Structure and stacking "traps" like YouTube's: fixed header, 56 px
     top margin, two columns, containers with their own z-index and a
     transform that changes the containing block. */
  body { margin: 0; background: #0f0f0f; color: #fff; font-family: sans-serif; }
  #masthead-container { position: fixed; top: 0; left: 0; right: 0; z-index: 2020; height: 56px; background: #212121; display: flex; align-items: center; padding: 0 16px; }
  #masthead-container input { width: 50%; }
  ytd-page-manager { display: block; margin-top: 56px; }
  ytd-watch-flexy { display: block; transform: translateZ(0); }
  #columns { display: flex; gap: 16px; padding: 16px; }
  #primary { flex: 1; min-width: 0; }
  #player { position: relative; z-index: 0; }
  #movie_player { position: relative; width: 100%; aspect-ratio: 16/9; background: #000; overflow: hidden; }
  #movie_player video { position: absolute; left: 0; top: 0; width: 100%; height: 100%; background: linear-gradient(135deg,#2b3a67,#0f0f0f); }
  .ytp-chrome-bottom { position: absolute; left: 12px; right: 12px; bottom: 0; height: 40px; display: flex; align-items: center; justify-content: flex-end; }
  .ytp-fullscreen-button { width: 40px; height: 32px; }
  #below { position: relative; z-index: 1; padding: 16px 0; }
  #secondary { position: relative; z-index: 1; width: 160px; }
  #secondary .thumb { height: 70px; margin-bottom: 8px; background: #c33; }
</style></head>
<body>
  <ytd-app>
    <div id="masthead-container"><input id="search" placeholder="Search"></div>
    <ytd-page-manager>
      <ytd-watch-flexy>
        <div id="columns">
          <div id="primary">
            <div id="player">
              <div id="movie_player" class="html5-video-player">
                <div class="html5-video-container"><video class="html5-main-video"></video></div>
                <div class="ytp-chrome-bottom"><button class="ytp-fullscreen-button">⛶</button></div>
              </div>
            </div>
            <div id="below"><h1 id="title">Video title</h1><p>Description and comments…</p></div>
          </div>
          <div id="secondary"><div class="thumb"></div><div class="thumb"></div><div class="thumb"></div></div>
        </div>
      </ytd-watch-flexy>
    </ytd-page-manager>
  </ytd-app>
<script>
  // Minimal imitation of YouTube's player.
  window.playerClicks = 0;
  const player = document.getElementById('movie_player');
  let st = 1;
  player.addEventListener('click', (e) => { if (!e.target.closest('.ytp-chrome-bottom')) window.playerClicks++; });
  player.getPlayerState = () => st;
  player.pauseVideo = () => { st = 2; window.lastMedia = 'pause'; };
  player.playVideo = () => { st = 1; window.lastMedia = 'play'; };
  player.nextVideo = () => { window.lastMedia = 'next'; };
  player.previousVideo = () => { window.lastMedia = 'previous'; };
  document.querySelector('.ytp-fullscreen-button').addEventListener('click', () => {
    if (document.fullscreenElement) document.exitFullscreen(); else player.requestFullscreen();
  });
</script>
</body></html>"""

def main():
    failures = []
    console_errors = []

    def check(cond, name):
        print(("  ok   " if cond else "  FAIL ") + name)
        if not cond:
            failures.append(name)

    host = {
        "version": "0.8.0", "pinned": True, "videoOnly": False, "opacity": 100,
        "ghost": False, "fullscreen": False, "autostart": False, "tray": True,
        "extensions": [
            {"folder": "sponsorblock", "name": "SponsorBlock", "version": "5.9", "id": EXT_ID,
             "enabled": True, "hasPage": True, "error": None, "pending": False},
            {"folder": "broken", "name": "Broken extension", "version": "1.0", "id": None,
             "enabled": False, "hasPage": False, "error": "Invalid manifest (0x80070057)", "pending": False},
        ],
        "extensionProblems": ["“empty” has no manifest.json"],
        "extensionsError": None,
        "hotkeys": [
            {"keys": "Ctrl+Alt+Y", "label": "Show or hide the player", "available": True, "note": None},
            {"keys": "Ctrl+Alt+G", "label": "Ghost mode", "available": False, "note": "another program uses it"},
        ],
    }

    with sync_playwright() as p:
        browser = p.chromium.launch()
        page = browser.new_page(viewport={"width": 640, "height": 360})
        page.on("console", lambda m: m.type == "error" and console_errors.append(m.text))
        page.on("pageerror", lambda e: console_errors.append(str(e)))

        def serve(route):
            route.fulfill(
                status=200,
                content_type="text/html; charset=utf-8",
                headers={"Content-Security-Policy": "require-trusted-types-for 'script'; trusted-types 'none'"},
                body=MOCK_HTML,
            )

        page.route("https://www.youtube.com/**", serve)
        # Same wrapper and order as webview.rs.
        add_app_scripts(page)

        def sent():
            """What the page sent, apart from the menu's open/closed reports
            (checked on their own below)."""
            return [m for m in page.evaluate("window.__sent.slice()")
                    if not m.startswith("menu-open:") and not m.startswith("page-check:")]

        def page_checks():
            return [json.loads(m.split(":", 1)[1]) for m in page.evaluate("window.__sent.slice()")
                    if m.startswith("page-check:")]

        def menu_reports():
            return [m for m in page.evaluate("window.__sent.slice()") if m.startswith("menu-open:")]

        def clear():
            page.evaluate("window.__sent.length = 0")

        def sync():
            page.evaluate(f"window.__ytmpSync({json.dumps(HOST_KEY)}, {json.dumps(host)})")

        cdp = page.context.new_cdp_session(page)

        def button_names():
            """Accessible names of the buttons (the bar lives in a closed shadow root)."""
            tree = cdp.send("Accessibility.getFullAXTree")
            return [n.get("name", {}).get("value") for n in tree["nodes"] if n.get("role", {}).get("value") == "button"]

        def all_names():
            tree = cdp.send("Accessibility.getFullAXTree")
            return [n.get("name", {}).get("value") or "" for n in tree["nodes"]]

        def click_named(predicate):
            """Clicks the element whose accessible name matches, wherever the
            closed shadow root put it. False if there is none."""
            tree = cdp.send("Accessibility.getFullAXTree")
            for node in tree["nodes"]:
                name = node.get("name", {}).get("value") or ""
                if predicate(name) and node.get("backendDOMNodeId"):
                    cdp.send("DOM.scrollIntoViewIfNeeded", {"backendNodeId": node["backendDOMNodeId"]})
                    quad = cdp.send("DOM.getBoxModel", {"backendNodeId": node["backendDOMNodeId"]})["model"]["border"]
                    page.mouse.click((quad[0] + quad[2]) / 2, (quad[1] + quad[5]) / 2)
                    return True
            return False

        def press_layout_key(key, code, vk, ctrl=True):
            """A real key press as a given keyboard layout produces it (CDP)."""
            for kind in ("rawKeyDown", "keyUp"):
                cdp.send("Input.dispatchKeyEvent", {"type": kind, "modifiers": 2 if ctrl else 0, "key": key, "code": code,
                                                    "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk})

        def shot(name):
            if SHOTS:
                SHOTS.mkdir(parents=True, exist_ok=True)
                page.screenshot(path=str(SHOTS / f"{name}.png"))

        page.goto("https://www.youtube.com/watch?v=test")
        page.wait_for_timeout(300)

        print("Startup")
        check("ready" in sent(), "the page sends 'ready'")
        page.wait_for_timeout(250)
        size = page.evaluate("innerWidth + 'x' + innerHeight")
        check(f"viewport:{size}" in sent(), f"…and its size in CSS px, for a guest's clicks ({size})")
        page.set_viewport_size({"width": 700, "height": 420})
        page.wait_for_timeout(300)
        check("viewport:700x420" in sent(), "…again after a resize")
        page.set_viewport_size({"width": 640, "height": 360})
        page.wait_for_timeout(300)
        push_locale(page, "en")
        sync()
        check(page.evaluate("!!document.querySelector('ytmp-ui')"), "the interface is mounted in the document")
        names = button_names()
        check("Always on top (Ctrl+T)" in names and "Main menu (Alt+Home)" in names, f"bar tooltips in English ({names[:4]})")

        print("Side bar")
        page.mouse.move(320, 180)
        page.wait_for_timeout(1700)
        bar_hidden = page.evaluate("""() => {
          const t = document.querySelector('ytmp-ui');
          return getComputedStyle(t).display !== 'none';
        }""")
        page.mouse.move(630, 180)
        page.wait_for_timeout(300)
        shot("1-bar")
        visible_at = page.evaluate("document.elementFromPoint(620, 180)?.tagName")
        check(visible_at == "YTMP-UI", "the bar shows up near the right edge")

        clear()
        clicks_before = page.evaluate("window.playerClicks")
        page.mouse.move(611, 180)
        # Buttons by their position on the bar (vertically centred).
        boxes = page.evaluate("""() => {
          // The shadow root is closed: the buttons are found by coordinates.
          const out = [];
          for (let y = 0; y < innerHeight; y += 2) {
            const el = document.elementFromPoint(611, y);
            if (el && el.tagName === 'YTMP-UI') out.push(y);
          }
          return [out[0], out[out.length - 1]];
        }""")
        top, bottom = boxes
        check(top is not None and bottom - top > 200, f"bar of a sensible height ({top}-{bottom})")

        # Order: handle, home, back, | video only, full screen, pin, hide, | settings, more, close
        # Each button 32 px + 2 px gap; separators of 7 px.
        y0 = top + 5 + 16
        ys = {}
        y = y0
        for key in ["grip", "home", "back", "sep", "videoOnly", "fullscreen", "pinned", "hide", "sep", "settings", "more", "quit"]:
            if key == "sep":
                y += 7 + 2
                continue
            ys[key] = y
            y += 34

        page.mouse.click(611, ys["pinned"])
        check(sent() == ["pin"], f"the pin button sends 'pin' ({sent()})")
        check(page.evaluate("window.playerClicks") == clicks_before, "clicks on the bar do not reach the player")

        clear()
        page.mouse.move(611, ys["grip"])
        page.mouse.down()
        page.mouse.up()
        check("drag" in sent(), "the handle sends 'drag'")

        clear()
        page.mouse.click(611, ys["settings"])
        check(sent() == ["settings"], f"the gear opens Settings ({sent()})")
        clear()
        page.keyboard.press("Control+Comma")
        check(sent() == ["settings"], f"so does Ctrl+, ({sent()})")

        print("More menu")
        clear()
        page.mouse.move(611, ys["more"])
        page.mouse.click(611, ys["more"])
        page.wait_for_timeout(200)
        check(menu_reports() == ["menu-open:1"], f"the app is told the menu is open (guests cannot click it) ({menu_reports()})")
        shot("2-menu")
        names = all_names()
        check(any(n.startswith(text("en", "ui.settings_all")) for n in names), "with a Settings row")
        check(any(n.startswith("Always on top") for n in names) and not any(n.startswith("Start with Windows") for n in names),
              "menu open, with the everyday rows (the rest is in Settings)")
        check(any(n.startswith("Language") for n in names), "with a Language row")
        problems = len(host["extensionProblems"]) + sum(1 for e in host["extensions"] if e["error"])
        check(any(n.startswith(text("en", "ui.section.extensions")) and text("en", "ui.ext.problems", count=problems) in n for n in names)
              and not any("SponsorBlock" in n for n in names), "extensions summed up in one row (with the ones that need attention)")
        clear()
        check(click_named(lambda n: n.startswith(text("en", "ui.section.extensions"))), "the Extensions row")
        check(sent() == ["settings:extensions", "menu-open:0"] or sent() == ["settings:extensions"],
              f"opens Settings at Extensions ({sent()})")
        page.mouse.click(611, ys["more"])
        page.wait_for_timeout(150)
        # The menu's first item is "Always on top": 8 px from the top edge.
        clear()
        page.mouse.click(400, 8 + 6 + 22 + 16)  # Window section, first row
        check(sent() == ["pin"], f"the 'Always on top' row sends 'pin' ({sent()})")

        clear()
        check(click_named(lambda n: n == text("en", "ui.together")), "with a Watch together row")
        check(sent() == ["together"], f"which opens its window ({sent()})")
        check(menu_reports() == ["menu-open:0"], f"…and the menu closed is reported once ({menu_reports()})")
        # During a session, the row says so.
        page.mouse.click(611, ys["more"])
        page.wait_for_timeout(150)
        host["together"] = "guest"
        sync()
        page.wait_for_timeout(100)
        check(any(n.startswith(text("en", "ui.together")) and text("en", "ui.together_watching") in n for n in all_names()),
              "in a session, the row says so")
        host["together"] = None
        sync()

        clear()
        page.keyboard.press("Escape")
        page.wait_for_timeout(100)
        page.mouse.click(320, 300)
        check(page.evaluate("window.playerClicks") >= clicks_before, "after closing the menu, the page gets clicks again")

        print("Dragging the video")
        page.mouse.move(10, 10)
        page.wait_for_timeout(1500)
        clear()
        before = page.evaluate("window.playerClicks")
        page.mouse.move(200, 150)
        page.mouse.down()
        page.mouse.move(215, 160, steps=4)
        page.mouse.up()
        page.wait_for_timeout(50)
        check(sent() == ["drag"], f"dragging the video sends 'drag' ({sent()})")
        check(page.evaluate("window.playerClicks") == before, "dragging does not pause the video")

        clear()
        before = page.evaluate("window.playerClicks")
        page.mouse.click(200, 150)
        check(sent() == [], f"a click without moving does not drag ({sent()})")
        check(page.evaluate("window.playerClicks") == before + 1, "a click without moving reaches the player")

        clear()
        page.mouse.move(200, 30)  # search bar: not video
        page.mouse.down()
        page.mouse.move(240, 40, steps=4)
        page.mouse.up()
        check(sent() == [], "dragging outside the video does not move the window")

        clear()
        before = page.evaluate("window.playerClicks")
        page.keyboard.down("Alt")
        page.mouse.click(200, 150)
        page.keyboard.up("Alt")
        check(sent() == ["drag"], "Alt+click sends 'drag'")
        check(page.evaluate("window.playerClicks") == before, "Alt+click does not pause")

        clear()
        page.mouse.move(639, 200)
        page.mouse.down()
        page.mouse.up()
        check(sent() == ["resize:e"], f"the right edge sends 'resize:e' ({sent()})")

        print("Shortcuts")
        page.mouse.move(320, 180)
        for keys, expected in [
            ("Control+t", "pin"), ("Control+m", "video-only"), ("Control+h", "hide"),
            ("Control+l", "open-clipboard"), ("Control+2", "size:m"), ("Control+Shift+4", "corner:se"),
            ("Control+q", "quit"),
        ]:
            clear()
            page.keyboard.press(keys)
            check(sent() == [expected], f"{keys} → {expected} ({sent()})")

        # Other keyboard layouts: Ctrl+T on a Russian keyboard is typed on the
        # "Е" key (same position); on Dvorak, T is somewhere else.
        clear()
        press_layout_key("е", "KeyT", 84)
        check(sent() == ["pin"], f"Russian layout: Ctrl+Е (the T key) → pin ({sent()})")
        clear()
        press_layout_key("т", "KeyN", 78)
        check(sent() == [], f"Russian layout: Ctrl+Т (the N key) does nothing ({sent()})")
        clear()
        press_layout_key("t", "KeyK", 84)
        check(sent() == ["pin"], f"Dvorak: the key labelled T → pin ({sent()})")
        clear()
        press_layout_key("k", "KeyT", 75)
        check(sent() == [], f"Dvorak: the key in T's position types K → nothing ({sent()})")

        clear()
        page.keyboard.press("F1")
        page.wait_for_timeout(150)
        shot("3-help")
        covers = page.evaluate("document.elementFromPoint(20, 20)?.tagName")
        check(covers == "YTMP-UI", "F1 opens the help")
        names = all_names()
        check("Keyboard shortcuts" in names and "Ghost mode (another program uses it)" in names,
              "the help lists the global shortcuts and why one is unavailable")
        check("Alt+Home" in names, "with the Home key's name")
        page.keyboard.press("Escape")
        page.wait_for_timeout(100)
        check(page.evaluate("document.elementFromPoint(20, 20)?.tagName") != "YTMP-UI", "Esc closes the help")

        # Shortcuts do not fire while typing a single letter in the search box.
        clear()
        page.click("#search")
        page.keyboard.type("hello tm")
        check(sent() == [], "typing in the search box does not trigger shortcuts")
        check(page.input_value("#search") == "hello tm", "the text reaches the search box")

        print("Language")
        page.mouse.move(320, 180)
        push_locale(page, "es")
        page.wait_for_timeout(100)
        names = button_names()
        check("Menú principal (Alt+Inicio)" in names, f"Spanish: the Home key is called Inicio, as printed on the keyboard ({names[1]})")
        push_locale(page, "ja")
        page.wait_for_timeout(100)
        names = button_names()
        check("常に手前に表示 (Ctrl+T)" in names, f"Japanese tooltips ({names[:3]})")
        page.mouse.move(630, 180)
        page.wait_for_timeout(300)
        page.mouse.click(611, ys["more"])
        page.wait_for_timeout(200)
        names = all_names()
        check("ウィンドウ" in names and "アプリ" in names, "the menu follows the new language")
        check("言語 / Language" in names, "the language row says 'Language' in English too")
        shot("6-menu-ja")
        # Open the language list and pick Polish.
        check(click_named(lambda n: n.startswith("言語 / Language")), "the language row can be clicked")
        page.wait_for_timeout(100)
        names = all_names()
        check(all(any(n.startswith(l) for n in names) for l in ("English", "Español", "Русский", "Polski", "日本語")),
              "it opens a list with each language's own name")
        check(any("ポーランド語" in n for n in names), "and the name in the current language (ポーランド語)")
        shot("7-menu-languages")
        clear()
        click_named(lambda n: n.startswith("Polski"))
        check(sent() == ["lang:pl"], f"choosing Polish sends lang:pl ({sent()})")
        page.keyboard.press("Escape")
        push_locale(page, "ru")
        page.wait_for_timeout(100)
        page.keyboard.press("F1")
        page.wait_for_timeout(100)
        names = all_names()
        check("Сочетания клавиш" in names and "Перемещает окно из любой точки" in names, "the help in Russian")
        shot("8-help-ru")
        page.keyboard.press("Escape")
        push_locale(page, "en")
        page.wait_for_timeout(100)

        print("Video only")
        host["videoOnly"] = True
        sync()
        page.wait_for_timeout(250)
        rect = page.evaluate("(() => { const r = document.getElementById('movie_player').getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; })()")
        check(rect == [0, 0, 640, 360], f"the player fills the whole window ({rect})")
        in_player = """([x, y]) => { const el = document.elementFromPoint(x, y);
          return !!el && !!el.closest('#movie_player'); }"""
        for name, point in [("header", (300, 20)), ("title", (60, 330)), ("recommendations", (560, 120)),
                            ("corner", (5, 355))]:
            check(page.evaluate(in_player, list(point)), f"video only: the player shows in the {name} area")
        page.mouse.move(630, 180)
        page.wait_for_timeout(300)
        shot("4-video-only")
        check(page.evaluate("document.elementFromPoint(611, 180)?.tagName") == "YTMP-UI",
              "video only: the bar stays above the player")
        clear()
        before = page.evaluate("window.playerClicks")
        page.mouse.click(540, 120)
        check(page.evaluate("window.playerClicks") == before + 1, "video only: a click where the recommendations were reaches the player")

        print("Video only + full screen")
        page.evaluate("document.getElementById('movie_player').requestFullscreen()")
        page.wait_for_timeout(300)
        check(not page.evaluate("document.documentElement.classList.contains('ytmp-video-only')"),
              "in full screen the video-only CSS is turned off")
        page.evaluate("document.exitFullscreen()")
        page.wait_for_timeout(300)
        check(page.evaluate("document.documentElement.classList.contains('ytmp-video-only')"),
              "leaving full screen brings video only back")
        check(page.evaluate(in_player, [60, 330]), "and the content stays hidden")

        host["videoOnly"] = False
        sync()
        page.wait_for_timeout(100)
        check(not page.evaluate("document.documentElement.classList.contains('ytmp-video-only')"), "turned off when synced")
        check(page.evaluate("!!document.elementFromPoint(300, 20)?.closest('#masthead-container')")
              and page.evaluate("!!document.elementFromPoint(560, 120)?.closest('#secondary')"),
              "without video only, the page is back to normal")

        print("Full screen")
        clear()
        page.mouse.move(630, 180)
        page.wait_for_timeout(300)
        page.mouse.click(611, ys["fullscreen"])
        page.wait_for_timeout(400)
        fs = page.evaluate("document.fullscreenElement?.id")
        check(fs == "movie_player", f"the button puts the player in full screen ({fs})")
        check("fullscreen:1" in sent(), f"tells the host ({sent()})")
        check(page.evaluate("document.querySelector('ytmp-ui').parentElement.id") == "movie_player",
              "the interface moves inside the full-screen element")
        clear()
        page.keyboard.press("F11")
        page.wait_for_timeout(400)
        check(page.evaluate("document.fullscreenElement") is None, "F11 leaves full screen")
        check("fullscreen:0" in sent(), "tells the host it left")

        print("Notices and media")
        page.evaluate(f"window.__ytmpToast({json.dumps(HOST_KEY)}, 'Always on top: on')")
        page.wait_for_timeout(250)
        shot("5-notice")
        page.evaluate("window.__ytmpMedia('toggle')")
        check(page.evaluate("window.lastMedia") == "pause", "Ctrl+Alt+P pauses through the player API")
        page.evaluate("window.__ytmpMedia('next')")
        check(page.evaluate("window.lastMedia") == "next", "Ctrl+Alt+N goes to the next one")

        print("Self-check (the page still has what the app relies on)")
        page.evaluate("document.exitFullscreen().catch(() => {})")
        page.wait_for_timeout(200)
        # YouTube renames its player (and its styles with it): found anyway,
        # and reported.
        page.evaluate("""(() => { const p = document.getElementById('movie_player');
          p.style.cssText = 'position: relative; width: 100%; aspect-ratio: 16/9; background: #000; overflow: hidden';
          p.querySelector('video').style.cssText = 'position: absolute; left: 0; top: 0; width: 100%; height: 100%';
          p.id = 'renamed_player'; p.className = 'new-player'; })()""")
        clear()
        page.evaluate("document.querySelector('video').dispatchEvent(new Event('playing'))")
        page.wait_for_timeout(700)
        renamed = page_checks()
        check(len(renamed) == 1 and renamed[0]["player"] == "video" and not renamed[0]["api"]
              and not renamed[0]["fullscreenButton"], f"a renamed player is reported, with what is missing ({renamed})")
        host["videoOnly"] = True
        sync()
        page.wait_for_timeout(250)
        rect = page.evaluate("(() => { const r = document.querySelector('[data-ytmp-player]').getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; })()")
        check(rect == [0, 0, 640, 360], f"video only still fills the window with the video's box ({rect})")
        check(page.evaluate("document.querySelectorAll('[data-ytmp-player]').length") == 1, "only one element is marked")
        host["videoOnly"] = False
        sync()
        page.wait_for_timeout(150)
        page.mouse.move(10, 10)
        page.wait_for_timeout(1500)
        clear()
        page.mouse.move(200, 150)
        page.mouse.down()
        page.mouse.move(215, 160, steps=4)
        page.mouse.up()
        page.wait_for_timeout(50)
        check(sent() == ["drag"], f"the renamed player's video still drags the window ({sent()})")
        page.mouse.move(630, 180)
        page.wait_for_timeout(300)
        page.mouse.click(611, ys["fullscreen"])
        page.wait_for_timeout(400)
        check(page.evaluate("!!document.fullscreenElement"), "full screen still works without YouTube's button")
        page.evaluate("document.exitFullscreen().catch(() => {})")
        page.wait_for_timeout(300)
        page.evaluate("""(() => { const p = document.getElementById('renamed_player');
          p.id = 'movie_player'; p.className = 'html5-video-player';
          p.style.cssText = ''; p.querySelector('video').style.cssText = ''; })()""")
        clear()
        page.evaluate("document.querySelector('video').dispatchEvent(new Event('playing'))")
        page.wait_for_timeout(700)
        healthy = {"site": "youtube", "page": "watch", "player": "main", "api": True, "video": True, "fullscreenButton": True}
        check(page_checks() == [healthy], f"when a video plays, it reports everything found ({page_checks()})")
        clear()
        page.evaluate("document.querySelector('video').dispatchEvent(new Event('playing'))")
        page.wait_for_timeout(700)
        check(page_checks() == [], "the same result is not sent twice")

        print("Robustness")
        check(page.evaluate("typeof window.__ytmpLocale") == "function" and
              page.evaluate("window.__ytmpLocale('wrong-key', {lang: 'ja', ui: {}}) === undefined"), "a wrong key cannot change the strings")
        check("Always on top (Ctrl+T)" in button_names(), "the strings stay as they were")
        check(not console_errors, f"no console or Trusted Types errors ({console_errors[:3]})")
        browser.close()

    print()
    if failures:
        print(f"{len(failures)} test(s) failed")
        sys.exit(1)
    print("ui.js + inject.js: all tests pass")


if __name__ == "__main__":
    main()
