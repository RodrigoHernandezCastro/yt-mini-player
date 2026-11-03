"""CONTRACT: the texts the code asks for exist, and each page gets the ones it reads."""
import json
import re

import i18n_keys
from bundle import PREFIXES
from suite import LANGS, SRC, each, expect, test

CATALOGS = {lang: json.loads((SRC / "locales" / f"{lang}.json").read_text(encoding="utf-8")) for lang in LANGS}
RUST_FILES = sorted(p for p in list(SRC.glob("*.rs")) + list((SRC / "together").glob("*.rs"))
                    if re.search(r'\b(?:t|msg)!\(\s*"', p.read_text(encoding="utf-8")))
PAGES = {"home.html": "home", "settings.html": "settings", "together.html": "together", "lock.html": "lock",
         "mini.html": "mini", "stream.html": "stream", "music.html": "music", "ui.js": "ui"}


def page_keys(file):
    src = (SRC / file).read_text(encoding="utf-8")
    keys = set(re.findall(r"\bt\('([a-z0-9_.]+)'", src)) | set(re.findall(r'data-i18n(?:-[a-z-]+)?="([a-z0-9_.]+)"', src))
    if file == "ui.js":
        keys |= set(re.findall(r"label: '([a-z0-9_.]+)'", src))
    return {k for k in keys if not k.endswith(".")}


@test("contract", "every key the code uses exists, and every key in the catalogs is used (i18n_keys.py)")
def _(env):
    known = i18n_keys.catalog_keys()
    used = i18n_keys.used_keys(known)
    not_keys = {k for k in used if re.search(r"\.(com|org|net|rs|js|html|json|exe|dll|enc|tmp|log)$", k)}
    missing = sorted(k for k in used if k not in known and k not in not_keys)
    unused = sorted(known - set(used))
    expect(not missing, f"missing: {missing[:8]}")
    expect(not unused, f"never used: {unused[:8]}")


@each("contract", "{0}: every t!/msg! key exists in all five languages", [p.relative_to(SRC).as_posix() for p in RUST_FILES])
def _(env, file):
    keys = set(re.findall(r'\b(?:t|msg)!\(\s*"([^"]+)"', (SRC / file).read_text(encoding="utf-8")))
    for lang in LANGS:
        missing = sorted(k for k in keys if k not in CATALOGS[lang])
        expect(not missing, f"{lang}: {missing[:6]}")


@each("contract", "{0}: every key it reads exists in all five languages", list(PAGES))
def _(env, file):
    keys = page_keys(file)
    expect(keys, "found no keys (the pattern no longer matches the page)")
    for lang in LANGS:
        missing = sorted(k for k in keys if k not in CATALOGS[lang])
        expect(not missing, f"{lang}: {missing[:6]}")


@each("contract", "{0}: only reads strings the app gives it", [f for f in PAGES if f not in ("home.html", "settings.html", "ui.js")])
def _(env, file):
    prefixes = PREFIXES[PAGES[file]]
    outside = sorted(k for k in page_keys(file) if not any(k.startswith(p) for p in prefixes))
    expect(not outside, f"keys the page never gets: {outside}")


@each("contract", "the {0} page's strings are a set the app really embeds", list(PREFIXES))
def _(env, kind):
    home_rs = (SRC / "home.rs").read_text(encoding="utf-8")
    expect(f"const {kind.upper()}_STRINGS: &[&str]" in home_rs, f"no {kind.upper()}_STRINGS in home.rs")
    for prefix in PREFIXES[kind]:
        expect(prefix.endswith(".") and any(k.startswith(prefix) for k in CATALOGS["en"]), f"{prefix} matches no text")
