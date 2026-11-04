"""End-to-end tests on real Windows, against the app itself.

The browser suites test the pages in Chromium with stand-ins for the app;
this one starts the real program (the drivable test build, src/e2e.rs) and
drives its real WebView2 windows over CDP, so what only happens natively is
covered too: windows opening, always on top, two copies talking through the
relay.

  1. Start: the start screen loads (welcome tour, keypad, gear).
  2. Settings: the gear opens the Settings window (its own WebView2 page);
     "Always on top" there changes the player window's WS_EX_TOPMOST and
     back; Escape hides the window.
  3. Watch together (with --relay): a second copy, with its own data folder
     and port, joins the first through the relay, and both see each other.

    cargo build --release --features e2e
    python tests/windows_e2e.py [--exe PATH] [--relay ws://127.0.0.1:8799]

Windows only. Each copy gets a fresh data folder under %TEMP%, so the
tester's own session is never touched. `--self-test` checks this script's
plumbing (CDP connection, finding pages) against a plain Chromium on any
system.
"""
import argparse
import ctypes
import json
import os
import pathlib
import subprocess
import sys
import tempfile
import time
import urllib.request

from playwright.sync_api import sync_playwright

ROOT = pathlib.Path(__file__).resolve().parent.parent
HOME = "https://ytmp.localhost/"
SETTINGS = "https://ytmp.localhost/settings"
TOGETHER = "https://ytmp.localhost/together"

failures = []


def check(cond, name):
    print(("  ok   " if cond else "  FAIL ") + name)
    if not cond:
        failures.append(name)


def wait_for(what, condition, timeout=20.0, step=0.25):
    """Polls `condition` until it returns something truthy."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        try:
            value = condition()
        except Exception:
            value = None
        if value:
            return value
        time.sleep(step)
    raise TimeoutError(f"timed out waiting for: {what}")


def devtools_up(port):
    with urllib.request.urlopen(f"http://127.0.0.1:{port}/json/version", timeout=2) as answer:
        return json.load(answer).get("webSocketDebuggerUrl")


_sessions = {}


def take_in_news(browser):
    """Lets Playwright take in what the browser told it since the last call.

    The sync API reads the browser's messages (such as "a window has a new
    page") only while a call waits for an answer, and reading .contexts or
    .pages is not such a call: a loop that only reads them and sleeps never
    sees a page that appeared after connecting (the Settings or Watch
    together page, or the start screen itself on a slow runner). A question
    to the browser is.
    """
    session = _sessions.get(id(browser))
    if session is None:
        session = _sessions[id(browser)] = browser.new_browser_cdp_session()
    session.send("Browser.getVersion")


def find_page(browser, address):
    """The app's page at `address` (any of its windows), or None."""
    take_in_news(browser)
    for context in browser.contexts:
        for page in context.pages:
            if page.url.split("#")[0].split("?")[0] == address:
                return page
    return None


class Copy:
    """One running copy of the app: its process, folder and DevTools port."""

    def __init__(self, exe, port, name):
        self.port = port
        self.data = tempfile.mkdtemp(prefix=f"ytmp-e2e-{name}-")
        env = dict(os.environ, YTMP_E2E_PORT=str(port), YTMP_E2E_DATA=self.data)
        self.process = subprocess.Popen([str(exe)], env=env)
        wait_for(f"DevTools of the {name} copy", lambda: devtools_up(port), timeout=40)

    def connect(self, playwright):
        self.browser = playwright.chromium.connect_over_cdp(f"http://127.0.0.1:{self.port}")
        return self.browser

    def diary(self):
        path = pathlib.Path(self.data, "session.log")
        return path.read_text(encoding="utf-8", errors="replace") if path.exists() else ""

    def stop(self):
        try:
            self.browser.close()
        except Exception:
            pass
        self.process.terminate()
        try:
            self.process.wait(timeout=10)
        except subprocess.TimeoutExpired:
            self.process.kill()


# --- Native checks (Win32) -----------------------------------------------------

def windows_of(pid, class_name):
    """Handles of the visible-or-not top-level windows of `pid` with that class."""
    user32 = ctypes.windll.user32
    found = []
    proc = ctypes.WINFUNCTYPE(ctypes.c_bool, ctypes.c_void_p, ctypes.c_void_p)

    def each(hwnd, _):
        owner = ctypes.c_ulong()
        user32.GetWindowThreadProcessId(hwnd, ctypes.byref(owner))
        name = ctypes.create_unicode_buffer(256)
        user32.GetClassNameW(hwnd, name, 256)
        if owner.value == pid and name.value == class_name:
            found.append(hwnd)
        return True

    user32.EnumWindows(proc(each), 0)
    return found


def topmost(hwnd):
    GWL_EXSTYLE, WS_EX_TOPMOST = -20, 0x8
    return bool(ctypes.windll.user32.GetWindowLongW(hwnd, GWL_EXSTYLE) & WS_EX_TOPMOST)


def visible(hwnd):
    return bool(ctypes.windll.user32.IsWindowVisible(hwnd))


# --- The steps -------------------------------------------------------------------

def start_screen(copy):
    print("Start screen")
    browser = copy.browser
    home = wait_for("the start screen", lambda: find_page(browser, HOME), timeout=30)
    home.wait_for_load_state()
    check(copy.diary().find("YTMP-E2E-BUILD-NOT-FOR-RELEASE") >= 0, "the diary says this is the test build")
    wait_for("the welcome tour", lambda: home.is_visible("#welcome"), timeout=10)
    home.click("#welcome-skip")
    check(wait_for("the tour to close", lambda: home.is_hidden("#welcome"), timeout=5), "the welcome tour closes with Skip")
    check(len(home.query_selector_all("button.key[data-id]")) >= 9 and home.is_visible("#settings"),
          "the keypad (every service and My music) and the gear are there")
    return home


def settings_window(copy, home):
    print("Settings")
    player = wait_for("the player window", lambda: windows_of(copy.process.pid, "YtMiniPlayerWindow"))[0]
    before = topmost(player)
    home.click("#settings")
    settings = wait_for("the Settings page", lambda: find_page(copy.browser, SETTINGS))
    settings.wait_for_load_state()
    window = wait_for("the Settings window", lambda: windows_of(copy.process.pid, "YtMiniPlayerSettings"))[0]
    check(wait_for("it to show", lambda: visible(window), timeout=5), "the gear opens the Settings window")
    # Settings opens on General; "Always on top" is under Window.
    settings.click("#tab-window")
    # A locator, not an element: the page is redrawn after every change.
    switch = settings.locator('[data-setting="pin"]')
    switch.wait_for(timeout=10000)
    check((switch.get_attribute("aria-checked") == "true") == before, "Always on top shows the player's real state")
    switch.click()
    check(wait_for("the player to change", lambda: topmost(player) != before, timeout=5) is True,
          "Always on top in Settings changes the player window (WS_EX_TOPMOST)")
    switch.click()
    check(wait_for("the player to change back", lambda: topmost(player) == before, timeout=5) is True, "and back")
    settings.keyboard.press("Escape")
    check(wait_for("it to hide", lambda: not visible(window), timeout=5) is True, "Escape hides the Settings window")


def open_together(copy, home, name):
    home.click("#together")
    panel = wait_for(f"the {name} Watch together page", lambda: find_page(copy.browser, TOGETHER))
    panel.wait_for_load_state()
    return panel


def set_relay(panel, relay):
    if panel.is_visible("#server-change"):
        panel.click("#server-change")
    panel.fill("#relay", relay)
    panel.click("#relay-save")
    wait_for("the relay address", lambda: relay.replace("ws://", "") in panel.inner_text("#server-addr"), timeout=5)


def watch_together(host, host_home, guest, guest_home, relay):
    print("Watch together (two copies)")
    host_panel = open_together(host, host_home, "host")
    guest_panel = open_together(guest, guest_home, "guest")
    for panel in (host_panel, guest_panel):
        set_relay(panel, relay)
    host_panel.fill("#name", "Host")
    host_panel.click("#start")
    code = wait_for("the room's code", lambda: host_panel.inner_text("#code-text").strip(), timeout=20)
    # Shown in groups with a dash of their own between them (N5J9 – S6HE – AFQ6).
    letters = "".join(ch for ch in code if ch.isalnum())
    check(len(letters) == 12, f"the host gets a code ({letters})")
    guest_panel.fill("#name", "Guest")
    guest_panel.fill("#code", code)
    guest_panel.click("#join")
    check(wait_for("the guest to be in", lambda: guest_panel.is_visible("#v-guest"), timeout=20) is True,
          "the second copy joins with the code")
    # People only: while the host is alone, the list has a "nobody yet" line.
    people = lambda: len(host_panel.query_selector_all("#host-people li:not(.empty)"))
    check(wait_for("the host to see the guest", lambda: people() == 2, timeout=20) is True
          and "Guest" in host_panel.inner_text("#host-people"), "the host sees the guest")
    check("Host" in guest_panel.inner_text("#guest-people"), "the guest sees the host by name")
    guest_panel.click("#leave")
    check(wait_for("the guest to leave", lambda: people() == 1, timeout=20) is True,
          "leaving: the host sees it")


def self_test():
    """The plumbing against a plain Chromium: CDP, finding pages by address."""
    print("Self-test (plain Chromium over CDP)")
    port = 9339
    with sync_playwright() as p:
        server = p.chromium.launch(args=[f"--remote-debugging-port={port}"])
        page = server.new_page()
        page.route("https://ytmp.localhost/**", lambda route: route.fulfill(
            status=200, content_type="text/html", body="<!doctype html><title>t</title><p id=x>ok</p>"))
        page.goto(SETTINGS)
        check(wait_for("DevTools", lambda: devtools_up(port), timeout=10) is not None, "DevTools answers")
        browser = p.chromium.connect_over_cdp(f"http://127.0.0.1:{port}")
        found = find_page(browser, SETTINGS)
        check(found is not None and found.inner_text("#x") == "ok", "the page is found by its address")
        check(find_page(browser, TOGETHER) is None, "and only that one")
        # Like the app opening a window: a page that appears after
        # connecting, made without going through this Playwright connection.
        request = urllib.request.Request(f"http://127.0.0.1:{port}/json/new?about:blank", method="PUT")
        urllib.request.urlopen(request, timeout=5).close()
        try:
            later = wait_for("the new page", lambda: find_page(browser, "about:blank"), timeout=5)
        except TimeoutError:
            later = None
        check(later is not None, "a page that appears after connecting is found too")
        try:
            wait_for("nothing", lambda: None, timeout=0.5)
            check(False, "waiting gives up")
        except TimeoutError:
            check(True, "waiting gives up")
        browser.close()
        server.close()


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--exe", default=str(ROOT / "target" / "release" / "yt-mini-player.exe"))
    parser.add_argument("--relay", help="ws://127.0.0.1:8799 (wrangler dev in server/together-relay)")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
    else:
        if sys.platform != "win32":
            sys.exit("windows_e2e.py needs Windows (use --self-test elsewhere)")
        exe = pathlib.Path(args.exe)
        if not exe.exists():
            sys.exit(f"{exe} not found: cargo build --release --features e2e")
        copies = []
        try:
            with sync_playwright() as p:
                host = Copy(exe, 9333, "host")
                copies.append(host)
                host.connect(p)
                host_home = start_screen(host)
                settings_window(host, host_home)
                if args.relay:
                    guest = Copy(exe, 9334, "guest")
                    copies.append(guest)
                    guest.connect(p)
                    guest_home = start_screen(guest)
                    watch_together(host, host_home, guest, guest_home, args.relay)
                else:
                    print("Watch together: skipped (no --relay)")
        except Exception as error:
            failures.append(f"error: {error}")
            print(f"  FAIL {error}")
            for copy in copies:
                print(f"--- diary of {copy.data} ---\n{copy.diary()[-3000:]}")
        finally:
            for copy in copies:
                copy.stop()
    print(f"\n{'FAILED: ' + str(len(failures)) if failures else 'all passed'}")
    return 1 if failures else 0


if __name__ == "__main__":
    sys.exit(main())
