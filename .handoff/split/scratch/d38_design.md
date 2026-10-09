### D38 - the generic event's two instants are `transunix` and `sendunix`, and the element's own names drop `curr`

**Decision.** `Event`'s instant is `transunix` - the instant the operation
really happened, its transaction time - and its technical clock is `sendunix` -
the instant the message was sent or received on the wire. `currunix` and
`recdunix` are deleted at every door in one sweep, and so is the `curr` prefix
of the element's own two names (the user's amendment): `curruuid` is `uuid`
and `currhashcode` is `hashcode` - the element's own identifier and content
code need no qualifier, where `prevuuid`, `crossuuid`, `crosshashcode` and
`srcuuids` keep the prefix that says whose they are. Nothing else is renamed:
`creaunix`, `exprunix`, `prevunix`, `snapunix`, `execunix`, `crosscode` keep
their names, and `currency` is not a `curr` name.

**`transunix`** (`i64`, nanoseconds since the Unix epoch, UTC, never absent):
when the operation really happened - the identity and order axis `currunix`
was. `time_uuid` floors it to milliseconds, `is_after` orders by it, an
`InstantSequence` run is the events at one `transunix` and `seqnum` the place
within it, `prevunix` is the predecessor's, `creaunix` defaults to it, an
expiry's is its deadline and a snapshot view's the tick (`snapunix` the
original), candles bucket by it, `BookService` and the `Lifecycle` view filter
and order by it, `IsinRegistry` dates `updunix`/`firstunix`/`lastunix` by it,
`execunix` falls back to it. FIX: the row's stated `transunix`, else
`TransactTime(60)` or a `TrdRegTimestamps(768)` stamp of an event type within
`official_time_delay_ms` (1000) of the sending clock - the venue's own answer
to when it happened - else the sending clock: `official_unix` as it is,
re-spelled. Text: the line's one instant - the `mtime` capture under
`parse_mtime`, else the handle's `mtime`, the epoch where the line has none; a
capture named `transunix` is refused as `currunix` is (`DERIVED_EVENT_COLUMNS`).

**`sendunix`** (`Option<i64>`, the same count): the technical clock - when the
message crossed the wire as the nearest clock saw it: FIX the carrier's instant
where a carrier exists (the text line's `transunix`, the capture's write
time), else the sender's `SendingTime(52)` where stated, else none - today's
`recdunix` precedence kept, because `SendingTime(52)` is already a column of
the fixed row and the capture's clock has no other; text a row-header capture
named `sendunix` alone (`EVENT_CAPTURES`), never the `mtime`. Outside identity
and every digest, as `recdunix` was. The merge reference keeps its shape over
the name: the statement sent last is the reference (`right_is_reference` - a
stated clock leads an unstated one, equal or absent clocks fall back to the
later `transunix`, exact ties keep `left`), two statements of one event keep
the earliest `sendunix` (`fold_event_instants`, the trade's `canonical_data`,
the book's `fold_bounds`), the book's `reference_clock` and the trade's
`reference_key` read `(sendunix, transunix[, uuid])`. The alternative
reading - `sendunix` the sender's `SendingTime(52)` first - is one precedence
line and its tests; it was not taken, and the handoff names it for the user.

**The element's own names.** `uuid` (`ElementColumn::Uuid`, display `UUID`,
`get_uuid`/`set_uuid`, `Event::time_uuid` unchanged as the derivation) and
`hashcode` (`ElementColumn::HashCode`, display `Hash Code`,
`get_hashcode`/`set_hashcode`) are the first two of the six element columns,
their positions, datatypes and derivations unchanged: `hashcode` is still the
XXH3-64 of what the element states and `uuid` the UUIDv7 over `transunix`,
`seqnum`, `hashcode` and `crosshashcode`. A text row's six element columns
open with `uuid`; the `marketdata` row, the FIX row, the BookService keys and
the pages spell them.

**Wire.** Crate fields 65_001 `uuid`, 65_004 `hashcode`, 65_007 `transunix`
and 65_009 `sendunix`: the tags kept, the names and descriptions re-spelled
(`UUID_TAG_NAME`, `HASHCODE_TAG_NAME`, `TRANSUNIX_TAG_NAME`,
`SENDUNIX_TAG_NAME`; every crate-field text naming one of the four re-spelled
with them), so the crate's field shard and the fixed-row component are written
again (the dump, `YGGDRYL_FIX_DUMP_WRITE=1`), the dictionary hash moves once
with the sentence "It last moved when `curruuid`, `currhashcode`, `currunix`
and `recdunix` became `uuid`, `hashcode`, `transunix` and `sendunix`: 65_001,
65_004, 65_007 and 65_009 re-spelled with their descriptions, so the crate's
field shard and the fixed row component were written again. No count of the
census below moved.", `docs/assets/fix.json` is
regenerated after the addon, and `rust/tests/fix/equivalence.snapshot` is
written again (`YGGDRYL_FIX_EQUIVALENCE_WRITE=1`): its keyed lines
re-spelled (`field.uuid`, `field.hashcode`, `field.transunix`,
`field.sendunix`), every value and every digest line unchanged - the digest
feeds no instant and no label. `EventColumn::TransUnix` and `SendUnix` keep positions 1 and 3 of
nine (display `Transaction Time`, `Sending Time`; descriptions "When the
operation happened: the settled transaction instant, UTC." and "When the
message crossed the wire, where that is known; the earliest its statements
know."). The `marketdata` row, the FIX row, the text row, the `BookService`
JSON and CSV keys and `node/book/audit.js` spell the new names. The one open
pin: `rust/src/fix/msg.rs:3201` pins a content code whose doc says a column
relabel moved every content code; the slice reads the content-code feed and
states whether the four labels enter it - if they do, that pin and every
`uuid` move once with the sentence; if not, both hold.

**Bindings and pages.** Python and Node getters, setters, the constructor's
first positional (`transunix`), `**facts` keys, error texts ("states
`transunix` once, as its first argument"), the `.pyi` stubs,
`.api-bindings.txt`, `.api-inventory.txt`; Node re-spelled only, no door
added. Every page and skill, the schema tables (tag literals unchanged),
`docs/fix/capture.md`'s dating rule, `docs/graph/serve.md`'s CSV header;
AGENTS.md: the text row contract sentence, the `graph/` (`element.rs`,
`column.rs`), `fix/` and `isin_registry.rs` rows.

**Pins.** `MarketData` at 912, every `allocations` and `iobase_calls` row and
the census unmoved; the hash, the dump, the snapshot and `fix.json` move once
each as above.

**The sweep's anchors beyond the bare names**: `get_/set_currunix`,
`get_/set_recdunix`, `get_/set_curruuid`, `get_/set_currhashcode`,
`walked_currunix`, `EventColumn::CurrUnix`/`RecdUnix`,
`ElementColumn::CurrUuid`/`CurrHashCode`, the `*_TAG_NAME` constants,
upper-case `CURRUNIX`/`CURRUUID`/`CURRHASHCODE` keys in binding tests,
`fix_event_recdunix`/`graph_order_event_recdunix`/`eventCurrunix` typing
names, test names, the Iceberg declarations in tests (`time_bucket('15
minutes', currunix)`, the sort and key lists); `refrecdunix` - the retired
FIX field 65_064, a name of its own - is excluded by the regex and not
renamed.

Slice: P4, in place, after S3 lands and before P5 (D37); one commit.
