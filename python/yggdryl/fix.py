"""FIX field definitions, over the fields and handles :mod:`yggdryl` already has.

A FIX field is an ordinary :class:`~yggdryl.Field` whose ``fix:`` metadata the
protocol view ``field.fix`` reads and writes as typed properties - ``id``,
``tag``, ``tags``, ``branches``, ``aliases``, ``identifiers``, ``description`` - so nothing here
is a second field class. A field is its tag and its name together: ``id`` is
the ``int`` the core derives from both under the one fold, never stored, and
what the dictionaries that contributed the field say is ``branches``, a
sorted list of names that a caller filters on and no lookup consults. The
registry is one namespace: :class:`FixRegistry` resolves those fields by
identifier, by tag, by name or by dotted path and persists them as JSON
shards through any ``IOBase`` location, and :class:`FixMsg` is one row typed against the registry it
was resolved against, written through :meth:`FixMsg.set` and
:meth:`FixMsg.remove` and read back from a fixed row by :meth:`FixMsg.from_row`.
Every registry holds thirty-four crate-owned scalar fields, the ``altids`` Map
group, the ``instids`` Struct and the separate ``pluginconfig``
component/message from construction,
and :class:`FixRegistry` seeds the standard clocks ``SendingTime`` (52) and
``TransactTime`` (60) beside them as ordinary definitions a loaded dictionary
may supply itself, so ``len(FixRegistry())`` is 36; ordinary size and
iteration count only the scalars. A store writes the crate's definitions like
any other and never lets a stored copy override them. A tag is a positive ``int``: ``fix.tag``,
``fix.tags`` and ``fix.counter`` refuse 0, which only an unresolved arrival
entry records. Resolution, folding, merging, sharding and validation are
native; this module only names them.

:func:`fix_cfb_fields` reads one Ullink ``CBlock`` for the vocabulary it
declares, in declaration order and keyed, which is what
:meth:`FixRegistry.add_fields` folds into a dictionary that already exists -
adding what is absent, merging what is stored, and writing nothing at all when
it refuses. :meth:`FixRegistry.from_cfb_file` is the same file read whole, answering
a dictionary and the message roots its grammar bindings describe.

:class:`Plugin` carries one bridge configuration: the ObjectName the read
named it by and the attributes it stated, which is all of it - what the Jolokia
exchange wrapped them in is the transport's. :class:`Plugins` lazily yields
the configurations a single or bulk document names, and none where it names
none, and :meth:`Plugin.into_fixmsg` converts one to a flat typed message. :func:`fix_plugin_fields` is the dictionary
those attributes type against, which
:meth:`FixRegistry.with_plugin_fields` registers. :func:`fix_plugin_message`
is the message they make up - the attributes beside FIX's own ``MsgType``,
``BeginString``, ``SenderCompID`` and ``TargetCompID`` - and registering that
one is nobody's choice: ``FixRegistry()`` holds it as it holds the crate's own
fields, so a configuration reads as ``pluginconfig`` and answers
``PLUGINCONFIG_CODE_NAME[0]`` on tag 35 whatever dictionary met it. The code
is a fact the crate adds rather than one the document made, so it is a built
child and the wire re-emits without a ``35=``.

:class:`FixCodec` parses lines into messages and enriches messages, each
as an iterator and each with an Arrow-batch twin. :meth:`FixCodec.parse_line`
turns one captured line into a lazy :class:`FixMessages` stream and
:meth:`FixCodec.parse_lines` a whole iterable of lines, one line at a time;
:meth:`FixCodec.parse_text_line` and :meth:`FixCodec.parse_text_lines` read
the lines a text reader answers, the line's own body beside the
row-header captures that state its ``pluginid`` and ``beginstring`` -
``capture_names`` is what says which capture is which, once for the whole
run, and a ``msgdirection`` capture or column states the direction FIX's
own tag 385 carries, filled from the verb in front of the payload where the
row states none. A line's ``timestamp`` is capture context and stamps nothing.
Every message it builds opens with ``beginstring`` - the wire's own, else the
version the message was read at - and carries the settled bundle, each member
non-null: ``SendingTime`` is the message's own, else the carrier's, else the
codec's ``default_sending_time``, else UTC now read once; ``snapshotat`` is the
real event instant, ``TransactTime`` else ``SendingTime``; ``updatedat`` and
``createdat`` start at ``snapshotat``; ``code`` is the empty unknown name; and
``msghash`` and ``msgphash`` are computed. No clock is read after that intake, so
replay carries the settled row or pins the same ``default_sending_time``, and
:meth:`FixMsg.updatedat`, :meth:`FixMsg.createdat`, :meth:`FixMsg.msghash` and
:meth:`FixMsg.msgphash` always answer. ``msghash`` is sixteen ``fixedbinary(16)``
bytes over ``updatedat``'s nanoseconds and the message's named content, and
``msgphash`` sixteen bytes over ``code`` alone; a row change recomputes both, a
stated one that disagrees is refused, and :meth:`FixMsg.remove` refuses a
mandatory field with ``ValueError``.
:meth:`FixCodec.parse_text_arrow_reader` turns a whole Arrow capture into
batches of FIX rows - the capture's own columns first, the dictionary's fixed
columns after, one source row's columns repeated for each message a bulk
document expands to - closed on the raw bytes of the payload column against
the codec's ``batch_byte_size``. A capture's ``timestamp`` column is carried as
its own context; its ``pluginid`` column names the plugin that logged the line;
and any other column named after a field - ``prevpluginid``,
``bridgesessionid``, or ``msgseqnum`` for ``MsgSeqNum`` - fills that field
where the frame did not state it, without becoming an entry. :meth:`FixCodec.enrich_message` and
:meth:`FixCodec.enrich_messages` fill what a message implied but did not carry,
and :meth:`FixCodec.enrich_messages_arrow_reader` does the same over batches
of rows without parsing them again. After restatement and existing fills,
``altids`` carries the message's declared direct identifiers as sorted,
unique canonical-name/text pairs, a native Map that crosses as a Python dict
and an Arrow Map with sorted keys. Null identifiers contribute nothing;
a known message with no stated identifiers gets an empty map, an unknown
message type gets none, and a stated non-null map, including an empty one,
is preserved. Scalar text conversion is native and an unrepresentable value
raises its typed error at the identifier's field path; no group is flattened,
no arrival entry is synthesized, and a second enrichment is equal.
:meth:`FixCodec.enrich_messages`
remembers every configuration it passes, by the plugin's ``Name``, and fills
the ``SenderCompID`` and ``TargetCompID`` of a later message naming that
plugin where it stated none of its own; :meth:`FixCodec.enrich_message` - one
message, not a stream - remembers nothing. Both compose through the two
converters every stage composes over batches:
:meth:`FixCodec.messages` reads a batch back as the
messages that made it and :meth:`FixCodec.arrow_reader` writes messages as
batches under a schema. :meth:`FixCodec.write_arrow_reader` is the encode
direction, re-emitting every row's wire. :meth:`FixCodec.format_messages` and
:meth:`FixCodec.format_arrow_reader` answer the same messages under whatever
field a consumer reads by - a venue's own message type, :func:`fix_schema`
itself, which keeps every column a capture lands in, or any Struct root a
caller built. A pin - ``default_sending_time``, ``separator``,
``payload_column``, ``null_values``, ``direction``, ``batch_byte_size`` - is on
the codec; a stage is a call, and no pin decides a version: a row states one in
its ``beginstring`` capture, else the line implies it.
:func:`fix_schema` is the one fixed row a whole capture lands in - columns
spelled by the dictionary's folded canonical names, ``msgtype`` and never
``35``, so a column is found with ``schema.index_of("msgtype")`` and nothing has
to be resolved per row; the tag stays on each column's ``fix:tag``. Its tags
end with the crate's own and ``MsgDirection`` (385), and one ``fixentries``
list closes the row with the whole arrival record, where an unresolved key has
tag 0; ``beginstring``, ``sendingtime``, ``updatedat``, ``msghash``,
``msgphash``, ``createdat`` and ``code`` are its non-null columns, while
``snapshotat`` is nullable because only a snapshot stamps it. A replayable row carries the whole settled bundle, and
:meth:`FixMsg.from_row` refuses one that lacks a member.
:func:`fix_schema_carrying` puts a capture's own columns in front of them,
dropping a capture column whose folded name a FIX column already takes.
:func:`fix_crate_fields` lists what this crate itself adds beside the
specification: 36 definitions in tag order, thirty-four scalar fields at tags
65001 to 65015, 65017 to 65019 and 65021 to 65038, the nullable sorted-key
``map<utf8, utf8>`` group ``altids`` at 65020 and the ``instids`` struct at
65036; the retired 65000, 65004 and 65016 are not reused.
The scalar fields are ``version``, ``symbolticker``, ``updatedat``,
``parentclordid`` and ``parentorderid``; what a bridge's own
log states about a line - ``sendersessionid`` and ``targetsessionid``, the
sessions the message itself names, ``bridgesessionid``, the session instance
the bridge handled the line on, ``msgctxid``, the plugin ``pluginid`` that
logged it and the ``prevpluginid`` it came through before that, the session
names ``sendersessionname`` and ``targetsessionname`` the line spells, and the
``sessionmsgid`` and ``sessionmsgseqid`` that join the bracket's parts; the
facts a row derives from what the message said - ``isincode``,
``bloombergcode``, ``cusipcode``, ``sedolcode``, ``miccode`` and
``state``; the ``fixedbinary(16)`` ``msghash`` and ``msgphash``; the previous
message's ``prevupdatedat`` and ``prevmsghash``; ``createdat``, ``code``,
``snapshotat``, ``recordedat``, ``expiredat`` and the two lane currencies
``bidcurrency`` and ``offercurrency``; the ``url``-typed ``sourceurl`` a line
was read from; and the
``nofixentries`` that counts the ``fixentries`` arrival record. ``updatedat``,
``msghash``, ``msgphash``, ``createdat`` and ``code`` are non-null, and
``snapshotat`` is empty on every row no snapshot was taken of. How a layout is
cut is the target's: an Iceberg table takes an ``hour`` transform over
``updatedat`` rather than reading a materialized column. The
separate ``pluginconfig`` component/message and the seeded clocks are not part
of this tag listing.

:class:`FixLifecycle` names the chains. It reads a stream once, in order,
through :meth:`FixLifecycle.fill`: a nonempty ``code`` selects its live chain
globally; otherwise the first identifier - stated ``altids``, else the message
type's declared identifiers - reaching a live chain under the instrument the
message names lends that chain's code, and a new chain is named
``<scope hex or ->/<identifier>``, never taking an identifier another live
chain holds. The instrument is the digest of what the message says it is -
its market, its classification, its ISIN else its symbol, and its currency -
and no column carries it. ``msgphash`` hashes that code. Every accepted message has
``updatedat`` floored to its epoch grid - :attr:`FixLifecycle.DEFAULT_INTERVAL_NS`,
one second, unless ``interval_ns`` says otherwise - while ``snapshotat`` keeps
the real instant; takes its live chain's first accepted ``createdat``; and fills
each absent ``prevupdatedat`` and ``prevmsghash`` from the chain's last message. A
terminal state closes the chain, so :meth:`FixLifecycle.alive` counts the events
still open and :meth:`FixLifecycle.clear` forgets them, keeping the interval,
which :meth:`FixLifecycle.set_interval_ns` changes only while none is live.
:meth:`FixLifecycle.snapshot` runs the same transition and answers only an
arrival off its grid in a bucket above the highest its live chain consumed,
and :meth:`FixLifecycle.snapshots` does so lazily over an iterable, taking the
lifecycle's live state with it. :meth:`FixCodec.lifecycle` fills an iterable of
messages at the default interval, lazily, and each composes over a whole
capture through :meth:`FixCodec.messages` and :meth:`FixCodec.arrow_reader`.

A dictionary is a membership, not a namespace: :meth:`FixRegistry.from_cfb_file`
and :meth:`FixRegistry.add_cfb_file` take a ``dialect`` and stamp it on every
field the file produces, :meth:`FixRegistry.dialects` lists the names any
field or definition carries, and ``PLUGIN_DIALECT`` is the one this crate
stamps itself, on :func:`fix_plugin_fields`. A message type is not one:
``pluginconfig`` is registered by every registry and carries no membership,
because the code it answers to is the crate's own rather than a dictionary's.

The registry stores scalar ``fields`` and named ``components`` and ``groups``;
a message is a component carrying ``fix:msgtype``. Enum codes remain inline in
each field's ``fix:codes`` metadata.
Repeating counts such as ``NoPartyIDs`` are ``int32`` fields; ``Parties`` is a
separate list of ``Party`` components. A crate Map is a group too: its occurrence is
its non-null entries Struct, its key stays non-null and its own tag is its
counter, with no second scalar counter or numeric wire tags for key/value.

A component's ``field.fix.identifiers`` accepts an iterable of its direct
scalar member names, aliases or decimal tags and stores canonical names in
component order; empty input removes the property. Empty, comma-bearing,
missing, nested, ambiguous or duplicate selections fail atomically, and
registry intake resolves raw metadata through the same owner after references
load. An incoming declaration replaces the previous one whole on merge.
:class:`MsgType` borrows one immutable, registry-owned message definition,
keeps its complete case-sensitive wire code and compiles identifier selection
once. :meth:`MsgType.identifier_values` returns a list of read-only declaration
Field clones paired with native Scalar wrappers in declaration order, skipping
absent/null members and never flattening a group. Exact canonical names win;
a renamed member's tag is used only when unique in both declaration and row.
"""

from __future__ import annotations

from ._native import (
    PLUGIN_DIALECT,
    PLUGINCONFIG_CODE_NAME,
    FixMsg,
    FixCodec,
    FixLifecycle,
    FixRegistry,
    FixMessages,
    MsgType,
    Plugin,
    Plugins,
    fix_cfb_fields,
    fix_crate_fields,
    fix_schema,
    fix_schema_carrying,
    fix_schema_tags,
    fix_plugin_fields,
    fix_plugin_message,
    global_registry,
    install_global_registry,
)

__all__ = [
    "PLUGIN_DIALECT",
    "PLUGINCONFIG_CODE_NAME",
    "FixMsg",
    "FixCodec",
    "FixLifecycle",
    "FixRegistry",
    "FixMessages",
    "MsgType",
    "Plugin",
    "Plugins",
    "fix_cfb_fields",
    "fix_crate_fields",
    "fix_schema",
    "fix_schema_carrying",
    "fix_schema_tags",
    "fix_plugin_fields",
    "fix_plugin_message",
    "global_registry",
    "install_global_registry",
]
