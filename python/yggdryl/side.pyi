"""Typing for `yggdryl.side`: the members are the native table's, listed for
static checkers; `python/tests/test_side.py` holds the list to it."""

from __future__ import annotations

import enum
from typing import Literal, TypeAlias

from ._common import MetadataInput
from ._typing import TypedField

class Side(enum.IntEnum):
    UKNW = 0
    BUYS = 1
    SELL = 2
    BUYM = 3
    SELP = 4
    SSHT = 5
    SSEX = 6
    UNDI = 7
    CROS = 8
    CRSH = 9
    CRSX = 10
    ASDF = 11
    OPPO = 12
    SUBS = 13
    REDM = 14
    LEND = 15
    BORR = 16
    SELU = 17
    @property
    def description(self) -> str: ...
    @property
    def fix_code(self) -> str | None: ...
    def is_bid(self) -> bool: ...
    def is_ask(self) -> bool: ...
    @classmethod
    def from_spelling(cls, spelling: str) -> Side | None: ...

SideField: TypeAlias = TypedField[Literal["side"], Side]

def side(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> SideField: ...

__all__ = ["Side", "SideField", "side"]
