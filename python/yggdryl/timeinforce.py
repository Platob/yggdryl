"""How long an order stands: FIX's ``TimeInForce(59)`` code set as an enum,
stored as a ``uint8``.

``TimeInForce`` is the core's enum member for member - its stored name,
``DAY``, ``GTC``, ``IOC``, and the code a ``timeinforce`` column stores - built
once at import from the native table, so nothing here lists a member. ``UNKN``
is ``0``, the thirteen values FIX names follow in wire order from ``DAY``
(``1``) to ``GFM`` (``13``), and ``OTHER`` (``99``) is a venue's own value no
member names. :meth:`TimeInForce.from_fix` reads a wire value and
:attr:`TimeInForce.fix_code` answers it back.
"""

from __future__ import annotations

import enum
from typing import TYPE_CHECKING, Literal, TypeAlias

from ._common import MetadataInput, new_field, simple_dtype
from ._native import (
    Field,
    timeinforce_from_fix,
    timeinforce_from_spelling,
    timeinforce_members,
)
from ._typing import TypedField

#: The native table: name, code, description and wire value.
_MEMBERS = timeinforce_members()
_FACTS = {code: (description, fix) for _, code, description, fix in _MEMBERS}


class _Standing(enum.IntEnum):
    """What every member of ``TimeInForce`` answers, read off the native table."""

    @property
    def description(self) -> str:
        """How long an order with this time in force stands, in a sentence."""

        return _FACTS[self.value][0]

    @property
    def fix_code(self) -> str | None:
        """The ``TimeInForce(59)`` wire value this member stands for - ``"1"``
        for ``GTC`` - or ``None`` for ``UNKN`` and ``OTHER``."""

        return _FACTS[self.value][1]

    @classmethod
    def from_spelling(cls, spelling: str) -> TimeInForce | None:
        """The member a stored name in any case - ``GTC``, ``gtc`` - the FIX
        specification's own name folded - ``GoodTillCancel`` - or the
        ``TimeInForce(59)`` wire value - ``"1"`` - names, or ``None`` where
        none does."""

        code = timeinforce_from_spelling(spelling)
        return None if code is None else TimeInForce(code)

    @classmethod
    def from_fix(cls, wire: str) -> TimeInForce:
        """The member one ``TimeInForce(59)`` wire value stands for -
        ``from_fix("1")`` is ``GTC`` - and ``OTHER`` for a value no member
        names."""

        return cls(timeinforce_from_fix(wire))

    def __str__(self) -> str:
        return self.name

    def __format__(self, format_spec: str) -> str:
        # An `IntEnum` renders as its integer, and the two runtimes disagree
        # about whether `__str__` or `int.__format__` decides that. Both render
        # the stored name; `int(member)` asks for the code.
        return format(self.name, format_spec)


TimeInForce = _Standing(  # type: ignore[misc]
    "TimeInForce",
    [(name, code) for name, code, *_ in _MEMBERS],
    module=__name__,
    qualname="TimeInForce",
)

if TYPE_CHECKING:
    TimeInForceField: TypeAlias = TypedField[Literal["timeinforce"], TimeInForce]
else:
    TimeInForceField = Field

_TIMEINFORCE = simple_dtype("timeinforce")


def timeinforce(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> TimeInForceField:
    """How long an order stands: a ``TimeInForce``, stored as the code of its
    member."""

    return new_field(TimeInForceField, name, _TIMEINFORCE, nullable, metadata)


__all__ = [
    "TimeInForce",
    "TimeInForceField",
    "timeinforce",
]
