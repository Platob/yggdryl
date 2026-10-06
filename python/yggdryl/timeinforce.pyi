"""Typing for `yggdryl.timeinforce`: the members are the native table's,
listed for static checkers; `python/tests/test_timeinforce.py` holds the list
to it."""

from __future__ import annotations

import enum
from typing import Literal, TypeAlias

from ._common import MetadataInput
from ._typing import TypedField

class TimeInForce(enum.IntEnum):
    UKNW = 0
    DAY = 1
    GTC = 2
    OPG = 3
    IOC = 4
    FOK = 5
    GTX = 6
    GTD = 7
    ATC = 8
    GTHX = 9
    ATX = 10
    GFT = 11
    GFA = 12
    GFM = 13
    OTHER = 99
    @property
    def description(self) -> str: ...
    @property
    def fix_code(self) -> str | None: ...
    @classmethod
    def from_spelling(cls, spelling: str) -> TimeInForce | None: ...
    @classmethod
    def from_fix(cls, wire: str) -> TimeInForce: ...

TimeInForceField: TypeAlias = TypedField[Literal["timeinforce"], TimeInForce]

def timeinforce(
    name: str, *, nullable: bool = True, metadata: MetadataInput = None
) -> TimeInForceField: ...

__all__ = ["TimeInForce", "TimeInForceField", "timeinforce"]
