"""What kind of market data an element is: FIX's MsgCat code set as an enum,
stored as a ``uint8``.

``MarketDataKind`` is the core's enum member for member - its stored name, the
four-letter MsgCat code, and the code a ``marketdatakind`` column stores -
built once at import from the native table, so nothing here lists a member.
A member is an ``int``: ``ORDR`` is ``10``, ``QUOT`` ``14``, ``EXEC`` ``8``,
``TRAD`` ``21``, ``BOOK`` ``3`` and the batches ``ORDB`` ``22`` to ``TRDB``
``25``, and a column of them reads back as the members they name.
"""

from __future__ import annotations

import enum
from typing import TYPE_CHECKING, Literal, TypeAlias

from ._common import MetadataInput, new_field, simple_dtype
from ._native import Field, marketdatakind_from_spelling, marketdatakind_members
from ._typing import TypedField

#: The native table: name, code and description.
_MEMBERS = marketdatakind_members()
_DESCRIPTIONS = {code: description for _, code, description in _MEMBERS}


class _Kinded(enum.IntEnum):
    """What every member of ``MarketDataKind`` answers, read off the native table."""

    @property
    def description(self) -> str:
        """What this kind of market data is, in a sentence."""

        return _DESCRIPTIONS[self.value]

    @classmethod
    def from_spelling(cls, spelling: str) -> MarketDataKind | None:
        """The kind a four-letter code in any case - ``ORDR``, ``ordr`` - or the
        member's own word folded - ``order``, ``Quotation`` - names, or ``None``
        where none does."""

        code = marketdatakind_from_spelling(spelling)
        return None if code is None else MarketDataKind(code)

    def __str__(self) -> str:
        return self.name

    def __format__(self, format_spec: str) -> str:
        # An `IntEnum` renders as its integer, and the two runtimes disagree
        # about whether `__str__` or `int.__format__` decides that. Both render
        # the stored name; `int(member)` asks for the code.
        return format(self.name, format_spec)


MarketDataKind = _Kinded(  # type: ignore[misc]
    "MarketDataKind",
    [(name, code) for name, code, _ in _MEMBERS],
    module=__name__,
    qualname="MarketDataKind",
)

if TYPE_CHECKING:
    MarketDataKindField: TypeAlias = TypedField[Literal["marketdatakind"], MarketDataKind]
else:
    MarketDataKindField = Field

_MARKETDATAKIND = simple_dtype("marketdatakind")


def marketdatakind(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> MarketDataKindField:
    """What kind of market data one thing is: a ``MarketDataKind``, stored as the
    code of its member."""

    return new_field(MarketDataKindField, name, _MARKETDATAKIND, nullable, metadata)


__all__ = [
    "MarketDataKind",
    "MarketDataKindField",
    "marketdatakind",
]
