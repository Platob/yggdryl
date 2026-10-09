"""FIX field definitions, messages and the codec, over the fields and handles :mod:`yggdryl` already has.

A FIX field is an ordinary :class:`~yggdryl.Field` whose ``FIX:`` metadata the
protocol view ``field.fix`` reads and writes as typed properties - ``id``,
``tag``, ``tags``, ``sources``, ``aliases``, ``identifiers``, ``codeset``,
``description`` - so nothing here is a second field class. A
field is its tag and its name together: ``id`` is the ``int`` the core derives
from both under the one fold, never stored, and the sources that contributed
the field are ``sources``, a sorted list of ids that a caller filters on and
no lookup consults, each naming an entry of the registry's catalog
(:meth:`FixRegistry.sources`). The registry is one namespace:
:class:`FixRegistry` resolves scalar fields, components and repeating groups by
identifier, by tag, by counter, by name or by dotted path - a Struct is a
component, a Serie of Structs or a Map a group, a message a component carrying
``FIX:msgtype``, each filed by :meth:`FixRegistry.insert` under the shape it
has - and persists them as JSON shards through any ``IOBase`` location, the
fixed row among them as ``components/fixmsg.json``. Every registry holds the
crate's own definitions from construction - its columns from tag 65001 and the
Map group ``metadata`` - and seeds the standard clocks
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
stamped with the dialect in ``FIX:sources`` and reading by the code set the
file's maps decode for it, which the dictionary carries under a name of its
own, and the dialect's catalog entry naming the file and the role of the
plugin its root's ``type`` names - and the message roots its grammar
bindings describe.
:meth:`FixRegistry.add_cfb_file` folds that same file into a dictionary that
already exists, adding what is absent, merging what is stored, and passing
over - and naming, in the report it answers - what the file declares otherwise
than the dictionary already does; a datatype declared at another precision of
the dictionary's - a ``float`` against ``decimal128``, a ``string`` against
``ccy`` - folds under the dictionary's and is counted as ``restated``. A file
that will not parse writes nothing.

:class:`FixMsg` is a typed market event with a content row. The typed facts
live in three holders and two extras - the facts the core's graph
vocabulary answers, each the message's own property (``uuid``, ``crossuuid``, ``crosscode``,
``hashcode``, ``crosshashcode``, ``transunix``, ``state``, ``seqnum``,
the lifecycle's ``creaunix``, ``exprunix``, ``sendunix``,
``prevunix``, ``prevuuid`` and ``snapunix``; the market's ``price``,
``currency``, ``quantity``, ``unit``, ``side`` - a :class:`yggdryl.Side`,
``UKNW`` where none is stated - its ``securityids`` - an :class:`yggdryl.Identifiers`
map keyed ``src:type`` of the codes it is known by, each sourced and typed ``isin``,
``cusip``, ``figi`` - and the ``isincode`` read off them, its CFI and
MIC codes, the ``execunix`` clock it last executed at, last, average,
cumulative, remaining and previous values, spot
rate and forward points, the ``fxrates`` it states, the bid and ask it quotes
(``bidpx``, ``bidqty``, ``bidccy``, ``askpx``, ``askqty``, ``askccy``),
``ticker`` and ``metadata``; the operation's time in force, tradability, the
``identifiers`` it names, each typed by the field that stated it and carrying the
parents its chain gave it (``origclordid``, ``parentorderid``, ``origorderid``),
and the ``partyids`` its ``Parties`` groups and its ``Account(1)`` are, each party id
typed by its role's name and sourced by its source's, the account typed
``account``; a key no dictionary resolves whose name is an identifier's
(``OMS_ClOrdID``, ``firm.x.ParentOrderID``) lands in the set its type belongs to
and stays in ``metadata`` as it arrived);
:meth:`FixMsg.header`, the standard
header (``beginstring``, ``msgtype``, ``sendercompid``, ``targetcompid``,
``msgseqnum``, ``sendingtime``, ``possdupflag``, ``msgdirection``); the
business category ``marketdatakind``, the :class:`yggdryl.MarketDataKind` member the
message type is filed under (``MsgType.marketdatakind`` answers the same member for
the definition); ``msgpluginside``, the :class:`yggdryl.Side` member
naming the role of the plugin whose session produced the message - the
codec's ``source`` entry's, ``UKNW`` where none is named - never a FIX tag's
and independent of ``Side(54)``; the ``strikepx`` of the option the message
identifies, its market fact derived from ``StrikePrice(202)``;
:meth:`FixMsg.capture`, what the line's own bridge row header said about
the capture it was written for (``msgpluginid``, ``msgpluginside``, ``msgctxid``,
``msgsessionid``, and the ``msgsesseventid`` the message type, session,
context and ``MsgSeqNum`` join to by ``:``) - never what a *reader* said
about the line, which is held nowhere on a message; the free
:attr:`FixMsg.text` of tag 58; and a bridge's own :attr:`FixMsg.metadata`,
the ``TECH.`` and ``firm.`` keys under the spelling it gave them - and the
row holds everything else the message states: the dictionary's fields,
groups as series - each its list alone, its length the count - components
as structs.
:meth:`FixMsg.market_data` answers the typed graph leaves the message
expands to - an order, a quote, an execution, a trade or, for a book ``W`` or
``X``, one per entry or one snapshot control - each a
:class:`yggdryl.graph.MarketData`. A lookup
by a typed tag - a header tag, a crate column, one of the fields a message
lifts (its identifiers, prices, quantities and FX parts), ``58`` - answers
the holder, typed as its column is; any other key reaches the row, the
``15``, ``54``, ``461`` and ``132`` to ``135`` the market facts are read from
included, as the text the message stated. :meth:`FixMsg.set` and
:meth:`FixMsg.remove` write both the same way, and every write settles the
identity again: the cross code from the first stated of ``OrderID``,
``ClOrdID``, ``OrigClOrdID``, ``QuoteID``, ``QuoteReqID`` and ``MDReqID``,
stored as ``{kind}:{side}:{base}`` (a buy order ``O-1`` is ``10:1:O-1``), the
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
capture is context and stamps nothing; the line's own ``transunix`` - an
``mtime`` capture, else its handle's modification time - is the message's
``sendunix``. A parse builds the message, lifts
its typed facts, explodes a nested ``XmlData`` into it, restates deprecated
fields to their latest aliases, runs the crate's native derivations, reads
the identifier maps off the fields that state them, splits an execution a
report or a trade states into sided messages of their own - a quote stays
one message holding both its legs - and settles the identity: ``SendingTime`` is the message's own, else
a row cell reaching tag 52, else the ``transunix`` of the line it was read out
of, else the codec's ``default_sending_time``, else UTC now
read once - a clock the parse supplied is never the message's own, so the
wire and the ``sendingtime`` column state none - and the instant
``transunix`` is the stated one, else the official
transaction clock standing within ``official_time_delay_ms`` of that
``SendingTime`` - a ``TransactTime``, else the ``TrdRegTimestamp`` its
``TrdRegTimestampType`` says is about the event or a hop - else that
``SendingTime``; what ``OrigSendingTime`` says is the lifecycle's to read off
the structured message. A message reporting an execution that states no
execution clock executed at that instant: its ``execunix`` is its
``transunix``. No clock is read after that intake,
so replay carries the settled row or pins the same ``default_sending_time``.
There is no separate enriching step: a parsed message already carries what
it implied. Nothing a capture states is an error: a value that will not type is
null beside an anomaly, a clock naming no instant is left unstated, and a line
or frame that builds no message is left out - each with a deduplicated
``logging`` warning under ``yggdryl.<module path>`` such as
``yggdryl.fix.messages`` - so only a source's own failure raises.
:meth:`FixCodec.parse_text_arrow_reader` turns a whole Arrow capture into
batches of FIX rows - the capture's own columns first, the dictionary's fixed
columns after, one source row's columns repeated for each message a bulk
document expands to - closed on the bytes each row lands as against the
codec's ``batch_byte_size``. :meth:`FixCodec.lifecycle` chains a stream
of messages lazily - each placed among the messages of its instant by
content, then stated as following the live message under its cross
identity, carrying ``prevuuid``, ``prevunix``, its place at ``seqnum`` the
higher of its own and one past the predecessor's where that happened at
the same instant or later, the lifecycle's ``creaunix`` and every ``metadata``
key of the chain it does not state - and :meth:`FixCodec.lifecycle_arrow_reader` does the same
over batches of rows without parsing them again. Both compose through the two
converters every stage composes over batches: :meth:`FixCodec.messages`
reads a batch back as the messages that made it and
:meth:`FixCodec.arrow_reader` writes messages as batches under a schema.
:meth:`FixCodec.book_arrow_reader` streams sorted messages through native
market data and books into lifted ``marketdata`` batches, one
``book_event`` row per book and book key - the instrument's ISIN, else its
ticker, else ``XX0000000000`` - read back by
:meth:`yggdryl.graph.MarketData.from_arrow_reader`; ``snapshot_millis``
selects an epoch-aligned snapshot grid, at which a complete book is
written, every other book a delta book stating its delta and events, and
``filter`` narrows what the books fold. A book folds orders, quotes and book
messages into its sides, records an execution among its events, and never
reaches a trade. Lifecycle enrichment remains an explicit composition.
:meth:`FixCodec.market_data` is the sorted door: it collects a
capture, admits what the book door admits, expands each message and answers
the market data stably sorted by the instant a book folds them at, nothing a
message states raising; :meth:`FixCodec.market_arrow_reader` writes them as
``marketdata`` rows and :meth:`FixCodec.market_data_arrow_reader` reads
them off FIX rows. Each leaf carries, in its metadata, what its message
states that no typed column reads and no identifier map of the leaf holds, and
lifts into its ``identifiers`` each scalar of it whose key ends with an identifier
its message's type declares, unless the codec's ``market_metadata`` is off.
:meth:`FixCodec.write_arrow_reader` is the encode direction, re-emitting
every row's wire. :meth:`FixCodec.format_messages` and
:meth:`FixCodec.format_arrow_reader` answer the same messages under whatever
field a consumer reads by - a venue's own message type, :func:`fix_schema`
itself, which keeps every column a capture lands in, or any Struct root a
caller built. A pin - ``default_sending_time``, ``separator``,
``payload_column``, ``null_values``, ``direction``, ``source`` (the sources
catalog entry whose plugin role every message is stamped with as
``msgpluginside``), ``batch_byte_size``,
``snapshot_ns``, ``sorted_lifecycle``, ``official_time_delay_ms``,
``dedup_window_ms`` and ``market_metadata`` - is on the codec; a positive
``snapshot_ns`` emits independent living views on its epoch-aligned grid -
each the live event as of its tick, dated at it, so its ``uuid`` is the
identity that tick derives - and
zero, a negative width or ``None`` disables them; ``sorted_lifecycle`` states
that the lifecycle's messages arrive in instant order, so it walks them one
epoch hour at a time instead of sorting the whole capture, while
``official_time_delay_ms`` is how far from ``SendingTime(52)`` an official
transaction clock may stand and still date the message, and ``dedup_window_ms``
is how long, in milliseconds of event time, the lifecycle remembers an identity
it yielded so it yields that identity once - one minute when not given, and
``None``, zero or a negative window remembering none. A stage is a call, and
no pin decides a version: a row states one in its ``beginstring`` capture, else
the line implies it.
:func:`fix_schema` is the one fixed row a whole capture lands in - the
crate's own columns first, its clocks then its identities, then the standard
header, the fields a consumer reads, the four groups worth persisting whole,
the trailer and ``MsgDirection`` (385) - each spelled by the dictionary's
folded canonical name, ``msgtype`` and never ``35``, so a column is found
with ``schema.index_of("msgtype")`` and nothing has to be resolved per row;
the tag stays on each column's ``FIX:tag``. One ``fixentries`` sorted map
closes the row with the residual content the dictionary resolves, each entry
under its ``tag:name`` key - the wire text of a leaf, the JSON of a group or
component - while a key no dictionary resolves lands in ``metadata``;
``beginstring``, ``transunix``, ``creaunix``,
``hashcode``, ``crosshashcode``, ``uuid`` and ``crossuuid`` are its
non-null columns. :func:`fix_schema_carrying` puts a capture's own columns in
front of them, dropping a capture column whose folded name a FIX column
already takes. :meth:`FixMsg.into_row` fills that row and
:meth:`FixMsg.from_row` reads one back, the content rebuilt from the
entries. :func:`fix_crate_fields` lists what this crate itself adds beside
the specification, in tag order.

A dictionary is a membership, not a namespace: :meth:`FixRegistry.from_cfb_file`
and :meth:`FixRegistry.add_cfb_file` take a ``dialect`` and stamp it on every
field the file produces in ``FIX:sources``, and :meth:`FixRegistry.dialects`
lists the ids any field or definition carries. What is known of a source is
held once, in the catalog a store writes as ``sources.json``:
:meth:`FixRegistry.sources` walks it as ``{"id", "file", "pluginside"}``
records in id order, :meth:`FixRegistry.get_source` reads one,
:meth:`FixRegistry.add_source` records one - an id already held taking only
the file and the role it lacked - and :meth:`FixRegistry.remove_source`
takes one away, refusing while a field still names it.

Repeating counts such as ``NoPartyIDs`` are ``int32`` fields of the
dictionary that frame a group on the wire and nothing else: ``Parties`` is a
serie of ``Party`` components whose length is its count, reached by its name
or by :meth:`FixRegistry.field_by_counter`, and no component, message, row or
entry lists the counter beside it. A crate Map is a group too: its
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

``ULBRIDGE_ROWHEADER`` is the row header a ULBridge log writes in front of
every line, as a ``rowheader`` for
:class:`~yggdryl.text.TextOptions` - the crate's own text rather than a
second copy of it. Its clock is ``mtime``, so the header dates each line it
matches: the capture is consumed into the line's ``transunix`` - the
``sendunix`` of its messages and the sending clock of one stating no
``SendingTime(52)`` - and read at ``datetime64(ns, UTC)`` under the text
options' ``timezone``, never the file's modification time. Four of the other
six captures are named for the fields they fill - ``msgsessionid``,
``msgctxid``, ``msgseqnum`` and ``msgpluginid`` - and ``msgthreadid`` and
``loglevel`` name none: they are columns of the line's row, carried in front
of a FIX row parsed from it, filling no field.
Its clock reads what bridges write, a point or a comma before three digits or
grouped microseconds, or no fraction at all - and a line a row header does not
match carries no capture context, which is what the lifecycle folds
deliveries on.
"""

from __future__ import annotations

from ._native import (
    ULBRIDGE_ROWHEADER,
    FixMsg,
    FixCapture,
    FixCodec,
    FixHeader,
    FixRegistry,
    FixMessages,
    MsgType,
    fix_crate_fields,
    fix_plugin_side as plugin_side,
    fix_schema,
    fix_schema_carrying,
    fix_schema_tags,
)

__all__ = [
    "ULBRIDGE_ROWHEADER",
    "FixMsg",
    "FixCapture",
    "FixCodec",
    "FixHeader",
    "FixRegistry",
    "FixMessages",
    "MsgType",
    "fix_crate_fields",
    "fix_schema",
    "plugin_side",
    "fix_schema_carrying",
    "fix_schema_tags",
]
