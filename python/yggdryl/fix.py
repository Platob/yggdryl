"""FIX field definitions, over the fields and handles :mod:`yggdryl` already has.

A FIX field is an ordinary :class:`~yggdryl.Field` whose ``fix:`` metadata the
protocol view ``field.fix`` reads and writes as typed properties - ``tag``,
``tags``, ``aliases``, ``description`` - so nothing here is a second field
class. :class:`FixRegistry` resolves those fields by tag, by name or by dotted
path and persists them as JSON shards through any ``IOBase`` location, and
:class:`FixMsg` is one row typed against the registry it was resolved against.
Resolution, folding, merging, sharding and validation are native;
this module only names them.
"""

from __future__ import annotations

from ._native import (
    FixMsg,
    FixRegistry,
    global_registry,
    install_global_registry,
)

__all__ = [
    "FixMsg",
    "FixRegistry",
    "global_registry",
    "install_global_registry",
]
