"""More tests for suite.py, one module per kind. Importing the package
registers them in suite.REGISTRY; they run with the rest:

    python tests/suite.py                       # everything
    python tests/suite.py --category performance
    python tests/suite.py -k "catalog:"         # by name
    python tests/suite.py --list                # what there is

  catalog        every text of the five catalogs, one test per key
  home_more      start screen and welcome, in every language
  settings_more  every Settings control, section and state, in every language
  sidebar_more   the side bar's buttons, shortcuts (any keyboard layout), menu, notices
  together_more  the Watch together window: every screen, the sound notice, statuses
  security_more  hostile text in every field, forged calls, markup rules
  a11y_more      names, aria references, ids, roles, target sizes
  perf_more      drawing time, redraw cost, memory, element counts, file budgets
  layout_more    more window sizes in every language
  contract_more  the texts the code asks for exist and reach their page
"""
from . import (  # noqa: F401
    a11y_more, catalog, contract_more, home_more, layout_more, perf_more, security_more, settings_more, sidebar_more,
    together_more,
)
