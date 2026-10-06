"""Typing for `yggdryl.pluginside`: the members are the native table's,
listed for static checkers; `python/tests/test_pluginside.py` holds the list
to it."""

from __future__ import annotations

import enum
from typing import Literal, TypeAlias

from ._common import MetadataInput
from ._typing import TypedField

class PluginSide(enum.IntEnum):
    UKNW = 0
    BUYS = 1
    SELL = 2
    @property
    def description(self) -> str: ...
    @classmethod
    def from_spelling(cls, spelling: str) -> PluginSide | None: ...
    @classmethod
    def from_plugin_type(cls, plugin_type: str) -> PluginSide: ...

PluginSideField: TypeAlias = TypedField[Literal["pluginside"], PluginSide]

def pluginside(
    name: str, *, nullable: bool = True, metadata: MetadataInput = None
) -> PluginSideField: ...

__all__ = ["PluginSide", "PluginSideField", "pluginside"]
