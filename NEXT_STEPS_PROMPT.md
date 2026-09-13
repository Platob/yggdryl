# Next: the message's own identity, and a hashing module to hold it

Read in this order and in full before you touch anything: `AGENTS.md`,
`DECISIONS.md` (the standing contract; decisions 13 to 20 were written by the
session before you), then `FIX_DIRECTION_MESSAGES_PROMPT.md`, then this file,
then `rust/tests/fix/equivalence.snapshot` (its header says how it is
regenerated and when you may).

Continue on `claude/upbeat-cori-4ujta6`, or on a branch cut from it - it holds
the eight landed pieces and has not been merged to `main`. Land the pieces below
as **one commit each**, each with its
decision written into `DECISIONS.md` **first**, then the code that keeps it,
then the pins it names. Where a piece says "decide and pin", decide, write it
down, and pin it - do not ask.

## Standing rules (unchanged)

- **Gate 1 whole, per commit**: `cargo fmt --all -- --check`; clippy on the
  crate and on the workspace with `--all-features`, both `-D warnings`;
  `cargo test --locked -p yggdryl --all-targets` and again with
  `--features "parquet iceberg"`; doctests; `cargo check --profile bench
  --benches`; `RUSTDOCFLAGS="-D warnings" cargo doc`; the four MSRV checks
  (1.85.0 three ways, 1.94.0 with `iceberg`); `--test iobase_calls --test
  allocations`; the equivalence tripwire; `scripts/generate_charset_tables.py
  --check`; `scripts/check_charset_interop.py`;
  `scripts/generate_fix_dictionary.py --check`.
- **Gates 2, 3 and 4** (Python suite + mypy strict; Node suite + generated
  files current; `check_docs_examples.py` per language + `mkdocs build
  --strict`) at least after every second piece and before the pull request.
  mkdocs must run as `/usr/bin/python3 -m mkdocs`.
- `export CARGO_INCREMENTAL=0` for every cargo command. The writable disk is a
  fixed allowance; if a write fails with "No space left on device", delete
  `target/release` and stale artifacts rather than starting a new session.
- The equivalence snapshot is regenerated **only** under
  `YGGDRYL_FIX_EQUIVALENCE_WRITE=1`, **only** in the commit of the decision
  that moved it, with the moved lines named in the commit message, and
  **never** to make a test pass.
- No aliases, no redirects, no second spelling for anything renamed or
  deleted. One owner per fact. No `unsafe`. No real client names in test data.
- Do not renumber an existing tag or discriminant. Do not reuse `DataTypeId`
  58.
- Do not run benchmarks. Do not land two pieces in one commit.

## 0. First: pieces 9 to 12 of the previous prompt are still owed

`FIX_DIRECTION_MESSAGES_PROMPT.md` at the repository root is NOT spent. Twelve
pieces were specified; eight landed (decisions 13 to 20, commits `419f09d`
through `01ce4ac` on `claude/upbeat-cori-4ujta6`). Four remain, and they come
first because everything below builds on them:

- **Piece 9 (decision 21)** - `fix:identifiers` as a component property, and the
  crate's `altids`. Section "9." of that prompt. A full reconnaissance of this
  piece is reproduced at the end of this file; read it before you start, it
  contains findings that are expensive to rediscover.
- **Piece 10 (decision 22)** - `uuid`, `puuid`, `instuuid` as RFC 9562 values.
- **Piece 11 (decision 23)** - the lifecycle keeps chains by `puuid`, scoped by
  `instuuid`.
- **Piece 12 (decision 24)** - `prevtimestamp` and `prevuuid`.

Delete `FIX_DIRECTION_MESSAGES_PROMPT.md` and `CHARSET_READ_PATH_PROMPT.md` from
the root once the twelve pieces are all landed, and not before - the first is
still the specification for four of them and the second is a prior task's record.

Note that pieces 10 to 12 and section 2 below overlap: pieces 10 to 12 introduce
`uuid`/`puuid`/`instuuid` and the chains that use them, and section 2 then makes
them mandatory struct fields and redefines `puuid`. Land the pieces as specified
first, then section 2 on top - do not try to merge them, and where section 2
contradicts a piece, section 2 is the later word and wins.

## 1. A `hashing` module

`xxhash` and `txhash` are two modules at the crate root doing one thing.
Regroup them under `rust/src/hashing/`, as `hashing::xxhash` and
`hashing::txhash`, with a module doc that says what the crate hashes and
why - the digest that identifies a message, the one that identifies an
instrument, the time-ordered pair - and centralize the documentation's
**Hashing** section on it, so the docs have one page for the subject rather
than a paragraph per module. Public paths move; nothing keeps the old path.
Update `.api-inventory.txt`, `.api-bindings.txt` and both bindings.

## 2. `timestamp` becomes `updatedat`, and a message states when it was made

Rename the crate field `timestamp` (65003) to `updatedat` - the tag does not
move, the spelling does, everywhere, with no alias. Add beside it:

- `createdat`, the same type `updatedat` has.
- `code`, a `utf8` crate field.
- `msghash`, an `i64`: the xxh64 of every other field of the message
  **except** `updatedat` and `createdat`, over a stable ordering you define
  and write down (name the ordering in the decision - a hash whose input
  order is undefined is not a fact).

All four are **always present and always filled**. Settle in the decision
what "filled" means for each and pin a message that states none of them.

Default rule for `updatedat`: the most relevant instant the message's own
entries state, else `SendingTime` (52), which is itself always defined and
defaults to the current UTC instant. Write down the order of preference -
the impact clock (60) before the session clock (52) is the crate's existing
reading, `fix/lifecycle.rs`; say whether that is what "most relevant" means
here and pin it.

Then: `updatedat`, `createdat`, `uuid`, `puuid` and `msghash` are **hard
fields of the `FixMsg` struct**, not children looked up in the entries on
every read. That is the point of this piece: the hot paths stop scanning.
Keep them consistent with the row's columns in both directions
(`into_row`/`from_row`), and keep `enrich` idempotent.

`uuid` is the txhash of `updatedat` and `msghash`.
`puuid` is the hash of `code` alone - that is the whole rule, and it is what
makes `code` the chain's name rather than a label on it: two messages
carrying one `code` are one chain by construction, with no lookup. Settle
which hash in the decision (the `hashing` module of piece 1 owns the choice)
and say what `puuid` is when `code` is not yet known.

### `snapshotat`, and `updatedat` as a grid

Add one more crate field, `snapshotat`: the instant of the **real event**,
as the message states it - what the "most relevant entry, else `SendingTime`,
else now" rule above answers. `updatedat` then holds the instant of the
**snapshot** that captured it, and the point of the split is that the
snapshot instant is on a grid: a consistent `updatedat` timeline that a
replay reproduces exactly, while `snapshotat` keeps the event's own untidy
clock.

(Confirm the two names read that way round with the user before you write
them - the instruction was "snapshotat for timestamp snapshotted of real
event, with updatedat holding the timestamp of snapshot". The semantics
above are what matters: one field is the event's own instant, the other is
the grid instant of the snapshot that captured it, and the grid one is
`updatedat` so that the `updatedat` timeline is the consistent one.)

The grid is the lifecycle's: **snapshot every X nanoseconds, aligned by
truncating the Unix epoch instant** - the snapshot instant is
`floor(t / X) * X`, never a wall-clock reading - so a replay of the same
stream produces the same snapshots at the same instants. And **do not
snapshot when `updatedat` already sits on that truncated instant**: the
message has already been snapshotted for this bucket and a second one would
add a duplicate to the timeline. X is configuration, with a default written
down in the decision; state what it is in nanoseconds and why.

Pin: a stream replayed twice snapshotting identically; two messages inside
one bucket producing one snapshot; a message whose `updatedat` is already
the bucket instant producing none; and the bucket arithmetic on an instant
exactly on a boundary (it belongs to the bucket it opens, not the one it
closes - say so).

## 3. The lifecycle keeps the new fields right

Adapt `FixLifecycle` so a chain updates `updatedat`, carries `createdat`
from its first message, and stamps `uuid`/`puuid`/`msghash` per decisions 22
to 24. A second pass over the same stream must be equal to the first.

## 4. Validate the main path as one pipeline

Prove the whole read as a pipeline, end to end, on
`rust/tests/fix/ulbridge.log`:

    ulbridge.log
      -> parse text rows          -> Iterator<TextLine>
      -> FixCodec parse           -> Iterator<FixMsg>
      -> FixCodec enrich          -> Iterator<FixMsg>

and then give the single-message and Arrow-reader doors **optimized
converters** both ways between a `BatchReader` and an `Iterator<TextLine>` or
an `Iterator<FixMsg>`, handling the lifted entry columns, so an intermediate
stored layer can be designed on top of them. The iterator pipeline and the
Arrow pipeline must agree; where they cannot, say exactly where and why, and
write the exception down as a named list the suite asserts is exact -
`LIFTED_OUT_OF_A_DOCUMENT` in `rust/tests/fix/dataset.rs` is the pattern to
copy.

Known bound you inherit, already written down in decision 20: a
document-bodied message (FIXML behind `XmlData(213)`) has its fields in no
column and in no arrival entry, so a row cannot carry them and the batch door
cannot restate them. Closing it means recording what a document stated as
entries of its own, which moves every `.entry[i]` the equivalence snapshot
pins. That is a decision of its own, and it is a good candidate for this
piece if you take it - if you do, take it deliberately and name the moved
lines.

## 5. Flatten the intermediate scalar families

`Scalar` (`rust/src/types/scalar.rs:110`) wraps four families in an extra
enum each, where every other variant is direct:

    Scalar::Integer(Integer::I32(Int32(2)))     // three levels
    Scalar::Floating(Floating::F64(..))
    Scalar::Decimal(Decimal::D128(..))
    Scalar::Nested(Nested::Sequence(..))

`DataType` (`rust/src/types/dtype.rs:31`) already spells the same families
flat - `Int8`, `Int16`, `Int32`, `Int64`, `UInt8`..`UInt64`, `Float16`,
`Float32`, `Float64` are variants of the type enum itself, with no `Integer`
or `Floating` in between. So the two enums that describe one thing describe
it two different ways, and every match on a scalar pays for the difference.

Remove the intermediate level. `Integer` (10 variants,
`types/integer/scalars.rs:226`), `Floating` (3,
`types/floating/scalars.rs:600`), `Decimal` (4,
`types/decimal/scalars.rs:116`) and `Nested` (3,
`types/nested/scalars.rs:116`) fold into `Scalar` as their 20 variants, so a
scalar reads `Scalar::I32(..)`, `Scalar::F64(..)`, `Scalar::D128(..)`,
`Scalar::Sequence(..)` and mirrors `DataType` one for one. Nothing keeps the
old spelling and there is no `Integer`-shaped alias left behind.

Watch for, and settle in the decision:
- `const _: () = assert!(std::mem::size_of::<Scalar>() == 48)`
  (`scalar.rs:147`). Say what the size is after the change and keep an
  assertion on it; a scalar is copied per cell of every batch and its size
  is a fact worth pinning, not a detail.
- The four family enums are not only a layer of `Scalar`: each carries its
  own inherent methods (`Floating::bit_width`, `Floating::as_f64`,
  `Decimal::coefficient`, `Decimal::scale`, `Nested::len`, `Nested::is_empty`
  and their neighbours). Decide for each whether it becomes a method on
  `Scalar`, a free function, or goes - and say why. A method that only made
  sense because the family existed should go with it.
- Whether the family enums stay as public types in their own right for
  callers that genuinely want "any integer" (they may; `DataType` has no
  such grouping, which is the evidence that nothing needs one), or are
  deleted outright. Deleting is the default: an unused grouping is the thing
  this piece exists to remove.
- Both bindings, `.api-inventory.txt`, `.api-bindings.txt`, and every
  `{held:?}` rendering in a test or snapshot that spells the old nesting -
  `Integer(I32(Int32(2)))` becomes `I32(Int32(2))`, and the equivalence
  snapshot may move as a result. If it does, regenerate it in this commit
  and name the moved lines.

This is a wide, shallow change touching most of the crate. It is worth
landing on its own, before anything that would conflict with it.

## 6. Lighter documentation code

The doc examples are the slowest part of Gates 1 and 4. Make them lighter -
smaller fixtures, fewer redundant round trips, shared setup where a page
repeats it - without weakening what they demonstrate, so testing and
integration runs get faster. `scripts/check_docs_examples.py` reports the
count per language; report the before and after in the commit message, as
counts, not as wall-clock numbers.

## Where the code is

- FIX core: `rust/src/fix/` - `codec.rs` (the doors), `enrich.rs` (the one
  enriching pass: recover from the arrival record, restate, fill),
  `latest.rs` (restatement, no longer a public door), `lifecycle.rs`,
  `schema.rs` (the fixed row and `FixMsg::from_row`/`into_row`),
  `crated.rs` (the crate's own fields), `batch.rs` (the Arrow doors).
- Bindings: `python/src/`, `node/src/`; hand-maintained inventories
  `.api-inventory.txt` and `.api-bindings.txt` must stay exact.
- Generators: `scripts/generate_fix_dictionary.py`,
  `scripts/generate_charset_tables.py`, both with `--check`.
- Docs: `docs/`, built with `mkdocs build --strict`.


---

# Appendix: reconnaissance for piece 9

## The blocking fact: a FIX field may not be nested, and a Map is nested

- `catalog.rs:354-365 check_shape`: `FixCategory::Fields if field.dtype().is_nested() => Err(not_scalar(field))`.
- `datatype_id.rs:493`: `DataTypeId::Map` is nested.
- `registry.rs:327`: `FixRegistry::new()` inserts crate fields as
  `let _ = registry.insert(field.clone());` — **the error is swallowed**. A crate
  field the catalog refuses disappears silently: no field, and then
  `fix_schema` (`schema.rs:171-173`) hits `let Some(held) = registry.get_field_by_tag(tag) else { continue }`
  and the schema is simply one column short, with nothing said.
- `rust/tests/fix/dictionary.rs:147` asserts outright:
  `assert!(!field.dtype().is_nested(), "wire field {name} is nested")`.
- No crated field is non-scalar today: 4 × fixed_size_binary(16), 1 × Int64,
  1 × DateTime64, Isin, Mic, State, 11 × utf8.

**Ruled: a map is a group, not a field.** A map is a list of entries, and a list
of entries is exactly what a FIX repeating group is - `parties` under
`NoPartyIDs(453)` is a list of non-null structs, and a `Map` is a list of non-null
`entries` structs of `key` and `value`. So the nested thing goes where the nested
things already live, and `check_shape`'s Fields arm - "a wire field is a scalar,
because a tag carries one value" - stays exactly as it is, still true, still
guarding the dictionary.

What that means concretely:

- `definition_category` (`catalog.rs:376-386`) learns one more shape:
  `DataType::Map(_) => Some(FixCategory::Groups)`, beside the
  `List/LargeList of non-null Struct` it already answers Groups for. One
  recognition added; no rule relaxed. `check_shape`'s Groups arm
  (`catalog.rs:359-362`) then admits `altids` because `definition_category`
  answers Groups for it.
- `altids` therefore registers as a **group**, and a group is reached by its
  counter: `catalog.counters` indexes it and `fix_schema:195` finds it through
  `registry.get_group_by_counter(tag)`, exactly as it finds `parties` at 453.
  So 65020 is `altids`' counter tag.
- `rust/tests/fix/dictionary.rs:147` (`assert!(!field.dtype().is_nested())` over
  every `FixCategory::Fields` definition) keeps passing, and now passes **by name
  rather than by accident**: `altids` is not a Field.

Verify before writing, because the recon did not cover it:
1. Does `GroupPlan` construction (`group_plan.rs`, reached from
   `DefinitionField::Group(_, plan)` at `catalog.rs:1051`) work over a `Map`'s
   `entries` struct, or does it assume `occurrence_of` (`catalog.rs:389-395`),
   which matches only `List`/`LargeList` and would refuse a `Map`? If it refuses,
   `occurrence_of` needs the `Map` arm too - the occurrence is `map.entries()`.
2. Does the counter index get populated for a definition whose `fix:counter` is
   its own tag, or does the catalog expect the counter to be a separate scalar
   field (as `nopartyids` is a field beside the `parties` group)? If a separate
   counter field is needed, that is a second crate tag and `CRATED` goes 20 -> 22,
   not 21 - settle it and say which.
3. `GROUP_TAGS: [i32; 3] = [453, 454, 768]` (`schema.rs:96`) has its length in the
   type. Does `altids` belong in it? It is a crate group, not a dictionary one, so
   probably not - but say why in the decision rather than leaving it unsaid.

The alternative rejected: relaxing `check_shape` to admit a nested dtype for a
crate tag. It would have made the crate's own fields a second kind of field with a
second rule, where this makes it no new kind of anything.

Separately, and either way: the silent swallow at `registry.rs:327`
(`let _ = registry.insert(field.clone())`) is its own small defect - a crate field
the catalog refuses disappears without a word, and the schema comes out one column
short with nothing said. Fix it in this piece or name it.

## Three more facts that bite

- `schema.rs:508-513 fitted()`: `if value.is_null() || value.id() == column.dtype().id() { return value; }`
  A mapping and a `DataType::Map` share `DataTypeId::Map`, so a mapping **bypasses
  the value contract on the way into a row** — no key typing, no duplicate check,
  no sortedness check. An unsorted mapping would be written into a `MapArray`
  flagged `keys_sorted=true` (`arrow/value.rs:1048`) and only fail later. `enrich`
  must therefore build `altids` already sorted and unique, and the decision should
  say the sortedness is the producer's duty because `fitted` will not check it.
- `schema.rs:882-904 column_value()`: a new crate tag falls through to
  `Scalar::Null` on every row unless a branch is added. `altids` is filled by
  `enrich`, not derived per row, so this is correct as-is — say so, so the next
  reader does not add a branch.
- `REQUIRED_TAGS` (`schema.rs:145-150`) is `[8, msghash, timestamp, unixpartition]`;
  `altids` is nullable like every other crate field.

## Where the code goes

- Property: bare key `IDENTIFIERS = "identifiers"` beside the others in
  `field.rs:25-62`; getter `identifiers()` returning `FixSpellings::over(self.get(IDENTIFIERS))`
  (the `nulls()`/`aliases()`/`branches()` shape, zero allocation); setter
  `set_identifiers()` on the `aliases` model (reject empty element, reject an
  embedded `,`, reject a folded duplicate); remover by empty input.
- Merge: add `IDENTIFIERS` to `MERGED_KEYS` (`field.rs:1364-1374`) — it is 9 today
  and becomes 10. Note the array length is in the type.
- Prose tables that are hand-maintained and must gain a row:
  `rust/src/fix/mod.rs:12-25`, `field.rs:1128-1142` (merge rules),
  `docs/fix/index.md:171-186`, `docs/fix/registry.md:805-812`,
  `docs/extensions/python.md:1361`, `docs/extensions/javascript.md:820-825`,
  `python/yggdryl/fix.py:5`.
- CLI: `cli/src/fix.rs:139` `conflicts_with_all` list and `cli/src/fix.rs:478-494`
  `dictionary_words` completion list.
- Generator: the families table beside `CODED_TAGS` in
  `scripts/generate_fix_dictionary.py`, the per-component derivation, and `--check`.
- Crate field: `altids` (65020, `AltIds`) in `crated.rs`; `CRATED` 20 → 21 in
  `python/tests/fix/test_fix.py:55`, `node/tests/fix/fix.test.js:1335-1339`
  (`schemaTags().slice(-21)` becomes `-22`), `rust/tests/fix/digest.rs:271-273,344`,
  `docs/fix/capture.md:366-385`, `.api-inventory.txt:700-725`, `.api-bindings.txt`.
- Fill: in `enrich`'s fill step, from the message's component's `fix:identifiers`,
  keyed by canonical field name, never overwriting a stated `altids`, a second pass
  equal. Group members are **not** flattened — say so in the decision.

## Bindings behaviour for a map column (already true, no work needed)

- Python: a `dict` both ways (`python/src/types/scalar.rs:1738-1760`, `2132-2180`).
- Node: a JavaScript `Map`, **not** a plain object (`node/src/types/value.rs:474-507`,
  `node/src/text/codec.rs:2186-2198`). A plain JS object encodes as a *record* and
  only reaches a map through the record→map coercion at `types/value.rs:476-487`.
  The Node fixture must use a `Map`.
- Snapshot: renders as a JSON object through `into_json_scalar`
  (`text/json/wire.rs:131-141`) in the mapping's own order; a non-string key would
  panic `equivalence.rs:190-203`. utf8 keys are safe.
