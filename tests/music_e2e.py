"""Prueba de extremo a extremo de "Mi música" (src/music.html) en Chromium.

- Sirve la página en https://ytmp.localhost/music con la MISMA política de
  seguridad que la app (script autorizado por su hash SHA-256).
- Inyecta el envoltorio real de la app (tests/bundle.py lee webview.rs):
  canal autenticado, media.js (barra de música) e interfaz.
- Hace de "app": responde a /media y /cover y envía los avisos del servicio
  de música con `__ytmpMusic(clave, aviso)`.

Uso:  python tests/music_e2e.py [carpeta_para_capturas]
"""

import base64
import hashlib
import io
import json
import math
import pathlib
import struct
import sys
import wave

from playwright.sync_api import sync_playwright

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from bundle import HOST_KEY, add_app_scripts  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
SHOTS = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else None
KEY = "clave-de-medios-e2e"
TEMPLATE = (ROOT / "src" / "music.html").read_text(encoding="utf-8")


def script_hash(html):
    start = html.rfind("<script>") + len("<script>")
    end = html.rfind("</script>")
    return "sha256-" + base64.b64encode(hashlib.sha256(html[start:end].encode("utf-8")).digest()).decode()


CSP = ("default-src 'none'; script-src '%s'; style-src 'unsafe-inline'; "
       "img-src 'self' data: https://archive.org https://*.archive.org; "
       "media-src 'self' https://archive.org https://*.archive.org; "
       "base-uri 'none'; form-action 'none'; frame-ancestors 'none'") % script_hash(TEMPLATE)


def page_html():
    data = json.dumps({"version": "0.7.0", "mediaKey": KEY})
    return TEMPLATE.replace("__YTMP_DATA__", data.replace("<", "\\u003c"), 1)


def tone(seconds, freq, rate=22050):
    out = io.BytesIO()
    with wave.open(out, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(rate)
        frames = b"".join(struct.pack("<h", int(12000 * math.sin(2 * math.pi * freq * i / rate))) for i in range(int(seconds * rate)))
        w.writeframes(frames)
    return out.getvalue()


SONG = tone(6, 440)
PNG = base64.b64decode("iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==")

TRACKS = [
    {"id": "aaaaaaaaaaaaaaa1", "title": "Supernova", "artist": "Ronan Dec.", "album": "Hirnwald EP", "trackNo": 1, "duration": 263.7,
     "size": 8438638, "format": "mp3", "bitrate": 256, "cover": "0123456789abcdef0123.png", "source": "archive", "license": "CC BY-SA",
     "item": "TAM033-Ronan_Dec", "path": "C:\\Users\\Ana\\Music\\YT Mini Player\\Ronan Dec\\01 - Supernova.mp3", "added": 30},
    {"id": "bbbbbbbbbbbbbbb2", "title": "Canción Número Dos", "artist": "Álvaro", "album": "Mañanas", "trackNo": 2, "duration": 200,
     "size": 5_000_000, "format": "flac", "bitrate": 900, "cover": None, "source": "local", "license": "", "item": "",
     "path": "C:\\Users\\Ana\\Music\\Mañanas\\02.flac", "added": 20},
    {"id": "ccccccccccccccc3", "title": "<img src=x onerror=alert(1)>", "artist": "Hacker", "album": "", "trackNo": None, "duration": 100,
     "size": 1_000_000, "format": "mp3", "bitrate": None, "cover": None, "source": "local", "license": "", "item": "",
     "path": "C:\\x.mp3", "added": 10},
]

FREE = {"type": "plan", "plan": "free", "configured": True, "dev": False,
        "entitlements": {"download": False, "maxPlaylists": 2}, "playlists": 0,
        "email": "", "key": "", "product": "", "status": "", "expiresAt": None, "validatedAt": 0, "busy": False,
        "message": None, "error": False, "canBuyDownloads": True, "canBuyLifetime": True, "canManage": True}
LIFETIME = dict(FREE, plan="lifetime", entitlements={"download": True, "maxPlaylists": None},
                email="a***@example.com", key="****-4D51", product="De por vida", status="active", validatedAt=1790000000,
                message="¡Listo! Plan De por vida activado en este equipo.")

ITEM = {"type": "item", "id": "TAM033-Ronan_Dec", "item": {
    "id": "TAM033-Ronan_Dec", "title": "Ronan Dec. - Hirnwald EP [TAM033]", "creator": "Ronan Dec.", "license": "CC BY-SA",
    "licenseTerms": "Cita a la autoría; si la modificas, comparte con la misma licencia.",
    "licenseUrl": "http://creativecommons.org/licenses/by-sa/2.0/de/", "details": "https://archive.org/details/TAM033-Ronan_Dec",
    "thumbnail": "https://archive.org/services/img/TAM033-Ronan_Dec",
    "tracks": [
        {"file": "TAM033-Ronan_Dec-1-Supernova.mp3", "title": "Supernova", "artist": "ronan dec", "album": "Hirnwald EP", "trackNo": None,
         "duration": 263.71, "size": 8438638, "sha1": "x", "md5": "y", "format": "mp3",
         "existing": {"reason": "same-download", "text": "ya la descargaste", "trackId": "aaaaaaaaaaaaaaa1"}, "queued": False},
        {"file": "TAM033-Ronan_Dec-2-Waehrend_du_schliefst.mp3", "title": "Waehrend du schliefst", "artist": "ronan dec", "album": "Hirnwald EP",
         "trackNo": None, "duration": 375.26, "size": 12008263, "sha1": "x", "md5": "y", "format": "mp3", "existing": None, "queued": False},
        {"file": "TAM033-Ronan_Dec-3-Para.mp3", "title": "Para", "artist": "ronan dec", "album": "Hirnwald EP",
         "trackNo": None, "duration": 257.76, "size": 8248258, "sha1": "x", "md5": "y", "format": "mp3", "existing": None, "queued": False},
    ]}}


def main():
    failures = []

    def check(cond, name):
        print(("  ok   " if cond else "  FAIL ") + name)
        if not cond:
            failures.append(name)

    with sync_playwright() as p:
        browser = p.chromium.launch(args=["--autoplay-policy=no-user-gesture-required"])
        errors = []
        media_requests = []
        page = browser.new_page(viewport={"width": 760, "height": 440})
        page.on("console", lambda m: m.type == "error" and errors.append(m.text))
        page.on("pageerror", lambda e: errors.append(str(e)))

        def ytmp(route):
            url = route.request.url
            if "/media/" in url:
                media_requests.append(url)
                route.fulfill(status=200, content_type="audio/wav", body=SONG, headers={"Accept-Ranges": "bytes"})
            elif "/cover/" in url:
                route.fulfill(status=200, content_type="image/png", body=PNG)
            else:
                route.fulfill(status=200, content_type="text/html; charset=utf-8", headers={"Content-Security-Policy": CSP}, body=page_html())
        page.route("https://ytmp.localhost/**", ytmp)
        page.route("https://archive.org/services/img/**", lambda r: r.fulfill(status=200, content_type="image/png", body=PNG))
        page.route("https://archive.org/download/**", lambda r: r.fulfill(status=200, content_type="audio/wav", body=tone(3, 660)))
        add_app_scripts(page, maintenance=False)
        page.goto("https://ytmp.localhost/music")
        page.wait_for_timeout(400)

        sent = lambda: page.evaluate("window.__sent.filter(m => !m.startsWith('media-state:'))")
        states = lambda: [json.loads(m[len("media-state:"):]) for m in page.evaluate("window.__sent.slice()") if m.startswith("media-state:")]
        clear = lambda: page.evaluate("window.__sent.length = 0")
        push = lambda msg: (page.evaluate(f"window.__ytmpMusic({json.dumps(HOST_KEY)}, {json.dumps(msg)})"), page.wait_for_timeout(120))

        def shot(name):
            if SHOTS:
                SHOTS.mkdir(parents=True, exist_ok=True)
                page.screenshot(path=str(SHOTS / f"{name}.png"))

        # ------------------------------------------------------------------
        print("Arranque y biblioteca")
        check("music:ready" in sent(), f"avisa a la app que está lista ({sent()})")
        check(page.is_visible("#library-empty"), "biblioteca vacía: invita a añadir música")
        shot("1-vacia")
        clear()
        page.click("#empty-add")
        check(sent() == ["music:pick-folder"], f"«Añadir carpeta» pide el diálogo a la app ({sent()})")

        # Un aviso con otra clave no hace nada.
        page.evaluate("window.__ytmpMusic('clave-falsa', { type: 'library', tracks: [{ id: 'x', title: 'falsa' }] })")
        page.wait_for_timeout(100)
        check(page.locator(".row").count() == 0, "un aviso sin la clave de la app se ignora")

        push({"type": "library", "tracks": TRACKS, "playlists": [], "folders": ["C:\\Users\\Ana\\Music"], "quality": "compact", "mediaKey": KEY})
        push(FREE)
        rows = page.locator("#library-list .row")
        check(rows.count() == 3, f"muestra las 3 canciones ({rows.count()})")
        check(rows.nth(0).locator(".title").inner_text() == "Supernova", "las más recientes primero")
        check(page.locator("#library-list img").count() >= 1, "con portada cuando la hay")
        check(page.evaluate("document.querySelectorAll('img[src=x]').length") == 0 and page.locator("text=<img src=x").count() == 1,
              "las etiquetas se muestran como texto, nunca como HTML")
        page.fill("#filter", "cancion numero")
        page.wait_for_timeout(80)
        check(rows.count() == 1 and "Dos" in rows.nth(0).inner_text(), "el filtro ignora tildes y mayúsculas")
        page.fill("#filter", "")
        page.select_option("#sort", "artist")
        page.wait_for_timeout(80)
        check(rows.nth(0).locator(".title").inner_text() == "Canción Número Dos", "ordenar por artista (Álvaro primero)")
        page.select_option("#sort", "added")
        check(page.is_visible("#folders"), "resumen de carpetas al final")
        shot("2-biblioteca")

        # ------------------------------------------------------------------
        print("Reproducción")
        clear()
        rows.nth(1).click()
        page.wait_for_timeout(700)
        src = page.evaluate("document.getElementById('audio').getAttribute('src')")
        check(src == f"/media/bbbbbbbbbbbbbbb2?k={KEY}", f"pide la canción con la clave de sesión ({src})")
        check(page.evaluate("!document.getElementById('audio').paused"), "suena")
        check(page.inner_text("#now-title") == "Canción Número Dos", "el reproductor muestra la canción")
        check(page.locator("#library-list .row.playing").count() == 1, "la fila que suena se resalta")
        meta = page.evaluate("({ t: navigator.mediaSession.metadata && navigator.mediaSession.metadata.title, s: navigator.mediaSession.playbackState })")
        check(meta == {"t": "Canción Número Dos", "s": "playing"}, f"Media Session al día ({meta})")
        page.evaluate("window.__ytmpMediaReport()")
        page.wait_for_timeout(200)
        st = states()
        last = st[-1] if st else {}
        check(last.get("site") == "ytmp.localhost" and last.get("title") == "Canción Número Dos" and last.get("canNext"),
              f"media.js informa a la app (barra de la barra de tareas): {last.get('title')!r}, {last.get('site')!r}")
        # La barra de música / bandeja / atajos globales: acción "siguiente".
        page.evaluate("window.__ytmpMedia('next')")
        page.wait_for_timeout(500)
        check(page.inner_text("#now-title") == "<img src=x onerror=alert(1)>", "«siguiente» desde la app pasa a la siguiente")
        page.evaluate("window.__ytmpMedia('toggle')")
        page.wait_for_timeout(200)
        check(page.evaluate("document.getElementById('audio').paused"), "y «reproducir/pausar» la pausa")
        page.click("#repeat")
        check(page.get_attribute("#repeat", "aria-pressed") == "true", "repetir se activa")
        page.click("#shuffle")
        check(page.get_attribute("#shuffle", "aria-pressed") == "true", "aleatorio se activa")
        shot("3-sonando")

        # ------------------------------------------------------------------
        print("Menú de una canción")
        clear()
        rows.nth(0).locator(".more").click()
        page.click(".menu >> text=Mostrar en la carpeta")
        check(sent() == ["music:reveal:aaaaaaaaaaaaaaa1"], f"mostrar en la carpeta ({sent()})")
        rows.nth(0).locator(".more").click()
        page.click(".menu >> text=Añadir a una playlist")
        check(page.is_visible(".dialog >> text=Añadir a una playlist") and page.is_enabled(".dialog >> text=Nueva playlist"),
              "con el plan Gratis se pueden crear playlists (hasta 2)")
        page.keyboard.press("Escape")
        # Un límite del plan: una línea discreta bajo la cabecera, sin ventanas.
        push({"type": "notice", "feature": "playlists", "text": "Tienes 2 playlists, el máximo del plan Gratis."})
        check(page.is_visible("#notice") and "máximo del plan Gratis" in page.inner_text("#notice"), "los límites se avisan en una línea")
        check(page.locator(".scrim").count() == 0, "sin ventanas que interrumpan")
        page.click("#notice-plans")
        check(page.get_attribute("[data-tab='plan']", "aria-selected") == "true" and page.is_hidden("#notice"),
              "«Ver planes» lleva a la pestaña Plan y cierra el aviso")
        push({"type": "notice", "feature": "download", "text": "Descargar necesita un plan."})
        page.click("#notice-close")
        check(page.is_hidden("#notice"), "el aviso se cierra con ×")
        page.click("[data-tab='library']")
        clear()
        rows.nth(2).locator(".more").click()
        page.click(".menu >> text=Enviar a la Papelera")
        check(sent() == [], "la papelera pide confirmación antes")
        page.click(".dialog >> .btn.primary")
        check(sent() == ['music:remove:{"ids":["ccccccccccccccc3"],"trash":true}'], f"y luego la pide a la app ({sent()})")

        # ------------------------------------------------------------------
        print("Plan y licencia")
        page.click("[data-tab='plan']")
        page.wait_for_timeout(100)
        check(page.locator(".plan").count() == 3, "tres planes")
        check(page.locator(".plan.current h3").inner_text() == "Gratis", "marca el plan actual")
        check("2 USD" in page.inner_text(".plans") and "10 USD" in page.inner_text(".plans"), "con sus precios")
        plans_text = page.inner_text(".plans")
        check("2 playlists" in plans_text and "5 playlists" in plans_text and "Playlists ilimitadas" in plans_text, "y sus playlists (2 / 5 / ilimitadas)")
        check("nada de lo que creaste se borra" in page.inner_text("#plan-body"), "explica que cambiar de plan no borra nada")
        clear()
        page.click(".plan:has-text('De por vida') >> button")
        check(sent() == ["music:buy:lifetime"], f"comprar abre la página de pago ({sent()})")
        clear()
        page.fill(".license-form input", "38B1460A-5104-4067-A91D-77B872934D51")
        page.click(".license-form button")
        check(sent() == ["music:license-activate:38B1460A-5104-4067-A91D-77B872934D51"], f"activar envía la clave ({sent()})")
        check(page.input_value(".license-form input") == "", "la clave no se queda en la página")
        push(dict(FREE, busy=True))
        check(page.is_disabled(".license-form button"), "mientras comprueba, el botón espera")
        push(LIFETIME)
        check(page.locator(".plan.current h3").inner_text() == "De por vida", "con licencia, cambia el plan")
        check("****-4D51" in page.inner_text("#plan-body") and "a***@example.com" in page.inner_text("#plan-body"), "muestra la licencia enmascarada")
        check("¡Listo!" in page.inner_text(".msg"), "y el mensaje de la tienda")
        shot("4-plan")
        push(dict(FREE, configured=False))
        check("no tiene la tienda configurada" in page.inner_text("#plan-body"), "avisa si la compilación no tiene tienda")
        push(dict(LIFETIME, dev=True, message=None))
        check("Compilación de desarrollo" in page.inner_text("#plan-body"), "y si es de desarrollo")

        # ------------------------------------------------------------------
        print("Descargar (Internet Archive)")
        push(FREE)
        page.click("[data-tab='download']")
        check("Internet Archive" in page.inner_text("#download-body") and "licencia libre" in page.inner_text("#download-body"),
              "explica de dónde viene la música y su licencia")
        clear()
        page.fill("#search", "ronan dec")
        page.press("#search", "Enter")
        check(sent() == ['music:search:{"q":"ronan dec","page":1}'], f"buscar ({sent()})")
        push({"type": "search", "query": "ronan dec", "page": 1, "total": 31, "hits": [
            {"id": "TAM033-Ronan_Dec", "title": "Ronan Dec. - Hirnwald EP [TAM033]", "creator": "Ronan Dec.", "license": "BySa",
             "licenseLabel": "CC BY-SA", "downloads": 191994, "year": "2008"}]})
        check(page.locator(".hit").count() == 1 and "CC BY-SA" in page.inner_text(".hit"), "resultados con su licencia")
        check(page.is_visible(".pager"), "con páginas")
        clear()
        page.click(".hit")
        check(sent() == ["music:item:TAM033-Ronan_Dec"], f"abrir un álbum ({sent()})")
        push(ITEM)
        tracks = page.locator(".track")
        check(tracks.count() == 3, "las canciones del álbum")
        check("Ya la tienes" in tracks.nth(0).inner_text(), "marca la que ya tienes")
        check("2 ya en tu biblioteca" not in page.inner_text("#download-body") and "1 ya en tu biblioteca" in page.inner_text("#download-body"),
              "cuenta lo que no se volverá a descargar")
        check(tracks.nth(1).locator(".btn").count() == 0 and "Para descargarlas" in page.inner_text(".album-actions")
              and page.locator(".scrim").count() == 0, "sin plan: ni botones de descarga ni ventanas; una nota y la vista previa gratis")
        # Vista previa en línea (gratis).
        tracks.nth(2).locator(".preview").click()
        page.wait_for_timeout(500)
        audio_src = page.evaluate("document.getElementById('audio').src")
        check(audio_src.startswith("https://archive.org/download/TAM033-Ronan_Dec/"), f"vista previa desde el Archive ({audio_src})")
        check("Vista previa" in page.inner_text("#now-sub"), "el reproductor dice que es una vista previa")
        push(LIFETIME)
        clear()
        page.click(".album-actions .btn.primary")
        msg = sent()
        check(len(msg) == 1 and msg[0].startswith("music:download:"), f"descargar el álbum ({msg})")
        if msg:
            args = json.loads(msg[0][len("music:download:"):])
            check(args["playlist"] is True and len(args["files"]) == 3, "como playlist, todo el álbum (la app salta lo repetido)")
        push({"type": "downloads", "jobs": [
            {"id": 1, "item": "TAM033-Ronan_Dec", "file": "a", "title": "Supernova", "artist": "ronan dec", "state": "skipped", "received": 0, "size": 1, "message": "ya la descargaste", "trackId": "aaaaaaaaaaaaaaa1"},
            {"id": 2, "item": "TAM033-Ronan_Dec", "file": "b", "title": "Waehrend du schliefst", "artist": "ronan dec", "state": "downloading", "received": 6004131, "size": 12008263, "message": "", "trackId": None},
            {"id": 3, "item": "TAM033-Ronan_Dec", "file": "c", "title": "Para", "artist": "ronan dec", "state": "queued", "received": 0, "size": 8248258, "message": "", "trackId": None}]})
        check(page.is_visible("#jobs") and "Descargando 2" in page.inner_text("#jobs-title"), "cola de descargas visible")
        check("50%" in page.inner_text("#jobs-list"), "con el progreso")
        check(page.inner_text("#jobs-count") == "2", "y el contador en la pestaña")
        clear()
        page.click("#jobs-cancel")
        check(sent() == ["music:cancel:all"], "cancelar todo")
        shot("5-descargas")

        # ------------------------------------------------------------------
        print("Duplicados")
        page.click("[data-tab='dupes']")
        clear()
        page.click("#dupes-run")
        check(sent() == ["music:dupes"], "buscar duplicados")
        push({"type": "dupes", "running": True, "done": 3, "total": 10})
        check("3 de 10" in page.inner_text("#dupes-status"), "muestra el avance")
        push({"type": "dupes", "running": False, "wasted": 8438638, "groups": [
            {"reason": "same-audio", "tracks": ["bbbbbbbbbbbbbbb2", "aaaaaaaaaaaaaaa1"], "wasted": 8438638}]})
        check(page.locator(".group").count() == 1 and "Mismo audio" in page.inner_text(".group"), "grupo con su motivo")
        check("Mejor calidad" in page.locator(".dupe").nth(0).inner_text(), "sugiere conservar la mejor")
        page.locator(".dupe input").nth(1).check()
        clear()
        page.click(".group >> text=Papelera para las demás")
        page.click(".dialog >> .btn.primary")
        check(sent() == ['music:remove:{"ids":["bbbbbbbbbbbbbbb2"],"trash":true}'], f"envía a la papelera las que no se conservan ({sent()})")
        shot("6-duplicados")

        # ------------------------------------------------------------------
        print("Playlists")
        push({"type": "library", "tracks": TRACKS, "folders": [], "quality": "compact", "playlists": [
            {"id": "a1b2c3d4e5f60708", "name": "Para correr", "tracks": ["aaaaaaaaaaaaaaa1", "bbbbbbbbbbbbbbb2"], "created": 1, "updated": 1, "origin": None}]})
        page.click("[data-tab='playlists']")
        check(page.locator(".playlist-row").count() == 1, "lista de playlists")
        page.click(".playlist-row")
        check(page.locator("#playlists-body .row").count() == 2, "abre la playlist")
        clear()
        page.fill(".pl-head input", "Para correr rápido")
        page.press(".pl-head input", "Tab")
        check(sent() == ['music:playlist-save:{"id":"a1b2c3d4e5f60708","name":"Para correr rápido","tracks":["aaaaaaaaaaaaaaa1","bbbbbbbbbbbbbbb2"]}'],
              f"renombrar guarda la playlist ({sent()})")
        clear()
        page.click(".pl-head >> text=Reproducir")
        page.wait_for_timeout(400)
        check(page.inner_text("#now-title") == "Supernova", "reproduce la playlist desde el principio")
        page.click(".pl-head .btn.icon")
        push(dict(FREE, playlists=1))
        check("1 de 2" in page.inner_text("#playlists-body") and page.is_enabled("#playlists-body >> text=Nueva playlist"),
              "plan Gratis: 1 de 2 playlists, se puede crear otra")
        two = [{"id": "a1b2c3d4e5f60708", "name": "Para correr", "tracks": ["aaaaaaaaaaaaaaa1"], "created": 1, "updated": 1, "origin": None},
               {"id": "b1b2c3d4e5f60709", "name": "Para estudiar", "tracks": ["bbbbbbbbbbbbbbb2"], "created": 2, "updated": 2, "origin": None}]
        push({"type": "library", "tracks": TRACKS, "folders": [], "quality": "compact", "playlists": two})
        body = page.inner_text("#playlists-body")
        check("2 de 2" in body and page.is_disabled("#playlists-body >> text=Nueva playlist") and page.is_visible("text=Más playlists"),
              "con 2 de 2, «Nueva playlist» se desactiva y aparece «Más playlists»")
        page.click(".playlist-row >> nth=1")
        clear()
        page.fill(".pl-head input", "Estudio")
        page.press(".pl-head input", "Tab")
        check(len(sent()) == 1 and '"name":"Estudio"' in sent()[0], "las que ya existen se siguen editando")
        page.click(".pl-head .btn.icon")
        push(LIFETIME)
        check("sin límite" in page.inner_text("#playlists-body") and page.is_enabled("#playlists-body >> text=Nueva playlist"),
              "De por vida: sin límite")
        check(page.locator(".scrim").count() == 0, "nunca quedó una ventana abierta")
        shot("7-playlists")

        # ------------------------------------------------------------------
        print("Ventanas pequeñas")
        page.click("[data-tab='library']")
        for w, h in [(480, 270), (256, 144)]:
            page.set_viewport_size({"width": w, "height": h})
            page.wait_for_timeout(150)
            fits = page.evaluate("""() => {
                const r = document.getElementById('player').getBoundingClientRect();
                return r.bottom <= innerHeight + 1 && r.right <= innerWidth + 1 && document.documentElement.scrollWidth <= innerWidth;
            }""")
            check(fits, f"{w}×{h}: el reproductor se ve entero y no hay desplazamiento lateral")
            check(page.is_visible("#play"), f"{w}×{h}: el botón de reproducir sigue a mano")
            shot(f"8-{w}x{h}")

        check(all("/media/" in u and f"k={KEY}" in u for u in media_requests) and media_requests, "todas las canciones se piden con la clave")
        check(not errors, f"sin errores de consola ni de CSP ({errors[:3]})")
        browser.close()

    print()
    if failures:
        print(f"{len(failures)} prueba(s) fallaron")
        sys.exit(1)
    print("Mi música: todas las pruebas pasan")


if __name__ == "__main__":
    main()
