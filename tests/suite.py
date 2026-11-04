"""YT Mini Player: the experience test suite (400+ automated tests).

One file, one command:

    python tests/suite.py                    # everything
    python tests/suite.py -k settings        # only tests whose name has "settings"
    python tests/suite.py --category security
    python tests/suite.py --report tests/report   # also writes report.md / report.json

It drives the app's own pages in Chromium (Playwright) exactly as the app
serves them (home.rs: strings embedded, the same Content-Security-Policy),
injects the side bar the way webview.rs does, and plays the app's part
(App::push_settings, App::push_locale, __ytmpSync). It also reads the Rust
sources to check the contract between the pages and the app (every message
a page can send is one ipc.rs accepts).

Categories:
    unit         small pieces of logic (plurals, placeholders, key caps, data)
    contract     pages ↔ app: messages, data, prefixes, served pages
    e2e          what a person does: start screen, welcome, Settings, side bar
    security     CSP, no HTML injection, forged messages, closed functions
    a11y         names, roles, focus, keyboard, contrast, target sizes, motion
    performance  weight, DOM size, render time, throttling, idle work
    layout       every language at the window's real sizes: nothing cut or
                 scrolling sideways
    i18n         every new text in the five languages, with the same
                 placeholders and key caps

Requires: pip install playwright && playwright install chromium
"""

import argparse
import base64
import hashlib
import json
import pathlib
import re
import sys
import time
import traceback

from playwright.sync_api import sync_playwright

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
from bundle import (  # noqa: E402
    HOST_KEY, IPC_STUB, LANGUAGES, PREFIXES, SRC, add_app_scripts, catalog, fill, i18n, push_locale, strings, text,
)
from home_e2e import home_html  # noqa: E402

LANGS = [l["code"] for l in LANGUAGES]
SECTIONS = ["general", "window", "together", "privacy", "extensions", "advanced"]
EXT_ID = "cjpalhdlnbpafiamejdnhcphjbkeiagm"
EXT_ID_2 = "abcdefghijklmnopabcdefghijklmnop"

# ---------------------------------------------------------------------------
# What the app serves (read from the Rust sources, not copied)
# ---------------------------------------------------------------------------

HOME_RS = (SRC / "home.rs").read_text(encoding="utf-8")
IPC_RS = (SRC / "ipc.rs").read_text(encoding="utf-8")
APP_RS = (SRC / "app.rs").read_text(encoding="utf-8")


def rust_const(name):
    """A `const NAME: &str = "…" \\ "…";` string, joined as Rust does."""
    match = re.search(r'const %s: &str = "(.*?)";' % name, HOME_RS, re.S)
    raw = match.group(1)
    return re.sub(r"\\\n\s*", "", raw)


CSP = rust_const("CSP")
MINI_CSP = rust_const("MINI_CSP")


def script_hash(html):
    """home::script_hash: sha256 of the last inline <script>."""
    start = html.rfind("<script>") + len("<script>")
    end = html.rfind("</script>")
    return "sha256-" + base64.b64encode(hashlib.sha256(html[start:end].encode("utf-8")).digest()).decode()


def music_csp():
    body = re.search(r'fn music_csp\(\).*?format!\(\s*"(.*?)",\s*script_hash', HOME_RS, re.S).group(1)
    body = re.sub(r"\\\n\s*", "", body)
    music = (SRC / "music.html").read_text(encoding="utf-8")
    return body.replace("'{}'", "'" + script_hash(music) + "'")


# The settings window's messages ipc::parse_settings accepts (read from its arms).
def settings_arms():
    body = re.search(r"pub fn parse_settings\(.*?\n}\n", IPC_RS, re.S).group(0)
    return set(re.findall(r'\("([a-z-]+)",', body))


SETTINGS_ARMS = settings_arms()

# The state App::push_settings sends (field names, camelCase).
def settings_view_fields():
    body = re.search(r"struct SettingsView<'a> \{(.*?)\n\}", APP_RS, re.S).group(1)
    names = re.findall(r"^\s+(\w+):", body, re.M)
    return [re.sub(r"_(\w)", lambda m: m.group(1).upper(), n) for n in names]


def settings_state(**overrides):
    state = {
        "version": "0.11.0", "webview": "130.0.2849.80", "section": None, "pinned": True, "opacity": 85,
        "ghost": False, "videoOnly": False, "fullscreenInWindow": False, "taskbarBar": True, "autostart": False, "startWithLast": False,
        "tray": True, "hardwareAcceleration": True, "hardwareAccelerationNow": True, "lock": "off",
        "overlay": False, "togetherName": "Rodrigo", "togetherMode": "share", "togetherActive": False,
        "extensions": [
            {"folder": "sponsorblock", "name": "SponsorBlock", "version": "5.9", "id": EXT_ID, "enabled": True,
             "hasPage": True, "error": None, "pending": False},
            {"folder": "ublock", "name": "uBlock Origin Lite", "version": "2025.1", "id": EXT_ID_2, "enabled": False,
             "hasPage": False, "error": None, "pending": False},
        ],
        "extensionProblems": [], "extensionsError": None,
        "hotkeys": [
            {"keys": "Ctrl+Alt+Space", "label": "Show or hide the player", "available": True, "note": None},
            {"keys": "Ctrl+Alt+P", "label": "Play / pause", "available": True, "note": None},
            {"keys": "Ctrl+Alt+G", "label": "Ghost mode", "available": False, "note": "another app uses it"},
        ],
    }
    state.update(overrides)
    return state


SIDEBAR_PAGE = """<!doctype html><html><head><meta charset="utf-8"><title>Video - YouTube</title><style>
  body { margin: 0; background: #0f0f0f; color: #fff; font-family: sans-serif; }
  #movie_player { position: fixed; inset: 0; background: linear-gradient(135deg,#2b3a67,#0f0f0f); }
</style></head><body><div id="movie_player" class="html5-video-player"><video></video></div>
<script>window.playerClicks = 0; document.getElementById('movie_player').addEventListener('click', () => window.playerClicks++);</script>
</body></html>"""

SIDEBAR_HOST = {
    "version": "0.11.0", "pinned": True, "videoOnly": False, "fullscreenInWindow": False, "opacity": 100, "ghost": False, "fullscreen": False,
    "autostart": False, "tray": True, "overlay": False, "hardwareAcceleration": True, "together": None,
    "extensions": [{"folder": "sponsorblock", "name": "SponsorBlock", "version": "5.9", "id": EXT_ID, "enabled": True,
                    "hasPage": True, "error": None, "pending": False}],
    "extensionProblems": [], "extensionsError": None, "hotkeys": [],
}

# ---------------------------------------------------------------------------
# A very small test runner
# ---------------------------------------------------------------------------

REGISTRY = []


class Fail(AssertionError):
    pass


def expect(cond, message):
    if not cond:
        raise Fail(message)


def test(category, name):
    def register(fn):
        REGISTRY.append({"category": category, "name": name, "fn": fn})
        return fn
    return register


def each(category, template, values):
    """Registers one test per value: `template` gets the value(s) with format()."""
    def register(fn):
        for value in values:
            args = value if isinstance(value, tuple) else (value,)
            REGISTRY.append({"category": category, "name": template.format(*args),
                             "fn": (lambda env, args=args: fn(env, *args))})
        return fn
    return register


class Env:
    """The browser and the pages a test opens (closed after each test)."""

    def __init__(self, browser):
        self.browser = browser
        self.contexts = []

    def close_all(self):
        for context in self.contexts:
            try:
                context.close()
            except Exception:  # noqa: BLE001 - already gone
                pass
        self.contexts = []

    def page(self, width=640, height=360, reduced_motion=False):
        context = self.browser.new_context(viewport={"width": width, "height": height},
                                           reduced_motion="reduce" if reduced_motion else "no-preference")
        self.contexts.append(context)
        page = context.new_page()
        page.errors = []
        page.external = []
        page.on("console", lambda m: m.type == "error" and page.errors.append(m.text))
        page.on("pageerror", lambda e: page.errors.append(str(e)))
        # Nothing leaves the app: any other request is recorded and refused.
        context.route(re.compile(r"^(?!https://ytmp\.localhost/|https://www\.youtube\.com/).*"),
                      lambda route: (page.external.append(route.request.url), route.abort()))
        return page

    @staticmethod
    def serve(page, body, csp=CSP, host="https://ytmp.localhost/**"):
        # As home::page_csp: the page's own script is allowed by its hash.
        csp = csp.replace("{script}", script_hash(body), 1)
        page.route(host, lambda route: route.fulfill(
            status=200, content_type="text/html; charset=utf-8", headers={"Content-Security-Policy": csp}, body=body))

    def home(self, lang="en", width=640, height=360, last=None, welcome=False, drm=None, reduced_motion=False,
             sidebar=True):
        page = self.page(width, height, reduced_motion)
        self.serve(page, home_html(last_service=last, lang=lang, welcome=welcome))
        if drm == "ok":
            page.add_init_script("navigator.requestMediaKeySystemAccess = () => Promise.resolve({});")
        if sidebar:
            add_app_scripts(page, maintenance=False)
        else:
            # Only the start screen's own channel (what webview.rs gives it).
            page.add_init_script("window.__sent = []; window.__ytmpHomeSend = (m) => window.__sent.push(String(m));")
        page.goto("https://ytmp.localhost/")
        if sidebar:
            push_locale(page, lang, "home")
        page.wait_for_timeout(60)
        return page

    def settings(self, lang="en", width=760, height=560, state=None, section=None, push=True):
        page = self.page(width, height)
        page.add_init_script("window.__sent = []; Object.defineProperty(window, 'ipc', { value: Object.freeze({ "
                             "postMessage: (m) => window.__sent.push(String(m)) }) });")
        self.serve(page, fill("settings.html", {"i18n": i18n(lang, "settings")}, lang))
        page.goto("https://ytmp.localhost/settings")
        if push:
            push_settings(page, state or settings_state(), section)
            clear(page)
        return page

    def sidebar(self, lang="en", width=640, height=360, host=None):
        page = self.page(width, height)
        self.serve(page, SIDEBAR_PAGE, csp="default-src 'self' 'unsafe-inline'", host="https://www.youtube.com/**")
        add_app_scripts(page)
        page.goto("https://www.youtube.com/watch?v=test")
        push_locale(page, lang)
        page.evaluate(f"window.__ytmpSync({json.dumps(HOST_KEY)}, {json.dumps(host or SIDEBAR_HOST)})")
        page.cdp = page.context.new_cdp_session(page)
        return page


def push_settings(page, state, section=None):
    if section:
        state = dict(state, section=section)
    page.evaluate(f"window.__ytmpSettings({json.dumps(state)})")


def sent(page):
    return page.evaluate("(window.__sent || []).slice()")


def clear(page):
    page.evaluate("window.__sent && (window.__sent.length = 0)")


def home_sent(page):
    return [m for m in sent(page) if not m.startswith(("viewport:", "menu-open:")) and m != "ready"]


def ax_nodes(page):
    return page.cdp.send("Accessibility.getFullAXTree")["nodes"]


def ax_names(page):
    return [n.get("name", {}).get("value") or "" for n in ax_nodes(page)]


def ax_click(page, predicate):
    for node in ax_nodes(page):
        name = node.get("name", {}).get("value") or ""
        if predicate(name) and node.get("backendDOMNodeId"):
            page.cdp.send("DOM.scrollIntoViewIfNeeded", {"backendNodeId": node["backendDOMNodeId"]})
            quad = page.cdp.send("DOM.getBoxModel", {"backendNodeId": node["backendDOMNodeId"]})["model"]["border"]
            page.mouse.click((quad[0] + quad[2]) / 2, (quad[1] + quad[5]) / 2)
            return True
    return False


def ax_box(page, predicate):
    for node in ax_nodes(page):
        name = node.get("name", {}).get("value") or ""
        if predicate(name) and node.get("backendDOMNodeId"):
            quad = page.cdp.send("DOM.getBoxModel", {"backendNodeId": node["backendDOMNodeId"]})["model"]["border"]
            return {"x": quad[0], "y": quad[1], "width": quad[2] - quad[0], "height": quad[5] - quad[1]}
    return None


def bar_button_y(page, index):
    """Centre of the bar's n-th button (the shadow root is closed: by position)."""
    page.mouse.move(page.viewport_size["width"] - 10, page.viewport_size["height"] / 2)
    page.wait_for_timeout(250)
    ys = page.evaluate("""(x) => { const out = []; for (let y = 0; y < innerHeight; y += 2) {
        const el = document.elementFromPoint(x, y); if (el && el.tagName === 'YTMP-UI') out.push(y); } return out; }""",
                       page.viewport_size["width"] - 29)
    return ys


def settings_button(page, message):
    return page.locator(f"[data-send='{message}']")


def tab_to(page, selector, limit=80):
    """Presses Tab until `selector` has the focus; False if it never does."""
    for _ in range(limit):
        if page.evaluate(f"document.activeElement && document.activeElement.matches({json.dumps(selector)})"):
            return True
        page.keyboard.press("Tab")
    return False


# JavaScript checks used by several tests --------------------------------------

FIT_JS = """(scope) => {
  const issues = [];
  const doc = document.documentElement;
  if (doc.scrollWidth > innerWidth + 1) issues.push('scrolls sideways (' + doc.scrollWidth + ' > ' + innerWidth + ')');
  const root = scope ? document.querySelector(scope) : document.body;
  const box = root.getBoundingClientRect();
  for (const el of root.querySelectorAll('*')) {
    const cs = getComputedStyle(el);
    if (cs.display === 'none' || cs.visibility === 'hidden' || el.closest('[hidden]')) continue;
    if (el.closest('svg') || el.tagName === 'INPUT' || el.tagName === 'OPTION') continue;
    const r = el.getBoundingClientRect();
    if (!r.width || !r.height) continue;
    // Text cut off: more text than room, without an intended clamp.
    const clamped = cs.webkitLineClamp !== 'none' || cs.textOverflow === 'ellipsis';
    if (!clamped && (cs.overflowX === 'hidden' || cs.overflow === 'hidden') && el.scrollWidth > el.clientWidth + 1
        && el.textContent.trim()) issues.push('cut: ' + (el.className || el.tagName) + ' "' + el.textContent.trim().slice(0, 30) + '"');
    // Out of its own area sideways.
    if (r.right > Math.max(box.right, innerWidth) + 1 || r.left < Math.min(box.left, 0) - 1) {
      issues.push('outside: ' + (el.className || el.tagName) + ' "' + (el.textContent || '').trim().slice(0, 30) + '"');
    }
  }
  return [...new Set(issues)].slice(0, 6);
}"""

CLAMPED_JS = """(selector) => [...document.querySelectorAll(selector)]
  .filter((el) => el.offsetParent && el.scrollHeight > el.clientHeight + 2)
  .map((el) => el.textContent.trim())"""

UNNAMED_JS = """() => {
  const unnamed = [];
  for (const el of document.querySelectorAll('button, input, [role="switch"], [role="radio"], [role="tab"], a[href]')) {
    if (el.closest('[hidden]') || getComputedStyle(el).display === 'none') continue;
    const labelledby = (el.getAttribute('aria-labelledby') || '').split(/\\s+/).filter(Boolean)
      .map((id) => document.getElementById(id)?.textContent || '').join(' ');
    const name = (el.getAttribute('aria-label') || labelledby || el.textContent || el.title
      || (el.labels && el.labels[0] && el.labels[0].textContent) || el.placeholder || '').trim();
    if (!name) unnamed.push(el.outerHTML.slice(0, 80));
  }
  return unnamed;
}"""


def contrast(fg, bg):
    def lum(hex_color):
        hex_color = hex_color.lstrip("#")
        channels = [int(hex_color[i:i + 2], 16) / 255 for i in (0, 2, 4)]
        linear = [c / 12.92 if c <= 0.03928 else ((c + 0.055) / 1.055) ** 2.4 for c in channels]
        return 0.2126 * linear[0] + 0.7152 * linear[1] + 0.0722 * linear[2]
    a, b = lum(fg), lum(bg)
    return (max(a, b) + 0.05) / (min(a, b) + 0.05)


def blend(fg, alpha, bg):
    f = [int(fg.lstrip("#")[i:i + 2], 16) for i in (0, 2, 4)]
    b = [int(bg.lstrip("#")[i:i + 2], 16) for i in (0, 2, 4)]
    return "#" + "".join(f"{round(fc * alpha + bc * (1 - alpha)):02x}" for fc, bc in zip(f, b))


PALETTE = {
    "deep": "#03045e", "french": "#023e8a", "teal": "#0077b6", "blue_green": "#0096c7", "turquoise": "#00b4d8",
    "sky": "#48cae4", "frost": "#90e0ef", "frost2": "#ade8f4", "cyan": "#caf0f8", "alert": "#ff9f8a",
}


def kbd_caps(value):
    return re.findall(r"\[([^\]]+)\]", value)


def placeholders(value):
    if isinstance(value, dict):
        found = set()
        for v in value.values():
            found |= set(re.findall(r"\{(\w+)\}", v))
        return found
    return set(re.findall(r"\{(\w+)\}", value))


CATALOGS = {lang: catalog(lang) for lang in LANGS}

# =============================================================================
# UNIT
# =============================================================================

PLURAL_CASES = {
    "en": {1: "one", 2: "other", 5: "other", 21: "other"},
    "es": {1: "one", 2: "other", 5: "other", 21: "other"},
    "ru": {1: "one", 2: "few", 5: "many", 11: "many", 21: "one", 22: "few", 25: "many", 111: "many"},
    "pl": {1: "one", 2: "few", 5: "many", 12: "many", 22: "few", 25: "many", 112: "many"},
    "ja": {1: "other", 2: "other", 5: "other"},
}


@each("unit", "plural form for {1} in {0} (ui.ext.problems)",
      [(lang, n) for lang, cases in PLURAL_CASES.items() for n in cases])
def _(env, lang, n):
    expected_form = PLURAL_CASES[lang][n]
    value = CATALOGS[lang]["ui.ext.problems"]
    expected = value.get(expected_form, value.get("other")).replace("{count}", str(n))
    page = env.sidebar(lang=lang, host=dict(SIDEBAR_HOST, extensionProblems=["x"] * n))
    page.mouse.move(630, 180)
    page.wait_for_timeout(200)
    ys = bar_button_y(page, 0)
    page.mouse.click(611, ys[-1] - 50)  # "more" is the second to last button
    page.wait_for_timeout(150)
    names = ax_names(page)
    expect(any(expected in name for name in names), f"expected {expected!r} in the menu")


@each("unit", "t() fills {{placeholders}} in {0}", LANGS)
def _(env, lang):
    page = env.settings(lang=lang, section="extensions")
    shown = page.locator(".list li").first.inner_text()
    expected = text(lang, "settings.ext_version", version="5.9")
    expect(expected in shown, f"{expected!r} not in {shown!r}")
    expect("{" not in page.inner_text("body"), "a placeholder was left unfilled")


@each("unit", "welcome step '{1}' turns [keys] into key caps in {0}",
      [(lang, step) for lang in LANGS for step in ("pick", "move", "controls", "together")])
def _(env, lang, step):
    page = env.home(lang=lang, welcome=True, sidebar=False)
    order = ["pick", "move", "controls", "together"]
    for _ in range(order.index(step)):
        page.click("#welcome-next")
    caps = page.locator("#welcome-body kbd").all_inner_texts()
    expected = kbd_caps(CATALOGS[lang][f"home.welcome.{step}.body"])
    expect(caps == expected, f"caps {caps} != {expected}")
    expect("[" not in page.inner_text("#welcome-body"), "a bracket was left in the text")


@test("unit", "the settings state carries every field the page reads")
def _(env):
    fields = set(settings_view_fields())
    page_src = (SRC / "settings.html").read_text(encoding="utf-8")
    used = set(re.findall(r"\bstate\.(\w+)", page_src)) | set(re.findall(r"\bnext\.(version|section)\b", page_src))
    missing = sorted(used - fields)
    expect(not missing, f"the page reads fields App::push_settings never sends: {missing}")


@test("unit", "the test state matches App::push_settings field for field")
def _(env):
    expect(set(settings_state()) == set(settings_view_fields()),
           f"{set(settings_state()) ^ set(settings_view_fields())}")


@test("unit", "percent values follow each language's format")
def _(env):
    for lang in LANGS:
        page = env.settings(lang=lang, section="window", state=settings_state(opacity=55))
        shown = page.locator(".value").inner_text().replace(" ", " ").replace(" ", " ")
        expect("55" in shown and "%" in shown, f"{lang}: {shown!r}")


@test("unit", "settings section ids match ipc::SettingsSection")
def _(env):
    ids = re.findall(r'SettingsSection::\w+ => "(\w+)"', IPC_RS)
    expect(ids == SECTIONS, f"{ids} != {SECTIONS}")


@test("unit", "the start screen's data has a welcome flag the page reads")
def _(env):
    expect("welcome: bool" in HOME_RS and "data.welcome" in (SRC / "home.html").read_text(encoding="utf-8"),
           "home.rs and home.html disagree about the welcome")


@test("unit", "languages list: five, each with its own name")
def _(env):
    expect([l["code"] for l in LANGUAGES] == ["en", "es", "ru", "pl", "ja"], LANGS)
    expect(len({l["name"] for l in LANGUAGES}) == 5, "names repeat")


# =============================================================================
# CONTRACT (pages ↔ app)
# =============================================================================

SETTINGS_MESSAGES = sorted({
    "ready", "close", "pin", "ghost", "video-only", "taskbar-bar", "autostart", "start-with-last",
    "hardware-acceleration", "opacity", "size", "corner", "lang", "lock", "lock-now", "sign-out", "security",
    "ext-toggle", "ext-options", "ext-folder", "ext-reload", "together", "together-name", "together-mode", "welcome",
    "help", "open-folder", "restart",
})


@each("contract", "parse_settings accepts '{0}'", SETTINGS_MESSAGES)
def _(env, name):
    expect(name in SETTINGS_ARMS, f"ipc::parse_settings has no arm for {name!r}")


@test("contract", "every message the Settings page can send is a known one")
def _(env):
    page = env.settings(state=settings_state(lock="pin", hardwareAcceleration=False, overlay=True))
    for section in SECTIONS:
        push_settings(page, settings_state(lock="pin", hardwareAcceleration=False, overlay=True), section)
        for el in page.locator("main button:not([disabled])").all():
            if el.get_attribute("data-send") in ("sign-out",):  # opens a system dialog in the app
                pass
            el.click()
            page.wait_for_timeout(5)
    page.keyboard.press("Escape")
    heads = {m.split(":")[1] for m in sent(page)}
    unknown = sorted(heads - SETTINGS_ARMS)
    expect(not unknown, f"messages the app would drop: {unknown}")
    expect(all(m.startswith("settings:") for m in sent(page)), "a message without the settings: prefix")


@each("contract", "the start screen sends '{0}' and ipc::parse knows it", ["settings", "together", "welcome-done"])
def _(env, message):
    arms = set(re.findall(r'\("([a-z-]+)", (?:None|Some)', IPC_RS))
    expect(message in arms, f"ipc::parse has no arm for {message!r}")
    home_src = (SRC / "home.html").read_text(encoding="utf-8")
    expect(f"send('{message}')" in home_src, f"home.html never sends {message!r}")


@each("contract", "side bar message '{0}' is accepted by ipc::parse", ["settings", "settings:extensions"])
def _(env, message):
    head, _, arg = message.partition(":")
    expect(f'("{head}", {"Some" if arg else "None"}' in IPC_RS, f"no arm for {message}")
    if arg:
        expect(f'"{arg}"' in IPC_RS, f"no section {arg}")


@test("contract", "the settings page is served at /settings with its strings")
def _(env):
    expect('"/settings" => (fill(SETTINGS' in HOME_RS, "home::respond does not serve /settings")
    expect(PREFIXES.get("settings") == ["settings.", "lang.", "common."], f"prefixes {PREFIXES.get('settings')}")


@test("contract", "the settings page only uses strings it is given")
def _(env):
    src = (SRC / "settings.html").read_text(encoding="utf-8")
    keys = set(re.findall(r"\bt\('([a-z0-9_.]+)'", src)) | set(re.findall(r'data-i18n(?:-[a-z-]+)?="([a-z0-9_.]+)"', src))
    outside = sorted(k for k in keys if not any(k.startswith(p) for p in PREFIXES["settings"]))
    expect(not outside, f"keys the page never gets: {outside}")


@test("contract", "the start screen only uses strings it is given")
def _(env):
    src = (SRC / "home.html").read_text(encoding="utf-8")
    keys = set(re.findall(r"\bt\('([a-z0-9_.]+)'", src)) | set(re.findall(r'data-i18n(?:-[a-z-]+)?="([a-z0-9_.]+)"', src))
    outside = sorted(k for k in keys if not any(k.startswith(p) for p in PREFIXES["home"]))
    expect(not outside, f"keys the page never gets: {outside}")


@test("contract", "ui.js only uses strings it is given")
def _(env):
    src = (SRC / "ui.js").read_text(encoding="utf-8")
    keys = set(re.findall(r"\bt\('([a-z0-9_.]+)'", src)) | set(re.findall(r"label: '([a-z0-9_.]+)'", src))
    outside = sorted(k for k in keys if not any(k.startswith(p) for p in PREFIXES["ui"]))
    expect(not outside, f"keys the side bar never gets: {outside}")


@test("contract", "the tray opens Settings")
def _(env):
    tray = (SRC / "tray.rs").read_text(encoding="utf-8")
    expect('"settings" => Command::SettingsOpen(None)' in tray and "open_settings" in tray, "no Settings item in the tray")


@test("contract", "Settings hides while the app is locked")
def _(env):
    body = re.search(r"fn lock_now\(&mut self\) \{(.*?)\n    \}", APP_RS, re.S).group(1)
    expect("self.settings_panel.hide()" in body, "lock_now leaves Settings showing")
    allowed = re.search(r"fn allowed_while_locked.*?\n    \}", APP_RS, re.S).group(0)
    for command in ("SettingsOpen", "Set(", "SettingsLock", "TogetherName", "Restart"):
        expect(command not in allowed, f"{command} works while locked")


@test("contract", "closing the Settings window only hides it")
def _(env):
    main = (SRC / "main.rs").read_text(encoding="utf-8")
    expect("app.settings_window_id()" in main and "Command::SettingsClose" in main, "no close handling")


@test("contract", "a language change reloads Settings in the new language")
def _(env):
    body = re.search(r"fn set_language\(&mut self, lang: Lang\) \{(.*?)\n    \}", APP_RS, re.S).group(1)
    expect("self.settings_panel.relocalize()" in body, "Settings keeps the old language")


@test("contract", "the Settings window is pinned to its page")
def _(env):
    panel = (SRC / "settings_panel.rs").read_text(encoding="utf-8")
    for needle in ("with_navigation_handler(|url| url == URL)", "NewWindowResponse::Deny",
                   "with_download_started_handler(|_, _| false)", "PermissionResponse::Deny",
                   "ipc::parse_settings", "crate::webview::harden"):
        expect(needle in panel, f"settings_panel.rs lacks {needle}")


@test("contract", "a section asked for before the page loads is not lost")
def _(env):
    panel = (SRC / "settings_panel.rs").read_text(encoding="utf-8")
    expect("self.webview.is_some() && self.ready.get()" in panel, "the state (and its section) is sent before the page listens")
    body = re.search(r"Command::SettingsReady => \{(.*?)\n            \}", APP_RS, re.S).group(1)
    expect("set_ready()" in body and "push_settings(true)" in body, "the page's ready is not used")


WEBVIEW_RS = (SRC / "webview.rs").read_text(encoding="utf-8")
DIAG_RS = (SRC / "diag.rs").read_text(encoding="utf-8")


@each("contract", "WebView2 starts with {0}", ["--autoplay-policy=no-user-gesture-required", "--disable-renderer-backgrounding"])
def _(env, flag):
    args = re.search(r'const BROWSER_ARGS: &str =\s*"(.*?)";', WEBVIEW_RS, re.S).group(1)
    expect(flag in args, f"missing: {args}")
    expect(args.count("--disable-features") == 1, "two feature lists (the second would replace the first)")


def page_stat_fields(file, function):
    src = (SRC / file).read_text(encoding="utf-8")
    body = re.search(r"async function %s\(.*?\n  \}\n" % function, src, re.S).group(0)
    return set(re.findall(r"\bout\.(\w+) =", body)) | set(re.findall(r"\b(\w+): [^,{}]+(?:,|$)", body.split("const page = {")[-1].split("};")[0]) if "const page = {" in body else set())


DIARY_FIELDS = set(re.findall(r'(?:number|word)\((?:&?\w+), "(\w+)"\)', DIAG_RS)) | {"muted", "paused", "id", "peers"}


@each("contract", "every number {0} reports reaches the diary", [("cast.html", "peerStats"), ("stream.html", "stats")])
def _(env, file, function):
    sent_fields = page_stat_fields(file, function)
    expect(sent_fields, "no fields found")
    lost = sorted(sent_fields - DIARY_FIELDS)
    expect(not lost, f"the diary drops: {lost}")


@test("contract", "the sending page's own numbers reach the diary")
def _(env):
    src = (SRC / "cast.html").read_text(encoding="utf-8")
    page = re.search(r"const page = \{(.*?)\};", src, re.S).group(1)
    fields = set(re.findall(r"(\w+):", page)) - {"peers"}
    expect(fields and fields <= DIARY_FIELDS, f"{sorted(fields - DIARY_FIELDS)}")


@each("contract", "{0} statistics have their own message", [("cast", "CastStats"), ("stream", "StreamStats")])
def _(env, prefix, command):
    expect(f'("stats", Some(json)) if json.len() <= MAX_STATS' in IPC_RS or f'Some(("stats", json)) if json.len() <= MAX_STATS' in IPC_RS,
           "no stats arm")
    expect(f"Command::{command}(json)" in APP_RS, f"{command} is not written to the diary")


@test("contract", "the video starts at a good bitrate (Chrome's codec parameters on the answer)")
def _(env):
    src = (SRC / "cast.html").read_text(encoding="utf-8")
    expect("x-google-start-bitrate" in src and "videoBitrates(stereo(" in src, "no start bitrate")


@test("contract", "the welcome is shown once and remembered")
def _(env):
    expect("welcome: !settings.welcome_seen" in (SRC / "main.rs").read_text(encoding="utf-8"), "main.rs")
    body = re.search(r"Command::WelcomeDone => \{(.*?)\n            \}", APP_RS, re.S).group(1)
    expect("welcome_seen = true" in body and "save_settings" in body, "WelcomeDone does not save")


# =============================================================================
# Contract: risk mitigations (evaluation plan R1, R2, R4, R6)
# =============================================================================

ROOT = SRC.parent
TOGETHER_HTML = (SRC / "together.html").read_text(encoding="utf-8")
TOGETHER_RS = (SRC / "together" / "mod.rs").read_text(encoding="utf-8")
CAST_RS = (SRC / "cast.rs").read_text(encoding="utf-8")
PACKAGE_PS1 = (ROOT / "tools" / "package.ps1").read_text(encoding="utf-8-sig")
SIGN_PS1 = (ROOT / "tools" / "sign.ps1").read_text(encoding="utf-8-sig")
CHECK_EXE_PS1 = (ROOT / "tools" / "check-release-exe.ps1").read_text(encoding="utf-8-sig")
INSTALL_PS1 = (ROOT / "dist" / "install.ps1").read_text(encoding="utf-8-sig")
RELEASE_YML = (ROOT / ".github" / "workflows" / "release.yml").read_text(encoding="utf-8")
CI_YML = (ROOT / ".github" / "workflows" / "ci.yml").read_text(encoding="utf-8")
E2E_RS = (SRC / "e2e.rs").read_text(encoding="utf-8")
LAUNCHERS = ["Install.cmd", "Install-overlay.cmd", "Uninstall.cmd"]


def camel(name):
    head, *rest = name.split("_")
    return head + "".join(w.title() for w in rest)


def fn_body(src, signature):
    start = src.index(signature)
    depth = 0
    for i in range(src.index("{", start), len(src)):
        depth += {"{": 1, "}": -1}.get(src[i], 0)
        if depth == 0:
            return src[start:i]
    raise Fail(f"no end for {signature}")


@each("contract", "R1: the panel's '{0}' button sends a message parse_together knows",
      [("sound-pc", "pc"), ("sound-app", "app")])
def _(env, button, arg):
    expect(f"$('{button}').addEventListener('click', () => send('together:sound:{arg}'))" in TOGETHER_HTML, "not wired")
    expect(f'("sound", Some("{arg}")) => Command::TogetherSound(' in IPC_RS, "parse_together does not know it")


@test("contract", "R1: every sound field the panel reads is in Together's view")
def _(env):
    view = re.search(r"struct View<'a> \{(.*?)\n\}", TOGETHER_RS, re.S).group(1)
    fields = {camel(f) for f in re.findall(r"^\s+(sound\w*):", view, re.M)}
    read = set(re.findall(r"view\.(sound\w*)", TOGETHER_HTML))
    expect(read and read <= fields, f"read but not sent: {sorted(read - fields)}")
    expect({"sound", "soundWholePc", "soundProblem", "soundAppUnavailable"} <= fields, sorted(fields))


@test("contract", "R1: the panel knows every sound problem the app can report")
def _(env):
    keys = set(re.findall(r'Self::\w+ => "(\w+)"', re.search(r"impl SoundProblem \{(.*?)\n\}", TOGETHER_RS, re.S).group(1)))
    expect(keys == {"page", "unavailable", "silent"}, sorted(keys))
    for key in keys:
        expect(f"'{key}'" in TOGETHER_HTML, f"together.html ignores '{key}'")


@test("contract", "R1: the PC's sound lasts one sharing session (back to the app's when it stops)")
def _(env):
    body = fn_body(CAST_RS, "pub fn stop(&mut self)")
    expect("self.source = Source::App" in body and "self.app_unavailable = false" in body, "stop() keeps the PC's sound")


@test("contract", "R1: the silence check runs on the app's timer and asks WebView2 if the page plays")
def _(env):
    expect("self.cast.sound_deadline()" in fn_body(APP_RS, "pub fn deadline(&self)"), "not in App::deadline")
    body = fn_body(APP_RS, "pub fn on_timer(&mut self)")
    expect("check_sound(now, || devtools::playing_audio(webview))" in body, "not in App::on_timer")


@test("contract", "R1: a replaced capture's late status is ignored")
def _(env):
    body = fn_body(CAST_RS, "pub fn set_capture_sound(&mut self, id: u32, ok: bool)")
    expect("id != self.capture_id" in body, "statuses are not matched to their capture")


@test("security", "R2: package.ps1 signs before writing the fingerprints")
def _(env):
    signing = PACKAGE_PS1.index("sign.ps1")
    expect(signing < PACKAGE_PS1.index("SHA256SUMS.txt"), "the fingerprints would be of unsigned files")
    expect("yt-mini-player-uia.exe" in PACKAGE_PS1[signing:signing + 300], "the overlay program is not signed")


@test("security", "R2: a signature that does not verify, or has no timestamp, stops the build")
def _(env):
    for needle in ("verify /pa", "Get-AuthenticodeSignature", "TimeStamperCertificate", "$required) { throw"):
        expect(needle in SIGN_PS1, f"sign.ps1 lacks: {needle}")


@test("security", "R2: once signing is set up in the release, a failure fails the release")
def _(env):
    expect('"YTMP_SIGN_REQUIRED=1" >> $env:GITHUB_ENV' in RELEASE_YML, "release.yml would package unsigned")
    expect("vars.AZURE_SIGNING_ENDPOINT != ''" in RELEASE_YML and "azure/login@" in RELEASE_YML, "no optional Azure step")


@each("security", "R2: {0} explains a quarantined install.ps1", LAUNCHERS)
def _(env, name):
    raw = (ROOT / "dist" / name).read_bytes()
    expect(raw.isascii(), "not ASCII (cmd would garble it)")
    src = raw.decode("ascii")
    expect('if not exist "%~dp0install.ps1" goto missing' in src and "ANTIVIRUS.txt" in src, "no explanation")
    expect("\r\n" in src, "not CRLF")


@test("security", "R2: ANTIVIRUS.txt is packaged and has a section in each language")
def _(env):
    expect("'ANTIVIRUS.txt'" in PACKAGE_PS1, "not packaged")
    text = (ROOT / "dist" / "ANTIVIRUS.txt").read_text(encoding="utf-8-sig")
    sections = re.findall(r"^\[([a-z]{2})\]\r?$", text, re.M)
    expect(sections == LANGS, f"{sections} vs {LANGS}")
    for part in re.split(r"^\[[a-z]{2}\]\r?$", text, flags=re.M)[1:]:
        expect("SHA-256" in part and "Install.cmd" in part and "filesubmission" in part, "a section misses a step")


@test("security", "R2: the overlay install skips the local certificate for a publisher-signed program")
def _(env):
    body = fn_body(INSTALL_PS1, "function Install-Machine")
    expect(body.index("Test-Checksum $MachineExe") < body.index("Get-Publisher $MachineExe") < body.index("Install-LocalSignature"),
           "the fingerprint is not checked first, or the signature is not looked at")


@test("i18n", "R2: the overlay intro's certificate lines are 6 to 8 in every language")
def _(env):
    intros = re.findall(r"OverlayIntro\s+= @\((.*?)\)\r?\n", INSTALL_PS1, re.S)
    expect(len(intros) == 5, f"{len(intros)} intros")
    for intro in intros:
        lines = re.findall(r"'((?:[^']|'')*)'|\"([^\"]*)\"", intro)
        lines = [a or b for a, b in lines]
        expect(len(lines) == 10 and "YT Mini Player (local signing)" in lines[5], lines[:6])
    expect(INSTALL_PS1.count("OverlaySignedLine = ") == 5 and INSTALL_PS1.count("AlreadySigned    = ") == 5, "missing texts")


@test("contract", "R4: ui.js sends page-check and ipc::parse reads it strictly")
def _(env):
    expect("send('page-check:' + json)" in (SRC / "ui.js").read_text(encoding="utf-8"), "ui.js does not send it")
    expect('("page-check", Some(json)) => Command::PageCheck(Box::new(crate::diag::PageCheck::from_report(json)?))' in IPC_RS,
           "not parsed into the closed type")
    body = re.search(r"Command::PageCheck\(check\) => \{(.*?)\n            \}", APP_RS, re.S).group(1)
    expect("page_warnings.insert" in body and "diag::note(check.line())" in body, "not noted, or warned more than once")


@test("contract", "R4: video only and full screen use the player finder, not one fixed name")
def _(env):
    ui = (SRC / "ui.js").read_text(encoding="utf-8")
    expect("const PLAYER = `[${PLAYER_ATTR}]`;" in ui and "markPlayer(player)" in ui, "video only is tied to #movie_player")
    expect("findPlayer().el" in fn_body(ui, "function toggleFullscreen()"), "full screen is tied to #movie_player")


@test("contract", "R4: the weekly live smoke test checks its own detection first and files one issue")
def _(env):
    smoke = (ROOT / ".github" / "workflows" / "live-smoke.yml").read_text(encoding="utf-8")
    expect("cron:" in smoke and "--offline --broken" in smoke and "gh issue" in smoke, "incomplete")
    expect("python tests/live_smoke.py --offline" in CI_YML, "CI does not keep the script working")


@test("security", "R6: the test build's marker is the same in e2e.rs and the release check")
def _(env):
    marker = re.search(r'pub const MARKER: &str = "([^"]+)";', E2E_RS).group(1)
    expect(f"GetBytes('{marker}')" in CHECK_EXE_PS1, "check-release-exe.ps1 looks for something else")
    expect(re.search(r'#\[cfg\(feature = "e2e"\)\]\s*pub const MARKER', E2E_RS), "the marker is in every build")


@test("security", "R6: without the feature, the test build's switches are inert")
def _(env):
    for fn in ("pub fn port()", "pub fn data_dir()"):
        expect('if cfg!(feature = "e2e")' in fn_body(E2E_RS, fn), f"{fn} is not guarded")


@test("security", "R6: package.ps1 and release.yml refuse the test build")
def _(env):
    expect(PACKAGE_PS1.index("check-release-exe.ps1") < PACKAGE_PS1.index("Copy-Item 'target\\release\\yt-mini-player.exe'"),
           "package.ps1 copies before checking")
    expect("./tools/check-release-exe.ps1 -Files (Get-ChildItem dist/YT-Mini-Player-*-win64/*.exe)" in RELEASE_YML,
           "release.yml does not check what it publishes")


@test("contract", "R6: CI runs the suite and the real app on Windows, and proves the refusal")
def _(env):
    for needle in ("python tests/suite.py", "cargo build --release --locked --features e2e",
                   "check-release-exe.ps1 did not catch the test build", "python tests/windows_e2e.py"):
        expect(needle in CI_YML, f"ci.yml lacks: {needle}")


# =============================================================================
# E2E: full screen in the window (any site's player fills the mini window)
# =============================================================================

FRAME_SITE = """<!doctype html><html><head><meta charset="utf-8"><title>Episode 1 - Crunchyroll</title><style>
  body { margin: 0; background: #000; color: #fff; font-family: sans-serif; }
  header { height: 60px; background: #222; }
  .wrap { display: flex; gap: 16px; padding: 16px; }
  .player { flex: 1; transform: translateZ(0); }
  iframe { width: 100%; aspect-ratio: 16 / 9; }
  aside { width: 160px; } .ep { height: 50px; margin-bottom: 8px; background: #a33; }
</style></head><body><header id="top">Crunchyroll</header>
<div class="wrap"><div class="player"><iframe id="vilos" src="https://static.crunchyroll.com/vilos/player.html" allowfullscreen></iframe></div>
<aside id="episodes"><div class="ep"></div><div class="ep"></div><div class="ep"></div></aside></div>
<h1 id="title">Episode title</h1></body></html>"""

FRAME_PLAYER = """<!doctype html><html><head><meta charset="utf-8"></head>
<body style="margin:0;background:#123;height:100vh"><video style="width:100%;height:100%"></video>
<button id="fs" style="position:absolute;right:6px;bottom:6px;width:40px;height:30px">FS</button>
<script>document.getElementById('fs').onclick = () => document.fullscreenElement
  ? document.exitFullscreen() : document.documentElement.requestFullscreen();</script></body></html>"""

INLINE_SITE = """<!doctype html><html><head><meta charset="utf-8"><title>Channel - Twitch</title><style>
  body { margin: 0; background: #0e0e10; color: #fff; font-family: sans-serif; }
  nav { height: 50px; background: #18181b; } .layout { display: flex; }
  main { flex: 1; } aside { width: 200px; background: #1f1f23; height: 300px; }
  #vp { position: relative; aspect-ratio: 16 / 9; background: #333; }
  #vp video { width: 100%; height: 100%; display: block; }
  .controls { position: absolute; left: 0; right: 0; bottom: 0; height: 30px; }
</style></head><body><nav id="nav">Twitch</nav><div class="layout"><main><div id="vp"><video></video>
<div class="controls"><button id="play">Play</button></div></div></main><aside id="chat">Chat</aside></div></body></html>"""

NO_VIDEO_SITE = """<!doctype html><html><head><meta charset="utf-8"></head><body><h1>News</h1><p>No player here.</p></body></html>"""


def service_page(env, body, url, frame=None, fullscreen_in_window=False, lang="en"):
    page = env.page(640, 360)
    origin = re.match(r"https://[^/]+/", url).group(0)
    env.serve(page, body, csp="default-src 'self' https: 'unsafe-inline'", host=origin + "**")
    if frame:
        page.route("https://static.crunchyroll.com/**", lambda route: route.fulfill(
            status=200, content_type="text/html; charset=utf-8", body=frame))
    add_app_scripts(page)
    page.goto(url)
    push_locale(page, lang)
    page.evaluate(f"window.__ytmpSync({json.dumps(HOST_KEY)}, "
                  f"{json.dumps(dict(SIDEBAR_HOST, fullscreenInWindow=fullscreen_in_window))})")
    page.wait_for_timeout(100)
    page.cdp = page.context.new_cdp_session(page)
    clear(page)
    return page


def crunchyroll(env, **kw):
    return service_page(env, FRAME_SITE, "https://www.crunchyroll.com/watch/G1/episode-1", FRAME_PLAYER, **kw)


def twitch(env, **kw):
    return service_page(env, INLINE_SITE, "https://www.twitch.tv/channel", **kw)


def rect(page, selector):
    return page.evaluate(f"(() => {{ const r = document.querySelector({json.dumps(selector)}).getBoundingClientRect(); "
                         f"return [Math.round(r.x), Math.round(r.y), Math.round(r.width), Math.round(r.height)]; }})()")


def visible_at(page, x, y, selector):
    return page.evaluate(f"!!document.elementFromPoint({x}, {y})?.closest({json.dumps(selector)})")


def filled(page):
    return page.evaluate("document.documentElement.classList.contains('ytmp-video-only')")


def press_fill(page):
    """The side bar's button, found by its accessible name."""
    page.mouse.move(630, 180)
    page.wait_for_timeout(250)
    expect(ax_click(page, lambda name: name.startswith("Full screen in the window")), "no such button")
    page.wait_for_timeout(200)


@test("e2e", "full screen in the window: the side bar's button fills the window with a player in a frame")
def _(env):
    page = crunchyroll(env)
    press_fill(page)
    expect(filled(page), "not filled")
    expect(rect(page, "#vilos") == [0, 0, 640, 360], rect(page, "#vilos"))
    for x, y, what in [(300, 20, "header"), (540, 100, "episodes"), (40, 340, "title")]:
        expect(visible_at(page, x, y, "#vilos"), f"the {what} still shows")
    expect(page.evaluate("document.fullscreenElement") is None, "a real full screen was used")
    expect(not any(m.startswith("fullscreen:") for m in sent(page)), sent(page))


@test("e2e", "full screen in the window: Ctrl+M switches it on and off, Escape leaves it")
def _(env):
    page = crunchyroll(env)
    page.keyboard.press("Control+m")
    page.wait_for_timeout(150)
    expect(filled(page), "Ctrl+M did not fill")
    page.keyboard.press("Control+m")
    page.wait_for_timeout(150)
    expect(not filled(page), "Ctrl+M did not leave")
    page.keyboard.press("Control+m")
    page.wait_for_timeout(150)
    page.keyboard.press("Escape")
    page.wait_for_timeout(150)
    expect(not filled(page), "Escape did not leave")
    expect(rect(page, "#vilos")[1] > 50, "the page is not back to normal")


@test("e2e", "full screen in the window: a player on the page takes its controls and leaves the chat out")
def _(env):
    page = twitch(env)
    page.keyboard.press("Control+m")
    page.wait_for_timeout(150)
    marked = page.evaluate("[...document.querySelectorAll('[data-ytmp-player]')].map((e) => e.id || e.tagName)")
    expect(marked in (["vp"], ["MAIN"]), marked)
    expect(visible_at(page, 320, 20, "#vp") and visible_at(page, 600, 180, "#vp"), "nav or chat still shows")
    expect(visible_at(page, 20, 345, "#play"), "the player's controls are hidden")


@test("e2e", "full screen in the window: a page without a video says so")
def _(env):
    page = service_page(env, NO_VIDEO_SITE, "https://news.example.com/article")
    page.keyboard.press("Control+m")
    page.wait_for_timeout(200)
    expect(not filled(page), "filled with nothing")
    expect(text("en", "ui.fill_nothing") in " ".join(ax_names(page)), "no notice")


@test("e2e", "full screen in the window: the window's edges still resize it over a framed player")
def _(env):
    page = crunchyroll(env)
    page.keyboard.press("Control+m")
    page.wait_for_timeout(150)
    clear(page)
    page.mouse.move(637, 200)
    page.mouse.down()
    page.mouse.up()
    expect("resize:e" in sent(page), sent(page))


@test("e2e", "full screen in the window: it ends by itself when the player goes away")
def _(env):
    page = crunchyroll(env)
    page.keyboard.press("Control+m")
    page.wait_for_timeout(150)
    page.evaluate("document.getElementById('vilos').remove()")
    page.wait_for_timeout(1300)
    expect(not filled(page), "the page stayed blank")


@test("e2e", "the video's own full-screen button fills the window when Settings say so")
def _(env):
    page = crunchyroll(env, fullscreen_in_window=True)
    page.frame_locator("#vilos").locator("#fs").click()
    page.wait_for_timeout(400)
    expect(page.evaluate("document.fullscreenElement") is None, "it stayed in real full screen")
    expect(filled(page) and rect(page, "#vilos") == [0, 0, 640, 360], "the player does not fill the window")
    expect("fullscreen:1" not in sent(page), "the app was told to fill the screen")
    page.frame_locator("#vilos").locator("#fs").click()
    page.wait_for_timeout(400)
    expect(not filled(page), "pressed again, it did not leave")


@test("e2e", "without that setting, the video's own button still fills the screen")
def _(env):
    page = crunchyroll(env)
    page.frame_locator("#vilos").locator("#fs").click()
    page.wait_for_timeout(400)
    expect(page.evaluate("document.fullscreenElement?.id") == "vilos", "not a real full screen")
    expect("fullscreen:1" in sent(page), sent(page))


@test("e2e", "F11 fills the screen even when the video's button fills the window")
def _(env):
    page = crunchyroll(env, fullscreen_in_window=True)
    page.keyboard.press("F11")
    page.wait_for_timeout(400)
    expect(page.evaluate("!!document.fullscreenElement"), "F11 was turned into the window")
    expect("fullscreen:1" in sent(page), sent(page))


@test("e2e", "full screen in the window + F11: the player fills the screen with the side bar on top")
def _(env):
    page = crunchyroll(env)
    page.keyboard.press("Control+m")
    page.wait_for_timeout(150)
    page.keyboard.press("F11")
    page.wait_for_timeout(400)
    expect(page.evaluate("document.fullscreenElement === document.documentElement"), "not the whole page")
    expect(filled(page), "the fill was dropped in full screen")


@test("e2e", "on YouTube the video's own full-screen button switches video only instead, when Settings say so")
def _(env):
    page = env.sidebar(host=dict(SIDEBAR_HOST, fullscreenInWindow=True))
    clear(page)
    page.evaluate("document.getElementById('movie_player').requestFullscreen()")
    page.wait_for_timeout(400)
    expect(page.evaluate("document.fullscreenElement") is None, "it stayed in real full screen")
    expect(sent(page).count("video-only") == 1 and "fullscreen:1" not in sent(page), sent(page))


@each("layout", "full screen in the window: its name and the setting fit in {0}", LANGS)
def _(env, lang):
    page = crunchyroll(env, lang=lang)
    names = ax_names(page)
    expect(any(n.startswith(text(lang, "ui.fill_window")) for n in names), f"no button named {text(lang, 'ui.fill_window')}")
    settings = env.settings(lang=lang, section="window")
    row = settings.locator('[data-row="fullscreen-in-window"]')
    expect(row.count() == 1, "no row in Settings")
    box = row.bounding_box()
    expect(box and box["width"] <= 760, box)
    expect(settings.evaluate("document.documentElement.scrollWidth <= document.documentElement.clientWidth"), "scrolls sideways")


@test("contract", "the Settings switch for the video's own button is a message parse_settings knows")
def _(env):
    page = env.settings(section="window")
    page.click('[data-setting="fullscreen-in-window"]')
    expect(sent(page) == ["settings:fullscreen-in-window:1"], sent(page))
    expect('("fullscreen-in-window", _) => Command::Set(Setting::FullscreenInWindow(on()?))' in IPC_RS, "not parsed")
    expect("fullscreen_in_window: self.settings.fullscreen_in_window" in APP_RS, "not sent to the pages")


@test("contract", "a game in exclusive full screen gets advice afterwards, without taking the focus")
def _(env):
    topmost = (SRC / "topmost.rs").read_text(encoding="utf-8")
    expect("SHQueryUserNotificationState" in topmost and "display_mode_changed" in topmost, "no detection")
    expect("fn leave_front" in topmost and "on_hint(hint)" in topmost, "the advice is not given when the game leaves")
    body = re.search(r"Command::GameHint\(hint\) => \{(.*?)\n            \}", APP_RS, re.S).group(1)
    expect("self.show()" not in body and "set_focus" not in body, "it would steal the focus")
    for key in ("app.game_exclusive", "app.game_exclusive_minecraft", "app.a_game"):
        expect(f't!("{key}"' in body, f"{key} not used")


@test("e2e", "long notices stay on screen longer than short ones")
def _(env):
    page = env.sidebar()
    long_text = "Minecraft used exclusive full screen. " * 5
    page.evaluate(f"window.__ytmpToast({json.dumps(HOST_KEY)}, 'Short')")
    page.evaluate(f"window.__ytmpToast({json.dumps(HOST_KEY)}, {json.dumps(long_text)})")
    page.wait_for_timeout(4200)
    names = " ".join(ax_names(page))
    expect("Short" not in names, "the short one stayed")
    expect("exclusive full screen" in names, "the long one went as fast")


# =============================================================================
# E2E: the start screen
# =============================================================================

SERVICE_IDS = ["youtube", "youtube-music", "netflix", "crunchyroll", "prime-video", "disney-plus", "hbo-max", "twitch",
               "music"]


@each("e2e", "start screen: clicking key {0} opens {1}", list(enumerate(SERVICE_IDS, 1)))
def _(env, number, service):
    page = env.home(sidebar=False)
    page.click(f".key[data-id='{service}']")
    expect(sent(page) == [f"service:{service}"], sent(page))
    expect(page.get_attribute(f".key[data-id='{service}']", "aria-busy") == "true", "the key does not say it is opening")


@each("e2e", "start screen: pressing {0} opens {1}", list(enumerate(SERVICE_IDS, 1)))
def _(env, number, service):
    page = env.home(sidebar=False)
    page.keyboard.press(str(number))
    expect(sent(page) == [f"service:{service}"], sent(page))


@test("e2e", "start screen: the number pad works too")
def _(env):
    page = env.home(sidebar=False)
    page.keyboard.press("Numpad5")
    expect(sent(page) == ["service:prime-video"], sent(page))


@test("e2e", "start screen: numbers do nothing with Ctrl held")
def _(env):
    page = env.home(sidebar=False)
    page.keyboard.press("Control+1")
    expect(sent(page) == [], sent(page))


@test("e2e", "start screen: numbers typed in the address do not open services")
def _(env):
    page = env.home(sidebar=False)
    page.click("#site-url")
    page.keyboard.type("123")
    expect(sent(page) == [], sent(page))


@test("e2e", "start screen: Enter opens the typed address")
def _(env):
    page = env.home(sidebar=False)
    page.fill("#site-url", "  plex.tv/web  ")
    page.keyboard.press("Enter")
    expect(sent(page) == ["open-url:plex.tv/web"], sent(page))


@test("e2e", "start screen: an empty address sends nothing")
def _(env):
    page = env.home(sidebar=False)
    page.click(".open")
    expect(sent(page) == [], sent(page))


@test("e2e", "start screen: the gear opens Settings")
def _(env):
    page = env.home(sidebar=False)
    page.click("#settings")
    expect(sent(page) == ["settings"], sent(page))


@test("e2e", "start screen: Watch together opens its window")
def _(env):
    page = env.home(sidebar=False)
    page.click("#together")
    expect(sent(page) == ["together"], sent(page))


@test("e2e", "start screen: the header moves the window")
def _(env):
    page = env.home()
    clear(page)
    box = page.locator("h1").bounding_box()
    page.mouse.move(box["x"] + 20, box["y"] + 8)
    page.mouse.down()
    page.mouse.up()
    expect("drag" in home_sent(page), home_sent(page))


@test("e2e", "start screen: the buttons in the header do not move the window")
def _(env):
    page = env.home()
    clear(page)
    page.click("#settings")
    expect(home_sent(page) == ["settings"], home_sent(page))


@test("e2e", "start screen: the last service is marked")
def _(env):
    page = env.home(last="twitch", sidebar=False)
    expect(page.locator(".key[data-id='twitch'] .last").count() == 1, "no mark")
    expect(page.locator(".last").count() == 1, "more than one mark")


@test("e2e", "start screen: a new last service from the app moves the mark")
def _(env):
    page = env.home(last="twitch")
    page.evaluate(f"window.__ytmpSync({json.dumps(HOST_KEY)}, {{ lastService: 'netflix' }})")
    page.wait_for_timeout(50)
    expect(page.locator(".key[data-id='netflix'] .last").count() == 1 and page.locator(".last").count() == 1, "mark")


@test("e2e", "start screen: services that need DRM say so when it is missing")
def _(env):
    page = env.home(sidebar=False)
    page.wait_for_timeout(250)
    expect(page.get_attribute("#drm", "data-state") == "missing", page.get_attribute("#drm", "data-state"))
    for service in ("netflix", "crunchyroll", "prime-video", "disney-plus", "hbo-max"):
        expect(page.locator(f".key[data-id='{service}'] .needs").count() == 1, f"{service} has no dot")
    for service in ("youtube", "youtube-music", "twitch", "music"):
        expect(page.locator(f".key[data-id='{service}'] .needs").count() == 0, f"{service} has a dot")
    expect(page.is_visible("#recheck"), "no way to check again")


@test("e2e", "start screen: with DRM, no warnings and the systems are named")
def _(env):
    page = env.home(drm="ok", sidebar=False)
    page.wait_for_timeout(150)
    expect(page.get_attribute("#drm", "data-state") == "ok", "state")
    expect(page.locator(".needs, .caption.warn").count() == 0, "warnings left")
    expect("Widevine and PlayReady" in page.inner_text("#drm .text"), page.inner_text("#drm .text"))
    expect(page.is_hidden("#recheck"), "recheck shown")


@test("e2e", "start screen: the language menu opens, moves with arrows and sends the choice")
def _(env):
    page = env.home(sidebar=False)
    page.click("#lang")
    page.keyboard.press("ArrowDown")
    page.keyboard.press("Enter")
    expect(sent(page) == ["lang:es"], sent(page))


@test("e2e", "start screen: numbers do nothing while the language menu is open")
def _(env):
    page = env.home(sidebar=False)
    page.click("#lang")
    page.keyboard.press("2")
    expect(sent(page) == [], sent(page))


@test("e2e", "start screen: a language change redraws without reloading")
def _(env):
    page = env.home()
    page.fill("#site-url", "plex.tv")
    push_locale(page, "pl", "home")
    page.wait_for_timeout(50)
    expect(page.inner_text("h1") == text("pl", "home.title"), page.inner_text("h1"))
    expect(page.input_value("#site-url") == "plex.tv", "typed text lost")
    expect(page.get_attribute("#settings", "aria-label") == text("pl", "home.settings"), "gear name")


@test("e2e", "start screen: the right-edge tab says where the controls are")
def _(env):
    page = env.home()
    expect(page.get_attribute(".edge", "title") == text("en", "home.edge_hint"), "no hint")
    box = page.locator(".edge").bounding_box()
    expect(box["x"] + box["width"] >= 639 and box["x"] > 600, f"not on the right edge ({box})")


@test("e2e", "start screen: pointing at the right edge brings the side bar")
def _(env):
    page = env.home()
    page.mouse.move(632, 180)
    page.wait_for_timeout(300)
    expect(page.evaluate("document.elementFromPoint(615, 180)?.tagName") == "YTMP-UI", "no bar")


# =============================================================================
# E2E: the welcome
# =============================================================================


@test("e2e", "welcome: opens the first time")
def _(env):
    page = env.home(welcome=True, sidebar=False)
    expect(page.is_visible("#welcome"), "hidden")
    expect(page.inner_text("#welcome-title") == text("en", "home.welcome.pick.title"), page.inner_text("#welcome-title"))
    expect(page.evaluate("document.activeElement.id") == "welcome-next", "focus is not on Next")


@test("e2e", "welcome: stays away once seen")
def _(env):
    page = env.home(sidebar=False)
    expect(page.is_hidden("#welcome"), "shown")


@test("e2e", "welcome: Next walks the four steps and the last one finishes")
def _(env):
    page = env.home(welcome=True, sidebar=False)
    titles = []
    for _ in range(4):
        titles.append(page.inner_text("#welcome-title"))
        page.click("#welcome-next")
    expect(titles == [text("en", f"home.welcome.{s}.title") for s in ("pick", "move", "controls", "together")], titles)
    expect(page.is_hidden("#welcome") and sent(page) == ["welcome-done"], sent(page))


@test("e2e", "welcome: the counter says where you are")
def _(env):
    page = env.home(welcome=True, sidebar=False)
    page.click("#welcome-next")
    expect(page.inner_text("#welcome-count") == text("en", "home.welcome.count", step=2, count=4), page.inner_text("#welcome-count"))
    expect(page.locator("#welcome-dots .on").count() == 1 and page.locator("#welcome-dots span").count() == 4, "dots")


@test("e2e", "welcome: Back goes back, and is hidden on the first step")
def _(env):
    page = env.home(welcome=True, sidebar=False)
    expect(page.is_hidden("#welcome-back"), "Back on the first step")
    page.click("#welcome-next")
    page.click("#welcome-back")
    expect(page.inner_text("#welcome-title") == text("en", "home.welcome.pick.title"), "did not go back")


@test("e2e", "welcome: Skip closes it and tells the app")
def _(env):
    page = env.home(welcome=True, sidebar=False)
    page.click("#welcome-skip")
    expect(page.is_hidden("#welcome") and sent(page) == ["welcome-done"], sent(page))


@test("e2e", "welcome: no Skip on the last step (its button finishes)")
def _(env):
    page = env.home(welcome=True, sidebar=False)
    for _ in range(3):
        page.click("#welcome-next")
    expect(page.is_hidden("#welcome-skip"), "Skip shown")
    expect(page.inner_text("#welcome-next") == text("en", "home.welcome.done"), page.inner_text("#welcome-next"))


@test("e2e", "welcome: Escape skips it")
def _(env):
    page = env.home(welcome=True, sidebar=False)
    page.keyboard.press("Escape")
    expect(page.is_hidden("#welcome") and sent(page) == ["welcome-done"], sent(page))


@test("e2e", "welcome: arrow keys move between steps")
def _(env):
    page = env.home(welcome=True, sidebar=False)
    page.keyboard.press("ArrowRight")
    page.keyboard.press("ArrowRight")
    expect(page.inner_text("#welcome-title") == text("en", "home.welcome.controls.title"), page.inner_text("#welcome-title"))
    page.keyboard.press("ArrowLeft")
    expect(page.inner_text("#welcome-title") == text("en", "home.welcome.move.title"), page.inner_text("#welcome-title"))


@test("e2e", "welcome: number keys do not open services behind it")
def _(env):
    page = env.home(welcome=True, sidebar=False)
    page.keyboard.press("1")
    expect(sent(page) == [], sent(page))


@test("e2e", "welcome: each step has its own picture")
def _(env):
    page = env.home(welcome=True, sidebar=False)
    pictures = []
    for _ in range(4):
        pictures.append(page.inner_html("#welcome-art"))
        page.keyboard.press("ArrowRight")
    expect(len(set(pictures)) == 4 and all("<svg" in p for p in pictures), "pictures repeat or are missing")


@test("e2e", "welcome: after it closes, the focus goes to the services")
def _(env):
    page = env.home(welcome=True, sidebar=False)
    page.keyboard.press("Escape")
    expect(page.evaluate("document.activeElement.classList.contains('key')"), "focus lost")


@test("e2e", "welcome: a language change while open redraws it")
def _(env):
    page = env.home(welcome=True)
    page.keyboard.press("ArrowRight")
    push_locale(page, "ja", "home")
    page.wait_for_timeout(50)
    expect(page.inner_text("#welcome-title") == text("ja", "home.welcome.move.title"), page.inner_text("#welcome-title"))


@test("e2e", "welcome: Settings can show it again (the app's way)")
def _(env):
    body = re.search(r"Command::ShowWelcome => \{(.*?)\n            \}", APP_RS, re.S).group(1)
    expect("state.welcome = true" in body and "HOME_URL" in body, "ShowWelcome does not bring it back")


# =============================================================================
# E2E: Settings
# =============================================================================


@test("e2e", "settings: says it is ready when it loads")
def _(env):
    page = env.settings(push=False)
    expect(sent(page) == ["settings:ready"], sent(page))


@each("e2e", "settings: the {0} section opens from the rail", SECTIONS)
def _(env, section):
    page = env.settings()
    page.click(f"#tab-{section}")
    expect(page.inner_text("#page h2") == text("en", f"settings.section.{section}"), page.inner_text("#page h2"))
    expect(page.get_attribute(f"#tab-{section}", "aria-selected") == "true", "tab not selected")
    expect(page.locator("[role='tab'][aria-selected='true']").count() == 1, "two selected tabs")


@each("e2e", "settings: the app can open it at {0}", SECTIONS)
def _(env, section):
    page = env.settings(section=section)
    expect(page.inner_text("#page h2") == text("en", f"settings.section.{section}"), page.inner_text("#page h2"))


@test("e2e", "settings: arrow keys, Home and End move through the sections")
def _(env):
    page = env.settings()
    page.focus("#tab-general")
    page.keyboard.press("ArrowDown")
    expect(page.evaluate("document.activeElement.id") == "tab-window", "ArrowDown")
    page.keyboard.press("End")
    expect(page.evaluate("document.activeElement.id") == "tab-advanced", "End")
    page.keyboard.press("Home")
    expect(page.evaluate("document.activeElement.id") == "tab-general", "Home")
    page.keyboard.press("ArrowUp")
    expect(page.evaluate("document.activeElement.id") == "tab-advanced", "ArrowUp wraps")


@test("e2e", "settings: the section survives the reload a language change makes")
def _(env):
    page = env.settings(section="privacy")
    page.reload()
    push_settings(page, settings_state())
    expect(page.inner_text("#page h2") == text("en", "settings.section.privacy"), page.inner_text("#page h2"))


SWITCHES = [("general", "autostart", "autostart"), ("general", "start-with-last", "startWithLast"),
            ("window", "pin", "pinned"), ("window", "ghost", "ghost"), ("window", "video-only", "videoOnly"),
            ("window", "taskbar-bar", "taskbarBar"), ("advanced", "hardware-acceleration", "hardwareAcceleration")]


@each("e2e", "settings: the '{1}' switch sends the value wanted", SWITCHES)
def _(env, section, name, field):
    page = env.settings(section=section)
    switch = page.locator(f"[data-setting='{name}']")
    before = switch.get_attribute("aria-checked") == "true"
    expect(before == bool(settings_state()[field]), "does not show the app's state")
    clear(page)
    switch.click()
    expect(sent(page) == [f"settings:{name}:{0 if before else 1}"], sent(page))
    expect(switch.get_attribute("aria-checked") == str(not before).lower(), "does not show the change at once")


@each("e2e", "settings: the '{1}' switch follows the app's state", SWITCHES)
def _(env, section, name, field):
    page = env.settings(section=section)
    flipped = not settings_state()[field]
    push_settings(page, settings_state(**{field: flipped}))
    expect(page.get_attribute(f"[data-setting='{name}']", "aria-checked") == str(flipped).lower(), "state ignored")


@test("e2e", "settings: two quick clicks end where the person left it")
def _(env):
    page = env.settings(section="window")
    switch = page.locator("[data-setting='pin']")
    switch.click()
    switch.click()
    expect(sent(page)[-2:] == ["settings:pin:0", "settings:pin:1"], sent(page))


@test("e2e", "settings: the opacity slider moves with the arrow keys, 5 % a step")
def _(env):
    page = env.settings(section="window")
    page.focus("[data-setting='opacity']")
    for _ in range(8):
        page.keyboard.press("ArrowLeft")
    page.wait_for_timeout(200)
    messages = sent(page)
    expect(messages and messages[-1] == "settings:opacity:45", messages)
    values = [int(m.rsplit(":", 1)[1]) for m in messages]
    expect(all(v % 5 == 0 and 20 <= v <= 100 for v in values) and values == sorted(values, reverse=True), values)


@test("e2e", "settings: the opacity slider stops at 20 %")
def _(env):
    page = env.settings(section="window", state=settings_state(opacity=25))
    page.focus("[data-setting='opacity']")
    for _ in range(4):
        page.keyboard.press("ArrowLeft")
    page.wait_for_timeout(200)
    expect(sent(page)[-1] == "settings:opacity:20", sent(page))


@test("e2e", "settings: the opacity value is spoken as a percentage")
def _(env):
    page = env.settings(section="window", state=settings_state(opacity=70))
    expect(page.get_attribute("[data-setting='opacity']", "aria-valuetext") == "70%", "no valuetext")


@each("e2e", "settings: the {0} size button resizes the player", ["s", "m", "l"])
def _(env, size):
    page = env.settings(section="window")
    settings_button(page, f"size:{size}").click()
    expect(sent(page) == [f"settings:size:{size}"], sent(page))


@each("e2e", "settings: the {0} corner button moves the player", ["nw", "ne", "sw", "se"])
def _(env, corner):
    page = env.settings(section="window")
    settings_button(page, f"corner:{corner}").click()
    expect(sent(page) == [f"settings:corner:{corner}"], sent(page))


@each("e2e", "settings: choosing {0} as the language", [c for c in LANGS if c != "en"])
def _(env, code):
    page = env.settings()
    page.click(f"[role='radio'][data-value='{code}']")
    expect(sent(page) == [f"settings:lang:{code}"], sent(page))


@test("e2e", "settings: the current language is checked and choosing it again sends nothing")
def _(env):
    page = env.settings(lang="ru")
    expect(page.get_attribute("[role='radio'][data-value='ru']", "aria-checked") == "true", "not checked")
    page.click("[role='radio'][data-value='ru']")
    expect(sent(page) == [], sent(page))


@test("e2e", "settings: language radios answer arrow keys")
def _(env):
    page = env.settings()
    page.focus("[role='radio'][data-value='en']")
    page.keyboard.press("ArrowRight")
    expect(sent(page) == ["settings:lang:es"], sent(page))


@test("e2e", "settings: the name is saved once the typing stops")
def _(env):
    page = env.settings(section="together")
    field = page.locator("[data-setting='together-name']")
    field.fill("")
    field.type("Ana María", delay=20)
    page.wait_for_timeout(900)
    expect(sent(page) == ["settings:together-name:Ana María"], sent(page))
    expect(page.inner_text(".saved") == text("en", "settings.saved"), "no 'Saved'")


@test("e2e", "settings: Enter saves the name at once")
def _(env):
    page = env.settings(section="together")
    page.fill("[data-setting='together-name']", "Bruno")
    page.keyboard.press("Enter")
    expect(sent(page)[-1:] == ["settings:together-name:Bruno"], sent(page))


@test("e2e", "settings: a state from the app does not wipe the name being typed")
def _(env):
    page = env.settings(section="together")
    page.click("[data-setting='together-name']")
    page.keyboard.press("End")
    page.keyboard.type("XYZ")
    push_settings(page, settings_state(pinned=False))
    expect(page.input_value("[data-setting='together-name']") == "RodrigoXYZ", page.input_value("[data-setting='together-name']"))


@test("e2e", "settings: the name is capped at 32 characters")
def _(env):
    page = env.settings(section="together")
    page.fill("[data-setting='together-name']", "")
    page.type("[data-setting='together-name']", "n" * 40)
    expect(len(page.input_value("[data-setting='together-name']")) == 32, "longer than 32")


@each("e2e", "settings: hosting mode '{0}'", ["sync", "share"])
def _(env, mode):
    other = "share" if mode == "sync" else "sync"
    page = env.settings(section="together", state=settings_state(togetherMode=other))
    page.click(f"[role='radio'][data-value='{mode}']")
    expect(sent(page) == [f"settings:together-mode:{mode}"], sent(page))


@test("e2e", "settings: Watch together opens from its section")
def _(env):
    page = env.settings(section="together")
    settings_button(page, "together").click()
    expect(sent(page) == ["settings:together"], sent(page))


@test("e2e", "settings: during a session the section says so")
def _(env):
    page = env.settings(section="together", state=settings_state(togetherActive=True))
    expect(text("en", "settings.together_open_active") in page.inner_text("main"), "no notice")


@each("e2e", "settings: lock mode '{0}'", ["hello", "pin"])
def _(env, mode):
    page = env.settings(section="privacy")
    page.click(f"[role='radio'][data-value='{mode}']")
    expect(sent(page) == [f"settings:lock:{mode}"], sent(page))


@test("e2e", "settings: turning the lock off")
def _(env):
    page = env.settings(section="privacy", state=settings_state(lock="pin"))
    page.click("[role='radio'][data-value='off']")
    expect(sent(page) == ["settings:lock:off"], sent(page))


@test("e2e", "settings: Lock now needs a lock")
def _(env):
    page = env.settings(section="privacy")
    expect(settings_button(page, "lock-now").is_disabled(), "enabled without a lock")
    expect(text("en", "settings.lock_now_off") in page.inner_text("main"), "no reason given")
    push_settings(page, settings_state(lock="hello"))
    settings_button(page, "lock-now").click()
    expect(sent(page) == ["settings:lock-now"], sent(page))


@each("e2e", "settings: privacy button '{0}'", ["sign-out", "security"])
def _(env, message):
    page = env.settings(section="privacy")
    settings_button(page, message).click()
    expect(sent(page) == [f"settings:{message}"], sent(page))


@test("e2e", "settings: extensions are listed with their version")
def _(env):
    page = env.settings(section="extensions")
    items = page.locator(".list li").all_inner_texts()
    expect(len(items) == 2 and "SponsorBlock" in items[0] and "5.9" in items[0], items)


@test("e2e", "settings: an extension's switch")
def _(env):
    page = env.settings(section="extensions")
    page.click(f"[data-send='ext-toggle:{EXT_ID_2}']")
    expect(sent(page) == [f"settings:ext-toggle:{EXT_ID_2}"], sent(page))
    expect(page.get_attribute(f"[data-send='ext-toggle:{EXT_ID_2}']", "aria-checked") == "true", "not shown at once")


@test("e2e", "settings: Options only for extensions that have a page")
def _(env):
    page = env.settings(section="extensions")
    expect(page.locator(f"[data-send='ext-options:{EXT_ID}']").count() == 1, "missing")
    expect(page.locator(f"[data-send='ext-options:{EXT_ID_2}']").count() == 0, "extra")
    page.click(f"[data-send='ext-options:{EXT_ID}']")
    expect(sent(page) == [f"settings:ext-options:{EXT_ID}"], sent(page))


@test("e2e", "settings: an extension with a problem says what it is")
def _(env):
    broken = {"folder": "b", "name": "Broken", "version": "1", "id": None, "enabled": False, "hasPage": False,
              "error": "Invalid manifest", "pending": False}
    state = settings_state(extensions=[broken], extensionProblems=["“empty” has no manifest.json"])
    page = env.settings(section="extensions", state=state)
    expect(page.locator(".sub.warn").count() == 2, page.inner_text("main"))
    expect(page.locator(".list [role='switch']").count() == 0, "a switch for a broken extension")


@test("e2e", "settings: no extensions yet explains how to add them")
def _(env):
    page = env.settings(section="extensions", state=settings_state(extensions=[]))
    expect(text("en", "settings.ext_empty") in page.inner_text("main"), "no explanation")


@each("e2e", "settings: extensions button '{0}'", ["ext-folder", "ext-reload"])
def _(env, message):
    page = env.settings(section="extensions")
    settings_button(page, message).click()
    expect(sent(page) == [f"settings:{message}"], sent(page))


@test("e2e", "settings: a pending extension says it is installing")
def _(env):
    pending = dict(settings_state()["extensions"][0], pending=True)
    page = env.settings(section="extensions", state=settings_state(extensions=[pending]))
    expect(text("en", "settings.ext_installing") in page.inner_text("main"), "no 'Installing…'")


@test("e2e", "settings: hardware acceleration changed shows Restart now")
def _(env):
    page = env.settings(section="advanced", state=settings_state(hardwareAcceleration=False))
    expect(page.is_visible(".notice"), "no notice")
    settings_button(page, "restart").click()
    expect(sent(page) == ["settings:restart"], sent(page))


@test("e2e", "settings: no restart notice when nothing changed")
def _(env):
    page = env.settings(section="advanced")
    expect(page.locator(".notice").count() == 0, "notice shown")


@test("e2e", "settings: the maximum overlay row only when it is on")
def _(env):
    page = env.settings(section="advanced")
    expect(page.locator("[data-row='overlay']").count() == 0, "shown when off")
    push_settings(page, settings_state(overlay=True))
    expect(page.locator("[data-row='overlay']").count() == 1, "missing when on")


@test("e2e", "settings: global shortcuts, unavailable ones crossed out with the reason")
def _(env):
    page = env.settings(section="advanced")
    expect(page.locator(".list li").count() == 3 and page.locator(".list li.off").count() == 1, "list")
    expect("another app uses it" in page.inner_text(".list li.off"), "no reason")


@each("e2e", "settings: advanced button '{0}'", ["help", "open-folder"])
def _(env, message):
    page = env.settings(section="advanced")
    settings_button(page, message).click()
    expect(sent(page) == [f"settings:{message}"], sent(page))


@test("e2e", "settings: version and engine are shown")
def _(env):
    page = env.settings(section="advanced")
    facts = page.inner_text(".facts")
    expect("0.11.0" in facts and "130.0.2849.80" in facts, facts)
    expect("0.11.0" in page.inner_text("#about"), "rail")


@test("e2e", "settings: Show it again brings the welcome back")
def _(env):
    page = env.settings()
    settings_button(page, "welcome").click()
    expect(sent(page) == ["settings:welcome"], sent(page))


@test("e2e", "settings: Escape closes the window")
def _(env):
    page = env.settings()
    page.keyboard.press("Escape")
    expect(sent(page) == ["settings:close"], sent(page))


@test("e2e", "settings: the focused control keeps the focus when the state changes")
def _(env):
    page = env.settings(section="window")
    page.focus("[data-setting='ghost']")
    push_settings(page, settings_state(opacity=60))
    expect(page.evaluate("document.activeElement.dataset.setting") == "ghost", "focus lost")


@test("e2e", "settings: ghost mode without a tray says how to turn it off")
def _(env):
    page = env.settings(section="window", state=settings_state(tray=False))
    expect(text("en", "settings.ghost_sub_no_tray") in page.inner_text("main"), "tray mentioned without a tray")


# =============================================================================
# E2E: the side bar
# =============================================================================


@test("e2e", "side bar: has a Settings button")
def _(env):
    page = env.sidebar()
    expect(any(n == "Settings (Ctrl+,)" for n in ax_names(page)), "no Settings button")


@test("e2e", "side bar: the gear opens Settings")
def _(env):
    page = env.sidebar()
    page.mouse.move(630, 180)
    page.wait_for_timeout(250)
    clear(page)
    expect(ax_click(page, lambda n: n == "Settings (Ctrl+,)"), "not found")
    expect(home_sent(page) == ["settings"], home_sent(page))


@test("e2e", "side bar: Ctrl+, opens Settings")
def _(env):
    page = env.sidebar()
    clear(page)
    page.keyboard.press("Control+Comma")
    expect(home_sent(page) == ["settings"], home_sent(page))


@test("e2e", "side bar: Ctrl+, works on a Russian keyboard (by key position)")
def _(env):
    page = env.sidebar()
    clear(page)
    for kind in ("rawKeyDown", "keyUp"):
        page.cdp.send("Input.dispatchKeyEvent", {"type": kind, "modifiers": 2, "key": "б", "code": "Comma",
                                                 "windowsVirtualKeyCode": 188, "nativeVirtualKeyCode": 188})
    expect(home_sent(page) == ["settings"], home_sent(page))


def shadow(page, selector):
    """Reads an element inside the side bar's closed shadow root (CDP sees
    through it): its HTML and, if it is drawn, its box."""
    doc = page.cdp.send("DOM.getDocument", {"depth": -1, "pierce": True})

    def find(node):
        if node.get("nodeName") == "YTMP-UI":
            return node
        for child in node.get("children", []) + node.get("shadowRoots", []):
            found = find(child)
            if found:
                return found
        return None

    host = find(doc["root"])
    node_id = page.cdp.send("DOM.querySelector", {"nodeId": host["shadowRoots"][0]["nodeId"], "selector": selector})["nodeId"]
    if not node_id:
        return None
    html = page.cdp.send("DOM.getOuterHTML", {"nodeId": node_id})["outerHTML"]
    try:
        quad = page.cdp.send("DOM.getBoxModel", {"nodeId": node_id})["model"]["border"]
        box = {"x": quad[0], "y": quad[1], "width": quad[2] - quad[0], "height": quad[5] - quad[1]}
    except Exception:  # noqa: BLE001 - not drawn
        box = None
    return {"html": html, "box": box}


@test("e2e", "side bar: hovering a button names it at once, with its keys")
def _(env):
    page = env.sidebar()
    page.mouse.move(630, 180)
    page.wait_for_timeout(250)
    box = ax_box(page, lambda n: n == "Settings (Ctrl+,)")
    page.mouse.move(box["x"] + box["width"] / 2, box["y"] + box["height"] / 2)
    page.wait_for_timeout(350)
    tip = shadow(page, ".tip")
    expect(tip["box"] and not re.search(r"\shidden(?=[\s=>])", tip["html"].split(">")[0]), f"no label ({tip['html'][:80]})")
    expect(">Settings<" in tip["html"] and "<kbd>Ctrl+,</kbd>" in tip["html"], tip["html"])
    expect(tip["box"]["x"] + tip["box"]["width"] <= box["x"], "the label covers the bar")
    expect(abs((tip["box"]["y"] + tip["box"]["height"] / 2) - (box["y"] + box["height"] / 2)) < 4, "not next to its button")


@test("e2e", "side bar: the label goes when the pointer leaves")
def _(env):
    page = env.sidebar()
    page.mouse.move(630, 180)
    page.wait_for_timeout(250)
    box = ax_box(page, lambda n: n == "Settings (Ctrl+,)")
    page.mouse.move(box["x"] + box["width"] / 2, box["y"] + box["height"] / 2)
    page.wait_for_timeout(350)
    page.mouse.move(300, 180)
    page.wait_for_timeout(100)
    expect(shadow(page, ".tip")["box"] is None, "label stayed")


@test("e2e", "side bar: reached with Tab, the bar shows and names the button")
def _(env):
    page = env.sidebar()
    page.mouse.move(300, 180)
    page.wait_for_timeout(1500)
    shown = None
    for _ in range(12):
        page.keyboard.press("Tab")
        page.wait_for_timeout(30)
        tip = shadow(page, ".tip")
        if tip["box"]:
            shown = tip
            break
    expect(shown, "Tab never reached a named bar button")
    expect(page.evaluate("document.elementFromPoint(620, 180)?.tagName") == "YTMP-UI", "the bar stayed hidden")


@test("e2e", "side bar: the label is hidden from screen readers (the button has the name)")
def _(env):
    page = env.sidebar()
    tip = shadow(page, ".tip")
    expect('aria-hidden="true"' in tip["html"], tip["html"][:80])


@test("e2e", "side bar: no system tooltips (the bar's own label replaces them)")
def _(env):
    src = (SRC / "ui.js").read_text(encoding="utf-8")
    expect("button.removeAttribute('title')" in src and "button.title =" not in src, "title still set")


@test("e2e", "side bar: the menu has a Settings row")
def _(env):
    page = env.sidebar()
    ys = bar_button_y(page, 0)
    page.mouse.click(611, ys[-1] - 50)
    page.wait_for_timeout(150)
    clear(page)
    expect(ax_click(page, lambda n: n.startswith(text("en", "ui.settings_all"))), "no row")
    expect(home_sent(page) == ["settings"], home_sent(page))


@test("e2e", "side bar: the menu's Extensions row opens Settings there")
def _(env):
    page = env.sidebar()
    ys = bar_button_y(page, 0)
    page.mouse.click(611, ys[-1] - 50)
    page.wait_for_timeout(150)
    clear(page)
    expect(ax_click(page, lambda n: n.startswith(text("en", "ui.section.extensions"))), "no row")
    expect(home_sent(page) == ["settings:extensions"], home_sent(page))


@test("e2e", "side bar: the menu is short (startup and graphics live in Settings)")
def _(env):
    page = env.sidebar()
    ys = bar_button_y(page, 0)
    page.mouse.click(611, ys[-1] - 50)
    page.wait_for_timeout(150)
    names = ax_names(page)
    for gone in ("Start with Windows", "Hardware acceleration", "SponsorBlock"):
        expect(not any(n.startswith(gone) for n in names), f"{gone} still in the menu")


@test("e2e", "side bar: the menu opens at its top every time")
def _(env):
    page = env.sidebar(height=270)
    ys = bar_button_y(page, 0)
    page.mouse.click(611, ys[-1] - 50)
    page.wait_for_timeout(100)
    ax_click(page, lambda n: n.startswith(text("en", "ui.section.extensions")))
    ys = bar_button_y(page, 0)
    page.mouse.click(611, ys[-1] - 50)
    page.wait_for_timeout(100)
    box = ax_box(page, lambda n: n.startswith("Always on top"))
    expect(box and box["y"] < 80, f"opened scrolled ({box})")


@test("e2e", "side bar: Settings › Keyboard shortcuts opens the help (only for the app)")
def _(env):
    page = env.sidebar()
    page.evaluate("window.__ytmpHelp('wrong-key')")
    expect(not any(n == text("en", "ui.shortcuts") for n in ax_names(page)[:3]) and "Ctrl+," not in
           " ".join(n for n in ax_names(page) if n.startswith(text("en", "ui.help.window"))), "opened without the key")
    page.evaluate(f"window.__ytmpHelp({json.dumps(HOST_KEY)})")
    page.wait_for_timeout(50)
    names = ax_names(page)
    expect(any(n == text("en", "ui.shortcuts") for n in names), "help not open")
    expect(any(n == "Ctrl+," for n in names), "Ctrl+, missing from the help")


# =============================================================================
# SECURITY
# =============================================================================

PAGES = ["home.html", "settings.html", "together.html", "stream.html", "cast.html", "lock.html", "mini.html",
         "music.html"]
SCRIPTS = ["ui.js", "inject.js", "media.js", "maintenance.js"]
DANGEROUS = {
    "innerHTML": r"\.innerHTML\s*=", "outerHTML": r"\.outerHTML\s*=", "insertAdjacentHTML": r"insertAdjacentHTML\(",
    "document.write": r"document\.write\(", "eval": r"(?<![\w.])eval\(", "new Function": r"new Function\(",
    "string timers": r"set(?:Timeout|Interval)\(\s*['\"`]",
}


@each("security", "{0}: no {1}", [(f, d) for f in PAGES + SCRIPTS for d in DANGEROUS])
def _(env, file, danger):
    src = (SRC / file).read_text(encoding="utf-8")
    hits = re.findall(DANGEROUS[danger], src)
    expect(not hits, f"{len(hits)} use(s)")


@each("security", "{0}: loads nothing from the Internet", PAGES)
def _(env, file):
    src = (SRC / file).read_text(encoding="utf-8")
    loads = re.findall(r"""(?:src|href)\s*=\s*["'](https?:)?//""", src) + re.findall(r"@import|url\(\s*['\"]?https?:", src)
    expect(not loads, loads)


@each("security", "{0} in {1}: no CSP violations", [(p, l) for p in ("home", "settings") for l in LANGS])
def _(env, page_kind, lang):
    page = env.page(760, 560)
    page.add_init_script("window.__csp = []; document.addEventListener('securitypolicyviolation', "
                         "(e) => window.__csp.push(e.violatedDirective + ' ' + e.blockedURI));")
    if page_kind == "home":
        page.add_init_script(IPC_STUB)
        env.serve(page, home_html(lang=lang, welcome=True))
        page.goto("https://ytmp.localhost/")
        for _ in range(4):
            page.keyboard.press("ArrowRight")
    else:
        page.add_init_script("Object.defineProperty(window, 'ipc', { value: { postMessage() {} } });")
        env.serve(page, fill("settings.html", {"i18n": i18n(lang, "settings")}, lang))
        page.goto("https://ytmp.localhost/settings")
        for section in SECTIONS:
            push_settings(page, settings_state(hardwareAcceleration=False, overlay=True), section)
    page.wait_for_timeout(50)
    expect(page.evaluate("window.__csp") == [], page.evaluate("window.__csp"))
    expect(not page.errors, page.errors[:2])


@each("security", "{0} page: served with a strict policy", ["CSP", "MINI_CSP"])
def _(env, name):
    csp = CSP if name == "CSP" else MINI_CSP
    for directive in ("default-src 'none'", "base-uri 'none'", "form-action 'none'", "frame-ancestors 'none'"):
        expect(directive in csp, f"missing {directive}")
    expect("unsafe-eval" not in csp, "unsafe-eval")
    expect("script-src '{script}'" in csp and "script-src 'unsafe-inline'" not in csp, "scripts only by their hash")


@test("security", "My music: its script is allowed by hash, not 'unsafe-inline'")
def _(env):
    csp = music_csp()
    expect("script-src 'sha256-" in csp and "script-src 'unsafe-inline'" not in csp, csp)


@test("security", "settings: an extension name with HTML is shown as text")
def _(env):
    evil = dict(settings_state()["extensions"][0], name="<img src=x onerror=alert(1)><b>bold</b>")
    page = env.settings(section="extensions", state=settings_state(extensions=[evil]))
    expect(page.locator("main img, main b").count() == 0, "markup was parsed")
    expect("<img src=x" in page.inner_text("main"), "name not shown")


@test("security", "settings: a hostile name in the state is shown as text")
def _(env):
    page = env.settings(section="together", state=settings_state(togetherName="\"><script>alert(1)</script>"))
    expect(page.input_value("[data-setting='together-name']") == "\"><script>alert(1)</script>", "value changed")
    expect(page.locator("main script").count() == 0, "a script element appeared")


@test("security", "settings: a broken state from outside is ignored")
def _(env):
    page = env.settings()
    page.evaluate("window.__ytmpSettings(null); window.__ytmpSettings('x'); window.__ytmpSettings(42)")
    expect(page.inner_text("#page h2") == text("en", "settings.section.general"), "the page broke")
    expect(not page.errors, page.errors[:2])


@test("security", "settings: its state function cannot be replaced")
def _(env):
    page = env.settings()
    page.evaluate("try { window.__ytmpSettings = () => {}; } catch (e) {}; try { delete window.__ytmpSettings } catch (e) {}")
    push_settings(page, settings_state(), "privacy")
    expect(page.inner_text("#page h2") == text("en", "settings.section.privacy"), "replaced")


@test("security", "settings: an unknown section name opens nothing")
def _(env):
    page = env.settings(section="../../etc")
    expect(page.inner_text("#page h2") == text("en", "settings.section.general"), page.inner_text("#page h2"))


@each("security", "side bar: {0} only obeys the app's key", ["__ytmpHelp", "__ytmpSync", "__ytmpToast", "__ytmpLocale"])
def _(env, function):
    page = env.sidebar()
    names_before = len(ax_names(page))
    args = {"__ytmpHelp": "'nope'", "__ytmpSync": "'nope', { ghost: true, pinned: false }",
            "__ytmpToast": "'nope', 'forged notice'", "__ytmpLocale": "'nope', { lang: 'ja', ui: {} }"}[function]
    page.evaluate(f"window.{function}({args})")
    page.wait_for_timeout(50)
    names = ax_names(page)
    expect("forged notice" not in names and abs(len(names) - names_before) < 3, "obeyed a forged call")
    page.evaluate(f"window.{function} = () => 'replaced'")
    expect(page.evaluate(f"String(window.{function}('x')) !== 'replaced'"), "replaced")


@test("security", "a hostile site cannot open Settings without the session token")
def _(env):
    page = env.sidebar()
    clear(page)
    page.evaluate("window.chrome.webview.postMessage('settings'); window.ipc.postMessage('settings:extensions')")
    expect("settings" not in home_sent(page) and "settings:extensions" not in home_sent(page), home_sent(page))
    expect("settings" in page.evaluate("window.__forged"), "not dropped as forged")


@test("security", "start screen: data that tries to close the script tag stays data")
def _(env):
    page = env.page()
    page.add_init_script(IPC_STUB)
    env.serve(page, home_html(last_service="</script><script>window.pwned=1</script>"))
    page.goto("https://ytmp.localhost/")
    expect(not page.evaluate("window.pwned"), "the data broke out")
    expect(page.locator(".key").count() == 9, "page broken")


@test("security", "the Settings parser refuses commands from other windows")
def _(env):
    body = re.search(r"pub fn parse_settings\(.*?\n}\n", IPC_RS, re.S).group(0)
    for forbidden in ("Quit", "OpenUrl", "OpenService", "Navigate", "Music(", "TogetherStart", "TogetherJoin",
                      "CastOffer", "LockPin", "OpenExternal"):
        expect(forbidden not in body, f"parse_settings can produce {forbidden}")


@test("security", "Settings cannot set a PIN or unlock by itself")
def _(env):
    body = re.search(r"Command::SettingsLock\(mode\) => \{(.*?)\n            \}", APP_RS, re.S).group(1)
    expect("begin_change" in body and "apply_lock" not in body, "changes the lock without verifying")


@test("security", "dialogs asked from Settings open above it")
def _(env):
    expect(APP_RS.count("security::ask(self.dialog_owner()") >= 2 and "security::inform(self.dialog_owner()" in APP_RS,
           "a dialog could hide behind the Settings window")


@test("security", "settings: the name field is not remembered by autofill")
def _(env):
    page = env.settings(section="together")
    expect(page.get_attribute("[data-setting='together-name']", "autocomplete") == "off", "autocomplete on")


@test("security", "settings: no right-click browser menu outside the fields")
def _(env):
    page = env.settings()
    prevented = page.evaluate("""() => { const e = new MouseEvent('contextmenu', { bubbles: true, cancelable: true });
        document.querySelector('h2').dispatchEvent(e); return e.defaultPrevented; }""")
    expect(prevented, "context menu allowed")


# =============================================================================
# ACCESSIBILITY
# =============================================================================


@each("a11y", "start screen in {0}: every control has a name", LANGS)
def _(env, lang):
    page = env.home(lang=lang, width=420, height=300, sidebar=False)
    unnamed = page.evaluate(UNNAMED_JS)
    expect(not unnamed, unnamed[:3])


@each("a11y", "welcome in {0}: every control has a name", LANGS)
def _(env, lang):
    page = env.home(lang=lang, welcome=True, sidebar=False)
    unnamed = page.evaluate(UNNAMED_JS)
    expect(not unnamed, unnamed[:3])


@each("a11y", "settings › {0}: every control has a name", SECTIONS)
def _(env, section):
    page = env.settings(section=section, state=settings_state(hardwareAcceleration=False, overlay=True, lock="pin"))
    unnamed = page.evaluate(UNNAMED_JS)
    expect(not unnamed, unnamed[:3])


@each("a11y", "settings › {0}: switches and fields say what they are", SECTIONS)
def _(env, section):
    page = env.settings(section=section)
    loose = page.evaluate("""() => [...document.querySelectorAll('main [role="switch"], main input')]
        .filter((el) => !el.getAttribute('aria-label') && !el.getAttribute('aria-labelledby')).map((el) => el.outerHTML.slice(0, 60))""")
    expect(not loose, loose)


@each("a11y", "settings › {0}: Tab reaches every control and shows where the focus is", SECTIONS)
def _(env, section):
    page = env.settings(section=section, state=settings_state(lock="hello"))
    page.focus(f"#tab-{section}")
    controls = page.locator("main button:not([disabled]), main input").count()
    seen = set()
    for _ in range(controls + 2):
        page.keyboard.press("Tab")
        info = page.evaluate("""() => { const el = document.activeElement; const cs = getComputedStyle(el);
            return { inMain: !!el.closest('main'), id: el.dataset.send || el.dataset.setting || el.dataset.value || el.textContent.slice(0, 20),
                     outline: cs.outlineStyle !== 'none' || cs.boxShadow !== 'none' }; }""")
        if info["inMain"]:
            expect(info["outline"], f"no visible focus on {info['id']}")
            seen.add(info["id"])
    expect(len(seen) >= min(controls, 3) - 1 if controls else True, f"reached {len(seen)} of {controls}")


@test("a11y", "settings: radio groups take one Tab stop each")
def _(env):
    page = env.settings(section="privacy")
    stops = page.evaluate("""() => [...document.querySelectorAll('[role="radiogroup"]')].map((g) =>
        [...g.querySelectorAll('[role="radio"]')].filter((r) => r.tabIndex === 0).length)""")
    expect(stops and all(s == 1 for s in stops), stops)


@test("a11y", "settings: the rail is a tablist with one selected tab and a linked panel")
def _(env):
    page = env.settings(section="window")
    expect(page.locator("[role='tablist'] [role='tab']").count() == 6, "tabs")
    expect(page.get_attribute("#page", "role") == "tabpanel" and page.get_attribute("#page", "aria-labelledby") == "tab-window",
           "panel not linked")
    expect(page.locator("[role='tab'][tabindex='0']").count() == 1, "roving tabindex")


@test("a11y", "settings: a switch toggles with Space and Enter")
def _(env):
    page = env.settings(section="window")
    page.focus("[data-setting='ghost']")
    page.keyboard.press("Space")
    page.keyboard.press("Enter")
    expect(sent(page) == ["settings:ghost:1", "settings:ghost:0"], sent(page))


@test("a11y", "welcome: a dialog with a title, a description and the focus kept inside")
def _(env):
    page = env.home(welcome=True, sidebar=False)
    expect(page.get_attribute("#welcome .sheet", "aria-modal") == "true", "not modal")
    expect(page.get_attribute("#welcome .sheet", "aria-labelledby") == "welcome-title", "no title")
    for _ in range(6):
        page.keyboard.press("Tab")
        expect(page.evaluate("!!document.activeElement.closest('#welcome')"), "focus escaped")
    page.keyboard.press("Shift+Tab")
    expect(page.evaluate("!!document.activeElement.closest('#welcome')"), "focus escaped backwards")


@test("a11y", "welcome: can be finished with the keyboard alone")
def _(env):
    page = env.home(welcome=True, sidebar=False)
    for _ in range(4):
        page.keyboard.press("Enter")
    expect(page.is_hidden("#welcome") and sent(page) == ["welcome-done"], sent(page))


@each("a11y", "{0} page says its language ({1})", [(p, l) for p in ("home", "settings") for l in LANGS])
def _(env, kind, lang):
    page = env.home(lang=lang, sidebar=False) if kind == "home" else env.settings(lang=lang)
    expect(page.evaluate("document.documentElement.lang") == lang, page.evaluate("document.documentElement.lang"))


@test("a11y", "start screen: pictures are hidden from screen readers")
def _(env):
    page = env.home(welcome=True, sidebar=False)
    loose = page.evaluate("""() => [...document.querySelectorAll('svg')].filter((s) => !s.closest('[aria-hidden="true"]')
        && s.getAttribute('aria-hidden') !== 'true').length""")
    expect(loose == 0, f"{loose} svg without aria-hidden")


@test("a11y", "settings: icons are hidden from screen readers")
def _(env):
    page = env.settings()
    loose = page.evaluate("[...document.querySelectorAll('svg')].filter((s) => s.getAttribute('aria-hidden') !== 'true').length")
    expect(loose == 0, f"{loose}")


@test("a11y", "start screen keys: the big digit is not read twice")
def _(env):
    page = env.home(sidebar=False)
    expect(page.locator(".key .digit[aria-hidden='true']").count() == 9, "digits read out")


@each("a11y", "{0}: buttons are at least 24×24 px (WCAG 2.2)", ["home", "welcome", "settings"])
def _(env, kind):
    if kind == "settings":
        page = env.settings(section="window")
    else:
        page = env.home(welcome=kind == "welcome", sidebar=False)
    small = page.evaluate("""() => [...document.querySelectorAll('button')].filter((b) => {
        if (b.closest('[hidden]') || getComputedStyle(b).display === 'none') return false;
        const r = b.getBoundingClientRect(); return r.width && (r.width < 24 || r.height < 24); })
        .map((b) => b.textContent.trim() || b.getAttribute('aria-label') || b.className)""")
    expect(not small, small[:4])


CONTRAST_PAIRS = [
    ("main text on the water", "cyan", "deep", 4.5),
    ("secondary text on the water", "frost", "deep", 4.5),
    ("pale text on the water", "frost2", "deep", 4.5),
    ("button text on turquoise", "deep", "turquoise", 4.5),
    ("button text on sky aqua (hover)", "deep", "sky", 4.5),
    ("key caps", "frost2", "french", 4.5),
    ("problems in coral", "alert", "deep", 4.5),
    ("section headings", "sky", "deep", 4.5),
    ("selected tab", "cyan", "french", 4.5),
    ("teal outlines on the water (non-text, 3:1)", "teal", "deep", 3.0),
    ("focus ring", "sky", "deep", 3.0),
    ("chip text on its outline", "frost2", "deep", 4.5),
]


@each("a11y", "contrast: {0}", CONTRAST_PAIRS)
def _(env, label, fg, bg, minimum):
    ratio = contrast(PALETTE[fg], PALETTE[bg])
    expect(ratio >= minimum, f"{ratio:.2f} < {minimum}")


@test("a11y", "contrast: the dimmest text (62–70 % frosted blue) still reads")
def _(env):
    for alpha in (0.62, 0.7):
        ratio = contrast(blend(PALETTE["frost"], alpha, PALETTE["deep"]), PALETTE["deep"])
        expect(ratio >= 4.5, f"{alpha}: {ratio:.2f}")


@each("a11y", "reduced motion: the waves stop on {0}", ["home.html", "together.html", "lock.html", "stream.html"])
def _(env, file):
    page = env.page(640, 360, reduced_motion=True)
    page.add_init_script("Object.defineProperty(window, 'ipc', { value: { postMessage() {} } });")
    kind = file.split(".")[0]
    data = {"i18n": i18n("en", kind)} if kind != "home" else None
    env.serve(page, home_html() if kind == "home" else fill(file, data))
    page.goto("https://ytmp.localhost/" + ("" if kind == "home" else kind))
    moving = page.evaluate("[...document.querySelectorAll('.waves svg')].map((s) => getComputedStyle(s).animationName)")
    expect(moving and all(m == "none" for m in moving), f"still moving: {moving}")


@test("a11y", "reduced motion: switches jump instead of sliding")
def _(env):
    page = env.page(760, 560, reduced_motion=True)
    page.add_init_script("Object.defineProperty(window, 'ipc', { value: { postMessage() {} } });")
    env.serve(page, fill("settings.html", {"i18n": i18n("en", "settings")}))
    page.goto("https://ytmp.localhost/settings")
    push_settings(page, settings_state())
    duration = page.evaluate("getComputedStyle(document.querySelector('.switch')).transitionDuration")
    expect(duration in ("0s", ""), duration)


@test("a11y", "start screen: the address field has a label")
def _(env):
    page = env.home(sidebar=False)
    expect(page.evaluate("document.getElementById('site-url').labels[0].textContent") == text("en", "home.other_site"), "label")


# =============================================================================
# PERFORMANCE
# =============================================================================

BUDGETS = {"home.html": 48_000, "settings.html": 48_000, "ui.js": 56_000, "inject.js": 16_000,
           "together.html": 60_000, "stream.html": 40_000, "lock.html": 24_000, "mini.html": 36_000,
           "media.js": 24_000, "maintenance.js": 12_000}


@each("performance", "{0} stays under {1:,} bytes", list(BUDGETS.items()))
def _(env, file, budget):
    size = len((SRC / file).read_bytes())
    expect(size <= budget, f"{size:,} bytes")


@test("performance", "start screen: drawn within 400 ms")
def _(env):
    page = env.page()
    page.add_init_script(IPC_STUB)
    env.serve(page, home_html())
    start = time.perf_counter()
    page.goto("https://ytmp.localhost/")
    page.wait_for_selector(".key")
    elapsed = (time.perf_counter() - start) * 1000
    expect(elapsed < 400, f"{elapsed:.0f} ms")


@test("performance", "settings: drawn within 400 ms of its state")
def _(env):
    page = env.settings(push=False)
    elapsed = page.evaluate("""(state) => { const t = performance.now(); window.__ytmpSettings(state);
        return performance.now() - t; }""", settings_state())
    expect(elapsed < 400, f"{elapsed:.0f} ms")


@test("performance", "settings: 100 state updates in under 600 ms")
def _(env):
    page = env.settings(section="advanced")
    elapsed = page.evaluate("""(state) => { const t = performance.now();
        for (let i = 0; i < 100; i++) window.__ytmpSettings(Object.assign({}, state, { opacity: 20 + i % 80 }));
        return performance.now() - t; }""", settings_state())
    expect(elapsed < 600, f"{elapsed:.0f} ms")


@test("performance", "settings: redrawing does not pile up nodes")
def _(env):
    page = env.settings(section="extensions")
    before = page.evaluate("document.getElementsByTagName('*').length")
    page.evaluate("(state) => { for (let i = 0; i < 50; i++) window.__ytmpSettings(state); }", settings_state())
    after = page.evaluate("document.getElementsByTagName('*').length")
    expect(after == before, f"{before} → {after}")


@each("performance", "settings › {0}: under 400 elements", SECTIONS)
def _(env, section):
    page = env.settings(section=section)
    count = page.evaluate("document.getElementsByTagName('*').length")
    expect(count < 400, count)


@test("performance", "start screen: under 450 elements, welcome included")
def _(env):
    page = env.home(welcome=True, sidebar=False)
    count = page.evaluate("document.getElementsByTagName('*').length")
    expect(count < 450, count)


@each("performance", "{0}: no request leaves the app", ["home", "settings", "welcome"])
def _(env, kind):
    page = env.settings() if kind == "settings" else env.home(welcome=kind == "welcome")
    page.wait_for_timeout(150)
    expect(not page.external, page.external[:3])


def own_long_tasks(page, url, after=None, settle=200):
    """The long tasks of the page's own load: it loads twice in the same tab,
    and the second load is the one measured.

    The first page a new browser process lays out also pays a one-time setup
    for drawing text (the system fonts in each weight, the Japanese ones for
    ja, their caches): 50 to 200 ms depending on the machine, before anything
    is on screen, and every test here starts a new process. The first load
    takes that; the second is what the page itself does on every load. The
    cold start has budgets of its own ("drawn within 500 ms").
    """
    for _ in range(2):
        page.goto(url)
        if after:
            after()
        page.wait_for_timeout(settle)
    return page.evaluate("window.__long")


@test("performance", "start screen: no long tasks while loading")
def _(env):
    page = env.page()
    page.add_init_script("window.__long = []; new PerformanceObserver((l) => l.getEntries().forEach((e) => "
                         "window.__long.push(Math.round(e.duration)))).observe({ type: 'longtask', buffered: true });")
    page.add_init_script(IPC_STUB)
    env.serve(page, home_html(welcome=True))
    tasks = own_long_tasks(page, "https://ytmp.localhost/")
    expect(tasks == [], tasks)


@test("performance", "settings: idle means idle (no timers keep running)")
def _(env):
    page = env.page(760, 560)
    page.add_init_script("""window.__timers = 0; const st = window.setTimeout, si = window.setInterval;
        window.setInterval = function () { window.__timers += 100; return si.apply(this, arguments); };
        window.setTimeout = function () { window.__timers++; return st.apply(this, arguments); };
        Object.defineProperty(window, 'ipc', { value: { postMessage() {} } });""")
    env.serve(page, fill("settings.html", {"i18n": i18n("en", "settings")}))
    page.goto("https://ytmp.localhost/settings")
    push_settings(page, settings_state())
    page.wait_for_timeout(100)
    before = page.evaluate("window.__timers")
    page.wait_for_timeout(600)
    expect(page.evaluate("window.__timers") == before and before < 100, f"timers: {before} → {page.evaluate('window.__timers')}")


@test("performance", "start screen: only the DRM check's one-shot timer while idle")
def _(env):
    page = env.page()
    page.add_init_script("""window.__intervals = 0; const si = window.setInterval;
        window.setInterval = function () { window.__intervals++; return si.apply(this, arguments); };""")
    page.add_init_script(IPC_STUB)
    env.serve(page, home_html())
    page.goto("https://ytmp.localhost/")
    page.wait_for_timeout(300)
    expect(page.evaluate("window.__intervals") == 0, "a repeating timer")


@test("performance", "settings: typing a name sends one message, not one per key")
def _(env):
    page = env.settings(section="together")
    page.fill("[data-setting='together-name']", "")
    page.type("[data-setting='together-name']", "Carla Fernández", delay=15)
    page.wait_for_timeout(900)
    expect(len(sent(page)) == 1, sent(page))


@test("performance", "settings: dragging opacity end to end sends a handful of messages")
def _(env):
    page = env.settings(section="window")
    box = page.locator("[data-setting='opacity']").bounding_box()
    clear(page)
    page.mouse.move(box["x"] + 2, box["y"] + box["height"] / 2)
    page.mouse.down()
    page.mouse.move(box["x"] + box["width"] - 2, box["y"] + box["height"] / 2, steps=40)
    page.mouse.up()
    page.wait_for_timeout(200)
    expect(1 <= len(sent(page)) <= 12, f"{len(sent(page))} messages")
    expect(sent(page)[-1] == "settings:opacity:100", sent(page)[-1:])


@test("performance", "the app sends Settings a state only when it changed")
def _(env):
    body = re.search(r"fn push_settings\(&self, force: bool\) \{(.*?)\n    \}", APP_RS, re.S).group(1)
    expect("settings_shown" in body and "!self.settings_panel.visible()" in body, "no change check or visibility check")


@test("performance", "the Settings page is created only when first opened")
def _(env):
    panel = (SRC / "settings_panel.rs").read_text(encoding="utf-8")
    expect("fn ensure_webview" in panel and "webview: None" in panel, "created at startup")


@each("performance", "{0}: no web fonts or @import", ["home.html", "settings.html", "ui.js"])
def _(env, file):
    src = (SRC / file).read_text(encoding="utf-8")
    expect("@font-face" not in src and "@import" not in src, "fonts or imports")


# =============================================================================
# LAYOUT: every language at the window's real sizes
# =============================================================================

HOME_SIZES = [(640, 360), (480, 270), (960, 540), (360, 220)]


@each("layout", "start screen {0} at {1}×{2}: nothing cut or sideways",
      [(l, w, h) for l in LANGS for (w, h) in HOME_SIZES])
def _(env, lang, width, height):
    page = env.home(lang=lang, width=width, height=height, last="youtube", sidebar=False)
    issues = page.evaluate(FIT_JS, "main")
    expect(not issues, issues)
    expect(page.evaluate("document.documentElement.scrollWidth <= innerWidth"), "scrolls sideways")


@each("layout", "start screen {0} at {1}×{2}: all nine keys and the address fit",
      [(l, w, h) for l in LANGS for (w, h) in HOME_SIZES[:3]])
def _(env, lang, width, height):
    page = env.home(lang=lang, width=width, height=height, sidebar=False)
    out = page.evaluate("""() => [...document.querySelectorAll('.key, #site-url, .open, #together, #settings')]
        .filter((el) => { const r = el.getBoundingClientRect(); return r.bottom > innerHeight + 1 || r.right > innerWidth + 1 || r.height < 28; })
        .map((el) => el.dataset.id || el.id || el.className)""")
    expect(not out, out)
    clamped = page.evaluate(CLAMPED_JS, ".key .name span")
    expect(not clamped, f"names cut: {clamped}")


@each("layout", "welcome {0} at {1}×{2}: every step fits, buttons in view",
      [(l, w, h) for l in LANGS for (w, h) in [(640, 360), (480, 270), (960, 540)]])
def _(env, lang, width, height):
    page = env.home(lang=lang, width=width, height=height, welcome=True, sidebar=False)
    for step in range(4):
        r = page.locator("#welcome-next").bounding_box()
        expect(r["y"] + r["height"] <= height and r["x"] + r["width"] <= width, f"step {step + 1}: Next out of view")
        scroll = page.evaluate("(() => { const s = document.querySelector('.sheet'); return s.scrollHeight - s.clientHeight; })()")
        expect(scroll <= 1, f"step {step + 1}: the text needs scrolling ({scroll}px)")
        issues = page.evaluate(FIT_JS, ".sheet")
        expect(not issues, f"step {step + 1}: {issues}")
        page.keyboard.press("ArrowRight")


@each("layout", "settings › {1} in {0} at {2}×{3}: nothing cut or sideways",
      [(l, s, w, h) for l in LANGS for s in SECTIONS for (w, h) in [(760, 560), (420, 400)]])
def _(env, lang, section, width, height):
    state = settings_state(hardwareAcceleration=False, overlay=True, lock="pin", togetherActive=True)
    page = env.settings(lang=lang, width=width, height=height, section=section, state=state)
    issues = page.evaluate(FIT_JS, "main")
    expect(not issues, issues)
    rail = page.evaluate(FIT_JS, "nav") if width > 620 else []
    expect(not rail, f"rail: {rail}")


@each("layout", "side bar menu in {0} at 480×270: rows fit the menu", LANGS)
def _(env, lang):
    page = env.sidebar(lang=lang, width=480, height=270)
    ys = bar_button_y(page, 0)
    page.mouse.click(451, ys[-1] - 50)
    page.wait_for_timeout(150)
    box = ax_box(page, lambda n: n.startswith(text(lang, "ui.settings_all")))
    expect(box and box["x"] >= 0 and box["x"] + box["width"] <= 480, f"Settings row out of view ({box})")


# =============================================================================
# I18N: every new text in the five languages
# =============================================================================

NEW_KEYS = sorted(k for k in CATALOGS["en"] if k.startswith(("settings.", "home.welcome.")) or k in (
    "home.title", "home.lede", "home.settings", "home.settings_tip", "home.edge_hint", "home.together_caption",
    "ui.settings", "ui.settings_all", "ui.settings_sub", "ui.ext.none", "ui.ext.summary", "ui.ext.problems",
    "tray.open_settings", "app.settings_failed"))


@each("i18n", "{0}: in all five languages, same placeholders and key caps", NEW_KEYS)
def _(env, key):
    english = CATALOGS["en"][key]
    for lang in LANGS:
        value = CATALOGS[lang].get(key)
        expect(value, f"{lang}: missing or empty")
        expect(placeholders(value) == placeholders(english), f"{lang}: placeholders {placeholders(value)}")
        if isinstance(value, str):
            expect(kbd_caps(value) == kbd_caps(english), f"{lang}: key caps {kbd_caps(value)}")
            expect(value.strip() == value and "  " not in value, f"{lang}: stray spaces")
            if lang != "en" and len(english) > 12 and key not in ("settings.engine", "settings.lock_hello"):
                expect(value != english, f"{lang}: still in English")


@test("i18n", "plural texts have every form each language needs")
def _(env):
    needs = {"en": {"one", "other"}, "es": {"one", "other"}, "ru": {"one", "few", "many"},
             "pl": {"one", "few", "many"}, "ja": {"other"}}
    for lang in LANGS:
        for key, value in CATALOGS[lang].items():
            if isinstance(value, dict):
                expect(needs[lang] <= set(value), f"{lang} {key}: {sorted(value)}")


@test("i18n", "no key is missing or left over in any language")
def _(env):
    keys = set(CATALOGS["en"])
    for lang in LANGS[1:]:
        expect(set(CATALOGS[lang]) == keys, f"{lang}: {sorted(set(CATALOGS[lang]) ^ keys)[:5]}")


@test("i18n", "Settings uses one word for Settings in each language")
def _(env):
    for lang in LANGS:
        word = CATALOGS[lang]["settings.title"]
        for key in ("home.settings", "ui.settings"):
            expect(CATALOGS[lang][key] == word, f"{lang}: {key} = {CATALOGS[lang][key]!r}, title = {word!r}")
        expect(word in CATALOGS[lang]["tray.open_settings"] or word.lower() in CATALOGS[lang]["tray.open_settings"].lower(),
               f"{lang}: tray says {CATALOGS[lang]['tray.open_settings']!r}")


@test("i18n", "the welcome's key caps exist on every keyboard (no translated key names)")
def _(env):
    for lang in LANGS:
        for step in ("pick", "move", "controls", "together"):
            for cap in kbd_caps(CATALOGS[lang][f"home.welcome.{step}.body"]):
                expect(re.fullmatch(r"(Ctrl\+)?(Alt|F1|[0-9])", cap), f"{lang} {step}: [{cap}]")


@each("i18n", "{0}: section names fit the rail on one or two lines", LANGS)
def _(env, lang):
    page = env.settings(lang=lang)
    tall = page.evaluate("""() => [...document.querySelectorAll('.tab span')].filter((s) => s.getBoundingClientRect().height > 40)
        .map((s) => s.textContent)""")
    expect(not tall, tall)


# =============================================================================
# More tests, one module per kind, in tests/cases/ (they import from here)
# =============================================================================

sys.modules.setdefault("suite", sys.modules[__name__])
import cases  # noqa: E402,F401


# =============================================================================
# Running
# =============================================================================


def main():
    parser = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    parser.add_argument("-k", help="only tests whose name contains this")
    parser.add_argument("--category", help="only this category")
    parser.add_argument("--report", help="folder for report.md and report.json")
    parser.add_argument("--list", action="store_true", help="list the tests and exit")
    args = parser.parse_args()

    tests = [t for t in REGISTRY
             if (not args.k or args.k.lower() in t["name"].lower()) and (not args.category or t["category"] == args.category)]
    if args.list:
        for t in tests:
            print(f"{t['category']:12} {t['name']}")
        print(f"{len(tests)} tests")
        return 0

    results = []
    started = time.perf_counter()
    with sync_playwright() as p:
        browser = p.chromium.launch()
        env = Env(browser)
        for i, t in enumerate(tests, 1):
            t0 = time.perf_counter()
            status, detail = "pass", ""
            try:
                t["fn"](env)
            except Fail as failure:
                status, detail = "fail", str(failure)
            except Exception as error:  # noqa: BLE001 - a broken test is reported, not fatal
                status, detail = "error", f"{type(error).__name__}: {error}".splitlines()[0][:300]
                if "-v" in sys.argv:
                    traceback.print_exc()
            finally:
                env.close_all()
            ms = (time.perf_counter() - t0) * 1000
            results.append(dict(category=t["category"], name=t["name"], status=status, detail=detail, ms=round(ms)))
            mark = {"pass": "  ok  ", "fail": "  FAIL", "error": "  ERR "}[status]
            if status != "pass" or "-v" in sys.argv:
                print(f"{mark} [{t['category']}] {t['name']}" + (f"\n         {detail}" if detail else ""))
            elif i % 50 == 0:
                print(f"  … {i}/{len(tests)}")
        browser.close()

    elapsed = time.perf_counter() - started
    by_category = {}
    for r in results:
        c = by_category.setdefault(r["category"], {"pass": 0, "fail": 0, "error": 0})
        c[r["status"]] += 1
    print()
    print(f"{'category':12} {'pass':>5} {'fail':>5} {'error':>5}")
    for category, c in by_category.items():
        print(f"{category:12} {c['pass']:5} {c['fail']:5} {c['error']:5}")
    passed = sum(1 for r in results if r["status"] == "pass")
    print(f"\n{passed}/{len(results)} passed in {elapsed:.0f} s")

    if args.report:
        folder = pathlib.Path(args.report)
        folder.mkdir(parents=True, exist_ok=True)
        (folder / "report.json").write_text(json.dumps({"elapsed_s": round(elapsed, 1), "results": results}, indent=1,
                                                       ensure_ascii=False), encoding="utf-8")
        lines = ["# YT Mini Player: experience test suite", "",
                 f"{passed} of {len(results)} tests passed in {elapsed:.0f} s.", "",
                 "| Category | Passed | Failed | Errors |", "|---|---:|---:|---:|"]
        for category, c in by_category.items():
            lines.append(f"| {category} | {c['pass']} | {c['fail']} | {c['error']} |")
        failed = [r for r in results if r["status"] != "pass"]
        if failed:
            lines += ["", "## Not passing", ""]
            lines += [f"- **[{r['category']}] {r['name']}**: {r['detail']}" for r in failed]
        lines += ["", "## Every test", ""]
        current = None
        for r in results:
            if r["category"] != current:
                current = r["category"]
                lines += ["", f"### {current}", ""]
            lines.append(f"- {'✅' if r['status'] == 'pass' else '❌'} {r['name']} ({r['ms']} ms)")
        (folder / "report.md").write_text("\n".join(lines) + "\n", encoding="utf-8")
        print(f"report: {folder / 'report.md'}")
    return 0 if passed == len(results) else 1


if __name__ == "__main__":
    sys.exit(main())
