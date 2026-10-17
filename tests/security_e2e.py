"""Pruebas de seguridad de lo que corre dentro de las páginas (Chromium).

1. Una página hostil (scripts propios que intentan usar la app) con el mismo
   envoltorio que inyecta la app (tests/bundle.py lee webview.rs).
2. La pantalla de bloqueo (src/lock.html) con la CSP real.

Uso:  python tests/security_e2e.py [carpeta_para_capturas]
"""

import json
import pathlib
import sys

from playwright.sync_api import sync_playwright

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from bundle import HOST_KEY, TOKEN, add_app_scripts  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
SHOTS = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else None
CSP = ("default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; "
       "img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'")

# Una página que intenta, con sus propios scripts, dar órdenes a la app.
HOSTILE = """<!doctype html><html><head><meta charset="utf-8"><title>Sitio hostil</title></head><body>
<video id="v" style="width:600px;height:300px;background:#222"></video>
<script>
  window.stolen = [];
  window.stateLeak = [];
  window.addEventListener('ytmp:state', (e) => window.stateLeak.push(e.detail));
  // Intenta interceptar el canal de WebView2 para robar el token.
  const original = window.chrome.webview.postMessage;
  window.chrome.webview.postMessage = (m) => { window.stolen.push(String(m)); original(m); };
  // Y escribir directamente a la app.
  window.ipc.postMessage('autostart');
  window.ipc.postMessage('ghost');
  window.ipc.postMessage('ext-toggle:abcdefghijklmnopabcdefghijklmnop');
  window.ipc.postMessage('quit');
</script>
</body></html>"""


def main():
    failures = []

    def check(cond, name):
        print(("  ok   " if cond else "  FAIL ") + name)
        if not cond:
            failures.append(name)

    with sync_playwright() as p:
        browser = p.chromium.launch()

        # ------------------------------------------------------------------
        print("Página hostil")
        errors = []
        page = browser.new_page(viewport={"width": 640, "height": 360})
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.route("https://evil.example/**", lambda r: r.fulfill(status=200, content_type="text/html; charset=utf-8", body=HOSTILE))
        add_app_scripts(page)
        page.goto("https://evil.example/")
        page.wait_for_timeout(400)

        sent = lambda: page.evaluate("window.__sent.slice()")
        forged = lambda: page.evaluate("window.__forged.slice()")
        clear = lambda: page.evaluate("window.__sent.length = 0; window.__forged.length = 0")

        check(not [m for m in sent() if m in ("autostart", "ghost", "quit") or m.startswith("ext-")],
              f"la app no acepta órdenes escritas por la página ({sent()})")
        check({"autostart", "ghost", "quit"} <= set(forged()), f"llegan sin token y se descartan ({forged()})")

        # Nuestros scripts siguen funcionando con el canal original…
        clear()
        page.mouse.move(300, 150)
        page.keyboard.press("Control+t")
        check(sent() == ["pin"], f"un atajo real sí llega con token ({sent()})")
        # …y el reemplazo de la página no ve el token.
        stolen = page.evaluate("window.stolen")
        check(not any(TOKEN in m for m in stolen), f"la página no intercepta el token ({stolen[:3]})")

        # Eventos sintéticos: no activan nuestros atajos ni el arrastre.
        clear()
        page.evaluate("""() => {
            for (const key of ['q', 't', 'h', 'l']) {
              window.dispatchEvent(new KeyboardEvent('keydown', { key, code: 'Key' + key.toUpperCase(), ctrlKey: true, bubbles: true }));
            }
            const v = document.getElementById('v');
            v.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerType: 'mouse', button: 0, altKey: true, clientX: 300, clientY: 150 }));
        }""")
        check(sent() == [], f"teclas y clics sintéticos de la página no hacen nada ({sent()})")

        # Funciones que solo la app puede llamar: un aviso falso de la página
        # no aparece (la captura no cambia); uno de la app, sí.
        check(page.evaluate("document.querySelector('ytmp-ui') && 1") == 1, "la interfaz de la app está montada")
        page.wait_for_timeout(300)
        before = page.screenshot()
        page.evaluate("window.__ytmpToast('Tu sesión expiró: escribe tu contraseña aquí')")
        page.evaluate("window.__ytmpSync({ autostart: true, extensions: [] })")
        page.wait_for_timeout(500)
        check(page.screenshot() == before, "un aviso falso de la página no se muestra")
        page.evaluate(f"window.__ytmpToast({json.dumps(HOST_KEY)}, 'Aviso real de la app')")
        page.wait_for_timeout(500)
        check(page.screenshot() != before, "un aviso de la app sí se muestra")
        leak = page.evaluate("window.stateLeak.length")
        check(leak == 0, f"los ajustes de la app no se publican a otros sitios ({leak})")
        page.evaluate(f"window.__ytmpSync({json.dumps(HOST_KEY)}, {{ autostart: true }})")
        page.wait_for_timeout(100)
        check(page.evaluate("window.stateLeak.length") == 0, "ni siquiera cuando la app sincroniza")

        # El token no está a la vista en ningún sitio que la página pueda leer.
        found = page.evaluate(f"""() => {{
            const t = {json.dumps(TOKEN)};
            const hits = [];
            if (document.documentElement.outerHTML.includes(t)) hits.push('DOM');
            for (const k of Object.getOwnPropertyNames(window)) {{
              try {{
                const v = window[k];
                if (typeof v === 'string' && v.includes(t)) hits.push(k);
                if (typeof v === 'function' && Function.prototype.toString.call(v).includes(t)) hits.push(k + '()');
              }} catch (_) {{}}
            }}
            return hits;
        }}""")
        check(found == [], f"el token no es accesible desde la página ({found})")
        check(page.evaluate("typeof window.__ytmpHomeSend") == "undefined", "el canal de la pantalla de inicio no existe fuera de ella")
        check(page.evaluate("typeof window.__ytmpStartDrag") == "undefined", "arrastrar la ventana no está expuesto")
        check(not errors, f"sin errores en la página ({errors[:3]})")
        page.close()

        # ------------------------------------------------------------------
        print("Pantalla de bloqueo")
        errors = []
        page = browser.new_page(viewport={"width": 640, "height": 360})
        page.on("console", lambda m: m.type == "error" and errors.append(m.text))
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.route("https://ytmp.localhost/**", lambda r: r.fulfill(
            status=200, content_type="text/html; charset=utf-8", headers={"Content-Security-Policy": CSP},
            body=(ROOT / "src" / "lock.html").read_text(encoding="utf-8")))
        page.add_init_script("window.__sent = []; Object.defineProperty(window, 'ipc', { value: Object.freeze({ postMessage: (m) => window.__sent.push(m) }) });")
        page.goto("https://ytmp.localhost/lock")
        page.wait_for_timeout(200)
        sent = lambda: page.evaluate("window.__sent.slice()")
        clear = lambda: page.evaluate("window.__sent.length = 0")
        push = lambda **s: (page.evaluate(f"window.__ytmpLock({json.dumps(s)})"), page.wait_for_timeout(80))

        def shot(name):
            if SHOTS:
                SHOTS.mkdir(parents=True, exist_ok=True)
                page.screenshot(path=str(SHOTS / f"{name}.png"))

        check(sent()[:1] == ["lock:ready"], f"avisa que está lista ({sent()})")
        push(stage="unlock", method="hello", busy=False, error=None, wait=0, canCancel=False)
        check(page.is_visible("#hello") and page.is_hidden("#pin-form"), "con Windows Hello: botón, sin campo de PIN")
        check(page.is_hidden("#cancel-box"), "desbloquear no se puede cancelar")
        clear()
        page.click("#hello")
        check(sent() == ["lock:hello"], f"el botón pide Windows Hello ({sent()})")
        push(busy=True)
        check(page.is_disabled("#hello") and "Windows Hello" in page.inner_text("#status"), "mientras espera, lo dice")
        shot("1-hello")

        push(method="pin", busy=False, error=None)
        check(page.is_visible("#pin") and page.is_hidden("#pin2"), "con PIN: un campo")
        check(page.evaluate("document.activeElement.id") == "pin", "el foco va al PIN")
        clear()
        page.keyboard.type("482915")
        page.keyboard.press("Enter")
        check(sent() == ["lock:pin:482915"], f"Enter envía el PIN ({sent()})")
        check(page.input_value("#pin") == "", "el PIN no se queda en la página")
        push(error="PIN incorrecto")
        check(page.inner_text("#status") == "PIN incorrecto" and page.evaluate("document.getElementById('status').classList.contains('error')"),
              "muestra el error")
        check(page.is_visible("#forgot"), "con PIN se ofrece «¿Olvidaste el PIN?»")
        clear()
        page.click("#forgot")
        check(sent() == ["lock:forgot"], f"y lo pide a la app ({sent()})")
        push(error=None, wait=3)
        check(page.is_disabled("#submit") and page.is_disabled("#pin") and "Espera 3 s" in page.inner_text("#status"),
              "tras muchos fallos, espera obligatoria")
        clear()
        page.evaluate("document.getElementById('pin-form').requestSubmit()")
        check(sent() == [], "durante la espera no se envía nada")
        page.wait_for_timeout(3300)
        check(not page.is_disabled("#submit"), "al terminar la espera se puede volver a intentar")
        shot("2-pin")

        push(stage="new-pin", method="pin", wait=0, error=None, canCancel=True)
        check(page.is_visible("#pin2") and page.is_visible("#hint") and page.is_visible("#cancel"), "crear PIN: dos campos, requisitos y cancelar")
        check(page.is_hidden("#forgot"), "al crear un PIN no aparece «olvidé»")
        clear()
        page.fill("#pin", "482915")
        page.fill("#pin2", "482916")
        page.click("#submit")
        check(sent() == [] and "no coinciden" in page.inner_text("#status"), "si no coinciden, no se envía")
        page.fill("#pin", "482915")
        page.fill("#pin2", "482915")
        page.click("#submit")
        check(sent() == ["lock:new-pin:482915"], f"envía el PIN nuevo ({sent()})")
        clear()
        page.keyboard.press("Escape")
        check(sent() == ["lock:cancel"], f"Escape cancela un cambio ({sent()})")
        shot("3-nuevo-pin")

        push(stage="verify", method="hello", canCancel=True)
        check(page.inner_text("#title") == "Confirma que eres tú" and page.is_visible("#cancel-box"), "verificar para cambiar: título y cancelar")

        for w, h in [(360, 220), (256, 144)]:
            page.set_viewport_size({"width": w, "height": h})
            push(stage="unlock", method="pin", canCancel=False)
            page.wait_for_timeout(100)
            fits = page.evaluate("""() => { const r = document.querySelector('main').getBoundingClientRect();
                return r.top >= 0 && r.bottom <= innerHeight && r.left >= 0 && r.right <= innerWidth; }""")
            check(fits, f"{w}×{h}: cabe entera")
            shot(f"4-{w}x{h}")
        check(not errors, f"sin errores de consola ni de CSP ({errors[:3]})")
        browser.close()

    print()
    if failures:
        print(f"{len(failures)} prueba(s) fallaron")
        sys.exit(1)
    print("seguridad (página hostil + pantalla de bloqueo): todas las pruebas pasan")


if __name__ == "__main__":
    main()
