"""FIX field definitions, over the fields and handles :mod:`yggdryl` already has.

A FIX field is an ordinary :class:`~yggdryl.Field` whose ``fix:`` metadata the
protocol view ``field.fix`` reads and writes as typed properties - ``branch``,
``id``, ``tag``, ``tags``, ``aliases``, ``description`` - so nothing here is a
second field class. A branch and an identifier cross as ``str``, coerced once at
the boundary, so there is no class for either. :class:`FixRegistry` resolves
those fields by identifier, by tag, by branch-qualified name or by
branch-qualified dotted path and persists them as JSON shards through any
``IOBase`` location, and :class:`FixMsg` is one row typed against the registry it
was resolved against. Resolution, folding, merging, sharding and validation are
native; this module only names them.

``STANDARD_BRANCH`` is what an absent ``fix:branch`` means, and
``STANDARD_TAG_LIMIT`` is the first tag the FIX specification does not assign
itself: below it, no other branch may claim a tag.
"""

from __future__ import annotations

from ._native import (
    STANDARD_BRANCH,
    STANDARD_TAG_LIMIT,
    FixMsg,
    FixRegistry,
    global_registry,
    install_global_registry,
)

__all__ = [
    "STANDARD_BRANCH",
    "STANDARD_TAG_LIMIT",
    "FixMsg",
    "FixRegistry",
    "global_registry",
    "install_global_registry",
]
