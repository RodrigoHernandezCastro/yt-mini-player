"""Script wrapper identical to the app's, for the tests in Chromium.

Reads the `bundle()` template and `POST_EXPR` straight from src/webview.rs,
so the tests use exactly what the app injects. The test "host" does what the
app does: it accepts only messages carrying the session token (removing it)
and sets the rest aside as forged.

It also serves the pages the way home.rs does: the strings of the page's
language embedded in the data block, and `__ytmpLocale` for the side bar.
"""

import json
import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "src"
TOKEN = "e2e-token-0123456789abcdef"
HOST_KEY = "e2e-host-fedcba9876543210"

# What wry does (window.ipc → chrome.webview) and what the app does with
# each message (policy::authenticate).
IPC_STUB = """
window.__raw = [];
window.__sent = [];
window.__forged = [];
window.chrome = window.chrome || {};
window.chrome.webview = { postMessage: (m) => {
  m = String(m);
  window.__raw.push(m);
  const prefix = '%s|';
  if (m.startsWith(prefix)) {
    const body = m.slice(prefix.length);
    window.__sent.push(body);
    if (window.__onIpc) window.__onIpc(body);
  } else {
    window.__forged.push(m);
  }
}};
Object.defineProperty(window, 'ipc', { value: Object.freeze({ postMessage: (s) => window.chrome.webview.postMessage(s) }) });
""" % TOKEN


def bundle(token=TOKEN, host_key=HOST_KEY):
    src = (SRC / "webview.rs").read_text(encoding="utf-8")
    template = re.search(r'pub fn bundle\(.*?format!\(\s*r#"(.*?)"#', src, re.S).group(1)
    post = re.search(r'const POST_EXPR: &str = "(.*?)";\n', src).group(1).replace('\\"', '"')
    js = template.replace("{{", "{").replace("}}", "}")
    js = js.replace("{post_expr}", post).replace("{ipc}", token).replace("{host}", host_key)
    for placeholder, name in (("{MEDIA_JS}", "media.js"), ("{INJECT_JS}", "inject.js"), ("{UI_JS}", "ui.js")):
        js = js.replace(placeholder, (SRC / name).read_text(encoding="utf-8"))
    return js


def add_app_scripts(page, maintenance=True):
    """Same order as webview.rs: wry stub, wrapper, upkeep."""
    page.add_init_script(IPC_STUB)
    page.add_init_script(bundle())
    if maintenance:
        page.add_init_script(path=str(SRC / "maintenance.js"))


# ---------------------------------------------------------------------------
# Languages, as home.rs and i18n.rs serve them
# ---------------------------------------------------------------------------

def _languages():
    src = (SRC / "i18n.rs").read_text(encoding="utf-8")
    def arms(function):
        body = re.search(r"fn %s\(self\).*?\{(.*?)\n    \}" % function, src, re.S).group(1)
        return dict(re.findall(r'Lang::(\w+) => "([^"]*)",', body))
    codes, names = arms("code"), arms("native_name")
    order = re.search(r"pub const ALL: \[Lang; \d+\] = \[(.*?)\];", src).group(1)
    return [{"code": codes[v], "name": names[v]} for v in re.findall(r"Lang::(\w+)", order)]


def _prefixes():
    src = (SRC / "home.rs").read_text(encoding="utf-8")
    found = {}
    for name, body in re.findall(r"const (\w+)_STRINGS: &\[&str\] = &\[(.*?)\];", src):
        found[name.lower()] = re.findall(r'"([^"]+)"', body)
    return found


LANGUAGES = _languages()
PREFIXES = _prefixes()  # home, music, mini, lock, ui


def catalog(lang):
    return json.loads((SRC / "locales" / f"{lang}.json").read_text(encoding="utf-8"))


def strings(lang, page):
    prefixes = PREFIXES[page]
    return {k: v for k, v in catalog(lang).items() if any(k.startswith(p) for p in prefixes)}


def i18n(lang, page):
    """The `i18n` block of a page's data."""
    return {"lang": lang, "languages": LANGUAGES, "strings": strings(lang, page)}


def fill(name, data, lang="en"):
    """Like home::fill: the language in <html lang> and the data, safe for <script>."""
    template = (SRC / name).read_text(encoding="utf-8")
    payload = json.dumps(data).replace("<", "\\u003c").replace(">", "\\u003e").replace("&", "\\u0026")
    return template.replace("__YTMP_LANG__", lang, 1).replace("__YTMP_DATA__", payload, 1)


def push_locale(page, lang, page_kind=None):
    """What App::push_locale sends: the side bar's strings and, on the app's
    own pages, the page's."""
    locale = {
        "lang": lang,
        "languages": LANGUAGES,
        "ui": strings(lang, "ui"),
        "page": strings(lang, page_kind) if page_kind else None,
    }
    page.evaluate(f"window.__ytmpLocale && window.__ytmpLocale({json.dumps(HOST_KEY)}, {json.dumps(locale)})")


_PLURALS = {
    "en": lambda n: "one" if n == 1 else "other",
    "es": lambda n: "one" if n == 1 else "other",
    "ja": lambda n: "other",
    "ru": lambda n: "one" if n % 10 == 1 and n % 100 != 11
    else "few" if 2 <= n % 10 <= 4 and not 12 <= n % 100 <= 14 else "many",
    "pl": lambda n: "one" if n == 1
    else "few" if 2 <= n % 10 <= 4 and not 12 <= n % 100 <= 14 else "many",
}


def text(lang, key, /, **args):
    """A catalog text with its arguments, for comparing with what a page shows."""
    value = catalog(lang)[key]
    if isinstance(value, dict):
        value = value.get(_PLURALS[lang](int(args.get("count", 0))), value.get("other"))
    return re.sub(r"\{(\w+)\}", lambda m: str(args.get(m.group(1), m.group(0))), value)
