"""The suite's process-wide seals: `YGGDRYL_ISIN_REGISTRY_URI` names a
folder of this session's own before any test module imports, so
`IsinRegistry.from_env` and `FixCodec.from_env` never read or lay out the
real `~/.config/yggdryl/isin/`.
"""

from __future__ import annotations

import os
import tempfile

_ISIN_STORE = tempfile.mkdtemp(prefix="yggdryl-isin-registry-")
os.environ["YGGDRYL_ISIN_REGISTRY_URI"] = _ISIN_STORE + os.sep
