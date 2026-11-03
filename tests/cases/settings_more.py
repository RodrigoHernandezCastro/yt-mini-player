"""E2E: the Settings window, in every language and from every state."""
from cases.common import LANG_NAMES, attr, dom_text
from suite import (
    EXT_ID, EXT_ID_2, LANGS, SECTIONS, SWITCHES, clear, each, expect, push_settings, sent, settings_button,
    settings_state, test, text,
)

ALL_SWITCHES = SWITCHES + [("window", "fullscreen-in-window", "fullscreenInWindow")]
BUTTONS = [("general", "welcome"), ("together", "together"), ("privacy", "sign-out"), ("privacy", "security"),
           ("extensions", "ext-folder"), ("extensions", "ext-reload"), ("advanced", "help"),
           ("advanced", "open-folder")]


# --- Sections ------------------------------------------------------------------

@each("e2e", "settings in {0}: the {1} section opens from the rail",
      [(lang, s) for lang in LANGS for s in SECTIONS])
def _(env, lang, section):
    page = env.settings(lang=lang)
    page.click(f"#tab-{section}")
    expect(dom_text(page, "#page h2") == text(lang, f"settings.section.{section}"), dom_text(page, "#page h2"))
    expect(dom_text(page, "#page .lede") == text(lang, f"settings.lede.{section}"), dom_text(page, "#page .lede"))
    expect(attr(page, f"#tab-{section}", "aria-selected") == "true", "tab not selected")
    expect(page.locator("[role='tab'][aria-selected='true']").count() == 1, "two selected tabs")


@each("e2e", "settings in {0}: the app can open it at {1}", [(lang, s) for lang in LANGS for s in SECTIONS])
def _(env, lang, section):
    page = env.settings(lang=lang, section=section)
    expect(dom_text(page, "#page h2") == text(lang, f"settings.section.{section}"), dom_text(page, "#page h2"))
    expect(attr(page, "#page", "aria-labelledby") == f"tab-{section}", "panel not linked to its tab")


@each("e2e", "settings in {0}: the rail names the six sections in order", LANGS)
def _(env, lang):
    page = env.settings(lang=lang)
    names = page.evaluate("[...document.querySelectorAll('.tab span')].map((s) => s.textContent)")
    expect(names == [text(lang, f"settings.section.{s}") for s in SECTIONS], names)
    expect(dom_text(page, "#settings-title") == text(lang, "settings.title"), dom_text(page, "#settings-title"))


@each("e2e", "settings: ArrowDown from {0} goes to the next section", SECTIONS)
def _(env, section):
    page = env.settings(section=section)
    page.focus(f"#tab-{section}")
    page.keyboard.press("ArrowDown")
    following = SECTIONS[(SECTIONS.index(section) + 1) % len(SECTIONS)]
    expect(page.evaluate("document.activeElement.id") == f"tab-{following}", page.evaluate("document.activeElement.id"))
    expect(dom_text(page, "#page h2") == text("en", f"settings.section.{following}"), "page not shown")


@each("e2e", "settings: ArrowUp from {0} goes to the previous section", SECTIONS)
def _(env, section):
    page = env.settings(section=section)
    page.focus(f"#tab-{section}")
    page.keyboard.press("ArrowUp")
    previous = SECTIONS[(SECTIONS.index(section) - 1) % len(SECTIONS)]
    expect(page.evaluate("document.activeElement.id") == f"tab-{previous}", page.evaluate("document.activeElement.id"))
    expect(attr(page, f"#tab-{previous}", "tabindex") == "0", "roving tabindex not moved")


@each("e2e", "settings: {0} is kept after the reload a language change makes", SECTIONS)
def _(env, section):
    page = env.settings()
    page.click(f"#tab-{section}")
    page.reload()
    push_settings(page, settings_state())
    expect(dom_text(page, "#page h2") == text("en", f"settings.section.{section}"), dom_text(page, "#page h2"))


@each("e2e", "settings in {0}: says it is ready and Escape closes it", LANGS)
def _(env, lang):
    page = env.settings(lang=lang, push=False)
    expect(sent(page) == ["settings:ready"], sent(page))
    clear(page)
    page.keyboard.press("Escape")
    expect(sent(page) == ["settings:close"], sent(page))


@each("e2e", "settings in {0}: nothing on the page before the app's state", LANGS)
def _(env, lang):
    page = env.settings(lang=lang, push=False)
    expect(page.evaluate("document.getElementById('main').children.length") == 0, "drawn without a state")
    expect(page.locator("[role='tab']").count() == 6, "no rail")


# --- Switches ------------------------------------------------------------------

@each("e2e", "settings in {0}: the '{2}' switch sends the value wanted",
      [(lang,) + s for lang in LANGS for s in ALL_SWITCHES])
def _(env, lang, section, name, field):
    page = env.settings(lang=lang, section=section)
    switch = page.locator(f"[data-setting='{name}']")
    before = switch.get_attribute("aria-checked") == "true"
    expect(before == bool(settings_state()[field]), "does not show the app's state")
    switch.click()
    expect(sent(page) == [f"settings:{name}:{0 if before else 1}"], sent(page))
    expect(switch.get_attribute("aria-checked") == str(not before).lower(), "does not show the change at once")


@each("e2e", "settings in {0}: the '{2}' switch follows the app's state", [(lang,) + s for lang in LANGS for s in ALL_SWITCHES])
def _(env, lang, section, name, field):
    page = env.settings(lang=lang, section=section)
    flipped = not settings_state()[field]
    push_settings(page, settings_state(**{field: flipped}))
    expect(attr(page, f"[data-setting='{name}']", "aria-checked") == str(flipped).lower(), "state ignored")


@each("e2e", "settings: the '{1}' switch, clicked three times, ends changed", ALL_SWITCHES)
def _(env, section, name, field):
    page = env.settings(section=section)
    start = bool(settings_state()[field])
    for _ in range(3):
        page.click(f"[data-setting='{name}']")
    values = [int(not start), int(start), int(not start)]
    expect(sent(page) == [f"settings:{name}:{v}" for v in values], sent(page))


@each("e2e", "settings: the '{1}' switch is named by its row", ALL_SWITCHES)
def _(env, section, name, field):
    page = env.settings(section=section)
    labelled = attr(page, f"[data-setting='{name}']", "aria-labelledby")
    expect(labelled and page.evaluate(f"document.getElementById('{labelled}').textContent").strip(), "no name")
    described = attr(page, f"[data-setting='{name}']", "aria-describedby")
    expect(described and page.evaluate(f"!!document.getElementById('{described}')"), "no description")


# --- Window: opacity, size, corner ---------------------------------------------

def pressed(env, start, key, times=1):
    page = env.settings(section="window", state=settings_state(opacity=start))
    page.focus("[data-setting='opacity']")
    for _ in range(times):
        page.keyboard.press(key)
    page.wait_for_timeout(250)
    return sent(page)


@each("e2e", "settings: opacity {0} %, ArrowRight sends {1} %", [(v, v + 5) for v in range(20, 100, 5)])
def _(env, start, expected):
    messages = pressed(env, start, "ArrowRight")
    expect(messages and set(messages) == {f"settings:opacity:{expected}"}, messages)


@each("e2e", "settings: opacity {0} %, ArrowLeft sends {1} %", [(v, v - 5) for v in range(25, 105, 5)])
def _(env, start, expected):
    messages = pressed(env, start, "ArrowLeft")
    expect(messages and set(messages) == {f"settings:opacity:{expected}"}, messages)


@each("e2e", "settings: opacity at {0} %, {1} sends nothing", [(100, "ArrowRight"), (20, "ArrowLeft")])
def _(env, start, key):
    expect(pressed(env, start, key) == [], "sent a value past the end")


@each("e2e", "settings: opacity from {0} %, {1} goes to {2} %", [(60, "Home", 20), (60, "End", 100), (35, "End", 100),
                                                              (90, "Home", 20)])
def _(env, start, key, expected):
    messages = pressed(env, start, key)
    expect(messages and messages[-1] == f"settings:opacity:{expected}", messages)


@each("e2e", "settings in {0}: opacity {1} % is shown and spoken as a percentage",
      [(lang, v) for lang in LANGS for v in (20, 45, 70, 100)])
def _(env, lang, value):
    page = env.settings(lang=lang, section="window", state=settings_state(opacity=value))
    spoken = attr(page, "[data-setting='opacity']", "aria-valuetext")
    shown = dom_text(page, ".range .value")
    for got in (spoken, shown):
        flat = got.replace(" ", "").replace(" ", "").replace(" ", "")
        expect(flat == f"{value}%", f"{got!r}")
    expect(attr(page, ".range .value", "aria-hidden") == "true", "the number is read twice")


@each("e2e", "settings in {0}: the {1} size button resizes the player", [(l, s) for l in LANGS for s in ("s", "m", "l")])
def _(env, lang, size):
    page = env.settings(lang=lang, section="window")
    key = {"s": "settings.size_small", "m": "settings.size_medium", "l": "settings.size_large"}[size]
    expect(dom_text(page, f"[data-send='size:{size}']") == text(lang, key), dom_text(page, f"[data-send='size:{size}']"))
    settings_button(page, f"size:{size}").click()
    expect(sent(page) == [f"settings:size:{size}"], sent(page))


CORNERS = [("nw", "settings.corner_top_left"), ("ne", "settings.corner_top_right"),
           ("sw", "settings.corner_bottom_left"), ("se", "settings.corner_bottom_right")]


@each("e2e", "settings in {0}: the {1} corner button is named and moves the player",
      [(l, c, k) for l in LANGS for c, k in CORNERS])
def _(env, lang, corner, key):
    page = env.settings(lang=lang, section="window")
    expect(attr(page, f"[data-send='corner:{corner}']", "aria-label") == text(lang, key), "name")
    settings_button(page, f"corner:{corner}").click()
    expect(sent(page) == [f"settings:corner:{corner}"], sent(page))


@each("e2e", "settings in {0}: ghost mode explains how to leave it ({1})",
      [(l, tray) for l in LANGS for tray in ("tray", "no tray")])
def _(env, lang, tray):
    page = env.settings(lang=lang, section="window", state=settings_state(tray=tray == "tray"))
    key = "settings.ghost_sub" if tray == "tray" else "settings.ghost_sub_no_tray"
    expect(dom_text(page, "[data-row='ghost'] .sub") == text(lang, key), dom_text(page, "[data-row='ghost'] .sub"))


# --- General: language ---------------------------------------------------------

@each("e2e", "settings in {0}: choosing {1} as the language", [(a, b) for a in LANGS for b in LANGS if a != b])
def _(env, current, target):
    page = env.settings(lang=current)
    page.click(f"[role='radio'][data-value='{target}']")
    expect(sent(page) == [f"settings:lang:{target}"], sent(page))
    expect(attr(page, f"[role='radio'][data-value='{target}']", "aria-checked") == "true", "not shown at once")


@each("e2e", "settings in {0}: its own language is checked, the only Tab stop, and sends nothing", LANGS)
def _(env, lang):
    page = env.settings(lang=lang)
    checked = page.evaluate("[...document.querySelectorAll('[role=\"radio\"][aria-checked=\"true\"]')].map((r) => r.dataset.value)")
    stops = page.evaluate("[...document.querySelectorAll('[role=\"radio\"][tabindex=\"0\"]')].map((r) => r.dataset.value)")
    expect(checked == [lang] and stops == [lang], f"{checked} {stops}")
    page.click(f"[role='radio'][data-value='{lang}']")
    expect(sent(page) == [], sent(page))


@each("e2e", "settings in {0}: ArrowRight on the language radios chooses the next one", LANGS)
def _(env, lang):
    page = env.settings(lang=lang)
    page.focus(f"[role='radio'][data-value='{lang}']")
    page.keyboard.press("ArrowRight")
    following = LANGS[(LANGS.index(lang) + 1) % len(LANGS)]
    expect(sent(page) == [f"settings:lang:{following}"], sent(page))
    expect(page.evaluate("document.activeElement.dataset.value") == following, "focus not moved")


@each("e2e", "settings in {0}: ArrowLeft on the language radios chooses the previous one", LANGS)
def _(env, lang):
    page = env.settings(lang=lang)
    page.focus(f"[role='radio'][data-value='{lang}']")
    page.keyboard.press("ArrowLeft")
    previous = LANGS[(LANGS.index(lang) - 1) % len(LANGS)]
    expect(sent(page) == [f"settings:lang:{previous}"], sent(page))


@each("e2e", "settings in {0}: each language is written in its own name and script", LANGS)
def _(env, lang):
    page = env.settings(lang=lang)
    radios = page.evaluate("[...document.querySelectorAll('[role=\"radio\"]')].map((r) => [r.dataset.value, r.lang, r.textContent])")
    expect(radios == [[code, code, LANG_NAMES[code]] for code in LANGS], radios)


# --- Buttons -------------------------------------------------------------------

@each("e2e", "settings in {0}: the '{2}' button in {1} sends its message",
      [(lang, section, message) for lang in LANGS for section, message in BUTTONS])
def _(env, lang, section, message):
    page = env.settings(lang=lang, section=section)
    settings_button(page, message).click()
    expect(sent(page) == [f"settings:{message}"], sent(page))


@each("e2e", "settings in {0}: Lock now with the lock set to {1}", [(l, m) for l in LANGS for m in ("off", "hello", "pin")])
def _(env, lang, mode):
    page = env.settings(lang=lang, section="privacy", state=settings_state(lock=mode))
    button = settings_button(page, "lock-now")
    sub = dom_text(page, "[data-row='lock-now'] .sub")
    if mode == "off":
        expect(button.is_disabled(), "enabled without a lock")
        expect(sub == text(lang, "settings.lock_now_off"), sub)
    else:
        expect(sub == text(lang, "settings.lock_now_sub"), sub)
        button.click()
        expect(sent(page) == ["settings:lock-now"], sent(page))


@each("e2e", "settings: lock from {0} to {1}", [(a, b) for a in ("off", "hello", "pin") for b in ("off", "hello", "pin") if a != b])
def _(env, before, after):
    page = env.settings(section="privacy", state=settings_state(lock=before))
    page.click(f"[role='radio'][data-value='{after}']")
    expect(sent(page) == [f"settings:lock:{after}"], sent(page))
    expect(attr(page, f"[role='radio'][data-value='{after}']", "aria-checked") == "true", "not shown")


@each("e2e", "settings: lock radios, {1} from {0}", [(m, k) for m in ("off", "hello", "pin") for k in ("ArrowRight", "ArrowLeft")])
def _(env, mode, key):
    order = ["off", "hello", "pin"]
    page = env.settings(section="privacy", state=settings_state(lock=mode))
    page.focus(f"[role='radio'][data-value='{mode}']")
    page.keyboard.press(key)
    step = 1 if key == "ArrowRight" else -1
    expected = order[(order.index(mode) + step) % 3]
    expect(sent(page) == [f"settings:lock:{expected}"], sent(page))


@each("e2e", "settings in {0}: hosting mode {1} from the other one", [(l, m) for l in LANGS for m in ("share", "sync")])
def _(env, lang, mode):
    other = "share" if mode == "sync" else "sync"
    page = env.settings(lang=lang, section="together", state=settings_state(togetherMode=other))
    card = f"[role='radio'][data-value='{mode}']"
    expect(dom_text(page, card + " strong") == text(lang, f"settings.together_{mode}"), dom_text(page, card + " strong"))
    page.click(card)
    expect(sent(page) == [f"settings:together-mode:{mode}"], sent(page))


@each("e2e", "settings: hosting mode with {0} from {1}", [(k, m) for k in ("ArrowRight", "ArrowDown") for m in ("share", "sync")])
def _(env, key, mode):
    other = "sync" if mode == "share" else "share"
    page = env.settings(section="together", state=settings_state(togetherMode=mode))
    page.focus(f"[role='radio'][data-value='{mode}']")
    page.keyboard.press(key)
    expect(sent(page) == [f"settings:together-mode:{other}"], sent(page))


@each("e2e", "settings in {0}: Watch together says whether a session is on ({1})",
      [(l, a) for l in LANGS for a in ("idle", "active")])
def _(env, lang, active):
    page = env.settings(lang=lang, section="together", state=settings_state(togetherActive=active == "active"))
    key = "settings.together_open_active" if active == "active" else "settings.together_open_sub"
    expect(dom_text(page, "[data-row='together'] .sub") == text(lang, key), dom_text(page, "[data-row='together'] .sub"))


# --- The name ------------------------------------------------------------------

NAMES = [("Ana", "Ana"), ("José Luis", "José Luis"), ("Иван", "Иван"), ("Łukasz", "Łukasz"), ("さくら", "さくら"),
         ("  Bea  ", "Bea"), ("O'Brien", "O'Brien"), ("Zoë & Co", "Zoë & Co"), ("Ñandú", "Ñandú"),
         ("Ana 🎬", "Ana 🎬"), ("x" * 32, "x" * 32), ("名前テスト", "名前テスト")]


@each("e2e", "settings: the name {0!r} is saved as {1!r} with Enter", NAMES)
def _(env, typed, expected):
    page = env.settings(section="together")
    page.fill("[data-setting='together-name']", typed)
    page.keyboard.press("Enter")
    expect(sent(page)[-1:] == [f"settings:together-name:{expected}"], sent(page))
    expect(len(sent(page)) == 1, f"sent more than once: {sent(page)}")


@each("e2e", "settings in {0}: the name says Saved after a pause", LANGS)
def _(env, lang):
    page = env.settings(lang=lang, section="together")
    page.fill("[data-setting='together-name']", "Marta")
    page.wait_for_timeout(900)
    expect(sent(page) == ["settings:together-name:Marta"], sent(page))
    expect(dom_text(page, ".saved") == text(lang, "settings.saved"), dom_text(page, ".saved"))


@each("e2e", "settings in {0}: the name field has its example and stays out of autofill", LANGS)
def _(env, lang):
    page = env.settings(lang=lang, section="together")
    field = "[data-setting='together-name']"
    expect(attr(page, field, "placeholder") == text(lang, "settings.together_name_placeholder"), "placeholder")
    expect(attr(page, field, "autocomplete") == "off" and attr(page, field, "maxlength") == "32", "attributes")


@test("e2e", "settings: the same name again sends nothing")
def _(env):
    page = env.settings(section="together")
    page.fill("[data-setting='together-name']", "Rodrigo")
    page.keyboard.press("Enter")
    expect(sent(page) == [], sent(page))


@test("e2e", "settings: spaces around the same name send nothing")
def _(env):
    page = env.settings(section="together")
    page.fill("[data-setting='together-name']", "   Rodrigo   ")
    page.keyboard.press("Enter")
    expect(sent(page) == [], sent(page))


@test("e2e", "settings: leaving the field saves the name")
def _(env):
    page = env.settings(section="together")
    page.fill("[data-setting='together-name']", "Lucía")
    page.focus("#tab-general")
    expect(sent(page)[:1] == ["settings:together-name:Lucía"], sent(page))


@test("e2e", "settings: a state that arrives while typing is drawn once the field is left")
def _(env):
    page = env.settings(section="together")
    page.click("[data-setting='together-name']")
    push_settings(page, settings_state(togetherActive=True))
    expect(dom_text(page, "[data-row='together'] .sub") == text("en", "settings.together_open_sub"), "redrawn while typing")
    page.focus("#tab-together")
    page.wait_for_timeout(50)
    expect(dom_text(page, "[data-row='together'] .sub") == text("en", "settings.together_open_active"), "never redrawn")


# --- Extensions ----------------------------------------------------------------

def extension(i, **kw):
    ext = {"folder": f"ext{i}", "name": f"Extension {i}", "version": f"{i}.0", "id": chr(97 + i % 16) * 32,
           "enabled": i % 2 == 0, "hasPage": i % 3 == 0, "error": None, "pending": False}
    ext.update(kw)
    return ext


@each("e2e", "settings: {0} extensions are listed, each with its switch", [1, 3, 8, 16])
def _(env, count):
    exts = [extension(i, id=f"{i:02d}" * 16) for i in range(count)]
    page = env.settings(section="extensions", state=settings_state(extensions=exts))
    expect(page.locator(".list li").count() == count, page.locator(".list li").count())
    expect(page.locator(".list [role='switch']").count() == count, "switches")
    expect(page.locator(".list .link").count() == sum(1 for e in exts if e["hasPage"]), "Options buttons")


@each("e2e", "settings in {0}: an extension shows its version in the language", LANGS)
def _(env, lang):
    page = env.settings(lang=lang, section="extensions")
    expect(dom_text(page, ".list li .sub") == text(lang, "settings.ext_version", version="5.9"), dom_text(page, ".list li .sub"))


@each("e2e", "settings in {0}: without extensions it explains how to add them", LANGS)
def _(env, lang):
    page = env.settings(lang=lang, section="extensions", state=settings_state(extensions=[]))
    expect(dom_text(page, "p.empty") == text(lang, "settings.ext_empty"), dom_text(page, "p.empty"))


@each("e2e", "settings in {0}: an installing extension says so", LANGS)
def _(env, lang):
    pending = dict(settings_state()["extensions"][0], pending=True)
    page = env.settings(lang=lang, section="extensions", state=settings_state(extensions=[pending]))
    expect(dom_text(page, ".list li .sub") == text(lang, "settings.ext_installing"), dom_text(page, ".list li .sub"))


@test("e2e", "settings: the enabled extension's switch turns it off")
def _(env):
    page = env.settings(section="extensions")
    page.click(f"[data-send='ext-toggle:{EXT_ID}']")
    expect(sent(page) == [f"settings:ext-toggle:{EXT_ID}"], sent(page))
    expect(attr(page, f"[data-send='ext-toggle:{EXT_ID}']", "aria-checked") == "false", "not shown at once")


@test("e2e", "settings: an extension's switch is named after it")
def _(env):
    page = env.settings(section="extensions")
    expect(attr(page, f"[data-send='ext-toggle:{EXT_ID}']", "aria-label") == "SponsorBlock", "name")
    expect(attr(page, f"[data-send='ext-toggle:{EXT_ID_2}']", "aria-label") == "uBlock Origin Lite", "name")


@test("e2e", "settings: the extensions folder can't be read: the reason comes first")
def _(env):
    page = env.settings(section="extensions", state=settings_state(extensionsError="Access denied"))
    expect(page.evaluate("document.querySelector('#page p.empty')?.textContent") == "Access denied", "no reason")


@test("e2e", "settings: an extension without an id has no switch and no Options")
def _(env):
    loose = dict(settings_state()["extensions"][0], id=None)
    page = env.settings(section="extensions", state=settings_state(extensions=[loose]))
    expect(page.locator(".list [role='switch'], .list .link").count() == 0, "controls for an extension without an id")


@each("e2e", "settings: {0} problem line(s) in the list", [1, 2, 5])
def _(env, count):
    page = env.settings(section="extensions", state=settings_state(extensionProblems=[f"problem {i}" for i in range(count)]))
    expect(page.locator(".list li .sub.warn").count() == count, page.locator(".list li .sub.warn").count())


# --- Advanced ------------------------------------------------------------------

@each("e2e", "settings: graphics {0} now, {1} chosen: restart notice {2}",
      [(now, chosen, "shown" if now != chosen else "hidden") for now in (True, False) for chosen in (True, False)])
def _(env, now, chosen, shown):
    page = env.settings(section="advanced", state=settings_state(hardwareAcceleration=chosen, hardwareAccelerationNow=now))
    expect(page.locator(".notice").count() == (1 if shown == "shown" else 0), shown)


@each("e2e", "settings in {0}: the restart notice says why and restarts", LANGS)
def _(env, lang):
    page = env.settings(lang=lang, section="advanced", state=settings_state(hardwareAcceleration=False))
    expect(dom_text(page, ".notice p") == text(lang, "settings.restart_needed"), dom_text(page, ".notice p"))
    expect(dom_text(page, "[data-send='restart']") == text(lang, "settings.restart_now"), "button")
    settings_button(page, "restart").click()
    expect(sent(page) == ["settings:restart"], sent(page))


@each("e2e", "settings in {0}: an unavailable shortcut gives its reason", LANGS)
def _(env, lang):
    page = env.settings(lang=lang, section="advanced")
    expected = text(lang, "settings.shortcut_unavailable", reason="another app uses it")
    expect(dom_text(page, ".list li.off .sub") == expected, dom_text(page, ".list li.off .sub"))


@each("e2e", "settings: {0} global shortcuts listed", [0, 1, 3, 9])
def _(env, count):
    keys = [{"keys": f"Ctrl+Alt+{i}", "label": f"Action {i}", "available": i != 1, "note": "taken"} for i in range(count)]
    page = env.settings(section="advanced", state=settings_state(hotkeys=keys))
    expect(page.locator(".list li").count() == count, page.locator(".list li").count())
    expect(page.locator(".list li.off").count() == (1 if count > 1 else 0), "unavailable ones")


@each("e2e", "settings in {0}: version and engine in the facts", LANGS)
def _(env, lang):
    page = env.settings(lang=lang, section="advanced", state=settings_state(version="0.13.2"))
    facts = page.evaluate("[...document.querySelectorAll('.facts dt, .facts dd')].map((e) => e.textContent)")
    expect(facts == [text(lang, "settings.version"), "0.13.2", text(lang, "settings.engine"), "130.0.2849.80"], facts)
    expect(dom_text(page, "#about") == "YT Mini Player 0.13.2", dom_text(page, "#about"))


@test("e2e", "settings: an unknown engine version shows a dash")
def _(env):
    page = env.settings(section="advanced", state=settings_state(webview=None))
    expect(page.evaluate("document.querySelectorAll('.facts dd')[1].textContent") == "—", "no dash")


@each("e2e", "settings in {0}: the maximum overlay row says it is on", LANGS)
def _(env, lang):
    page = env.settings(lang=lang, section="advanced", state=settings_state(overlay=True))
    expect(dom_text(page, "[data-row='overlay'] .label") == text(lang, "settings.overlay"), "row")
