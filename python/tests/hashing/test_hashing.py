"""Both digest families have one owner, and no top-level path is kept for them."""

from __future__ import annotations

import importlib

import pytest

import yggdryl
from yggdryl import hashing
from yggdryl.hashing import txhash, xxhash


def test_the_hashing_package_owns_both_families() -> None:
    assert set(hashing.__all__) == {"txhash", "xxhash"}
    assert hashing.xxhash is xxhash
    assert hashing.txhash is txhash
    assert "hashing" in yggdryl.__all__


@pytest.mark.parametrize("retired", ["xxhash", "txhash"])
def test_the_top_level_paths_are_gone(retired: str) -> None:
    assert retired not in yggdryl.__all__
    assert not hasattr(yggdryl, retired)
    with pytest.raises(ModuleNotFoundError):
        importlib.import_module(f"yggdryl.{retired}")
