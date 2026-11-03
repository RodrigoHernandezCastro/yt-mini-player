"""End-to-end test of the "Watch together" window in Chromium.

Serves src/together.html at https://ytmp.localhost/together with the app's
CSP and strings (as home.rs serves them), sends it states the way the app
does (window.__ytmpTogether) and checks what it draws and what it sends
back. Then every screen again in the five languages, at the window's normal
and narrowest size, looking for text that does not fit.

Usage:  python tests/together_e2e.py [folder_for_screenshots]
"""

import json
import pathlib
import sys
import time

from playwright.sync_api import sync_playwright

sys.path.insert(0, str(pathlib.Path(__file__).resolve().parent))
from bundle import LANGUAGES, fill, i18n, text  # noqa: E402

SHOTS = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else None
CSP = ("default-src 'none'; script-src 'unsafe-inline'; style-src 'unsafe-inline'; "
       "img-src data:; base-uri 'none'; form-action 'none'; frame-ancestors 'none'")
IPC_STUB = """
window.__sent = [];
Object.defineProperty(window, 'ipc', { value: Object.freeze({ postMessage: (m) => window.__sent.push(m) }) });
"""
RELAY = "wss://ytmp-together.example.workers.dev"


def views():
    """One state per screen, as Together::view_json builds them."""
    now = int(time.time())
    base = {"phase": "idle", "ended": None, "codeError": None, "name": "", "relay": RELAY, "relayFixed": False,
            "relayError": False, "reconnecting": False, "locked": False, "mode": "share", "maxPeople": 6,
            "code": None, "codeEnds": 0, "control": False, "members": [], "sharing": False, "protectedPage": False,
            "sound": None, "soundWholePc": False, "soundProblem": None, "soundAppUnavailable": False, "stream": None, "hostName": "", "hostAway": False, "status": None, "title": "",
            "site": "", "openUrl": None, "rtt": None, "watching": False, "paused": False}
    person = lambda id, name, **kw: dict({"id": id, "name": name, "you": False, "online": True, "control": False,
                                          "asking": False, "stream": "none"}, **kw)
    members = [
        person("host", "Rodrigo", you=True, control=True),
        person("g1", "Ana", stream="watching", control=True),
        person("g2", "Bruno", online=False),
        person("g3", "Carla Fernández", stream="connecting", asking=True),
        person("g4", "Dani", stream="failed"),
        person("g5", "Eli", stream="watching"),
    ]
    guest_members = [dict(members[0], you=False), dict(members[1], you=True), members[4]]
    return {
        "idle": dict(base, name="Rodrigo"),
        "idle-sync": dict(base, name="Rodrigo", mode="sync"),
        "idle-no-server": dict(base, relay=None),
        "connecting": dict(base, phase="connecting", code="K7Q4-9XM2-PF3T"),
        "host": dict(base, phase="host", code="K7Q4-9XM2-PF3T", codeEnds=now + 23 * 60 + 41, members=members,
                     sharing=True, sound=True),
        "host-protected": dict(base, phase="host", code="K7Q4-9XM2-PF3T", codeEnds=now + 600, members=members[:2],
                               sharing=True, sound=False, soundProblem="unavailable", protectedPage=True),
        "host-sound-silent": dict(base, phase="host", code="K7Q4-9XM2-PF3T", codeEnds=now + 600, members=members[:2],
                                  sharing=True, sound=False, soundProblem="silent"),
        "host-sound-pc": dict(base, phase="host", code="K7Q4-9XM2-PF3T", codeEnds=now + 600, members=members[:2],
                              sharing=True, sound=True, soundWholePc=True),
        "host-sound-pc-silent": dict(base, phase="host", code="K7Q4-9XM2-PF3T", codeEnds=now + 600, members=members[:2],
                                     sharing=True, sound=False, soundWholePc=True, soundProblem="silent",
                                     soundAppUnavailable=True),
        "host-sync": dict(base, phase="host", mode="sync", code="K7Q4-9XM2-PF3T", codeEnds=now + 600,
                          members=[members[0], dict(members[1], stream="none"), dict(members[4], stream="none")]),
        "host-alone": dict(base, phase="host", code="PTQ7-K20N-PW5J", codeEnds=now + 90, members=members[:1]),
        "guest": dict(base, phase="guest", mode="sync", hostName="Rodrigo", status="in-sync", control=False,
                      title="Lo-fi beats to relax and study to — 24/7 live radio with a rather long title",
                      site="www.youtube.com", rtt=84, members=guest_members),
        "guest-site": dict(base, phase="guest", mode="sync", hostName="Rodrigo", status="other-site", site="vimeo.com",
                           openUrl="https://vimeo.com/76979871", reconnecting=True, members=guest_members),
        "guest-share": dict(base, phase="guest", hostName="Rodrigo", watching=True, stream="watching",
                            title="Lo-fi beats to relax and study to", site="www.youtube.com", rtt=84,
                            members=guest_members),
        "guest-share-closed": dict(base, phase="guest", hostName="Rodrigo", watching=False, members=guest_members),
        "guest-share-control": dict(base, phase="guest", hostName="Rodrigo", watching=True, stream="connecting",
                                    control=True, members=guest_members),
        "ended": dict(base, phase="ended", ended="together.ended_bad_code"),
    }


def main():
    failures = []

    def check(cond, name):
        print(("  ok   " if cond else "  FAIL ") + name)
        if not cond:
            failures.append(name)

    with sync_playwright() as p:
        browser = p.chromium.launch()
        lang = {"code": "en"}
        errors = []
        page = browser.new_page(viewport={"width": 380, "height": 600})
        page.on("console", lambda m: m.type == "error" and errors.append(m.text))
        page.on("pageerror", lambda e: errors.append(str(e)))
        page.route("https://ytmp.localhost/**", lambda r: r.fulfill(
            status=200, content_type="text/html; charset=utf-8", headers={"Content-Security-Policy": CSP},
            body=fill("together.html", {"i18n": i18n(lang["code"], "together")}, lang["code"])))
        page.add_init_script(IPC_STUB)

        sent = lambda: page.evaluate("window.__sent.slice()")
        clear = lambda: page.evaluate("window.__sent.length = 0")

        def push(view):
            page.evaluate(f"window.__ytmpTogether({json.dumps(view)})")
            page.wait_for_timeout(60)

        def shot(name):
            if SHOTS:
                SHOTS.mkdir(parents=True, exist_ok=True)
                page.screenshot(path=str(SHOTS / f"{name}.png"), full_page=True)

        def overflowing():
            """Elements whose text does not fit (and a page that scrolls sideways)."""
            return page.evaluate("""() => {
              const bad = [];
              if (document.documentElement.scrollWidth > document.documentElement.clientWidth + 1) bad.push('page');
              for (const el of document.querySelectorAll('button, h1, h2, h3, .label, .code, .nm, .status, .server-line, .segmented span')) {
                if (!el.offsetParent) continue;
                if (el.scrollWidth > el.clientWidth + 1) bad.push((el.id || el.className || el.tagName) + ': ' + el.textContent.trim().slice(0, 40));
              }
              return bad;
            }""")

        V = views()
        page.goto("https://ytmp.localhost/together")
        page.wait_for_timeout(150)
        print("English: what it draws and sends")
        check(sent() == ["together:ready"], f"asks for its state when it loads ({sent()})")
        check(page.title() == "Watch together", "window title")

        # --- Not in a session --------------------------------------------------
        push(V["idle-no-server"])
        check(page.is_disabled("#start") and page.is_disabled("#join"), "no server yet: nothing can start")
        check(page.is_visible("#server-form") and page.is_hidden("#server-line"), "it asks for the server")
        clear()
        page.fill("#relay", "ytmp-together.example.workers.dev")
        page.click("#relay-save")
        check(sent() == ["together:relay:ytmp-together.example.workers.dev"], f"sends the address to check ({sent()})")
        push(dict(V["idle-no-server"], relayError=True))
        check(page.inner_text("#relay-error") == text("en", "together.server_invalid"), "says when it is not an address")

        push(V["idle"])
        check(page.get_attribute('.modes [data-mode="share"]', "aria-checked") == "true"
              and page.get_attribute('.modes [data-mode="sync"]', "aria-checked") == "false", "sharing the window is chosen")
        clear()
        page.click('.modes [data-mode="sync"]')
        page.click('.modes [data-mode="share"]')
        check(sent() == ["together:mode:sync"], f"the other way is asked for, the chosen one is not sent again ({sent()})")
        push(V["idle-sync"])
        check(page.get_attribute('.modes [data-mode="sync"]', "aria-checked") == "true", "syncing chosen")
        push(V["idle"])
        check(page.input_value("#name") == "Rodrigo", "the saved name is filled in")
        check(page.is_visible("#server-line") and "ytmp-together.example.workers.dev" in page.inner_text("#server-addr"),
              "the server, in one line")
        check(not page.is_disabled("#start") and page.is_disabled("#join"), "Start on; Join waits for a whole code")
        clear()
        page.fill("#name", "  Ana  ")
        page.click("#start")
        check(sent() == ["together:start:Ana"], f"Start sends the name, trimmed ({sent()})")

        # Codes as people type them.
        page.fill("#code", "")
        page.type("#code", "k7q4 9xm2 pf3t")
        check(page.input_value("#code") == "K7Q4-9XM2-PF3T", f"upper case, dashes every four ({page.input_value('#code')})")
        page.fill("#code", "")
        page.type("#code", "oil0-uuuu-abcd-efgh-jkmn")
        check(page.input_value("#code") == "0110-ABCD-EFGH", f"O→0, I/L→1, U and extra left out ({page.input_value('#code')})")
        check(not page.is_disabled("#join"), "12 characters: Join on")
        clear()
        page.click("#join")
        check(sent() == ["together:join:0110ABCDEFGH:Ana"], f"Join sends the code and the name ({sent()})")
        page.fill("#name", "")
        page.fill("#code", "K7Q4-9XM2-PF3T")
        page.dispatch_event("#code", "input")
        clear()
        page.press("#code", "Enter")
        check(sent() == ["together:join:K7Q49XM2PF3T"], f"Enter joins too; no name, no name part ({sent()})")
        push(dict(V["idle"], codeError="together.code_check"))
        check(page.inner_text("#code-error") == text("en", "together.code_check"), "a typo is explained")
        push(dict(V["idle"], relayFixed=True))
        check(page.is_hidden("#server"), "a build with its own server does not mention it")
        push(V["idle"])
        shot("en-1-idle")

        # --- Connecting ----------------------------------------------------------
        push(V["connecting"])
        check(page.inner_text("#connecting-text") == text("en", "together.connecting"), "connecting as host")
        push(dict(V["connecting"], code=None))
        check(page.inner_text("#connecting-text") == text("en", "together.joining"), "joining as guest")
        clear()
        page.click("#cancel")
        check(sent() == ["together:leave"], "Cancel leaves")

        # --- Hosting ---------------------------------------------------------------
        push(dict(V["host"], codeEnds=int(time.time()) + 23 * 60 + 41))
        check(page.inner_text("#code-text").replace("\n", "") == "K7Q4–9XM2–PF3T", f"the code, in three groups ({page.inner_text('#code-text')!r})")
        left = page.inner_text("#code-left")
        check(left in (text("en", "together.changes_in", time="23:41"), text("en", "together.changes_in", time="23:40")),
              f"counts down to the next code ({left})")
        fill_ratio = page.evaluate("document.getElementById('code-fill').getBoundingClientRect().width / document.querySelector('.meter').getBoundingClientRect().width")
        check(abs(fill_ratio - (23 * 60 + 41) / 1800) < 0.01, f"the bar shows the time left ({fill_ratio:.3f})")
        check(page.inner_text("#host-people-title").upper() == text("en", "together.people", count=6, max=6).upper(), "people count")
        rows = page.query_selector_all("#host-people li")
        check(len(rows) == 6 and "you" in rows[0].inner_text() and "host" in rows[0].inner_text(), "the host first, as you")
        check("reconnecting" in rows[2].inner_text(), "a guest whose connection dropped says so")
        check(len(page.query_selector_all("#host-people .remove")) == 4, "guests can be removed (after answering one who asks), the host cannot")
        check(page.get_attribute('.segmented [data-mode="share"]', "aria-checked") == "true", "sharing, in the switch")
        check(page.inner_text("#mode-status-text") == text("en", "together.sharing_now", count=2)
              and page.get_attribute("#mode-status", "data-tone") == "live", "two are watching")
        check(text("en", "together.stream.watching") in rows[1].inner_text()
              and text("en", "together.can_control") in rows[1].inner_text(), "Ana watches and may use the window")
        check(text("en", "together.stream.failed") in rows[4].inner_text(), "Dani cannot connect")
        check(text("en", "together.asks") in rows[3].inner_text() and page.is_visible("#host-people li.asking .allow"),
              "Carla asks: Allow and Not now")
        check(page.get_attribute("#host-people li:nth-child(2) .ctl", "aria-pressed") == "true"
              and page.get_attribute("#host-people li:nth-child(5) .ctl", "aria-pressed") == "false", "who may use it")
        check(page.is_hidden("#protected-note") and page.is_hidden("#sound-box"), "no warnings")
        clear()
        page.click("#host-people li:nth-child(2) .ctl")
        page.click("#host-people li:nth-child(5) .ctl")
        page.click("#host-people li.asking .allow")
        page.click("#host-people li.asking .deny")
        check(sent() == ["together:member-control:g1:0", "together:member-control:g4:1", "together:member-control:g3:1",
                         "together:dismiss-ask:g3"], f"control per person, allow, not now ({sent()})")
        clear()
        page.click('.segmented [data-mode="sync"]')
        check(sent() == ["together:mode:sync"], "the switch changes the way of watching")
        clear()
        page.click("#copy")
        page.evaluate("window.__ytmpTogetherCopied()")
        check(sent() == ["together:copy"] and page.inner_text("#copy-label") == "Copied", "Copy asks the app, then says Copied")
        clear()
        page.click("#new-code")
        check(sent() == ["together:new-code"], "New code now")
        clear()
        remove = page.query_selector_all("#host-people .remove")[0]
        remove.click()
        check(sent() == [] and remove.inner_text() == text("en", "together.remove_confirm"), "Remove asks first")
        remove.click()
        check(sent() == ["together:kick:g1"], f"…and removes on the second click ({sent()})")
        clear()
        page.click("#end")
        check(sent() == [] and page.inner_text("#end") == text("en", "together.end_confirm"), "End asks first")
        page.click("#end")
        check(sent() == ["together:leave"], "…and ends on the second click")
        push(V["host"])
        shot("en-2-host")
        push(V["host-protected"])
        check(page.is_visible("#protected-note") and page.is_visible("#sound-box"), "protected video, no sound: said")
        check(page.inner_text("#sound-text") == text("en", "together.sound_unavailable")
              and page.is_visible("#sound-pc") and page.is_hidden("#sound-app"), "no app sound: the PC's sound is offered")
        check(text("en", "together.sound_pc_sub") in page.inner_text("#sound-pc"), "…saying what else it carries")
        check(page.inner_text("#mode-status-text") == text("en", "together.sharing_now", count=1), "one watching")
        shot("en-2b-host-protected")

        # --- The window's sound -----------------------------------------------------
        push(V["host-sound-silent"])
        check(page.inner_text("#sound-text") == text("en", "together.sound_silent")
              and "warn" in page.get_attribute("#sound-box", "class"), "playing but not heard: said, as a warning")
        clear()
        page.focus("#sound-pc")
        page.keyboard.press("Enter")
        check(sent() == ["together:sound:pc"], f"the PC's sound, from the keyboard ({sent()})")
        push(V["host-sound-pc"])
        check(page.inner_text("#sound-text") == text("en", "together.sound_pc_on")
              and "warn" not in page.get_attribute("#sound-box", "class"), "the PC's sound: what it carries, not a warning")
        check(page.is_hidden("#sound-pc") and page.is_visible("#sound-app"), "…and the way back")
        check(page.evaluate("document.activeElement.id") == "sound-app", "the focus moves to the way back")
        shot("en-2d-host-sound-pc")
        clear()
        page.click("#sound-app")
        check(sent() == ["together:sound:app"], "back to the app's sound")
        push(V["host-sound-pc-silent"])
        check(page.inner_text("#sound-text") == text("en", "together.sound_pc_silent")
              and page.is_hidden("#sound-app") and page.is_hidden("#sound-pc"),
              "the PC's sound not heard either, the app's unavailable: what to check, no buttons")
        push(dict(V["host-sound-pc"], soundProblem="unavailable"))
        check(page.inner_text("#sound-text") == text("en", "together.sound_pc_failed") and page.is_visible("#sound-app"),
              "the PC's sound failed: said, the app's still offered")
        push(dict(V["host-sound-silent"], soundProblem="page"))
        check(page.inner_text("#sound-text") == text("en", "together.sound_page") and page.is_hidden("#sound-pc"),
              "the page can't send sound: nothing to switch to")
        push(dict(V["host-sound-silent"], mode="sync"))
        check(page.is_hidden("#sound-box"), "syncing: nothing about the sound")
        push(dict(V["host-sound-silent"], sharing=False))
        check(page.is_hidden("#sound-box"), "not capturing: nothing about the sound")
        push(dict(V["host-protected"], locked=True))
        check(page.inner_text("#mode-status-text") == text("en", "together.sharing_paused"), "locked: paused")
        push(V["host-sync"])
        check(page.inner_text("#mode-status-text") == text("en", "together.sync_status"), "syncing")
        check(text("en", "together.stream.none") not in page.inner_text("#host-people"), "no window states when syncing")
        check(page.get_attribute("#host-people li:nth-child(2) .ctl", "title") == text("en", "together.control_take_sync", name="Ana"),
              "control means pause and seek here")
        shot("en-2c-host-sync")
        push(dict(V["host-alone"], reconnecting=True))
        check(text("en", "together.nobody_yet") in page.inner_text("#host-people"), "alone: says where guests will show up")
        check(page.is_visible("#host-banner"), "reconnecting: a banner")

        # --- Watching -----------------------------------------------------------------
        push(V["guest"])
        check(page.inner_text("#lede") == text("en", "together.watching_with", name="Rodrigo"), "who hosts")
        check(page.inner_text("#status-text") == text("en", "together.status.in_sync"), "in sync")
        check(page.get_attribute("#status", "data-tone") == "ok", "in sync is green")
        check(page.is_hidden("#open-site") and page.is_hidden("#guest-banner"), "nothing to open, no banner")
        check(page.inner_text("#control-note") == text("en", "together.control_host_only"), "who may pause")
        check(page.inner_text("#rtt") == text("en", "together.delay", ms=42), "delay: half the round trip")
        for status, tone in [("syncing", "busy"), ("host-ad", "warn"), ("cannot-seek", "warn"), ("host-elsewhere", "idle")]:
            push(dict(V["guest"], status=status))
            key = "together.status." + status.replace("-", "_")
            check(page.inner_text("#status-text") == text("en", key) and page.get_attribute("#status", "data-tone") == tone,
                  f"status {status}")
        push(V["guest"])
        shot("en-3-guest")
        push(V["guest-site"])
        check(page.is_visible("#open-site") and page.inner_text("#open-site") == "Open vimeo.com", "another site: open it by hand")
        check(page.inner_text("#status-text") == text("en", "together.status.other_site", site="vimeo.com"), "says why")
        check(page.inner_text("#guest-banner") == text("en", "together.reconnecting"), "reconnecting banner")
        clear()
        page.click("#open-site")
        page.click("#resync")
        page.click("#leave")
        check(sent() == ["together:open-site", "together:resync", "together:leave"], f"open, resync, leave ({sent()})")
        push(dict(V["guest-site"], reconnecting=False, hostAway=True))
        check(page.inner_text("#guest-banner") == text("en", "together.host_away"), "the host's connection dropped")

        # --- Watching the host's window --------------------------------------------
        push(V["guest-share"])
        check(page.inner_text("#status-text") == text("en", "together.window.watching", name="Rodrigo")
              and page.get_attribute("#status", "data-tone") == "ok", "watching the host's window")
        check(page.inner_text("#control-note") == text("en", "together.you_watch", name="Rodrigo"), "only the host uses it")
        check(page.is_visible("#ask") and page.is_hidden("#show-window") and page.is_hidden("#open-site"), "Ask, nothing else")
        check(page.inner_text("#resync") == text("en", "together.reconnect"), "Resync becomes Reconnect")
        clear()
        page.click("#ask")
        check(sent() == ["together:ask"] and page.is_disabled("#ask") and page.inner_text("#ask") == text("en", "together.asked"),
              f"asks once, then waits ({sent()})")
        shot("en-3b-guest-share")
        push(V["guest-share-closed"])
        check(page.inner_text("#status-text") == text("en", "together.window.closed", name="Rodrigo")
              and page.is_visible("#show-window"), "not on screen: Show the window")
        clear()
        page.click("#show-window")
        check(sent() == ["together:watch"], "Show the window")
        push(V["guest-share-control"])
        check(page.is_hidden("#ask") and page.inner_text("#control-note") == text("en", "together.you_control", name="Rodrigo"),
              "may use it")
        check(page.inner_text("#status-text") == text("en", "together.window.connecting", name="Rodrigo"), "connecting")
        push(dict(V["guest-share"], paused=True))
        check(page.inner_text("#status-text") == text("en", "together.window.paused", name="Rodrigo")
              and page.get_attribute("#status", "data-tone") == "warn", "paused by the host")

        # --- Over -------------------------------------------------------------------
        push(V["ended"])
        check(page.inner_text("#ended-text") == text("en", "together.ended_bad_code"), "why it ended")
        clear()
        page.click("#dismiss")
        check(sent() == ["together:dismiss"], "OK goes back")
        clear()
        page.keyboard.press("Escape")
        check(sent() == ["together:close"], "Escape closes the window")
        shot("en-4-ended")
        check(not errors, f"no errors in the console ({errors})")

        # --- Every language, every screen, two sizes ----------------------------------
        for language in LANGUAGES:
            code = language["code"]
            lang["code"] = code
            for width, height in ((380, 600), (320, 480)):
                page.set_viewport_size({"width": width, "height": height})
                page.goto("https://ytmp.localhost/together")
                page.wait_for_timeout(80)
                bad = {}
                for name, view in V.items():
                    push(view)
                    found = overflowing()
                    if found:
                        bad[name] = found
                    if width == 380 and name in ("idle", "host", "host-sound-silent", "guest", "guest-share") and code != "en":
                        shot(f"{code}-{name}")
                check(not bad, f"{code} at {width}px: everything fits ({bad})")
            check(page.evaluate("document.documentElement.lang") == code, f"{code}: <html lang>")
        check(not errors, f"no errors in any language ({errors})")
        browser.close()

    print(f"\n{'FAILED: ' + str(len(failures)) if failures else 'all passed'}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
