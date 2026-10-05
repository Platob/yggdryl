"""The package root: what importing ``yggdryl`` gives a caller.

Each digest family is a module of its own at the root, and on a free-threaded
interpreter the import leaves the GIL disabled.
"""

from __future__ import annotations

import sys
import sysconfig

import pytest

import yggdryl
from yggdryl import txhash, xxhash


def test_each_family_is_a_root_module() -> None:
    # The crate gives `xxhash/` and `txhash/` a root folder each and keeps
    # `hashing/` for the private adapters they share; the package follows it,
    # so neither family is reached through a namespace that owns nothing.
    assert sys.modules["yggdryl.xxhash"] is xxhash
    assert sys.modules["yggdryl.txhash"] is txhash
    assert yggdryl.xxhash is xxhash
    assert yggdryl.txhash is txhash
    assert {"txhash", "xxhash"} <= set(yggdryl.__all__)


def test_importing_the_package_leaves_a_free_threaded_interpreter_without_a_gil() -> None:
    # PyO3 declares `#[pymodule] fn _native` free-threading safe by default
    # (`gil_used = false`); an extension that did not would make CPython turn
    # the GIL back on at import, with a warning and nothing failing.
    if sysconfig.get_config_var("Py_GIL_DISABLED") != 1:
        pytest.skip("this interpreter has a GIL")
    assert yggdryl.__name__ == "yggdryl"
    assert not sys._is_gil_enabled()
