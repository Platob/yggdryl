"""FIX field definitions, over the fields and handles :mod:`yggdryl` already has.

A FIX field is an ordinary :class:`~yggdryl.Field` whose ``fix:`` metadata the
protocol view ``field.fix`` reads and writes as typed properties - ``branch``,
``id``, ``tag``, ``tags``, ``aliases``, ``description`` - so nothing here is a
second field class. A branch key and an identifier cross as ``str``, coerced
once at the boundary, so neither has a class of its own. :class:`FixRegistry` resolves
those fields by identifier, by tag, by branch-qualified name or by
branch-qualified dotted path and persists them as JSON shards through any
``IOBase`` location, and :class:`FixMsg` is one row typed against the registry it
was resolved against. Resolution, folding, merging, sharding and validation are
native; this module only names them.

:func:`fix_cfb_fields` reads one Ullink ``CBlock`` for the vocabulary it
declares, in declaration order and keyed, which is what
:meth:`FixRegistry.add_fields` folds into a dictionary that already exists -
adding what is absent, merging what is stored, and writing nothing at all when
it refuses. :meth:`FixRegistry.from_cfb_file` is the same file read whole, answering
a dictionary and the message roots its grammar bindings describe.

:class:`FixCodec` turns a captured line into one of those messages,
:func:`parse_arrow_reader` turns a whole Arrow capture into batches of them --
the capture's own columns first, the dictionary's fixed columns after, one
input row per output row -- and
:func:`fix_schema` is the one fixed row a whole capture lands in - columns named
by tag, because a tag is the one name a field has in every version and every
dialect. :class:`FixProjection` resolves those columns once so a row is an
indexed read rather than a dictionary lookup per column, and
:func:`fix_crate_fields` is what this crate itself adds beside the
specification: the digest, the version read, the cross-venue symbol, the market
clock, the partition it falls in, and the two parent order identifiers.

A branch is a ``str`` wherever it is a *key*; :class:`FixBranch` is what a
*declaration* is, because a declaration also carries the dialect's default FIX
version and the other spellings it answers to.

``STANDARD_BRANCH`` is what an absent ``fix:branch`` means, and
``USER_TAG_MIN`` and ``USER_TAG_MAX`` bound the half-open range a
non-standard branch may claim.
"""

from __future__ import annotations

from ._native import (
    STANDARD_BRANCH,
    USER_TAG_MAX,
    USER_TAG_MIN,
    FixBranch,
    FixMsg,
    FixProjection,
    FixCodec,
    FixRegistry,
    fix_cfb_fields,
    fix_classify_arrow_array as classify_arrow_array,
    fix_crate_fields,
    fix_parse_arrow_reader as parse_arrow_reader,
    fix_schema,
    fix_schema_tags,
    global_registry,
    install_global_registry,
)

__all__ = [
    "STANDARD_BRANCH",
    "USER_TAG_MAX",
    "USER_TAG_MIN",
    "FixBranch",
    "FixMsg",
    "FixProjection",
    "FixCodec",
    "FixRegistry",
    "classify_arrow_array",
    "fix_cfb_fields",
    "fix_crate_fields",
    "fix_schema",
    "fix_schema_tags",
    "global_registry",
    "install_global_registry",
    "parse_arrow_reader",
]
