"""FIX field definitions, over the fields and handles :mod:`yggdryl` already has.

A FIX field is an ordinary :class:`~yggdryl.Field` whose ``fix:`` metadata the
protocol view ``field.fix`` reads and writes as typed properties - ``branch``,
``id``, ``tag``, ``tags``, ``aliases``, ``description`` - so nothing here is a
second field class. A branch key and an identifier cross as ``str``, coerced
once at the boundary, so neither has a class of its own. :class:`FixRegistry` resolves
those fields by identifier, by tag, by branch-qualified name or by
branch-qualified dotted path and persists them as JSON shards through any
``IOBase`` location, and :class:`FixMsg` is one row typed against the registry it
was resolved against. Every registry holds this crate's own fields from
construction - ``FixRegistry()`` is those eleven fields, never nothing - and a
store neither writes them nor overrides them. Resolution, folding, merging,
sharding and validation are native; this module only names them.

:func:`fix_cfb_fields` reads one Ullink ``CBlock`` for the vocabulary it
declares, in declaration order and keyed, which is what
:meth:`FixRegistry.add_fields` folds into a dictionary that already exists -
adding what is absent, merging what is stored, and writing nothing at all when
it refuses. :meth:`FixRegistry.from_cfb_file` is the same file read whole, answering
a dictionary and the message roots its grammar bindings describe.

:class:`UlPlugin` is one plugin a bridge configuration document answers for -
the ObjectName the bridge holds it under beside the attributes it stated - read
out of the bytes a line carries, out of a document already parsed, or out of a
typed message, and crossing back to one through
:meth:`UlPlugin.into_fixmsg`. :func:`fix_ulbridge_fields` is the dictionary
those attributes type against, which
:meth:`FixRegistry.with_ulbridge_fields` registers.

:class:`FixCodec` turns a captured line into one of those messages. Every
message it builds opens with ``beginstring`` - the wire's own, else the version
the message was read at - and closes with the crate's ``timestamp``: the row's
own clock where the capture stated one, else the first clock the message
carries, else the epoch, so :meth:`FixMsg.market_timestamp` always answers.
:func:`parse_arrow_reader` turns a whole Arrow capture into batches of them --
the capture's own columns first, the dictionary's fixed columns after, one
input row per output row. A capture's ``timestamp`` column stamps its row, and
any other column named after a field - ``sessionId``, or a bridge's ``seqNum``
for ``MsgSeqNum`` - fills that field where the frame did not state it, without
becoming an entry.
:func:`fix_schema` is the one fixed row a whole capture lands in - columns
spelled by the dictionary's folded canonical names, ``msgtype`` and never
``35``, so a column is found with ``schema.index_of("msgtype")`` and nothing has
to be resolved per row; the tag stays on each column's ``fix:tag``.
:func:`fix_schema_carrying` puts a capture's own columns in front of them,
dropping a capture column whose folded name a FIX column already takes.
:func:`fix_crate_fields` lists what this crate itself adds beside the
specification, on its own ``yggdryl`` branch: ``msghash``, ``version``,
``symbolticker``, ``timestamp``, ``unixpartition``, ``parentclordid``,
``parentorderid``, and the four facts a bridge's own log states about a line -
``sessionid``, ``msgctxid``, ``senderpluginid``, ``targetpluginid``.

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
    ULBRIDGE_BRANCH,
    USER_TAG_MAX,
    USER_TAG_MIN,
    FixBranch,
    FixMsg,
    FixCodec,
    FixRegistry,
    UlPlugin,
    fix_cfb_fields,
    fix_classify_arrow_array as classify_arrow_array,
    fix_crate_fields,
    fix_parse_arrow_reader as parse_arrow_reader,
    fix_schema,
    fix_schema_carrying,
    fix_schema_tags,
    fix_ulbridge_fields,
    global_registry,
    install_global_registry,
)

__all__ = [
    "STANDARD_BRANCH",
    "ULBRIDGE_BRANCH",
    "USER_TAG_MAX",
    "USER_TAG_MIN",
    "FixBranch",
    "FixMsg",
    "FixCodec",
    "FixRegistry",
    "UlPlugin",
    "classify_arrow_array",
    "fix_cfb_fields",
    "fix_crate_fields",
    "fix_schema",
    "fix_schema_carrying",
    "fix_schema_tags",
    "fix_ulbridge_fields",
    "global_registry",
    "install_global_registry",
    "parse_arrow_reader",
]
