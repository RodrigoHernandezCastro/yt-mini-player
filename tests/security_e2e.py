"""Security tests of what runs inside the pages (Chromium).

1. A hostile page (its own scripts trying to use the app) with the same
   wrapper the app injects (tests/bundle.py reads webview.rs).
2. The lock screen (src/lock.html) with the real CSP, in English and in the
   languages with longer or wider texts.

Usage:  python tests/security_e2e.py [folder_for_screenshots]
"""

import json
import pathlib
import sys

from playwright.sync_api import sync_playwright

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from bundle import HOST_KEY, TOKEN, add_app_scripts, fill, i18n, push_locale, text  # noqa: E402

SHOTS = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else None
CSP = ("default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; "
       "img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'")

# A page that tries, with its own scripts, to give the app orders.
HOSTILE = """<!doctype html><html><head><meta charset="utf-8"><title>Hostile site</title></head><body>
<video id="v" style="width:600px;height:300px;background:#222"></video>
<script>
  window.stolen = [];
  window.stateLeak = [];
  window.addEventListener('ytmp:state', (e) => window.stateLeak.push(e.detail));
  window.addEventListener('ytmp:locale', (e) => window.stateLeak.push(e.detail));
  // Tries to intercept WebView2's channel to steal the token.
  const original = window.chrome.webview.postMessage;
  window.chrome.webview.postMessage = (m) => { window.stolen.push(String(m)); original(m); };
  // And to write to the app directly.
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
        print("Hostile page")
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
              f"the app accepts no orders written by the page ({sent()})")
        check({"autostart", "ghost", "quit"} <= set(forged()), f"they arrive without a token and are dropped ({forged()})")

        # Our scripts keep working with the original channel…
        clear()
        page.mouse.move(300, 150)
        page.keyboard.press("Control+t")
        check(sent() == ["pin"], f"a real shortcut does arrive with the token ({sent()})")
        # …and the page's replacement does not see the token.
        stolen = page.evaluate("window.stolen")
        check(not any(TOKEN in m for m in stolen), f"the page does not intercept the token ({stolen[:3]})")

        # Synthetic events: they trigger neither our shortcuts nor dragging.
        clear()
        page.evaluate("""() => {
            for (const key of ['q', 't', 'h', 'l']) {
              window.dispatchEvent(new KeyboardEvent('keydown', { key, code: 'Key' + key.toUpperCase(), ctrlKey: true, bubbles: true }));
            }
            const v = document.getElementById('v');
            v.dispatchEvent(new PointerEvent('pointerdown', { bubbles: true, pointerType: 'mouse', button: 0, altKey: true, clientX: 300, clientY: 150 }));
        }""")
        check(sent() == [], f"synthetic keys and clicks from the page do nothing ({sent()})")

        # Functions only the app can call: a fake notice from the page does
        # not show (the screenshot does not change); one from the app does.
        check(page.evaluate("document.querySelector('ytmp-ui') && 1") == 1, "the app's interface is mounted")
        page.wait_for_timeout(300)
        before = page.screenshot()
        page.evaluate("window.__ytmpToast('Your session expired: type your password here')")
        page.evaluate("window.__ytmpSync({ autostart: true, extensions: [] })")
        page.evaluate("window.__ytmpLocale({ lang: 'ja', ui: { 'ui.first_hint': 'Type your password' } })")
        page.wait_for_timeout(500)
        check(page.screenshot() == before, "a fake notice from the page does not show")
        page.evaluate(f"window.__ytmpToast({json.dumps(HOST_KEY)}, 'A real notice from the app')")
        page.wait_for_timeout(500)
        check(page.screenshot() != before, "a notice from the app does show")
        leak = page.evaluate("window.stateLeak.length")
        check(leak == 0, f"the app's settings are not published to other sites ({leak})")
        page.evaluate(f"window.__ytmpSync({json.dumps(HOST_KEY)}, {{ autostart: true }})")
        push_locale(page, "es", "home")
        page.wait_for_timeout(100)
        check(page.evaluate("window.stateLeak.length") == 0, "not even when the app syncs or changes language")

        # The token is nowhere the page can read.
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
        check(found == [], f"the token cannot be reached from the page ({found})")
        check(page.evaluate("typeof window.__ytmpHomeSend") == "undefined", "the start screen's channel does not exist outside it")
        check(page.evaluate("typeof window.__ytmpMusic") == "undefined", "nor does My music's bridge (library, paths, license)")
        check(page.evaluate("typeof window.__ytmpStartDrag") == "undefined", "dragging the window is not exposed")
        check(not errors, f"no errors on the page ({errors[:3]})")
        page.close()

        # ------------------------------------------------------------------
        print("Lock screen")
        errors = []
        page = browser.new_page(viewport={"width": 640, "height": 360})
        page.on("console", lambda m: m.type == "error" and errors.append(m.text))
        page.on("pageerror", lambda e: errors.append(str(e)))
        lang = {"code": "en"}
        page.route("https://ytmp.localhost/**", lambda r: r.fulfill(
            status=200, content_type="text/html; charset=utf-8", headers={"Content-Security-Policy": CSP},
            body=fill("lock.html", {"i18n": i18n(lang["code"], "lock")}, lang["code"])))
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

        check(sent()[:1] == ["lock:ready"], f"says it is ready ({sent()})")
        check(page.title() == "Locked" and page.evaluate("document.documentElement.lang") == "en", "in English")
        push(stage="unlock", method="hello", busy=False, error=None, wait=0, canCancel=False)
        check(page.is_visible("#hello") and page.is_hidden("#pin-form"), "with Windows Hello: a button, no PIN field")
        check(page.inner_text("#hello") == "Use Windows Hello", "the button says what it does")
        check(page.is_hidden("#cancel-box"), "unlocking cannot be cancelled")
        clear()
        page.click("#hello")
        check(sent() == ["lock:hello"], f"the button asks for Windows Hello ({sent()})")
        push(busy=True)
        check(page.is_disabled("#hello") and page.inner_text("#status") == "Waiting for Windows Hello…", "while waiting, it says so")
        shot("1-hello")

        push(method="pin", busy=False, error=None)
        check(page.is_visible("#pin") and page.is_hidden("#pin2"), "with a PIN: one field")
        check(page.evaluate("document.activeElement.id") == "pin", "focus goes to the PIN")
        clear()
        page.keyboard.type("482915")
        page.keyboard.press("Enter")
        check(sent() == ["lock:pin:482915"], f"Enter sends the PIN ({sent()})")
        check(page.input_value("#pin") == "", "the PIN does not stay on the page")
        push(error=text("en", "lock.pin_wrong"))
        check(page.inner_text("#status") == "Wrong PIN" and page.evaluate("document.getElementById('status').classList.contains('error')"),
              "shows the error the app sends")
        check(page.is_visible("#forgot") and page.inner_text("#forgot") == "Forgot your PIN?", "with a PIN, “Forgot your PIN?” is offered")
        clear()
        page.click("#forgot")
        check(sent() == ["lock:forgot"], f"and asks the app for it ({sent()})")
        push(error=None, wait=3)
        check(page.is_disabled("#submit") and page.is_disabled("#pin") and "Wait 3 s" in page.inner_text("#status"),
              "after many failures, a forced wait")
        clear()
        page.evaluate("document.getElementById('pin-form').requestSubmit()")
        check(sent() == [], "nothing is sent during the wait")
        page.wait_for_timeout(3300)
        check(not page.is_disabled("#submit"), "when the wait is over, it can be tried again")
        shot("2-pin")

        push(stage="new-pin", method="pin", wait=0, error=None, canCancel=True)
        check(page.is_visible("#pin2") and page.is_visible("#hint") and page.is_visible("#cancel"), "creating a PIN: two fields, rules and cancel")
        check(page.is_hidden("#forgot"), "creating a PIN shows no “forgot”")
        clear()
        page.fill("#pin", "482915")
        page.fill("#pin2", "482916")
        page.click("#submit")
        check(sent() == [] and page.inner_text("#status") == "The two PINs don't match", "if they differ, nothing is sent")
        page.fill("#pin", "482915")
        page.fill("#pin2", "482915")
        page.click("#submit")
        check(sent() == ["lock:new-pin:482915"], f"sends the new PIN ({sent()})")
        clear()
        page.keyboard.press("Escape")
        check(sent() == ["lock:cancel"], f"Escape cancels a change ({sent()})")
        shot("3-new-pin")

        push(stage="verify", method="hello", canCancel=True)
        check(page.inner_text("#title") == "Confirm it's you" and page.is_visible("#cancel-box"), "verifying before a change: title and cancel")

        # The app reloads the lock screen when the language changes.
        for code in ("en", "ru", "ja", "pl"):
            lang["code"] = code
            page.set_viewport_size({"width": 640, "height": 360})
            page.reload()
            page.wait_for_timeout(150)
            push(stage="new-pin", method="pin", busy=False, error=None, wait=0, canCancel=True)
            check(page.inner_text("#title") == text(code, "lock.title_new_pin"), f"{code}: the screen in that language")
            check(page.get_attribute("#pin2", "placeholder") == text(code, "lock.repeat_pin"), f"{code}: placeholders too")
            for w, h in [(360, 220), (256, 144)]:
                page.set_viewport_size({"width": w, "height": h})
                push(stage="unlock", method="pin", canCancel=False)
                page.wait_for_timeout(100)
                fits = page.evaluate("""() => { const r = document.querySelector('main').getBoundingClientRect();
                    return r.top >= 0 && r.bottom <= innerHeight && r.left >= 0 && r.right <= innerWidth; }""")
                check(fits, f"{code} {w}×{h}: fits whole")
                shot(f"4-{code}-{w}x{h}")
        check(not errors, f"no console or CSP errors ({errors[:3]})")
        browser.close()

    print()
    if failures:
        print(f"{len(failures)} test(s) failed")
        sys.exit(1)
    print("security (hostile page + lock screen): all tests pass")


if __name__ == "__main__":
    main()
