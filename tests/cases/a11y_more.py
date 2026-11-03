"""ACCESSIBILITY: names, references, ids, roles and keyboard, in every language."""
from cases.common import attr
from cases.together_more import base_views, push_view, together_page
from suite import LANGS, SECTIONS, UNNAMED_JS, each, expect, settings_state, text

STATE = settings_state(hardwareAcceleration=False, overlay=True, lock="pin", togetherActive=True)

REFS_JS = """() => {
  const broken = [];
  for (const el of document.querySelectorAll('[aria-labelledby], [aria-describedby], [aria-controls]')) {
    for (const attr of ['aria-labelledby', 'aria-describedby', 'aria-controls']) {
      for (const id of (el.getAttribute(attr) || '').split(/\\s+/).filter(Boolean)) {
        if (!document.getElementById(id)) broken.push(attr + '=' + id);
      }
    }
  }
  return broken;
}"""
DUPLICATE_IDS_JS = """() => {
  const seen = new Map();
  for (const el of document.querySelectorAll('[id]')) seen.set(el.id, (seen.get(el.id) || 0) + 1);
  return [...seen].filter(([, n]) => n > 1).map(([id]) => id);
}"""
ROLES_JS = """() => {
  const allowed = new Set(['button', 'switch', 'radio', 'radiogroup', 'tab', 'tablist', 'tabpanel', 'group', 'status',
    'menu', 'menuitem', 'menuitemradio', 'menuitemcheckbox', 'dialog', 'list', 'listitem', 'img', 'presentation',
    'none', 'alert', 'region', 'toolbar', 'note', 'progressbar', 'separator', 'heading', 'navigation', 'log']);
  return [...document.querySelectorAll('[role]')].map((el) => el.getAttribute('role')).filter((r) => !allowed.has(r));
}"""
EMPTY_HEADINGS_JS = """() => [...document.querySelectorAll('h1, h2, h3')]
  .filter((h) => !h.closest('[hidden]') && getComputedStyle(h).display !== 'none' && !h.textContent.trim())
  .map((h) => h.id || h.tagName)"""


def settings(env, lang, section):
    return env.settings(lang=lang, section=section, state=STATE)


@each("a11y", "settings › {1} in {0}: every control has a name", [(l, s) for l in LANGS for s in SECTIONS])
def _(env, lang, section):
    unnamed = settings(env, lang, section).evaluate(UNNAMED_JS)
    expect(not unnamed, unnamed[:3])


@each("a11y", "settings › {1} in {0}: every aria reference points at something", [(l, s) for l in LANGS for s in SECTIONS])
def _(env, lang, section):
    broken = settings(env, lang, section).evaluate(REFS_JS)
    expect(not broken, broken[:4])


@each("a11y", "settings › {1} in {0}: no two elements share an id", [(l, s) for l in LANGS for s in SECTIONS])
def _(env, lang, section):
    dupes = settings(env, lang, section).evaluate(DUPLICATE_IDS_JS)
    expect(not dupes, dupes)


@each("a11y", "settings › {0}: only known roles", SECTIONS)
def _(env, section):
    odd = settings(env, "en", section).evaluate(ROLES_JS)
    expect(not odd, odd)


@each("a11y", "settings › {1} in {0}: no empty headings", [(l, s) for l in LANGS for s in SECTIONS])
def _(env, lang, section):
    empty = settings(env, lang, section).evaluate(EMPTY_HEADINGS_JS)
    expect(not empty, empty)


@each("a11y", "settings › {0}: radio groups have a name and one Tab stop", ["general", "together", "privacy"])
def _(env, section):
    page = settings(env, "en", section)
    groups = page.evaluate("""() => [...document.querySelectorAll('[role="radiogroup"]')].map((g) => ({
        name: g.getAttribute('aria-label') || '',
        stops: [...g.querySelectorAll('[role="radio"]')].filter((r) => r.tabIndex === 0).length,
        checked: [...g.querySelectorAll('[role="radio"][aria-checked="true"]')].length }))""")
    expect(groups and all(g["name"] and g["stops"] == 1 and g["checked"] == 1 for g in groups), groups)


@each("a11y", "settings in {0}: the rail and its tabs are named", LANGS)
def _(env, lang):
    page = settings(env, lang, "general")
    expect(attr(page, "#tabs", "aria-label") == text(lang, "settings.sections"), attr(page, "#tabs", "aria-label"))
    expect(attr(page, "nav", "aria-labelledby") == "settings-title", "nav not named")
    names = page.evaluate("[...document.querySelectorAll('[role=\"tab\"]')].map((t) => t.textContent.trim())")
    expect(all(names), names)


@each("a11y", "settings in {0}: size and corner groups are named", LANGS)
def _(env, lang):
    page = settings(env, lang, "window")
    labels = page.evaluate("[...document.querySelectorAll('[role=\"group\"]')].map((g) => g.getAttribute('aria-label'))")
    expect(labels == [text(lang, "settings.size"), text(lang, "settings.corner")], labels)


@each("a11y", "settings in {0}: the extensions list is named", LANGS)
def _(env, lang):
    page = settings(env, lang, "extensions")
    expect(attr(page, ".list", "aria-label") == text(lang, "settings.section.extensions"), attr(page, ".list", "aria-label"))


@each("a11y", "settings in {0}: the shortcuts list is named", LANGS)
def _(env, lang):
    page = settings(env, lang, "advanced")
    expect(attr(page, ".list", "aria-label") == text(lang, "settings.shortcuts"), attr(page, ".list", "aria-label"))


@each("a11y", "settings in {0}: the restart notice is announced", LANGS)
def _(env, lang):
    page = settings(env, lang, "advanced")
    expect(attr(page, ".notice", "role") == "status", "not a status")


@each("a11y", "settings in {0}: 'Saved' is announced politely", LANGS)
def _(env, lang):
    page = settings(env, lang, "together")
    expect(attr(page, ".saved", "aria-live") == "polite", "no live region")


@each("a11y", "start screen in {0}: aria references point at something", LANGS)
def _(env, lang):
    page = env.home(lang=lang, sidebar=False)
    broken = page.evaluate(REFS_JS)
    expect(not broken, broken)


@each("a11y", "start screen in {0}: no two elements share an id", LANGS)
def _(env, lang):
    page = env.home(lang=lang, welcome=True, sidebar=False)
    dupes = page.evaluate(DUPLICATE_IDS_JS)
    expect(not dupes, dupes)


@each("a11y", "start screen in {0}: the language menu is a menu of radio items", LANGS)
def _(env, lang):
    page = env.home(lang=lang, sidebar=False)
    expect(attr(page, "#lang-menu", "role") == "menu" and attr(page, "#lang", "aria-haspopup") == "menu", "roles")
    expect(attr(page, "#lang-menu", "aria-label") == text(lang, "lang.label"), attr(page, "#lang-menu", "aria-label"))
    roles = page.evaluate("[...document.querySelectorAll('#lang-menu button')].map((b) => b.getAttribute('role'))")
    expect(roles == ["menuitemradio"] * len(LANGS), roles)


@each("a11y", "start screen in {0}: no empty headings", LANGS)
def _(env, lang):
    page = env.home(lang=lang, sidebar=False)
    empty = page.evaluate(EMPTY_HEADINGS_JS)
    expect(not empty, empty)


@each("a11y", "welcome in {0} step {1}: the dialog has its title and text", [(l, n) for l in LANGS for n in range(1, 5)])
def _(env, lang, step):
    page = env.home(lang=lang, welcome=True, sidebar=False)
    for _ in range(step - 1):
        page.keyboard.press("ArrowRight")
    broken = page.evaluate(REFS_JS)
    expect(not broken, broken)
    expect(page.evaluate("document.getElementById('welcome-title').textContent.trim().length > 0"), "no title")
    expect(page.evaluate("document.getElementById('welcome-body').textContent.trim().length > 0"), "no text")
    expect(page.evaluate("!!document.activeElement.closest('#welcome')"), "the focus is outside the dialog")


@each("a11y", "{0}: buttons are at least 24×24 px in {1}", [(k, l) for k in ("home", "welcome", "settings › window") for l in LANGS])
def _(env, kind, lang):
    if kind == "settings › window":
        page = env.settings(lang=lang, section="window")
    else:
        page = env.home(lang=lang, welcome=kind == "welcome", sidebar=False)
    small = page.evaluate("""() => [...document.querySelectorAll('button')].filter((b) => {
        if (b.closest('[hidden]') || getComputedStyle(b).display === 'none') return false;
        const r = b.getBoundingClientRect(); return r.width && (r.width < 24 || r.height < 24); })
        .map((b) => b.textContent.trim() || b.getAttribute('aria-label') || b.className)""")
    expect(not small, small[:4])


@each("a11y", "together › {0} in {1}: every control has a name", [(v, l) for v in ("idle", "host", "guest", "guest-share", "ended")
                                                                   for l in LANGS])
def _(env, view, lang):
    page = together_page(env, lang)
    push_view(page, base_views()[view])
    unnamed = page.evaluate(UNNAMED_JS)
    expect(not unnamed, unnamed[:3])


@each("a11y", "together › {0} in {1}: aria references point at something and ids are unique",
      [(v, l) for v in ("idle", "host", "guest", "ended") for l in LANGS])
def _(env, view, lang):
    page = together_page(env, lang)
    push_view(page, base_views()[view])
    expect(not page.evaluate(REFS_JS), page.evaluate(REFS_JS))
    expect(not page.evaluate(DUPLICATE_IDS_JS), page.evaluate(DUPLICATE_IDS_JS))
