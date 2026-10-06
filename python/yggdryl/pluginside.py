"""The role of a FIX plugin: the side of the session a dialect's plugin stands
on, as an enum stored as a ``uint8``.

``PluginSide`` is the core's enum member for member - its stored name,
``UKNW``, ``BUYS``, ``SELL``, and the code a ``pluginside`` column stores -
built once at import from the native table, so nothing here lists a member.
``UKNW`` is ``0``, a Buy-Side plugin ``BUYS`` (``1``) and a Sell-Side one
``SELL`` (``2``). It is a session fact read off a dialect's source entry - a
``CBlock``'s root ``type`` names the plugin's class, which
:meth:`PluginSide.from_plugin_type` reads - and stamped on every message read
under that source as ``msgpluginside``; never read off a FIX tag, and a
separate enum from :class:`~yggdryl.Side` though ``BUYS`` and ``SELL`` are
spelled alike.
"""

from __future__ import annotations

import enum
from typing import TYPE_CHECKING, Literal, TypeAlias

from ._common import MetadataInput, new_field, simple_dtype
from ._native import (
    Field,
    pluginside_from_plugin_type,
    pluginside_from_spelling,
    pluginside_members,
)
from ._typing import TypedField

#: The native table: name, code and description.
_MEMBERS = pluginside_members()
_DESCRIPTIONS = {code: description for _, code, description in _MEMBERS}


class _Role(enum.IntEnum):
    """What every member of ``PluginSide`` answers, read off the native table."""

    @property
    def description(self) -> str:
        """What a plugin of this role does, in a sentence."""

        return _DESCRIPTIONS[self.value]

    @classmethod
    def from_spelling(cls, spelling: str) -> PluginSide | None:
        """The member a stored name in any case - ``SELL``, ``sell`` - or the
        role's own name folded - ``SellSide``, ``sell-side``, ``sell_side`` -
        names, or ``None`` where none does."""

        code = pluginside_from_spelling(spelling)
        return None if code is None else PluginSide(code)

    @classmethod
    def from_plugin_type(cls, plugin_type: str) -> PluginSide:
        """The role one plugin class name states - a ``CBlock``'s root
        ``type`` attribute: its last ``.``-separated segment, folded, holding
        ``buyside`` is ``BUYS`` and one holding ``sellside`` is ``SELL`` - and
        ``UKNW`` for a class naming neither. Never raises."""

        return cls(pluginside_from_plugin_type(plugin_type))

    def __str__(self) -> str:
        return self.name

    def __format__(self, format_spec: str) -> str:
        # An `IntEnum` renders as its integer, and the two runtimes disagree
        # about whether `__str__` or `int.__format__` decides that. Both render
        # the stored name; `int(member)` asks for the code.
        return format(self.name, format_spec)


PluginSide = _Role(  # type: ignore[misc]
    "PluginSide",
    [(name, code) for name, code, _ in _MEMBERS],
    module=__name__,
    qualname="PluginSide",
)

if TYPE_CHECKING:
    PluginSideField: TypeAlias = TypedField[Literal["pluginside"], PluginSide]
else:
    PluginSideField = Field

_PLUGINSIDE = simple_dtype("pluginside")


def pluginside(
    name: str,
    *,
    nullable: bool = True,
    metadata: MetadataInput = None,
) -> PluginSideField:
    """The role of a FIX plugin: a ``PluginSide``, stored as the code of its
    member."""

    return new_field(PluginSideField, name, _PLUGINSIDE, nullable, metadata)


__all__ = [
    "PluginSide",
    "PluginSideField",
    "pluginside",
]
