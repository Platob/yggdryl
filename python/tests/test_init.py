"""Each digest family is a module of its own at the package root."""

from __future__ import annotations

import importlib
import sys

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


@pytest.mark.parametrize("retired", ["hashing"])
def test_the_grouping_package_is_gone(retired: str) -> None:
    assert retired not in yggdryl.__all__
    assert not hasattr(yggdryl, retired)
    with pytest.raises(ModuleNotFoundError):
        importlib.import_module(f"yggdryl.{retired}")
