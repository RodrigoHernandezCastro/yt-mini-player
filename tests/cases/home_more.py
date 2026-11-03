"""E2E: the start screen and the welcome, in every language."""
import json

from bundle import HOST_KEY
from cases.common import LANG_NAMES, dom_text, attr, locale_payload, service_name
from home_e2e import home_html
from suite import LANGS, SERVICE_IDS, each, expect, push_locale, sent, test, text

KEYS = list(enumerate(SERVICE_IDS, 1))
STEPS = ["pick", "move", "controls", "together"]


# --- The nine keys -------------------------------------------------------------

@each("e2e", "start screen in {0}: key {1} says what it opens and opens {2}",
      [(lang, n, service) for lang in LANGS for n, service in KEYS])
def _(env, lang, number, service):
    page = env.home(lang=lang, sidebar=False)
    selector = f".key[data-id='{service}']"
    expected = text(lang, "home.open_service", name=service_name(lang, service), key=number)
    expect(attr(page, selector, "title") == expected, f"{attr(page, selector, 'title')!r} != {expected!r}")
    expect(dom_text(page, selector + " .name span") == service_name(lang, service), dom_text(page, selector + " .name span"))
    page.click(selector)
    expect(sent(page) == [f"service:{service}"], sent(page))
    expect(dom_text(page, selector + " .caption") == text(lang, "home.opening"), dom_text(page, selector + " .caption"))


@each("e2e", "start screen: Numpad{0} opens {1}", KEYS)
def _(env, number, service):
    page = env.home(sidebar=False)
    page.keyboard.press(f"Numpad{number}")
    expect(sent(page) == [f"service:{service}"], sent(page))


@each("e2e", "start screen: Alt+{0} opens nothing", [n for n, _ in KEYS])
def _(env, number):
    page = env.home(sidebar=False)
    page.keyboard.press(f"Alt+Digit{number}")
    expect(sent(page) == [], sent(page))


@each("e2e", "start screen: Ctrl+{0} opens nothing", [n for n, _ in KEYS])
def _(env, number):
    page = env.home(sidebar=False)
    page.keyboard.press(f"Control+Digit{number}")
    expect(sent(page) == [], sent(page))


@each("e2e", "start screen: {0} is not a service key", ["Digit0", "Numpad0", "KeyA", "Space", "Minus"])
def _(env, key):
    page = env.home(sidebar=False)
    page.keyboard.press(key)
    expect(sent(page) == [], sent(page))


@each("e2e", "start screen: two keys in a row open {0}, then {1}",
      [(SERVICE_IDS[i], SERVICE_IDS[(i + 4) % 9]) for i in range(9)])
def _(env, first, second):
    page = env.home(sidebar=False)
    page.keyboard.press(str(SERVICE_IDS.index(first) + 1))
    page.keyboard.press(str(SERVICE_IDS.index(second) + 1))
    expect(sent(page) == [f"service:{first}", f"service:{second}"], sent(page))


@each("e2e", "start screen: the last service mark sits on {0}", SERVICE_IDS)
def _(env, service):
    page = env.home(last=service, sidebar=False)
    expect(page.locator(".last").count() == 1, f"{page.locator('.last').count()} marks")
    expect(page.locator(f".key[data-id='{service}'] .last").count() == 1, "on another key")


@each("e2e", "start screen in {0}: the last service mark says so", LANGS)
def _(env, lang):
    page = env.home(lang=lang, last="netflix", sidebar=False)
    expect(dom_text(page, ".key[data-id='netflix'] .last") == text(lang, "home.last"), dom_text(page, ".last"))


@each("e2e", "start screen: an unknown last service marks nothing ({0!r})", ["nope", "", "YOUTUBE", "music "])
def _(env, last):
    page = env.home(last=last, sidebar=False)
    expect(page.locator(".last").count() == 0, "a key was marked")


@each("e2e", "start screen: the app moves the mark from {0} to {1}",
      [(SERVICE_IDS[i], SERVICE_IDS[(i + 1) % 9]) for i in range(9)])
def _(env, before, after):
    page = env.home(last=before)
    page.evaluate(f"window.__ytmpSync({json.dumps(HOST_KEY)}, {{ lastService: {json.dumps(after)} }})")
    page.wait_for_timeout(50)
    expect(page.locator(f".key[data-id='{after}'] .last").count() == 1 and page.locator(".last").count() == 1, "mark")


# --- The address ---------------------------------------------------------------

ADDRESSES = [
    ("plex.tv", "plex.tv"),
    ("https://www.netflix.com/browse", "https://www.netflix.com/browse"),
    ("   vimeo.com/76979871   ", "vimeo.com/76979871"),
    ("\tdailymotion.com\t", "dailymotion.com"),
    ("пример.рф", "пример.рф"),
    ("例え.jp/パス", "例え.jp/パス"),
    ("youtube.com/watch?v=dQw4w9WgXcQ&t=42", "youtube.com/watch?v=dQw4w9WgXcQ&t=42"),
    ("localhost:8080", "localhost:8080"),
    ("a b c", "a b c"),
    ("1", "1"),
]


@each("e2e", "start screen: the address {0!r} is sent as {1!r}", ADDRESSES)
def _(env, typed, expected):
    page = env.home(sidebar=False)
    page.fill("#site-url", typed)
    page.keyboard.press("Enter")
    expect(sent(page) == [f"open-url:{expected}"], sent(page))


@each("e2e", "start screen: the open button sends {1!r}", ADDRESSES[:4])
def _(env, typed, expected):
    page = env.home(sidebar=False)
    page.fill("#site-url", typed)
    page.click(".open")
    expect(sent(page) == [f"open-url:{expected}"], sent(page))


@each("e2e", "start screen: a blank address ({0!r}) sends nothing", ["", " ", "     ", "\t"])
def _(env, typed):
    page = env.home(sidebar=False)
    page.fill("#site-url", typed)
    page.keyboard.press("Enter")
    expect(sent(page) == [], sent(page))


@each("e2e", "start screen in {0}: the address has its label and example", LANGS)
def _(env, lang):
    page = env.home(lang=lang, sidebar=False)
    expect(page.evaluate("document.getElementById('site-url').labels[0].textContent") == text(lang, "home.other_site"), "label")
    expect(attr(page, "#site-url", "placeholder") == text(lang, "home.other_site_placeholder"), "placeholder")


# --- Header --------------------------------------------------------------------

@each("e2e", "start screen in {0}: title, gear and Watch together in the language", LANGS)
def _(env, lang):
    page = env.home(lang=lang, sidebar=False)
    expect(dom_text(page, "h1") == text(lang, "home.title"), dom_text(page, "h1"))
    expect(attr(page, "#settings", "aria-label") == text(lang, "home.settings"), attr(page, "#settings", "aria-label"))
    expect(attr(page, "#settings", "title") == text(lang, "home.settings_tip"), attr(page, "#settings", "title"))
    expect(dom_text(page, "#together .word") == text(lang, "home.together"), dom_text(page, "#together .word"))
    expect(attr(page, "#together", "title") == text(lang, "home.together_caption"), attr(page, "#together", "title"))


@each("e2e", "start screen in {0}: the gear opens Settings", LANGS)
def _(env, lang):
    page = env.home(lang=lang, sidebar=False)
    page.click("#settings")
    expect(sent(page) == ["settings"], sent(page))


@each("e2e", "start screen in {0}: Watch together opens its window", LANGS)
def _(env, lang):
    page = env.home(lang=lang, sidebar=False)
    page.click("#together")
    expect(sent(page) == ["together"], sent(page))


# --- The language menu ---------------------------------------------------------

@each("e2e", "start screen in {0}: the language button names the language", LANGS)
def _(env, lang):
    page = env.home(lang=lang, sidebar=False)
    expect(attr(page, "#lang", "aria-label") == f"{text(lang, 'lang.label')}: {LANG_NAMES[lang]}", attr(page, "#lang", "aria-label"))
    expect(dom_text(page, "#lang-name") == LANG_NAMES[lang], dom_text(page, "#lang-name"))
    expect(attr(page, "#lang", "aria-expanded") == "false", "says it is open")


@each("e2e", "start screen in {0}: the menu checks the current language and focuses it", LANGS)
def _(env, lang):
    page = env.home(lang=lang, sidebar=False)
    page.click("#lang")
    checked = page.evaluate("[...document.querySelectorAll('#lang-menu [aria-checked=\"true\"]')].map((b) => b.dataset.code)")
    expect(checked == [lang], checked)
    expect(page.evaluate("document.activeElement.dataset.code") == lang, "focus elsewhere")
    expect(attr(page, "#lang", "aria-expanded") == "true", "does not say it is open")


@each("e2e", "start screen in {0}: the menu lists the five languages in their own names", LANGS)
def _(env, lang):
    page = env.home(lang=lang, sidebar=False)
    names = page.evaluate("[...document.querySelectorAll('#lang-menu .native')].map((n) => [n.lang, n.textContent])")
    expect(names == [[code, LANG_NAMES[code]] for code in LANGS], names)


@each("e2e", "start screen in {0}: arrows down to {1} and Enter send it",
      [(a, b) for a in LANGS for b in LANGS if a != b])
def _(env, current, target):
    page = env.home(lang=current, sidebar=False)
    page.click("#lang")
    for _ in range((LANGS.index(target) - LANGS.index(current)) % len(LANGS)):
        page.keyboard.press("ArrowDown")
    page.keyboard.press("Enter")
    expect(sent(page) == [f"lang:{target}"], sent(page))
    expect(page.is_hidden("#lang-menu"), "the menu stayed open")


@each("e2e", "start screen in {0}: arrows up to {1} and Enter send it",
      [(a, b) for a in LANGS for b in LANGS if a != b])
def _(env, current, target):
    page = env.home(lang=current, sidebar=False)
    page.click("#lang")
    for _ in range((LANGS.index(current) - LANGS.index(target)) % len(LANGS)):
        page.keyboard.press("ArrowUp")
    page.keyboard.press("Enter")
    expect(sent(page) == [f"lang:{target}"], sent(page))


@each("e2e", "start screen in {0}: choosing the same language sends nothing", LANGS)
def _(env, lang):
    page = env.home(lang=lang, sidebar=False)
    page.click("#lang")
    page.keyboard.press("Enter")
    expect(sent(page) == [], sent(page))
    expect(page.is_hidden("#lang-menu"), "the menu stayed open")


@test("e2e", "start screen: End and Home jump to the last and first language")
def _(env):
    page = env.home(lang="ru", sidebar=False)
    page.click("#lang")
    page.keyboard.press("End")
    expect(page.evaluate("document.activeElement.dataset.code") == LANGS[-1], "End")
    page.keyboard.press("Home")
    expect(page.evaluate("document.activeElement.dataset.code") == LANGS[0], "Home")
    page.keyboard.press("Enter")
    expect(sent(page) == [f"lang:{LANGS[0]}"], sent(page))


@test("e2e", "start screen: Escape closes the language menu and gives the focus back")
def _(env):
    page = env.home(sidebar=False)
    page.click("#lang")
    page.keyboard.press("Escape")
    expect(page.is_hidden("#lang-menu") and page.evaluate("document.activeElement.id") == "lang", "focus")
    expect(sent(page) == [], sent(page))


@test("e2e", "start screen: a click outside closes the language menu")
def _(env):
    page = env.home(sidebar=False)
    page.click("#lang")
    page.mouse.click(5, page.viewport_size["height"] - 5)
    expect(page.is_hidden("#lang-menu") and attr(page, "#lang", "aria-expanded") == "false", "still open")


@test("e2e", "start screen: the language button opens its menu with ArrowDown")
def _(env):
    page = env.home(sidebar=False)
    page.focus("#lang")
    page.keyboard.press("ArrowDown")
    expect(page.is_visible("#lang-menu") and page.evaluate("document.activeElement.dataset.code") == "en", "not open")


@test("e2e", "start screen: clicking the language button again closes the menu")
def _(env):
    page = env.home(sidebar=False)
    page.click("#lang")
    page.click("#lang")
    expect(page.is_hidden("#lang-menu"), "still open")


@each("e2e", "start screen: from {0} to {1}, the page redraws without reloading",
      [(a, b) for a in LANGS for b in LANGS if a != b])
def _(env, before, after):
    page = env.home(lang=before)
    page.fill("#site-url", "plex.tv")
    page.evaluate("window.__marker = 42")
    push_locale(page, after, "home")
    page.wait_for_timeout(50)
    expect(page.evaluate("window.__marker") == 42, "the page reloaded")
    expect(page.evaluate("document.documentElement.lang") == after, "lang")
    expect(dom_text(page, "h1") == text(after, "home.title"), dom_text(page, "h1"))
    expect(attr(page, ".key[data-id='youtube']", "title") == text(after, "home.open_service", name="YouTube", key=1), "keys")
    expect(dom_text(page, ".key[data-id='music'] .name span") == service_name(after, "music"), "My music's name")
    expect(page.input_value("#site-url") == "plex.tv", "typed text lost")


@each("e2e", "start screen in {0}: a locale for the same language changes nothing", LANGS)
def _(env, lang):
    page = env.home(lang=lang)
    page.evaluate("document.querySelector('h1').textContent = 'kept'")
    page.evaluate(f"window.__ytmpLocale({json.dumps(HOST_KEY)}, {json.dumps(locale_payload(lang, 'home'))})")
    page.wait_for_timeout(50)
    expect(dom_text(page, "h1") == "kept", "redrawn for nothing")


# --- DRM -----------------------------------------------------------------------

DRM_STUBS = {
    "both": "navigator.requestMediaKeySystemAccess = () => Promise.resolve({});",
    "Widevine": "navigator.requestMediaKeySystemAccess = (ks) => ks === 'com.widevine.alpha' ? Promise.resolve({}) : Promise.reject(new Error('no'));",
    "PlayReady": "navigator.requestMediaKeySystemAccess = (ks) => ks.startsWith('com.microsoft.playready') ? Promise.resolve({}) : Promise.reject(new Error('no'));",
    "none": "navigator.requestMediaKeySystemAccess = () => Promise.reject(new Error('no'));",
}


def drm_page(env, lang, stub):
    page = env.page()
    page.add_init_script("window.__sent = []; window.__ytmpHomeSend = (m) => window.__sent.push(String(m));")
    page.add_init_script(DRM_STUBS[stub])
    env.serve(page, home_html(lang=lang))
    page.goto("https://ytmp.localhost/")
    page.wait_for_timeout(250)
    return page


@each("e2e", "start screen in {0}: DRM ready names both systems", LANGS)
def _(env, lang):
    page = drm_page(env, lang, "both")
    shown = dom_text(page, "#drm .text")
    before, _, after = text(lang, "home.drm_ready", systems="\x00").partition("\x00")
    expect(attr(page, "#drm", "data-state") == "ok", attr(page, "#drm", "data-state"))
    expect(shown.startswith(before) and shown.endswith(after) and "Widevine" in shown and "PlayReady" in shown, shown)
    expect(page.locator(".needs").count() == 0 and page.is_hidden("#recheck"), "warnings left")


@each("e2e", "start screen in {0}: only {1} is still DRM ready", [(l, s) for l in LANGS for s in ("Widevine", "PlayReady")])
def _(env, lang, system):
    page = drm_page(env, lang, system)
    expect(attr(page, "#drm", "data-state") == "ok", attr(page, "#drm", "data-state"))
    expect(dom_text(page, "#drm .text") == text(lang, "home.drm_ready", systems=system), dom_text(page, "#drm .text"))


@each("e2e", "start screen in {0}: no DRM says what to do and marks the services that need it", LANGS)
def _(env, lang):
    page = drm_page(env, lang, "none")
    expect(attr(page, "#drm", "data-state") == "missing", attr(page, "#drm", "data-state"))
    expect(dom_text(page, "#drm .text") == text(lang, "home.drm_missing"), dom_text(page, "#drm .text"))
    for service in ("netflix", "crunchyroll", "prime-video", "disney-plus", "hbo-max"):
        expect(dom_text(page, f".key[data-id='{service}'] .caption") == text(lang, "home.needs_drm"), f"{service} caption")
    expect(page.is_visible("#recheck"), "no way to check again")


@test("e2e", "start screen: checking DRM again finds it once it was installed")
def _(env):
    page = drm_page(env, "en", "none")
    page.evaluate(DRM_STUBS["both"])
    page.click("#recheck")
    page.wait_for_timeout(250)
    expect(attr(page, "#drm", "data-state") == "ok", attr(page, "#drm", "data-state"))
    expect(page.locator(".needs, .caption.warn").count() == 0, "warnings left")


@test("e2e", "start screen: services without DRM keep their caption when it is missing")
def _(env):
    page = drm_page(env, "en", "none")
    for service, key in (("youtube", "service.videos_live"), ("twitch", "service.live"), ("music", "service.music_caption")):
        expect(dom_text(page, f".key[data-id='{service}'] .caption") == text("en", key), service)


# --- The welcome ---------------------------------------------------------------

@each("e2e", "welcome in {0}: Next walks the four steps and the last one finishes", LANGS)
def _(env, lang):
    page = env.home(lang=lang, welcome=True, sidebar=False)
    titles = []
    for _ in STEPS:
        titles.append(dom_text(page, "#welcome-title"))
        page.click("#welcome-next")
    expect(titles == [text(lang, f"home.welcome.{s}.title") for s in STEPS], titles)
    expect(page.is_hidden("#welcome") and sent(page) == ["welcome-done"], sent(page))


@each("e2e", "welcome in {0}: step {1} of 4 says where you are",
      [(lang, n) for lang in LANGS for n in range(1, 5)])
def _(env, lang, number):
    page = env.home(lang=lang, welcome=True, sidebar=False)
    for _ in range(number - 1):
        page.click("#welcome-next")
    expect(dom_text(page, "#welcome-count") == text(lang, "home.welcome.count", step=number, count=4), dom_text(page, "#welcome-count"))
    on = page.evaluate("[...document.querySelectorAll('#welcome-dots span')].map((d) => d.classList.contains('on'))")
    expect(on == [i == number - 1 for i in range(4)], on)
    expect(dom_text(page, "#welcome-body").strip(), "no text")


@each("e2e", "welcome in {0}: Next on steps 1–3, Done on the last", LANGS)
def _(env, lang):
    page = env.home(lang=lang, welcome=True, sidebar=False)
    labels = []
    for _ in STEPS:
        labels.append(dom_text(page, "#welcome-next"))
        page.keyboard.press("ArrowRight")
    expect(labels == [text(lang, "home.welcome.next")] * 3 + [text(lang, "home.welcome.done")], labels)


@each("e2e", "welcome in {0}: Back and Skip appear where they belong", LANGS)
def _(env, lang):
    page = env.home(lang=lang, welcome=True, sidebar=False)
    seen = []
    for _ in STEPS:
        seen.append((page.evaluate("document.getElementById('welcome-back').hidden"),
                     page.evaluate("document.getElementById('welcome-skip').hidden")))
        page.keyboard.press("ArrowRight")
    expect(seen == [(True, False), (False, False), (False, False), (False, True)], seen)


@each("e2e", "welcome in {0}: Escape skips it", LANGS)
def _(env, lang):
    page = env.home(lang=lang, welcome=True, sidebar=False)
    page.keyboard.press("Escape")
    expect(page.is_hidden("#welcome") and sent(page) == ["welcome-done"], sent(page))


@each("e2e", "welcome: Skip on step {0} closes it", [1, 2, 3])
def _(env, step):
    page = env.home(welcome=True, sidebar=False)
    for _ in range(step - 1):
        page.click("#welcome-next")
    page.click("#welcome-skip")
    expect(page.is_hidden("#welcome") and sent(page) == ["welcome-done"], sent(page))


@each("e2e", "welcome: ArrowLeft on step {0} goes back one", [2, 3, 4])
def _(env, step):
    page = env.home(welcome=True, sidebar=False)
    for _ in range(step - 1):
        page.keyboard.press("ArrowRight")
    page.keyboard.press("ArrowLeft")
    expect(dom_text(page, "#welcome-title") == text("en", f"home.welcome.{STEPS[step - 2]}.title"), dom_text(page, "#welcome-title"))


@test("e2e", "welcome: ArrowLeft on the first step stays there")
def _(env):
    page = env.home(welcome=True, sidebar=False)
    page.keyboard.press("ArrowLeft")
    expect(dom_text(page, "#welcome-title") == text("en", "home.welcome.pick.title"), dom_text(page, "#welcome-title"))
    expect(page.is_visible("#welcome") and sent(page) == [], sent(page))


@each("e2e", "welcome: number key {0} does nothing while it is open", [n for n, _ in KEYS])
def _(env, number):
    page = env.home(welcome=True, sidebar=False)
    page.keyboard.press(str(number))
    expect(sent(page) == [], sent(page))


@each("e2e", "welcome: changing to {0} on step 3 redraws that step", [l for l in LANGS if l != "en"])
def _(env, lang):
    page = env.home(welcome=True)
    page.keyboard.press("ArrowRight")
    page.keyboard.press("ArrowRight")
    push_locale(page, lang, "home")
    page.wait_for_timeout(50)
    expect(dom_text(page, "#welcome-title") == text(lang, "home.welcome.controls.title"), dom_text(page, "#welcome-title"))
    expect(dom_text(page, "#welcome-count") == text(lang, "home.welcome.count", step=3, count=4), "counter")


@test("e2e", "welcome: finishing twice tells the app once")
def _(env):
    page = env.home(welcome=True, sidebar=False)
    page.keyboard.press("Escape")
    page.keyboard.press("Escape")
    expect(sent(page) == ["welcome-done"], sent(page))
