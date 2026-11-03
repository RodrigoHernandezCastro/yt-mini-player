"""LAYOUT: more window sizes, every language."""
import json

from bundle import HOST_KEY
from cases.common import bar_label, open_menu, show_bar
from suite import (
    CLAMPED_JS, FIT_JS, LANGS, SECTIONS, ax_box, each, expect, settings_state, shadow, text,
)

SETTINGS_SIZES = [(560, 480), (1024, 700), (640, 420)]
HOME_SIZES = [(800, 450), (1280, 720), (560, 315)]
STATE = settings_state(hardwareAcceleration=False, overlay=True, lock="pin", togetherActive=True)


@each("layout", "settings › {1} in {0} at {2}×{3}: nothing cut or sideways",
      [(l, s, w, h) for l in LANGS for s in SECTIONS for (w, h) in SETTINGS_SIZES])
def _(env, lang, section, width, height):
    page = env.settings(lang=lang, width=width, height=height, section=section, state=STATE)
    issues = page.evaluate(FIT_JS, "main")
    expect(not issues, issues)
    rail = page.evaluate(FIT_JS, "nav") if width > 620 else []
    expect(not rail, f"rail: {rail}")


@each("layout", "settings › {1} in {0} at 760×560: many extensions and shortcuts still fit",
      [(l, s) for l in LANGS for s in ("extensions", "advanced")])
def _(env, lang, section):
    exts = [{"folder": f"e{i}", "name": f"An extension with a rather long name number {i}", "version": f"{i}.0.{i}",
             "id": f"{i:02d}" * 16, "enabled": i % 2 == 0, "hasPage": True, "error": None, "pending": False} for i in range(12)]
    keys = [{"keys": f"Ctrl+Alt+Shift+F{i}", "label": f"A global shortcut with a long label {i}", "available": i % 3 != 0,
             "note": "another program registered this combination first"} for i in range(12)]
    page = env.settings(lang=lang, section=section, state=settings_state(extensions=exts, hotkeys=keys))
    issues = page.evaluate(FIT_JS, "main")
    expect(not issues, issues)


@each("layout", "start screen {0} at {1}×{2}: nothing cut or sideways", [(l, w, h) for l in LANGS for (w, h) in HOME_SIZES])
def _(env, lang, width, height):
    page = env.home(lang=lang, width=width, height=height, last="youtube", sidebar=False)
    issues = page.evaluate(FIT_JS, "main")
    expect(not issues, issues)
    expect(page.evaluate("document.documentElement.scrollWidth <= innerWidth"), "scrolls sideways")


@each("layout", "start screen {0} at {1}×{2}: all nine keys and the address fit",
      [(l, w, h) for l in LANGS for (w, h) in HOME_SIZES[:2]])
def _(env, lang, width, height):
    page = env.home(lang=lang, width=width, height=height, sidebar=False)
    out = page.evaluate("""() => [...document.querySelectorAll('.key, #site-url, .open, #together, #settings')]
        .filter((el) => { const r = el.getBoundingClientRect(); return r.bottom > innerHeight + 1 || r.right > innerWidth + 1 || r.height < 28; })
        .map((el) => el.dataset.id || el.id || el.className)""")
    expect(not out, out)
    expect(not page.evaluate(CLAMPED_JS, ".key .name span"), "names cut")


@each("layout", "start screen {0}: the open language menu stays on screen", LANGS)
def _(env, lang):
    page = env.home(lang=lang, sidebar=False)
    page.click("#lang")
    box = page.locator("#lang-menu").bounding_box()
    expect(box and box["x"] >= 0 and box["y"] >= 0 and box["x"] + box["width"] <= 641 and box["y"] + box["height"] <= 361, box)


@each("layout", "welcome {0} at {1}×{2}: every step fits, buttons in view",
      [(l, w, h) for l in LANGS for (w, h) in [(800, 450), (1280, 720)]])
def _(env, lang, width, height):
    page = env.home(lang=lang, width=width, height=height, welcome=True, sidebar=False)
    for step in range(4):
        r = page.locator("#welcome-next").bounding_box()
        expect(r["y"] + r["height"] <= height and r["x"] + r["width"] <= width, f"step {step + 1}: Next out of view")
        issues = page.evaluate(FIT_JS, ".sheet")
        expect(not issues, f"step {step + 1}: {issues}")
        page.keyboard.press("ArrowRight")


@each("layout", "side bar menu in {0} at {1}×{2}: the Settings row is in view",
      [(l, w, h) for l in LANGS for (w, h) in [(640, 360), (960, 540)]])
def _(env, lang, width, height):
    page = env.sidebar(lang=lang, width=width, height=height)
    open_menu(page, lang)
    box = ax_box(page, lambda n: n.startswith(text(lang, "ui.settings_all")))
    expect(box and box["x"] >= 0 and box["x"] + box["width"] <= width, f"Settings row out of view ({box})")


@each("layout", "side bar in {0} at {1}×{2}: the bar's buttons are inside the window",
      [(l, w, h) for l in LANGS for (w, h) in [(480, 270), (640, 360), (960, 540)]])
def _(env, lang, width, height):
    page = env.sidebar(lang=lang, width=width, height=height)
    show_bar(page)
    # (Below 300 px of height the secondary buttons step aside: these stay.)
    for label, keys in (("ui.main_menu", "Alt+{home}"), ("ui.settings", "Ctrl+,"), ("ui.more_options", None),
                        ("ui.close", "Ctrl+Q")):
        name = bar_label(lang, label, keys)
        box = ax_box(page, lambda n, name=name: n == name)
        expect(box and 0 <= box["x"] and box["x"] + box["width"] <= width and 0 <= box["y"]
               and box["y"] + box["height"] <= height, f"{label}: {box}")


@each("layout", "side bar in {0}: the help fits at {1}×{2}", [(l, w, h) for l in LANGS for (w, h) in [(640, 360), (960, 540)]])
def _(env, lang, width, height):
    page = env.sidebar(lang=lang, width=width, height=height)
    page.evaluate(f"window.__ytmpHelp({json.dumps(HOST_KEY)})")
    page.wait_for_timeout(50)
    card = shadow(page, ".help .card")
    box = card and card["box"]
    expect(box and box["x"] >= 0 and box["x"] + box["width"] <= width + 1, f"the help sticks out sideways ({box})")


@each("layout", "side bar menu in {0}: the chips row fits at 480×270", LANGS)
def _(env, lang):
    page = env.sidebar(lang=lang, width=480, height=270)
    open_menu(page, lang)
    name = text(lang, "ui.with_keys", label=text(lang, "ui.corner.bottom_right"), keys="Ctrl+Shift+4")
    box = ax_box(page, lambda n: n == name)
    expect(box and box["x"] >= 0 and box["x"] + box["width"] <= 480, f"the last corner chip is out of view ({box})")
