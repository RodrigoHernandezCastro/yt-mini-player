"""End-to-end test of "My music" (src/music.html) in Chromium.

- Serves the page at https://ytmp.localhost/music with the SAME security
  policy as the app (script allowed by its SHA-256 hash) and its strings.
- Injects the app's real wrapper (tests/bundle.py reads webview.rs):
  authenticated channel, media.js (music bar) and side bar.
- Acts as the "app": answers /media and /cover and sends the music
  service's messages with `__ytmpMusic(key, message)`. Texts from the app
  are messages ({key, args}) that the page shows in its current language.
- Switches language the way the app does and checks plurals, number and
  size formats, and that texts already shown follow the change.

Usage:  python tests/music_e2e.py [folder_for_screenshots]
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
from bundle import HOST_KEY, add_app_scripts, fill, i18n, push_locale, text  # noqa: E402

ROOT = pathlib.Path(__file__).resolve().parent.parent
SHOTS = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else None
KEY = "e2e-media-key"
TEMPLATE = (ROOT / "src" / "music.html").read_text(encoding="utf-8")


def script_hash(html):
    start = html.rfind("<script>") + len("<script>")
    end = html.rfind("</script>")
    return "sha256-" + base64.b64encode(hashlib.sha256(html[start:end].encode("utf-8")).digest()).decode()


CSP = ("default-src 'none'; script-src '%s'; style-src 'unsafe-inline'; "
       "img-src 'self' data: https://archive.org https://*.archive.org; "
       "media-src 'self' https://archive.org https://*.archive.org; "
       "base-uri 'none'; form-action 'none'; frame-ancestors 'none'") % script_hash(TEMPLATE)


def page_html(lang="en"):
    return fill("music.html", {"version": "0.8.0", "mediaKey": KEY, "i18n": i18n(lang, "music")}, lang)


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
                email="a***@example.com", key="****-4D51", product="Lifetime", status="active", validatedAt=1790000000,
                message={"key": "music.license.activated_lifetime"})

ITEM = {"type": "item", "id": "TAM033-Ronan_Dec", "item": {
    "id": "TAM033-Ronan_Dec", "title": "Ronan Dec. - Hirnwald EP [TAM033]", "creator": "Ronan Dec.", "license": "BySa",
    "licenseLabel": "CC BY-SA",
    "licenseUrl": "http://creativecommons.org/licenses/by-sa/2.0/de/", "details": "https://archive.org/details/TAM033-Ronan_Dec",
    "thumbnail": "https://archive.org/services/img/TAM033-Ronan_Dec",
    "tracks": [
        {"file": "TAM033-Ronan_Dec-1-Supernova.mp3", "title": "Supernova", "artist": "ronan dec", "album": "Hirnwald EP", "trackNo": None,
         "duration": 263.71, "size": 8438638, "sha1": "x", "md5": "y", "format": "mp3",
         "existing": {"reason": "same-download", "message": {"key": "music.existing.same_download"}, "trackId": "aaaaaaaaaaaaaaa1"}, "queued": False},
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
        push_locale(page, "en", "music")
        page.wait_for_timeout(400)

        sent = lambda: page.evaluate("window.__sent.filter(m => !m.startsWith('media-state:'))")
        states = lambda: [json.loads(m[len("media-state:"):]) for m in page.evaluate("window.__sent.slice()") if m.startswith("media-state:")]
        clear = lambda: page.evaluate("window.__sent.length = 0")
        push = lambda msg: (page.evaluate(f"window.__ytmpMusic({json.dumps(HOST_KEY)}, {json.dumps(msg)})"), page.wait_for_timeout(120))
        toast = lambda: page.evaluate("document.querySelector('.toast')?.textContent || ''")

        def shot(name):
            if SHOTS:
                SHOTS.mkdir(parents=True, exist_ok=True)
                page.screenshot(path=str(SHOTS / f"{name}.png"))

        # ------------------------------------------------------------------
        print("Startup and library")
        check("music:ready" in sent(), f"tells the app it is ready ({sent()})")
        check(page.is_visible("#library-empty"), "empty library: invites to add music")
        check(page.inner_text("#library-empty h2") == "Your library is empty", "in English")
        check(page.title() == "My music" and page.evaluate("document.documentElement.lang") == "en", "title and <html lang>")
        shot("1-empty")
        clear()
        page.click("#empty-add")
        check(sent() == ["music:pick-folder"], f"“Add a music folder” asks the app for the dialog ({sent()})")

        # A message with another key does nothing.
        page.evaluate("window.__ytmpMusic('fake-key', { type: 'library', tracks: [{ id: 'x', title: 'fake' }] })")
        page.wait_for_timeout(100)
        check(page.locator(".row").count() == 0, "a message without the app's key is ignored")

        push({"type": "library", "tracks": TRACKS, "playlists": [], "folders": ["C:\\Users\\Ana\\Music"], "quality": "compact", "mediaKey": KEY})
        push(FREE)
        rows = page.locator("#library-list .row")
        check(rows.count() == 3, f"shows the 3 songs ({rows.count()})")
        check(rows.nth(0).locator(".title").inner_text() == "Supernova", "most recent first")
        check(page.locator("#library-list img").count() >= 1, "with a cover when there is one")
        check(page.evaluate("document.querySelectorAll('img[src=x]').length") == 0 and page.locator("text=<img src=x").count() == 1,
              "tags are shown as text, never as HTML")
        page.fill("#filter", "cancion numero")
        page.wait_for_timeout(80)
        check(rows.count() == 1 and "Dos" in rows.nth(0).inner_text(), "the filter ignores accents and case")
        page.fill("#filter", "")
        page.select_option("#sort", "artist")
        page.wait_for_timeout(80)
        check(rows.nth(0).locator(".title").inner_text() == "Canción Número Dos", "sorting by artist (Álvaro first)")
        page.select_option("#sort", "added")
        check(page.is_visible("#folders"), "folder summary at the end")
        summary = page.inner_text("#folders-summary")
        check(summary.startswith("3 songs ·") and summary.endswith("· 1 folder"), f"with plurals ({summary})")
        push({"type": "scan", "running": False, "added": 3, "removed": 0,
              "messages": [{"key": "music.scan.new", "args": {"count": 3}}, {"key": "music.scan.unreadable", "args": {"count": 1}}]})
        check(toast() == "3 new songs. 1 file couldn't be read.", f"scan messages, one after the other ({toast()})")
        shot("2-library")

        # ------------------------------------------------------------------
        print("Playback")
        clear()
        rows.nth(1).click()
        page.wait_for_timeout(700)
        src = page.evaluate("document.getElementById('audio').getAttribute('src')")
        check(src == f"/media/bbbbbbbbbbbbbbb2?k={KEY}", f"asks for the song with the session key ({src})")
        check(page.evaluate("!document.getElementById('audio').paused"), "it plays")
        check(page.inner_text("#now-title") == "Canción Número Dos", "the player shows the song")
        check(page.get_attribute("#play", "title") == "Pause", "the play button says Pause")
        check(page.locator("#library-list .row.playing").count() == 1, "the playing row is highlighted")
        meta = page.evaluate("({ t: navigator.mediaSession.metadata && navigator.mediaSession.metadata.title, s: navigator.mediaSession.playbackState })")
        check(meta == {"t": "Canción Número Dos", "s": "playing"}, f"Media Session up to date ({meta})")
        page.evaluate("window.__ytmpMediaReport()")
        page.wait_for_timeout(200)
        st = states()
        last = st[-1] if st else {}
        check(last.get("site") == "ytmp.localhost" and last.get("title") == "Canción Número Dos" and last.get("canNext"),
              f"media.js tells the app (taskbar music bar): {last.get('title')!r}, {last.get('site')!r}")
        # The music bar / tray / global shortcuts: the "next" action.
        page.evaluate("window.__ytmpMedia('next')")
        page.wait_for_timeout(500)
        check(page.inner_text("#now-title") == "<img src=x onerror=alert(1)>", "“next” from the app goes to the next one")
        page.evaluate("window.__ytmpMedia('toggle')")
        page.wait_for_timeout(200)
        check(page.evaluate("document.getElementById('audio').paused"), "and “play/pause” pauses it")
        page.click("#repeat")
        check(page.get_attribute("#repeat", "aria-pressed") == "true" and page.get_attribute("#repeat", "title") == "Repeat: all",
              "repeat turns on and says how")
        page.click("#shuffle")
        check(page.get_attribute("#shuffle", "aria-pressed") == "true", "shuffle turns on")
        shot("3-playing")

        # ------------------------------------------------------------------
        print("A song's menu")
        clear()
        rows.nth(0).locator(".more").click()
        page.click(".menu >> text=Show in folder")
        check(sent() == ["music:reveal:aaaaaaaaaaaaaaa1"], f"show in folder ({sent()})")
        rows.nth(0).locator(".more").click()
        check(page.is_visible(".menu >> text=License: CC BY-SA"), "downloaded songs show their license")
        page.click(".menu >> text=Add to a playlist")
        check(page.is_visible(".dialog >> text=Add to a playlist") and page.is_enabled(".dialog >> text=New playlist"),
              "on the Free plan playlists can be created (up to 2)")
        page.keyboard.press("Escape")
        # A plan limit: a quiet line under the header, no windows.
        push({"type": "notice", "feature": "playlists", "messages": [{"key": "music.limit.playlists_free", "args": {"count": 2, "next": 5}}]})
        check(page.is_visible("#notice") and page.inner_text("#notice-text") == text("en", "music.limit.playlists_free", count=2, next=5),
              f"limits are shown on one line ({page.inner_text('#notice-text')})")
        check(page.locator(".scrim").count() == 0, "with no windows that interrupt")
        page.click("#notice-plans")
        check(page.get_attribute("[data-tab='plan']", "aria-selected") == "true" and page.is_hidden("#notice"),
              "“See plans” goes to the Plan tab and closes the notice")
        push({"type": "notice", "feature": "download", "messages": [{"key": "music.limit.download"}]})
        page.click("#notice-close")
        check(page.is_hidden("#notice"), "the notice closes with ×")
        page.click("[data-tab='library']")
        clear()
        rows.nth(2).locator(".more").click()
        page.click(".menu >> text=Move to Recycle Bin")
        check(sent() == [] and page.inner_text(".dialog h2") == "Move to the Recycle Bin?", "the Recycle Bin asks first")
        page.click(".dialog >> .btn.primary")
        check(sent() == ['music:remove:{"ids":["ccccccccccccccc3"],"trash":true}'], f"and then asks the app ({sent()})")

        # ------------------------------------------------------------------
        print("Plan and license")
        page.click("[data-tab='plan']")
        page.wait_for_timeout(100)
        check(page.locator(".plan").count() == 3, "three plans")
        check(page.locator(".plan.current h3").inner_text() == "Free", "marks the current plan")
        plans_text = page.inner_text(".plans")
        check("$2" in plans_text and "$10" in plans_text and "/month" in plans_text, f"with their prices, written the English way")
        check("2 playlists" in plans_text and "5 playlists" in plans_text and "Unlimited playlists" in plans_text, "and their playlists (2 / 5 / unlimited)")
        check("nothing you made is deleted" in page.inner_text("#plan-body"), "explains that changing plans deletes nothing")
        clear()
        page.click(".plan:has-text('Lifetime') >> button")
        check(sent() == ["music:buy:lifetime"], f"buying opens the payment page ({sent()})")
        clear()
        page.fill(".license-form input", "38B1460A-5104-4067-A91D-77B872934D51")
        page.click(".license-form button")
        check(sent() == ["music:license-activate:38B1460A-5104-4067-A91D-77B872934D51"], f"activating sends the key ({sent()})")
        check(page.input_value(".license-form input") == "", "the key does not stay on the page")
        push(dict(FREE, busy=True))
        check(page.is_disabled(".license-form button") and page.inner_text(".license-form button") == "Checking…", "while checking, the button waits")
        push(LIFETIME)
        check(page.locator(".plan.current h3").inner_text() == "Lifetime", "with a license, the plan changes")
        body = page.inner_text("#plan-body")
        check("****-4D51" in body and "a***@example.com" in body and "Active" in body, "shows the masked license and its status")
        check(page.inner_text(".msg") == "Done! The Lifetime plan is active on this PC.", f"and the store's message ({page.inner_text('.msg')})")
        shot("4-plan")
        push(dict(FREE, configured=False))
        check("has no store set up" in page.inner_text("#plan-body"), "warns if the build has no store")
        push(dict(LIFETIME, dev=True, message=None))
        check("Development build" in page.inner_text("#plan-body"), "and if it is a development build")

        # ------------------------------------------------------------------
        print("Download (Internet Archive)")
        push(FREE)
        page.click("[data-tab='download']")
        check("Internet Archive" in page.inner_text("#download-body") and "Openly licensed" in page.inner_text("#download-body"),
              "explains where the music comes from and its license")
        clear()
        page.fill("#search", "ronan dec")
        page.press("#search", "Enter")
        check(sent() == ['music:search:{"q":"ronan dec","page":1}'], f"search ({sent()})")
        push({"type": "search", "query": "ronan dec", "page": 1, "total": 31, "hits": [
            {"id": "TAM033-Ronan_Dec", "title": "Ronan Dec. - Hirnwald EP [TAM033]", "creator": "Ronan Dec.", "license": "BySa",
             "licenseLabel": "CC BY-SA", "downloads": 191994, "year": "2008"},
            {"id": "pd-album", "title": "Old recordings", "creator": "Someone", "license": "PublicDomain",
             "licenseLabel": "Public domain", "downloads": 10, "year": "1920"}]})
        check(page.locator(".hit").count() == 2 and "CC BY-SA" in page.inner_text(".hit >> nth=0"), "results with their license")
        check("31 openly licensed albums for “ronan dec”" in page.inner_text("#download-body"), "the count, with the language's quotes")
        check(page.is_visible(".pager") and "Page 1 of 2" in page.inner_text(".pager"), "with pages")
        push({"type": "search", "query": "ronan dec", "page": 1, "error": {"key": "net.status", "args": {"code": 503}}})
        check("Couldn't search: the server answered with an error (503)" in page.inner_text("#download-body"),
              f"an error from the app, in words ({page.inner_text('#download-body')[:80]})")
        push({"type": "search", "query": "ronan dec", "page": 1, "total": 31, "hits": [
            {"id": "TAM033-Ronan_Dec", "title": "Ronan Dec. - Hirnwald EP [TAM033]", "creator": "Ronan Dec.", "license": "BySa",
             "licenseLabel": "CC BY-SA", "downloads": 191994, "year": "2008"}]})
        clear()
        page.click(".hit")
        check(sent() == ["music:item:TAM033-Ronan_Dec"], f"opening an album ({sent()})")
        push(ITEM)
        tracks = page.locator(".track")
        check(tracks.count() == 3, "the album's songs")
        check("You have it" in tracks.nth(0).inner_text(), "marks the one you have")
        check(tracks.nth(0).locator(".badge").get_attribute("title") == "already downloaded", "and says why")
        check("Credit the artist; if you change it" in page.inner_text(".album-head"), "explains the license terms")
        check("1 is already in your library" in page.inner_text("#download-body"), "counts what won't be downloaded again")
        check(tracks.nth(1).locator(".btn").count() == 0 and "To download them" in page.inner_text(".album-actions")
              and page.locator(".scrim").count() == 0, "without a plan: no download buttons or windows; a note and the free preview")
        # Online preview (free).
        tracks.nth(2).locator(".preview").click()
        page.wait_for_timeout(500)
        audio_src = page.evaluate("document.getElementById('audio').src")
        check(audio_src.startswith("https://archive.org/download/TAM033-Ronan_Dec/"), f"preview from the Archive ({audio_src})")
        check("Preview" in page.inner_text("#now-sub"), "the player says it is a preview")
        push(LIFETIME)
        clear()
        button = page.inner_text(".album-actions .btn.primary")
        check("Download album as a playlist (2 songs," in button and "MB)" in button, f"the button counts songs and size ({button})")
        page.click(".album-actions .btn.primary")
        msg = sent()
        check(len(msg) == 1 and msg[0].startswith("music:download:"), f"downloading the album ({msg})")
        if msg:
            args = json.loads(msg[0][len("music:download:"):])
            check(args["playlist"] is True and len(args["files"]) == 3, "as a playlist, the whole album (the app skips repeats)")
        push({"type": "toast", "messages": [{"key": "music.download.queued", "args": {"count": 2}}, {"key": "music.download.skipped", "args": {"count": 1}}]})
        check(toast() == "2 downloads on the way. You already had 1, so it takes no extra space.", f"what was queued and skipped ({toast()})")
        jobs = {"type": "downloads", "jobs": [
            {"id": 1, "item": "TAM033-Ronan_Dec", "file": "a", "title": "Supernova", "artist": "ronan dec", "state": "skipped", "received": 0, "size": 1,
             "message": {"key": "music.existing.same_download"}, "trackId": "aaaaaaaaaaaaaaa1"},
            {"id": 2, "item": "TAM033-Ronan_Dec", "file": "b", "title": "Waehrend du schliefst", "artist": "ronan dec", "state": "downloading", "received": 6004131, "size": 12008263, "message": None, "trackId": None},
            {"id": 3, "item": "TAM033-Ronan_Dec", "file": "c", "title": "Para", "artist": "ronan dec", "state": "queued", "received": 0, "size": 8248258, "message": None, "trackId": None},
            {"id": 4, "item": "TAM033-Ronan_Dec", "file": "d", "title": "Broken", "artist": "ronan dec", "state": "failed", "received": 0, "size": 1,
             "message": {"key": "net.corrupt"}, "trackId": None}]}
        push(jobs)
        check(page.is_visible("#jobs") and page.inner_text("#jobs-title") == "Downloading 2…", "download queue visible")
        jobs_text = page.inner_text("#jobs-list")
        check("50%" in jobs_text and "already downloaded" in jobs_text and "Failed: the file arrived damaged" in jobs_text,
              "with the progress and each job's reason")
        check(page.inner_text("#jobs-count") == "2", "and the counter on the tab")
        clear()
        page.click("#jobs-cancel")
        check(sent() == ["music:cancel:all"], "cancel all")
        shot("5-downloads")

        # ------------------------------------------------------------------
        print("Duplicates")
        page.click("[data-tab='dupes']")
        check("acoustic fingerprint" in page.inner_text("#dupes-status"), "explains how it looks")
        clear()
        page.click("#dupes-run")
        check(sent() == ["music:dupes"], "find duplicates")
        push({"type": "dupes", "running": True, "done": 3, "total": 10})
        check("3 of 10" in page.inner_text("#dupes-status"), "shows the progress")
        push({"type": "dupes", "running": False, "wasted": 8438638, "groups": [
            {"reason": "same-audio", "tracks": ["bbbbbbbbbbbbbbb2", "aaaaaaaaaaaaaaa1"], "wasted": 8438638}]})
        check(page.locator(".group").count() == 1 and "Same audio" in page.inner_text(".group"), "a group with its reason")
        check("Best quality" in page.locator(".dupe").nth(0).inner_text() and "256 kbps" in page.inner_text(".group"), "suggests keeping the best one")
        check(page.inner_text("#dupes-status").startswith("1 group of repeated songs"), f"singular for one group ({page.inner_text('#dupes-status')})")
        page.locator(".dupe input").nth(1).check()
        clear()
        page.click(".group >> text=Recycle Bin for the rest")
        page.click(".dialog >> .btn.primary")
        check(sent() == ['music:remove:{"ids":["bbbbbbbbbbbbbbb2"],"trash":true}'], f"sends the ones not kept to the Recycle Bin ({sent()})")
        shot("6-duplicates")

        # ------------------------------------------------------------------
        print("Playlists")
        push({"type": "library", "tracks": TRACKS, "folders": [], "quality": "compact", "playlists": [
            {"id": "a1b2c3d4e5f60708", "name": "For running", "tracks": ["aaaaaaaaaaaaaaa1", "bbbbbbbbbbbbbbb2"], "created": 1, "updated": 1, "origin": None}]})
        page.click("[data-tab='playlists']")
        check(page.locator(".playlist-row").count() == 1, "list of playlists")
        page.click(".playlist-row")
        check(page.locator("#playlists-body .row").count() == 2, "opens the playlist")
        clear()
        page.fill(".pl-head input", "For running fast")
        page.press(".pl-head input", "Tab")
        check(sent() == ['music:playlist-save:{"id":"a1b2c3d4e5f60708","name":"For running fast","tracks":["aaaaaaaaaaaaaaa1","bbbbbbbbbbbbbbb2"]}'],
              f"renaming saves the playlist ({sent()})")
        clear()
        page.click(".pl-head >> text=Play")
        page.wait_for_timeout(400)
        check(page.inner_text("#now-title") == "Supernova", "plays the playlist from the start")
        page.click(".pl-head .btn.icon")
        push(dict(FREE, playlists=1))
        check("1 of 2 playlists" in page.inner_text("#playlists-body") and page.is_enabled("#playlists-body >> text=New playlist"),
              "Free plan: 1 of 2 playlists, another can be created")
        two = [{"id": "a1b2c3d4e5f60708", "name": "For running", "tracks": ["aaaaaaaaaaaaaaa1"], "created": 1, "updated": 1, "origin": None},
               {"id": "b1b2c3d4e5f60709", "name": "For studying", "tracks": ["bbbbbbbbbbbbbbb2"], "created": 2, "updated": 2, "origin": None}]
        push({"type": "library", "tracks": TRACKS, "folders": [], "quality": "compact", "playlists": two})
        body = page.inner_text("#playlists-body")
        check("2 of 2" in body and page.is_disabled("#playlists-body >> text=New playlist") and page.is_visible("text=More playlists"),
              "at 2 of 2, “New playlist” is off and “More playlists” shows up")
        page.click(".playlist-row >> nth=1")
        clear()
        page.fill(".pl-head input", "Study")
        page.press(".pl-head input", "Tab")
        check(len(sent()) == 1 and '"name":"Study"' in sent()[0], "existing ones can still be edited")
        page.click(".pl-head .btn.icon")
        push(LIFETIME)
        check("no limit" in page.inner_text("#playlists-body") and page.is_enabled("#playlists-body >> text=New playlist"),
              "Lifetime: no limit")
        check(page.locator(".scrim").count() == 0, "no window was ever left open")
        shot("7-playlists")

        # ------------------------------------------------------------------
        print("Language")
        push(FREE)
        push({"type": "library", "tracks": TRACKS, "folders": ["C:\\a", "C:\\b"], "quality": "compact", "playlists": two})
        push({"type": "notice", "feature": "playlists", "messages": [{"key": "music.limit.playlists_free", "args": {"count": 2, "next": 5}}]})
        page.click("[data-tab='library']")
        page.fill("#filter", "")
        push_locale(page, "ru", "music")
        page.wait_for_timeout(200)
        check(page.inner_text("[data-tab='library'] .label") == "Библиотека" and page.evaluate("document.documentElement.lang") == "ru",
              "the page switches to Russian without reloading")
        check(page.inner_text("#notice-text") == text("ru", "music.limit.playlists_free", count=2, next=5),
              f"the notice on show is rewritten ({page.inner_text('#notice-text')[:40]}…)")
        summary = page.inner_text("#folders-summary")
        check(summary.startswith("3 песни ·") and summary.endswith("· 2 папки"), f"Russian plurals ({summary})")
        check(" МБ" in summary, f"sizes in Russian units ({summary})")
        check(page.get_attribute("#filter", "placeholder") == text("ru", "music.filter"), "placeholders too")
        push({"type": "scan", "running": True, "done": 1234, "total": 5678})
        check(page.inner_text("#scan-text") == "Проверяем вашу музыку… 1\u00a0234 из 5\u00a0678", f"numbers grouped the Russian way ({page.inner_text('#scan-text')!r})")
        push({"type": "scan", "running": False, "added": 22, "removed": 0,
              "messages": [{"key": "music.scan.new", "args": {"count": 22}}, {"key": "music.scan.unreadable", "args": {"count": 5}}]})
        check(toast() == "22 новые песни. Не удалось прочитать 5 файлов.", f"22 → «новые песни», 5 → «файлов» ({toast()})")
        shot("8-ru")
        page.click("[data-tab='playlists']")
        check("Плейлистов: 2 из 2 · тариф «Бесплатный»" in page.inner_text("#playlists-body"), f"the quota in Russian ({page.inner_text('#playlists-body')[:60]})")
        page.click("[data-tab='plan']")
        check("2 $" in page.inner_text(".plans") or "2\u00a0$" in page.inner_text(".plans"), f"prices written the Russian way ({page.inner_text('.plans')[:60]!r})")
        shot("9-ru-plan")

        push_locale(page, "pl", "music")
        page.wait_for_timeout(150)
        page.click("[data-tab='download']")
        push({"type": "toast", "messages": [{"key": "music.download.queued", "args": {"count": 3}}]})
        check(toast() == "Dodano do pobierania: 3.", f"Polish ({toast()})")
        page.click("[data-tab='dupes']")
        push({"type": "dupes", "running": False, "wasted": 1, "groups": [
            {"reason": "same-song", "tracks": ["bbbbbbbbbbbbbbb2", "aaaaaaaaaaaaaaa1"], "wasted": 1},
            {"reason": "same-file", "tracks": ["aaaaaaaaaaaaaaa1", "bbbbbbbbbbbbbbb2"], "wasted": 1}]})
        check(page.inner_text("#dupes-status").startswith("2 grupy powtórzonych utworów"), f"Polish plural (2 → grupy) ({page.inner_text('#dupes-status')})")
        shot("10-pl-dupes")

        push_locale(page, "ja", "music")
        page.wait_for_timeout(150)
        page.click("[data-tab='plan']")
        check("$2" in page.inner_text(".plans") and page.locator(".plan.current h3").inner_text() == "無料", "Japanese plan names and prices")
        push({"type": "toast", "messages": [{"key": "music.download.queued", "args": {"count": 2}}, {"key": "music.download.skipped", "args": {"count": 1}}]})
        check(toast() == "2 曲をダウンロードしています。1 曲は入手済みのため、容量は増えません。", f"Japanese sentences joined without a space ({toast()})")
        shot("11-ja-plan")
        page.click("[data-tab='library']")
        shot("12-ja-library")

        # ------------------------------------------------------------------
        print("Small windows")
        push_locale(page, "en", "music")
        for w, h in [(480, 270), (256, 144)]:
            page.set_viewport_size({"width": w, "height": h})
            page.wait_for_timeout(150)
            fits = page.evaluate("""() => {
                const r = document.getElementById('player').getBoundingClientRect();
                return r.bottom <= innerHeight + 1 && r.right <= innerWidth + 1 && document.documentElement.scrollWidth <= innerWidth;
            }""")
            check(fits, f"{w}×{h}: the player shows whole, with no sideways scrolling")
            check(page.is_visible("#play"), f"{w}×{h}: the play button stays at hand")
            shot(f"13-{w}x{h}")

        check(all("/media/" in u and f"k={KEY}" in u for u in media_requests) and media_requests, "every song is requested with the key")
        check(not errors, f"no console or CSP errors ({errors[:3]})")
        browser.close()

    print()
    if failures:
        print(f"{len(failures)} test(s) failed")
        sys.exit(1)
    print("My music: all tests pass")


if __name__ == "__main__":
    main()
