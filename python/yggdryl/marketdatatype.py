"""What type of its kind a market element is - the order type, the quote type,
the trade type, the book entry type - as an enum stored as a ``uint16``.

``MarketDataType`` is the core's enum member for member - its stored name and
the code a ``marketdatatype`` column stores - built once at
import from the native table, so nothing here lists a member. A member is an
``int`` whose hundreds name the FIX code set it types: ``ORDLIMIT`` is ``102``
(``OrdType(40)``), ``QUOTRAD`` ``201`` (``QuoteType(537)``), ``TRDBLOCK``
``301`` (``TrdType(828)``), ``BOOKBID`` ``400`` (``MDEntryType(269)``), then
the trade report types (``5xx``, ``TradeReportType(856)``), the quote request
types (``6xx``, ``QuoteRequestType(303)``), the mass cancel types (``7xx``,
``MassCancelRequestType(530)``) and the market data request types (``8xx``,
``SubscriptionRequestType(263)``), each set with its ``*OTHER`` catch-all.
:meth:`MarketDataType.from_fix` reads a wire value and
:attr:`MarketDataType.fix_code` answers it back.
"""

from __future__ import annotations

import enum
from typing import TYPE_CHECKING, Literal, TypeAlias

from ._common import MetadataInput, new_field, simple_dtype
from ._native import (
    Field,
    marketdatatype_fix_code,
    marketdatatype_fix_tags,
    marketdatatype_fix_tags_of,
    marketdatatype_from_fix,
    marketdatatype_from_spelling,
    marketdatatype_members,
)
from ._typing import TypedField
from .marketdatakind import MarketDataKind

#: The native table: name, code and description.
_MEMBERS = marketdatatype_members()
_DESCRIPTIONS = {code: description for _, code, description in _MEMBERS}


class _Kinded(enum.IntEnum):
    """What every member of ``MarketDataType`` answers, read off the native table."""

    @property
    def description(self) -> str:
        """What this type is, in a sentence."""

        return _DESCRIPTIONS[self.value]

    @classmethod
    def from_spelling(cls, spelling: str) -> MarketDataType | None:
        """The member a stored name in any case - ``ORDLIMIT``, ``ordlimit`` - or
        the FIX specification's own name folded - ``Limit``, ``block trade`` -
        names, or ``None`` where none does. A wire value is no spelling: read
        it with :meth:`from_fix`."""

        code = marketdatatype_from_spelling(spelling)
        return None if code is None else MarketDataType(code)

    @classmethod
    def from_fix(cls, tag: int, wire: str) -> MarketDataType | None:
        """The member FIX field ``tag``'s wire value ``wire`` types an element
        as - ``from_fix(40, "2")`` is ``ORDLIMIT`` - the field's catch-all for a
        value no member names, or ``None`` for a field that types nothing."""

        code = marketdatatype_from_fix(tag, wire)
        return None if code is None else cls(code)

    @property
    def fix_code(self) -> tuple[int, str] | None:
        """The FIX field and wire value this member stands for - ``(40, "2")``
        for ``ORDLIMIT`` - or ``None`` for ``UKNW`` and a catch-all."""

        return marketdatatype_fix_code(self.value)

    @staticmethod
    def fix_tags(kind: MarketDataKind) -> tuple[int, ...]:
        """The FIX fields that type an element of ``kind``, its own first."""

        return tuple(marketdatatype_fix_tags(int(kind)))

    @staticmethod
    def fix_tags_of(msgtype: str, kind: MarketDataKind) -> tuple[int, ...]:
        """The FIX fields that type a message of type ``msgtype`` filed under
        ``kind``, first stated first: the message type's own rule where the
        core states one - a trade capture report ``AE`` reads
        ``TradeReportType(856)`` first - else :meth:`fix_tags` of its kind."""

        return tuple(marketdatatype_fix_tags_of(msgtype, int(kind)))

    def __str__(self) -> str:
        return self.name

    def __format__(self, format_spec: str) -> str:
        # An `IntEnum` renders as its integer, and the two runtimes disagree
        # about whether `__str__` or `int.__format__` decides that. Both render
        # the stored name; `int(member)` asks for the code.
        return format(self.name, format_spec)


MarketDataType = _Kinded(  # type: ignore[misc]
    "MarketDataType",
    [(name, code) for name, code, _ in _MEMBERS],
    module=__name__,
    qualname="MarketDataType",
)

if TYPE_CHECKING:
    MarketDataTypeField: TypeAlias = TypedField[Literal["marketdatatype"], MarketDataType]
else:
    MarketDataTypeField = Field

_MARKETDATAKIND = simple_dtype("marketdatatype")


def marketdatatype(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> MarketDataTypeField:
    """What kind of market data one thing is: a ``MarketDataType``, stored as the
    code of its member."""

    return new_field(MarketDataTypeField, name, _MARKETDATAKIND, nullable, metadata)


__all__ = [
    "MarketDataType",
    "MarketDataTypeField",
    "marketdatatype",
]
