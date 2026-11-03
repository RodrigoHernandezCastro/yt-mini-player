"""i18n: every text of the five catalogs, one test each (and the catalogs whole).

Each key is checked in every language: present with the same shape (text or
plural), no stray spaces or control characters, the same {placeholders} and
the same [key caps] as English, balanced brackets and quotes in that
language's style, the plural forms the language needs, and nothing left to do.
"""
import json
import re

from suite import LANGS, SRC, each, expect, test

CATALOGS = {lang: json.loads((SRC / "locales" / f"{lang}.json").read_text(encoding="utf-8")) for lang in LANGS}
KEYS = sorted(CATALOGS["en"])
PLURAL_FORMS = {"en": {"one", "other"}, "es": {"one", "other"}, "ru": {"one", "few", "many"},
                "pl": {"one", "few", "many"}, "ja": {"other"}}
PLACEHOLDER = re.compile(r"\{([A-Za-z0-9_]+)\}")
# Key caps: [Ctrl+1], [Alt], [F1]… (Japanese also brackets screen names like [設定]: those are not keys).
KEY_CAP = re.compile(r"\[([A-Za-z0-9+←→↑↓ ,.\-/]+)\]")
PAIRS = [("(", ")"), ("（", "）"), ("[", "]"), ("«", "»"), ("「", "」")]


def forms(value):
    return list(value.values()) if isinstance(value, dict) else [value]


def found(regex, value):
    return set().union(*[set(regex.findall(text)) for text in forms(value)])


@each("i18n", "catalog: {0} is complete and consistent in every language", KEYS)
def _(env, key):
    english = CATALOGS["en"][key]
    for lang in LANGS:
        value = CATALOGS[lang].get(key)
        expect(value is not None, f"{lang}: missing")
        expect(isinstance(value, dict) == isinstance(english, dict), f"{lang}: text in one language, plural in another")
        if isinstance(value, dict):
            expect(PLURAL_FORMS[lang] <= set(value), f"{lang}: needs the forms {sorted(PLURAL_FORMS[lang])}, has {sorted(value)}")
        for text in forms(value):
            expect(isinstance(text, str) and text.strip(), f"{lang}: empty")
            expect(text == text.strip(), f"{lang}: spaces around {text!r}")
            expect("  " not in text, f"{lang}: double space in {text!r}")
            expect(not any(ord(c) < 32 and c != "\n" for c in text), f"{lang}: control character")
            expect(not re.search(r"TODO|FIXME|XXX", text), f"{lang}: left to do")
            for open_, close in PAIRS:
                expect(text.count(open_) == text.count(close), f"{lang}: unbalanced {open_}{close} in {text!r}")
            if lang == "pl":
                expect(text.count("„") == text.count("”"), f"pl: unbalanced „” in {text!r}")
            else:
                expect(text.count("“") == text.count("”"), f"{lang}: unbalanced “” in {text!r}")
        expect(found(PLACEHOLDER, value) == found(PLACEHOLDER, english),
               f"{lang}: placeholders {sorted(found(PLACEHOLDER, value))} vs {sorted(found(PLACEHOLDER, english))}")
        expect(found(KEY_CAP, value) == found(KEY_CAP, english),
               f"{lang}: key caps {sorted(found(KEY_CAP, value))} vs {sorted(found(KEY_CAP, english))}")


@each("i18n", "catalog: {0} has exactly the English keys", LANGS)
def _(env, lang):
    missing = sorted(set(CATALOGS["en"]) - set(CATALOGS[lang]))
    extra = sorted(set(CATALOGS[lang]) - set(CATALOGS["en"]))
    expect(not missing and not extra, f"missing {missing[:5]}, extra {extra[:5]}")


@each("i18n", "catalog: {0} is valid UTF-8 JSON without a BOM", LANGS)
def _(env, lang):
    raw = (SRC / "locales" / f"{lang}.json").read_bytes()
    expect(not raw.startswith(b"\xef\xbb\xbf"), "BOM")
    json.loads(raw.decode("utf-8"))


@test("i18n", "catalog: every key is namespaced (area.name) in lowercase")
def _(env):
    bad = [k for k in KEYS if not re.fullmatch(r"[a-z]+(\.[a-z0-9_]+)+", k)]
    expect(not bad, bad[:10])
