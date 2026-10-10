---
name: yggdryl-fix
description: Decodes, encodes and streams FIX messages with yggdryl against a FIX dictionary (FixRegistry), in Rust, Python and Node.js. Use when parsing tag=value/SOH frames, bridge rows or FIXML from logs (parse_line / parseLine, parse_text_arrow_reader / parseTextArrowReader), re-emitting wire bytes (into_text / intoText), landing captures in Arrow or Parquet (fix_schema, arrow_reader / arrowReader), chaining order lifecycles (lifecycle, crossuuid), turning FIX into market data and books (market_data, book_arrow_reader), building or storing a dictionary (FixRegistry.from_handle / fromHandle, from_cfb_file, merge_with, commit, code sets, FIX metadata) or running the yggdryl fix CLI.
---

# yggdryl FIX

A FIX field is an ordinary `Field` whose `FIX:` metadata (`FIX:tag`, `FIX:names`,
`FIX:codeset`, ...) the `fix` protocol view reads - in Rust `FixField::new(&field)`
and `FixFieldMut::new(&mut field)`, the FIX crate's own view, `Field` having no
`as_fix`; there is no second field class. `FixRegistry` is the dictionary: **one namespace** holding scalar fields,
components (a message is a component carrying `FIX:msgtype`), groups, and the
named code sets beside them. `FixCodec` is one dictionary plus the pins of a
run; its `parse_*` readers turn captured bytes into `FixMsg` values - a market
event (identity, clocks, state, side, price...) over a content row typed by the
dictionary. Every message projects onto one fixed row, `fix_schema(registry)`,
decided from the dictionary alone, so a whole capture streams as Arrow batches.
Each message states its `marketdatakind` - the `MarketDataKind` its type files under
(`ORDR`, `QUOT`, `EXEC`, `TRAD`, `BOOK`, the batches `ORDB`, `QUOB`, `TRDB`,
...) - and the parse splits what it reports once: an execution report is its
order's report (`ORDR`, `QUOT` naming a `QuoteID`), a filling one adds its
`EXEC` message, a trade one execution per side, a batch one message per
entry. A quote is one message holding both its legs. A lifecycle chains
within one `marketdatakind`, so a fill never follows its order.

In Rust FIX is the `yggdryl-fix` crate (`yggdryl_fix::FixRegistry`,
`yggdryl_fix::FixCodec`) over `yggdryl-market`: call `yggdryl_fix::install()?`
once - it installs the market crate first - before anything reads a FIX Latest
datatype name (`UTCTimestamp`, `Qty`) or a market kind. Python and Node.js
install both on import.

Hold two speeds apart. **Decoding is per message**: each frame is parsed on its
own, in parallel (`threads`), answers in input order, and never reads another
message - it only takes the next place (`seqnum`) of its instant in stream
order, so a report and the execution split off it are places 0 and 1.
**Lifecycle is the only cross-message stage**: `lifecycle` collects a finite
capture, sorts it by event time, folds duplicate deliveries, places each
message by content among the messages of its instant (a content repeated
there keeps its place), chains it to the live one of its order within its own `marketdatakind`, side and instrument - by one identifier of the same type and value, every type of its `identifiers` but a shared one (`trdmatchid`, `quotereqid`, `mdreqid`, a parent slot), and by the first value a lineage field names, `OrigClOrdID(41)`, `OrigTradeID(1126)`, `TradeReportRefID(572)` - (`crossuuid`,
`prevuuid`; an order and an execution under one cross code are two chains), re-keys it onto its chain's side and first cross code, takes every bridge `metadata` key of the chain it does not state
and the ids its dictionary follows, each with its parents, states a message citing two live chains as a conflict - a `FixAnomaly` under `crosscode`, warned once per kind - rather than picking one, and learns instrument associations.
Nothing chains unasked.

The dictionary is data, not code: the committed FIX Latest dictionary
(fields, 181 messages, 739 code sets, every tag FIX 4.0 to 5.0 SP2 declared) is
the `config/fix` folder of the yggdryl repository, generated and committed,
~14 MB, **not shipped** in the crate, wheel or npm package. Load it by path, or
point `YGGDRYL_FIX_REGISTRY` (or `~/.config/fix`) at it for the process default.

## Choose the door

| Task | Rust | Python | JavaScript |
| --- | --- | --- | --- |
| load a stored dictionary | `FixRegistry::from_handle(&LocalFolder::new(path)?)?` | `FixRegistry.from_handle(path)` | `fix.FixRegistry.fromHandle(path)` |
| the process default | `FixRegistry::from_env()?`, `FixRegistry::install_env(r)?` | `FixRegistry.from_env()`, `FixRegistry.install_env(r)` | `fix.FixRegistry.fromEnv()`, `fix.FixRegistry.installEnv(r)` |
| a dictionary in memory | `FixRegistry::from_fields([..])?`, `registry.insert(field)?` | `FixRegistry.from_fields([...])`, `registry.insert(field)` | `fix.FixRegistry.fromFields([...])`, `registry.insert(field)` |
| FIX facts on a field | `FixField::new(&field).tag()?`, `FixFieldMut::new(&mut field).set_tag(38)?` | `field.fix.tag`, `field.fix.tag = 38` | `field.fix.tag`, `field.fix.tag = 38` |
| look a field up | `field(55)`, `field_by_name`, `field_by_path(&FieldPath)`, `field_by_counter(453)`, `field_by_id(FixId)` | `field(55)`, `field_by_name`, `field_by_path("Parties.PartyID")`, `field_by_counter`, `field_by_id(int)` | `field(55)`, `fieldByName`, `fieldByPath`, `fieldByCounter`, `fieldById` |
| a message definition | `registry.msgtype("D")?` | `registry.msgtype("D")` | `registry.msgtype('D')` |
| a field's code set | `registry.codeset_of(field)`, `set_codeset(name, &[FixCode])?` | `codeset_of(field)`, `set_codeset(name, [{...}])` | `codesetOf(field)`, `setCodeset(name, [...])` |
| persist a dictionary | `registry.commit(&mut folder)?` | `registry.commit(path)` | `registry.commit(path)` |
| read a venue CBlock (`.cfb`) | `FixRegistry::from_cfb_file(&LocalFile::new(path)?, Some("venue"))?` | `FixRegistry.from_cfb_file(path, "venue")` | `fix.FixRegistry.fromCfbFile(path, 'venue')` |
| the sources a dictionary was built from | `FixField::new(&field).sources()`, `registry.sources()`, `get_source("venue")`, `add_source(FixSource::new("venue")?)` | `field.fix.sources`, `registry.sources()`, `get_source("venue")`, `add_source("venue", file=..., pluginside=...)` | `field.fix.sources`, `registry.sources()`, `addSource('venue', { file, pluginside })` |
| fold CBlocks or another dictionary in | `registry.add_cfb_file(&file, None)?`, `registry.add_cfb_files(&[Holder::local("cblocks")?], None)?` (a file, a folder or a glob each), `merge_with(&other)?` - each answering a `FixMerge` | `registry.add_cfb_file(path)`, `add_cfb_files(folder)` or `add_cfb_files(folder / "*.cfb")` - answering a `dict` | not bound (`yggdryl fix ingest`) |
| a codec for a run | `FixCodec::new(Arc::new(registry)).with_threads(4)` | `FixCodec(registry, threads=4)` | `new fix.FixCodec(registry, { threads: 4 })` |
| read under one source, stamping its plugin's role | `.with_source("venue")?`, then `msg.msgpluginside()` | `FixCodec(r, source="venue")`, then `msg.msgpluginside` | `{ source: 'venue' }`, then `msg.capture().msgpluginside` |
| read only some types | `.with_include_msgtypes(["D", "8"])` | `FixCodec(r, include_msgtypes=[...])` | `{ includeMsgtypes: [...] }` |
| sniff a line's type, no dictionary | `FixCodec::infer_msgtype_bytes(bytes)` | `FixCodec.infer_msgtype_bytes(bytes)` | `fix.FixCodec.inferMsgtypeBytes(buffer)` |
| decode one captured line | `codec.parse_line(bytes)?` (iterator) | `codec.parse_line(bytes)` | `codec.parseLine(buffer)` |
| decode exactly one frame | `codec.parse_fix_line(bytes)?` | `codec.parse_fix_line(bytes)` | `codec.parseFixLine(buffer)` |
| decode many lines lazily | `codec.parse_lines(lines)` | `codec.parse_lines(lines)` | `codec.parseLines(lines)` |
| decode text-reader lines | `codec.parse_text_lines(lines)` | `codec.parse_text_lines(lines)` | `codec.parseTextLines(lines)` |
| a capture's batches to FIX rows | `codec.parse_text_arrow_reader(reader)?` | `codec.parse_text_arrow_reader(reader)` | `codec.parseTextArrowReader(reader)` |
| read a fact | `msg.by_tag(55)?`, `by_name`, `by_path`, `header()`, `get_side()` | `msg.by_tag(55)`, `by_path(...)`, `header()`, `msg.side` | `msg.byTag(55)`, `byPath(...)`, `header()`, `msg.side` |
| the category, the strike | `msg.marketdatakind()`, `msg.get_strikepx()` (`Market`) | `msg.marketdatakind`, `msg.strikepx` | `msg.marketdatakind`, `msg.strikepx` |
| the type, how long it stands | `msg.get_marketdatatype()`, `msg.get_timeinforce()` (`Operation`) | `msg.marketdatatype`, `msg.timeinforce` (the `IntEnum` members) | `msg.marketdatatype`, `msg.timeinforce` (the member names) |
| the parents of an identifier | `registry.parents_of(&IdType::ClOrdId)`, `parent_of(&kind)`, `parent_sources()`; `FixFieldMut::new(&mut field).set_parents(..)?` | `registry.parents_of("clordid")`, `parent_of("origclordid")`, `field.fix.parents` | `registry.parentsOf('clordid')`, `parentOf('origclordid')`, `field.fix.parents` |
| a venue's own values onto members | `FixFieldMut::new(&mut field).set_marketdatatypes(..)?`, `set_timeinforces(&[("D", TimeInForce::Day)])?`; `registry.marketdatatype_of(tag, wire)`, `timeinforce_of(tag, wire)` | `field.fix.marketdatatypes`, `field.fix.timeinforces = [("D", "DAY")]`; `registry.marketdatatype_of`, `timeinforce_of` | `field.fix.marketdatatypes`, `field.fix.timeinforces = [{ wire: 'D', timeinforce: 'DAY' }]`; `registry.marketdatatypeOf`, `timeinforceOf` |
| a message as one market data value | `MarketData::from(msg)` (held whole, kind `fix`), `msg.into_market_leaf()?` (the one leaf it is) | `graph.MarketData(msg)` | `new graph.MarketData(msg)` |
| compose a message | `FixMsg::with_registry(Arc, root, value)?` | `FixMsg(root, value, registry)` | `new fix.FixMsg(root, value, registry)` |
| write or clear a fact | `msg.set(key, scalar)?`, `msg.remove(key)?` | `msg.set(key, value)`, `msg.remove(key)` | `msg.set(key, value)`, `msg.remove(key)` |
| encode to the wire | `msg.into_text('\x01')?`, `msg.into_bytes(SOH)` | `msg.into_text()`, `msg.into_bytes()` | `msg.intoText()`, `msg.intoBytes()` |
| the fixed row schema | `fix_schema(&registry, "fix")?` | `fix_schema(registry, "fix")` | `fix.schema(registry, 'fix')` |
| a message as one row | `msg.into_row(&schema)?`, `FixMsg::from_row(arc, &schema, &row)?` | `msg.into_row(schema)`, `FixMsg.from_row(schema, row, registry)` | `msg.intoRow(schema)`, `fix.FixMsg.fromRow(schema, row, registry)` |
| messages to batches | `codec.arrow_reader(schema, messages)?` | `codec.arrow_reader(schema, messages)` | `codec.arrowReader(schema, messages)` |
| batches back to messages | `codec.messages(reader)` | `codec.messages(reader)` | `codec.messages(reader)` |
| batches back to the wire | `codec.write_arrow_reader(reader, &mut sink)?` | `codec.write_arrow_reader(reader, sink)` | `codec.writeArrowReader(reader, { write })` |
| chain order lifecycles | `codec.lifecycle(messages)` | `codec.lifecycle(messages)` | `codec.lifecycle(messages)` |
| chain rows already in Arrow | `codec.lifecycle_arrow_reader(reader)?` | `codec.lifecycle_arrow_reader(reader)` | `codec.lifecycleArrowReader(reader)` |
| share what lifecycles learn about instruments, and what parses fill from | `codec.with_instruments(Arc::new(Mutex::new(Instruments::from_url(&url, props)?)))`, `FixCodec::from_env()?` for the process's own, `instruments.lock()?.commit()?` to write it back | `FixCodec(registry, instruments=Instruments.from_url(path))`, `FixCodec.from_env()`, `instruments.commit()` | `new fix.FixCodec(registry, { instruments: Instruments.fromUrl(path) })`, `fix.FixCodec.fromEnv()`, `instruments.commit()` |
| the common instruments, before any store | `Instruments::seeded()` - what `from_env` lays its store over; `Instruments::seeded_from_url(&url, props)?` lays a store you name over it, its rows winning | `Instruments.seeded()`, `Instruments.seeded_from_url(path)` | `Instruments.seeded()`, `Instruments.seededFromUrl(path)` |
| what a lifecycle learned about an instrument | `instruments.get(key)` - by its cross code, an alias or an ISIN, real or minted - then `instrument.listings()` (one per market, MIC order), `listing(Some(&mic))`, `firstunix()` and `lastunix()` - the earliest and the latest message instants that met it - beside `updunix()`, the last moved fact, `origccy()`, `underlying()`, `legs()`, `characteristics()`, `metadata()` | `get(key)` a `dict`, `listings(key)`, `get_listing(key, "XSWX")`, `row["firstunix"]`, `row["metadata"]` | `get(key)` a plain object, `listings(key)`, `getListing(key, 'XSWX')`, `row.firstunix`, `row.metadata` |
| the instrument a row is about | `msg.get_instcode()` - the instrument's cross code, crate tag `65054`, the instruments table's key: join on `instcode = crosscode`, or `instcode` among `aliascodes` for a row filled before a re-key | `msg.instcode` | `msg.instcode` |
| the number minted for an instrument no agency numbers | `Instrument::minted_number(code)` (`QY` + nine base-36 digits + check digit; `IF:EUR/USD` is `QYLTVIRYHNX5`) | `Instruments.mint(code)` | the instrument's `isin` |
| which instrument a message means | `instruments.resolve(&msg)` (ISIN, then the cross code its facts spell, then a minted number, then `LOOKUP_CODES`, then ticker on its market, then - scored, a fill taking it only under `set_economic_match(true)` - its `FinancialInstrumentShortName(2737)` in its currency) | `instruments.resolve(msg)` -> `Resolution` | `instruments.resolve(msg)` -> a plain object |
| one message as graph leaves | `msg.market_data()?`, `msg.into_market_data()?` | `msg.market_data()` | `msg.marketData()` |
| sorted market data | `codec.market_data(messages)` | `codec.market_data(messages)` | `codec.marketData(messages)` |
| books as `marketdata` rows | `codec.book_arrow_reader(msgs, 0, None)?`, `Some(&filter)` to narrow | `codec.book_arrow_reader(msgs, snapshot_millis=0, filter=None)` | `codec.bookArrowReader(msgs, 0, filter)` |
| sorted market data as `marketdata` rows | `codec.market_arrow_reader(msgs)?` | `codec.market_arrow_reader(messages)` | `codec.marketArrowReader(messages)` |
| FIX rows in Arrow to `marketdata` rows | `codec.market_data_arrow_reader(reader)?` | `codec.market_data_arrow_reader(reader)` | `codec.marketDataArrowReader(reader)` |
| manage a dictionary from a shell | `yggdryl fix --root <dir> ...` ([cli](references/cli.md)) | same binary, shipped in the wheel | same binary |

## Rules for fast, correct use

1. Load the dictionary once and share it. The full seed takes about a second
   to load and resolve (release Rust; two from Python): build one
   `FixRegistry`, wrap it in one `Arc` (Rust) or pass the same object
   (bindings) to every codec, or install it as the process default before
   anything resolves the default.
2. Pin a run on the codec, never per call: `threads`, `batch_byte_size`
   (128 MiB target) / `batch_row_size` (32,768), `include_msgtypes`,
   `payload_column`, `default_sending_time`, `snapshot_ns`,
   `sorted_lifecycle`, `dedup_window_ms`. One codec reads a
   line and a batch alike; there is no second options struct.
3. For bulk, stay in Arrow: `parse_text_arrow_reader` over the text reader's
   batches pools parsing across workers and merges rows in source order under
   the byte and row targets, so memory stays bounded. Materializing one
   `FixMsg`/`Scalar` per message and rebuilding rows yourself is the slow path.
4. Filter at the codec. `include_msgtypes` / `exclude_msgtypes` are read off
   the `35=` a row states before anything is built; a refused keepalive costs
   one look. By default `Heartbeat`, `TestRequest` and the untyped row are
   refused (`DEFAULT_REFUSED_MSGTYPES`); pass an empty exclude list to audit.
5. Decode is embarrassingly parallel; lifecycle is not. `lifecycle`,
   `lifecycle_arrow_reader` and `market_data` collect the whole finite
   capture (they sort by event time), so feed them one session or day, not an
   unbounded stream. A source already in instant order - a table read hour
   partition by hour partition, sorted by `transunix` - pins
   `sorted_lifecycle` (`with_sorted_lifecycle(true)`, `sortedLifecycle`):
   `lifecycle` then holds one epoch hour at a time, walking an hour once a
   message two hours past it is read, and answers the same walk. Parse and
   project in the parallel doors; chain once. The walk yields each
   `uuid` once within `dedup_window_ms` of event time (one minute by
   default; `None`/`null`/`0` yields every restated twin too), so a
   consumer keyed by `uuid` needs no dedup of its own. Join a chain on
   `crossuuid`: every message of one chain carries the chain's first
   `crosscode` - a replace under a new `ClOrdID` keeps it - so never re-key by
   an identifier yourself. A message citing two live chains is joined to
   neither: it stands under its own identity and its `anomalies` hold a
   `crosscode` entry naming both, so read them before trusting a split.
6. Market hand-off is `market_data(lifecycle(messages))`: the walk settles
   each message, the sorted door orders every leaf by the instant a book folds
   it. One message is one leaf - an order, a quote holding both its legs, an
   execution - and a `W`/`X` message one per entry; a trade and a batch reach
   the book as the messages their parse split off, never twice. An
   `ExecutionReport` (`8`) of no fill is its order's leaf (its quote's, naming
   a `QuoteID(117)`), so a venue's cancel, reject or expiry moves the book; an
   `ExecutionAcknowledgement` (`BN`) and a `DontKnowTrade` (`Q`) answer no
   leaf - they state no fact of the order. `book_arrow_reader` folds orders,
   quotes and `W`/`X` entries, pruning executions and trades before a book
   sees them, one book per instrument cross code (`instcode`) - a message
   stating none, a ticker-only line no lifecycle filled, pruned before it is
   expanded - and its `filter` narrows what folds, never admitting
   an execution back. It does not sort - an operation dated before its book is
   left out with a warning - and neither door runs the lifecycle for you. For a capture already landed as FIX rows,
   `market_data_arrow_reader` reads each row as its message (no line parsed
   again) and sorts its leaves; it runs no lifecycle either, and a
   `lifecycle_arrow_reader` row lacks what the walk settled (`prevpx`, a
   carried side), so hand walked messages, not rows.
7. Pin `default_sending_time` for reproducible reads. A frame stating no
   `SendingTime(52)` whose line carries no clock is dated by one UTC-now read,
   and the clock feeds `uuid`, as does a `SendingTime(52)` naming no
   instant; a pinned instant (or a row-header `mtime` capture) makes two reads
   identical.
8. A column is found by name, never position: `schema.index_of("msgtype")`, or
   `fix_column_of(&schema, 35)` in Rust. Columns are the dictionary's folded
   names; the tag stays on each column's `FIX:tag`. Two captures under one
   dictionary share one schema exactly.
9. A capture's own columns (`url`, `rownum`, `loglevel`, a `thread` of your own...) follow the
   element, event, market and operation columns every row opens with; a column named after a FIX field fills that field where the frame
   stated none; `beginstring` and `msgdirection` columns are per-row
   parameters. An `mtime` capture dates the line - its messages' `sendunix`
   and the sending clock of any stating no `SendingTime(52)` - read under the
   text options' `timezone`; a `timestamp` capture of your own dates nothing.
   `ULBRIDGE_ROWHEADER` captures `mtime`, so it dates every line it matches
   (set `timezone` to the bridge's local zone), and `msgthreadid` and
   `loglevel`, the capture's own columns: on the text row, carried in front
   of a FIX row, filling no field. Hand the codec `options.capture_names`
   rather than a spelled list: a line answers its captures by position.
10. Store and reload a dictionary through `commit`/`from_handle` (or `yggdryl fix`).
    `commit` writes only documents that changed, prunes what no definition
    holds, and answers `written`/`removed`; never hand-edit the generated
    shards, and state a code set before the field that names it. A fold
    (`add_cfb_file`, `add_cfb_files`, `merge_with`) keeps every declaration the
    dictionary already holds, stamps each source's id in `FIX:sources` (a JSON
    array of lowercase ids) and records what is known of the source once, in
    the registry's sources catalog (`sources.json` in a store): the file and
    the `Side` its CBlock root's `type` names, read by `yggdryl_fix::plugin_side`. A datatype a source states at another precision
    of the stored one - unbounded text against anything, any two numbers, an
    integer against an enum, a date against a datetime (a CBlock's `float`
    against `decimal128`, `string` against `ccy`) - folds under it and is
    counted in `restated`; a contradiction (`boolean` against `int32`, a
    time of day against an instant), a member a held definition declares in
    another shape, a code set that does not fold and a definition whose fold
    refuses are passed over, each named in `dropped` rather than refusing the
    whole source. The one datatype a fold changes is a group's counter: held
    as unbounded text, a float or another integer width, it is retyped
    `int32` with a warning; a counter on the alternate tag of a field that is
    no count is that field's value on the wire, so its group is passed over
    instead. A group
    on another counter than the held group of its name stands beside it as
    `{name}_{counter}`, read by one member per counter. A field named by nothing
    but its tag (a CBlock tag no `alt` or binding names) is unnamed: one on a
    tag a held field answers, as its own or as an alternate, folds into that
    field, and the first name to arrive on its tag names it. A field on a held
    tag under another name stands beside the holder, and neither learns the
    other's name, unless that name is a third field's canonical name, which
    it merges into, the tag staying with its holder. A message member is the
    field it reads before the name it carries. A component or group stating
    a held definition's structure - its members in order, each tag and the
    field it reads - is that definition: the held name wins, each member's
    nullability relaxes and the sources union; two definitions the
    dictionary held as they were stay two. `add_cfb_file` and
    `merge_with` refuse whole only where nothing is left to keep (malformed
    XML or JSON, a source whose own catalog does not validate); `add_cfb_files` folds each file as one mutation, so
    such a file is left out alone and named in `failed` while the rest fold,
    and `is_clean()` means `dropped` and `failed` are both empty. What a
    CBlock states that the reader cannot keep is dropped, or kept another way
    (an unread type word types the tag string), with a `log` warning naming
    the line, the column, the element and what the reader did instead; a
    counter a grammar states beside the group it counts is left out
    silently, the group's length being its count, in the file and in every
    fold; a map entry whose key or value is blank, `none` or `null` states no
    code and is skipped, named once per code set. `add_cfb_files` folds in ascending URL
    order, so where two files type one tag two ways the first-sorting file's
    declaration is held, and a code set only widens (the held name wins a
    shared value; a new value under a name another code claims keeps no
    name).
11. Mutations are atomic: a refused `insert`, `set` or `set_codeset` leaves the
    registry or message unchanged. A registry shared by a codec or message is
    frozen in the bindings; mutate first, then build codecs.
12. Row door and wire agree: `into_row` keeps typed facts in their columns and
    only what no column holds in `fixentries`, a sorted `map<utf8, utf8>`
    keyed `tag:name` (`58:text`) - a scalar as its wire text, a group or a
    component as JSON keyed the same way, a repeated key as the JSON array of
    its occurrences. A key the dictionary does not resolve is no field: it
    lands in the row's `metadata` under its own spelling, and a row read back
    restores it, so the wire re-emits it. `from_row` refuses a row that
    disagrees with itself - a `fixentries` key naming another field than its
    tag (`55:securityid`), an identifier map holding a key that reads as none
    or two spellings of one key with two values - and `messages` leaves such a
    row out with a
    warning. `write_arrow_reader` rebuilds each
    message from the row and refuses a batch with no `fixentries`. A group is
    one entry filed under its counter's tag (`453:parties`), valued its
    length: the count is no field of its own and the wire re-emits `453=N`
    from the list. A list holding nothing is the group stated empty (`802=0`
    re-emits) and a null list is the group absent; a table column holds a
    group as null or as at least one occurrence, so a stated zero rides the
    residual `fixentries` record, and `[]` read back where the row held null -
    what a table such as PyIceberg does - states nothing, so a row read back
    keeps its `hashcode` and folds with the delivery it was. A miscount
    (`453=2`, one occurrence) is no anomaly: the group holds what arrived and
    re-emits `453=1`.
13. The derived fills and the retired-field restatements are native code: a
    registry carries no rule of its own, and nothing in `FIX:` metadata
    changes how a field is filled.
14. Data is never an error; only a source failure is. A parse, the market
    projection, the lifecycle and the book walk default what a message states
    that they cannot read - a value that will not type is null beside a
    `FixAnomaly`, a clock naming no instant is unstated - or leave the item
    out, each with a deduplicated warning: Rust `log` at `WARN` (the core's
    `yggdryl::logging` tree or any `log` backend), Python `logging` under
    `yggdryl.<module path>`, JavaScript standard error as the core's terminal
    line (`... ! WARNING  [main] yggdryl.fix.build build:<line> › FIX clock
    left unstated: ...`) unless a handler on `logging.getLogger('yggdryl')`
    takes them, and the CLI on standard output once its progress line is
    done. Only a reader, store or runtime that could not answer is an error
    item, yielded after the messages before it, and it ends the stream.

## Pitfalls

- An Iceberg table widens the two `uint64` digests to `decimal(20, 0)` unless
  the fixed row's `hashcode` and `crosshashcode` columns state
  `FIELD:representation=bits` before `into_scheme_compat`: then each is a
  `long` holding the digest's bits, written through the value door and read
  back by `messages` as the digest. A row's `int64` digest cell is read as
  its bits whatever its schema says; `seqnum` is a count and a negative one
  refuses the row.
- An alias stating another value than its field is **not** a second child: it
  is an anomaly kept in `msg.metadata` (never on the wire), and one stating
  the same value leaves nothing. Bridge spellings the dictionary names are
  FIX fields (`OMSDEALERACCOUNT` is `Account(1)`, `ULTRADERCLORDID`
  `ClOrdID(11)`, `EXCHANGECLIENTORDERID` `SecondaryClOrdID(526)`, `OMSUSERID`
  `Username(553)`, `PARENTCLORDID` `OrigClOrdID(41)`). `msg.set(tag, v)` also
  restates every sibling - an alias kept in the metadata, a composed
  `NAMESPACE.FIELD` key, another child on the tag - and `remove` clears them.
- `SecurityIDSource(22)`/`SecurityAltIDSource(456)` read every code
  (`1`-`9`, `A`-`N`, `P`-`Y`, case-sensitive) or name, its remarks passed
  over (`ISIN number`, `ISDA/FpML Product URL (URL in SecurityID)`), and so do
  the derivations (`22=isin` opens `CountryOfIssue(470)` as `22=4` does); a
  source no member names is kept as stated - a private `100` is the type
  `100`, `Z` is `z` - while `ticker`, an order's or a party's identifier
  (`ClOrdID`, `Exchange`) and a spelling no word holds (`House/Key`) are
  anomalies, left on the wire. A code is held to its type's shape and its
  validity is a rank, never a refusal: an ISIN whose check digit does not
  close, a masked `XX0000000001`, an `isoctry` ISO does not list or an
  `isoccy` of `XXX` is a value of a lower rank, which a real value - stated or
  derived, earlier or later - replaces wherever two meet; only another shape
  (an eleven-character ISIN) is an anomaly.
- A bare integer is always a **tag**: `registry.field(55)`, `msg.get(55)`. A
  field identity (`FixId`, the signed XXH32 of tag + folded name) is only
  reached through `field_by_id` / `get_by_id` (`FixKey::Id` in Rust).
- A captured line is not a message. `parse_line` answers an iterator - none
  for a sentence, two for two frames on one line; unpack it
  (`message, = codec.parse_line(b)` / `const [m] = codec.parseLine(buf)`).
  `parse_fix_line` refuses a body holding a second frame.
- An empty line, a frame that builds no message and a clock naming no instant
  are no error items: `parse_lines` leaves the line out - or reads the message
  with its clock unstated - beside a warning, and the stream reads on. Only
  `parse_line(b"")`, the one-line door, still refuses no bytes at all.
- Names are folded (ASCII case, `_`, `-`, space dropped): `MsgType`,
  `msg_type`, `MSG-TYPE` are one name; `Größe` and `GRÖSSE` are two. Every
  name lookup also reads four word pairs either way inside the folded name -
  offer/ask, size/qty, bid/demand, px/price - so `AskPrice` is
  `OfferPx(133)` and `DemandQty` is `BidSize(134)`; an exact name wins, and a
  spelling reaching two fields reaches none.
- A coded value reads as its name (`by_tag(54)` -> `BUYS`; Python answers the
  `Side.BUYS` member) but emits as its wire code (`54=1`); set it with the wire
  code or any spelling the code set resolves.
- A status the dictionary types as an integer (`QuoteStatus(297)`) states the
  message's state as its code reads, whether the column holds an integer or
  text: a `QuoteStatusReport` (`AI`) stating `297=4` reads `CANCELED`, and
  through the lifecycle takes the quote off its book, while the `QuoteCancel`
  (`Z`) before it reads `PENDING_CANCEL` and keeps it there.
- A group member needs its index on a message: `Parties[0].PartyID`;
  `Parties.PartyID` is the schema spelling and misses on a value. A group is
  its list and its length is its count: `by_tag(453)` reaches nothing on a
  message (`get_by_tag(453)` is `None`/`null`). Read Rust
  `by_name("parties")?.as_sequence().map(<[Scalar]>::len)`, Python
  `len(by_name("parties").as_py())`, JavaScript `byName('parties').length`.
  A message root built by hand lists no counter beside its group, and a
  registry definition that does is refused - by hand; a CBlock grammar or a
  fold that does is read without the counter.
- `into_text` output reflects what the dictionary derived (a day order's
  `59=0`), minus facts supplied at intake (an unstated `SendingTime`); it is
  canonical wire, not a byte-for-byte copy of the input.
- A Python `str` in `parse_lines` is refused; pass `bytes`. JavaScript
  `parseLine` takes a `Buffer`; `parseLines` accepts strings or buffers.
- Without a dictionary (`FixRegistry()` / `new fix.FixRegistry()`) frames
  still parse and the header, the lifted numbers and identifiers (11, 37, 38,
  44...), the market facts (`side`, `price`...) and the crate's own columns are
  typed, but every other key is unmapped - it lands in `metadata` under its raw
  spelling: no code names, no groups, no `fixentries`.
- A message's `crosscode` is stored `{kind}:{side}:{base}` - `marketdatakind` code, side
  code, then the code as the message names it: an order `A1` buying is
  `10:1:A1`, an execution `8:1:E-1`, a quote `14:0:Q1` whatever side it
  states. Only an order or an execution states its side there; every other
  message stores side `0`. A quote stating a bid and an offer is one message
  of side `UKNW`, no `price` of its own, its `bidpx`/`bidqty` and
  `askpx`/`askqty` the two legs; one stating `Side(54)` tags the leg it
  quotes. A derived execution is chained under its `ExecID(17)` as given
  (`8:1:E-1`), else `TradeID=<TradeID(1003)>`. A trade capture (`TRAD`)
  reads `TradeID(1003)` and `TradeReportID(571)` before any order tag
  (`21:0:T1`). After `lifecycle` every message of a chain carries its first
  message's code, `crosshashcode` and `crossuuid` derived from it: read a
  message's own spelling off its fields. Count messages after the
  parse, not lines: one filling report is two messages.
- A message's identifiers are logical `Identifiers` maps keyed `src:type`, read
  off its fields, the wire kept as sent, each identifier `key=value` in
  lower-case words, a FIX field's under the base key spelled as its type alone
  (`isin=US0378331005`, `clordid=C1`): `securityids` (`SecurityID(48)` under its
  `SecurityIDSource(22)`'s type and each `SecAltIDGrp(454)` occurrence - a
  `{NAMESPACE}INSTRUMENTID` source an `instrumentid` from that namespace,
  `ULLINK.INSTRUMENTID` `ullink`, a reserved `base`, `derived` or `fix`
  namespace none, so the base `instrumentid`;
  `FinancialInstrumentShortName(2737)` under the base `fisn` key,
  upper-cased, a value of no FISN's shape an anomaly named
  `financialinstrumentshortname`; an ISIN's embedded codes and a
  symbol's FX pair from `derived`; `get(type)` answers the base key, which a
  named source fills where nothing states it), `identifiers` (each
  `FIX:idmap` field its type's base key; regulatory trade ids under
  `regtradeid`, `tvtic`, ...) and `partyids` - each `PartyID(448)` typed by its
  `PartyRole(452)` code's name folded (`executingtrader`, `21`
  `clearingorganization`; an unnamed code `partyrole{code}`, none `party`) from
  its `PartyIDSource(447)` code's name (`D` `proprietary`, `C`
  `generalidentifier`; an unnamed word itself, an unnamed bare code
  `partyidsource{code}`; none the base source), the first of a source and
  role standing, a named source filling the role's base key, and `Account(1)` an `account` from its `AcctIDSource(660)` by the
  same rule (`acctidsource{code}`). Two sources hold a party to a code's
  shape whatever its role: under `bic` (`447=B`, `660=1`) a BIC's, under
  `legalentityidentifier` (`447=N`) an LEI's, upper-cased; a party the rule
  refuses stays on the wire as an anomaly of `partyid`, `rootpartyid` or
  `account` and in no map. An entry no dictionary resolves - a
  bridge's `FIRM.X.PARENTORDERID=`, `OMS_InstrumentID=` - names the identifier
  it ends with and the source before it (`firm.x:parentorderid`,
  `oms:instrumentid`), a reserved `base`, `derived` or `fix` namespace naming
  none (`Derived_ISIN` is `isin`), and
  another instrument's word before a security type naming no identifier
  (`OMS_UnderlyingISIN`, `FIX.LegISIN`); one naming an operation's or a party's
  identifier whose value its type refuses stays on the wire, no anomaly. Such
  an entry is **captured**: one whose folded name ends with a security type's
  spelling (`ISINCODE`, `RICCODE`, `OMS_CUSIPCODE`, `SEDOLCODE`,
  `BLOOMBERGCODE`, `OMS_InstrumentID`) lands in `securityids`, an operation's
  or a party's identifier in `identifiers` or `partyids`; a captured entry
  leaves the fixed row's `metadata` cell and rides `fixentries` under
  `0:<key>` (`RICCODE=AAPL.O` is `0:riccode`), so the row holds every arrival
  once, and a row read back restores it on the wire. A value its type refuses
  by shape stays in `metadata`.
  A field states `FIX:parents`, the types holding the parents of its identifier
  nearest first (`ClOrdID(11)` has `["origclordid"]`); a follower and every
  settle fill the parent's own type from its nearest stated parent (`orderid` from
  `parentorderid`, else `origorderid`); a follower whose `orderid` changed keeps
  the previous value as `parentorderid` and the chain's first as `origorderid`,
  and one naming no `orderid` carries the chain's with both. Only a chain
  identity has parents: `ExecID(17)` and `TrdMatchID(880)` carry none, and
  `TradeReportID(571)`'s is `tradereportrefid` (`572`). A bridge's
  `PARENTCLORDID` is the word `parentclordid` - no parent, no chain - and its
  `PARENTORDERID` the previous-value slot `parentorderid`, which names no
  chain but fills a missing `orderid`. A caller's
  `insert_*`/`set_*` is the message's word and writes no field: to change the
  wire, write the field. `SecurityID(48)`, `SecurityIDSource(22)`,
  `Parties(453)` and `SecAltIDGrp(454)` are no columns of the fixed row (151
  columns): `fixentries` keeps them as sent (`453:parties`, the group's own name).
- A graph leaf (`market_data`) carries in its `metadata` what its message
  states that no typed column reads and none of the leaf's identifier maps
  holds: a party, the account and a regulatory id its maps hold are left out (a
  second party of one role and source stays, in `parties`). A scalar whose key ends with
  an identifier its message's type declares (`marketorderid`, `RefOrderID(1080)`,
  a bridge's `venue.x.parentorderid` on an execution report) is lifted into the
  leaf's `identifiers` as the identifier its key names (`market:orderid`,
  `venue.x:parentorderid`), so a leaf's `identifiers` can hold more than its message's.
  `with_market_metadata(false)` / `market_metadata=False` /
  `marketMetadata: false` turns both off and moves the leaf's identity.
- A `Symbol(55)` naming one currency pair - `EUR/USD`, `EURUSD`, `EUR-USD 1M`,
  a RIC's `EURUSD=` - states the derived security identifier `derived:forex=EUR/USD`
  (the `forexcode` column).
  A row's `isincode`, `figicode`, `bloombergcode` and `forexcode` are views:
  each the code `get` answered when the row was written, resolved once as
  the row is read: a view is its type's base key. The symbol's derivation
  reads back from `derived` (the pair alone follows a written symbol, the
  cells detection wrote reading back as the row's word). Without the
  `securityids` column a narrow row is lossy: a code any entry of its type in
  the reading (the wire's, a bridge key's) holds states nothing; any other
  replaces the type's base key, its named sources staying as evidence, and a
  view disagreeing with a held base key it may not replace is dropped with an
  anomaly naming the view column - whether a code was derived is lost (an
  instruments- or caller-derived code reads back stated). Write `securityids` for
  a round trip that keeps sources.
- Instrument learning is the lifecycle's, never the parse's: each walk learns
  every message's instrument into an `Instruments`, keyed by its cross code -
  a security by its real ISIN alone, an FX pair, a forward, a swap, an option,
  a future or a strategy by `class:body`, its CFI class and the
  characteristics its message spells (`OC:US0378331005:2026-12-18:200`,
  `FF:EU0009658145:2026-12`, `KE:<leg>+<leg>`), the underlying and the legs
  resolved to the codes the collection keys, a derivative stating only its
  venue's ISIN a placeholder re-keyed once its body arrives, the old code kept
  in `aliascodes`. It learns the CFI code, country of issue, the underlying
  (`UnderlyingSecurityID(309)`, a bridge's `UnderlyingISIN`; lifted into no
  `securityids`), a strategy's `NoLegs(555)`, a structured product's EUSIPA
  category (`eusipacode`, off a bridge key whose folded name ends `eusipa`,
  `eusipacode`, `eusipacategory`, `sspa`, `sspacode` or `sspacategory` -
  `EUSIPACode=2300`, `OMS_SSPACategory=1260` - four digits once trimmed, two
  different ones stating none; lifted into no map, the entry kept as it came),
  the descriptive fields (`Issuer(106)`, `SecurityDesc(107)`,
  `SecurityType(167)` and eight more) as its `metadata`, market, ticker,
  currency, pair and security codes - each source's under its own `src:type`
  key - a valid stated value filling and replacing whatever the time, and
  fills what later messages of that instrument leave unsaid - `derived`
  identifiers, the ticker, the CFI code and the currency as market facts, the
  instrument's cross code as `instcode` - never the wire, `CFICode(461)` or
  the message's identity. The parse writes `instcode` only where the message
  alone spells it - a stated real ISIN, a detected FX pair (`IF:EUR/USD`, its
  book `3:0:IF:EUR/USD`, its minted number `QYLTVIRYHNX5` derived under
  `derived:isin` a fact beside it) - and a parse through a codec sharing a collection fills
  derived identifiers from the table its door fixed as it opened, nothing
  else, and learns nothing. Without `instruments=` each walk learns into its
  own, starting empty, and a parse fills nothing; pass one collection - bound
  to a store with `from_url` and written back with `commit()` only where it
  moved, or the process's own `from_env()`, which `FixCodec.from_env()`
  attaches - to share it across walks run one after another. A code of
  `Instruments::LOOKUP_CODES` - a CUSIP, a SEDOL, a RIC, a Bloomberg symbol -
  leads back to its instrument like a ticker on its market, one instrument per
  code; a parse fills from those exact keys alone, never from a short name's
  score. A message's `origccy` (crate tag `65018`, no FIX field) is learned as
  the instrument's issue currency and filled where a later message states
  none; `origin_currency` reads the currency otherwise. `from_env()` starts
  from the embedded seed of common instruments (`Instruments.seeded()`, the
  store's rows winning), and an instrument the collection folds carries the
  CUSIP, SEDOL, WKN or Valor its ISIN embeds and each listing its market's
  country's currency where it states none - defaults a statement replaces. In
  a medallion pipeline commit the codec's collection once, as the stage right
  after the FIX-message parse, after the refined write has drained the
  lifecycle: one snapshot of one row per instrument, nothing where clean; a
  table laid out before the instrument row is refused by name, so drop and
  recreate it. The capture's own lines are appended by key (`append_serie` on
  a table whose `identifier-field-ids` is the row's key: a line already stored
  in its partition is skipped, no file rewritten); every derived table is
  overwritten partition by partition.
- `StrikePrice(202)` is read into the market fact `strikepx` (`Market`), the
  fixed row's `strikepx` column right after `ticker` (crate tag `65036`), so
  a graph leaf carries no `strikeprice` metadata key; `msg.set(202, v)`
  restates it, a row stating `strikepx` is the row's word, and an unreadable
  202 is an anomaly named `strikeprice`. A follower of the same instrument
  carries the strike along its chain.
- `DETAILEDCFICODE`, the bridge's detailed classification, is a name of
  `CFICode(461)`: one message stating both folds them into the one 461 value
  through `Cfi::refined` - the leading code's letters kept, its `X` positions
  filled from the other - and two codes that contradict keep the 461 value,
  the other staying in `metadata` beside an anomaly.
- A row header that stops matching silently changes lifecycle results: the
  line keeps its body but is dated by its file's modification time and
  carries no session context (no delivery folding); assert the matched-line
  count beside the parsed-message count. The `prevunix` a text read states on
  each line (the instant the line before it was dated by) is ignored by the FIX
  text doors: a message's own comes from the lifecycle.

## Language references

- Rust: [references/rust.md](references/rust.md) - `yggdryl_fix::{FixCodec, FixRegistry, FixMsg, fix_schema}`, `Arc`, iterators of `Result`.
- Python: [references/python.md](references/python.md) - `yggdryl.fix`, pyarrow readers in and out.
- JavaScript: [references/javascript.md](references/javascript.md) - the `fix` namespace, `Buffer` input, `BatchReader`.
- CLI: [references/cli.md](references/cli.md) - `yggdryl fix` catalog commands.

Read the one for the language you write; recipes appear in the same order in each.

## Deeper

- FIX overview and `FIX:` vocabulary: https://platob.github.io/yggdryl/fix/
- Decode rules (none, one or many per line; type filter): https://platob.github.io/yggdryl/fix/decode/
- Encode order and separators: https://platob.github.io/yggdryl/fix/encode/
- Registry, one namespace, code sets, process default: https://platob.github.io/yggdryl/fix/registry/
- Store layout and snapshots: https://platob.github.io/yggdryl/fix/store/
- `FixMsg` holders, accessors, writes: https://platob.github.io/yggdryl/fix/message/
- Arrow doors, pins, market books: https://platob.github.io/yggdryl/fix/arrow/
- Capture, fixed row, clocks, warnings: https://platob.github.io/yggdryl/fix/capture/
- Lifecycle walk: https://platob.github.io/yggdryl/fix/lifecycle/
- CLI: https://platob.github.io/yggdryl/fix/cli/
- Sibling skills: `yggdryl-market-data` (the market data and books FIX turns
  into), `yggdryl-records` (text reader, row headers, Parquet/IPC sinks),
  `yggdryl-arrow` (`Serie`, `BatchReader`), `yggdryl-types` (`Field`,
  `Scalar`, the `side` and `marketdatakind` enums, the `forex` code),
  `yggdryl-hashing` (the identities a message settles).
