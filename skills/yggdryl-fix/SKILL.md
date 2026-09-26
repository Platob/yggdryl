---
name: yggdryl-fix
description: Decodes, encodes and streams FIX messages with yggdryl against a FIX dictionary (FixRegistry), in Rust, Python and Node.js. Use when parsing tag=value/SOH frames, bridge rows or FIXML from logs (parse_line / parseLine, parse_text_arrow_reader / parseTextArrowReader), re-emitting wire bytes (into_text / intoText), landing captures in Arrow or Parquet (fix_schema, arrow_reader / arrowReader), chaining order lifecycles (lifecycle, crossuuid), turning FIX into market operations and books (market_operations, book_arrow_reader), building or storing a dictionary (FixRegistry.from_handle / fromHandle, from_cfb_file, merge_with, commit, code sets, FIX metadata) or running the ygg fix CLI.
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

Hold two speeds apart. **Decoding is per message**: each frame is parsed on its
own, in parallel (`threads`), answers in input order, and never looks at
another message. **Lifecycle is the only cross-message stage**: `lifecycle`
collects a finite capture, sorts it by event time, folds duplicate deliveries,
chains each message to the live one of its order (`crossuuid`, `prevuuid`,
`seqnum`) and learns instrument associations. Nothing chains unasked.

The dictionary is data, not code: the committed FIX Latest dictionary
(fields, 181 messages, 736 code sets, every tag FIX 4.0 to 5.0 SP2 declared) is
the `config/fix` folder of the yggdryl repository, generated and committed,
~14 MB, **not shipped** in the crate, wheel or npm package. Load it by path, or
point `YGGDRYL_FIX_REGISTRY` (or `~/.config/fix`) at it for the process default.

## Choose the door

| Task | Rust | Python | JavaScript |
| --- | --- | --- | --- |
| load a stored dictionary | `FixRegistry::from_handle(&LocalFolder::new(path)?)?` | `FixRegistry.from_handle(path)` | `fix.FixRegistry.fromHandle(path)` |
| the process default | `FixRegistry::global()?`, `FixRegistry::install_global(r)?` | `global_registry()`, `install_global_registry(r)` | `fix.globalRegistry()`, `fix.installGlobalRegistry(r)` |
| a dictionary in memory | `FixRegistry::from_fields([..])?`, `registry.insert(field)?` | `FixRegistry.from_fields([...])`, `registry.insert(field)` | `fix.FixRegistry.fromFields([...])`, `registry.insert(field)` |
| FIX facts on a field | `field.as_fix().tag()?`, `field.as_fix_mut().set_tag(38)?` | `field.fix.tag`, `field.fix.tag = 38` | `field.fix.tag`, `field.fix.tag = 38` |
| look a field up | `field(55)`, `field_by_name`, `field_by_path(&FieldPath)`, `field_by_counter(453)`, `field_by_id(FixId)` | `field(55)`, `field_by_name`, `field_by_path("Parties.PartyID")`, `field_by_counter`, `field_by_id(int)` | `field(55)`, `fieldByName`, `fieldByPath`, `fieldByCounter`, `fieldById` |
| a message definition | `registry.msgtype("D")?` | `registry.msgtype("D")` | `registry.msgtype('D')` |
| a field's code set | `registry.codeset_of(field)`, `set_codeset(name, &[FixCode])?` | `codeset_of(field)`, `set_codeset(name, [{...}])` | `codesetOf(field)`, `setCodeset(name, [...])` |
| persist a dictionary | `registry.commit(&mut folder)?` | `registry.commit(path)` | `registry.commit(path)` |
| read a venue CBlock (`.cfb`) | `FixRegistry::from_cfb_file(&LocalFile::new(path)?, Some("venue"))?` | `FixRegistry.from_cfb_file(path, "venue")` | `fix.FixRegistry.fromCfbFile(path, 'venue')` |
| fold CBlocks or another dictionary in | `registry.add_cfb_file(&file, None)?`, `add_cfb_files(&folder, "*.cfb", None)?`, `merge_with(&other)?` | `registry.add_cfb_file(path)`, `add_cfb_files(folder, "*.cfb")`, `merge_with(other)` | not bound (`ygg fix sync`) |
| a codec for a run | `FixCodec::new(Arc::new(registry)).with_threads(4)` | `FixCodec(registry, threads=4)` | `new fix.FixCodec(registry, { threads: 4 })` |
| read only some types | `.with_include_msgtypes(["D", "8"])` | `FixCodec(r, include_msgtypes=[...])` | `{ includeMsgtypes: [...] }` |
| sniff a line's type, no dictionary | `FixCodec::infer_msgtype_bytes(bytes)` | `FixCodec.infer_msgtype_bytes(bytes)` | `fix.FixCodec.inferMsgtypeBytes(buffer)` |
| decode one captured line | `codec.parse_line(bytes)?` (iterator) | `codec.parse_line(bytes)` | `codec.parseLine(buffer)` |
| decode exactly one frame | `codec.parse_fix_line(bytes)?` | `codec.parse_fix_line(bytes)` | `codec.parseFixLine(buffer)` |
| decode many lines lazily | `codec.parse_lines(lines)` | `codec.parse_lines(lines)` | `codec.parseLines(lines)` |
| decode text-reader lines | `codec.parse_text_lines(lines)` | `codec.parse_text_lines(lines)` | `codec.parseTextLines(lines)` |
| a capture's batches to FIX rows | `codec.parse_text_arrow_reader(reader)?` | `codec.parse_text_arrow_reader(reader)` | `codec.parseTextArrowReader(reader)` |
| read a fact | `msg.by_tag(55)?`, `by_name`, `by_path`, `header()`, `get_side()` | `msg.by_tag(55)`, `by_path(...)`, `header()`, `msg.side` | `msg.byTag(55)`, `byPath(...)`, `header()`, `msg.side` |
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
| sorted market operations | `codec.market_operations(messages)` | `codec.market_operations(messages)` | `codec.marketOperations(messages)` |
| books as `marketdata` rows | `codec.book_arrow_reader(msgs, 0, false)?` | `codec.book_arrow_reader(msgs, snapshot_millis=0, global_=False)` | `codec.bookArrowReader(msgs, 0, false)` |
| sorted operations as `marketdata` rows | `codec.market_arrow_reader(msgs)?` | `codec.market_arrow_reader(messages)` | `codec.marketArrowReader(messages)` |
| FIX rows in Arrow to operation rows | `codec.market_operations_arrow_reader(reader)?` | `codec.market_operations_arrow_reader(reader)` | `codec.marketOperationsArrowReader(reader)` |
| manage a dictionary from a shell | `ygg fix --root <dir> ...` ([cli](references/cli.md)) | same binary, shipped in the wheel | same binary |

## Rules for fast, correct use

1. Load the dictionary once and share it. The full seed takes about a second
   to load and resolve (release Rust; two from Python): build one
   `FixRegistry`, wrap it in one `Arc` (Rust) or pass the same object
   (bindings) to every codec, or install it as the process default before
   anything resolves the default.
2. Pin a run on the codec, never per call: `threads`, `batch_byte_size`
   (128 MiB target) / `batch_row_size` (32,768), `include_msgtypes`,
   `payload_column`, `default_sending_time`, `snapshot_ns`. One codec reads a
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
   `lifecycle_arrow_reader` and `market_operations` collect the whole finite
   capture (they sort by event time), so feed them one session or day, not an
   unbounded stream. Parse and project in the parallel doors; chain once.
6. Market hand-off is `market_operations(lifecycle(messages))`: the walk
   settles each message, the sorted door orders every operation by the instant
   a book folds it. `book_arrow_reader` is strict - it refuses out-of-order
   input - and neither door runs the lifecycle for you. For a capture already
   landed as FIX rows, `market_operations_arrow_reader` reads each row as its
   message (no line parsed again) and sorts its operations; it runs no
   lifecycle either, and a `lifecycle_arrow_reader` row lacks what the walk
   settled (`prevpx`, a carried side), so hand walked messages, not rows.
7. Pin `default_sending_time` for reproducible reads. A frame stating no
   `SendingTime(52)` whose line carries no clock is dated by one UTC-now read,
   and the clock feeds `curruuid`; a pinned instant (or a row-header `mtime`
   capture) makes two reads identical.
8. A column is found by name, never position: `schema.index_of("msgtype")`, or
   `fix_column_of(&schema, 35)` in Rust. Columns are the dictionary's folded
   names; the tag stays on each column's `FIX:tag`. Two captures under one
   dictionary share one schema exactly.
9. A capture's own columns (`url`, `rownum`, `timestamp`, `level`...) lead the
   row; a column named after a FIX field fills that field where the frame
   stated none; `beginstring` and `msgdirection` columns are per-row
   parameters. A `timestamp` capture never dates a message - name it `mtime`
   to date the line.
10. Store and reload a dictionary through `commit`/`from_handle` (or `ygg fix`).
    `commit` writes only documents that changed, prunes what no definition
    holds, and answers `written`/`removed`; never hand-edit the generated
    shards, and state a code set before the field that names it. Folds
    (`add_cfb_file`, `add_cfb_files`, `merge_with`) are atomic; `add_cfb_files`
    folds in ascending URL order, so the last-sorting file wins a disputed tag,
    and a code set only widens (the held name wins a shared value).
11. Mutations are atomic: a refused `insert`, `set` or `set_codeset` leaves the
    registry or message unchanged. A registry shared by a codec or message is
    frozen in the bindings; mutate first, then build codecs.
12. Row door and wire agree: `into_row` keeps typed facts in their columns and
    only unexplained arrival content in `fixentries`; `write_arrow_reader`
    rebuilds each message from both and refuses a batch with no `fixentries`.

## Pitfalls

- A bare integer is always a **tag**: `registry.field(55)`, `msg.get(55)`. A
  field identity (`FixId`, the signed XXH32 of tag + folded name) is only
  reached through `field_by_id` / `get_by_id` (`FixKey::Id` in Rust).
- A captured line is not a message. `parse_line` answers an iterator - none
  for a sentence, two for two frames on one line; unpack it
  (`message, = codec.parse_line(b)` / `const [m] = codec.parseLine(buf)`).
  `parse_fix_line` refuses a body holding a second frame.
- An empty line through `parse_lines` is an error *item*, not a stream end:
  iterate and handle per item (Rust `Result`, Python raises at `next`).
- Names are folded (ASCII case, `_`, `-`, space dropped): `MsgType`,
  `msg_type`, `MSG-TYPE` are one name; `Größe` and `GRÖSSE` are two.
- A coded value reads as its name (`by_tag(54)` -> `BUY`) but emits as its
  wire code (`54=1`); set it with the wire code or any spelling the code set
  resolves.
- A group member needs its index on a message: `Parties[0].PartyID`;
  `Parties.PartyID` is the schema spelling and misses on a value.
- `into_text` output reflects what the dictionary derived (a day order's
  `59=0`), minus facts supplied at intake (an unstated `SendingTime`); it is
  canonical wire, not a byte-for-byte copy of the input.
- A Python `str` in `parse_lines` is refused; pass `bytes`. JavaScript
  `parseLine` takes a `Buffer`; `parseLines` accepts strings or buffers.
- Without a dictionary (`FixRegistry()` / `new fix.FixRegistry()`) frames
  still parse and the header, the lifted numbers and identifiers (11, 37, 38,
  44...) and the crate's own columns are typed, but every other key lands under
  tag 0 with its raw spelling: no code names, no groups, no typed row.
- A row header that stops matching silently changes lifecycle results (no
  session context, no delivery folding); assert the matched-line count beside
  the parsed-message count.

## Language references

- Rust: [references/rust.md](references/rust.md) - `yggdryl::{FixCodec, FixRegistry, FixMsg, fix_schema}`, `Arc`, iterators of `Result`.
- Python: [references/python.md](references/python.md) - `yggdryl.fix`, pyarrow readers in and out.
- JavaScript: [references/javascript.md](references/javascript.md) - the `fix` namespace, `Buffer` input, `BatchReader`.
- CLI: [references/cli.md](references/cli.md) - `ygg fix` catalog commands.

Read the one for the language you write; recipes appear in the same order in each.

## Deeper

- FIX overview and `FIX:` vocabulary: https://platob.github.io/yggdryl/fix/
- Decode rules (none, one or many per line; type filter): https://platob.github.io/yggdryl/fix/decode/
- Encode order and separators: https://platob.github.io/yggdryl/fix/encode/
- Registry, one namespace, code sets, global default: https://platob.github.io/yggdryl/fix/registry/
- Store layout and snapshots: https://platob.github.io/yggdryl/fix/store/
- `FixMsg` holders, accessors, writes: https://platob.github.io/yggdryl/fix/message/
- Arrow doors, pins, market books: https://platob.github.io/yggdryl/fix/arrow/
- Capture, fixed row, clocks: https://platob.github.io/yggdryl/fix/capture/
- Lifecycle walk: https://platob.github.io/yggdryl/fix/lifecycle/
- CLI: https://platob.github.io/yggdryl/fix/cli/
- Sibling skills: `yggdryl-market-data` (the operations and books FIX turns
  into), `yggdryl-records` (text reader, row headers, Parquet/IPC sinks),
  `yggdryl-arrow` (`Serie`, `BatchReader`), `yggdryl-types` (`Field`,
  `Scalar`), `yggdryl-expressions` (the plans `FIX:replacements` spell),
  `yggdryl-hashing` (the identities a message settles).
