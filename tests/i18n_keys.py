"""Checks that every text key the code uses exists in the catalogs, and that
the catalogs have no keys nobody uses.

    python tests/i18n_keys.py

Looks at the Rust sources (t!, msg!, key literals such as service captions
and shortcut labels) and at the pages and scripts (t(), tx(), data-i18n*).
Keys built at run time are listed in DYNAMIC with every value they can take.
"""
import json
import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / 'src'
LOCALES = SRC / 'locales'

# Keys put together in code: prefix → the endings it can get.
DYNAMIC = {
    'music.plan.': ['free', 'downloads', 'lifetime'],
    'music.license.terms.': ['public_domain', 'cc0', 'by', 'by_sa', 'by_nd'],
    'music.reason.': ['same_file', 'same_download', 'same_song', 'same_audio'],
    'music.license.status.': ['active', 'inactive', 'expired', 'disabled', 'invalid'],
    'music.repeat.': ['off', 'all', 'one'],
    'together.status.': ['waiting', 'host_elsewhere', 'other_site', 'opening', 'no_video', 'in_sync', 'syncing',
                         'host_ad', 'own_ad', 'live', 'cannot_seek'],
    'together.window.': ['watching', 'connecting', 'failed', 'paused', 'closed'],
    'settings.section.': ['general', 'window', 'together', 'privacy', 'extensions', 'advanced'],
    'settings.lede.': ['general', 'window', 'together', 'privacy', 'extensions', 'advanced'],
    'home.welcome.': [f'{step}.{part}' for step in ('pick', 'move', 'controls', 'together') for part in ('title', 'body')],
}


def catalog_keys():
    english = json.loads((LOCALES / 'en.json').read_text(encoding='utf-8'))
    return set(english)


def used_keys(known):
    prefixes = sorted({k.split('.')[0] + '.' for k in known})
    literal = re.compile(r'"((?:%s)[a-z0-9_.]*[a-z0-9_])"' % '|'.join(map(re.escape, prefixes)))
    js_call = re.compile(r"""\b(?:t|tx)\(\s*'([a-z0-9_.]+)'""")
    js_literal = re.compile(r"""'((?:%s)[a-z0-9_.]*[a-z0-9_])'""" % '|'.join(map(re.escape, prefixes)))
    attr = re.compile(r'data-i18n(?:-[a-z-]+)?="([a-z0-9_.]+)"')
    used = {}
    for path in sorted(SRC.rglob('*')):
        if path.suffix not in ('.rs', '.js', '.html'):
            continue
        text = path.read_text(encoding='utf-8')
        found = set()
        if path.suffix == '.rs':
            found |= set(literal.findall(text))
        else:
            found |= set(js_call.findall(text)) | set(attr.findall(text)) | set(js_literal.findall(text))
        for key in found:
            if key.endswith('.') or key in ('home.html', 'ui.js', 'music.html', 'lock.html', 'mini.html', 'together.html',
                                            'stream.html', 'cast.html'):
                continue
            used.setdefault(key, path.relative_to(ROOT).as_posix())
    for prefix, endings in DYNAMIC.items():
        for ending in endings:
            used.setdefault(prefix + ending, '(dynamic)')
    return used


def main():
    known = catalog_keys()
    used = used_keys(known)
    # Literals that only look like keys (file names, domains...).
    not_keys = {k for k in used if re.search(r'\.(com|org|net|rs|js|html|json|exe|dll|enc|tmp|log)$', k)}
    missing = sorted(k for k in used if k not in known and k not in not_keys)
    unused = sorted(known - set(used))
    for key in missing:
        print(f'missing in the catalogs: {key} (used in {used[key]})')
    for key in unused:
        print(f'never used: {key}')
    print(f'{len(known)} keys in the catalogs, {len(used) - len(not_keys)} used')
    return 1 if missing or unused else 0


if __name__ == '__main__':
    sys.exit(main())
