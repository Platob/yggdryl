"""What state one thing is in: a lifecycle-sorted enum, stored as an ``int32``.

``State`` is the core's enum member for member - its stored name, and the code
a ``state`` column stores - built once at import from the native table, so
nothing here lists a member or decides a band. A member is an ``int``: the
codes sort from the first state to the terminal ones, the hundreds of a code
are its rank, and a column of them reads back as the members they name.
"""

from __future__ import annotations

import enum
from typing import TYPE_CHECKING, Literal, TypeAlias

from ._common import MetadataInput, new_field, simple_dtype
from ._native import (
    Field,
    state_from_fix_msgtype,
    state_from_fix_status,
    state_from_spelling,
    state_members,
)
from ._typing import TypedField

#: The native table: name, code, description, rank and the six band facts.
_MEMBERS = state_members()
_FACTS = {code: (description, rank, facts) for _, code, description, rank, facts in _MEMBERS}


class _Stated(enum.IntEnum):
    """What every member of ``State`` answers, read off the native table."""

    @property
    def description(self) -> str:
        """What this state means, in a sentence."""

        return _FACTS[self.value][0]

    @property
    def rank(self) -> int:
        """The rank this state stands at: its code's hundreds, ``0`` to ``99``."""

        return _FACTS[self.value][1]

    def is_pending(self) -> bool:
        """Whether this state was asked for and not yet acknowledged: rank ``10``."""

        return _FACTS[self.value][2][0]

    def is_live(self) -> bool:
        """Whether this state can still change: every rank below ``80``."""

        return _FACTS[self.value][2][1]

    def is_done(self) -> bool:
        """Whether this state ended having done what was asked: rank ``80``-``89``."""

        return _FACTS[self.value][2][2]

    def is_cancelled(self) -> bool:
        """Whether this state ended because someone stopped it: rank ``90``-``94``."""

        return _FACTS[self.value][2][3]

    def is_failed(self) -> bool:
        """Whether this state ended because it could not be done: rank ``95``-``99``."""

        return _FACTS[self.value][2][4]

    def is_execution(self) -> bool:
        """Whether this state itself reports an execution."""

        return _FACTS[self.value][2][5]

    @classmethod
    def from_spelling(cls, spelling: str) -> State | None:
        """The state a stored name, FIX wire code, FIX name, scheduler's word or
        bridge's short name names, or ``None`` where none does."""

        code = state_from_spelling(spelling)
        return None if code is None else State(code)

    @classmethod
    def from_fix_status(cls, tag: int, code: str) -> State | None:
        """The state one FIX status field's code names, or ``None``."""

        held = state_from_fix_status(tag, code)
        return None if held is None else State(held)

    @classmethod
    def from_fix_msgtype(cls, msgtype: str) -> State | None:
        """The state a FIX message type asks for, or ``None``."""

        held = state_from_fix_msgtype(msgtype)
        return None if held is None else State(held)

    def __str__(self) -> str:
        return self.name

    def __format__(self, format_spec: str) -> str:
        # An `IntEnum` renders as its integer, and the two runtimes disagree
        # about whether `__str__` or `int.__format__` decides that. Both render
        # the stored name; `int(member)` asks for the code.
        return format(self.name, format_spec)


State = _Stated(  # type: ignore[misc]
    "State",
    [(name, code) for name, code, *_ in _MEMBERS],
    module=__name__,
    qualname="State",
)

if TYPE_CHECKING:
    StateField: TypeAlias = TypedField[Literal["state"], State]
else:
    StateField = Field

_STATE = simple_dtype("state")


def state(name: str, *, nullable: bool = True, metadata: MetadataInput = None) -> StateField:
    """What state one thing is in: a ``State``, stored as the code of its member."""

    return new_field(StateField, name, _STATE, nullable, metadata)


__all__ = [
    "State",
    "StateField",
    "state",
]
