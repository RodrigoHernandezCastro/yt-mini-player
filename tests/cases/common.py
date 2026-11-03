"""Helpers shared by the modules in tests/cases (on top of suite.py's)."""
import json

from bundle import HOST_KEY, LANGUAGES, strings
from home_e2e import SERVICES
from suite import ax_nodes, expect, text

SERVICE_NAMES = {sid: name for sid, name, *_ in SERVICES}
LANG_NAMES = {l["code"]: l["name"] for l in LANGUAGES}
MENU_ROLES = ("menuitem", "menuitemcheckbox", "menuitemradio")


def service_name(lang, service):
    return text(lang, "service.music_name") if service == "music" else SERVICE_NAMES[service]


def dom_text(page, selector):
    """textContent (not innerText: no CSS text-transform, hidden parts included)."""
    return page.evaluate(f"document.querySelector({json.dumps(selector)})?.textContent ?? null")


def attr(page, selector, name):
    return page.evaluate(f"document.querySelector({json.dumps(selector)})?.getAttribute({json.dumps(name)}) ?? null")


def prop_hidden(page, element_id):
    """The element's own `hidden` (whatever its parents)."""
    return page.evaluate(f"document.getElementById({json.dumps(element_id)}).hidden")


def ax_role_click(page, roles, predicate):
    """Clicks the accessible node with one of `roles` whose name matches;
    among several, the one with the shortest name ("Full screen" is not
    "Full screen in the window")."""
    found = []
    for node in ax_nodes(page):
        role = (node.get("role") or {}).get("value")
        name = (node.get("name") or {}).get("value") or ""
        if role in roles and predicate(name) and node.get("backendDOMNodeId"):
            found.append((len(name), node["backendDOMNodeId"]))
    if not found:
        return False
    backend = min(found)[1]
    page.cdp.send("DOM.scrollIntoViewIfNeeded", {"backendNodeId": backend})
    quad = page.cdp.send("DOM.getBoxModel", {"backendNodeId": backend})["model"]["border"]
    page.mouse.click((quad[0] + quad[2]) / 2, (quad[1] + quad[5]) / 2)
    return True


def show_bar(page):
    width, height = page.viewport_size["width"], page.viewport_size["height"]
    page.mouse.move(width - 10, height / 2)
    page.wait_for_timeout(250)


def open_menu(page, lang):
    """Opens the side bar's "More" menu through its button (found by name)."""
    show_bar(page)
    expect(ax_role_click(page, ("button",), lambda n: n == text(lang, "ui.more_options")), "no More button")
    page.wait_for_timeout(150)


def locale_payload(lang, page_kind=None):
    """What App::push_locale sends (as bundle.push_locale builds it)."""
    return {"lang": lang, "languages": LANGUAGES, "ui": strings(lang, "ui"),
            "page": strings(lang, page_kind) if page_kind else None}


def host_call(function, *args):
    """JavaScript that calls one of the app's functions with its key."""
    return f"window.{function}({', '.join(json.dumps(a) for a in (HOST_KEY,) + args)})"


def heap_used(cdp):
    cdp.send("HeapProfiler.collectGarbage")
    return cdp.send("Runtime.getHeapUsage")["usedSize"]


def bar_label(lang, label_key, keys):
    label = text(lang, label_key)
    if not keys:
        return label
    return text(lang, "ui.with_keys", label=label, keys=keys.replace("{home}", text(lang, "key.home")))
