"""Typing for `yggdryl.marketdatakind`: the members are the native table's,
listed for static checkers; `python/tests/test_marketdatakind.py` holds the
list to it."""

from __future__ import annotations

import enum
from typing import Literal, TypeAlias

from ._common import MetadataInput
from ._typing import TypedField

class MarketDataKind(enum.IntEnum):
    UNKN = 0
    ACCT = 1
    ALLO = 2
    BOOK = 3
    CERT = 4
    COLL = 5
    COMM = 6
    CONF = 7
    EXEC = 8
    MKST = 9
    ORDR = 10
    PAYM = 11
    POSN = 12
    PRTY = 13
    QUOT = 14
    REGI = 15
    RISK = 16
    SECU = 17
    SESS = 18
    SETL = 19
    STRM = 20
    TRAD = 21
    ORDB = 22
    QUOB = 23
    EXEB = 24
    TRDB = 25
    @property
    def description(self) -> str: ...
    @classmethod
    def from_spelling(cls, spelling: str) -> MarketDataKind | None: ...

MarketDataKindField: TypeAlias = TypedField[Literal["marketdatakind"], MarketDataKind]

def marketdatakind(
    name: str, *, nullable: bool = True, metadata: MetadataInput = None
) -> MarketDataKindField: ...

__all__ = ["MarketDataKind", "MarketDataKindField", "marketdatakind"]
