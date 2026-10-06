"""FIX's side of a trade: an enum stored as the ``uint8`` code of its member.

``Side`` is the core's enum member for member - its four-letter code as its
name, ``BUYS``, ``SSHT``, and the integer code a ``side`` column stores - built
once at import from the native table, so nothing here lists a member or decides
which side takes the bid. ``UKNW`` is ``0`` and the seventeen sides FIX's
``Side(54)`` names follow in the order of their wire characters, ``1``..``9``
then ``A``..``H``.
"""

from __future__ import annotations

import enum
from typing import TYPE_CHECKING, Literal, TypeAlias

from ._common import MetadataInput, new_field, simple_dtype
from ._native import Field, side_from_spelling, side_members
from ._typing import TypedField

#: The native table: name, code, description, wire character and the bid and
#: ask facts.
_MEMBERS = side_members()
_FACTS = {code: (description, fix, facts) for _, code, description, fix, facts in _MEMBERS}


class _Sided(enum.IntEnum):
    """What every member of ``Side`` answers, read off the native table."""

    @property
    def description(self) -> str:
        """What this side means, in a sentence."""

        return _FACTS[self.value][0]

    @property
    def fix_code(self) -> str | None:
        """The ``Side(54)`` wire character - ``1``..``9`` then ``A``..``H`` -
        or ``None`` for ``UKNW``, which no message carries."""

        return _FACTS[self.value][1]

    def is_bid(self) -> bool:
        """Whether this side takes the bid of a quote or a book: a party willing
        to pay."""

        return _FACTS[self.value][2][0]

    def is_ask(self) -> bool:
        """Whether this side takes the ask of a quote or a book: a party willing
        to be paid."""

        return _FACTS[self.value][2][1]

    @classmethod
    def from_spelling(cls, spelling: str) -> Side | None:
        """The side a four-letter code, a FIX wire code or the specification's
        name names, or ``None`` where none does."""

        code = side_from_spelling(spelling)
        return None if code is None else Side(code)

    def __str__(self) -> str:
        return self.name

    def __format__(self, format_spec: str) -> str:
        # An `IntEnum` renders as its integer, and the two runtimes disagree
        # about whether `__str__` or `int.__format__` decides that. Both render
        # the four-letter code; `int(member)` asks for the code.
        return format(self.name, format_spec)


Side = _Sided(  # type: ignore[misc]
    "Side",
    [(name, code) for name, code, *_ in _MEMBERS],
    module=__name__,
    qualname="Side",
)

if TYPE_CHECKING:
    SideField: TypeAlias = TypedField[Literal["side"], Side]
else:
    SideField = Field

_SIDE = simple_dtype("side")


def side(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> SideField:
    """FIX's ``Side(54)``: a ``Side``, stored as the code of its member."""

    return new_field(SideField, name, _SIDE, nullable, metadata)


__all__ = [
    "Side",
    "SideField",
    "side",
]
