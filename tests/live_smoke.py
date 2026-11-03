"""Smoke test against the real services (weekly, .github/workflows/live-smoke.yml).

Services change their pages without notice, and the first to find out used
to be the users. This opens the real pages in Chromium with the app's page
scripts, injected as webview.rs does, and checks what the app relies on:

- YouTube's watch page: the self-check in ui.js finds the player by its usual
  name, its API, the video and the full-screen button.
- Google's sign-in page still has its email field and does not refuse the
  browser.
- Every service in the catalog (services.rs) still answers at its address.

A site that blocks the runner (a bot check, 403, 429, a consent page) makes
the result "inconclusive", not a failure: GitHub's machines are often
treated as bots. The exit code is 1 only when something the app relies on
has changed.

    python tests/live_smoke.py [--report FILE]      against the Internet
    python tests/live_smoke.py --offline [--broken]  against stand-ins, to test
                                                     this script (the suite does)
"""
import argparse
import json
import pathlib
import re
import sys

from playwright.sync_api import sync_playwright

ROOT = pathlib.Path(__file__).resolve().parent.parent
sys.path.insert(0, str(ROOT / "tests"))
from bundle import add_app_scripts  # noqa: E402

# "Me at the zoo": YouTube's first video, up since 2005.
WATCH_URL = "https://www.youtube.com/watch?v=jNQXAC9IVRw"
SIGN_IN_URL = "https://accounts.google.com/ServiceLogin?service=youtube"
HEALTHY = {"site": "youtube", "page": "watch", "player": "main", "api": True, "video": True, "fullscreenButton": True}
BLOCKED_STATUS = {401, 403, 429}


def services():
    """(id, address) of every service in the catalog."""
    src = (ROOT / "src" / "services.rs").read_text(encoding="utf-8")
    return re.findall(r'id: "([^"]+)",.*?url: "(https://[^"]+)"', src, re.S)


def offline_routes(page, broken):
    """Stand-ins for the services (a renamed player and a missing page when
    `broken`)."""
    from ui_e2e import MOCK_HTML

    watch = MOCK_HTML
    if broken:
        watch = watch.replace('id="movie_player" class="html5-video-player"', 'id="new_player" class="player-v2"')
        watch = watch.replace("document.getElementById('movie_player')", "document.getElementById('new_player')")
        watch = watch.replace('<button class="ytp-fullscreen-button">', '<button class="fs-button">')
        watch = watch.replace("document.querySelector('.ytp-fullscreen-button')", "document.querySelector('.fs-button')")
    sign_in = "<!doctype html><title>Sign in</title><form><input type=email id=identifierId></form>"
    if broken:
        sign_in = "<!doctype html><title>Sign in</title><form><input type=text id=login></form>"

    def handle(route):
        url = route.request.url
        if url.startswith("https://www.youtube.com/watch"):
            route.fulfill(status=200, content_type="text/html; charset=utf-8", body=watch)
        elif url.startswith("https://accounts.google.com/"):
            route.fulfill(status=200, content_type="text/html; charset=utf-8", body=sign_in)
        elif broken and "twitch" in url:
            route.fulfill(status=404, content_type="text/html", body="<!doctype html><title>Not found</title>")
        elif broken and "netflix" in url:
            route.fulfill(status=403, content_type="text/html", body="<!doctype html><title>Forbidden</title>")
        else:
            route.fulfill(status=200, content_type="text/html", body="<!doctype html><title>Service</title><main>ok</main>")

    page.route("**/*", handle)


def sent(page, prefix):
    try:
        messages = page.evaluate("window.__sent ? window.__sent.slice() : []")
    except Exception:
        return []
    return [m[len(prefix):] for m in messages if m.startswith(prefix)]


def check_watch(page):
    response = page.goto(WATCH_URL, wait_until="domcontentloaded", timeout=45000)
    host = page.url.split("/")[2]
    if host.startswith("consent."):
        return "inconclusive", "a consent page was shown instead of the video"
    if response and response.status in BLOCKED_STATUS:
        return "inconclusive", f"YouTube answered {response.status}"
    # ui.js reports when the video plays, or 10 s after loading.
    for _ in range(40):
        page.wait_for_timeout(500)
        checks = sent(page, "page-check:")
        if checks:
            break
    body = page.inner_text("body") if page.query_selector("body") else ""
    if not checks:
        if re.search(r"confirm you.re not a bot|unusual traffic", body, re.I):
            return "inconclusive", "YouTube asked to confirm this is not a bot"
        return "failed", "the page never reported its self-check (ui.js did not run or the page did not load)"
    result = json.loads(checks[-1])
    if result == HEALTHY:
        return "ok", "player, API, video and full-screen button found"
    missing = [k for k in ("api", "video", "fullscreenButton") if not result.get(k)]
    if result.get("player") != "main":
        missing.insert(0, f"player (found as: {result.get('player')})")
    if re.search(r"confirm you.re not a bot", body, re.I):
        return "inconclusive", "bot check shown; partial result: missing " + ", ".join(missing)
    return "failed", "missing: " + ", ".join(missing)


def check_sign_in(page):
    response = page.goto(SIGN_IN_URL, wait_until="domcontentloaded", timeout=45000)
    if response and response.status in BLOCKED_STATUS:
        return "inconclusive", f"Google answered {response.status}"
    try:
        page.wait_for_selector('input[type="email"]', timeout=15000)
    except Exception:
        if sent(page, "page-check:"):
            return "failed", "Google refused the browser (its 'rejected' page)"
        return "failed", "the email field was not found"
    return "ok", "the email field is there"


def check_service(page, url):
    try:
        response = page.goto(url, wait_until="domcontentloaded", timeout=45000)
    except Exception as error:
        return "failed", f"did not load ({str(error).splitlines()[0][:120]})"
    status = response.status if response else 0
    if status in BLOCKED_STATUS:
        return "inconclusive", f"answered {status} (the runner may be blocked)"
    if status >= 400:
        return "failed", f"answered {status}"
    return "ok", f"answered {status} at {page.url.split('?')[0][:80]}"


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--report", help="write the Markdown report here")
    parser.add_argument("--offline", action="store_true", help="stand-ins instead of the Internet")
    parser.add_argument("--broken", action="store_true", help="with --offline: stand-ins that changed")
    args = parser.parse_args()

    rows = []
    with sync_playwright() as p:
        browser = p.chromium.launch()
        context = browser.new_context(viewport={"width": 960, "height": 540}, locale="en-US")
        page = context.new_page()
        add_app_scripts(page)
        if args.offline:
            offline_routes(page, args.broken)
        for name, check in (("YouTube watch page", check_watch), ("Google sign-in", check_sign_in)):
            try:
                rows.append((name, *check(page)))
            except Exception as error:
                rows.append((name, "failed", f"error: {str(error).splitlines()[0][:160]}"))
        for service_id, url in services():
            rows.append((f"{service_id} ({url})", *check_service(page, url)))
        browser.close()

    failed = [r for r in rows if r[1] == "failed"]
    mark = {"ok": "ok", "inconclusive": "inconclusive", "failed": "FAILED"}
    lines = ["# Live smoke test" + (" (offline stand-ins)" if args.offline else ""), "",
             "| Check | Result | Details |", "|---|---|---|"]
    lines += [f"| {name} | {mark[result]} | {details} |" for name, result, details in rows]
    lines += ["", f"{len(failed)} failed, {sum(r[1] == 'inconclusive' for r in rows)} inconclusive, "
              f"{sum(r[1] == 'ok' for r in rows)} ok."]
    report = "\n".join(lines) + "\n"
    print(report)
    if args.report:
        pathlib.Path(args.report).write_text(report, encoding="utf-8")
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main())
