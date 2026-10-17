"""Prueba de extremo a extremo de la pantalla de inicio y de otros servicios.

- Sirve src/home.html en https://ytmp.localhost/ con la misma política de
  seguridad (CSP) que usa la app, e inyecta inject.js y ui.js.
- Sirve una página que imita un reproductor de otro servicio (video grande
  con controles propios) en https://www.netflix.com/watch/1.

Uso:  python tests/home_e2e.py [carpeta_para_capturas]
"""

import json
import pathlib
import sys

from playwright.sync_api import sync_playwright

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from bundle import HOST_KEY, add_app_scripts  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
SHOTS = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else None
CSP = ("default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; "
       "img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'")

SERVICES = [
    ("youtube", "YouTube", "Videos y directos", "#ff4e45", False),
    ("youtube-music", "YouTube Music", "Música y podcasts", "#ff6f61", False),
    ("netflix", "Netflix", "Series y películas", "#e50914", True),
    ("crunchyroll", "Crunchyroll", "Anime", "#f47521", True),
    ("prime-video", "Prime Video", "Series y películas", "#1f9bde", True),
    ("disney-plus", "Disney+", "Series y películas", "#2fb5c4", True),
    ("hbo-max", "HBO Max", "Series y películas", "#8a5cf6", True),
    ("twitch", "Twitch", "Directos", "#a970ff", False),
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
    <div class="controls"><button aria-label="Reproducir">▶</button><div class="seek" role="slider" aria-valuenow="10"></div><button aria-label="Pantalla completa">⛶</button></div>
  </div>
  <script>
    window.clicks = 0;
    document.querySelector('video').addEventListener('click', () => window.clicks++);
  </script>
</body></html>"""

IPC_STUB = """
window.__sent = [];
Object.defineProperty(window, 'ipc', { value: Object.freeze({ postMessage: (m) => window.__sent.push(m) }) });
"""


def home_html(last_service=None, start_with_last=False):
    data = {
        "version": "0.3.0",
        "services": [{"id": i, "name": n, "caption": c, "accent": a, "drm": d} for i, n, c, a, d in SERVICES],
        "lastService": last_service,
        "startWithLast": start_with_last,
    }
    template = (ROOT / "src" / "home.html").read_text(encoding="utf-8")
    payload = json.dumps(data).replace("<", "\\u003c").replace(">", "\\u003e").replace("&", "\\u0026")
    return template.replace("__YTMP_DATA__", payload, 1)


def main():
    failures = []

    def check(cond, name):
        print(("  ok   " if cond else "  FAIL ") + name)
        if not cond:
            failures.append(name)

    with sync_playwright() as p:
        browser = p.chromium.launch()

        def new_page(width=640, height=360, drm_stub=None, last=None):
            errors = []
            page = browser.new_page(viewport={"width": width, "height": height})
            page.on("console", lambda m: m.type == "error" and errors.append(m.text))
            page.on("pageerror", lambda e: errors.append(str(e)))
            page.route("https://ytmp.localhost/**", lambda r: r.fulfill(
                status=200, content_type="text/html; charset=utf-8",
                headers={"Content-Security-Policy": CSP}, body=home_html(last_service=last)))
            page.route("https://www.netflix.com/**", lambda r: r.fulfill(
                status=200, content_type="text/html; charset=utf-8", body=STREAMING_HTML))
            if drm_stub:
                page.add_init_script(drm_stub)
            add_app_scripts(page, maintenance=False)
            return page, errors

        sent = lambda page: page.evaluate("window.__sent.slice()")
        clear = lambda page: page.evaluate("window.__sent.length = 0")

        def shot(page, name):
            if SHOTS:
                SHOTS.mkdir(parents=True, exist_ok=True)
                page.screenshot(path=str(SHOTS / f"{name}.png"))

        # ------------------------------------------------------------------
        print("Pantalla de inicio")
        page, errors = new_page(last="crunchyroll")
        page.goto("https://ytmp.localhost/")
        page.wait_for_timeout(400)
        rows = page.locator(".channel")
        check(rows.count() == 8, f"muestra los 8 servicios ({rows.count()})")
        check(rows.nth(2).inner_text().startswith("3"), "cada fila lleva su número de tecla")
        check(page.locator(".channel[data-id='crunchyroll'] .last").count() == 1, "marca el último servicio usado")
        shot(page, "1-inicio")

        clear(page)
        rows.nth(2).click()
        check(sent(page) == ["service:netflix"], f"clic en Netflix envía 'service:netflix' ({sent(page)})")
        check("Abriendo" in rows.nth(2).inner_text(), "la fila indica que está abriendo")

        clear(page)
        page.keyboard.press("4")
        check(sent(page) == ["service:crunchyroll"], f"la tecla 4 abre Crunchyroll ({sent(page)})")

        clear(page)
        page.click("#site-url")
        page.keyboard.type("plex.tv/web5")
        check(sent(page) == [], "escribir una dirección no dispara las teclas 1–8")
        page.keyboard.press("Enter")
        check(sent(page) == ["open-url:plex.tv/web5"], f"Enter abre la dirección ({sent(page)})")

        clear(page)
        page.click("#startup")
        check(sent(page) == ["start-with-last:1"], f"el interruptor envía start-with-last:1 ({sent(page)})")
        check(page.get_attribute("#startup", "aria-checked") == "true", "y queda marcado")

        clear(page)
        box = page.locator("h1").bounding_box()
        page.mouse.move(box["x"] + 10, box["y"] + 8)
        page.mouse.down()
        page.mouse.up()
        check(sent(page) == ["drag"], f"la cabecera mueve la ventana ({sent(page)})")

        page.wait_for_timeout(200)
        state = page.locator("#drm").get_attribute("data-state")
        check(state == "missing", f"sin Widevine en este Chromium, el estado DRM es 'missing' ({state})")
        warn = page.locator(".channel[data-id='netflix'] .caption.warn").count()
        check(warn == 1, "Netflix avisa de que necesita DRM")
        check(page.locator(".channel[data-id='youtube'] .caption.warn").count() == 0, "YouTube no avisa (no lo necesita)")
        check(page.locator("#recheck").is_visible(), "ofrece comprobar de nuevo")
        shot(page, "2-inicio-sin-drm")

        page.evaluate(f"window.__ytmpSync({json.dumps(HOST_KEY)}, {{ startWithLast: false, lastService: 'netflix', tray: true }})")
        page.wait_for_timeout(100)
        check(page.get_attribute("#startup", "aria-checked") == "false", "refleja el estado que envía la app")
        check(page.locator(".channel[data-id='netflix'] .last").count() == 1, "y el nuevo último servicio")

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
        check(bar_buttons > 0, "la barra de la app también está en el menú")
        shot(page, "3-inicio-barra")
        check(not errors, f"sin errores de consola ni de CSP ({errors[:3]})")
        page.close()

        print("Con DRM disponible")
        stub = """
        navigator.requestMediaKeySystemAccess = (ks) => ks === 'com.widevine.alpha'
          ? Promise.resolve({}) : Promise.reject(new Error('no'));
        """
        page, errors = new_page(drm_stub=stub)
        page.goto("https://ytmp.localhost/")
        page.wait_for_timeout(300)
        check(page.locator("#drm").get_attribute("data-state") == "ok", "el estado DRM es 'ok'")
        check("Widevine" in page.locator("#drm .text").inner_text(), "nombra el sistema disponible")
        check(page.locator(".caption.warn").count() == 0, "sin avisos en los servicios")
        page.close()

        print("Ventana pequeña")
        for w, h in [(360, 220), (256, 144)]:
            page, errors = new_page(width=w, height=h)
            page.goto("https://ytmp.localhost/")
            page.wait_for_timeout(300)
            overflow = page.evaluate("document.documentElement.scrollWidth > innerWidth")
            check(not overflow, f"{w}×{h}: sin desplazamiento horizontal")
            shot(page, f"4-inicio-{w}x{h}")
            page.close()

        # ------------------------------------------------------------------
        print("Otro servicio (reproductor genérico)")
        page, errors = new_page()
        page.goto("https://www.netflix.com/watch/1")
        page.wait_for_timeout(300)

        clear(page)
        page.mouse.move(300, 150)
        page.mouse.down()
        page.mouse.move(320, 165, steps=4)
        page.mouse.up()
        check(sent(page) == ["drag"], f"arrastrar el video mueve la ventana ({sent(page)})")
        check(page.evaluate("window.clicks") == 0, "y no le llega un clic al video")

        clear(page)
        page.mouse.click(300, 150)
        check(sent(page) == [], "un clic sin mover no arrastra")
        check(page.evaluate("window.clicks") == 1, "y llega al video (pausa/reproduce)")

        clear(page)
        page.mouse.move(300, 336)  # barra de progreso (role=slider)
        page.mouse.down()
        page.mouse.move(340, 336, steps=4)
        page.mouse.up()
        check(sent(page) == [], "arrastrar la barra de progreso no mueve la ventana")

        page.mouse.move(630, 180)
        page.wait_for_timeout(300)
        shot(page, "5-servicio-barra")
        clear(page)
        # El botón de menú principal es el segundo de la barra (tras el asa).
        ys = [y for y in range(0, 360, 2) if page.evaluate(f"document.elementFromPoint(611, {y})?.tagName") == "YTMP-UI"]
        page.mouse.click(611, ys[0] + 5 + 16 + 34)
        check(sent(page) == ["home"], f"el botón de menú principal envía 'home' ({sent(page)})")

        clear(page)
        page.mouse.move(300, 150)
        page.keyboard.press("Alt+Home")
        check(sent(page) == ["home"], f"Alt+Inicio vuelve al menú ({sent(page)})")

        clear(page)
        page.keyboard.press("Control+m")
        check(sent(page) == [], "Ctrl+M (solo video) no actúa fuera de YouTube")
        check(not errors, f"sin errores de consola ({errors[:3]})")
        browser.close()

    print()
    if failures:
        print(f"{len(failures)} prueba(s) fallaron")
        sys.exit(1)
    print("home.html + servicios: todas las pruebas pasan")


if __name__ == "__main__":
    main()
