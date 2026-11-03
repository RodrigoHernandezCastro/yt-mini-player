"""E2E: the "Watch together" window: each screen in each language and size,
the sound notice in every state, and the statuses a guest sees."""
import copy
import json

from bundle import fill, i18n
from cases.common import dom_text, prop_hidden
from suite import LANGS, each, expect, test, text
from together_e2e import CSP as TOGETHER_CSP, IPC_STUB as TOGETHER_IPC, views

VIEW_NAMES = list(views())
SIZES = [(380, 600), (320, 480)]
OVERFLOW_JS = """() => {
  const bad = [];
  if (document.documentElement.scrollWidth > document.documentElement.clientWidth + 1) bad.push('page');
  for (const el of document.querySelectorAll('button, h1, h2, h3, .label, .code, .nm, .status, .server-line, .segmented span')) {
    if (!el.offsetParent) continue;
    if (el.scrollWidth > el.clientWidth + 1) bad.push((el.id || el.className || el.tagName) + ': ' + el.textContent.trim().slice(0, 40));
  }
  return bad;
}"""


def base_views():
    return copy.deepcopy(views())


def together_page(env, lang, width=380, height=600):
    page = env.page(width, height)
    page.add_init_script(TOGETHER_IPC)
    env.serve(page, fill("together.html", {"i18n": i18n(lang, "together")}, lang), csp=TOGETHER_CSP)
    page.goto("https://ytmp.localhost/together")
    page.wait_for_timeout(60)
    return page


def push_view(page, view):
    page.evaluate(f"window.__ytmpTogether({json.dumps(view)})")
    page.wait_for_timeout(60)


def sent(page):
    return page.evaluate("window.__sent.slice()")


# --- Every screen fits ---------------------------------------------------------

@each("layout", "together › {0} in {1} at {2}×{3}: everything fits",
      [(v, lang, w, h) for v in VIEW_NAMES for lang in LANGS for (w, h) in SIZES])
def _(env, view, lang, width, height):
    page = together_page(env, lang, width, height)
    push_view(page, base_views()[view])
    bad = page.evaluate(OVERFLOW_JS)
    expect(not bad, bad[:4])
    expect(not page.errors, page.errors[:2])


@each("e2e", "together in {0}: asks for its state, says its language, Escape closes", LANGS)
def _(env, lang):
    page = together_page(env, lang)
    expect(sent(page) == ["together:ready"], sent(page))
    expect(page.evaluate("document.documentElement.lang") == lang, "lang")
    page.evaluate("window.__sent.length = 0")
    page.keyboard.press("Escape")
    expect(sent(page) == ["together:close"], sent(page))


# --- The sound notice (host, sharing the window) -------------------------------

SOUND_CASES = [(problem, whole, app_off) for problem in (None, "unavailable", "silent", "page")
               for whole in (False, True) for app_off in (False, True)]


def sound_expected(problem, whole, app_off):
    """(text key or None, PC button shown, app button shown) as soundBox decides."""
    if problem == "page":
        return "together.sound_page", False, False
    if not whole:
        key = {"unavailable": "together.sound_unavailable", "silent": "together.sound_silent"}.get(problem)
        return key, key is not None, False
    key = {"unavailable": "together.sound_pc_failed", "silent": "together.sound_pc_silent"}.get(problem, "together.sound_pc_on")
    return key, False, not app_off


@each("e2e", "together sound in {0}: problem {1}, whole PC {2}, app sound unavailable {3}",
      [(lang,) + case for lang in LANGS for case in SOUND_CASES])
def _(env, lang, problem, whole, app_off):
    page = together_page(env, lang)
    push_view(page, dict(base_views()["host"], soundProblem=problem, soundWholePc=whole, soundAppUnavailable=app_off))
    key, offer_pc, offer_app = sound_expected(problem, whole, app_off)
    expect(prop_hidden(page, "sound-box") == (key is None), "box shown or hidden wrongly")
    if key:
        expect(dom_text(page, "#sound-text") == text(lang, key), dom_text(page, "#sound-text"))
    expect(prop_hidden(page, "sound-pc") == (not offer_pc), "PC sound button")
    expect(prop_hidden(page, "sound-app") == (not offer_app), "app sound button")
    warn = page.evaluate("document.getElementById('sound-box').classList.contains('warn')")
    expect(warn == bool(problem), "warning colour")


@each("e2e", "together sound: the {0} button sends '{1}'", [("sound-pc", "together:sound:pc"), ("sound-app", "together:sound:app")])
def _(env, button, message):
    page = together_page(env, "en")
    if button == "sound-pc":
        push_view(page, dict(base_views()["host"], soundProblem="silent"))
    else:
        push_view(page, dict(base_views()["host"], soundWholePc=True))
    page.evaluate("window.__sent.length = 0")
    page.click(f"#{button}")
    expect(sent(page) == [message], sent(page))


@test("e2e", "together sound: no notice when the window is not shared")
def _(env):
    page = together_page(env, "en")
    push_view(page, dict(base_views()["host"], sharing=False, soundProblem="silent"))
    expect(prop_hidden(page, "sound-box"), "shown without sharing")


@test("e2e", "together sound: no notice in sync mode")
def _(env):
    page = together_page(env, "en")
    push_view(page, dict(base_views()["host-sync"], sharing=True, soundProblem="silent"))
    expect(prop_hidden(page, "sound-box"), "shown in sync mode")


# --- What the host sees about the guests ----------------------------------------

def guests(n, stream="watching"):
    return [{"id": f"g{i}", "name": f"Guest {i}", "you": False, "online": True, "control": False, "asking": False,
             "stream": stream} for i in range(1, n + 1)]


@each("e2e", "together host in {0}: {1} guest(s) watching", [(lang, n) for lang in LANGS for n in range(1, 6)])
def _(env, lang, count):
    page = together_page(env, lang)
    view = base_views()["host"]
    view["members"] = [view["members"][0]] + guests(count)
    push_view(page, view)
    expect(dom_text(page, "#mode-status-text") == text(lang, "together.sharing_now", count=count), dom_text(page, "#mode-status-text"))
    expect(page.get_attribute("#mode-status", "data-tone") == "live", "tone")


HOST_STATES = {
    "locked": (lambda v: dict(v, locked=True), "together.sharing_paused"),
    "connecting": (lambda v: dict(v, members=[v["members"][0]] + guests(1, "connecting")), "together.sharing_connecting"),
    "waiting": (lambda v: dict(v, members=[v["members"][0]]), "together.sharing_waiting"),
    "sync": (lambda v: dict(v, mode="sync"), "together.sync_status"),
}


@each("e2e", "together host in {0}: the status line when {1}", [(lang, s) for lang in LANGS for s in HOST_STATES])
def _(env, lang, state):
    change, key = HOST_STATES[state]
    page = together_page(env, lang)
    push_view(page, change(base_views()["host"]))
    expect(dom_text(page, "#mode-status-text") == text(lang, key), dom_text(page, "#mode-status-text"))
    expect(page.get_attribute("#mode-status", "data-tone") == "idle", "tone")


# --- What a guest sees ----------------------------------------------------------

GUEST_STATUSES = [("in-sync", "ok"), ("syncing", "busy"), ("host-ad", "warn"), ("cannot-seek", "warn"),
                  ("host-elsewhere", "idle")]


@each("e2e", "together guest in {0}: status '{1}' reads right and has its colour",
      [(lang, s, tone) for lang in LANGS for s, tone in GUEST_STATUSES])
def _(env, lang, status, tone):
    page = together_page(env, lang)
    push_view(page, dict(base_views()["guest"], status=status))
    key = "together.status." + status.replace("-", "_")
    expect(dom_text(page, "#status-text") == text(lang, key), dom_text(page, "#status-text"))
    expect(page.get_attribute("#status", "data-tone") == tone, page.get_attribute("#status", "data-tone"))


@each("e2e", "together guest in {0}: who hosts", LANGS)
def _(env, lang):
    page = together_page(env, lang)
    push_view(page, base_views()["guest"])
    expect(dom_text(page, "#lede") == text(lang, "together.watching_with", name="Rodrigo"), dom_text(page, "#lede"))


WINDOW_STATES = [("guest-share", {}, "together.window.watching", "ok"),
                 ("guest-share-closed", {}, "together.window.closed", None),
                 ("guest-share-control", {}, "together.window.connecting", None),
                 ("guest-share", {"paused": True}, "together.window.paused", "warn")]


@each("e2e", "together guest in {0}: the host's window, {2}",
      [(lang, view, key.rsplit(".", 1)[1]) for lang in LANGS for view, _, key, _ in WINDOW_STATES])
def _(env, lang, view, state):
    _, change, key, tone = next(w for w in WINDOW_STATES if w[0] == view and w[2].endswith(state))
    page = together_page(env, lang)
    push_view(page, dict(base_views()[view], **change))
    expect(dom_text(page, "#status-text") == text(lang, key, name="Rodrigo"), dom_text(page, "#status-text"))
    if tone:
        expect(page.get_attribute("#status", "data-tone") == tone, page.get_attribute("#status", "data-tone"))


@each("e2e", "together guest: the {0} button sends '{1}'",
      [("open-site", "together:open-site"), ("resync", "together:resync"), ("leave", "together:leave")])
def _(env, button, message):
    page = together_page(env, "en")
    push_view(page, base_views()["guest-site"])
    page.evaluate("window.__sent.length = 0")
    page.click(f"#{button}")
    expect(sent(page) == [message], sent(page))


@each("e2e", "together in {0}: why a session ended", LANGS)
def _(env, lang):
    page = together_page(env, lang)
    push_view(page, base_views()["ended"])
    expect(dom_text(page, "#ended-text") == text(lang, "together.ended_bad_code"), dom_text(page, "#ended-text"))
    page.evaluate("window.__sent.length = 0")
    page.click("#dismiss")
    expect(sent(page) == ["together:dismiss"], sent(page))
