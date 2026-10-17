"""Prueba de extremo a extremo del mini controlador y de media.js en Chromium.

1. Sirve src/mini.html en https://ytmp.localhost/mini con la misma CSP que la
   app, le pasa estados como lo hace la app y comprueba lo que envía.
2. Carga media.js en una página con un <audio> real (WAV generado en el
   navegador) y la Media Session real de Chromium: comprueba que anota los
   controles del sitio, que reproduce/pausa/busca/cambia el volumen y que
   informa el estado.

Uso:  python tests/mini_e2e.py [carpeta_para_capturas]
"""

import json
import pathlib
import sys

from playwright.sync_api import sync_playwright

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from bundle import HOST_KEY, add_app_scripts  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
SHOTS = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else None
MINI_CSP = ("default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; "
            "img-src https: data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'")

# Miniatura 16:9 servida como https (un SVG sencillo).
THUMB = """<svg xmlns="http://www.w3.org/2000/svg" width="320" height="180">
<defs><linearGradient id="g" x1="0" x2="1" y1="0" y2="1"><stop offset="0" stop-color="#3b4b8f"/><stop offset="1" stop-color="#b0476a"/></linearGradient></defs>
<rect width="320" height="180" fill="url(#g)"/><circle cx="230" cy="70" r="38" fill="#ffd27a"/></svg>"""

IPC_STUB = """
window.__sent = [];
Object.defineProperty(window, 'ipc', { value: Object.freeze({ postMessage: (m) => window.__sent.push(m) }) });
"""

# Página con un <audio> real de 30 s de silencio y controles de Media Session.
MEDIA_PAGE = """<!doctype html><html><head><meta charset="utf-8"><title>Canción de prueba - YouTube</title></head><body>
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
    title: 'Canción de prueba', artist: 'Artista',
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
        print("Barra de música (anclada en la barra de tareas, 360×40)")
        errors = []
        page = browser.new_page(viewport={"width": 360, "height": 40})
        # El 404 de la miniatura rota es intencional (se prueba el respaldo).
        page.on("console", lambda m: m.type == "error" and "404" not in m.text and errors.append(m.text))
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.route("https://ytmp.localhost/**", lambda r: r.fulfill(
            status=200, content_type="text/html; charset=utf-8",
            headers={"Content-Security-Policy": MINI_CSP},
            body=(ROOT / "src" / "mini.html").read_text(encoding="utf-8")))
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

        check(sent()[:1] == ["mini:ready"], f"avisa que está lista ({sent()})")
        check(page.inner_text("#title") == "Nada sonando", "estado vacío por defecto")
        check(all(page.is_disabled(b) for b in ("#toggle", "#next", "#previous")), "sin medio, los controles están desactivados")
        check(has_class("#wave", "idle") and has_class("#progress", "disabled"), "onda en reposo y progreso desactivado")
        push(site="music.youtube.com")
        check(page.inner_text("#artist") == "YouTube Music", "vacía en un servicio de música: dice cuál es")
        clear()
        page.mouse.move(300, 20)
        page.mouse.wheel(0, -100)
        page.click("#wave")
        page.mouse.click(220, 38)
        check(sent() in ([], ["mini:hover:1"]), f"sin medio, rueda, onda y progreso no envían nada ({sent()})")
        shot("1-vacia")

        state = dict(hasMedia=True, title="Una canción con un título bastante largo que no cabe en la barra y sigue",
                     artist="Canal de ejemplo", artwork="https://i.ytimg.com/vi/x/mqdefault.jpg", playing=True,
                     position=83, duration=245, volume=0.6, muted=False, canNext=True, canPrevious=True, canSeek=True,
                     site="music.youtube.com")
        push(**state)
        check(page.inner_text("#artist") == "Canal de ejemplo", "muestra el artista")
        check(page.is_hidden("#icon-play") and page.is_visible("#icon-pause"), "reproduciendo: muestra pausa")
        check(has_class("#wave", "playing") and not has_class("#wave", "idle"), "la onda se anima mientras suena")
        check(abs(fill_ratio() - 83 / 245) < 0.01, f"la línea de progreso marca 1:23 de 4:05 ({fill_ratio():.3f})")
        page.wait_for_timeout(200)
        check(page.evaluate("document.getElementById('artwork').naturalWidth > 0"), "carga la miniatura https")
        shot("2-sonando")

        for button, message in [("#toggle", "media:toggle"), ("#next", "media:next"), ("#previous", "media:previous"),
                                ("#art", "player:show"), ("#wave", "media:mute")]:
            clear()
            page.click(button)
            check(message in sent(), f"{button} envía {message} ({sent()})")

        clear()
        page.click("#title", button="right")
        check("mini:close" in sent(), f"clic derecho cierra ({sent()})")

        box = page.locator("#progress").bounding_box()
        y = box["y"] + box["height"] - 2
        clear()
        page.mouse.move(box["x"] + box["width"] * 0.5, y)
        page.wait_for_timeout(50)
        tip = page.locator(".tip")
        check(tip.count() == 1 and tip.inner_text().endswith("/ 4:05"), f"al pasar por la línea muestra el tiempo ({tip.all_inner_texts()})")
        page.mouse.down()
        page.mouse.up()
        msgs = [m for m in sent() if m.startswith("media:seek-to:")]
        check(len(msgs) == 1 and 115 <= int(msgs[0].split(":")[-1]) <= 130, f"clic en la mitad busca ~2:02 ({msgs})")

        clear()
        page.mouse.move(box["x"] + box["width"] * 0.2, y)
        page.mouse.down()
        page.mouse.move(box["x"] + box["width"] * 0.5, y - 20, steps=4)
        mid = [m for m in sent() if m.startswith("media:seek-to:")]
        check(not mid and has_class("#progress", "dragging"), "arrastrando la línea no busca hasta soltar")
        check(abs(fill_ratio() - 0.5) < 0.03, f"la línea sigue al ratón aunque salga de ella ({fill_ratio():.3f})")
        page.mouse.move(box["x"] + box["width"] * 0.75, y - 20, steps=4)
        page.mouse.up()
        msgs = [m for m in sent() if m.startswith("media:seek-to:")]
        check(len(msgs) == 1 and 175 <= int(msgs[0].split(":")[-1]) <= 190, f"al soltar busca ~3:04 ({msgs})")
        check(page.locator(".tip").count() == 0 and not has_class("#progress", "dragging"), "y se limpia el arrastre")
        drag_msgs = [m for m in sent() if m.startswith("mini:drag")]
        check(not drag_msgs, f"buscar en la línea no mueve la barra ({drag_msgs})")

        # Mover la barra por la barra de tareas arrastrando el texto.
        clear()
        tb = page.locator("#text").bounding_box()
        page.mouse.move(tb["x"] + 20, tb["y"] + 10)
        page.mouse.down()
        page.mouse.move(tb["x"] + 22, tb["y"] + 10)
        check(not [m for m in sent() if m.startswith("mini:drag")], "un temblor de 2 px no empieza a arrastrar")
        page.mouse.move(tb["x"] - 60, tb["y"] + 10, steps=6)
        page.wait_for_timeout(60)
        page.mouse.up()
        drag_msgs = [m for m in sent() if m.startswith("mini:drag")]
        check(drag_msgs[:1] == ["mini:drag:start"] and drag_msgs[-1:] == ["mini:drag:end"] and "mini:drag:move" in drag_msgs,
              f"arrastrar el texto mueve la barra: start, move…, end ({drag_msgs[:3]}…{drag_msgs[-1:]})")
        check(not any(m.startswith("media:") or m == "player:show" for m in sent()), "y no dispara ningún control")
        check(page.evaluate("getComputedStyle(document.getElementById('text')).cursor") == "grab", "el texto tiene cursor de mano")
        clear()
        push({"docked": False}, **state)
        page.mouse.move(tb["x"] + 20, tb["y"] + 10)
        page.mouse.down()
        page.mouse.move(tb["x"] - 60, tb["y"] + 10, steps=6)
        page.mouse.up()
        check(not [m for m in sent() if m.startswith("mini:drag")], "flotante (sin barra de tareas) no se arrastra")
        push(**state)

        clear()
        page.mouse.move(150, 20)
        page.mouse.wheel(0, -100)
        check(has_class("#wave", "meter") and page.inner_text("#level") == "Volumen 65 %"
              and has_class("#text", "show-level"), f"la rueda muestra el nivel ({page.inner_text('#level')})")
        page.mouse.wheel(0, 100)
        vols = [m for m in sent() if m.startswith("media:volume-by:")]
        check(vols == ["media:volume-by:5", "media:volume-by:-5"], f"la rueda sube y baja el volumen ({vols})")
        page.wait_for_timeout(1600)
        check(not has_class("#wave", "meter") and not has_class("#text", "show-level"), "el medidor vuelve a la onda y al artista")

        push(**dict(state, volume=0.35))
        on = page.locator("#wave i.on").count()
        check(has_class("#wave", "meter") and page.inner_text("#level") == "Volumen 35 %" and on == 6,
              f"un cambio de volumen desde el atajo también se ve ({page.inner_text('#level')}, {on} barras)")
        shot("3-volumen")

        clear()
        page.mouse.move(-10, -10)
        page.mouse.move(100, 20)
        page.mouse.move(500, 500)
        hov = [m for m in sent() if m.startswith("mini:hover")]
        check(hov[-2:] == ["mini:hover:1", "mini:hover:0"], f"informa cuando el ratón entra y sale ({hov})")

        page.wait_for_timeout(1500)
        push(**dict(state, playing=False, muted=True, canNext=False, artwork="https://broken.example/x.jpg"))
        page.wait_for_timeout(250)
        check(page.is_visible("#icon-play"), "en pausa: muestra reproducir")
        check(page.is_disabled("#next") and not page.is_disabled("#previous"), "respeta lo que el sitio permite")
        check(has_class("#wave", "muted") and not has_class("#wave", "meter") and page.inner_text("#level") == "Silencio",
              "silenciado: manda sobre el medidor y lo dice")
        check(page.evaluate("getComputedStyle(document.querySelector('.mute-icon')).opacity") == "1", "silenciado: icono de altavoz tachado")
        check(not has_class("#wave", "playing"), "silenciado: la onda no se anima")
        check(page.is_visible("#empty"), "si la miniatura falla, muestra el icono")
        shot("4-silencio")

        push(hasMedia=True, title="Directo", artist="Canal", live=True, duration=0, canSeek=False, playing=True, volume=1)
        check(has_class("#progress", "live") and has_class("#progress", "disabled"), "directo: línea llena y sin búsqueda")
        check(page.inner_text("#artist") == "En directo · Canal", "directo: lo indica junto al artista")
        clear()
        page.mouse.click(220, 38)
        check(not [m for m in sent() if m.startswith("media:seek")], "directo: clic en la línea no busca")

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

        # Windows 11 (40), Windows 10 (32), flotante (48) y estrecha (para no tapar iconos).
        for w, h in [(360, 40), (360, 32), (360, 48), (280, 40), (250, 32), (540, 60)]:
            m = layout(w, h)
            shown = [b for b in m["boxes"] if b[5]]
            inside = all(0 <= l and r <= w and 0 <= t and b <= h for _, l, r, t, b, _ in shown)
            ordered = all(shown[i][2] <= shown[i + 1][1] for i in range(len(shown) - 1))
            text_w = next(b[2] - b[1] for b in m["boxes"] if b[0] == "text")
            wave_shown = next(b[5] for b in m["boxes"] if b[0] == "wave")
            check(inside and ordered and not m["overflow"] and m["lines"] == 1 and text_w >= 60,
                  f"{w}×{h}: todo en fila y dentro, título en una línea, texto de {text_w:.0f} px")
            check(wave_shown == (w > 300), f"{w}×{h}: onda {'visible' if w > 300 else 'oculta para ganar sitio'}")
            if (w, h) == (360, 32):
                shot("5-windows10")
            if (w, h) == (280, 40):
                shot("6-estrecha")
        layout(360, 40)
        check(page.evaluate("document.getElementById('title').scrollWidth > document.getElementById('title').clientWidth"),
              "el título largo se corta con puntos suspensivos")

        check(not page.evaluate("document.body.classList.contains('css-border')"), "en Windows 11 el borde lo pone el sistema")
        push({"cssBorder": True}, **state)
        check(page.evaluate("document.body.classList.contains('css-border')"), "en Windows 10 dibuja su propio borde")
        check(not errors, f"sin errores de consola ni de CSP ({errors[:3]})")
        page.close()

        # ------------------------------------------------------------------
        print("media.js con un medio real")
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
        check(s["title"] == "Canción de prueba" and s["artist"] == "Artista", "lee los metadatos reales de la Media Session")
        check(s["artwork"] == "https://img.example/512.jpg", "elige la miniatura adecuada")
        check(abs(s["duration"] - 30) < 0.5 and not s["playing"], f"duración real y en pausa ({s['duration']})")
        check(s["canNext"] and s["canPrevious"], "detecta los controles del sitio")
        check(s["site"] == "www.youtube.com" and s["watching"] is False, f"informa el sitio (decide la barra de música) ({s['site']})")

        page.evaluate("window.__ytmpMedia('toggle')")
        page.wait_for_timeout(400)
        check(page.evaluate("!window.audio.paused"), "reproduce el audio real")
        check(states()[-1]["playing"], "e informa que está reproduciendo")

        page.evaluate("window.__ytmpMedia('next'); window.__ytmpMedia('previous')")
        check(page.evaluate("window.siteCalls") == ["next", "previous"], "siguiente/anterior usan los controles del sitio")

        page.evaluate("window.__ytmpMedia('seek-to', 12)")
        page.wait_for_timeout(200)
        check(abs(page.evaluate("window.audio.currentTime") - 12) < 0.6, "busca en el audio real")
        page.evaluate("window.__ytmpMedia('volume', 0.35)")
        page.wait_for_timeout(200)
        check(abs(page.evaluate("window.audio.volume") - 0.35) < 1e-6, "cambia el volumen real")
        check(abs(states()[-1]["volume"] - 0.35) < 1e-6, "e informa el volumen")

        page.evaluate("window.__ytmpMediaWatch(true)")
        before = len(states())
        page.wait_for_timeout(2300)
        check(len(states()) - before >= 2, f"observando, informa la posición cada segundo ({len(states()) - before})")
        check(states()[-1]["watching"] is True, "y dice que está observando (la app lo comprueba tras recargas)")
        page.evaluate("window.__ytmpMediaWatch(false)")
        page.evaluate("window.__ytmpMedia('toggle')")
        page.wait_for_timeout(300)
        check(page.evaluate("window.audio.paused"), "pausa el audio real")
        check(not errors, f"sin errores en la página ({errors[:3]})")
        browser.close()

    print()
    if failures:
        print(f"{len(failures)} prueba(s) fallaron")
        sys.exit(1)
    print("mini.html + media.js: todas las pruebas pasan")


if __name__ == "__main__":
    main()
