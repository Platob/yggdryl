---
name: yggdryl-fix
description: Decodes, encodes and streams FIX messages with yggdryl against a FIX dictionary (FixRegistry), in Rust, Python and Node.js. Use when parsing tag=value/SOH frames, bridge rows or FIXML from logs (parse_line / parseLine, parse_text_arrow_reader / parseTextArrowReader), re-emitting wire bytes (into_text / intoText), landing captures in Arrow or Parquet (fix_schema, arrow_reader / arrowReader), chaining order lifecycles (lifecycle, crossuuid), turning FIX into market data and books (market_data, book_arrow_reader), building or storing a dictionary (FixRegistry.from_handle / fromHandle, from_cfb_file, merge_with, commit, code sets, FIX metadata) or running the yggdryl fix CLI.
---

# yggdryl FIX

A FIX field is an ordinary `Field` whose `FIX:` metadata (`FIX:tag`, `FIX:names`,
`FIX:codeset`, ...) the `fix` protocol view reads; there is no second field
class. `FixRegistry` is the dictionary: **one namespace** holding scalar fields,
components (a message is a component carrying `FIX:msgtype`), groups, and the
named code sets beside them. `FixCodec` is one dictionary plus the pins of a
run; its `parse_*` readers turn captured bytes into `FixMsg` values - a market
event (identity, clocks, state, side, price...) over a content row typed by the
dictionary. Every message projects onto one fixed row, `fix_schema(registry)`,
decided from the dictionary alone, so a whole capture streams as Arrow batches.
Each message states its `msgcat` - the `MarketDataKind` its type files under
(`ORDR`, `QUOT`, `EXEC`, `TRAD`, `BOOK`, the batches `ORDB`, `QUOB`, `TRDB`,
...) - and the parse splits what it reports once: an execution report is its
order's report (`ORDR`, `QUOT` naming a `QuoteID`), a filling one adds its
`EXEC` message, a trade one execution per side, a two-sided quote a `BUYS` and
a `SELL` quote, a batch one message per entry. A lifecycle chains within one
`msgcat`, so a fill never follows its order.

Hold two speeds apart. **Decoding is per message**: each frame is parsed on its
own, in parallel (`threads`), answers in input order, and never reads another
message - it only takes the next place (`seqnum`) of its instant in stream
order, so a report and the execution split off it are places 0 and 1.
**Lifecycle is the only cross-message stage**: `lifecycle` collects a finite
capture, sorts it by event time, folds duplicate deliveries, places each
message by content among the messages of its instant (a content repeated
there keeps its place), chains it to the live one of its order within its own `msgcat` (`crossuuid`,
`prevuuid`; an order and an execution under one cross code are two chains), takes every bridge `metadata` key of the chain it does not state
and the ids its dictionary follows, and learns instrument associations.
Nothing chains unasked.

The dictionary is data, not code: the committed FIX Latest dictionary
(fields, 181 messages, 737 code sets, every tag FIX 4.0 to 5.0 SP2 declared) is
the `config/fix` folder of the yggdryl repository, generated and committed,
~14 MB, **not shipped** in the crate, wheel or npm package. Load it by path, or
point `YGGDRYL_FIX_REGISTRY` (or `~/.config/fix`) at it for the process default.

## Choose the door

| Task | Rust | Python | JavaScript |
| --- | --- | --- | --- |
| load a stored dictionary | `FixRegistry::from_handle(&LocalFolder::new(path)?)?` | `FixRegistry.from_handle(path)` | `fix.FixRegistry.fromHandle(path)` |
| the process default | `FixRegistry::from_env()?`, `FixRegistry::install_env(r)?` | `FixRegistry.from_env()`, `FixRegistry.install_env(r)` | `fix.FixRegistry.fromEnv()`, `fix.FixRegistry.installEnv(r)` |
| a dictionary in memory | `FixRegistry::from_fields([..])?`, `registry.insert(field)?` | `FixRegistry.from_fields([...])`, `registry.insert(field)` | `fix.FixRegistry.fromFields([...])`, `registry.insert(field)` |
| FIX facts on a field | `field.as_fix().tag()?`, `field.as_fix_mut().set_tag(38)?` | `field.fix.tag`, `field.fix.tag = 38` | `field.fix.tag`, `field.fix.tag = 38` |
| look a field up | `field(55)`, `field_by_name`, `field_by_path(&FieldPath)`, `field_by_counter(453)`, `field_by_id(FixId)` | `field(55)`, `field_by_name`, `field_by_path("Parties.PartyID")`, `field_by_counter`, `field_by_id(int)` | `field(55)`, `fieldByName`, `fieldByPath`, `fieldByCounter`, `fieldById` |
| a message definition | `registry.msgtype("D")?` | `registry.msgtype("D")` | `registry.msgtype('D')` |
| a field's code set | `registry.codeset_of(field)`, `set_codeset(name, &[FixCode])?` | `codeset_of(field)`, `set_codeset(name, [{...}])` | `codesetOf(field)`, `setCodeset(name, [...])` |
| persist a dictionary | `registry.commit(&mut folder)?` | `registry.commit(path)` | `registry.commit(path)` |
| read a venue CBlock (`.cfb`) | `FixRegistry::from_cfb_file(&LocalFile::new(path)?, Some("venue"))?` | `FixRegistry.from_cfb_file(path, "venue")` | `fix.FixRegistry.fromCfbFile(path, 'venue')` |
| fold CBlocks or another dictionary in | `registry.add_cfb_file(&file, None)?`, `registry.add_cfb_files(folder.glob("*.cfb", false)?, None)?`, `merge_with(&other)?` - each answering a `FixMerge` | `registry.add_cfb_file(path)`, `add_cfb_files(folder, "*.cfb")` - answering a `dict` | not bound (`yggdryl fix ingest`) |
| a codec for a run | `FixCodec::new(Arc::new(registry)).with_threads(4)` | `FixCodec(registry, threads=4)` | `new fix.FixCodec(registry, { threads: 4 })` |
| read only some types | `.with_include_msgtypes(["D", "8"])` | `FixCodec(r, include_msgtypes=[...])` | `{ includeMsgtypes: [...] }` |
| sniff a line's type, no dictionary | `FixCodec::infer_msgtype_bytes(bytes)` | `FixCodec.infer_msgtype_bytes(bytes)` | `fix.FixCodec.inferMsgtypeBytes(buffer)` |
| decode one captured line | `codec.parse_line(bytes)?` (iterator) | `codec.parse_line(bytes)` | `codec.parseLine(buffer)` |
| decode exactly one frame | `codec.parse_fix_line(bytes)?` | `codec.parse_fix_line(bytes)` | `codec.parseFixLine(buffer)` |
| decode many lines lazily | `codec.parse_lines(lines)` | `codec.parse_lines(lines)` | `codec.parseLines(lines)` |
| decode text-reader lines | `codec.parse_text_lines(lines)` | `codec.parse_text_lines(lines)` | `codec.parseTextLines(lines)` |
| a capture's batches to FIX rows | `codec.parse_text_arrow_reader(reader)?` | `codec.parse_text_arrow_reader(reader)` | `codec.parseTextArrowReader(reader)` |
| read a fact | `msg.by_tag(55)?`, `by_name`, `by_path`, `header()`, `get_side()` | `msg.by_tag(55)`, `by_path(...)`, `header()`, `msg.side` | `msg.byTag(55)`, `byPath(...)`, `header()`, `msg.side` |
| the category, the strike | `msg.msgcat()`, `msg.strikepx()` | `msg.msgcat`, `msg.strikepx` | `msg.msgcat`, `msg.strikepx` |
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
| one message as graph leaves | `msg.market_data()?`, `msg.into_market_data()?` | `msg.market_data()` | `msg.marketData()` |
| sorted market data | `codec.market_data(messages)` | `codec.market_data(messages)` | `codec.marketData(messages)` |
| books as `marketdata` rows | `codec.book_arrow_reader(msgs, 0)?` | `codec.book_arrow_reader(msgs, snapshot_millis=0)` | `codec.bookArrowReader(msgs, 0)` |
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
   partition by hour partition, sorted by `currunix` - pins
   `sorted_lifecycle` (`with_sorted_lifecycle(true)`, `sortedLifecycle`):
   `lifecycle` then holds one epoch hour at a time, walking an hour once a
   message two hours past it is read, and answers the same walk. Parse and
   project in the parallel doors; chain once. The walk yields each
   `curruuid` once within `dedup_window_ms` of event time (one minute by
   default; `None`/`null`/`0` yields every restated twin too), so a
   consumer keyed by `curruuid` needs no dedup of its own.
6. Market hand-off is `market_data(lifecycle(messages))`: the walk settles
   each message, the sorted door orders every leaf by the instant a book folds
   it. One message is one leaf - an order, a one-sided quote, an execution -
   and a `W`/`X` message one per entry; a trade, a batch and a two-sided quote
   reach the book as the messages their parse split off, never twice.
   `book_arrow_reader` does not sort - an operation dated before its book is
   left out with a warning - and neither door runs the lifecycle for you. For a capture already landed as FIX rows,
   `market_data_arrow_reader` reads each row as its message (no line parsed
   again) and sorts its leaves; it runs no lifecycle either, and a
   `lifecycle_arrow_reader` row lacks what the walk settled (`prevpx`, a
   carried side), so hand walked messages, not rows.
7. Pin `default_sending_time` for reproducible reads. A frame stating no
   `SendingTime(52)` whose line carries no clock is dated by one UTC-now read,
   and the clock feeds `curruuid`, as does a `SendingTime(52)` naming no
   instant; a pinned instant (or a row-header `mtime` capture) makes two reads
   identical.
8. A column is found by name, never position: `schema.index_of("msgtype")`, or
   `fix_column_of(&schema, 35)` in Rust. Columns are the dictionary's folded
   names; the tag stays on each column's `FIX:tag`. Two captures under one
   dictionary share one schema exactly.
9. A capture's own columns (`url`, `rownum`, `loglevel`...) lead the
   row; a column named after a FIX field fills that field where the frame
   stated none; `beginstring` and `msgdirection` columns are per-row
   parameters. An `mtime` capture dates the line - its messages' `recdunix`
   and the sending clock of any stating no `SendingTime(52)` - read under the
   text options' `timezone`; a `timestamp` capture of your own dates nothing.
   `ULBRIDGE_ROWHEADER` captures `mtime` and `loglevel`, so it dates every
   line it matches (set `timezone` to the bridge's local zone).
10. Store and reload a dictionary through `commit`/`from_handle` (or `yggdryl fix`).
    `commit` writes only documents that changed, prunes what no definition
    holds, and answers `written`/`removed`; never hand-edit the generated
    shards, and state a code set before the field that names it. A fold
    (`add_cfb_file`, `add_cfb_files`, `merge_with`) keeps every declaration the
    dictionary already holds. A datatype a source states at another precision
    of the stored one - unbounded text against anything, any two numbers, an
    integer against an enum, a date against a datetime (a CBlock's `float`
    against `decimal128`, `string` against `ccy`) - folds under it and is
    counted in `restated`; a contradiction (`boolean` against `int32`, a
    time of day against a timestamp), a member a held definition declares in
    another shape and a group on another counter are passed over, each named
    in the answered `FixMerge` rather than refusing the whole source. A field
    named by nothing but its tag (a CBlock tag no `alt` or binding names) is
    unnamed: one on a held tag folds into the holder, and the first name to
    arrive on its tag names it. A field on a held tag under another name
    stands beside the holder, and neither learns the other's name. A fold
    refuses whole only where nothing is left to keep (malformed XML or JSON,
    a source whose own catalog does not validate); what a CBlock states that
    the reader cannot keep is dropped, or kept another way (an unread type
    word types the tag string), with a `log` warning naming the line, the
    column, the element and what the reader did instead. `add_cfb_files`
    folds in ascending URL order, so where two files type one tag two ways the
    first-sorting file's declaration is held, and a code set only widens (the
    held name wins a shared value; a new value under a taken name stays
    unnamed).
11. Mutations are atomic: a refused `insert`, `set` or `set_codeset` leaves the
    registry or message unchanged. A registry shared by a codec or message is
    frozen in the bindings; mutate first, then build codecs.
12. Row door and wire agree: `into_row` keeps typed facts in their columns and
    only what no column holds in `fixentries`, a sorted `map<utf8, utf8>`
    keyed `tag:name` (`58:text`) - a scalar as its wire text, a group or a
    component as JSON keyed the same way, a repeated key as the JSON array of
    its occurrences. A key the dictionary does not resolve is no field: it
    lands in the row's `metadata` under its own spelling, and a row read back
    restores it, so the wire re-emits it. `write_arrow_reader` rebuilds each
    message from the row and refuses a batch with no `fixentries`. A group
    holding no occurrence (`802=0`, or `[]` where a table such as PyIceberg
    stored an absent list) states nothing: no entry, no content in
    `currhashcode`, so a row read back folds with the delivery it was.
13. The derived fills and the retired-field restatements are native code: a
    registry carries no rule of its own, and nothing in `FIX:` metadata
    changes how a field is filled.
14. Data is never an error; only a source failure is. A parse, the market
    projection, the lifecycle and the book walk default what a message states
    that they cannot read - a value that will not type is null beside a
    `FixAnomaly`, a clock naming no instant is unstated - or leave the item
    out, each with a deduplicated warning: Rust `log` at `WARN`, Python
    `logging` under `yggdryl.<module path>`, standard error in JavaScript and
    the CLI. Only a reader, store or runtime that could not answer is an error
    item, yielded after the messages before it, and it ends the stream.

## Pitfalls

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
- A group member needs its index on a message: `Parties[0].PartyID`;
  `Parties.PartyID` is the schema spelling and misses on a value.
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
- An order's, a quote's or an execution's `crosscode` carries its side
  (`BUYS:A1`) - `msgcat` `ORDR`, `QUOT` or `EXEC`; every other message keeps
  its code as spelled, whatever side it states. A derived execution is chained
  under its `ExecID(17)` as given (`BUYS:E-1`), else
  `TradeID=<TradeID(1003)>`. Count messages after the parse, not lines: one
  filling report is two messages.
- A message's parties and its `Account(1)` are its `accountids` - each
  `PartyID(448)` under its `PartyRole(452)`'s upper-cased name
  (`EXECUTINGTRADER`, `CUSTOMERACCOUNT`), `PARTY` where no role is stated, the
  first party of a role standing, and the account under `ACCOUNT` - and
  read-only: write the `Parties` occurrence or `Account(1)`, not the map.
  Regulatory trade ids (`NoRegulatoryTradeIDs(1907)`) are `altids` under
  `REGTRADEID`, `TVTIC`, ...
- A graph leaf (`market_data`) carries in its `metadata` what its message
  states that no typed column reads and none of the leaf's identifier maps
  holds: a party, the account and a regulatory id its maps hold are left out (a
  second party of one role stays, in `parties`). A scalar whose key ends with
  an identifier its message's type declares (`marketorderid`, `RefOrderID(1080)`,
  a bridge's `venue.x.parentorderid` on an execution report) is lifted into the
  leaf's `altids`, so a leaf's `altids` can hold more than its message's.
  `with_market_metadata(false)` / `market_metadata=False` /
  `marketMetadata: false` turns both off and moves the leaf's identity.
- A `Symbol(55)` naming one currency pair - `EUR/USD`, `EURUSD`, `EUR-USD 1M`,
  a RIC's `EURUSD=` - states the derived `FOREX` security identifier `EUR/USD`
  (the `forexcode` column); a pair a row states is stated, never re-derived.
- A row header that stops matching silently changes lifecycle results: the
  line keeps its body but is dated by its file's modification time and
  carries no session context (no delivery folding); assert the matched-line
  count beside the parsed-message count. The `prevunix` a text read states on
  each line (the instant the line before it was dated by) is ignored by the FIX
  text doors: a message's own comes from the lifecycle.

## Language references

- Rust: [references/rust.md](references/rust.md) - `yggdryl::{FixCodec, FixRegistry, FixMsg, fix_schema}`, `Arc`, iterators of `Result`.
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
