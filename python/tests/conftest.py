"""The suite's process-wide seals.

`FixCodec.from_env` resolves the process's instrument registry in-process
(`IsinRegistry.from_env`), from `YGGDRYL_ISIN_REGISTRY_URI` else the home's
`~/.config/yggdryl/isin/`: the variable is pointed at a folder of this
session's own before any test module imports, so no test reads or lays out
the real one.
"""

from __future__ import annotations

import os
import tempfile

_ISIN_STORE = tempfile.mkdtemp(prefix="yggdryl-isin-registry-")
os.environ["YGGDRYL_ISIN_REGISTRY_URI"] = _ISIN_STORE + os.sep
