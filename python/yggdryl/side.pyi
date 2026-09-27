"""Typing for `yggdryl.side`: the members are the native table's, listed for
static checkers; `python/tests/test_side.py` holds the list to it."""

from __future__ import annotations

import enum
from typing import Literal, TypeAlias

from ._common import MetadataInput
from ._typing import TypedField

class Side(enum.IntEnum):
    UNKNOWN = 0
    BUY = 1
    SELL = 2
    BUYMINUS = 3
    SELLPLUS = 4
    SSHORT = 5
    SSHORTEX = 6
    UNDISC = 7
    CROSS = 8
    CROSSSH = 9
    CROSSSHX = 10
    ASDEF = 11
    OPPOSITE = 12
    SUBSCR = 13
    REDEEM = 14
    LEND = 15
    BORROW = 16
    SELLUND = 17
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
