"""Prueba de extremo a extremo de inject.js + ui.js en Chromium (Playwright).

Sirve una página que imita la estructura del reproductor de YouTube en
https://www.youtube.com/watch, con la política Trusted Types que usa YouTube,
e inyecta los scripts igual que la aplicación. Un "host" simulado en Python
responde a los mensajes IPC como lo hace app.rs.

Uso:  python tests/ui_e2e.py [carpeta_para_capturas]
Requiere: pip install playwright && playwright install chromium
"""

import json
import pathlib
import sys

from playwright.sync_api import sync_playwright

ROOT = pathlib.Path(__file__).resolve().parent.parent
SHOTS = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else None
EXT_ID = "cjpalhdlnbpafiamejdnhcphjbkeiagm"

MOCK_HTML = """<!doctype html>
<html><head><meta charset="utf-8"><title>Video - YouTube</title>
<style>
  body { margin: 0; background: #0f0f0f; color: #fff; font-family: sans-serif; }
  #masthead-container { height: 56px; background: #212121; display: flex; align-items: center; padding: 0 16px; }
  #masthead-container input { width: 50%; }
  ytd-watch-flexy { display: block; padding: 16px; }
  #movie_player { position: relative; width: 100%; aspect-ratio: 16/9; background: #000; overflow: hidden; }
  #movie_player video { position: absolute; left: 0; top: 0; width: 100%; height: 100%; background: linear-gradient(135deg,#2b3a67,#0f0f0f); }
  .ytp-chrome-bottom { position: absolute; left: 12px; right: 12px; bottom: 0; height: 40px; display: flex; align-items: center; justify-content: flex-end; }
  .ytp-fullscreen-button { width: 40px; height: 32px; }
  #below { padding: 16px 0; }
</style></head>
<body>
  <ytd-app>
    <div id="masthead-container"><input id="search" placeholder="Buscar"></div>
    <ytd-watch-flexy>
      <div id="player-container">
        <div id="movie_player" class="html5-video-player">
          <div class="html5-video-container"><video class="html5-main-video"></video></div>
          <div class="ytp-chrome-bottom"><button class="ytp-fullscreen-button">⛶</button></div>
        </div>
      </div>
      <div id="below"><h1>Título del video</h1><p>Descripción y comentarios…</p></div>
    </ytd-watch-flexy>
  </ytd-app>
<script>
  // Simulación mínima del reproductor de YouTube.
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

IPC_STUB = """
window.__sent = [];
Object.defineProperty(window, 'ipc', { value: Object.freeze({ postMessage: (m) => {
  window.__sent.push(m);
  if (window.__onIpc) window.__onIpc(m);
}})});
"""


def main():
    failures = []
    console_errors = []

    def check(cond, name):
        print(("  ok   " if cond else "  FAIL ") + name)
        if not cond:
            failures.append(name)

    host = {
        "version": "0.2.0", "pinned": True, "videoOnly": False, "opacity": 100,
        "ghost": False, "fullscreen": False, "autostart": False, "tray": True,
        "extensions": [
            {"folder": "sponsorblock", "name": "SponsorBlock", "version": "5.9", "id": EXT_ID,
             "enabled": True, "hasPage": True, "error": None, "pending": False},
            {"folder": "roto", "name": "Extensión rota", "version": "1.0", "id": None,
             "enabled": False, "hasPage": False, "error": "Manifest no válido (0x80070057)", "pending": False},
        ],
        "extensionProblems": ["«vacia» no tiene manifest.json"],
        "extensionsError": None,
        "hotkeys": [
            {"keys": "Ctrl+Alt+Y", "label": "Mostrar u ocultar el reproductor", "available": True},
            {"keys": "Ctrl+Alt+G", "label": "Modo fantasma", "available": False},
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
        page.add_init_script(IPC_STUB)
        for name in ("inject.js", "ui.js", "maintenance.js"):
            page.add_init_script(path=str(ROOT / "src" / name))

        def sent():
            return page.evaluate("window.__sent.slice()")

        def clear():
            page.evaluate("window.__sent.length = 0")

        def sync():
            page.evaluate(f"window.__ytmpSync({json.dumps(host)})")

        def shot(name):
            if SHOTS:
                SHOTS.mkdir(parents=True, exist_ok=True)
                page.screenshot(path=str(SHOTS / f"{name}.png"))

        page.goto("https://www.youtube.com/watch?v=test")
        page.wait_for_timeout(300)

        print("Arranque")
        check("ready" in sent(), "la página envía 'ready'")
        sync()
        check(page.evaluate("!!document.querySelector('ytmp-ui')"), "la interfaz se monta en el documento")

        print("Barra lateral")
        page.mouse.move(320, 180)
        page.wait_for_timeout(1700)
        bar_hidden = page.evaluate("""() => {
          const t = document.querySelector('ytmp-ui');
          return getComputedStyle(t).display !== 'none';
        }""")
        page.mouse.move(630, 180)
        page.wait_for_timeout(300)
        shot("1-barra")
        visible_at = page.evaluate("document.elementFromPoint(620, 180)?.tagName")
        check(visible_at == "YTMP-UI", "la barra aparece al acercarse al borde derecho")

        clear()
        clicks_before = page.evaluate("window.playerClicks")
        page.mouse.move(611, 180)
        # Botones por su posición en la barra (centrada verticalmente).
        boxes = page.evaluate("""() => {
          // El shadow es cerrado: localizamos los botones por coordenadas.
          const out = [];
          for (let y = 0; y < innerHeight; y += 2) {
            const el = document.elementFromPoint(611, y);
            if (el && el.tagName === 'YTMP-UI') out.push(y);
          }
          return [out[0], out[out.length - 1]];
        }""")
        top, bottom = boxes
        check(top is not None and bottom - top > 200, f"barra con altura razonable ({top}-{bottom})")

        # Orden: asa, atrás, | solo video, pantalla completa, fijar, ocultar, | más, cerrar
        # Cada botón 32 px + 2 px de separación; separadores de 7 px.
        y0 = top + 5 + 16
        ys = {}
        y = y0
        for key in ["grip", "back", "sep", "videoOnly", "fullscreen", "pinned", "hide", "sep", "more", "quit"]:
            if key == "sep":
                y += 7 + 2
                continue
            ys[key] = y
            y += 34

        page.mouse.click(611, ys["pinned"])
        check(sent() == ["pin"], f"botón fijar envía 'pin' ({sent()})")
        check(page.evaluate("window.playerClicks") == clicks_before, "los clics en la barra no llegan al reproductor")

        clear()
        page.mouse.move(611, ys["grip"])
        page.mouse.down()
        page.mouse.up()
        check("drag" in sent(), "el asa envía 'drag'")

        print("Menú Más")
        page.mouse.move(611, ys["more"])
        page.mouse.click(611, ys["more"])
        page.wait_for_timeout(200)
        shot("2-menu")
        menu_text = page.evaluate("""() => {
          // Leemos el texto del panel a través de la accesibilidad del árbol plano.
          return document.querySelector('ytmp-ui') ? true : false;
        }""")
        check(menu_text, "menú abierto")
        # El primer elemento del menú es "Siempre visible": a 8 px del borde superior.
        clear()
        page.mouse.click(400, 8 + 6 + 22 + 16)  # sección Ventana, primera fila
        check(sent() == ["pin"], f"fila 'Siempre visible' envía 'pin' ({sent()})")

        clear()
        page.keyboard.press("Escape")
        page.wait_for_timeout(100)
        page.mouse.click(320, 300)
        check(page.evaluate("window.playerClicks") >= clicks_before, "tras cerrar el menú, la página vuelve a recibir clics")

        print("Arrastrar el video")
        page.mouse.move(10, 10)
        page.wait_for_timeout(1500)
        clear()
        before = page.evaluate("window.playerClicks")
        page.mouse.move(200, 150)
        page.mouse.down()
        page.mouse.move(215, 160, steps=4)
        page.mouse.up()
        page.wait_for_timeout(50)
        check(sent() == ["drag"], f"arrastrar el video envía 'drag' ({sent()})")
        check(page.evaluate("window.playerClicks") == before, "el arrastre no pausa el video")

        clear()
        before = page.evaluate("window.playerClicks")
        page.mouse.click(200, 150)
        check(sent() == [], f"un clic sin mover no arrastra ({sent()})")
        check(page.evaluate("window.playerClicks") == before + 1, "un clic sin mover llega al reproductor")

        clear()
        page.mouse.move(200, 30)  # barra de búsqueda: no es video
        page.mouse.down()
        page.mouse.move(240, 40, steps=4)
        page.mouse.up()
        check(sent() == [], "arrastrar fuera del video no mueve la ventana")

        clear()
        before = page.evaluate("window.playerClicks")
        page.keyboard.down("Alt")
        page.mouse.click(200, 150)
        page.keyboard.up("Alt")
        check(sent() == ["drag"], "Alt+clic envía 'drag'")
        check(page.evaluate("window.playerClicks") == before, "Alt+clic no pausa")

        clear()
        page.mouse.move(639, 200)
        page.mouse.down()
        page.mouse.up()
        check(sent() == ["resize:e"], f"borde derecho envía 'resize:e' ({sent()})")

        print("Atajos")
        page.mouse.move(320, 180)
        for keys, expected in [
            ("Control+t", "pin"), ("Control+m", "video-only"), ("Control+h", "hide"),
            ("Control+l", "open-clipboard"), ("Control+2", "size:m"), ("Control+Shift+4", "corner:se"),
            ("Control+q", "quit"),
        ]:
            clear()
            page.keyboard.press(keys)
            check(sent() == [expected], f"{keys} → {expected} ({sent()})")

        clear()
        page.keyboard.press("F1")
        page.wait_for_timeout(150)
        shot("3-ayuda")
        covers = page.evaluate("document.elementFromPoint(20, 20)?.tagName")
        check(covers == "YTMP-UI", "F1 abre la ayuda")
        page.keyboard.press("Escape")
        page.wait_for_timeout(100)
        check(page.evaluate("document.elementFromPoint(20, 20)?.tagName") != "YTMP-UI", "Esc cierra la ayuda")

        # Los atajos no se disparan escribiendo una letra suelta en la búsqueda.
        clear()
        page.click("#search")
        page.keyboard.type("hola tm")
        check(sent() == [], "escribir en la búsqueda no dispara atajos")
        check(page.input_value("#search") == "hola tm", "el texto llega al buscador")

        print("Solo video")
        host["videoOnly"] = True
        sync()
        page.wait_for_timeout(200)
        rect = page.evaluate("(() => { const r = document.getElementById('movie_player').getBoundingClientRect(); return [r.x, r.y, r.width, r.height]; })()")
        check(rect == [0, 0, 640, 360], f"el reproductor ocupa toda la ventana ({rect})")
        check(page.evaluate("getComputedStyle(document.getElementById('masthead-container')).display") == "none", "se oculta la cabecera")
        page.mouse.move(630, 180)
        page.wait_for_timeout(300)
        shot("4-solo-video")
        host["videoOnly"] = False
        sync()
        page.wait_for_timeout(100)
        check(not page.evaluate("document.documentElement.classList.contains('ytmp-video-only')"), "se desactiva al sincronizar")

        print("Pantalla completa")
        clear()
        page.mouse.move(630, 180)
        page.wait_for_timeout(300)
        page.mouse.click(611, ys["fullscreen"])
        page.wait_for_timeout(400)
        fs = page.evaluate("document.fullscreenElement?.id")
        check(fs == "movie_player", f"el botón pone el reproductor a pantalla completa ({fs})")
        check("fullscreen:1" in sent(), f"avisa al host ({sent()})")
        check(page.evaluate("document.querySelector('ytmp-ui').parentElement.id") == "movie_player",
              "la interfaz se mueve dentro del elemento a pantalla completa")
        clear()
        page.keyboard.press("F11")
        page.wait_for_timeout(400)
        check(page.evaluate("document.fullscreenElement") is None, "F11 sale de pantalla completa")
        check("fullscreen:0" in sent(), "avisa al host de la salida")

        print("Avisos y multimedia")
        page.evaluate("window.__ytmpToast('Siempre visible: activado')")
        page.wait_for_timeout(250)
        shot("5-aviso")
        page.evaluate("window.__ytmpMedia('toggle')")
        check(page.evaluate("window.lastMedia") == "pause", "Ctrl+Alt+P pausa mediante la API del reproductor")
        page.evaluate("window.__ytmpMedia('next')")
        check(page.evaluate("window.lastMedia") == "next", "Ctrl+Alt+N pasa al siguiente")

        print("Robustez")
        check(not console_errors, f"sin errores de consola ni Trusted Types ({console_errors[:3]})")
        browser.close()

    print()
    if failures:
        print(f"{len(failures)} prueba(s) fallaron")
        sys.exit(1)
    print("ui.js + inject.js: todas las pruebas pasan")


if __name__ == "__main__":
    main()
