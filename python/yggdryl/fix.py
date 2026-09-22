"""FIX field definitions, messages and the codec, over the fields and handles :mod:`yggdryl` already has.

A FIX field is an ordinary :class:`~yggdryl.Field` whose ``FIX:`` metadata the
protocol view ``field.fix`` reads and writes as typed properties - ``id``,
``tag``, ``tags``, ``branches``, ``aliases``, ``identifiers``, ``codeset``,
``description``, ``derivation`` - so nothing here is a second field class. A
field is its tag and its name together: ``id`` is the ``int`` the core derives
from both under the one fold, never stored, and what the dictionaries that
contributed the field say is ``branches``, a sorted list of names that a caller
filters on and no lookup consults. The registry is one namespace:
:class:`FixRegistry` resolves scalar fields, components and repeating groups by
identifier, by tag, by counter, by name or by dotted path - a Struct is a
component, a List of Structs or a Map a group, a message a component carrying
``FIX:msgtype``, each filed by :meth:`FixRegistry.insert` under the shape it
has - and persists them as JSON shards through any ``IOBase`` location, the
fixed row among them as ``components/fixmsg.json``. Every registry holds the
crate's own definitions from construction - its columns from tag 65003 and the
two Map groups ``identifiers`` and ``metadata`` - and seeds the standard clocks
``SendingTime`` (52) and ``TransactTime`` (60) beside them as ordinary
definitions a loaded dictionary may supply itself; ``len`` counts the scalar
fields, the components and the groups, and iteration walks the scalars. A store
writes the crate's definitions like any other and never lets a stored copy
override them. A tag is a positive ``int``: ``fix.tag``, ``fix.tags`` and
``fix.counter`` refuse 0, which only an unresolved entry records. Resolution,
folding, merging, sharding and validation are native; this module only names
them.

:meth:`FixRegistry.from_cfb_file` reads one Ullink ``CBlock`` whole: the
dictionary its vocabulary declares - every field keyed by its ``FIX:tag``,
stamped with the dialect in ``FIX:branches`` and reading by the code set the
file's maps decode for it, which the dictionary carries under a name of its
own - and the message roots its grammar bindings describe.
:meth:`FixRegistry.add_cfb` folds that same file into a dictionary that
already exists, adding what is absent, merging what is stored, and writing
nothing at all when it refuses.

:class:`FixMsg` is a typed market event with a content row. The typed facts
live in three holders and two extras - :meth:`FixMsg.event`, the facts the
core's graph vocabulary answers (``curruuid``, ``crossuuid``, ``crosscode``,
``currhashcode``, ``crosshashcode``, ``identifiers``, ``parentuuids``, ``currunix``,
``state``, ``seqnum``, the lifecycle's ``creaunix``, ``exprtime``,
``prevunix``, ``prevuuid`` and ``snapunix``, the market's ``px``, ``qty``,
``currency``, ``unit``, ``side``, its ISIN, CUSIP, SEDOL, Bloomberg, CFI and
MIC codes and the bid and ask lanes); :meth:`FixMsg.header`, the standard
header (``beginstring``, ``msgtype``, ``sendercompid``, ``targetcompid``,
``msgseqnum``, ``sendingtime``, ``possdupflag``, ``msgdirection``); the
message type's fixed four-byte ``msgcat``;
:meth:`FixMsg.capture`, what the line's own bridge row header said about
the capture it was written for (``msgpluginid``, ``msgctxid``,
``msgsessionid``) - never what a *reader* said about the line, which is
held nowhere on a message; the free
:attr:`FixMsg.text` of tag 58; and a bridge's own :attr:`FixMsg.metadata`,
the ``TECH.`` and ``firm.`` keys under the spelling it gave them - and the
row holds everything else the message states: the dictionary's fields,
groups as lists beside their counter, components as structs. The graph
facts a consumer reads most are the message's own properties too. A lookup
by a typed tag - a header tag, a crate column, the event's own ``15``,
``54``, ``461`` and the four lane tags, ``58`` - answers the holder, typed as
its column is; any other key reaches the row. :meth:`FixMsg.set` and
:meth:`FixMsg.remove` write both the same way, and every write settles the
identity again: the cross code from the first stated of ``OrderID``,
``ClOrdID``, ``OrigClOrdID``, ``QuoteID``, ``QuoteReqID`` and ``MDReqID``, the
hash code over the facts and the row, the identities from both.
:meth:`FixMsg.entries` reads the row as a tree of ``(tag, name, value,
entries)`` tuples; :meth:`FixMsg.into_bytes` and :meth:`FixMsg.into_text`
re-emit the message as it now stands, the header and the event's own tags
in front; :meth:`FixMsg.digest` digests that wire.

:class:`FixCodec` parses lines into messages and chains them, each as an
iterator and each with an Arrow-batch twin. :meth:`FixCodec.parse_line`
turns one captured line into a lazy :class:`FixMessages` stream and
:meth:`FixCodec.parse_lines` a whole iterable of lines, one line at a time;
:meth:`FixCodec.parse_text_line` and :meth:`FixCodec.parse_text_lines` read
the lines a text reader answers, the line's own body beside the
row-header captures that state its ``msgpluginid``, ``msgsessionid``,
``msgctxid`` and ``msgseqnum`` - ``capture_names`` is what says which
capture is which, once for the whole run, and a ``msgdirection`` capture or
column states the direction FIX's own tag 385 carries, filled from the verb
in front of the payload where the row states none. A line's ``timestamp``
is capture context and stamps nothing. A parse builds the message, lifts
its typed facts, explodes a nested ``XmlData`` into it, restates deprecated
fields to their latest aliases, runs the dictionary's ``FIX:derivation``
rules, fills the identifiers the message component declares and an order's
lanes, and settles the identity: ``SendingTime`` is the message's own, else
the carrier's, else the codec's ``default_sending_time``, else UTC now
read once, and the instant ``currunix`` is the stated one, else the official
transaction clock standing within ``official_time_delay_ms`` of that
``SendingTime`` - a ``TransactTime``, else the ``TrdRegTimestamp`` its
``TrdRegTimestampType`` says is about the event or a hop - else that
``SendingTime``; what ``OrigSendingTime`` says is the lifecycle's to read off
the structured message. No clock is read after that intake,
so replay carries the settled row or pins the same ``default_sending_time``.
There is no separate enriching step: a parsed message already carries what
it implied.
:meth:`FixCodec.parse_text_arrow_reader` turns a whole Arrow capture into
batches of FIX rows - the capture's own columns first, the dictionary's fixed
columns after, one source row's columns repeated for each message a bulk
document expands to - closed on the bytes each row lands as against the
codec's ``batch_byte_size``. :meth:`FixCodec.lifecycle` chains a stream
of messages lazily - each stated as the one after the live message it
follows under its cross identity, carrying ``prevuuid``, ``prevunix``,
``seqnum``, the predecessor's whole lineage as its ``parentuuids`` and the
lifecycle's ``creaunix`` - and :meth:`FixCodec.lifecycle_arrow_reader` does the same
over batches of rows without parsing them again. Both compose through the two
converters every stage composes over batches: :meth:`FixCodec.messages`
reads a batch back as the messages that made it and
:meth:`FixCodec.arrow_reader` writes messages as batches under a schema.
:meth:`FixCodec.write_arrow_reader` is the encode direction, re-emitting
every row's wire. :meth:`FixCodec.format_messages` and
:meth:`FixCodec.format_arrow_reader` answer the same messages under whatever
field a consumer reads by - a venue's own message type, :func:`fix_schema`
itself, which keeps every column a capture lands in, or any Struct root a
caller built. A pin - ``default_sending_time``, ``separator``,
``payload_column``, ``null_values``, ``direction``, ``batch_byte_size``,
``snapshot_ns`` and ``official_time_delay_ms`` - is on the codec; a positive
``snapshot_ns`` emits independent living views on its epoch-aligned grid and
zero, a negative width or ``None`` disables them, while
``official_time_delay_ms`` is how far from ``SendingTime(52)`` an official
transaction clock may stand and still date the message. A stage is a call, and
no pin decides a version: a row states one in its ``beginstring`` capture, else
the line implies it.
:func:`fix_schema` is the one fixed row a whole capture lands in - the
crate's own columns first, its clocks then its identities, then the standard
header, the fields a consumer reads, the four groups worth persisting whole,
the trailer and ``MsgDirection`` (385) - each spelled by the dictionary's
folded canonical name, ``msgtype`` and never ``35``, so a column is found
with ``schema.index_of("msgtype")`` and nothing has to be resolved per row;
the tag stays on each column's ``FIX:tag``. One ``fixentries`` list closes
the row with the whole content under the ``nofixentries`` that counts it,
where an unresolved key has tag 0; ``beginstring``, ``currunix``, ``creaunix``,
``currhashcode``, ``crosshashcode``, ``curruuid`` and ``crossuuid`` are its
non-null columns. :func:`fix_schema_carrying` puts a capture's own columns in
front of them, dropping a capture column whose folded name a FIX column
already takes. :meth:`FixMsg.into_row` fills that row and
:meth:`FixMsg.from_row` reads one back, the content rebuilt from the
entries. :func:`fix_crate_fields` lists what this crate itself adds beside
the specification, in tag order.

A dictionary is a membership, not a namespace: :meth:`FixRegistry.from_cfb_file`
and :meth:`FixRegistry.add_cfb` take a ``dialect`` and stamp it on every
field the file produces, and :meth:`FixRegistry.dialects` lists the names any
field or definition carries.

Repeating counts such as ``NoPartyIDs`` are ``int32`` fields; ``Parties`` is a
separate list of ``Party`` components, reached by its name or by
:meth:`FixRegistry.field_by_counter`. A crate Map is a group too: its
occurrence is its non-null entries Struct, its key stays non-null and its own
tag is its counter.

A field's values are drawn from a *named code set*, and the dictionary holds
each set once. ``field.fix.codeset`` is the name the field reads by - the one
thing its ``FIX:codeset`` metadata carries - and the members live under that name
in the registry, persisted as ``codesets/<name>.json`` beside ``fields/``,
``components/`` and ``groups/`` and as the fourth ``codesets`` key of
:meth:`FixRegistry.into_json`. So one vocabulary is named, documented and
aliased in one place however many fields read by it.
:meth:`FixRegistry.codeset` answers a set's members and
:meth:`FixRegistry.codeset_of` the members one field reads by, each a list of
``{"value", "name", "description", "aliases", "group"}`` records in the order
the set states them; :meth:`FixRegistry.codeset_names` lists the names held.
:meth:`FixRegistry.set_codeset` states a set's members as those records, an
empty list removing the set;
:meth:`FixRegistry.merge_codeset` folds into what it held, keyed by wire value,
the reading the dictionary already holds winning a shared one and every
spelling either side declared kept as an alias, and
:meth:`FixRegistry.remove_codeset` takes one away, refusing while a field still
reads by it. A dictionary refuses a field naming a set it does not hold - at
insert, update, ``from_fields``, ``from_json`` and store load alike - so a set
is stated before a field points at it, and a merge folds the sets first while
each field keeps the name it already reads by.

A component's ``field.fix.identifiers`` accepts an iterable of its direct
scalar member names, aliases or decimal tags and stores canonical names in
component order; empty input removes the property. Empty, comma-bearing,
missing, nested, ambiguous or duplicate selections fail atomically, and
registry intake resolves raw metadata through the same owner after references
load. An incoming declaration replaces the previous one whole on merge.
:class:`MsgType` is one immutable message definition beside the registry it
came from, keeps its complete case-sensitive wire code and compiles identifier
selection once. :meth:`MsgType.identifier_values` returns a list of read-only
declaration Field clones paired with native Scalar wrappers in declaration
order, skipping absent/null members and never flattening a group. Exact
canonical names win; a renamed member's tag is used only when unique in both
declaration and row.
"""

from __future__ import annotations

from ._native import (
    FixMsg,
    FixCapture,
    FixCodec,
    FixHeader,
    FixRegistry,
    FixMessages,
    MarketEventData,
    MsgType,
    fix_crate_fields,
    fix_schema,
    fix_schema_carrying,
    fix_schema_tags,
    global_registry,
    install_global_registry,
)

__all__ = [
    "FixMsg",
    "FixCapture",
    "FixCodec",
    "FixHeader",
    "FixRegistry",
    "FixMessages",
    "MarketEventData",
    "MsgType",
    "fix_crate_fields",
    "fix_schema",
    "fix_schema_carrying",
    "fix_schema_tags",
    "global_registry",
    "install_global_registry",
]
