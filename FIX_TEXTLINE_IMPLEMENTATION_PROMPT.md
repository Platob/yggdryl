# Reading FIX off `TextLine`

Adapt `claude/fix-text-line-parsing-d89lw6` onto the text reader that landed in
`main`. The FIX layer currently reads a message out of a name-to-value `Scalar`
record, and splits its own pairs to do it. Both jobs now have an owner: a text
read answers `TextLine`, and pair scanning belongs to `TextEntries`. Follow
`AGENTS.md`; this file carries only the decisions this adaptation needs.

**Order.** `main` has the text reader first. Rebase this branch onto it, resolve
against what landed rather than around it, and only then start the work below.

## Outcome

- The FIX codec takes a `TextLine`, not a `Scalar` record. The payload, the
  direction, the write time and the source URL are read as typed fields rather
  than looked up by name per row.
- `TextEntries` is the one pair scanner. The codec stops splitting on `=` and
  stops choosing a separator; it asks for the entries a line already has.
- `FixEntry` keeps only what FIX adds and a line cannot say: the tag. Its key and
  value become ranges of the page the line was read into rather than its own
  `SmolStr` copies, and its `branch` goes.
- Every path the FIX layer addresses a value by is a `FieldPath`, resolved once.
- What this deletes is named, not left beside its replacement.

## What is landed

Read these before deciding anything. They are on `main` and are not rebuilt.

| notion | what it is |
| --- | --- |
| `TextLine` | one decoded text row: `index`, `url`, `timestamp` (nanoseconds UTC in 128 bits), `bodytype`, `body`, `captures`, `entries`, `direction`, `dropped_byte_size` |
| `TextBytes` | a counted range over one retained page; `as_bytes` borrows, `slice` stays in the same page, identity is the bytes |
| `TextEntry` / `TextEntries` | the recursive key/value tree a line carries, keys and values as ranges, built only when something asks |
| `read_entries` | the tree reader, over `mime_type::line::entry_spans` |
| `entry_spans` | every pair a line declares, wherever it sits, as ranges - deliberately not the walk `inspect` runs |
| `read_text_lines` | the one decode entry point; every record method routes through it |
| `into_arrow_batch` / `into_arrow_reader` / `from_arrow_batch` / `from_arrow_reader` | both directions between lines and Arrow |
| `TextPlan` | the compiled column plan; `source_field` is built from it |
| `TextOptions::lift_names` | entry paths lifted into columns, each named by its `as` alias or its last segment |
| `FieldPath` / `FieldSegment` | the one path grammar, in `expression/`, with a SQL-style `as` alias |

## Read first

- `AGENTS.md` in full, especially **"Paths are resolved, never split"** and
  **"Resolution is a boundary event, never a per-item one"**. Both are the
  argument for everything below.
- `media/text/line.rs`, `entry.rs`, `bytes.rs`, `plan.rs` - the whole surface,
  it is small.
- `mime_type/line.rs`: `inspect`, `classify`, `entry_spans`, `pairs`,
  `pair_at`, `next_entry`, `is_field_end`, `locate_frame`, `LineSeparator`.
  **This is the most important reading in the list**, because the codec's own
  splitting and this scanner disagree in ways the section below names.
- On this branch: `fix/record.rs` whole, `fix/entry.rs` whole, and the parsing
  half of `fix/codec.rs` - `parse_line`, `parse_line_with`, `parse_fix_line`,
  `split_fix_with`, and the two places that call `memchr(b'=')` directly.

## The record surface goes

`fix/record.rs` exists to pull a payload and a handful of per-row facts out of a
name-to-value `Scalar`. Every one of those facts is now a typed field on
`TextLine`, so the lookup, the map that held it, and the row canonicalization
behind it are all cost with nothing left to buy:

| what the record reads today | where it comes from now |
| --- | --- |
| the payload column, `body` by default | `line.body()`, a range, no copy |
| `direction` | `line.direction()` |
| the clock column | `line.timestamp()`, already nanoseconds UTC |
| `beginstring`, `sep`, `pluginid` | a lifted entry or a row-header capture, resolved by `FieldPath` once |

Replace `FixCodec::parse_text_record` with a `TextLine` entry point. Keep the
split the module doc already draws: reading one line speaks no Arrow, so it
stays out of the `arrow`-gated batch module, exactly as it is today.

**Decide and state**: whether the per-row parameter columns arrive as lifted
entries, as row-header captures, or as an explicit `TextOptions` the FIX layer
builds. Pick one, write it down, and do not accept two. The column names
`DEFAULT_PAYLOAD_COLUMN`, `BEGINSTRING_COLUMN`, `SEPARATOR_COLUMN`,
`CLOCK_COLUMN`, `DIRECTION_COLUMN` and the plugin one either become resolved
`FieldPath` constants or they go; they must not stay as strings looked up per
row.

Deleted by this, and named in the commit: the record entry point, the column-name
constants that no longer address anything, and every `Scalar` map walk on the
per-row read path.

## One pair scanner

`FixEntry` and `TextEntry` are the same shape. `FixEntry` is
`{ tag, branch, key: SmolStr, value: SmolStr, children: Vec<FixEntry> }`;
`TextEntry` is `{ key: TextBytes, value: TextBytes, entries: Option<TextEntries> }`.
The difference is FIX's two resolved integers - and two owned string copies per
pair that the text reader does not make.

**Collapse the storage, keep only the resolution that earns its place.**
`FixEntry` holds a `TextEntry` plus its `tag`, or holds the same ranges
directly. Either way:

- keys and values stop being `SmolStr` and become `TextBytes`, so a message read
  from a line copies none of the bytes it names;
- `children` becomes `TextEntries`;
- `tag` stays. It is what FIX adds and no text reader can answer it.

The codec then stops scanning. Where it calls `memchr(b'=')` and where it walks
a frame to find the next pair, it asks the line for `entries` instead.

### `branch` goes

An entry says what arrived. The tag and the key are what arrived; the branch is
what a *dictionary* decided about it, and it is the same value for every entry of
one message, so storing it per pair repeats one fact once per field on the wire.
The message already knows its branch, and `FixId::branch` still answers branch
identity for a field - nothing about resolution is lost, only its duplication.

Delete, in the same change: the `branch` member, `with_branch`,
`with_branch_digest`, the `branch` reader, the `signed`/`unsigned` digest
helpers and `standard_digest` if nothing else uses them, and the `branch` column
wherever an entry is written to or read back from a row. Where a caller needs
the branch of an entry, it asks the message that holds it.

**The one thing to check before deleting:** the doc on `with_branch_digest`
says a row's `branch` column is copied back verbatim so an entry read out of a
row does not re-resolve. Confirm the round trip still holds once the column is
gone - re-emission has to produce the same bytes - and if a fixture depends on
the column, change the fixture and say so.


### The disagreements you must settle first

These are real and they are why this is not a mechanical swap. Work each one out
against the code before writing any of it, and write the answer into the module
docs:

1. **Separator choice.** The codec pins or infers a frame separator, unescaping
   `\x01`, `^A` and `<SOH>` and splitting on the real byte. `entry_spans` finds
   pairs at their `=` and closes each value at `is_field_end`, which already
   knows those spellings. Confirm they agree on every fixture on this branch, and
   where they do not, say which is right and fix that one - do not keep both.
2. **A `Len`-prefixed data field.** The codec reads a data field to the length
   its `Len` field stated rather than splitting it, precisely so a value carrying
   the separator survives. `entry_spans` has no such notion. This is the one case
   the generic scanner cannot answer alone: decide whether the FIX layer
   post-processes the entries it was handed, or whether the scanner learns a
   bounded "read to a stated length" step. **Do not silently lose it** - there is
   a test on this branch that covers it.
3. **The trailer.** The codec finds the checksum with `rfind` on a trailer
   pattern. `entry_spans` reads every pair including the one after a checksum,
   because it deliberately does not stop where `inspect` stops. Decide what the
   message is bounded by and make that one rule.
4. **Stated absence and marked restatement.** `fix/entry.rs` documents pairs the
   reader reads as *never sent*, and a bridge's marked restatement of a bare
   pair. `entry_spans` reports what is written. Say where that filtering lives -
   it is FIX's reading, so it belongs on the FIX side of the boundary.

If any of these forces a change to `entry_spans` or `read_entries`, make it
there and keep one scanner. A second scanner in the FIX layer is the outcome
this work exists to prevent.

## Paths

Every place the FIX layer addresses a value by a dotted or indexed name becomes
a `FieldPath`, resolved once at the boundary that accepts it and carried
resolved. `FixMsg::get_by_path`/`by_path` and
`FixRegistry::get_field_by_path`/`field_by_path` keep their names - they are the
pair this codebase already uses - and stop splitting strings.

FIX's alias-then-exact name resolution and its folded matching are **real
behaviour and must survive**. They belong in how a segment resolves a name
against the registry, not in a second parser. That is the test of whether this
is right: a dialect's naming rule composes with the one grammar, or the grammar
was too narrow and gets widened once, for everyone.

A path used per row, per column or per field is hoisted out of that loop. The
registry's own lookups are the hottest of these; resolve once and reuse.

## Order of work

Each step is a commit, each green on its own gate.

| # | lands | gate |
| --- | --- | --- |
| 0 | rebase onto `main`; resolve conflicts against the landed reader, no new code | full Rust gate |
| 1 | the four disagreements settled and written down; fixtures proving each | full Rust gate |
| 2 | `FixEntry` over `TextBytes`/`TextEntries`, `branch` deleted; the codec's own splitting deleted | full Rust gate; the entry fixtures unchanged in meaning; the round trip re-emits the same bytes |
| 3 | the `TextLine` entry point; `fix/record.rs` and its constants deleted | full Rust gate; the pipeline tests read the same values |
| 4 | `FieldPath` through the registry and the message navigator | full Rust gate; the FIX naming tests unchanged |
| 5 | Python, then Node, then docs and both inventories | each behind its own gate |

## Proving it

- **Equivalence first.** Before step 2, pin what the current codec answers over
  every fixture on this branch - the batch, codec, merge, pipeline, dataset and
  store tests all read real captures. Keep that green through step 4. It is worth
  more than any test written afterwards.
- **Allocations.** `rust/tests/allocations.rs` on this branch already counts the
  FIX path. Add the claim this work is for: reading a message from a line copies
  none of the bytes the entries name, and the count per message drops by two per
  pair, which is what the two `SmolStr` copies cost today.
- **Call counts.** `rust/tests/iobase_calls.rs` must not move. Reading a line and
  reading a message from it is one decode.
- **Benchmarks.** The FIX pipeline benchmark on this branch is the before/after.
  Regenerate it, publish the numbers, and state the machine. Report the decode
  and the message build separately, the way `text_lines` and `text_batch`
  already separate theirs, so a change in the total says which half moved.

## Do not

- Do not keep `parse_text_record` beside the line entry point. One current
  contract; the record shape is what this removes.
- Do not add a `TextLine` field for a FIX fact. The line says what the transport
  wrote; the tag, the branch and the message type are what a dictionary says
  about it, and they belong to `FixMsg` - the branch to the message, not to each
  of its pairs.
- Do not reintroduce `msgtype` on the text side. `main` deleted it deliberately:
  a line's type is what its frame says, which is this codec's reading.
- Do not widen `entry_spans` to answer a FIX question that FIX can answer from
  what it already returns. Widen it only where the *scanner* is wrong.
