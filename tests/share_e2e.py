"""End-to-end test of window sharing in Chromium, with real WebRTC.

Two pages in one browser play the two apps:
- the hidden page that sends the host's window (src/cast.html), fed the
  way cast.rs feeds it: JPEG pictures and 48 kHz stereo sound;
- the page that shows it in a guest's player (src/stream.html), with the
  app's own wrapper (webview.rs' bundle) as in the main window.

The test passes the offer and the answer between them the way the room
does, then checks that the guest sees the picture and hears the sound, and
that its mouse and keys reach the host only while it may use the window
(and that shortcuts stay on its own PC). Then the page's screens in the
five languages, at the player's smallest and a big size.

Usage:  python tests/share_e2e.py [folder_for_screenshots]
"""

import base64
import io
import json
import math
import pathlib
import struct
import sys
import time

from PIL import Image, ImageDraw
from playwright.sync_api import sync_playwright

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from bundle import HOST_KEY, LANGUAGES, SRC, add_app_scripts, fill, i18n, text  # noqa: E402

SHOTS = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else None
CSP = ("default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; "
       "img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'")
CAST_IPC = """
window.__sent = [];
Object.defineProperty(window, 'ipc', { value: Object.freeze({ postMessage: (m) => window.__sent.push(String(m)) }) });
"""
W, H = 640, 360
LEFT = (0, 180, 216)     # turquoise
RIGHT = (255, 159, 138)  # coral


def jpeg(box_x):
    image = Image.new("RGB", (W, H), LEFT)
    draw = ImageDraw.Draw(image)
    draw.rectangle([W // 2, 0, W, H], fill=RIGHT)
    draw.rectangle([box_x, 140, box_x + 60, 220], fill=(3, 4, 94))
    out = io.BytesIO()
    image.save(out, "JPEG", quality=80)
    return base64.b64encode(out.getvalue()).decode()


def jpeg_hd(i):
    """A 1280×720 picture with plenty of detail (like a real page)."""
    image = Image.new("RGB", (1280, 720), (20, 30, 60))
    draw = ImageDraw.Draw(image)
    for k in range(80):
        x, y = (k * 97 + i * 31) % 1280, (k * 53 + i * 17) % 720
        draw.ellipse([x, y, x + 140, y + 90], fill=((k * 37) % 255, (k * 71 + i * 9) % 255, (k * 13) % 255))
    out = io.BytesIO()
    image.save(out, "JPEG", quality=72)
    return base64.b64encode(out.getvalue()).decode()


def sound_chunks(seconds=1.0, freq=440.0):
    """20 ms pieces of a sine, s16le stereo, as loopback.rs sends them."""
    chunks = []
    frames = 960
    for c in range(int(seconds * 50)):
        data = bytearray()
        for i in range(frames):
            v = int(0.3 * 32767 * math.sin(2 * math.pi * freq * (c * frames + i) / 48000))
            data += struct.pack("<hh", v, v)
        chunks.append(base64.b64encode(bytes(data)).decode())
    return chunks


def main():
    failures = []

    def check(cond, name):
        print(("  ok   " if cond else "  FAIL ") + name)
        if not cond:
            failures.append(name)

    frames = [jpeg(x) for x in range(40, 560, 40)]
    chunks = sound_chunks()

    with sync_playwright() as p:
        browser = p.chromium.launch(args=[
            "--autoplay-policy=no-user-gesture-required",
            # Real addresses between the two pages of this machine.
            "--disable-features=WebRtcHideLocalIpsWithMdns",
        ])
        errors = []
        lang = {"code": "en"}

        def route(r):
            path = r.request.url.split("ytmp.localhost", 1)[1].split("?")[0]
            if path == "/cast":
                body = (SRC / "cast.html").read_text(encoding="utf-8")
            elif path == "/stream":
                body = fill("stream.html", {"i18n": i18n(lang["code"], "stream")}, lang["code"])
            else:
                return r.fulfill(status=404, body="")
            r.fulfill(status=200, content_type="text/html; charset=utf-8", headers={"Content-Security-Policy": CSP}, body=body)

        context = browser.new_context()
        context.route("https://ytmp.localhost/**", route)
        cast = context.new_page()
        cast.set_viewport_size({"width": 320, "height": 240})
        cast.add_init_script(CAST_IPC)
        viewer = context.new_page()
        viewer.set_viewport_size({"width": 800, "height": 600})
        add_app_scripts(viewer, maintenance=False)
        for page in (cast, viewer):
            page.on("console", lambda m: m.type == "error" and errors.append(m.text))
            page.on("pageerror", lambda e: errors.append(str(e)))

        cast_sent = lambda: cast.evaluate("window.__sent.slice()")
        viewer_sent = lambda: viewer.evaluate("window.__sent.slice()")
        order = lambda message: cast.evaluate("(m) => window.__ytmpPost(m)", json.dumps(message))
        tell = lambda message: viewer.evaluate("([k, m]) => window.__ytmpStream(k, m)", [HOST_KEY, message])

        def wait(fn, what, timeout=15):
            deadline = time.time() + timeout
            while time.time() < deadline:
                value = fn()
                if value:
                    return value
                time.sleep(0.05)
            return None

        # --- The sending page -------------------------------------------------
        print("The page that sends the window")
        cast.goto("https://ytmp.localhost/cast")
        caps = wait(lambda: [m for m in cast_sent() if m.startswith("cast:caps:")], "caps")
        check(bool(caps) and json.loads(caps[0][len("cast:caps:"):]) == {"generator": True, "audio": True},
              f"says it can send picture and sound ({caps})")
        check("cast:ready" in cast_sent(), "says it is ready")
        # Pictures (15 a second, a moving box) and sound, as cast.rs does.
        cast.evaluate("""([frames, chunks]) => {
          let f = 0, c = 0;
          window.__pictures = setInterval(() => window.__ytmpPost('f' + frames[f++ % frames.length]), 66);
          setInterval(() => window.__ytmpPost('a' + chunks[c++ % chunks.length]), 20);
        }""", [frames, chunks])

        # --- The guest's page ------------------------------------------------------
        print("The guest's page")
        viewer.goto("https://ytmp.localhost/stream")
        check(wait(lambda: "stream:ready" in viewer_sent(), "ready") is not None, "asks for the window when it loads")
        tell({"t": "state", "host": "Rodrigo", "control": False, "paused": False, "hostAway": False})
        check(viewer.title() == text("en", "stream.title", name="Rodrigo"), f"title ({viewer.title()})")
        check(viewer.is_visible("#spinner") and viewer.inner_text("#message") == text("en", "stream.connecting", name="Rodrigo"),
              "connecting…")
        if SHOTS:
            SHOTS.mkdir(parents=True, exist_ok=True)
            viewer.screenshot(path=str(SHOTS / "stream-connecting.png"))

        # The room: the host's page makes a connection, the offer and answer
        # travel (here, through the test).
        order({"t": "peer", "id": "g1", "ice": [], "bitrate": 2500000})
        offer = wait(lambda: [m for m in cast_sent() if m.startswith("cast:offer:g1:")], "offer")
        check(bool(offer), "an offer for g1")
        sdp = offer[0][len("cast:offer:g1:"):]
        check(len(sdp) < 10_000, f"small enough to travel sealed ({len(sdp)} bytes)")
        codecs = [line.split(" ", 1)[1].split("/")[0] for line in sdp.split("\r\n") if line.startswith("a=rtpmap:")]
        check(set(codecs) <= {"H264", "VP8", "rtx", "opus"}, f"few codecs ({codecs})")
        check("a=sendonly" in sdp and "webrtc-datachannel" in sdp, "send only, with the input channel")
        tell({"t": "offer", "sdp": sdp, "ice": []})
        answer = wait(lambda: [m for m in viewer_sent() if m.startswith("stream:answer:")], "answer")
        check(bool(answer), "the guest's page answers")
        order({"t": "answer", "id": "g1", "sdp": answer[0][len("stream:answer:"):]})

        check(wait(lambda: "cast:state:g1:connected" in cast_sent(), "connected") is not None, "connected (host side)")
        check(wait(lambda: "stream:state:watching" in viewer_sent(), "watching") is not None, "watching (guest side)")
        # WebRTC starts small and grows as it measures the connection.
        size_of = lambda: viewer.evaluate("[document.getElementById('video').videoWidth, document.getElementById('video').videoHeight]")
        wait(lambda: size_of() == [W, H], "full size", 20)
        size = size_of()
        check(size == [W, H], f"the picture's full size, once the connection is measured ({size})")
        viewer.wait_for_timeout(1500)
        colors = viewer.evaluate("""() => {
          const v = document.getElementById('video');
          const c = document.createElement('canvas');
          c.width = v.videoWidth; c.height = v.videoHeight;
          const g = c.getContext('2d');
          g.drawImage(v, 0, 0);
          const at = (x, y) => Array.from(g.getImageData(x, y, 1, 1).data.slice(0, 3));
          return [at(60, 40), at(c.width - 60, 40)];
        }""")
        close = lambda a, b: all(abs(x - y) <= 40 for x, y in zip(a, b))
        check(close(colors[0], LEFT) and close(colors[1], RIGHT), f"the host's picture, as drawn ({colors})")
        level = viewer.evaluate("""async () => {
          const v = document.getElementById('video');
          const ctx = new AudioContext();
          await ctx.resume();
          const src = ctx.createMediaStreamSource(v.srcObject);
          const an = ctx.createAnalyser();
          an.fftSize = 4096;
          src.connect(an);
          let best = { rms: 0, peak: 0 };
          for (let i = 0; i < 20; i++) {
            await new Promise((r) => setTimeout(r, 100));
            const t = new Float32Array(an.fftSize);
            an.getFloatTimeDomainData(t);
            const rms = Math.sqrt(t.reduce((s, x) => s + x * x, 0) / t.length);
            const f = new Float32Array(an.frequencyBinCount);
            an.getFloatFrequencyData(f);
            let bin = 0;
            for (let k = 1; k < f.length; k++) if (f[k] > f[bin]) bin = k;
            if (rms > best.rms) best = { rms, peak: bin * ctx.sampleRate / an.fftSize };
          }
          return best;
        }""")
        check(level["rms"] > 0.05 and abs(level["peak"] - 440) < 30, f"the host's sound, as played ({level})")

        # The numbers both pages send for the diary (every 10 s; here at once).
        cast.evaluate("window.__ytmpCastStats()")
        viewer.wait_for_timeout(1200)
        host = cast.evaluate("window.__ytmpCastStats()")
        guest = viewer.evaluate("window.__ytmpStreamStats()")
        peer = (host or {}).get("peers", [{}])[0]
        check(host["in"] > 5 and host["out"] > 5 and host["sound"] > 30, f"host: pictures and sound go through the page ({host})")
        check(peer.get("id") == "g1" and peer.get("state") == "connected" and peer.get("fps", 0) > 5 and peer.get("kbps", 0) > 50
              and peer.get("audioKbps", 0) > 10 and peer.get("route", "").count("-") == 1,
              f"host: what goes to the guest, with no addresses ({peer})")
        check(guest and guest["fps"] > 5 and guest.get("level", 0) > 0.05 and guest["muted"] is False and guest["paused"] is False,
              f"guest: pictures and sound arrive, not muted ({guest})")
        stats_sent = [m for m in cast_sent() if m.startswith("cast:stats:")] + [m for m in viewer_sent() if m.startswith("stream:stats:")]
        check(stats_sent and all(len(m) < 4096 for m in stats_sent) and not any("192.168." in m or "127.0." in m for m in stats_sent),
              "the reports are small and carry no addresses")
        check(viewer.is_visible("#live") and viewer.is_hidden("#center"), "live, nothing in the way")
        check(viewer.is_visible("#ask") and viewer.is_hidden("#using"), "may ask to use it")
        if SHOTS:
            viewer.mouse.move(400, 300)
            viewer.screenshot(path=str(SHOTS / "stream-watching.png"))

        # --- Using the host's window ----------------------------------------------
        print("Mouse and keyboard")
        inputs = lambda: [json.loads(m.split(":", 3)[3]) for m in cast_sent() if m.startswith("cast:input:g1:")]
        viewer.mouse.click(200, 187)
        viewer.keyboard.press("a")
        viewer.wait_for_timeout(300)
        check(inputs() == [], "not allowed: nothing reaches the host")
        before = len(viewer_sent())
        viewer.click("#ask")
        check("stream:ask" in viewer_sent()[before:] and viewer.is_disabled("#ask"), "Ask to use it, once")

        tell({"t": "state", "host": "Rodrigo", "control": True, "paused": False, "hostAway": False})
        check(viewer.is_visible("#using") and viewer.is_hidden("#ask"), "allowed: it says so")
        check(viewer.get_attribute("#input", "data-ytmp-nodrag") is not None, "a press is for the host, not to move this window")
        # Letterboxed: 640×360 in 800×600 is 800×450 from y = 75.
        viewer.mouse.move(200, 75 + 450 * 0.25)
        viewer.mouse.down()
        viewer.mouse.up()
        viewer.mouse.wheel(0, 120)
        viewer.keyboard.press("a")
        viewer.keyboard.press("Shift+B")
        viewer.keyboard.press("Enter")
        viewer.wait_for_timeout(400)
        got = inputs()
        down = next((i for i in got if i.get("e") == "down"), None)
        check(down is not None and abs(down["x"] - 0.25) < 0.01 and abs(down["y"] - 0.25) < 0.01,
              f"a click lands where it was made on the picture ({down})")
        check(any(i.get("e") == "up" for i in got) and any(i.get("e") == "wheel" and i["dy"] > 0 for i in got), "release and wheel")
        keys = [i["key"] for i in got if i.get("t") == "k" and i.get("e") == "down"]
        check(keys == ["a", "B", "Enter"], f"keys, Shift travelling with them ({keys})")
        check(next(i for i in got if i.get("key") == "B")["s"] is True, "…held for B")
        count = len(inputs())
        before = len(viewer_sent())
        viewer.keyboard.press("Control+t")
        viewer.wait_for_timeout(300)
        check(len(inputs()) == count, "Ctrl shortcuts stay on the guest's PC")
        check("pin" in viewer_sent()[before:], "…where the player's own shortcut works")
        viewer.mouse.click(790, 300)
        viewer.wait_for_timeout(200)
        check(all(i.get("x", 0) <= 1 for i in inputs()), "only points on the picture")

        order({"t": "busy", "id": "g1", "by": "Ana"})
        check(wait(lambda: viewer.is_visible("#hint"), "hint", 5) and viewer.inner_text("#hint") == text("en", "stream.busy", name="Ana"),
              "someone else is using it: said")
        order({"t": "revoked", "id": "g1"})
        viewer.wait_for_timeout(300)
        count = len(inputs())
        viewer.mouse.click(400, 300)
        viewer.keyboard.press("x")
        viewer.wait_for_timeout(300)
        check(len(inputs()) == count and viewer.is_visible("#ask"), "taken back: nothing more goes, Ask again")

        # --- Paused, the host away, leaving ------------------------------------------
        tell({"t": "state", "host": "Rodrigo", "control": False, "paused": True, "hostAway": False})
        check(viewer.inner_text("#message") == text("en", "stream.paused", name="Rodrigo") and viewer.is_visible("#mark"),
              "paused by the host")
        if SHOTS:
            viewer.screenshot(path=str(SHOTS / "stream-paused.png"))
        tell({"t": "state", "host": "Rodrigo", "control": False, "paused": False, "hostAway": True})
        check(viewer.inner_text("#message") == text("en", "stream.host_away", name="Rodrigo"), "the host's connection dropped")
        tell({"t": "state", "host": "Rodrigo", "control": False, "paused": False, "hostAway": False})

        before = len(viewer_sent())
        viewer.click("#people")
        viewer.click("#leave")
        check(viewer_sent()[before:] == ["together", "stream:leave"], f"Watch together and Leave ({viewer_sent()[before:]})")

        # The host's page closes the connection: the guest tries again.
        before = len(viewer_sent())
        order({"t": "close", "id": "g1"})
        again = wait(lambda: "stream:ready" in viewer_sent()[before:], "retry", 60)
        check(again is not None, "the connection ends: the guest asks again by itself")
        check("stream:state:failed" in viewer_sent()[before:], "…saying it failed meanwhile")
        if SHOTS:
            order({"t": "close", "id": "g1"})
        check(not errors, f"no errors in the console ({errors})")

        # --- Full quality from the first seconds ------------------------------------
        print("Quality from the start")
        big = [jpeg_hd(i) for i in range(12)]
        cast.evaluate("clearInterval(window.__pictures)")
        cast.evaluate("""(frames) => { let f = 0; window.__hd = setInterval(() => window.__ytmpPost('f' + frames[f++ % frames.length]), 33); }""", big)
        viewer.goto("https://ytmp.localhost/stream")
        wait(lambda: viewer_sent().count("stream:ready") >= 1, "ready")
        tell({"t": "state", "host": "Rodrigo", "control": False, "paused": False, "hostAway": False})
        order({"t": "peer", "id": "g2", "ice": [], "bitrate": 2500000})
        offer = wait(lambda: [m for m in cast_sent() if m.startswith("cast:offer:g2:")], "offer")
        tell({"t": "offer", "sdp": offer[0][len("cast:offer:g2:"):], "ice": []})
        answer = wait(lambda: [m for m in viewer_sent() if m.startswith("stream:answer:")][-1:], "answer")
        order({"t": "answer", "id": "g2", "sdp": answer[0][len("stream:answer:"):]})
        wait(lambda: "cast:state:g2:connected" in cast_sent(), "connected")
        first = wait(lambda: viewer.evaluate("document.getElementById('video').videoWidth"), "a picture", 8)
        check(first and first >= 960, f"the first pictures are already sharp ({first} wide; WebRTC alone starts at 640 or less)")
        viewer.evaluate("window.__ytmpStreamStats()")
        viewer.wait_for_timeout(3000)
        fps = viewer.evaluate("window.__ytmpStreamStats()")
        check(fps and fps["fps"] >= 24 and not fps.get("freezes") and not fps.get("dropped"),
              f"and it keeps up: 30 pictures a second in, {fps and fps['fps']} out ({fps}; host {cast.evaluate('window.__ytmpCastStats()')})")
        cast.evaluate("clearInterval(window.__hd)")
        order({"t": "close", "id": "g2"})

        # --- Every language, two sizes: nothing cut off -----------------------------
        print("Languages")

        def overflowing():
            return viewer.evaluate("""() => {
              const bad = [];
              if (document.documentElement.scrollWidth > document.documentElement.clientWidth + 1) bad.push('page');
              for (const el of document.querySelectorAll('.top, .bottom, .card, .pill, .badge, .nm, #message, #fine')) {
                if (!el.offsetParent) continue;
                const r = el.getBoundingClientRect();
                if (r.left < -1 || r.right > innerWidth + 1 || r.top < -1 || r.bottom > innerHeight + 1) bad.push('outside: ' + (el.id || el.className));
                if (el.scrollWidth > el.clientWidth + 1) bad.push((el.id || el.className) + ': ' + el.textContent.trim().slice(0, 40));
              }
              const top = document.querySelector('.top').getBoundingClientRect();
              const bottom = document.querySelector('.bottom').getBoundingClientRect();
              // The app's side bar lives in the right 64 px.
              if (bottom.right > innerWidth - 64) bad.push('the controls reach the side bar');
              const card = document.getElementById('card');
              if (card.offsetParent) {
                const c = card.getBoundingClientRect();
                const meets = (a, b) => a.left < b.right && b.left < a.right && a.top < b.bottom && b.top < a.bottom;
                if (meets(c, bottom)) bad.push('the message covers the controls');
                if (meets(c, top)) bad.push('the message covers the name');
              }
              return bad;
            }""")

        states = {
            "connecting": {"t": "state", "host": "Rodrigo", "control": False, "paused": False, "hostAway": False},
            "using": {"t": "state", "host": "Rodrigo", "control": True, "paused": False, "hostAway": False},
            "paused": {"t": "state", "host": "Rodrigo", "control": False, "paused": True, "hostAway": False},
            "away": {"t": "state", "host": "Rodrigo", "control": False, "paused": False, "hostAway": True},
        }
        for language in LANGUAGES:
            code = language["code"]
            lang["code"] = code
            for width, height in ((480, 270), (960, 540)):
                viewer.set_viewport_size({"width": width, "height": height})
                viewer.goto("https://ytmp.localhost/stream")
                viewer.wait_for_timeout(100)
                bad = {}
                for name, state in states.items():
                    tell(state)
                    viewer.mouse.move(width / 2, height / 2)
                    viewer.wait_for_timeout(50)
                    found = overflowing()
                    if found:
                        bad[name] = found
                    if SHOTS and width == 480 and name in ("connecting", "using"):
                        viewer.screenshot(path=str(SHOTS / f"stream-{code}-{name}.png"))
                check(not bad, f"{code} at {width}×{height}: everything fits ({bad})")
            check(viewer.evaluate("document.documentElement.lang") == code, f"{code}: <html lang>")
        browser.close()

    print(f"\n{'FAILED: ' + str(len(failures)) if failures else 'all passed'}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
