"""Envoltorio de scripts idéntico al de la app, para las pruebas en Chromium.

Lee la plantilla de `bundle()` y `POST_EXPR` directamente de src/webview.rs,
así que las pruebas usan exactamente lo que inyecta la app. El "host" de
prueba hace lo mismo que la app: acepta solo los mensajes con el token de
sesión (quitándolo) y aparta el resto como falsificados.
"""

import pathlib
import re

ROOT = pathlib.Path(__file__).resolve().parent.parent
TOKEN = "e2e-token-0123456789abcdef"
HOST_KEY = "e2e-host-fedcba9876543210"

# Lo que hace wry (window.ipc → chrome.webview) y lo que hace la app con
# cada mensaje (policy::authenticate).
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
    src = (ROOT / "src" / "webview.rs").read_text(encoding="utf-8")
    template = re.search(r'pub fn bundle\(.*?format!\(\s*r#"(.*?)"#', src, re.S).group(1)
    post = re.search(r'const POST_EXPR: &str = "(.*?)";\n', src).group(1).replace('\\"', '"')
    js = template.replace("{{", "{").replace("}}", "}")
    js = js.replace("{post_expr}", post).replace("{ipc}", token).replace("{host}", host_key)
    for placeholder, name in (("{MEDIA_JS}", "media.js"), ("{INJECT_JS}", "inject.js"), ("{UI_JS}", "ui.js")):
        js = js.replace(placeholder, (ROOT / "src" / name).read_text(encoding="utf-8"))
    return js


def add_app_scripts(page, maintenance=True):
    """Mismo orden que webview.rs: stub de wry, envoltorio, mantenimiento."""
    page.add_init_script(IPC_STUB)
    page.add_init_script(bundle())
    if maintenance:
        page.add_init_script(path=str(ROOT / "src" / "maintenance.js"))
