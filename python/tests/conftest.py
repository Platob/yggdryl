"""The suite's process-wide seals.

`FixCodec.from_env` resolves the process's instruments in-process
(`Instruments.from_env`), from `YGGDRYL_INSTRUMENTS_URI` else the home's
`~/.config/yggdryl/instruments/`: the variable is pointed at a folder of this
session's own before any test module imports, so no test reads or lays out
the real one.
"""

from __future__ import annotations

import os
import tempfile

_INSTRUMENTS_STORE = tempfile.mkdtemp(prefix="yggdryl-instruments-")
os.environ["YGGDRYL_INSTRUMENTS_URI"] = _INSTRUMENTS_STORE + os.sep
