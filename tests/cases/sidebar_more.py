"""E2E: the side bar on a site (its buttons, keys, menu and notices), in every language."""
import json
import re

from bundle import HOST_KEY
from cases.common import MENU_ROLES, LANG_NAMES, ax_role_click, bar_label, open_menu, show_bar
from suite import (
    LANGS, SIDEBAR_HOST, ax_names, clear, each, expect, home_sent, sent, shadow, test, text,
)

BAR = [("grip", "ui.move_window", None), ("home", "ui.main_menu", "Alt+{home}"), ("back", "ui.back", "Alt+←"),
       ("videoOnly", "ui.fill_window", "Ctrl+M"), ("fullscreen", "ui.fullscreen", "F11"),
       ("pinned", "ui.always_on_top", "Ctrl+T"), ("hide", "ui.hide_to_tray", "Ctrl+H"),
       ("settings", "ui.settings", "Ctrl+,"), ("more", "ui.more_options", None), ("quit", "ui.close", "Ctrl+Q")]
BAR_CLICKS = [("home", "home"), ("videoOnly", "video-only"), ("pinned", "pin"), ("hide", "hide"),
              ("settings", "settings"), ("quit", "quit")]
SHORTCUTS = [("Control+t", "pin"), ("Control+m", "video-only"), ("Control+h", "hide"), ("Control+Comma", "settings"),
             ("Control+l", "open-clipboard"), ("Control+Digit1", "size:s"), ("Control+Digit2", "size:m"),
             ("Control+Digit3", "size:l"), ("Control+Shift+Digit1", "corner:nw"), ("Control+Shift+Digit2", "corner:ne"),
             ("Control+Shift+Digit3", "corner:sw"), ("Control+Shift+Digit4", "corner:se"), ("Alt+Home", "home"),
             ("Control+q", "quit")]
NOT_SHORTCUTS = ["t", "m", "h", "l", "q", "Digit1", "Control+Alt+t", "Control+Shift+t", "Control+Digit4",
                 "Control+Shift+Digit5", "Control+Alt+q", "Shift+Home", "Control+Home"]


def bar_entry(key):
    return next(entry for entry in BAR if entry[0] == key)


# --- The bar's buttons ---------------------------------------------------------

@each("e2e", "side bar in {0}: the {1} button is named with its keys",
      [(lang, key) for lang in LANGS for key, *_ in BAR])
def _(env, lang, key):
    page = env.sidebar(lang=lang)
    _, label, keys = bar_entry(key)
    expected = bar_label(lang, label, keys)
    expect(expected in ax_names(page), f"no button named {expected!r}")


@each("e2e", "side bar in {0}: the {1} button sends '{2}'", [(lang, k, m) for lang in LANGS for k, m in BAR_CLICKS])
def _(env, lang, key, message):
    page = env.sidebar(lang=lang)
    show_bar(page)
    clear(page)
    _, label, keys = bar_entry(key)
    expect(ax_role_click(page, ("button",), lambda n: n == bar_label(lang, label, keys)), "not found")
    expect(home_sent(page) == [message], home_sent(page))


@test("e2e", "side bar: no Hide button without a tray")
def _(env):
    page = env.sidebar(host=dict(SIDEBAR_HOST, tray=False))
    expect(bar_label("en", "ui.hide_to_tray", "Ctrl+H") not in ax_names(page), "Hide shown without a tray")


@test("e2e", "side bar: the full-screen button sends the change")
def _(env):
    page = env.sidebar()
    show_bar(page)
    clear(page)
    expect(ax_role_click(page, ("button",), lambda n: n == bar_label("en", "ui.fullscreen", "F11")), "not found")
    page.wait_for_timeout(400)
    expect("fullscreen:1" in home_sent(page), home_sent(page))


@test("e2e", "side bar: in full screen the button offers to leave it")
def _(env):
    page = env.sidebar()
    page.keyboard.press("F11")
    page.wait_for_timeout(400)
    expect(bar_label("en", "ui.exit_fullscreen", "F11") in ax_names(page), "still says Full screen")


# --- Keyboard shortcuts --------------------------------------------------------

@each("e2e", "side bar in {0}: {1} sends '{2}'", [(lang, keys, m) for lang in LANGS for keys, m in SHORTCUTS])
def _(env, lang, keys, message):
    page = env.sidebar(lang=lang)
    clear(page)
    page.keyboard.press(keys)
    expect(home_sent(page) == [message], home_sent(page))


@each("e2e", "side bar: {0} is not a shortcut", NOT_SHORTCUTS)
def _(env, keys):
    page = env.sidebar()
    clear(page)
    page.keyboard.press(keys)
    expect(home_sent(page) == [], home_sent(page))


# Letters by what the key prints when it prints a Latin letter, by position
# otherwise: (layout, key typed, physical key, virtual key, message or None).
LAYOUTS = [
    ("Russian", "е", "KeyT", 0x54, "pin"), ("Russian", "р", "KeyH", 0x48, "hide"),
    ("Russian", "д", "KeyL", 0x4C, "open-clipboard"), ("Russian", "й", "KeyQ", 0x51, "quit"),
    ("Russian", "ь", "KeyM", 0x4D, "video-only"), ("Ukrainian", "е", "KeyT", 0x54, "pin"),
    ("Greek", "τ", "KeyT", 0x54, "pin"), ("Hebrew", "א", "KeyT", 0x54, "pin"), ("Arabic", "ف", "KeyT", 0x54, "pin"),
    ("Dvorak", "t", "KeyK", 0x54, "pin"), ("Dvorak", "k", "KeyT", 0x4B, None),
    ("AZERTY", "q", "KeyA", 0x51, "quit"), ("AZERTY", "a", "KeyQ", 0x41, None), ("AZERTY", "m", "Semicolon", 0x4D, "video-only"),
]


@each("e2e", "side bar: Ctrl + '{1}' on {0} ({2}) sends {4}", LAYOUTS)
def _(env, layout, key, code, vk, message):
    page = env.sidebar()
    clear(page)
    for kind in ("rawKeyDown", "keyUp"):
        page.cdp.send("Input.dispatchKeyEvent", {"type": kind, "modifiers": 2, "key": key, "code": code,
                                                 "windowsVirtualKeyCode": vk, "nativeVirtualKeyCode": vk})
    expect(home_sent(page) == ([message] if message else []), home_sent(page))


@each("e2e", "side bar in {0}: F1 opens the help and Escape closes it", LANGS)
def _(env, lang):
    page = env.sidebar(lang=lang)
    page.keyboard.press("F1")
    page.wait_for_timeout(50)
    expect(text(lang, "ui.shortcuts") in ax_names(page), "help not open")
    page.keyboard.press("Escape")
    page.wait_for_timeout(50)
    expect(text(lang, "ui.shortcuts") not in ax_names(page), "help still open")


@each("e2e", "side bar in {0}: the help lists the window's shortcuts by name", LANGS)
def _(env, lang):
    page = env.sidebar(lang=lang)
    page.evaluate(f"window.__ytmpHelp({json.dumps(HOST_KEY)})")
    page.wait_for_timeout(50)
    names = " ".join(ax_names(page))
    for key in ("ui.always_on_top", "ui.fill_window", "ui.settings", "ui.quit", "ui.this_help"):
        expect(text(lang, key) in names, f"{key} missing")
    expect(text(lang, "ui.help.window") in names and text(lang, "ui.help.mouse") in names, "headings")


@test("e2e", "side bar: the help lists unavailable global shortcuts with the reason")
def _(env):
    hotkeys = [{"keys": "Ctrl+Alt+G", "label": "Ghost mode", "available": False, "note": "another app uses it"}]
    page = env.sidebar(host=dict(SIDEBAR_HOST, hotkeys=hotkeys))
    page.evaluate(f"window.__ytmpHelp({json.dumps(HOST_KEY)})")
    page.wait_for_timeout(50)
    expected = text("en", "ui.help.unavailable", label="Ghost mode", reason="another app uses it")
    expect(expected in ax_names(page), f"{expected!r} missing")


# --- The More menu -------------------------------------------------------------

MENU = [("menuitemcheckbox", "ui.always_on_top", "pin"), ("menuitemcheckbox", "ui.fill_window", "video-only"),
        ("menuitemcheckbox", "ui.ghost", "ghost"), ("menuitem", "ui.main_menu", "home"),
        ("menuitem", "ui.open_copied_link", "open-clipboard"), ("menuitem", "ui.together", "together"),
        ("menuitem", "ui.settings_all", "settings"), ("menuitem", "ui.section.extensions", "settings:extensions"),
        ("menuitem", "ui.hide_to_tray", "hide"), ("menuitem", "ui.quit", "quit")]
CHIPS = [("ui.size.small", "Ctrl+1", "size:s"), ("ui.size.medium", "Ctrl+2", "size:m"), ("ui.size.large", "Ctrl+3", "size:l"),
         ("ui.corner.top_left", "Ctrl+Shift+1", "corner:nw"), ("ui.corner.top_right", "Ctrl+Shift+2", "corner:ne"),
         ("ui.corner.bottom_left", "Ctrl+Shift+3", "corner:sw"), ("ui.corner.bottom_right", "Ctrl+Shift+4", "corner:se")]


@each("e2e", "side bar menu in {0}: the {2} row sends '{3}'", [(lang,) + row for lang in LANGS for row in MENU])
def _(env, lang, role, label, message):
    page = env.sidebar(lang=lang)
    open_menu(page, lang)
    clear(page)
    expect(ax_role_click(page, (role,), lambda n: n.startswith(text(lang, label))), "no such row")
    expect(home_sent(page) == [message], home_sent(page))


@each("e2e", "side bar menu in {0}: the chip '{1}' sends '{3}'", [(lang,) + chip for lang in LANGS for chip in CHIPS])
def _(env, lang, label, keys, message):
    page = env.sidebar(lang=lang)
    open_menu(page, lang)
    clear(page)
    name = text(lang, "ui.with_keys", label=text(lang, label), keys=keys)
    expect(ax_role_click(page, ("button",), lambda n: n == name), f"no chip named {name!r}")
    expect(home_sent(page) == [message], home_sent(page))


@each("e2e", "side bar menu: opacity {0} %, {1} sends {2}",
      [(50, "ui.opacity_down", "opacity:40"), (50, "ui.opacity_up", "opacity:60"), (25, "ui.opacity_down", "opacity:20"),
       (95, "ui.opacity_up", "opacity:100"), (100, "ui.opacity_down", "opacity:90"), (20, "ui.opacity_up", "opacity:30")])
def _(env, opacity, label, message):
    page = env.sidebar(host=dict(SIDEBAR_HOST, opacity=opacity))
    open_menu(page, "en")
    clear(page)
    expect(ax_role_click(page, ("button",), lambda n: n == text("en", label)), "no chip")
    expect(home_sent(page) == [message], home_sent(page))


@each("e2e", "side bar menu: at {0} % the {1} chip is off", [(20, "ui.opacity_down"), (100, "ui.opacity_up")])
def _(env, opacity, label):
    page = env.sidebar(host=dict(SIDEBAR_HOST, opacity=opacity))
    open_menu(page, "en")
    clear(page)
    ax_role_click(page, ("button",), lambda n: n == text("en", label))
    expect(home_sent(page) == [], home_sent(page))


@each("e2e", "side bar menu in {0}: switching to {1} from the language rows",
      [(a, b) for a in LANGS for b in LANGS if a != b])
def _(env, current, target):
    page = env.sidebar(lang=current)
    open_menu(page, current)
    expect(ax_role_click(page, ("menuitem",), lambda n: n.startswith(text(current, "lang.label"))), "no language row")
    page.wait_for_timeout(100)
    clear(page)
    expect(ax_role_click(page, ("menuitemradio",), lambda n: n.startswith(LANG_NAMES[target])), f"no {target} row")
    expect(home_sent(page) == [f"lang:{target}"], home_sent(page))


@each("e2e", "side bar menu in {0}: the current language is checked and sends nothing", LANGS)
def _(env, lang):
    page = env.sidebar(lang=lang)
    open_menu(page, lang)
    ax_role_click(page, ("menuitem",), lambda n: n.startswith(text(lang, "lang.label")))
    page.wait_for_timeout(100)
    clear(page)
    expect(ax_role_click(page, ("menuitemradio",), lambda n: n.startswith(LANG_NAMES[lang])), "no row")
    expect(home_sent(page) == [], home_sent(page))


@each("e2e", "side bar menu in {0}: section headings in the language", LANGS)
def _(env, lang):
    page = env.sidebar(lang=lang)
    open_menu(page, lang)
    names = ax_names(page)
    for key in ("ui.section.window", "ui.section.navigate", "ui.section.app"):
        expect(text(lang, key) in names, f"{key} missing")


@each("e2e", "side bar menu: Always on top shows the state (pinned: {0})", [True, False])
def _(env, pinned):
    # (Ghost mode keeps the bar away, so its menu is not reachable then.)
    page = env.sidebar(host=dict(SIDEBAR_HOST, pinned=pinned))
    open_menu(page, "en")
    checked = {}
    for node in page.cdp.send("Accessibility.getFullAXTree")["nodes"]:
        if (node.get("role") or {}).get("value") != "menuitemcheckbox":
            continue
        name = (node.get("name") or {}).get("value") or ""
        checked[name] = next((p["value"]["value"] for p in node.get("properties", []) if p["name"] == "checked"), None)
    pin = next(v for k, v in checked.items() if k.startswith(text("en", "ui.always_on_top")))
    ghost = next(v for k, v in checked.items() if k.startswith(text("en", "ui.ghost")))
    expect(pin == ("true" if pinned else "false") and ghost == "false", checked)


@test("e2e", "side bar: in ghost mode the bar stays away")
def _(env):
    page = env.sidebar(host=dict(SIDEBAR_HOST, ghost=True))
    show_bar(page)
    expect(page.evaluate("document.elementFromPoint(innerWidth - 20, innerHeight / 2)?.tagName") != "YTMP-UI", "the bar showed")


@each("e2e", "side bar menu: Watch together says {0}", [("host", "ui.together_hosting"), ("guest", "ui.together_watching")])
def _(env, role, key):
    page = env.sidebar(host=dict(SIDEBAR_HOST, together=role))
    open_menu(page, "en")
    expect(any(n.startswith(text("en", "ui.together")) and text("en", key) in n for n in ax_names(page)), "no status")


@test("e2e", "side bar menu: the maximum overlay line only when it is on")
def _(env):
    page = env.sidebar(host=dict(SIDEBAR_HOST, overlay=True))
    open_menu(page, "en")
    expect(any(text("en", "ui.overlay_on") in n for n in ax_names(page)), "missing")


@test("e2e", "side bar menu: the extensions row says when none are installed")
def _(env):
    page = env.sidebar(host=dict(SIDEBAR_HOST, extensions=[]))
    open_menu(page, "en")
    expect(any(text("en", "ui.ext.none") in n for n in ax_names(page)), "missing")


@test("e2e", "side bar menu: the extensions row counts the ones on")
def _(env):
    exts = [dict(SIDEBAR_HOST["extensions"][0], id=c * 32, enabled=c != "c") for c in "abc"]
    page = env.sidebar(host=dict(SIDEBAR_HOST, extensions=exts))
    open_menu(page, "en")
    expected = text("en", "ui.ext.summary", on=2, count=3)
    expect(any(expected in n for n in ax_names(page)), f"{expected!r} missing")


@test("e2e", "side bar menu: Escape closes it")
def _(env):
    page = env.sidebar()
    open_menu(page, "en")
    page.keyboard.press("Escape")
    page.wait_for_timeout(50)
    expect(not any(n.startswith(text("en", "ui.settings_all")) for n in ax_names(page)), "still open")


@test("e2e", "side bar menu: a click on the page closes it")
def _(env):
    page = env.sidebar()
    open_menu(page, "en")
    page.mouse.click(60, 60)
    page.wait_for_timeout(50)
    expect(not any(n.startswith(text("en", "ui.settings_all")) for n in ax_names(page)), "still open")


@test("e2e", "side bar menu: the More button opens and closes it")
def _(env):
    page = env.sidebar()
    open_menu(page, "en")
    ax_role_click(page, ("button",), lambda n: n == text("en", "ui.more_options"))
    page.wait_for_timeout(50)
    expect(not any(n.startswith(text("en", "ui.settings_all")) for n in ax_names(page)), "still open")


@test("e2e", "side bar menu: opening it tells the app (and closing it too)")
def _(env):
    page = env.sidebar()
    clear(page)
    open_menu(page, "en")
    page.keyboard.press("Escape")
    page.wait_for_timeout(50)
    overlay = [m for m in sent(page) if m.startswith("menu-open:")]
    expect(overlay == ["menu-open:1", "menu-open:0"], overlay)


@test("e2e", "side bar menu: the footer shows the version")
def _(env):
    page = env.sidebar()
    open_menu(page, "en")
    expect(f"YT Mini Player {SIDEBAR_HOST['version']}" in ax_names(page), "no version")


@test("e2e", "side bar menu: no menu rows for roles it should not have")
def _(env):
    page = env.sidebar()
    open_menu(page, "en")
    roles = {(n.get("role") or {}).get("value") for n in page.cdp.send("Accessibility.getFullAXTree")["nodes"]}
    expect("menu" in roles and set(MENU_ROLES) & roles, roles)


# --- Notices -------------------------------------------------------------------

@each("e2e", "side bar: a notice in {0} is shown and read out", LANGS)
def _(env, lang):
    page = env.sidebar(lang=lang)
    message = text(lang, "ui.fill_nothing")
    page.evaluate(f"window.__ytmpToast({json.dumps(HOST_KEY)}, {json.dumps(message)})")
    page.wait_for_timeout(50)
    expect(message in " ".join(ax_names(page)), "not shown")
    toasts = shadow(page, ".toasts")
    expect('role="status"' in toasts["html"] and 'aria-live="polite"' in toasts["html"], toasts["html"][:120])


@each("e2e", "side bar: {0} notices at once leave at most three", [4, 6, 12])
def _(env, count):
    page = env.sidebar()
    for i in range(count):
        page.evaluate(f"window.__ytmpToast({json.dumps(HOST_KEY)}, 'notice-{i}')")
    html = shadow(page, ".toasts")["html"]
    shown = len(re.findall(r'class="toast(?: out)?"', html))
    expect(1 <= shown <= 3, shown)
    expect(f"notice-{count - 1}" in html and "notice-0<" not in html, "the newest is not the one kept")


@each("e2e", "side bar: a notice of {0} characters stays at least {1} s", [(20, 3), (120, 6)])
def _(env, length, seconds):
    page = env.sidebar()
    message = ("x" * (length - 4)) + "-end"
    page.evaluate(f"window.__ytmpToast({json.dumps(HOST_KEY)}, {json.dumps(message)})")
    page.wait_for_timeout(seconds * 1000 - 600)
    expect(message in shadow(page, ".toasts")["html"], "gone too early")
