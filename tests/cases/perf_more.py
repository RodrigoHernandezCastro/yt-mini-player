"""PERFORMANCE: drawing times, redraw cost, memory, idle work and file budgets.

Budgets are generous (a slow CI machine passes them); a failure means the
page got several times slower, not a few milliseconds.
"""
import json
import time

from bundle import HOST_KEY, IPC_STUB, LANGUAGES, fill, i18n, strings
from cases.common import heap_used, locale_payload, open_menu
from cases.together_more import base_views, together_page
from home_e2e import home_html
from suite import LANGS, SECTIONS, SIDEBAR_HOST, SRC, each, expect, own_long_tasks, settings_state, shadow, test

LONGTASKS = ("window.__long = []; new PerformanceObserver((l) => l.getEntries().forEach((e) => "
             "window.__long.push(Math.round(e.duration)))).observe({ type: 'longtask', buffered: true });")


def timed(page, js, arg=None):
    return page.evaluate(f"(arg) => {{ const t = performance.now(); ({js})(arg); return performance.now() - t; }}", arg)


# --- Drawing -------------------------------------------------------------------

@each("performance", "start screen in {0} at {1}×{2}: drawn within 500 ms",
      [(l, w, h) for l in LANGS for (w, h) in [(640, 360), (480, 270), (960, 540)]])
def _(env, lang, width, height):
    page = env.page(width, height)
    page.add_init_script(IPC_STUB)
    env.serve(page, home_html(lang=lang))
    start = time.perf_counter()
    page.goto("https://ytmp.localhost/")
    page.wait_for_selector(".key")
    elapsed = (time.perf_counter() - start) * 1000
    expect(elapsed < 500, f"{elapsed:.0f} ms")


@each("performance", "settings › {1} in {0}: drawn within 250 ms of its state", [(l, s) for l in LANGS for s in SECTIONS])
def _(env, lang, section):
    page = env.settings(lang=lang, push=False)
    elapsed = timed(page, "(state) => window.__ytmpSettings(state)", dict(settings_state(), section=section))
    expect(elapsed < 250, f"{elapsed:.0f} ms")


@each("performance", "settings › {0}: 100 state updates in under 800 ms", SECTIONS)
def _(env, section):
    page = env.settings(section=section)
    elapsed = timed(page, """(state) => { for (let i = 0; i < 100; i++)
        window.__ytmpSettings(Object.assign({}, state, { opacity: 20 + (i % 17) * 5 })); }""", settings_state())
    expect(elapsed < 800, f"{elapsed:.0f} ms")


@each("performance", "settings › {0}: 50 redraws leave the same number of elements", SECTIONS)
def _(env, section):
    page = env.settings(section=section)
    before = page.evaluate("document.getElementsByTagName('*').length")
    page.evaluate("(state) => { for (let i = 0; i < 50; i++) window.__ytmpSettings(state); }", settings_state())
    after = page.evaluate("document.getElementsByTagName('*').length")
    expect(after == before, f"{before} → {after}")


@each("performance", "settings in {0}: switching sections 120 times takes under 1.5 s", LANGS)
def _(env, lang):
    page = env.settings(lang=lang)
    elapsed = timed(page, """(ids) => { for (let i = 0; i < 120; i++)
        document.getElementById('tab-' + ids[i % ids.length]).click(); }""", SECTIONS)
    expect(elapsed < 1500, f"{elapsed:.0f} ms")


@each("performance", "settings › {0}: 500 updates keep the JS heap within 4 MB", SECTIONS)
def _(env, section):
    page = env.settings(section=section)
    cdp = page.context.new_cdp_session(page)
    cdp.send("HeapProfiler.enable")
    page.evaluate("(state) => { for (let i = 0; i < 20; i++) window.__ytmpSettings(state); }", settings_state())
    before = heap_used(cdp)
    page.evaluate("""(state) => { for (let i = 0; i < 500; i++)
        window.__ytmpSettings(Object.assign({}, state, { opacity: 20 + (i % 17) * 5 })); }""", settings_state())
    after = heap_used(cdp)
    expect(after - before < 4 * 1024 * 1024, f"+{(after - before) / 1024:.0f} KB")


@each("performance", "settings › {0}: no long tasks while drawing", SECTIONS)
def _(env, section):
    page = env.page(760, 560)
    page.add_init_script(LONGTASKS)
    page.add_init_script("window.__sent = []; Object.defineProperty(window, 'ipc', { value: Object.freeze({ "
                         "postMessage: (m) => window.__sent.push(String(m)) }) });")
    env.serve(page, fill("settings.html", {"i18n": i18n("en", "settings")}))
    tasks = own_long_tasks(page, "https://ytmp.localhost/settings", settle=150, after=lambda: page.evaluate(
        "(state) => window.__ytmpSettings(state)", dict(settings_state(), section=section)))
    expect(tasks == [], tasks)


@each("performance", "settings › {1} in {0}: under 400 elements", [(l, s) for l in LANGS for s in SECTIONS])
def _(env, lang, section):
    page = env.settings(lang=lang, section=section)
    count = page.evaluate("document.getElementsByTagName('*').length")
    expect(count < 400, count)


@each("performance", "start screen in {0}: under 450 elements, welcome included", LANGS)
def _(env, lang):
    page = env.home(lang=lang, welcome=True, sidebar=False)
    count = page.evaluate("document.getElementsByTagName('*').length")
    expect(count < 450, count)


@each("performance", "start screen in {0}: no long tasks while loading", LANGS)
def _(env, lang):
    page = env.page()
    page.add_init_script(LONGTASKS)
    page.add_init_script(IPC_STUB)
    env.serve(page, home_html(lang=lang, welcome=True))
    tasks = own_long_tasks(page, "https://ytmp.localhost/")
    expect(tasks == [], tasks)


@each("performance", "{0} in {1}: no request leaves the app", [(k, l) for k in ("home", "settings", "welcome") for l in LANGS])
def _(env, kind, lang):
    page = env.settings(lang=lang) if kind == "settings" else env.home(lang=lang, welcome=kind == "welcome")
    page.wait_for_timeout(150)
    expect(not page.external, page.external[:3])


@each("performance", "welcome in {0}: walking the four steps takes under 150 ms", LANGS)
def _(env, lang):
    page = env.home(lang=lang, welcome=True, sidebar=False)
    elapsed = timed(page, "() => { for (let i = 0; i < 3; i++) document.getElementById('welcome-next').click(); }")
    expect(elapsed < 150, f"{elapsed:.0f} ms")


@test("performance", "start screen: 200 language changes take under 3 s")
def _(env):
    page = env.home()
    locales = [locale_payload(lang, "home") for lang in LANGS]
    elapsed = timed(page, f"""(locales) => {{ for (let i = 1; i <= 200; i++)
        window.__ytmpLocale({json.dumps(HOST_KEY)}, locales[i % locales.length]); }}""", locales)
    expect(elapsed < 3000, f"{elapsed:.0f} ms")


@test("performance", "start screen: 100 language changes keep the JS heap within 4 MB")
def _(env):
    page = env.home()
    cdp = page.context.new_cdp_session(page)
    cdp.send("HeapProfiler.enable")
    locales = [locale_payload(lang, "home") for lang in LANGS]
    page.evaluate(f"(l) => {{ for (let i = 1; i <= 10; i++) window.__ytmpLocale({json.dumps(HOST_KEY)}, l[i % l.length]); }}", locales)
    before = heap_used(cdp)
    page.evaluate(f"(l) => {{ for (let i = 1; i <= 100; i++) window.__ytmpLocale({json.dumps(HOST_KEY)}, l[i % l.length]); }}", locales)
    after = heap_used(cdp)
    expect(after - before < 4 * 1024 * 1024, f"+{(after - before) / 1024:.0f} KB")


@test("performance", "start screen: 100 last-service changes take under 1 s")
def _(env):
    page = env.home()
    ids = ["youtube", "netflix", "twitch", "music"]
    elapsed = timed(page, f"""(ids) => {{ for (let i = 0; i < 100; i++)
        window.__ytmpSync({json.dumps(HOST_KEY)}, {{ lastService: ids[i % ids.length] }}); }}""", ids)
    expect(elapsed < 1000, f"{elapsed:.0f} ms")


# --- The side bar ----------------------------------------------------------------

@test("performance", "side bar: 200 state syncs in under 1 s")
def _(env):
    page = env.sidebar()
    elapsed = timed(page, f"""(host) => {{ for (let i = 0; i < 200; i++)
        window.__ytmpSync({json.dumps(HOST_KEY)}, Object.assign({{}}, host, {{ pinned: i % 2 === 0, opacity: 20 + (i % 9) * 10 }})); }}""",
                    SIDEBAR_HOST)
    expect(elapsed < 1000, f"{elapsed:.0f} ms")


@test("performance", "side bar: 100 notices in under 500 ms")
def _(env):
    page = env.sidebar()
    elapsed = timed(page, f"() => {{ for (let i = 0; i < 100; i++) window.__ytmpToast({json.dumps(HOST_KEY)}, 'notice ' + i); }}")
    expect(elapsed < 500, f"{elapsed:.0f} ms")


@test("performance", "side bar: 100 language changes in under 2 s")
def _(env):
    page = env.sidebar()
    locales = [locale_payload(lang) for lang in LANGS]
    elapsed = timed(page, f"""(l) => {{ for (let i = 1; i <= 100; i++)
        window.__ytmpLocale({json.dumps(HOST_KEY)}, l[i % l.length]); }}""", locales)
    expect(elapsed < 2000, f"{elapsed:.0f} ms")


@test("performance", "side bar: 300 syncs keep the JS heap within 4 MB")
def _(env):
    page = env.sidebar()
    cdp = page.cdp
    cdp.send("HeapProfiler.enable")
    before = heap_used(cdp)
    page.evaluate(f"""(host) => {{ for (let i = 0; i < 300; i++)
        window.__ytmpSync({json.dumps(HOST_KEY)}, Object.assign({{}}, host, {{ opacity: 20 + (i % 9) * 10 }})); }}""", SIDEBAR_HOST)
    after = heap_used(cdp)
    expect(after - before < 4 * 1024 * 1024, f"+{(after - before) / 1024:.0f} KB")


def shadow_count(page):
    """Elements of the side bar (inside its closed shadow root), notices
    left out: they come and go on their own clock."""
    doc = page.cdp.send("DOM.getDocument", {"depth": -1, "pierce": True})

    def count(node, inside):
        attrs = node.get("attributes", [])
        classes = dict(zip(attrs[::2], attrs[1::2])).get("class", "")
        if "toasts" in classes.split():
            return 0
        total = 1 if inside and node.get("nodeType") == 1 else 0
        for child in node.get("children", []) + node.get("shadowRoots", []):
            total += count(child, inside or node.get("nodeName") == "YTMP-UI")
        return total

    return count(doc["root"], False)


def menu_tags(page):
    html = shadow(page, ".panel")["html"]
    return html.count("<") - html.count("</")


@test("performance", "side bar: under 300 elements with the menu closed")
def _(env):
    page = env.sidebar()
    n = shadow_count(page)
    expect(0 < n < 300, n)


@each("performance", "side bar in {0}: under 700 elements with the menu open", LANGS)
def _(env, lang):
    page = env.sidebar(lang=lang)
    open_menu(page, lang)
    n = shadow_count(page)
    expect(0 < n < 700, n)


@test("performance", "side bar: opening the menu 10 times does not pile up elements")
def _(env):
    page = env.sidebar()
    open_menu(page, "en")
    first = menu_tags(page)
    for _ in range(9):
        page.keyboard.press("Escape")
        open_menu(page, "en")
    expect(menu_tags(page) == first, f"{first} → {menu_tags(page)}")


@test("performance", "side bar: the bar's labels are not rebuilt on every sync (no node churn)")
def _(env):
    page = env.sidebar()
    before = shadow_count(page)
    page.evaluate(f"(host) => {{ for (let i = 0; i < 50; i++) window.__ytmpSync({json.dumps(HOST_KEY)}, host); }}", SIDEBAR_HOST)
    expect(shadow_count(page) == before, f"{before} → {shadow_count(page)}")


@test("performance", "side bar: notices clean up after themselves")
def _(env):
    page = env.sidebar()
    for i in range(3):
        page.evaluate(f"window.__ytmpToast({json.dumps(HOST_KEY)}, 'zq-notice-{i}')")
    page.wait_for_timeout(4200)
    html = shadow(page, ".toasts")["html"]
    expect("zq-notice-" not in html, "notices left behind")


# --- The together window --------------------------------------------------------

@each("performance", "together in {0}: 200 state updates in under 1.5 s", LANGS)
def _(env, lang):
    page = together_page(env, lang)
    views = list(base_views().values())
    elapsed = timed(page, "(views) => { for (let i = 0; i < 200; i++) window.__ytmpTogether(views[i % views.length]); }", views)
    expect(elapsed < 1500, f"{elapsed:.0f} ms")


@test("performance", "together: 50 redraws of the host screen leave the same number of elements")
def _(env):
    page = together_page(env, "en")
    host = base_views()["host"]
    page.evaluate("(v) => window.__ytmpTogether(v)", host)
    before = page.evaluate("document.getElementsByTagName('*').length")
    page.evaluate("(v) => { for (let i = 0; i < 50; i++) window.__ytmpTogether(v); }", host)
    after = page.evaluate("document.getElementsByTagName('*').length")
    expect(after == before, f"{before} → {after}")


@test("performance", "together: 500 updates keep the JS heap within 4 MB")
def _(env):
    page = together_page(env, "en")
    cdp = page.context.new_cdp_session(page)
    cdp.send("HeapProfiler.enable")
    views = list(base_views().values())
    page.evaluate("(views) => views.forEach((v) => window.__ytmpTogether(v))", views)
    before = heap_used(cdp)
    page.evaluate("(views) => { for (let i = 0; i < 500; i++) window.__ytmpTogether(views[i % views.length]); }", views)
    after = heap_used(cdp)
    expect(after - before < 4 * 1024 * 1024, f"+{(after - before) / 1024:.0f} KB")


# --- Sizes of what ships -----------------------------------------------------------

FILE_BUDGETS = {"locales/en.json": 80_000, "locales/es.json": 88_000, "locales/ru.json": 130_000,
                "locales/pl.json": 94_000, "locales/ja.json": 104_000, "music.html": 136_000, "cast.html": 28_000}


@each("performance", "{0} stays under {1:,} bytes", list(FILE_BUDGETS.items()))
def _(env, file, budget):
    size = len((SRC / file).read_bytes())
    expect(size <= budget, f"{size:,} bytes")


STRING_BUDGETS = {"home": 8_000, "settings": 16_000, "together": 24_000, "stream": 4_000, "lock": 8_000,
                  "mini": 4_000, "music": 42_000, "ui": 10_000}


@each("performance", "the strings embedded for {0} stay under {1:,} bytes in every language", list(STRING_BUDGETS.items()))
def _(env, kind, budget):
    for lang in LANGS:
        size = len(json.dumps(strings(lang, kind), ensure_ascii=False).encode("utf-8"))
        expect(size < budget, f"{lang}: {size:,} bytes")


@test("performance", "the page data for Settings is small (strings and languages)")
def _(env):
    for lang in LANGS:
        size = len(json.dumps({"i18n": i18n(lang, "settings")}, ensure_ascii=False).encode("utf-8"))
        expect(size < 24_000, f"{lang}: {size:,} bytes")
    expect(len(LANGUAGES) == 5, LANGUAGES)
