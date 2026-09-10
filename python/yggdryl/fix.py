"""FIX field definitions, over the fields and handles :mod:`yggdryl` already has.

A FIX field is an ordinary :class:`~yggdryl.Field` whose ``fix:`` metadata the
protocol view ``field.fix`` reads and writes as typed properties - ``branch``,
``id``, ``tag``, ``tags``, ``aliases``, ``description`` - so nothing here is a
second field class. A branch key and an identifier cross as ``str``, coerced
once at the boundary, so neither has a class of its own. :class:`FixRegistry` resolves
those fields by identifier, by tag, by branch-qualified name or by
branch-qualified dotted path and persists them as JSON shards through any
``IOBase`` location, and :class:`FixMsg` is one row typed against the registry it
was resolved against, written through :meth:`FixMsg.set` and
:meth:`FixMsg.remove` and read back from a fixed row by :meth:`FixMsg.from_row`.
Every registry holds this crate's own fields from
construction - ``FixRegistry()`` is those twenty fields, never nothing - and a
store neither writes them nor overrides them. Resolution, folding, merging,
sharding and validation are native; this module only names them.

:func:`fix_cfb_fields` reads one Ullink ``CBlock`` for the vocabulary it
declares, in declaration order and keyed, which is what
:meth:`FixRegistry.add_fields` folds into a dictionary that already exists -
adding what is absent, merging what is stored, and writing nothing at all when
it refuses. :meth:`FixRegistry.from_cfb_file` is the same file read whole, answering
a dictionary and the message roots its grammar bindings describe.

:class:`UlPlugin` carries one bridge configuration's ObjectName, attributes,
and response envelope. :class:`UlPlugins` lazily yields configurations from a
single or bulk document, and :meth:`UlPlugin.into_fixmsg` converts one to a
flat typed message. :func:`fix_ulbridge_fields` is the dictionary
those attributes type against, which
:meth:`FixRegistry.with_ulbridge_fields` registers.

:class:`FixCodec` parses lines into messages and enriches messages, each
as an iterator and each with an Arrow-batch twin. :meth:`FixCodec.parse_line`
turns one captured line into a lazy :class:`FixMessages` stream and
:meth:`FixCodec.parse_lines` a whole iterable of lines, one line at a time;
:meth:`FixCodec.parse_text_record` and :meth:`FixCodec.parse_text_records` read
the records a text reader answers, the payload column beside the row's own
``pluginid``, ``beginstring``, ``sep`` and ``timestamp`` parameters -
``direction`` is a parameter of theirs too, and the one only
:meth:`FixCodec.parse_text_arrow_reader` has a column to put in. Every message
it builds opens with ``beginstring`` - the wire's own, else the version the
message was read at - and closes with the
crate's ``timestamp``: the row's own clock where the capture stated one, else
the first clock the message carries, else the epoch, so
:meth:`FixMsg.market_timestamp` always answers.
:meth:`FixCodec.parse_text_arrow_reader` turns a whole Arrow capture into
batches of FIX rows - the capture's own columns first, the dictionary's fixed
columns after, one source row's columns repeated for each message a bulk
document expands to - closed on the raw bytes of the payload column against
the codec's ``batch_byte_size``. A capture's ``timestamp`` column stamps its
row; its ``pluginid`` column names the plugin that logged the line, and where
that text is the name or an alias of a branch the dictionary declares it also
names the dialect the row is read under, outranking the codec's own pin; and
any other column named after a field - ``prevpluginid``, ``senderSessionId``,
or a bridge's ``seqNum`` for ``MsgSeqNum`` - fills that field where the frame
did not state it, without becoming an entry. :meth:`FixCodec.enrich_message` and
:meth:`FixCodec.enrich_messages` fill what a message implied but did not carry,
and :meth:`FixCodec.enrich_messages_arrow_reader` does the same over batches
of rows without parsing them again, through the two converters every stage
composes over batches: :meth:`FixCodec.messages` reads a batch back as the
messages that made it and :meth:`FixCodec.arrow_reader` writes messages as
batches under a schema. :meth:`FixCodec.write_arrow_reader` is the encode
direction, re-emitting every row's wire. A pin - ``branch``, ``version``,
``separator``, ``payload_column``, ``null_values``, ``direction``,
``batch_byte_size`` - is on the codec; a stage is a call.
:func:`fix_schema` is the one fixed row a whole capture lands in - columns
spelled by the dictionary's folded canonical names, ``msgtype`` and never
``35``, so a column is found with ``schema.index_of("msgtype")`` and nothing has
to be resolved per row; the tag stays on each column's ``fix:tag``.
:func:`fix_schema_carrying` puts a capture's own columns in front of them,
dropping a capture column whose folded name a FIX column already takes.
:func:`fix_crate_fields` lists what this crate itself adds beside the
specification: twenty standard fields from tag 65000, above every tag FIX or
a venue publishes, so they need no branch of their own. ``msghash``,
``version``, ``symbolticker``, ``timestamp``, ``unixpartition``,
``parentclordid`` and ``parentorderid``; what a bridge's own log states about a
line - ``sendersessionid``, the session the message itself names, ``msgctxid``,
the plugin ``pluginid`` that logged it and the ``prevpluginid`` it came
through before that, and the session names ``sendersessionname`` and
``targetsessionname`` the line spells; the three facts a row derives from what
the message said - ``isincode``,
``miccode`` and ``state``; and the three identities a stream implies -
``instid``, ``id`` and ``persistentid``.

:class:`FixLifecycle` stamps those three. It reads a stream once, in order,
through :meth:`FixLifecycle.fill`: every message gets the instrument's identity
and its own, and one naming an order gets the identity of the chain that order
belongs to - joined on ``OrigClOrdID``, ``ClOrdID``, ``OrderID``,
``SecondaryClOrdID`` and ``SecondaryOrderID``, closed by a terminal state, so
:meth:`FixLifecycle.alive` counts the orders still open and
:meth:`FixLifecycle.clear` forgets them. :meth:`FixCodec.lifecycle` runs one over
an iterable of messages, lazily, and composes over a whole capture through
:meth:`FixCodec.messages` and :meth:`FixCodec.arrow_reader`.

A branch is a ``str`` wherever it is a *key*; :class:`FixBranch` is what a
*declaration* is, because a declaration also carries the dialect's default FIX
version and the other spellings it answers to.

The registry stores scalar ``fields`` and named ``messages``, ``components``,
and ``groups``. Enum codes remain inline in each field's ``fix:codes`` metadata.
Repeating counts such as ``NoPartyIDs`` are ``int32`` fields; ``Parties`` is a
separate list of ``Party`` components. :class:`MsgType` borrows one immutable,
registry-owned message definition and keeps its complete case-sensitive wire code.

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
    FixLifecycle,
    FixRegistry,
    FixMessages,
    MsgType,
    UlPlugin,
    UlPlugins,
    fix_cfb_fields,
    fix_crate_fields,
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
    "FixLifecycle",
    "FixRegistry",
    "FixMessages",
    "MsgType",
    "UlPlugin",
    "UlPlugins",
    "fix_cfb_fields",
    "fix_crate_fields",
    "fix_schema",
    "fix_schema_carrying",
    "fix_schema_tags",
    "fix_ulbridge_fields",
    "global_registry",
    "install_global_registry",
]
