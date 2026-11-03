"""SECURITY: hostile text everywhere the pages show outside data.

Each payload tries to become markup (an element with class "pwn") or run
code (window.__pwned). Every place must show it as plain text.
"""
import json

import re

from bundle import HOST_KEY
from cases.common import open_menu
from cases.together_more import base_views, push_view, together_page
from home_e2e import home_html
from suite import (
    CSP, IPC_STUB, LANGS, SIDEBAR_HOST, SRC, ax_names, clear, each, expect, home_sent, sent, settings_state, shadow,
    test, text,
)

PAYLOADS = [
    '<img src=x onerror="window.__pwned=1" class="pwn">',
    '<script class="pwn">window.__pwned=1</script>',
    '<svg class="pwn" onload="window.__pwned=1"></svg>',
    '"><b class="pwn">bold</b>',
    "'><iframe class=\"pwn\" src=\"javascript:window.__pwned=1\"></iframe>",
    '<a class="pwn" href="javascript:window.__pwned=1">link</a>',
    '{version}{label}{reason}{count} <u class="pwn">u</u>',
    '&lt;b class=&quot;pwn&quot;&gt;entity&lt;/b&gt;',
    '<style class="pwn">*{display:none}</style>',
    '‮<i class="pwn">rtl</i>',
    '<math class="pwn"><mi>x</mi></math>',
    '</script><script class="pwn">window.__pwned=1</script>',
]
IDS = list(range(1, len(PAYLOADS) + 1))


def clean(page):
    expect(page.evaluate("document.querySelectorAll('.pwn').length") == 0, "the text became markup")
    expect(not page.evaluate("window.__pwned"), "the text ran code")


def ext(**kw):
    return dict(settings_state()["extensions"][0], **kw)


# Where Settings shows outside data: (name, section, state, where the text is).
FIELDS = {
    "extension name": ("extensions", lambda p: settings_state(extensions=[ext(name=p)]), "text"),
    "extension version": ("extensions", lambda p: settings_state(extensions=[ext(version=p)]), "text"),
    "extension error": ("extensions", lambda p: settings_state(extensions=[ext(error=p)]), "text"),
    "extension problem": ("extensions", lambda p: settings_state(extensionProblems=[p]), "text"),
    "extensions folder error": ("extensions", lambda p: settings_state(extensionsError=p), "text"),
    "shortcut label": ("advanced", lambda p: settings_state(hotkeys=[{"keys": "Ctrl+Alt+X", "label": p, "available": True, "note": None}]), "text"),
    "shortcut keys": ("advanced", lambda p: settings_state(hotkeys=[{"keys": p, "label": "L", "available": True, "note": None}]), "text"),
    "shortcut note": ("advanced", lambda p: settings_state(hotkeys=[{"keys": "Ctrl+Alt+X", "label": "L", "available": False, "note": p}]), "text"),
    "app version": ("advanced", lambda p: settings_state(version=p), "text"),
    "engine version": ("advanced", lambda p: settings_state(webview=p), "text"),
    "together name": ("together", lambda p: settings_state(togetherName=p), "input"),
}


@each("security", "settings: payload {1} in the {0} stays text",
      [(field, i) for field in FIELDS for i in IDS])
def _(env, field, number):
    payload = PAYLOADS[number - 1]
    section, state, where = FIELDS[field]
    page = env.settings(section=section, state=state(payload))
    clean(page)
    if where == "input":
        expect(page.input_value("[data-setting='together-name']") == payload, "the value changed")
    else:
        expect("pwn" in page.evaluate("document.body.textContent"), "the text is not shown")
    expect(not page.errors, page.errors[:2])


@each("security", "side bar: payload {0} in a notice stays text", IDS)
def _(env, number):
    payload = PAYLOADS[number - 1]
    page = env.sidebar()
    page.evaluate(f"window.__ytmpToast({json.dumps(HOST_KEY)}, {json.dumps(payload)})")
    page.wait_for_timeout(50)
    expect(shadow(page, ".pwn") is None, "the text became markup")
    expect(not page.evaluate("window.__pwned"), "the text ran code")
    expect("pwn" in " ".join(ax_names(page)), "not shown")


@each("security", "side bar: payload {0} as an extensions error stays text in the menu", IDS)
def _(env, number):
    payload = PAYLOADS[number - 1]
    page = env.sidebar(host=dict(SIDEBAR_HOST, extensionsError=payload))
    open_menu(page, "en")
    expect(shadow(page, ".pwn") is None, "the text became markup")
    expect(not page.evaluate("window.__pwned"), "the text ran code")


@each("security", "side bar: payload {0} as a global shortcut stays text in the help", IDS)
def _(env, number):
    payload = PAYLOADS[number - 1]
    keys = [{"keys": payload, "label": payload, "available": False, "note": payload}]
    page = env.sidebar(host=dict(SIDEBAR_HOST, hotkeys=keys))
    page.evaluate(f"window.__ytmpHelp({json.dumps(HOST_KEY)})")
    page.wait_for_timeout(50)
    expect(shadow(page, ".pwn") is None, "the text became markup")
    expect(not page.evaluate("window.__pwned"), "the text ran code")


@each("security", "side bar: payload {0} as the version stays text in the menu", IDS)
def _(env, number):
    payload = PAYLOADS[number - 1]
    page = env.sidebar(host=dict(SIDEBAR_HOST, version=payload))
    open_menu(page, "en")
    expect(shadow(page, ".pwn") is None, "the text became markup")
    expect(not page.evaluate("window.__pwned"), "the text ran code")


def home_with(name):
    """The start screen with YouTube's name replaced."""
    page_html = home_html()
    data_start = page_html.index('<script id="ytmp-data" type="application/json">') + len('<script id="ytmp-data" type="application/json">')
    data_end = page_html.index("</script>", data_start)
    raw = page_html[data_start:data_end]
    data = json.loads(raw)
    data["services"][0]["name"] = name
    payload = json.dumps(data).replace("<", "\\u003c").replace(">", "\\u003e").replace("&", "\\u0026")
    return page_html[:data_start] + payload + page_html[data_end:]


@each("security", "start screen: payload {0} as a service name stays text", IDS)
def _(env, number):
    payload = PAYLOADS[number - 1]
    page = env.page()
    page.add_init_script("window.__sent = []; window.__ytmpHomeSend = (m) => window.__sent.push(String(m));")
    env.serve(page, home_with(payload))
    page.goto("https://ytmp.localhost/")
    clean(page)
    expect(page.locator(".key").count() == 9, "page broken")
    shown = page.evaluate("document.querySelector('.key[data-id=\"youtube\"] .name span').textContent")
    expect(shown == payload, f"{shown!r}")
    expect(page.get_attribute(".key[data-id='youtube']", "title") == text("en", "home.open_service", name=payload, key=1),
           "title")


@each("security", "start screen: payload {0} as the last service marks nothing", IDS)
def _(env, number):
    page = env.page()
    page.add_init_script(IPC_STUB)
    env.serve(page, home_html(last_service=PAYLOADS[number - 1]))
    page.goto("https://ytmp.localhost/")
    clean(page)
    expect(page.locator(".last").count() == 0 and page.locator(".key").count() == 9, "page broken or marked")


@each("security", "start screen: payload {0} typed as an address is sent as typed, nothing runs", IDS)
def _(env, number):
    payload = PAYLOADS[number - 1]
    page = env.home(sidebar=False)
    page.fill("#site-url", payload)
    page.keyboard.press("Enter")
    clean(page)
    expect(sent(page) == [f"open-url:{payload.strip()}"], sent(page))


@each("security", "together window: payload {0} as a person's name stays text", IDS)
def _(env, number):
    payload = PAYLOADS[number - 1]
    page = together_page(env, "en")
    view = base_views()["host"]
    view["members"] = [dict(view["members"][0])] + [dict(view["members"][1], name=payload)]
    push_view(page, view)
    clean(page)
    expect("pwn" in page.evaluate("document.body.textContent"), "not shown")


@each("security", "together window: payload {0} as the video title stays text", IDS)
def _(env, number):
    payload = PAYLOADS[number - 1]
    page = together_page(env, "en")
    push_view(page, dict(base_views()["guest"], title=payload, hostName=payload))
    clean(page)


# --- Forged calls --------------------------------------------------------------

FORGED = ["window.__ytmpSync('{key}', {{ ghost: true, pinned: false }})",
          "window.__ytmpToast('{key}', 'forged notice')",
          "window.__ytmpHelp('{key}')",
          "window.__ytmpLocale('{key}', {{ lang: 'ja', ui: {{}} }})"]
WRONG_KEYS = ["", "nope", HOST_KEY.upper(), HOST_KEY + " ", " " + HOST_KEY, HOST_KEY[:-1]]


@each("security", "side bar: '{0}' with a wrong key ({1!r}) is ignored",
      [(call.split("(")[0], key) for call in FORGED for key in WRONG_KEYS])
def _(env, function, key):
    page = env.sidebar()
    before = ax_names(page)
    call = next(c for c in FORGED if c.startswith(function))
    page.evaluate(call.format(key=key.replace("'", "\\'")))
    page.wait_for_timeout(50)
    after = ax_names(page)
    expect("forged notice" not in after, "showed a forged notice")
    expect(text("en", "ui.shortcuts") not in after or text("en", "ui.shortcuts") in before, "opened the help")
    expect(any(n == text("en", "ui.with_keys", label=text("en", "ui.settings"), keys="Ctrl+,") for n in after),
           "the bar changed language or broke")


@each("security", "side bar: a page cannot send '{0}' without the session token",
      ["settings", "quit", "pin", "hide", "lang:ja", "open-url:https://evil.example", "service:netflix"])
def _(env, message):
    page = env.sidebar()
    clear(page)
    page.evaluate(f"window.chrome.webview.postMessage({json.dumps(message)}); window.ipc.postMessage({json.dumps(message)})")
    expect(message not in home_sent(page), home_sent(page))
    expect(page.evaluate("window.__forged").count(message) >= 1, "not set aside as forged")


@each("security", "{0}: the app's functions cannot be replaced or deleted", ["__ytmpSync", "__ytmpToast", "__ytmpHelp", "__ytmpLocale"])
def _(env, function):
    page = env.sidebar()
    page.evaluate(f"try {{ window.{function} = 1; }} catch (e) {{}}; try {{ delete window.{function}; }} catch (e) {{}}")
    expect(page.evaluate(f"typeof window.{function}") == "function", "replaced or deleted")


@each("security", "settings in {0}: the page's state function cannot be replaced", LANGS)
def _(env, lang):
    page = env.settings(lang=lang)
    page.evaluate("try { window.__ytmpSettings = () => {}; } catch (e) {}; try { delete window.__ytmpSettings } catch (e) {}")
    expect(page.evaluate("typeof window.__ytmpSettings") == "function", "replaced")


@each("security", "settings: a section name {0!r} from outside opens General", ["../general", "constructor", "__proto__",
                                                                                "toString", "GENERAL", " window", ""])
def _(env, section):
    page = env.settings()
    page.evaluate(f"window.__ytmpSettings({json.dumps(dict(settings_state(), section=section))})")
    expect(page.evaluate("document.querySelector('#page h2').textContent") == text("en", "settings.section.general"),
           "opened something else")
    expect(not page.errors, page.errors[:2])


@each("security", "settings: a state of type {0} is ignored", ["null", "undefined", "42", "'text'", "true"])
def _(env, value):
    page = env.settings(section="window")
    page.evaluate(f"window.__ytmpSettings({value})")
    expect(page.evaluate("document.querySelector('#page h2')?.textContent") == text("en", "settings.section.window"), "page broke")


@each("security", "{0}: no inline event handlers in the markup", ["home.html", "settings.html", "together.html",
                                                                 "lock.html", "mini.html", "stream.html", "cast.html",
                                                                 "music.html"])
def _(env, file):
    src = (SRC / file).read_text(encoding="utf-8")
    markup = re.sub(r"<script\b.*?</script>", "", src, flags=re.S)
    handlers = re.findall(r"<[^>]*\son[a-z]+\s*=", markup)
    expect(not handlers, handlers[:3])


@each("security", "{0}: no javascript: links", ["home.html", "settings.html", "together.html", "lock.html", "mini.html",
                                                "stream.html", "cast.html", "music.html", "ui.js", "inject.js"])
def _(env, file):
    src = (SRC / file).read_text(encoding="utf-8").lower()
    expect("javascript:" not in src, "javascript: URL")


@test("security", "the app's pages allow nothing from the network (no http, https or *)")
def _(env):
    for source in ("http:", "https:", "*", "unsafe-eval", "connect-src", "frame-src"):
        expect(source not in CSP, f"{source} in the CSP")
