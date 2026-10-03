# Yggdryl agent contract

Arrow-native core in Rust (`rust/`), two native views - Python (`python/`) and
Node (`node/`) - and the `yggdryl` CLI (`cli/`). Rust owns `DataType`, `Field`,
`Scalar`, identifiers, I/O, codecs, shared enums; a binding redirects into it and
implements nothing of its own.

One direction, each layer settled before the next opens:

**Rust core -> Python -> Node -> docs.**

Two speeds of checking, and the fast one is the instruction:

- **Smoke while you build.** After every edit that changes behavior, run the
  smallest command that *executes* what was just written - one target, one
  filter, debug build, default features. Warm, that is under a second, and a
  defect it catches costs one edit; the same defect found after the bindings and
  the pages are written costs four, and a design flaw it was hiding costs the
  branch. [Smoke loop](#smoke-loop) is normative, and the rest of this file
  assumes it.
- **CI proves the change.** The exhaustive matrix - two feature lanes, the MSRV
  toolchain, seven exchange jobs against outside implementations, both pyarrow
  legs, a JVM, every documentation example in three languages - is
  `.github/workflows/ci.yml`'s work, never a local rehearsal of it. Push the
  branch and read the run. §2 says what each job proves, what CI never runs, and
  what to do with a red one.

A layer opens when the one below it smokes clean and its contract is settled,
not when a local sweep has rehearsed CI. Done means: smoke clean, pushed, CI
read and green, and the local-only checks in §2 run. Never report done over an
unrun smoke check, an unpushed branch, or a red or unread CI run. Report exact
results and exact skipped checks.

## Workflow

1. **Locate** the owning layer in [Layout](#layout); read its neighbours and the
   names in `.api-inventory.txt` (Rust) / `.api-bindings.txt` (Python, JS).
2. **Design against §1**: one owner per fact, one spelling per verb, no second
   schema, no second dispatcher - and the [Patterns](#patterns) the core already
   has for equivalences, the handle stack, row accessors, how outside data
   becomes a resolved type, and what is zero copy.
3. **Implement in Rust, smoking each step**: behavior, edges, errors,
   `rust/tests/`, `rust/benchmarks/`, rustdoc examples, both directions of any
   exchange format. Delete what it replaces in the same commit. The narrow test
   runs before the next edit, never after the layer.
4. **Settle the core** - smoke clean, cost pinned, contract decided - before a
   binding exists. Never pin an unsettled design by writing a binding first.
5. **Python** (§3): redirects, parity tests, boundary benchmarks.
6. **Node** (§4): the same.
7. **Docs** (§5): every layer touched, examples in all three languages, and
   the skill under `skills/` that teaches the surface.
8. **Push and read CI** (§2), then **handoff** (§5): sweeps, inventories,
   local-only checks, cleanup, report.

Rust-only is complete work when the core is the requested scope; a missing
binding is documented as Rust-only.

### Pace

The steps above are what a change is; what makes a wide one fast is how each
step is run, never which step is skipped.

- **One script per sweep.** A rename, a moved constructor or a changed
  signature that reaches every caller is one script of exact-string edits,
  each asserting that its anchor matches once, driven by the compiler's own
  list: the `it builds` row of the [smoke loop](#smoke-loop) with its two
  flags names every site the compiler has reached, and the script is re-run
  from that list until the check is clean. A site edited by hand is the slow
  way to the same text, and a regex with no exact anchor edits the site it
  did not mean.
- **One phase per change, each settled narrowly.** Several changes on one
  branch are phases in the order of the steps above - each core change, then
  the bindings, then the docs - and a phase is settled by its build check and
  the one suite it touched, never by the whole run. The theme is the one
  whose `src/` subtree the phase changes - `types` for `field.rs`, `fix` for
  `fix/` - so a change altering two subtrees is two phases; a test in another
  theme that a sweep only re-spelled is proven compiling by the build check
  and behaving by the whole run, and a surface with a cost pin adds its
  filtered `iobase_calls` or `allocations` row. A binding phase's build check
  is `cargo check --workspace --all-targets --keep-going
  --message-format=short`, which lists the bindings' broken call sites
  without building an extension or an addon, and its suite is one area of
  the §3 or §4 loop, run from the chain step that built what it needs. The
  docs phase is settled by `mkdocs build --strict`, seconds in the
  foreground; the example runner has no per-page filter, so it is a chain
  step per language after the step that built what it runs on.
- **One whole run, its pins re-pinned once.** The whole run - the
  `cargo test --all-targets` line of [Before you push](#before-you-push) in
  the `--all-features` lane, with `--no-fail-fast` so every failure is
  listed - leads the chain below, launched when the last phase that takes
  the cargo lock is settled, so every pin that moved surfaces in one pass
  and is re-pinned in one edit with the sentence that says why. A pin is
  re-pinned when the change accounts for its move exactly - one retired
  crate field is one tag fewer in the census, one re-spelled key is one
  dictionary hash - and a number the change does not account for is a defect
  found before the pin is touched; a cost pin - `iobase_calls`,
  `allocations`, a benchmark - is never re-pinned from that pass, because a
  moved count is a design answer ([Read the cost](#smoke-loop)). A re-pinned
  test is re-run by its own row of the smoke loop, never by the whole run
  again.
- **Disjoint files per worker.** A sweep that spans tests, bindings and docs
  is split across workers by file set, never by concern: the sets are a
  partition of the `it builds` list by path, so no two workers touch one
  file; a worker returns its script and the files it edited rather than a
  check of its own, and the one `cargo check` after the last worker returns
  is the next list. While workers edit, nobody runs `cargo fmt --all`,
  because a reformat moves the anchors their scripts match; the formatter
  runs once after the last worker returns.
- **Long runs in the background, one log, read once.** Anything over a minute -
  the whole run, clippy, a binding build, the example runner - is one chained
  script writing a log with a marker per step, and the foreground keeps
  editing: the docs phase, the inventories and the commit message are its
  work while the chain holds the cargo lock. Steps that take the lock are
  chained in that one script rather than launched side by side, because the
  workspace shares one target directory and parallel cargo invocations wait
  on each other. The chain's order is the layer order, each step reading
  what the one before wrote: the whole run, clippy, the rustdoc examples,
  §3's pre-push block, §4's, the two docs manifests, the example runner per
  language; the whole run leads because its pins are the foreground's next
  edit, and a clippy warning or a broken example is an edit that moves no
  pin.
- **One regeneration each, in dependency order.** The generated files, their
  tools, their triggers and their order are the table under [Before you
  push](#before-you-push); one regenerated before its input settles is
  regenerated twice. The dictionary and the crate dump are regenerated
  before the whole run, because the hash the run reports is the committed
  store's, the crate's own documents included.
- **The model fits the step.** A worker runs on the model its step needs,
  never the most capable one by default: a review, a verification or a check
  that reads and reports runs on the tier below the foreground's - `opus`
  where the foreground is `fable` - and a mechanical sweep on a cheaper one
  still. The most capable model is the foreground's alone, for the design
  and the edits no script makes.
- **One commit.** The phases land as one commit that says what the tree is,
  its pins and its regenerated files included, never a commit per phase: a
  phase alone is a tree that does not build.

### Common changes, in order

| Change | Touch, in this order |
| --- | --- |
| datatype variant | `<type>.rs` at the root, `DataTypeId`/`DataTypeKind`, parser, serde, comparison, Arrow, cast, `scalar` -> tests -> bindings -> `docs/types/` |
| logical name | `DataType::LOGICAL_NAMES` only; resolves to an existing datatype, adds no variant |
| codec | `<name>.rs` at the root (`load`, `dump`, `reader`, `writer`, `IOBase` wrapper) + a `Codec` variant -> bench -> bindings -> a section of `docs/media/compression.md` |
| string leaf | a `StringType` variant + `DataTypeId` appended + `string.rs` (spellings, the number rule, Arrow storage, grammar, value) + the charset's own root file - `utf8.rs`, `ascii.rs` or `cp1252.rs` - for the leaf's constructor, validation and reading -> tests -> bindings -> `docs/types/` |
| byte leaf | a `BytesType` variant + `DataTypeId` appended + `bytes.rs` (spellings, the number rule, Arrow storage, grammar, value) -> tests -> bindings -> `docs/types/` |
| charset | a row in `scripts/generate_charset_tables.py` + a regenerated `charset/tables.rs` + a `Charset` variant; a charset that gets string leaves is a root file of its own beside `utf8.rs`, `ascii.rs` and `cp1252.rs`, holding its codec and those leaves -> interop both directions -> bench -> bindings -> `docs/media/charsets.md` |
| storage backend | `<name>/` at the root with a location/container/leaf trio over the root traits - `<Name>Path`, `<Name>Folder`, `<Name>File` over a host tree; `<Name>Path`, `<Name>Node`, `<Name>Leaf` where the store has no tree to promise (`zip/`); state and assert its call/request counts -> interop script -> docs |
| media format | `<name>/` at the root, free functions over `IOBase` + a stateful wrapper, reached through `MediaType`/`RecordOptions` -> interop both directions -> `docs/media/<name>.md` - Overview, Read, Write - with its entry in the `Media` nav and its row in the overview's table |
| metadata property | a protocol view keyed `<SCHEME>:<property>`, the scheme upper case; never a new `Field` accessor |
| binding method | core method first; the binding only infers, coerces, redirects - plus a parity test, a boundary benchmark, a docs entry |
| Arrow collection verb | a `Serie` verb in `serie.rs` or `serie/arrow.rs`, a `ChunkedSerie` verb in `chunked_serie.rs`, or a node or rule of `ArrowCastPlan` in `cast.rs` - never a function in `arrow/`, a method on `DataTypeValue`, `FieldValue` or `Field`, or a helper in the calling module -> `rust/tests/serie/<file>.rs`, `rust/tests/root/chunked_serie.rs` or `rust/tests/root/cast.rs`, plus an `allocations` row at two corpus sizes -> both bindings' `Serie`, `ChunkedSerie`, `SerieReader` or `ArrowCastPlan` -> `docs/types/serie.md`, `docs/types/chunked-serie.md` or `docs/types/cast.md` |
| a surface reading Arrow rows | one `SerieReader` per stream, or one landing per batch with the root resolved once per reader; casts through one `ArrowCastPlan` compiled before the loop; each column's storage proven once - `as_<leaf>`, or, for a text or byte column that takes any of its layouts, the crate-private `Serie::is_string_storage`/`is_byte_storage` - and read per row through that leaf's accessors and `value`, or the crate-private `Serie::value_bytes` -> a pin that the per-row path allocates only what it hands back -> the surface's bench |

## Smoke loop

A smoke check is the smallest command that *executes* what was just written: one
target, one filter, debug build, default features. It is not a reduced gate and
it proves nothing about the change as a whole - it is the feedback that keeps a
defect one edit away from its cause, and it is what makes §2 a formality instead
of a discovery. Run the narrowest loop that can fail, and widen only when it
passes.

| Loop | Command | Answers |
| --- | --- | --- |
| it builds | `cargo check -p yggdryl --all-targets`, plus `--keep-going --message-format=short` when a change reaches every caller | types and borrows, and every test and benchmark still compiling against the changed signature; with the two flags, one line per diagnostic across every target rather than a stop at the first failing one, re-run because the compiler reports the errors of the phase it reached |
| it behaves | `cargo test -p yggdryl --test <entry> <filter>` | the `rust/tests/<entry>.rs` harness over the source entry touched - the folder's own name, or `root` for a file the crate root holds ([Where a test lives](#where-a-test-lives)) |
| a private pin holds | the same loop plus `--features internals` | the `internal` module of the mirrored file, which is where what no caller can name is pinned |
| the published example runs | `cargo test -p yggdryl --doc <path::to::item>` | the rustdoc example on the item, which is also what a docs page shows |
| it still costs what it claims | `cargo test -p yggdryl --test iobase_calls <filter>` / `--test allocations` | the pinned `IOBase` call counts and allocation claims for that surface |
| it got faster or slower | `cargo bench -p yggdryl --bench <name> -- <filter> --quick` | direction only; a number a page states comes from the release run |
| a gated path works | the loop above plus `--features "parquet iceberg"` or `--features s3` | only when the change is under that gate |
| the Python view redirects | `VIRTUAL_ENV=python/.venv python/.venv/bin/python -m maturin develop -m python/Cargo.toml`, then the same interpreter's `-m pytest python/tests/<file> -x -q` | the binding against the core it redirects to, with no wheel built; maturin finds the environment it installs into by `VIRTUAL_ENV`, `CONDA_PREFIX` or a `.venv` in the working folder or above it, never by the interpreter running it, so the root needs the variable |
| the Node view redirects | `npm run --prefix node build:debug`, then `node --test node/tests/<file>.test.js` | the same, with no package audit |
| the inventories are not stale | `python scripts/check_api_inventory.py` | every section header names a file or folder that exists; a Rust name still occurs somewhere in that crate's `src/`, and so does every type the signature beside it names; a binding entry's dotted key still resolves through the tree its section names - each segment a module beside its parent or a name that parent binds. What is omitted is counted - source files with no section, `pub` names the inventory never spells - never failed |
| a page example runs | `python scripts/check_docs_examples.py --lang rust`, or `python`, or `javascript` | every block in that language under `docs/` and `skills/` - there is no per-page filter, so this is a pre-push check, not a loop |
| the installed wheel works | `python scripts/check_wheel_smoke.py` | what `pip install yggdryl` gives a reader: the extension loads and an Iceberg table round-trips. It reads `yggdryl` from the environment, never `python/yggdryl`, so install a wheel (or `maturin develop`) first - the release runs it in every CPython a `smoke: true` build row lists, against the wheels of manylinux and musllinux on x86_64 and aarch64, macOS x86_64 (under Rosetta 2) and arm64, and Windows x64; it smokes neither the Windows arm64 wheel, whose platform PyArrow publishes no wheel for, nor the two musllinux CPython 3.10 wheels, which uv's musl CPython 3.10 cannot import and whose extension the release reads for initial-exec thread-locals instead |

The measured costs that shape the loop: an already-built harness is under a
second (`--test root` is 946 tests in 0.6s), the first build of a
target is about a minute and a half, re-checking the crate after an edit is
about thirty seconds, `--all-targets` costs roughly ten seconds more than
`--lib` and is worth it because it is the only build with any test in it at
all - and it catches a test or benchmark left behind by a changed signature -
and `--test iobase_calls` unfiltered is half a minute, so filter it to the
surface touched.

Three habits are what make the loop pay:

- **Smoke the refusal first.** Write the refusal, the edge, or the pinned count
  before the happy path and run it while it can still fail. A check that has
  never been red has never been a check: `rust/tests/interop/` shipped reading
  tests that printed `SKIPPED`, reported `ok`, and had never once run under CI
  until a job existed to write the input they read.
- **Read the cost, not just the color.** `iobase_calls`, `allocations`, and a
  `--quick` bench are the fast way to see a per-row parse, a re-open, a stray
  clone, or a materialized stream while the change is still small enough to
  restructure. A count that moved is a design answer, not a number to re-pin -
  §1's cost rules say which direction it was allowed to move.
- **Widen on a signal, not on a schedule.** The gated features, the whole theme,
  the other binding, the docs pass: each earns its turn by the narrower one
  passing. Rehearsing CI locally costs tens of minutes and proves what the push
  proves in parallel for free.

`cargo test --all-targets` executes every Criterion target at its smoke corpus,
because `benchmarks/bench_profile.rs::corpus` selects small fixtures in a debug
build. Benchmarks therefore compile and run in the ordinary loop, and only
`cargo bench` measures.

## Always

**No back-compat.** One current contract. A change deletes the replaced symbol,
parser spelling, serialized shape, fallback branch, test, and doc in the same
change, and updates every caller. Never: deprecated alias, shim, migration
reader/writer, dual behavior, warning period, legacy branch. External standards
hold only where the contract names standard and version - describe them by
protocol/version, never as a compatibility layer.

**Simple code.** Direct control flow, one source of truth, existing generic
traits/types, minimal state. Abstract only to remove real duplication or enforce
an invariant; a value's behavior is a method on it, not a helper. Delete dead
branches and redundant wrappers you touch. No speculative generality, no
binding-side core logic. Comments carry non-obvious constraints, ownership,
bounds, safety - never prose translation.

**Wide in, typed through.** Data from outside - a user argument, a wire byte, a
host runtime object - meets one boundary that accepts every documented spelling
of the thing it is, resolves it exactly once into `DataType`, `Field`, `Scalar`,
or the owning enum, and hands the interior a value whose type is already proven.
Flexibility belongs to intake, never to meaning: accept more spellings, never
pick between two readings - ambiguous, disagreeing, or unrepresentable input
fails with expected, actual, and location rather than widening to fit. Past that
boundary nothing re-infers, re-parses, re-validates, or branches on a string:
work is planned once against the resolved type, and the per-item path moves
bytes under it. The interior is fast because the edge was exact, never because
it skipped a check. Every type reads its own text in its own root file, one
reader each - `boolean.rs` a flag (`bool_from_text`), `integer.rs` a count,
`floating.rs` a float, `decimal.rs` a decimal, `duration.rs` a lifetime,
`uuid.rs` a UUID - and every setting, cell, wire value and binding coercion
reads through it, so a spelling one door takes no other refuses.

**Best effort, then a named refusal.** An expression, a cast, a read or a write
does what the text asks whenever one reading does it, and refuses only what no
reading can, naming the column, the value, or the section it could not honour.
A constant coerces into the operand it meets, operands with no common type
compare as text, a column casts safely unless it is `not null`, an
empty text cell entering a non-text column is null and the column's
nullability is what may refuse it, a missing store reads as the empty stream,
a `select *` or a same-type cast costs nothing. Never a technical error a
caller has to work around by hand when the intent is unambiguous; never a
silent widening when it is not. DuckDB's SQL and Python expression API are
the reference for what a spelling should mean when engines differ - its
column, star-exclude, alias, cast, `isin`, `between`, `isnull`,
`when`/`otherwise` and ordering vocabulary keep their names here - and the
crate's own abstractions (`DataType`, `Field`, `Scalar`, `Selector`,
`Filter`, `Plan`, `Holder`) carry the behaviour; nothing is a second engine.

**Defaults in the signature, absence skipped.** Every optional argument
carries its default in the signature. An argument that was not given is `...`
in Python and `undefined` in JavaScript and is skipped; `None` and `null` are
values, and clear. Every record read and write takes `options` and, beside it,
the option properties by name - `**properties` in Python, a plain object in
JavaScript - each set on a copy of the options by its own setter, so
`read_arrow_reader(rowheader=...)` and `readArrowReader({ rowheader })` read
with the handle's own options carrying that property. The inputs of the
expression layer cross as one `Scalar` (`Scalar.from_`, `Scalar.from`; a
columnar object lands as the serie `Scalar` its `Serie` is, sharing its
buffers - a stream is drained into one, never held as a `Scalar`) and the core
reads them with `from_scalar` - a binding never re-implements a parser, and the
function set stays closed: `namespace.name(...)` is a registered user function,
typed and called through its signature field, opaque to pushdown.

**Compressed output.** Only what changes a decision, proves a result, names a
blocker, or enables the next action; each fact once; outcome first (state,
evidence, next action). No greeting, praise, throat-clearing, repeated context,
narrated tool use, reassurance, sign-off, restated request. Progress at start,
material change, blocker, check or CI result. Handoff keys: `Goal`, `Invariants`,
`State`, `Checks` (command + exact result), `Blockers`, `Next` (exact command);
omit empty, resolved, stale. Brevity never drops a contract, safety boundary,
error semantic, edge case, verification result, or material uncertainty.

# 1. Rust core

Invariants under every section below: one owner per fact and one spelling per
verb; no second schema, dispatcher, parser, or storage trait; nothing streamable
materializes, held state names its bound and reason; errors are typed and
located, mutations fail atomically; every surface you touch gets tests, a
benchmark, and a docs entry, with cost asserted in `IOBase` call counts,
allocation claims in the counting allocator, and exchange formats checked both
directions against an outside implementation.

## Layout

Every member has `src/`, `tests/`, `benchmarks/`; root owns pins and lints with
`default-members = ["rust"]`; features are `default = []`, `parquet`,
`iceberg` (implies `parquet`), `http`, `http2` (implies `http`), `http3`
(implies `http2`), `aws` (implies `http`), `s3` (implies `aws`). Examples live in docs - no `examples/`
dir. The crate is flat: every type and every shared trait, enum or value is a
root file, and `value/` - the contracts a datatype, a field and a value each
owe the root that holds them - is the one folder among them; every
implementation - a medium, a codec, a storage backend, a digest, a charset
with string leaves - is a root file or folder of its own name; a parent
folder (`media/`, `text/`, `coding/`, `holder/`, `hashing/`, `charset/`)
holds only what its implementations share. A folder is never a
facade over root-owned vocabulary, and a module owns implementation rather
than an empty facade. A binding's `src/` is flat the same way and for the same
reason: `python/src/datatype.rs`, `field.rs`, `scalar.rs`, `cast.rs` and the
rest hold one type each at the crate root, `avro.rs` and `iceberg.rs` are
implementations of their own name, and `media/` keeps only the handle classes
and the partition renderer every medium shares - there is no `types/` or
`media/` facade over vocabulary the root owns. The caller-facing packages -
`python/yggdryl/` and the Node JavaScript files - are laid out the same way,
and so are the tests: `rust/tests/` mirrors `rust/src/` file for file
([Where a test lives](#where-a-test-lives)), and `python/tests/` and
`node/tests/` mirror their own sources the same way. Benchmarks and docs are
grouped by theme - `types`, `holder`, `media` and the rest - which is a
caller's vocabulary rather than a source path.

Paths below are under `rust/src/` unless stated otherwise.

| Path | Owns |
| --- | --- |
| `<name>.rs` | one shared trait, enum, value or type each, re-exported from the crate root; a type file holds its datatype, its field and its scalar in that order ([One type, one file](#one-type-one-file)) |
| `iobase.rs` + `iobase/` | the single `IOBase` trait and its behavior modules; `iopath.rs`, `iofolder.rs`, `iofile.rs` the three roles every storage backend implements, `iocursor.rs` the one retained position, `iomedia.rs` the record operations derived from the byte trait, `iokind.rs` and `iomode.rs` their vocabulary |
| `datatype.rs` | `DataType`, the shared logical datatype enum and its cross-family value contract; `datatype_id.rs` and `datatype_kind.rs` the exact-variant and family enums, a family being the range of identifier bytes it owns (`DataTypeKind::range`, `contains`), the identifier the one owner of the Arrow extension name its datatype rides (`DataTypeId::arrow_extension_name`, the thirty `arrow_extension_names`), `parser.rs` the canonical display and the Arrow, SQL, Hive, Spark and Iceberg parsing, `serde.rs` the structural document, `compatibility.rs` the concrete targets, `vocabulary.rs` the logical names, `default.rs` the canonical defaults, `diff.rs` schema equality and its differences, `merge.rs` the one place two schemas become one |
| `field.rs` | `Field`, one variant per `DataType` shape, each carrying name, nullability, metadata and the Arrow projection cache; `metadata.rs` + `metadata/` the `<SCHEME>:<property>` map and its validation, `protocol.rs` the borrowed protocol views |
| `scalar.rs` | `Scalar`, the one value every part of the project speaks; `arithmetic.rs` checked arithmetic over exact natives, `path.rs` the one allocation-free value path every recursive walk uses, `pretty.rs` the indented rendering of a schema |
| `value/` | what a datatype, a field and a value each owe the root that holds them, and what the leaves of one family share: the `Value` contract, `DataTypeValue` (its `kind` the family whose range its `id` is in), `FieldValue`, `FieldSidecar` and the payload datatypes `GeometryType`, `GeographyType`, `UnionType`, `RunEndType`, the leaf contracts `IntegerValue`, `FloatingValue`, `DecimalValue`, `TemporalValue`, `GeospatialValue`, `CodeValue`, `EnumValue` and `NestedValue` - declared here, implemented beside each leaf - and `SerieValue`, what every column leaf of `Serie` owes, its `id` and `kind` its field's, with `Children::Column`/`ColumnRows` walking a column's rows as `Cow`; a family is no type - it is the `DataTypeId` byte range its `DataTypeKind` owns, and a value is its leaf; `canonical.rs` the schema-directed validation and canonicalization of row values. The module is private, and every name is `yggdryl::<Name>` at the crate root |
| `typed.rs` | the typed markers and the field-borrowing values: `TypedField<K>`, `FieldScalar<'_>`, `UncheckedFieldScalar<'_>`, `FieldRecord<'_>` and the prebuilt shared fields |
| `cast.rs` | the one recursive cast engine, and it is `Serie`'s: the crate-private `ArrayCastPlan` node tree - exact, bit, kernel, byte bridge, the ingest and render kinds, and the nested and encoded arms - with the kernels it calls; the public `ArrowCastPlan`, one `Field`-to-`Field` cast compiled once, applied to a `Serie` - or, by `apply_chunked`, to every chunk of a `ChunkedSerie` - certifying per node which leaves it proved, with a transport face (`reconcile_batch`, `reconcile_array`) for batches that are only moved; `ArrowCastOptions`; and the crate's `PlanCache`, one plan per distinct source schema for a loop whose batches can change schema. `budget.rs` holds the bounded scratch and output reservations it draws on. Nothing reaches the engine except `Serie::cast`, the `Serie` and `ChunkedSerie` Arrow doors, `ChunkedSerie::cast`, `SerieReader`, a held `ArrowCastPlan`, and the crate's stage plan (the write session's shaping) |
| `integer.rs`, `floating.rs`, `decimal.rs`, `boolean.rs`, `bytes.rs`, `uuid.rs`, `geospatial.rs`, `enums.rs`, `structure.rs`, `mapping.rs`, `union.rs`, `runend.rs`, `version.rs` | one family per file, each the whole of its datatype, field and scalar; `decimal.rs` holds the four parameterized widths and, beside them, the two fixed leaves at scale eighteen - `Decimal` over `decimal128(38, 18)` and `BigDecimal` over `decimal256(76, 18)`, each a datatype, field and scalar of its own under its `yggdryl.` extension name; `int256.rs` holds the `i256`/`u256` pair the exact decimals compute in, the one type file not named for its type because a module and a struct share one namespace at the root; `wkb.rs` the Well-Known Binary reader three types need; `regex.rs` the Struct inference from named captures |
| `serie.rs` + `serie/` | `Serie`, the fourth side of the value model: many values, as a schema-free `Run` or as a column - the Arrow buffers of one `Field`, holding no `Scalar`, nested as `Serie` children all the way down - with the collection verbs (`scalar`, `get`, `rows`, `iter`, `slice`, `splice` and the writes spelled over it: `set`, `push`, `insert`, `remove`, `pop`, `truncate`, `clear`, `extend`, `extend_from_serie`, `resize`, `set_child`, `set_cell`), identity over the rows alone, serde, a flat root of one variant per storage layout named as the leaf that holds it (`Utf8String`, `DurationMillisecond`, `IntervalDayTime`) - a variant names the layout, never the datatype, because one layout serves several (`Utf8String` every string leaf laid out as UTF-8, `DurationSecond` both widths), so a column's datatype is its field's, `SerieValue::id`; the five serie layouts each serve one datatype and are named as it is - `Serie`, `SerieView`, `FixedSizeSerie`, `LargeSerie`, `LargeSerieView` - each holding its `<Variant>Serie` leaf (`SerieSerie`, `SerieViewSerie`, `FixedSizeSerieSerie`, `LargeSerieSerie`, `LargeSerieViewSerie`, over the `OffsetSerie<O>`/`OffsetViewSerie<O>` shapes whose offset width is an `OffsetLeaf`), so `Serie::as_serie` narrows a column to `SerieSerie` where `Scalar::as_serie` borrows the whole `Serie` a value holds; the codes, `Version`, `Url`, `Urn`, `Timezone`, `MimeType`, `MediaType`, `Uuid`, `Geometry`, `Geography` and `SortedMap` keep variants of their own - and the `as_<leaf>`/`get_<leaf>_mut` narrowings - a constant column narrowed through the leaf it lays out as, `as_lit`/`get_lit_mut` the constant itself - and, crate-private, the `is_string_storage`/`is_byte_storage` predicates the text body readers and the FIX payload guard on, and the `value_bytes` those readers and the digest feed - which matches the same storage variants by name - read per row; `serie/datatype.rs` is the family's datatype and field - `SerieType`, the five serie layouts over one item field, and `SerieField` - and `Run`, the schema-free ordered run a row canonicalizes to: a window - a start and a length - over one shared `Arc<[Scalar]>`, one allocation to build, so a slice of a run shares its values and a slice of a slice reaches them with the offsets summed, and the one leaf of `Serie` that declares no field; `serie/` otherwise holds `order.rs` - the ordering, uniqueness, selection and grouping verbs (`sort_indices`, `is_sorted`, `is_unique`, `unique_count`, `partition_by`, `window_by`, `memory_size`, the `into_<state>`/`as_<state>` pairs) over one comparison ladder - Arrow's comparator over buffers that order as their values, a record compared child by child with each child on its own rung, and the values' own order - with the crate-private `window_starts` every `window_by` cuts by: one comparator and one bit per row, the first descent read in the same pass, and, asked to regroup, the runs sorted stably by key - and one leaf per Arrow layout - `primitive.rs`, `boolean.rs`, `null.rs`, `bytes.rs` with its `string.rs` aliases, `structure.rs`, `sequence.rs` (the five serie layouts), `mapping.rs`, `variant.rs`, `enums.rs`, `runend.rs`, `union.rs` - each lending its buffers and writing them in place through prove-check-write - a primitive, boolean or byte leaf reading its own typed buffer through the reading its field resolved where it landed, and a string or byte leaf's `value(i)` lending a run's bytes where they lie across offsets, views and fixed widths with no value built - `lit.rs` `LitSerie`, the constant column no Arrow layout is: one field, one value proven by `Field::scalar`, a length, the one row it lays out as, and the whole array built on the first export and shared by every later one and every slice of it (`value`, `row`, `is_built`, `array`), what `Serie::lit` and `Serie::from_default` answer - a cell read clones the value, `slice`, `into_taken`, `into_filtered`, `into_reversed` and the ordering questions move or read the count alone, a write of the value or a removal moves the count, a write of another value lays the column out as its field's leaf first, a spill forgets the built array and writes nothing, a cast through a plan casts the one row, and serde and the digest read it as the rows it is - `layout.rs` the buffer edits they share, `value.rs` the one codec between a row and an Arrow slot (named by nothing outside `serie/`, `cast.rs` and `temporal.rs`), and `arrow.rs` the one door buffers take in and out (`from_arrow_array`, `from_scalars`, `empty`, `with_capacity`, the batch and reader pairs, `SerieReader::from_serie` and `SerieReader::from_chunked`) - and the stream's windows: `SerieReader::window_by(by, sorted)` answering `SerieReaderWindows`, one lazy `SerieReader` per window of equal adjacent keys pulled through one walk holding at most one batch, its key record and one bit per row of it, windows read in order (a window its walk passed refuses once, naming it), `sorted` verifying the keys arrive in order and refusing the first going backwards by batch and row, `field` and `static_field` known before a pull; `SerieReader::static_values` the window's record - the windowed reader's own cells but `windownum` and `rownum`, the key cells, `windownum`, and `rownum` absolute in the stream - kept by `cast`, dropped at `into_arrow_reader`, and stated by nothing but a window - proving the layout against the field's projection, absence on the validity words, and - only where the layout is not the datatype's whole contract (`DataType::layout_is_contract`) - each row once, a refusal naming the landed row and the path below it (`$[3].alive[0].miccode`). `SerieValue`, the contract every column leaf owes, is in `value/` |
| `chunked_serie.rs` | `ChunkedSerie`: many `Serie` columns under one field, held apart - the chunked array and the table - each chunk a column of exactly that field, proven at its own door, with the chunk ends kept beside the chunks so a row is a binary search and the length a read; the row verbs read across the chunks, a child is the child of every chunk, identity is the rows alone; `from_arrow_arrays`/`into_arrow_arrays` cross a chunked array one array per chunk, `from_arrow_reader`/`into_arrow_reader` a table one batch per chunk, `cast` is one plan over every chunk and `into_serie` the one join; `SerieReader::from_chunked`, beside `from_serie` in `serie/arrow.rs`, streams its chunks; the ordering verbs `Serie` answers, across the chunks - `is_sorted` reading each chunk and every chunk edge with no join, `into_reversed`, `into_filtered` and `partition_by` chunk by chunk and kept apart, `sort_indices`, `is_unique`, `unique_count`, `into_sorted`, `into_unique` and `into_taken` the one join then the verb, `memory_size` the chunks summed, and the `as_*` writes replacing the chunks in place; `window_by(by, sorted)` the windows `Serie::window_by` cuts the joined column into, as `Vec<(Scalar, ChunkedSerie)>` of zero-copy chunk pieces - a run crossing a chunk edge one window, the edge compared in place against the pending key so a continuing run builds no key, `sorted` regrouping runs and never rows, nothing joined - and stating no record, the key the pair's first half and the place its index, so a key cell named `windownum` or `rownum` is taken; `into_sorted`, `into_sort_by` and `into_unique` join nothing: each chunk sorted on its own and the sorted chunks merged - one cursor per chunk in a heap, a tie to the earlier chunk, blocks of one output batch's worth shared among the cursors, the output cut into chunks of at most `DEFAULT_RECORD_BATCH_ROW_SIZE` rows, each gathered by one `interleave`, landed proven and settled - the rows copied twice and never a third time, uniqueness the same merge over each chunk's order marking first occurrences in its chunk's mask and filtering each chunk apart; `sort_indices`, `sort_indices_by`, `is_unique`, `unique_count` and `into_taken` stay the one join, their answer being one column; a sorted result's field and every chunk declare the order (`declared_order`), chunks already declaring at least it are merged unsorted, and every chunk edge is verified where chunks arrive under a declaring field (`from_series`, `from_arrow_arrays`, `cast`, `push_chunk`); `joined` is `into_serie` before it settles |
| `serie_source.rs` | `SerieSource`, the one intake every verb that reads rows from a caller takes - `Serie(Serie)`, `Chunked(ChunkedSerie)`, `Reader(SerieReader)`, `From` each, so a caller names no variant - with `root` (a record column's own, any other column the one child of a `row` record, a stream's own; a run refused), `is_held`, `memory_size` (`None` for a stream) and `into_reader`, the rows as the stream of the batches they already are - a held column one record batch, a chunk one each, a stream itself, nothing cast, copied or read; what `IOMedia::write_serie` and its three intents and `SerieReader::join_with` take, and the plan's join sides |
| `window_serie.rs` | `WindowSerie<'a>` and `WindowSerieMut<'a>`: a window over a serie - a reference, an offset, a length and, on a window `window_by` lent, where its record comes from: forty bytes and `Copy` - that reads and writes through the serie's own implementation, every index window-relative, bounds-checked against the window and rebased by the offset before the serie answers, so a leaf's `value(i)` under it is one bounds check and one buffer read; `Serie::window`/`window_mut` the doors; the reads of a `Serie` and its ordering verbs over the window, `into_serie` being `Serie::slice`; the writes `set`, `fill`, `swap`, `copy_from`, `splice`, `as_sorted`, `as_reversed`, `as_taken`, each `Serie::splice` or `Serie::set` on the rebased range, a window never growing or shrinking what it views (`as_unique` and `as_filtered` are not offered); identity the window's rows alone, `WindowSerieRows` its iterator; `as_window` the borrowed view of a mutable window; `SerieWindows`, what `window_by` answers on a `Serie`, a `WindowSerie` and a `WindowSerieMut` - the owner of the windows, holding the serie they view (borrowed, or the one copy `sorted` gathered where the keys descend), the key column, where each window opens and the record every window states, typed once - with `len`, `is_empty`, `serie`, `static_field`, `iter` and `into_owned` (a borrowed serie cloned, buffers shared, so the windows outlive the borrow), `SerieWindowsIter` its exact-size, fused walk whose `nth` builds no key it skips; `WindowSerie::static_values` the record a lent window states - the kept cells of the window windowed, the key cells, `windownum: uint64` required and `rownum: uint64` nullable, null after a gather - a `FieldScalar` built only when read, never identity, never carried by `into_serie`, and stated by no narrower window, `Serie::window` or `WindowSerieMut`; the crate-private `WindowRecord` the stream's walk types its record by |
| `sort_options.rs` | `SortOptions`, the two facts an ordering states beside its key - `descending`, `nulls_first` - `Copy`, `Default` ascending with nulls last as the plan's `Ordering` and DuckDB's `order by` default - the opposite of Arrow's own default, which puts nulls first - `ascending()`, `descending()`, `with_nulls_first`, `is_descending`, `is_nulls_first`, `Display` the suffix the plan's ordering writes after its key (` desc nulls first`, empty for the default) and `FromStr` of it, the crate-private `into_arrow` what the Arrow kernels read; what `Serie`, `ChunkedSerie` and `WindowSerie` sort by, and what the expression `Ordering` carries |
| `spill.rs` | `SpillOptions`, the bound a column stays resident under - `byte_size`, `NEVER`, the folder its files are created in - `DEFAULT_SPILL_BYTE_SIZE` (64 MiB), `from_env` the process default read once from `YGGDRYL_SPILL_BYTE_SIZE`/`YGGDRYL_SPILL_FOLDER` and `install_env` stating it before anything reads it; crate-private, `Backing` (the flag every leaf carries, `Heap` or `Mapped`), `Mapping` and `spill_array`, the walk that writes every buffer of an array 64-byte aligned to one private unlinked file `yggdryl-spill-<pid>-<seq>` and rebuilds the array over the mapping through `Buffer::from_custom_allocation`, the one `unsafe` beside `local/file.rs`'s - the mapping holding the pages and never the descriptor, which closes once the file is mapped, so the live spilled units of a process are bounded by disk and not by its descriptor limit. `serie/spill.rs` holds the verbs: `Serie::resident_size` (what `memory_size` counts that is still on the heap), `is_spilled` (no byte resident and some bytes, read in that order), `spill(&SpillOptions)` - greedy: a run left alone, a column under the bound untouched, a flat column or one whose own buffers alone pass the bound spilled whole, any other child by child heaviest first under what the bound leaves - `as_spilled` (the spill, answering `&mut Self` so calls chain) and `into_spilled` (a copy spilled, this one untouched, the buffers the bound leaves resident shared), and the crate-private `settled`/`settle`, what every door that lays a column out answers through (`from_scalars`, a casting `from_arrow_array`/`from_arrow_batch`, `ArrowCastPlan::apply`, `taken`/`filtered` and so the sorts, `extend_from_serie`, `ChunkedSerie::into_serie` over several chunks, `push_chunk`, the merge, a join); a door sharing a caller's buffers - an exact landing, `slice`, `window`, a `SerieReader` batch in flight - and a row write never settle; a write through a mapped leaf sets it back to `Heap`. `SerieValue` carries `memory_size`, `resident_size`, `is_spilled`, `spill`; `ChunkedSerie::spill` spills the heaviest chunks whole first and keeps its resident total beside the chunks, so `resident_size` is a read, `WindowSerie` reads through its serie, `SerieReader` spills the records it holds; `ChunkedSerie` and `SerieReader` answer `as_spilled`/`into_spilled` too, the reader's `into_spilled` consuming it. A write cadence held between publications - a leaf's, a write session's, an Iceberg table's partition holds - is held under the process bound, its heaviest batches spilled first (`media/options/commit.rs`' `CommitBuffer`, crate-private) |
| `join.rs` | the hash join engine: `JoinKind` (`Inner`, `Left`, `Right`, `Full`, `Semi`, `Anti`; `keeps_unmatched_left`, `keeps_unmatched_right`, `emits_right`, `is_filtering`), `JoinSide`, `JoinOptions` (`coalesce`, `suffix` `_right`, `build` - `None` the held side over a stream, else the smaller by `memory_size`, else `Right` - `prune`, `spill` - `None` the process default - `pushdown_keys` 10,000), `DEFAULT_JOIN_SUFFIX`, `DEFAULT_PUSHDOWN_KEYS`; crate-private, `JoinPlan::compile` (every key bound once per side and cast to `common_type`, the output root the left columns then the right - a `using` key once where coalesced, a collision suffixed - named after the left, metadata cleared; `KeyRung::Buffers` where every key's buffers order as its values, `Values` otherwise), `BuildTable` (the held side's chunks settled; `Chains` - one node per present build row chained under its key's hash, one `GroupHead` per distinct key, so a build of unique keys allocates nothing per row; `KeyRange`, the least and greatest build key rows, which `excludes` prunes a probe batch by without a hash; `KeyTable::Sorted` where both sides declare the key order ascending with no key cast - the build keys in their order and one cursor the probe rows walk them with, the merge path, no table and no partition; the `matched` bits where the kind needs them), `JoinOutput` (the probe read batch by batch, pairs cut at `DEFAULT_RECORD_BATCH_ROW_SIZE`, `lay_out` one take of the probe, one interleave of the build and one record per batch, `one_sided` a pruned or unmatched batch as slices of itself, `drain_build` the build's unmatched rows last; `Grace` where the build passes the spill bound on the `Buffers` rung: both sides scattered by `xxh3(row) % P`, `P = ceil(build_bytes / bound).next_power_of_two().clamp(2, 64)`, into `Spool`s spilled at a zero bound, joined partition by partition, the same rows in partition order), `pushes_down` and `pushdown_term` (the probe key `in` the build's distinct keys for a one-key join whose kind emits no unmatched probe row, under `pushdown_keys`) and `isin_term`. `serie/join.rs` holds the three verbs: `Serie::join_with` (the output batches joined once, unsettled, then settled under the join's own options), `ChunkedSerie::join_with` (the batches kept apart), `SerieReader::join_with` (its other side any `SerieSource`; the held side - of two streams, the right one - built before a pull, each probe batch answering its own output lazily through `Source::Lazy`). `expression/join.rs` is the key grammar: `JoinKey` (`left`, `right`, `using`, `from_term` splitting one `=`), `JoinKeys`, `IntoJoinKeys` (texts, lists, `(Term, Term)` pairs, `(Selector, Selector)`, a `Scalar`); the plan's `join` clause (`expression/plan.rs` `Join`, `Plan::join`, `joins`) runs the engine over two targets, the first join's key filter pushed into the probe's read |
| `code.rs` | the contract every registered code answers - the trait and the two builders; the twelve codes are one file each, named as the code is: `ccy.rs`, `country.rs`, `mic.rs`, `cfi.rs`, `isin.rs`, `cusip.rs`, `sedol.rs`, `bbg.rs`, `figi.rs`, `ric.rs`, `forex.rs`, `unit.rs`. A code's `new` holds its shape - width, character classes, case folded where the code folds - and refuses nothing else; how real a value is - a check digit that closes (`Isin::is_closed`, a listed agency prefix `is_listed_prefix`), a listed country, a classified CFI, a currency or MIC other than its none code - is its rank (`CodeValue::{MAX_RANK, rank, is_real}`, `IdType::rank`), which a merge reads so a real value replaces a placeholder whatever the order and two of one rank keep the order's rule; `Isin::NONE` (`XX0000000000`) is the book key of an element stating no instrument, and `is_canonical` is the strict upper-case and shape gate a cast reads |
| `forex.rs` | the `forex` code - `Forex`, one ISO 4217 currency pair stored as the canonical seven-byte `CCY/CCY`, the `FOREX` security identifier key - and, Rust-only, the FX symbol detector: `FxSymbol::from_symbol` reads a venue's symbol into its pair, its `FxTenor` and the `SettlType(63)` the tenor spells, allocation-free, which is how a FIX parse detects the pair off `Symbol(55)` |
| `state.rs` | one of the `enum` family's five leaves, each written by the `enum_leaf!` macro in `enums.rs` from its member table and answering `EnumValue`: `State`, sixty-one lifecycle members stored as the `uint16` code of each - the hundreds of the code its rank, so the stored integers sort from asked for to ended, `UPDATED` (3004) a stated `NEW` over a live new-like predecessor - with `StateType`/`StateField`, `Scalar::State`, the `yggdryl.state` extension over Arrow `UInt16`, the five spelling vocabularies, the FIX status tables `from_fix_status` reads and the message types `from_fix_msgtype` answers, and the one ingest a text or integer column lands through; the member table is the FIX dictionary's intrinsic `statecodeset` |
| `marketdatakind.rs` | the `enum` family's `MarketDataKind`: FIX's MsgCat code set, twenty-six members - `UNKN` (0) to `TRAD` (21), `BOOK` 3, `EXEC` 8, `ORDR` 10, `QUOT` 14, then the four batch categories `ORDB` 22, `QUOB` 23, `EXEB` 24 and `TRDB` 25 - stored as the `uint8` code under `yggdryl.marketdatakind`, the four-letter code its stored name; the one owner of that set - the `marketdatakind` column a `marketdata` row opens with, what `FixMsg::msgcat` answers and the intrinsic `msgcatcodeset` the crate renders from it - and of the sided rule: `is_sided` (`ORDR` and `EXEC` alone state their side in their stored cross code `{kind}:{side}:{base}`, every other kind - a quote holding both its legs included - a `0` there; the kind's `code` is that prefix's first number), of the book rule `is_booked` (`ORDR`, `QUOT` and `BOOK` fold into a book, every other kind is pruned before a book walk routes it), with `is_batch` and `item`, the single category one entry of a batch is filed under |
| `marketdatatype.rs` | the `enum` family's `MarketDataType`: what type of its kind an element is, one generic set over the eight FIX code sets that type one - `OrdType(40)` as the order types `ORD*` (1xx), `QuoteType(537)` the quote types `QUO*` (2xx), `TrdType(828)` the trade types `TRD*` (3xx), `MDEntryType(269)` the book entry types `BOOK*` (4xx), `TradeReportType(856)` the trade report types `TRPT*` (5xx), `QuoteRequestType(303)` `QRQ*` (6xx), `MassCancelRequestType(530)` `MCX*` (7xx) and `SubscriptionRequestType(263)` `MDR*` (8xx), each set closing with its `*OTHER` catch-all and `UNKN` (0) none stated - stored as the `uint16` code under `yggdryl.marketdatatype`; `from_fix`/`fix_code` the crate's reading of the wire, `fix_tags_of(msgtype, kind)` which fields type a message - its message type's own rule in `MARKETDATATYPE_MSGTYPE_RULES` (a trade capture report by `TradeReportType`, a mass cancel by `MassCancelRequestType`, a market data request by `SubscriptionRequestType`) before its kind's (`fix_tags`) - which a dictionary overrides per field with `FIX:marketdatatype` (`value=MEMBER` words, `FixRegistry::marketdatatype_of`), the intrinsic `marketdatatypecodeset` rendered from it |
| `side.rs` | the `enum` family's `Side`: FIX `Side(54)`, `UNKN` (0) then the seventeen sides in wire order to `SELU` (17), each spelled by a fixed four-letter code - `BUYS`, `SELL`, `SSHT` - and described by its long name, the stored names before the codes (`BUY`, `SSHORT`) read and never written, one byte in memory and the `uint8` code under `yggdryl.side` in a column - the number (`BUYS` 1, `SELL` 2) a sided element's stored cross code carries; never null - `UNKN` (stored `0`) is a side nobody stated, which a merge takes the other side over and which takes neither the bid nor the ask |
| `timeinforce.rs` | the `enum` family's `TimeInForce`: FIX `TimeInForce(59)`, `UNKN` (0) then one member per wire value in wire order - `DAY` (1) for `0` to `GFM` (13) for `C` - and `OTHER` (99) for a venue's own value, stored as the `uint8` code under `yggdryl.timeinforce`; `from_fix`/`fix_code` the crate's reading of the wire, `from_spelling` a stored name, the standard's name folded or the wire value itself; a dictionary maps any field's values onto members with `FIX:timeinforce` (`value=MEMBER` words, `FixRegistry::timeinforce_of`), what an operation's `timeinforce` - `TimeInForce(59)` itself in the fixed row - reads through |
| `temporal.rs` | what the five temporal families share and nothing any one of them owns: the crate-private `TemporalKind` tag the arithmetic, the digests and canonicalization branch on - public only as `DataTypeId::temporal_family`, `date`, `time`, `datetime`, `duration` or `interval` - the `temporal_leaf!` macro the family files build their count-unit-zone values with, the unit validators the constructors call, the ISO 8601 spellings every text codec and the scalar renderer write through, the Arrow casts that take any temporal, and the `Scalar` readers that answer across the families (`temporal_unit`, `temporal_timezone`, `temporal_count`); `TemporalValue`, the contract every leaf answers, is in `value/`; no datatype, no field and no leaf value live here |
| `date.rs` | the date family: `DateType` - `Date32`, `Date64`, no parameter, the unit being what the leaf is - the typed field's payload over the flat `DataType::Date32` and `DataType::Date64` leaves, with `date32()`, `date64()` and `date_type()`, the `Date32` and `Date64` values with their `Scalar` constructors, one Arrow projection (`Date32`, `Date64`) |
| `time.rs` | the time family: `TimeType` - `Time32(unit)`, `Time64(unit)`, the resolution a parameter of the leaf and `for_unit` the one rule `DataType::time` picks a width by - the typed field's payload over the flat `DataType::Time32(TimeUnit)` and `DataType::Time64(TimeUnit)` leaves, with `time`, `time32`, `time64`, `time_of` and `time_type`, SQL's `time(p)` grammar, the `Time32` and `Time64` values, one Arrow projection (`Time32`, `Time64`) |
| `datetime.rs` | the datetime family: `DateTimeType` - one leaf, `DateTime64 { unit, timezone }` - the typed field's payload over the flat `DataType::DateTime64 { unit, timezone }` leaf, with `datetime64` and `datetime_type`, every `timestamp` spelling of the grammar, the `DateTime64` value, one Arrow projection (`Timestamp`, carrying the zone only when the datatype states one) |
| `duration.rs` | the duration family: `DurationType` - `Duration32(unit)`, `Duration64(unit)` - the typed field's payload over the flat `DataType::Duration32(TimeUnit)` and `DataType::Duration64(TimeUnit)` leaves, with `duration32`, `duration64`, `duration_of` and `duration_type`, the `Duration32` and `Duration64` values, one Arrow projection (`Duration`, which imports back as `duration64` because Arrow has one width) |
| `interval.rs` | the interval family: `IntervalType` - one leaf, `Interval(layout)`, the layout a `TimeUnit` interval member - the typed field's payload over the flat `DataType::Interval(TimeUnit)` leaf, with the validating `interval(unit)` and `interval_type`, the `interval` grammar with SQL's bare `interval day`, the `Interval` value holding every component of every layout, one Arrow projection (`Interval`) |
| `timezone.rs` | the `Timezone` value, its bundled IANA registry, and the `timezone` datatype a column of zones declares |
| `mime_type.rs` + `mime_type/`, `media_type.rs` + `media_type/` | the root `MimeType` and `MediaType` values, which stay the media routing vocabulary, each with a `datatype.rs` beneath it for the `mimetype` and `mediatype` datatypes a column declares; `mime_type/` also holds the extension registry and the line classifier |
| `string.rs` | every string the crate has, one family: the `StringType` enum of eighteen leaves - six shapes in each of UTF-8, US-ASCII and windows-1252 - the eighteen `DataType` and `Scalar` leaf variants it views, the characters `Str` every string value holds, the `FIELD:enum` dictionary `StringEnum` and its ISO listings, one Arrow projection, one cast tier, one grammar, one set of field markers. The twelve registered codes are not strings and are not here: each is its own file named as the code is - `ccy.rs`, `country.rs`, `mic.rs`, `cfi.rs`, the five identifiers `isin.rs`, `cusip.rs`, `sedol.rs`, `figi.rs` and `ric.rs`, `bbg.rs`, `forex.rs` and `unit.rs` - over the contract in `code.rs`. `utf8`, `large_utf8`, `sized_ascii(4)`, `fixed_cp1252(8)` and the spelling `string(windows-1252,32)` are all string leaves and all answer `DataType::string_parameters`; a code answers `DataType::code_width` and `is_code` instead, because it is an identity over a registry rather than a charset, and rides `Utf8` under its own extension name. The per-charset arms - `charset()`, the fixed and sized leaf constructors, `with_charset`, the decode and encode behind `StringType::scalar_from_bytes` and `StringType::encode`, a value's repertoire check - dispatch to `utf8.rs`, `ascii.rs` and `cp1252.rs`; the eighteen-variant enum itself stays here, because a variant is not a type of its own |
| `utf8.rs`, `ascii.rs`, `cp1252.rs` | one root file per charset that has string leaves, each holding that charset's codec and its six leaves together. `utf8.rs`: the UTF-8 decode, transcribe, pending and fault rules under the `utf-8` name, and `Utf8String` through `SizedUtf8String` with `utf8()`, `large_utf8()`, `utf8_view()`, `large_utf8_view()`, `fixed_utf8(w)`, `sized_utf8(n)`. `ascii.rs`: the `ascii_len` scan, `decode`/`encode` and their `_into` forms, `text`, the `us-ascii` name, the `ascii_text`/`ascii_bytes`/`ascii_repertoire` helpers, the `ascii_packed`/`ascii_value`/`packed_width` pair the codes and `StringEnum` ride on, and the six ASCII leaves. `cp1252.rs`: a thin codec over `charset::single_byte` with `tables::CP1252` under the `windows-1252` name, and the six windows-1252 leaves. Each owns its leaves' `DataType` constructors, its `LEAVES` list, and the decode and encode that `StringType::read_text` and `StringType::encode` in `string.rs` dispatch to; only `ascii.rs` judges a repertoire (`ascii_repertoire`) and holds the `i128` packing; `Charset` and `StringType` dispatch to them and duplicate nothing |
| `charset.rs` + `charset/` | the `Charset` vocabulary beside what every code page shares: `single_byte` and the generated `tables.rs` own the code pages, `utf16` owns UTF-16, `bom` the byte-order mark, `Decoder`/`Reader`/`Writer`/`sink` the chunked doors, `Transcoded` the decoding handle. The three charsets with string leaves are root files; every other code page reaches `single_byte` through `Charset` and is not a public module of its own |
| `holder/` | what every backend shares: `Holder`, the one concrete handle unifying every backend, `Buffer`, `Buffered<H>`, `Counted<H>`, and the `Catalog`, `Namespace` and `Table` variants that hold a `warehouse/` object as the handle it is (`Object::into_holder`, `From<Catalog>`/`From<Namespace>`/`From<Table>`), a catalog or a namespace a container whose `ls` yields its children as handles and whose byte verbs are `NotAtomic`, a table the rows its own handle holds. The root traits follow no backend: `IOPath`/`IOFolder`/`IOFile` and their `path_*`/`folder_*`/`file_*` methods are the same on every one |
| `warehouse/` | one abstraction for every place that answers which tables there are and how one is read, every name `yggdryl::<Name>` at the crate root, an object a description - its path, what it states, where its storage is - never a view borrowed from its parent, so it sits in an enum, registers and crosses a binding: `object.rs` the `ObjectValue` contract (`name`, `path`, `kind` - `IOKind::Catalog`/`Namespace`/`Table` - `description`, `url`, `modified`, `properties`, `update_properties`), the `Object` enum (`as_catalog`/`as_namespace`/`as_table`, `into_table`/`into_namespace` answering absence at the path, `into_holder`), `IntoObjectPath` - the one path intake: dotted text through the plan's location grammar (`"..."`, backticks, `[...]` quote a part, the empty text is the root), or parts as they are (`&[&str]`, `[&str; N]`, `Vec<SmolStr>`, a `Location`), a URL or a `with (...)` clause refused at `$.path`, nothing past it splitting on `.` - and the lazy fused `Objects`; `namespace.rs` `NamespaceValue` (`children`, `get`, `create_namespace`, `create_table` - the two creations refused by implementation name by default), the `Namespace` enum (`resolve` one `get` per part, a table met before the last part the absence of the namespace below it, `namespaces()`/`tables()`), the `Namespaces`/`Tables` views over one level (`get`, `create`, `open_or_create`, `contains`, `iter` -> `Names`, `len` and `is_empty` draining the listing, dotted names descending; `Tables` also `open_or_create_from_arrow_reader`, `append_arrow_reader`, `overwrite_arrow_reader` and their `_with_options` forms, creating from the reader's schema where the parent creates), and `container_object_io!`, the `IOBase`/`IOMedia` face of a catalog or a namespace - `ls` the children as handles, `child_by_path` parts, byte verbs `Error::NotAtomic`, record verbs refused naming a table under it, `clear`/`remove` unsupported; `catalog.rs` `CatalogValue` (`namespace_levels`), the `Catalog` enum - `Memory`, `Folder` and, under `iceberg`, `Iceberg` - and `Catalog::from_url` - the `type` property `memory`, `folder` or, under `iceberg`, `hadoop` (`rest`, `xmla` and a `hadoop` this build has no catalog for refused at `$.with.type`, `s3tables://` likewise), the name from `name` else the last segment, every property travelling on; `table.rs` `TableValue` (`field` with no row read, `storage` - a leaf's media type, `directory`, `table`) and the `Table` enum - `Media`, and under `iceberg` `Iceberg(Box<IcebergTable<Handle>>)`, what a folder catalog answers for a table-format folder - delegating every byte and record verb; `properties.rs` `Properties` - the one ordered name/value bag a target's `with (...)` clause, `Holder::from_url`, the backend option doors and every object read: one value per name replaced in place, names kept as written, `inherit` the parent's order with the child's values winning, `knob_bool` and `knob_count::<T>` the typed reads - a flag through the one boolean table, a whole number at the option's width through the one integer grammar - refused at `$.with.<name>`, Display the clause, serde a JSON object; `handle.rs` `Handle`, public and opaque: the `IOBase`/`IOMedia` over the `Holder` an object resolves once on first use under the properties it was given and keeps (`get`, `get_mut`, `From<Holder>`), the root an `IcebergTable<Handle>` sits on so a table is `Clone`/`Eq`/`Hash` where a `Holder` is not; a clone starts unresolved, equality and hash are the site, one bound to an unlocated handle refuses by name; `Site` beside it, crate-private; `memory.rs` `MemoryCatalog`/`MemoryNamespace` - registered objects in order, exactly one level below, no storage, nothing created; `folder.rs` `FolderCatalog`/`FolderNamespace`/`FolderLayout` - a container listed when asked and nothing cached but its handle: a folder a namespace while `levels` remain (`DEFAULT_LEVELS` one, zero under a standalone namespace) else a table of the rows beneath it, a table format's folder a table at any depth by the store's `IOKind::Table` or one listing of its `metadata/` and never a read, a leaf a table when a record medium of this build reads its name's media type, a name less every extension a media type claims (a ZIP member by its last segment, escapes decoded once), a private entry skipped at every level, two entries of one name both listed and a conflict when asked; `media.rs` `MediaTable` - a table over any location a record medium reads, `new` from a `Uri` or `bound` to a handle, its handle opened under its effective properties and composed as its name declares, `with_field`/`with_dtype` the declared field renamed after the table and answered before any read, `with_layout`; `system.rs` `SystemWarehouse`, the process's one `Warehouse` under a lock taken per call, starting with the memory catalog `local` of the folder namespaces `temporary`, `home` and `config`; `mod.rs` `Warehouse` - catalogs by name in order, a namespace or a table at its path with the memory levels built along it and registration under an object that lists its own store refused by name, `replace`, `unregister`, `get`/`table`/`namespace`, `properties_for` the deepest registered object whose URL holds a location on a path boundary. Cost pinned in `iobase_calls` (`mod warehouse`): describing and resolving registered objects nothing, a folder's children one listing plus one listing of `metadata/` per folder entry, a path one listing per level, a read through `Holder::Table` what the table's own handle costs |
| `auth/` | what every identity provider shares, private and under the `http` feature - `Secret` and the `variable` reader the HTTP client needs, the rest under `aws`: `secret.rs` `Secret`, text that renders as `<redacted>` so a holder derives `Debug`, and `write_private`, a cache file holding a secret replaced whole or not at all - written to a private sibling, synced, renamed over it; `lease.rs` `Lease<T: Expiring>`, one expiring value obtained on demand under a lock, refreshed a window before it lapses - the lease's, or the value's own where its source keeps one (`Expiring::refresh_window`) - kept while obtaining another fails and it still stands, its failure held for a pause rather than repeated per request, `is_holding` whether a value stands now, with `Bearer` (a token and its expiry, under `s3` for the two dialects that hand one) and the expiry spellings (`instant`, `instant_from_millis`, `iso8601`); `environment.rs` `Environment`, the process environment or the pairs a caller handed over; `report.rs` `Report`, the failures and absences one walk of the sources recorded - each logged as it is recorded under the walking module's own `log` target, `failures` the recorded ones - and the refusal that names them - or none, when nothing was configured. `aws/`, `s3/google/` and `s3/azure/` carry only where their answer comes from and how it is spelled on the wire |
| `aws/` | who this process is to AWS, and where AWS is, for every consumer that signs an AWS request: `session.rs` the one door - `Session`, what a caller states, the rest resolved lazily, the credential chain walked in botocore's order with every configured-but-broken source recorded and passed over rather than failing the walk, a temporary set refreshed before it lapses and kept while a refresh fails until it has; the shared files read again by a walk that finds either moved on disk, after a store's refusal and while an unsigned or failed answer is held, and a set a source answers passed over by name when it has lapsed or a store refused its key (`invalidate_if`; a lapse for good, an unrecognized key for a pause or until the files move), a set the caller stated never; every identity call sent through `Session::http`, the crate's own `http::Session` with no redirect, cookie or `.netrc`, and every walk logged under `yggdryl.aws.session` with key ids masked - `credentials.rs` the `Credentials` value, the JSON document the metadata services and a `credential_process` answer and `Refusal`, what a store's error code says of a key, `environment.rs` (under `s3`) the variables the session reads for itself and the S3 sweep leaves to it, `profile.rs` the `~/.aws/config` and `~/.aws/credentials` reading (`[profile x]`, `[sso-session x]`, `[services x]`, indented tables, the credentials file winning, the expiry a tool wrote beside a set, a pasted shell block, half a set refused by name) and `Profile`, `sts.rs` `AssumedRole` with `AssumeRole`, `AssumeRoleWithWebIdentity` and the `~/.aws/cli/cache` the CLI shares, `sso.rs` the IAM Identity Center token cache, its refresh, the device sign-in and the portal exchange, `login.rs` the console sign-in `aws login` files under `~/.aws/login/cache` and its refresh at the Sign-In service under a DPoP proof, `process.rs`, `container.rs` and `metadata.rs` the three remaining sources, `sigv4.rs` Signature Version 4 for every service - the signer owning the one rule the two families differ by, the canonical URI: the path as sent for the S3 family (`is_s3_family`: `s3`, `s3express`, `s3-object-lambda`, `s3-outposts`), normalized and percent-encoded once more for every other, as botocore's `SigV4Auth` does, `x-amz-content-sha256` sent and signed for all, pinned against AWS's published vectors and vectors botocore computed - `request.rs` `Request::with_sigv4(session, service, region)`, an inherent method of `http::Request` written here so `http/` depends on nothing in `aws/`: the request's per-attempt hook set to sign what the attempt really sends and its resend rule set to read a refused key, no credential source answering refused by name rather than sent unsigned, a streamed body refused outside the S3 family; `properties.rs` `Session::from_properties`/`with_properties`, the one reader of AWS identity properties - bare names, `aws_` names and PyIceberg's `client.*`, never `token`, never `s3.*` - which `S3Options::with_properties` hands its identity names to; the session keeps the one signer cache (`signer(service, region, now)`, by credential set, region and service), the one decision a refused key leads to (`answers_another`, shared by the S3 client's own transport and `with_sigv4`) and the one endpoint door (`service_endpoint(service, region)`: the configured endpoint, else the partition's host under the FIPS and dual-stack switches, STS's legacy global rule over it); a partition's hosts are `ArnPartition`'s (`uri/arn.rs`); under the non-default `aws` feature, which `s3` implies |
| `xml/` | the XML structured codec over `Scalar` - a document is the record naming its root element, `@name` an attribute, `#text` an element's own text beside attributes or children, a repeated element a sequence, a self-closed element null and an emptied one the empty text, every leaf text - `mod.rs` the doors, `parser.rs` the quick-xml event fold, `wire.rs` the writer and the field-directed reshaping (a repeated element read once is one item, absent is the empty sequence, text trimmed and empty text null under a non-text leaf); `scanner.rs` beside them, private and under the `aws` feature, the deterministic scanner for the small fixed-shape XML documents S3, Azure Blob Storage and STS answer, knowing the name of no element, each reader naming its own vocabulary over it; `element.rs` the namespace-aware `Element`/`Scope` view over a parsed document, which `soap/` and `xmla/` read through |
| `local/`, `fs/`, `zip/`, `s3/` | one root folder per storage backend, each a location/container/leaf trio over the root traits: `LocalPath`, `LocalFolder`, `LocalFile`, `FsPath`, `FsFolder`, `FsFile` and `S3Path`, `S3Folder`, `S3File` in `local/`, `fs/` and `s3/`; `ZipPath`, `ZipNode`, `ZipLeaf` in `zip/`, which indexes names and has no directories or files to name after. `local/` is memory-mapped local storage, and remote backends change neither it nor the root traits; `fs::FileSystem` is Arrow's seven-method shape for interop, while the core contract and variants keep generic `FileSystem`/`Fs*` names; `s3/` holds Amazon S3, Google Cloud Storage and Azure Blob Storage inside it, since all three answer that dialect, under the non-default `s3` feature |
| `http/` | HTTP behind `IOBase`, under the non-default `http` feature, which `aws` implies: one synchronous HTTP/1.1 client over `ureq` (`client.rs` `Client`, the pool and transport knobs, and `StatsSnapshot`), `session.rs` `Session` - where a request's defaults, authorization, cookie jar, `Accept-Encoding`, retries and redirects are applied - `request.rs` `Request` and `Body`, `response.rs` `Response`, `stream.rs` `Stream`, `pages.rs` `Pages` and `pagination.rs` `Pagination`, the four roles a `Holder` holds (`HttpSession` a container over a base URL, `HttpRequest` the leaf a URL names, `HttpResponse` one answer's body as sent, `HttpStream` a body on the wire); `headers.rs` `Headers` over the crate's `Metadata` under the `HTTP:` protocol key with the typed readers beside it and `headers/` the date, link, range and entity-tag grammars; `method.rs`, `status.rs`, `cookie.rs`, `authorization.rs`, `options.rs` `HttpOptions`; `wire.rs` the RFC 9112 message grammar every `message/http` door and the server share; `retry.rs`, crate-private, the retry budget, backoff and verdicts the S3 client draws on too; `server.rs` with `server/` the `Server` hosting any `Holder` and programmable routes over the same grammar, which the tests run every client feature against, a route's handler answering bytes in hand or, never held whole, a body `Response::with_writer` runs as the answer is sent, and `ServerOptions::with_trace` writing every exchange under a folder as `message/http` documents |
| `coding/` | what every codec shares: the transparent `Coded<H>` handle and the `Codec` dispatch helpers |
| `gzip.rs`, `zlib.rs`, `zstd.rs` | one root file per codec; each owns `load`, `dump`, `reader`, `writer`, an `IOBase` wrapper |
| `media/` | what every medium shares: the `Media` value naming every implementation, record options, inference, magic, merge, partition, structured routing |
| `ipc/`, `parquet/`, `avro/`, `csv/` | one root folder per record medium; each owns free functions over `IOBase` plus a stateful wrapper. `csv/` is delimited text - `text/csv` and `text/tab-separated-values`, `CsvOptions` holding the separator, the quote, the escape, the comment, the header, the null spellings, the trim and the sample the inference reads - read as one streaming RFC 4180 tokenizer over the handle's coding and charset (`reader.rs`), typed by the declared field's value door or by the inference ladder over the first `infer_row_size` records, and written cell by cell off the record `Serie` (`writer.rs`); `Csv<H>` the wrapper (`media.rs`) |
| `soap/` | the SOAP 1.1 envelope over the XML codec - `Envelope`, `Fault`, `Fragment`, the streaming `EnvelopeWriter`; protocol vocabulary no medium owns, a root folder of its own name like every implementation, which `xmla/` speaks over, the HTTP it travels on being `http/`'s |
| `xmla/` | the XML for Analysis 1.1 medium and provider: `vocabulary.rs` the methods, request types, properties, restrictions and enumerations, `request.rs` `Discover`, `Execute`, `Request` and the session headers, `response.rs` `Response`, `XmlaError` and the fault door, `rowset.rs` the rowset document - `Rowset`, `XsdType`, the `_xHHHH_` name escape - written cell by cell off the column leaves and read back under the schema it carries or a declared field, `dbtype.rs` the OLE DB type codes `DBSCHEMA_COLUMNS` states, `definitions.rs` the schema rowsets the provider answers, `options.rs` `XmlaOptions`, `media.rs` `Xmla<H>` the `.xmla` handle, `service.rs` the provider over a `Warehouse` answering a request's bytes - `with_catalog` any `Catalog` in place of one of its name, `with_warehouse` a whole one; Discover over the definitions and the catalogs' traits (`DBSCHEMA_SCHEMATA` every namespace under a catalog, `DBSCHEMA_TABLES` every table with `TABLE_SCHEMA` the namespace parts between as a path and `DESCRIPTION` the table's description else its `storage()`, `DBSCHEMA_COLUMNS` from `field()`); Execute through the expression `Plan` against the whole path a statement's table resolves to (`table` under the `Catalog` property, `catalog.table`, `schema.table`, `catalog.schema.table`, as deep as `namespace_levels` allows, verified before it runs so an unknown table is that fault), run by `Plan::execute_in(&warehouse)` so no target is rewritten to a URL, a URL target refused outside every served catalog's location - and, under the `http` feature, `server.rs` the routes a `Service` answers on `http::Server` (`Service::route`); `yggdryl xmla serve` in `cli/` is its terminal |
| `excel/` | the Office Open XML workbook medium over `zip/` and the XML parser: `cell.rs` `CellRef`, `CellRange`, `CellKind`, `Cell` and `DateSystem` (the 1900 and 1904 serial systems, the value door every cell crosses), `styles.rs` `NumberFormat` and the `cellXfs` classification, `shared_strings.rs` the shared string table and the `_xHHHH_` escape, `package.rs` the OPC part names, relationships, content types and the one-pass document rewrite, `parser.rs` the streaming worksheet fold, `sheet.rs` `Sheet` and `Row` (random access, `from_serie`/`into_serie`), `workbook.rs` `Workbook` (the package opened lazily, its sheets parsed on first access, written back preserving every other part), `reader.rs` the record path streaming one sheet under an inferred or declared field, `writer.rs` the part rendered as the batches arrive, `options.rs` `ExcelOptions` (`sheet`, `header`, `range`), `media.rs` `Excel<H>` and the free doors; a number cell is `float64`, a styled serial the temporal its format names, a missing sheet the empty stream, a `.xlsx.gz` name refused |
| `iceberg/` | separate modules: types, schema, partition, snapshots, metadata, manifests, statistics, scalar rendering, scan, table - `IcebergTable<H>`, its root eager and its current document read on the first verb that needs it, `Clone` opening afresh, `Eq`/`Hash` over the path, `ObjectValue`/`TableValue` so a warehouse `Table` holds it over a `Handle` - options, `catalog/` - `IcebergCatalog` and `IcebergNamespace`, the warehouse implementations over a folder laid out as `HadoopCatalog` lays one out: namespaces to any depth (`namespace_levels` none), each level's stored properties in its own `metadata/catalog.json` or `metadata/namespace.json` beneath what was stated (the `ICEBERG:` prefix refused), a listing classifying each entry by one listing of its `metadata/` and no read, `create_namespace` writing the document and `create_table` `IcebergTable::create` under `PartitionSpec::from_schema` over the schema as Iceberg expresses it (`into_scheme_compat`), a create descending through existing namespaces only, a table's properties riding its metadata (`update_properties` refused for `commit_metadata_changes`), cost pinned in `rust/tests/iceberg/catalog/mod_.rs` (`mod call_counts`) - evolution, inspection |
| `text/` | the plain-text medium - `Text<H>`, flat `TextOptions`, bounded physical-line splitting, row-header capture, body rendering, `TextBytes`/`TextLine`/`TextEntries` - `TextLine` an `Event` of the graph holding the whole line, row header included, and the `Arc<TextOptions>` it reads itself by, every reading resolved once on its first ask and a `set_` stated over it - beside what the structured codecs share: `Format`, `Limits`, `Formatting`, `Loading`, placeholders, `TextCodec`, io, wire, typed |
| `json/`, `toml/`, `yaml/`, `xml/` | one root folder per structured codec over `Scalar`, each its own parser over the machinery in `text/` |
| `uri/` | the URI, URL, URN and ARN values, `ArnPartition` (`arn.rs`) - botocore's partition table, the one owner of an AWS partition's name, its regions' prefix, its DNS suffixes and the host a service answers on in a region (`service_host`) - and, in `datatype.rs`, the `uri` family - `UriType` with its `url` and `urn` leaves - and the fields and scalars over them |
| `arrow/` | Arrow interop: stream combinators, schema projections, the IPC dictionary sidecar, `size.rs` - the one memory estimator every byte bound reads (`memory_size`, `array_memory_size`, `scalar_memory_size`: a batch or an array costs what its own slice reaches) - `extension.rs` - arrow-rs's typed `ExtensionType` for each datatype marker that rides a name and for the `StringType`/`BytesType` leaves that ride a document, each redirecting into the one recognizer a field is imported through, the name owned by `DataTypeId::arrow_extension_name` - and `rows.rs`, the bounded row-to-batch reader that lays each batch out through `Serie`; every value crossing is `Serie`'s and every cast `cast.rs`'s, reached through `Serie`, `SerieReader` and `ArrowCastPlan` |
| `expression/` | one term grammar and one plan grammar: `Term`/`Bound`, `Filter`, `Selector`/`BoundSelector`, `Plan` (create, write verbs, `select`, `from`, `where`, `order by`, `limit`, `offset`), `Target` (a `Location` and the `Properties` bag its `with (...)` clause states - `warehouse/properties.rs`'s, the same bag `Holder::from_url` reads; `Target::holder(&warehouse, base)` the one resolution - a URL through `Holder::from_url` under the warehouse's `properties_for` beneath the target's own, parts joined onto a base where a media gives one, parts alone the table registered at that path as `Holder::Table` with the target's properties stated on it, a namespace or a catalog refused by kind, absence told to register the table or name a URL - and `write_holder` the same for a write, an absent last part created through the namespace above it from the stream's root), `Plan::execute` resolving against `SystemWarehouse` and `execute_in(&warehouse)` against a given one, each taking the registry's lock for the resolution alone and restoring a registered table's declared field after the pushed plan is set; the grammar's one location token - after `from`, `into`, `to`, a write verb or `create`, raw text opening as `<scheme>://`, `/`, `./`, `../`, `~/` or a drive letter runs to the first whitespace, `,`, `;` or `)`, reads through `Url::from_location` and prints back quoted, so `price / size` after a column stays a division, `Expression` (clause, plan, or `;` sequence), `Records`, `Bounds`, `explain`, `FieldPath`/`FieldSegment`, `time_bucket` (`eval.rs` `TimeBucket`, the one bucket floor the row, batch and statistics tiers share), `user` (registered `namespace.name` functions, `FunctionSignature` as a struct field, `Function::User`), `transform` (`TRANSFORM:function`/`TRANSFORM:by`, else `TRANSFORM:expression`); every application (`apply_datatype` first and `apply_field` derived from it, `apply_scalar`, `apply_arrow_reader` first and `apply_arrow_batch` derived from it, `apply_records`, `from_scalar` readers) lives here and nowhere else |
| `graph/` | the graph vocabulary: `element.rs` holds `Element` - an element's `Uuid`, its cross identity and code, its codes and its canonically sorted source UUIDs (the elements it was read from: provenance, never carried along a chain), read and written - and `Event`, an element with an instant (`currunix`, `i64` nanoseconds since the epoch, UTC), a precise optional recording instant, a state and a place among the events of its instant, the predecessor named by `prevuuid` alone; `market.rs` holds `Market` - thirty-four facts and no supertrait, every setter taking a trailing `overwrite` - `false` fills only an unstated fact, `true` states it - and carrying what it implies onto the facts that follow it by direct writes that never recurse (a price or quantity the side's bid or ask, a side the quote and the side of a sided kind's stored cross code, a `displayqty` the `hiddenqty`, `leavesqty` the quantity, an order's `ordqty`, `cumqty` and `leavesqty` one another by its state, and back, fill-only: a bid or ask the price, `hiddenqty` the shown part, two FX parts the third): the `MarketDataType`, the price, the stop price, the currency, the quantity, its shown and hidden parts, the `Unit` and the `Side` by value, the `securityids` `Identifiers` behind the fallible `insert_securityid`/`remove_securityid`/`derive_securityid` verbs, the CFI, the MIC, `execunix` (when it last executed, a nanosecond UTC clock: an event whose state reports an execution and states none is dated from its own instant, following keeps the later of its own and its predecessor's, two statements of one event keep the earliest - a market fact, never an `Event` one, so a text line states none), the ticker, the last-trade, average, progress, previous-step and FX numbers, the bid and ask each as a price, a quantity and a currency (`bidpx`, `bidqty`, `bidccy`, `askpx`, `askqty`, `askccy`), the `FxRates` map from a target currency to the rate an amount is divided by, which nothing fills, the `Metadata`, and the required `marketdatakind`, the kind a holder stamps (a FIX message's `msgcat`) and the category a lifecycle chains within - with four provided readings: `is_sided`, `MarketDataKind::is_sided` of that kind, `get_isincode`, the `isin` security identifier borrowed, `stored_crosscode`, the one speller of the prefix every element's cross code is stored under, `{kind}:{side}:{base}` - the `MarketDataKind` code, the `Side` code of a sided kind (an order or an execution) and `0` for every other kind or a side not stated (`10:1:ORD-1`, `14:0:Q-1`, `21:0:T-1`, `3:0:US0378331005`), idempotent, a prefix of another kind or side replaced, an empty code kept empty, a copy of a sided element into another kind's holder taking its base code - and `book_crosscode`, the book an element stands in (its instrument's ISIN wherever it holds one, else its non-empty ticker, else `Isin::NONE`); a quote is one element holding its two legs (`bidpx`, `bidqty`, `bidccy` and the ask's), its `side` a tag, and an untagged follower carries each leg it states nothing of - `Operation: Market` - five more: the `ordqty` it asked for, the `TimeInForce`, whether it trades and the `Identifiers` `identifiers` and `partyids` behind the same verbs (`insert_identifier`, `insert_partyid`), a FIX message's `partyids` its `Parties` groups and its `Account(1)`, read off its fields - each providing, `where Self: Event`, what an event that is one of them answers (`digest_market_event`, `following_market`, `merging_market_event`, and the `_operation_event` three); a follower takes every `metadata` key of its chain that it lacks and, of `Operation`, every `identifiers` type but `mdentryrefid` (`is_followed_identifier`; a FIX message its dictionary's `FIX:idmap` follow flags, each followed type's parents with it) and every party id (`Identifiers::carry`), and, for each identifier it states, the parents its chain gave it - `Identifiers::follow_parents` over `Operation::parents_of` and `parent_of`, which a FIX message answers through its registry's `FIX:parents`: the first parent the type's previous value, the last of two or more the chain's first, so `orderid` A, B, C, D ends with `parentorderid` C and `origorderid` A and `clordid`'s one parent `origclordid` is the previous `clordid` - while every finalize fills a parent's own type from its nearest stated parent (`Identifiers::fill_parents`); `facts.rs` the crate-private `MarketFacts`, `MarketEventFacts`, `OperationFacts` and `OperationEventFacts` holders, one per role, each leaf holding its role's; `operation.rs` the operation leaves - `OperationElement<K>` (`Order`, `Quote`, `Execution`) and `OperationEvent<K>` (`OrderEvent`, `QuoteEvent`, `ExecutionEvent`) over the sealed `OperationKind` markers - a boxed `BookRef` book control and `MdUpdateAction`; `trade.rs` the composite `TradeEvent`; `book.rs` `BookEvent` - complete (`is_complete`: its `alive` entries, each side read as `alive_on(side)` and `limits(side)`, a level keeping its quantity in place) only at a snapshot tick, else its `deltas` alone in the order applied, rebuilt over the complete book before it by `with_previous` and a code's first over `BookEvent::keyed` - and the first tradable level of each side its `best_price`/`best_quantity` and its `bidpx`/`askpx` in both forms; no execution - `SnapshotEvent` and `BookIterator`, one book per `book_crosscode`, every booked input a delta, a book yielded where it holds one or at a snapshot tick holding an alive entry, an entry restated under another key withdrawn from the book it stood in, `with_filter` an expression narrowing what it folds; `market_data.rs` `MarketData`, the one enum over every leaf and a FIX message held whole (`MarketData::Fix`, `MarketKind::Fix`) - which walks and merges as itself and which the Arrow writer and the book fold take as the leaves it splits into (`FixMsg::into_market_data`; `FixMsg::into_market_leaf` the one leaf a message is) - and `kind.rs` its `MarketKind` and the `MarketDataKind` each leaf stands under; `arrow.rs` the one lifted `marketdata` row - the element and event columns, then the market columns opening with `marketdatakind` and `marketdatatype`, then the operation columns with `bookaction` and `bookposition`, `bookscope`, and a book's nested `alive`, `deltas`, `executions` (a trade's, null on a book), `bidlimits` and `asklimits` - sixty-two columns - and its two doors; `view.rs` `MarketView` and the six plans over the `marketdata` row, each one `Plan` built structurally and applied by the expression engine; `candle.rs` `Candle`, one OHLC per stored book cross code (`3:0:ACME`) and bucket - `Ohlc` over the best bid, the best ask, the midpoint and the spread, the last best quantities and the books - twenty-three cells - `CandleOptions` (the interval and the zone its buckets align to) and `CandleIterator`, the fold of sorted books, with `Candle::field()` its Arrow row; `serve.rs`, under the `http` feature, `BookService`, the HTTP face of a market-data table the display reads - `/api/tables`, `/api/tickers`, `/api/candles`, `/api/book`, `/api/events` as JSON and `/api/audit.csv[.gz|.zst]` through the CSV medium, keyed by the book key - a reading of a key one filtered read, a ticker resolved to its one key only when that read answers nothing, a book rebuilt from the delta rows at or before its instant and stating `complete` - every reading a method a caller reaches without HTTP; iterator.rs` the one walk over any `Event + Operation` and over `MarketData`, its live chains keyed by the stored cross code within one `MarketDataKind`, so an order and an execution under one code are two chains, and the two sides of one order, quote or execution identifier are two - an element stating `UNKN` joining the one side its base code is alive on, the base index holding sided elements alone - naming live identities by `identifiers`, a parent identifier's value also under its base; `element_column.rs` the six element columns (`ElementColumn`), `column.rs` the nine event columns (`EventColumn`), `market_column.rs` the thirty-four market columns (`marketdatakind`, `marketdatatype`, `isincode`, `execunix`, the six bid and ask facts and `fxrates` among them) and `operation_column.rs` the five operation columns (`ordqty`, `timeinforce`, `tradable`, `identifiers`, `partyids`), every generated schema stating them under one name and one datatype each - `securityids`, `identifiers` and `partyids` each a sorted `map<utf8, utf8>` from the key's text - `src:type`, the type alone for the base source - to the value, the base key each type's answer - a text line's batch opens with the element and event columns in `ElementColumn::ALL` then `EventColumn::ALL` order, while a FIX row contains the same fields through the crate's protocol-oriented bands and lifecycle rows retain them; what a lifecycle learns about instruments is `isin_registry.rs`'s at the root; signatures and provided readings, no storage |
| `idkey.rs` | `IdKey` - the source and the type an identifier is held under, spelled `src:type` and, for the base source, as its type alone (`isin`); read exactly by `FromStr` (a bare word the base type, `fix:` and `base:` spellings the base, nothing inferred), ordered by the bytes of that spelling - the one order of storage, Arrow and the digest - and spelled from a static table for every pair of member words (`known`, `write_into`), the crate-private `KeyReader` caching what a reader reads past it |
| `identifier.rs` | `Identifier` - a value under an `IdKey`, ordered by the key as spelled then by value - and `Identifiers`, the sorted map from the key to the value, held as one vector in key order, a market element states three of: its `securityids`, its `identifiers` and its `partyids`; the words are folded to lower case here (`IdWord`, the fold and the `id_vocabulary!` macro the two vocabularies are written by); the base rule is `Identifiers`' alone, for all three maps and every holder, setter, door and binding: a named source fills its type's base key where it is empty or ranks below it (`ullink:isin=X` alone is also `isin=X`), a statement takes back the type's derivation (`derived:T`) and its echo unless the derivation outranks it (`IdType::rank`: a registry's real number is never displaced by a masked one or a typo, whatever the order), a derivation lands only where nothing of its type is held or the base key ranks below it and its echo follows it, the base key moves only through its own key - `remove(&IdKey::base(T))` removing the type - so wherever a type is held its base key is and `get` is one binary search, and a map read back is closed (`from_scalar`, the Arrow readers); `merge(other, later)` is the one union; a value is checked by its type; `Identifier::from_key` is the one inference of an identifier from a name no key spells - an explicit `src:type`, a whole security name as the base key, else the identifier name the folded name ends with under the source before it, a reserved `base`, `fix` or `derived` naming none (`OMS_InstrumentID` is `oms:instrumentid`, `Derived_ISIN` `isin`) - which a FIX message reads its unmapped entries by; parentage is a relation between types (`IdType::parents`, `parent_of`), never a field of a value, moved along a chain by `follow_parents` and `fill_parents` and carried by `carry`; the Arrow shape `map<entries: struct<key: utf8, value: utf8>, keys_sorted = true>` (`Identifiers::dtype()`), a key that reads as none, a value its type refuses and two spellings of one key with two values refused, located on the key |
| `idtype.rs`, `idsource.rs` | the two word vocabularies an identifier is keyed by, each an enum of the members the crate names plus `Other(IdWord)`, parsed by the one fold and equal to a `&str`: `IdType` - FIX's `SecurityIDSource(22)` types, the crate's own `forex`, `cfi` and `instrumentid`, the operation identifiers, the regulatory trade ids, the party roles and `account` - which owns each type's value rule (`max_value_width`, the check digits, the canonical pair), the security-type reading (`from_fix_security_source`, `from_security_source`, `from_field_name`, `check_security`), so no second security-type enum exists, the sets a type belongs to (`is_security`, `is_party`, `is_listing` - a venue listing's code rather than the instrument's), the datatype its values declare (`value_dtype`) and its parentage (`parents`, nearest first - `clordid`'s is `origclordid`, any other type's `parent{type}` then `orig{type}` - and `parent_of`, the parent's own type and its place); `IdSource` - `base`, which the word `fix` reads as and nothing writes, `derived`, and the `PartyIDSource(447)` and `AcctIDSource(660)` names, a bridge such as `oms` an `Other` word |
| `isin_registry.rs` | `IsinRegistry` and its row `IsinEntry`: one row per ISIN - `isin`, `updunix`, the detailed `cficode`, the `miccode` its listing columns belong to, the `ticker`, and one typed column per `SecurityIDSource(22)` type but the ISIN, at most twelve stated - beside the exact inverse indexes of the rows' RICs and tickers (`get_by_ticker`); `merge` the update rule, refusing a key that is no real ISIN (the latest statement leads column by column, an older one only fills, a refining CFI refines whatever the time, a newer listing fact on another market switches the listing whole, a RIC owned by the latest statement); `learn` keyed by a stated real ISIN else a stated RIC, which only fills, `fill` through `derive_securityid` - the ISIN by ISIN, else RIC, else ticker and market where the element holds none or an unreal one - and gated by market, `enrich` both - the ordered lifecycle's alone, never a parse's or a setter's; a copy-on-write value, the lock only in its sharers (`FixCodec::with_isin_registry`, the bindings); `max_instruments` (16,384 at 3 KiB each) skipped by `learn` and refused by `merge` and a load; read by `from_handle`/`extend_from_arrow_reader` resolving each column's spelling once and adopting a code column's cells as the landing proved them, a `utf8` one's through its type's rule, written by `IOBase::write_arrow_reader` of its snapshot stream |
| `limit.rs` | one price level of a book side - price, quantity, uuids, tradable - a root value type over `struct<price: decimal?, quantity: decimal, uuids: serie<uuid>, tradable: boolean>`: `Limit` with its `dtype`, `field`, `into_scalar` and `from_scalar`, never a `DataType` variant; a level is tradable when any of its entries does not state `tradable = false`, so an entry stating nothing trades and only a level every entry of which says `false` cannot; answered by `BookEvent::limits(side)` and written as a book row's `bidlimits` and `asklimits`, best first |
| `hashing/` | the private structural/display stable-hash adapters the digests share; shared dispatch vocabulary is `digest.rs` |
| `xxhash/` | one-shot digests, four resumable states, `reader`/`writer`, `Hashed<H>`, the canonical `Scalar` byte feed, Arrow row digests |
| `variant.rs` | the Apache Parquet Variant binary encoding, version 1: `Variant` - one metadata dictionary and one value payload - `Scalar::Variant`, the encode and decode doors, the canonical `arrow.parquet.variant` projection, and what every medium writes for a `variant` column |
| `valuestream.rs` | the crate's own byte stream: any `Scalar` as version, `DataTypeId`, payload, children - and back, leaf for leaf; the stream, the whole, the `DataType` doors; what pickle carries |
| `txhash/` | raw Unix-count/digest pairs, clock-unit conversion, configured hashing and Arrow coupling; not RFC UUIDs |
| `logging/` | Python's `logging`, owned by the core: `Level` - Python's numbers, `TRACE` (5) the level a `log` trace arrives at - `Record`, the borrowed event, `Logger` and `get_logger`, the dotted tree with Python's parent fix-up, inherited levels, `propagate`, `disabled` and filters, every logger's threshold cached under one generation that a level, `disable`, a handler or the host bumps, so a disabled record is a few atomic loads and no lock; `Handler` over a shared `HandlerState`, `StreamHandler`, `NullHandler` and `FileHandler<H: IOBase>` - one `append_bytes` per publish, `with_capacity`/`with_flush_level` Python's `MemoryHandler` folded in; `Formatter`, Python's `%`-style format and its `strftime` date, parsed once and refused by byte, and `Formatter::terminal`, the modern UTF-8 line - time, level glyph and name, `[thread]`, logger, call site, message, later lines hanging - every handler stating no format spells, coloured where `terminal.rs`'s `is_color_enabled` (`NO_COLOR`, `FORCE_COLOR`, `CLICOLOR_FORCE`, `TERM=dumb`, a terminal) says, the thread named by `set_thread_name` else as spawned else `Thread-N`; `basic_config`, `disable`, `shutdown`, the last resort - the terminal line on standard error from `WARNING`; `install`, the tree as the `log` facade's backend, its ceiling following the tree and `set_foreign_level` the floor a binding states for crates outside this one; `Host`, the runtime logging a binding attaches - asked once per change for a logger's threshold, never under a lock - and `invalidate`, what the host calls when its levels move; `repeats.rs` `Repeats`, the one deduplication - a lock-free table of key and count slots bounded at 4096 keys, keyed by `Record::stable_hash` (XXH3-64 of the logger, the level and the message rendered into the hash) - that a logger stating `set_deduplicating` counts its records through where they are logged, before any handler or host, saying the first and each tenfold occurrence; `warning.rs` the deduplicated data warnings every door raises through `warned!`, counted in a `Repeats` of their own |
| `parallel.rs` | the one ordered map over persistent stream workers the FIX doors read on: line, message-row and write doors use 64-item chunks at lane depth two; Arrow capture parsing holds at most one whole input batch per worker; answers stay in input order and one thread is the lazy sequential map |
| `fix/` | FIX protocol behavior: `FixRegistry`, every name lookup of which reads the four word pairs `offer`/`ask`, `size`/`qty`, `bid`/`demand` and `px`/`price` either way inside the folded name (`aliases.rs`) - an exact name first, a spelling reaching two fields reaching none, nothing written into the dictionary; `FixCodec` and `FixMsg`, whose parse splits once - an execution report's fill into its order's or quote's report and an `EXEC` message, a trade into one execution per side, a batch (`ORDB`, `QUOB`, `EXEB`, `TRDB`) into one message of its item category per entry of its entry group, chained by the order the entry names - so every door and the book read each fill, side and entry once; a quote is one message holding both its legs, a `W`/`X` entry's `MDEntryType` in its cross code's base; an execution report of no fill is its order's or quote's leaf, an acknowledgement of an execution (`BN`, `Q`) none; the fills and the retired-field restatements are native code, and a registry carries no rule of its own - there is no `FIX:derivation` and no `FIX:replacements`; the fixed row (`schema.rs`), whose `fixentries` is a sorted `map<utf8, utf8>` keyed `tag:name` - a scalar's wire text, a group or a component as JSON keyed the same way - and whose `metadata` holds every key no dictionary resolves, restored as the message's own entry when the row is read back so the wire and the digest are the parse's; the crate's own fields in `crated.rs`, tagged contiguously from `65_001` to `65_050` - none a bridge's: a bridge's own key no dictionary resolves (`OMS_UserID`, `firm.x.ParentOrderID`, `ISINCODE`) arrives as an unmapped entry, which the message reads by `Identifier::from_key` into `securityids`, `partyids` or `identifiers` by its type - every security spelling ending `code`, `symbol`, `number` or `ticker` but `ticker`, `symbol` and `tickercode` themselves - and keeps in its metadata and on the wire as it came, the fixed row filing a captured entry in `fixentries` under `0:<key>` rather than in its `metadata` cell; `StrikePrice(202)` is the dictionary's own field, read by `FixMsg::strikeprice`; `detailedcficode` is a name of `CFICode(461)`, and a message's two statements of 461 fold into one code by `Cfi::refined` where they describe one instrument; a field states the parents of the identifier it holds with `FIX:parents` (a JSON array of identifier types, nearest first: `ClOrdID(11)` states `["origclordid"]`, and a field named as another's parent - `parent` or `orig` before its name - is listed on it wherever fields arrive), which `FixRegistry::parents_of` and `parent_of` read before the name rule |
| binding `lib.rs` | boundary helpers, exports, registration - nothing else |
| binding `src/` | the crate layout above, one layer thinner: one type per root file (`datatype.rs`, `field.rs`, `scalar.rs`, `cast.rs`, `parameters.rs`, `timezone.rs`, `protocol.rs`, `value.rs`, `version.rs`), one root file per implementation (`avro.rs`, `iceberg.rs`), `text/` holding `codec.rs`, `line.rs` and Node's `options.rs`, and `media/` holding only what every medium shares - Python's `handles.rs` and `partition.rs`, Node's `options.rs` |

Parquet is feature-gated; Avro's scalar codec is unconditional and its record
surface uses Arrow; Iceberg sits on these codecs. `Text<H>` keeps only options
and delegates ordinary `IOMedia` - no line value, custom iterator, schema
builder, or line-only read/write. Sole dispatchers, delegating complete contracts
with no variant-specific public vocabulary: `Codec` (coding), `DigestAlgorithm`
(digests), `MediaType` via `RecordOptions` (encoding).

### Where a test lives

`rust/tests/` mirrors `rust/src/`, file for file: `rust/src/avro/schema.rs` is
pinned by `rust/tests/avro/schema.rs`, and a file the crate root holds -
`datatype.rs`, `field.rs`, `scalar.rs` - by `rust/tests/root/<name>.rs`. One
harness target per top-level source entry declares those files as `#[path]`
modules: `rust/tests/<folder>.rs` per source folder, and `rust/tests/root.rs`
for the root files. The mirror is the rule, so a new source file gets its test
file at the matching path and nothing has to be decided. A file that pins no
source file is not a suite: a fixture several targets share lives in
`rust/tests/support/` and is declared by each of them, and what is pinned as a
cost or an exchange rather than as a file - `allocations.rs`,
`iobase_calls.rs`, `benchmark_mode.rs`, `docs_index.rs`, `interop/` - is its
own target, and so is what must own its process: `spill_doors.rs`, which
installs the process spill bound before anything reads it, and
`scale_ulbridge.rs`, the streamed capture path from a `.log.zst` to an Iceberg
table under a bounded `RssAnon` - three copies of the capture in the ordinary
loop, and the scale run `#[ignore]`d and a no-op printing `SKIPPED` unless
`YGGDRYL_SCALE_BYTES` names the size to generate.

A test file opens with a `//!` line naming the source file it pins, and holds
no module named after itself: `avro::schema::schema::x` says the name twice, so
what would carry it sits at the file's top level instead.

A test reaches the crate through `yggdryl::` and nothing else, so what it
proves is what a caller can rely on, and a fixture builds its own inputs rather
than borrowing the code under test.

`src/` holds no test code at all. What a caller cannot reach - a signing step,
a format reader, a canonical rewrite, the bundled zone registry, the
single-item pins - is reached through `yggdryl::internals::<module path>`,
which exists only under the non-default `internals` feature. A module opts in
by declaring its own, beside what it owns:

```rust
#[cfg(feature = "internals")]
#[doc(hidden)]
pub mod internals {
    //! What `rust/tests/root/utf8.rs` pins and a caller cannot reach.
    pub fn transcribe_into(input: &[u8], target: &mut String) -> usize {
        super::transcribe_into(input, target)
    }
}
```

A forwarding `pub fn` is the shape to reach for: it changes no visibility, so
the item stays exactly as private as it was and a default build's API is
untouched. `pub use super::Item` is for a module the crate root declares
privately, where raising the item to `pub` reaches nobody. Never make an item
`pub` inside a module the crate root publishes - that is an API change wearing
a feature's name.

`scripts/generate_internals.py` writes the crate-level re-export from those
declarations, so no two changes edit one file to add theirs; `--check` fails a
stale one. `--all-features` turns the feature on, which is how CI's second lane
runs these tests; a default build compiles `yggdryl::internals` out entirely.

One source file has one test file, so a file that pins both kinds keeps the
reaching half in its own `#[cfg(feature = "internals")] mod internal`. What a
caller can observe then stays at the file's top level and runs in a default
build, and only the module naming `yggdryl::internals` drops out; gating the
whole file would take the caller-facing tests out of the default lane with it.

The bindings hold to the same rule against their own sources, which carry the
crate's layout: `python/tests/` mirrors `python/yggdryl/` and `python/src/` -
one shape, so `charset/__init__.py` and `src/charset.rs` are pinned by the one
`python/tests/charset/test_init.py` - and `node/tests/` mirrors `node/src/` and
the JavaScript files beside it, `node/src/text/line.rs` by
`node/tests/text/line.test.js` and `node/records.js` by
`node/tests/records.test.js`. A folder's `__init__.py` or `mod.rs` is pinned by
`test_init.py` or `index.test.js`, the way a Rust folder's `mod.rs` is pinned by
`mod_.rs`.

## Ownership

- One row schema: a non-null Struct `Field`. Rows canonicalize to ordered
  `Scalar::Serie`; `Scalar::Struct` is a sorted name-to-scalar *input* shape.
  No second row/schema class or accessor; `FieldRecord<'_>` is a borrowed view
  of one row under that field, never a class of its own.
- Codec parsing and local per-event enrichment depend only on that event,
  save the one piece of stream state a parse door carries: the place
  (`seqnum`) it gives each message among the messages of its instant, in
  stream order. They may use `parallel.rs`'s persistent ordered workers when
  parallelism helps, preserving input order with bounded concurrency. Every
  other cross-event state and prior-event order belong only to lifecycle,
  which performs deeper logical enrichment after parsing.
- `Field` alone owns metadata, Arrow IPC dictionary identity and cache-aware
  mutation; `DataType` has none. A `Field` *is* its type: one variant per
  `DataType` shape, each carrying name, nullability, metadata and the Arrow
  projection cache, so a datatype is never stored beside a name and
  `dictionary_id`/`dictionary_is_ordered` exist only on `Field::Dictionary`.
  Protocol metadata is inert `<SCHEME>:<property>` text in one map, the scheme
  spelled upper case as `ARROW:extension:name` and `PARQUET:field_id` are; a protocol
  view borrows a whole `Field` and derefs to it, and typed protocol vocabulary
  (`DIGEST:role`, the `PARTITION:` pair, the `PYTHON:` class declaration) lives
  there, never on `Field`. `Field` owns `FIELD:init`, `FIELD:partition`,
  `alias`, `comment`, `display`, `location` under any key; `PARQUET:field_id`
  is the reserved typed exception.
- `holder` is the only digest role: a declaration says what a field holds or
  derives, never what another contributes - mark one field, leave its sources
  ordinary columns.
- Shared and dispatch enums each live in their named root file, re-exported from
  the crate root: `Charset`, `Codec`, `DataTypeId`, `DataTypeKind`,
  `DigestAlgorithm`, `EdgeAlgorithm`, `IdSource`, `IdType`, `IOKind`, `IOMode`,
  `Level`, `Magic`, `MediaType`, `MimeType`, `Scheme`, `TimeUnit`, `UnionMode`.
  `Timezone` is a datatype of its own, so `timezone.rs` holds it as a type file and the crate
  root re-exports it like `Scalar`. No local copies, no `enums` module.
  `Digest`/`Digester` sit beside `DigestAlgorithm`, `Encoder` beside `Codec`;
  `Scalar` -> `scalar.rs`, the `Holder` variants -> `holder`, record settings ->
  `media`, `FieldPath`/`FieldSegment` -> `expression`, whose grammar already
  writes their steps.
- `IOMode` = `ReadOnly`, `Overwrite`, `Append`, `Merge`, `Random`; operations
  reject modes that do not apply; no alias.
- `DataTypeId` = exact variant, `DataTypeKind` = family, which is the range of
  `DataTypeId` bytes it owns (`range`, `contains`), never a value type. `TimeUnit` is the only
  temporal/interval unit parser and Arrow converter; `MimeType`/`MediaType` own
  MIME parsing, suffix and content-coding inference, preferred extensions;
  `Scheme` owns URI and compatibility scheme vocabulary. `DateTime64::from_text`
  is the one reader of datetime text whose zone the text may or may not
  state - an expiry, a capture, a parameter, a cell - over the ISO 8601
  readers in `temporal.rs`, reading exactly what they read: a blank around
  the text is refused as a datetime's value door refuses it, and a tool's
  habit (the AWS CLI's trailing `UTC`) is taken off by the intake that meets
  it (`auth::instant`); no module pairs those readers for itself.

## Patterns

### One type, one file

A type lives in `rust/src/<type>.rs`, and that one file holds the whole
of it in this order: the **datatype** - the `DataType` constructors and
predicates naming it - then the **field** - its `FieldType` marker and
`TypedField` alias - then the **scalar** - its value, its `Value` impl and the
`Scalar` variant that carries it. Its Arrow reading belongs there too: the
storage it projects to, the extension name it is recognised by, and the cast
that reads it back. Nothing about a currency is anywhere but `ccy.rs`.
A type too big for one file keeps the file and adds a folder of its own name
beside it, never a folder under another type: `mime_type.rs` holds the
`MimeType` value and `mime_type/` its datatype, its registry and its line
classifier; `media_type.rs` and `media_type/datatype.rs` the same. There is no
`types/` folder: a type is a root file, and the root is where a caller finds
it as `yggdryl::<Type>`.

`<type>` is the vocabulary, not the width. Every integer width is one
`IntegerValue` over one set of rules, so `integer.rs` holds all ten; every
registered code is its own standard with its own validity, so `country.rs`,
`ccy.rs`, `isin.rs` and the nine beside them are twelve files, each type
named as its datatype is spelled - `Isin` is `isin`, `Bbg` is `bbg`.

What is shared by several types is a file of its own beside them, never a
folder under one of them: `code.rs` carries the contract every registered code
answers, `wkb.rs` the reader three of them need. A file splits into inline
modules only where the language forces it - a `#[cfg]` cannot gate a loose
group of items, and a section reading `crate::arrow::{Error, Result}` cannot
share a scope with one reading `crate::{Error, Result}`.

### `DataType`, `Field`, `Scalar`

| Concern | Type | Holds |
| --- | --- | --- |
| shape | `DataType` | no name, no nullability, no metadata |
| schema | `Field` = one variant per `DataType` shape, each carrying name + nullable + metadata | a non-null `Field::Struct` is the row schema; `Field::new` takes a `DataType` and `Field::dtype()` answers one |
| descent | `Field::fields`, `get_field_at`, `field_at`, `get_field_by_path` | borrow children out of the variant; never build a `DataType` to reach a child |
| value | `Scalar` | one variant per physical width |
| checked value | `DataType::scalar(v)`, `Field::scalar(v)` | the only way a caller value becomes a stored one |
| narrowed view | `TypedField<K>`, `TypedFieldRef<'_, K>` | a marker validating the variant; parameters stay in the wrapped `Field` |
| field-borrowing value | `FieldScalar<'_>`, `FieldRecord<'_>` | a borrowed `Field` and the value its `scalar` contract answered; `UncheckedFieldScalar<'_>` is the pairing before that proof |
| many values | `Serie` | a schema-free `Run` - what a row is - or the Arrow buffers of one `Field`, nested as `Serie` children; `Scalar::Serie` holds one, every column leaf is a `SerieValue`, identity is the rows alone, and it is the type that reads a cell, casts, or builds an Arrow array or batch |
| many values, chunked | `ChunkedSerie` | `Serie` columns of one `Field` held apart - a chunked array, or a table of one batch per chunk - read across the chunks as one column; identity is the rows alone, `cast` is one plan over every chunk, and `into_serie` is the one join |
| many values, streamed | `SerieReader` | one record `Serie` per batch of a `BatchReader`, under one `ArrowCastPlan` compiled from the reader's schema; `into_arrow_reader` is its transport face and hands the inner reader back untouched when the plan is the identity. A stream is never a `Scalar` |
| Arrow value | `Serie`, `ChunkedSerie`, `SerieReader` | a held column, table or one-row array is a `Serie` - `Scalar::from(serie)` makes it one value and `as_serie` borrows it back, neither reading a row - a held chunked column or table is a `ChunkedSerie`, its arrays or batches kept apart, and a stream is a `SerieReader`, never a `Scalar`; `SerieReader::from_serie` is the one-item reader over a held column and `SerieReader::from_chunked` the reader of a held chunked one's chunks, and `IOMedia::read_serie` answers a `SerieReader` while `write_serie`, `overwrite_serie`, `append_serie` and `merge_serie` take any of the three as one `SerieSource` |

Equivalences a change keeps lossless, in both directions:

- `DataType`/`Field` <-> Arrow, through `from_arrow`/`into_arrow` and the core
  recursive exporters - never a schema rebuilt in a binding.
- `Scalar` <-> Arrow array or scalar, through `Serie::from_scalars(field,
  [value])` and `Serie::from_arrow_array(Some(field), array, options)?.scalar(0)`
  under the exact `Field`, which decides nullability,
  dictionaries, extension identity.
- rows <-> ordered `Scalar::Serie`; named input <-> sorted `Scalar::Struct`
  (`from_record`), canonicalized against the Struct `Field`;
  `Serie::from_scalars(root, rows)` and `Scalar::from(serie)` cross the same
  way, the second zero copy.
- `Serie` <-> Arrow array, batch or reader, through `from_arrow_array`/
  `into_arrow_array`, `from_arrow_batch`/`into_arrow_batch` and
  `from_arrow_reader`/`SerieReader`/`into_arrow_reader`, by sharing buffers.
  Every intake is `(field: Option<&Field>, input, ArrowCastOptions)` and
  reconciles a layout that is not the field's through the one engine; an exact
  layout is the identity plan. No row is read except to prove a leaf whose
  layout is not its datatype's whole contract, once, and not even then when the
  caller's `Proof` certifies it - never on the strength of an extension label.
- `ChunkedSerie` <-> Arrow chunked array or table, through `from_arrow_arrays`/
  `into_arrow_arrays` and `from_arrow_reader`/`into_arrow_reader`, one chunk
  per array or batch, by sharing buffers; `into_serie` is the one join.
- a datatype's canonical default is `default_value`/`is_default_value` - the
  value a declaring protocol's `apply_arrow_batch` replaces.
- widths: a family constructor picks the physical width once, and shared logic
  reads across widths with `as_i128`/`as_u128`, `as_f64`, `as_decimal`, and
  `temporal_unit`/`temporal_timezone`/`temporal_count`; which family is the
  `id`'s range (`family`, `DataTypeKind::contains`), which temporal family
  `id().temporal_family()`.

### Stack: holder -> media -> arrow

| Level | Surface | Answers |
| --- | --- | --- |
| bytes | `IOBase`: `pread`/`pwrite`, `read_all_bytes`, `read_range_bytes`, `append_bytes`, `pstream_bytes`, `read_digest` | positional bytes, digests, bounded streams |
| position | `IOCursor`, `Cursor<H>` | the only place a cursor is retained |
| records | `IOMedia`: `read_arrow_field`, `read_arrow_reader`, `read_serie`, `write_serie`/`overwrite_serie`/`append_serie`/`merge_serie`, `write_arrow_*`, `*_records`, `row_size`, `column_size`, `record_options` | schema, rows, batches, statistics |
| values | `Scalar::from(serie)`, `Scalar::as_serie`, `Serie::from_scalars`, `Serie::scalar` | the `Scalar`/Arrow boundary: a column is one serie value and a value lays out as a column, through `Serie` alone |
| columns | `Serie`: `from_arrow_array`/`from_arrow_batch`/`from_arrow_reader`, `cast`, `into_arrow_*`; `ChunkedSerie` for columns held apart; `SerieReader` for a stream | every Arrow cast and every collection read or written in place |

`IOBase: Send + IOMedia`, so every handle answers records; a media wrapper
implements `overwrite_arrow_reader` and inherits streamed append and merge.
Wrappers compose over a handle, never inside it - `Coded` (coding),
`Transcoded` (charset), `Buffered` (holder), `Hashed` (xxhash), `Counted`
(tests) - each forwarding through
`delegate_iobase!` and overriding only what it changes. Commit cadence belongs to
`RecordOptions` and the write session in `iobase/transfer.rs`
(`ArrowWriteSession::{overwrite,append,merge}` with `push` and `finish`/`abort`),
never to a wrapper's own buffer.

### Struct and row accessors

- Whole value: `read_scalar(field)` / `write_scalar(value)`. Schema alone:
  `read_arrow_field(options)`.
- Rows out: `read_arrow_reader(options)` streams batches; `read_serie(options)`
  answers a `SerieReader`, one record `Serie` per batch - a structured text
  document is the one column its rows parse into, a container the table its
  leaves hold; absent options are the handle's own (`record_options()`).
- Rows in, by shape, each with `overwrite`/`append`/`merge` plus a generic
  `write_*` taking an `IOMode`: `*_arrow_reader` (the streamed primitive),
  `*_arrow_batch` (one batch), `*_records` (a row iterator), `*_serie`
  (one `SerieSource` - a held `Serie`, a `ChunkedSerie` or a `SerieReader`,
  `From` each - written as the batches it already is: a record column one
  batch, a chunk one each, a stream itself, nothing copied or re-landed).
  Absent options are the handle's own for a write too; a structured text
  document takes `overwrite_serie` alone and reads only the declared `field`
  off the options, and a run or a record holding an absent row is refused
  before the destination is touched.
- Navigate a row `Scalar` with `get`, `get_key_str`, `path`, `iter`,
  `sequence_rows`, `record_iter`, and update with `with_field`/`without_field`;
  a row is an ordered sequence, never a map, and `get`, `path` and `iter`
  answer `Cow` - borrowed from a run, built from a column.
- A column read off Arrow is a `Serie`: `len`, `field`, `scalar(i)`, the
  leaf narrowings, `cast`, and `into_arrow_array`/`into_arrow_batch`/
  `into_arrow_reader`/`into_arrow_scalar`; `Scalar::from(serie)` holds it as
  one value. A chunked array or a table kept apart is a `ChunkedSerie`, read
  with the same verbs across its chunks.
- `FieldRecord<'_>` is the row view under one Struct `Field`, borrowing
  the field: cell `i` is a `FieldScalar` borrowing child `i`, built by the
  field's own row canonicalization from a `Sequence` or a `Struct`, read with
  `get`/`get_by_name`/`get_by_index`/`names`/`iter`/`as_str` and subscripts,
  and collapsed with `into_scalar`. A name reaches a cell exactly as
  `Field::index_of` resolves it - by exact match; a folded name or a dotted
  path reaches no cell - and it is not a second schema: it adds no accessor a
  `Field` does not already answer.
- Add no second row type, schema accessor, or per-row map/JSON bridge; a
  binding's row helper closes over one Struct `Field`.

### Intake: accept, resolve, exploit

| Step | Surface | Contract |
| --- | --- | --- |
| accept | `from_str`/`from_*`, `Uri::from_path`, `impl Into<Holder>`, `MimeType`/`MediaType`, `Coded::infer`, `text::io::Plan::infer`, `RecordOptions::for_media_type`, binding coercion (§3, §4) | every documented spelling of one thing, each listed and tested |
| resolve | `DataType::from_str`, `Field::from_str`, `DataType::LOGICAL_NAMES`, `Scalar::dtype`, `inferred_*_field`, `DataType::scalar`/`Field::scalar` | one exact answer or a typed error, computed once |
| carry | `DataType`, `Field`, `Scalar`, `TypedField<K>`, `FieldScalar<'_>`/`FieldRecord<'_>`, the dispatch enums | the proof travels with the value; no later caller re-derives it |
| exploit | `ArrowCastPlan::compile`/`preflight`/`apply`, `SerieReader`, the crate's `PlanCache` and stage plan, the leaf reads (`as_<leaf>`, `values`, `value`, `offsets`, `nulls`) and the reading each leaf resolved where it landed, cached Arrow projections, `as_i128`/`as_u128`/`as_f64`/`as_decimal`/`temporal_*`, `default_value` | schema-dependent work leaves the per-item path |

- Precedence, where the caller did not say: an explicit argument, then a declared
  `Field`, `MediaType`, or path suffix, then one bounded content read - never a
  second read to break a tie, never a host runtime's guess. An input that answers
  none of them names every step that failed.
- Inference reads what a value already is, not what a column could hold:
  `Scalar::dtype` names the variant's own datatype with its width, unit, zone and
  scale intact; children that disagree are an error rather than a widened common
  type, and a null child only makes its field nullable. Rows carrying no schema
  get one from `inferred_scalar_field`/`inferred_array_field`/
  `inferred_struct_field`; rows under a `Field` get that field's answer and never
  an inferred one.
- Resolution is a boundary event, never a per-item one: parse, lookup, layout
  choice, and plan compilation hoist into options, a typed marker, or the
  compiled plan, leaving masks, offsets and buffers to vary per row or batch.
  `TextOptions::autotype` is the shape to copy - capture datatypes settle from
  the pattern before a byte is read. A per-row `from_str`, dictionary rebuild,
  metadata lookup, or format branch is a defect, and the benchmark plus the
  `IOBase` call counts are where it shows.
- **Paths are resolved, never split.** Every path into a nested schema or value
  is a `FieldPath`, parsed once at its boundary by that type's one parser and
  carried resolved. It lives in `expression/`, whose grammar writes the same
  steps and shares the one segment type. Its grammar is the whole vocabulary:
  `.name` for a struct child, `[0]` and `[-1]` for a serie element, `['key']` for
  a map entry, a quoted name for one carrying a dot, so a field named `a.b` has
  exactly one spelling and it is not two levels, and a trailing `as name` saying
  what to call what the path reached. A surface may accept a path as text at its
  own intake and must resolve it there; past that boundary nothing takes a path
  as a string, splits one at a separator, or writes a second path grammar. A
  path used per row, per column, per batch or per node is hoisted out of that
  loop - one parsed inside one is a defect the benchmark and the allocation
  count will show. `FieldPath` is the selector a caller writes; the crate-private
  `Path` cons-list is where a recursive walk reports a failure, and the two never
  merge.
- Nothing re-enters the boundary from inside: no round trip through text, JSON,
  or a host runtime's casting to recover a type the value already carries, and no
  second inference over values a `Field` types.

### Zero copy

Holds, and is asserted with the counting allocator at several corpus sizes -
timing alone proves nothing:

- borrowed views allocate nothing: `as_<noun>` (`as_serie`, `as_field`,
  `as_window`), `TypedFieldRef`, `ProtocolField`; `as_<state>` transforms in
  place and allocates only what the kernel it runs needs; `into_*` is the
  allocating counterpart.
- typing allocates nothing where the proof already exists: `FieldScalar::infer`
  borrows the prebuilt shared field of a leaf datatype, `FieldScalar::new`
  answers a canonical value untouched, and `FieldRecord::get*`/`names`/`iter`
  borrow; `FieldRecord::new` (the cells' `Vec`), `into_scalar`, and `into_str`
  allocate by contract.
- an identity `ArrowCastPlan` hands back the buffers of the `Serie` it was given
  (`Arc::ptr_eq`), a cast onto the `Serie`'s own field is a clone, a
  `SerieReader` over an identity plan hands back its inner reader, and the
  transport form returns the caller's own batch; `Representation::Bits` shares
  the value buffer between two same-width layouts.
- `local/` is memory-mapped, `Buffered<H>` pins pages instead of copying
  them forward, and shared nesting clones a reference while empty collections
  hold no backing.
- a `ChunkedSerie` clone, window or child moves chunk pointers into one vector
  of chunks and one of their ends - a window slicing only the two chunks at
  its edges - and never a row.
- `Serie::slice` of a run shares its values, its start and length moved, and
  a window of a window reads the serie it views, the offsets summed.
- `window_by` moves no row: every window is a view over the serie it cut -
  or a chunked window zero-copy pieces of the chunks, or a stream window the
  landed batch itself wherever it spans one whole - and the record is built
  only when read. The one copy is a held `sorted` gather where the keys
  descend: one take of every column into the serie the windows own; keys
  already in order gather nothing, and a chunked serie regroups runs, never
  rows.
- Python crosses the C Data Interface and PyArrow holders.
- a constant column (`Serie::lit`, `LitSerie`) holds one row however many
  it states: a cell read clones the value, a slice, a take, a filter and a
  reversal move the count, `memory_size` states one row's bytes times the
  rows without building, and the array is built on the first export and
  shared after - a slice of a built lit shares its buffers.
- a spilled column reads as a heap one: the mapping is the buffer's own allocation, so `scalar(i)`, a leaf's `value(i)`, `slice` and `clone` cost exactly what they cost resident, and `resident_size`/`is_spilled` read flags and lengths without allocating; a declaration (`SORT:by`) is written by swapping the record leaf's field - the children and buffers untouched - and read without a row compared, so a sort asking what the root already states is a clone sharing the buffers; a join's pruned probe batch is emitted as slices of itself, and a build of unique keys allocates nothing per row.

Does not hold, and is never claimed: JavaScript interop is copied IPC with
bounded cursors; `read_all_bytes`, any `Vec` return, `into_*`, and text or JSON
rendering allocate by contract.

### Serie is the collection

`Serie` is the one type that reads a cell, casts, or builds an Arrow array or
batch, `ChunkedSerie` the one that holds such columns of one field apart, and
`SerieReader` the one that does it to a stream; `ArrowCastPlan` is the cast
all three run. A held column is a `Serie`, a held chunked column or table a
`ChunkedSerie`, a stream a `SerieReader`; `RecordBatch`, `ArrayRef` and
`BatchReader` are transport, never a second collection API.

| Want | Spell it |
| --- | --- |
| an Arrow array, batch or drained stream as a column | `Serie::from_arrow_array(field, array, options)`, `Serie::from_arrow_batch(root, &batch, options)`, `Serie::from_arrow_reader(root, reader, options)` - `None` is the input's own field, and a layout that is not the field's is cast |
| a chunked array or a table, kept apart | `ChunkedSerie::from_arrow_arrays(field, arrays, options)` - one array per chunk, one plan compiled from the first array's layout - or `ChunkedSerie::from_arrow_reader(root, reader, options)`, one chunk per batch; `ChunkedSerie::from_series(field, chunks, options)` for columns in hand; `into_serie` is the one join |
| a stream, lazily | `SerieReader::from_arrow_reader(root, reader, options)`: one record `Serie` per batch under one plan; `into_arrow_reader` back to a `BatchReader` |
| rows, or the canonical default | `Serie::from_scalars(field, rows)`, `Serie::from_default(field, rows)` |
| one value repeated | `Serie::lit(field, value, rows)`: a constant column holding the value once, proven by the field, laid out only when something exports it; `from_default` is the field's default as one; `as_lit` asks |
| a column under another field | `serie.cast(target, options)` once, `chunked.cast(target, options)` for every chunk by one plan; `ArrowCastPlan::compile(source, target, options)` held and `apply`ed wherever it repeats |
| a typed read | `serie.as_<leaf>()` or `<Leaf as SerieValue>::from_serie(&serie)` once - the leaf names the storage layout, its `id` the datatype - then the leaf's `values`, `value`, `offsets`, `nulls`, `array` per row |
| Arrow out | `into_arrow_array`, `into_arrow_batch`, `into_arrow_reader`, `into_arrow_scalar`; a `ChunkedSerie`'s `into_arrow_arrays` and `into_arrow_reader`, one array or batch per chunk |
| a column as one value | `Scalar::from(serie)`; read back with `as_serie` and `iter` or `sequence_rows` |
| a held column as a stream | `SerieReader::from_serie(serie)`: one record column, a non-record column as the one child of a record; a run is refused. `SerieReader::from_chunked(chunked)` is the same per chunk |
| rows written to a handle, or read off one | `handle.write_serie(source.into(), mode, options)` and `overwrite_serie`, `append_serie`, `merge_serie` - `source` a `Serie`, a `ChunkedSerie` or a `SerieReader` as one `SerieSource` - and `handle.read_serie(options)` for a `SerieReader`; `None` options are the handle's own |
| a text or byte cell's bytes | the storage leaf once (`as_utf8`, `as_binary_string`, `as_fixed_string`, `as_binary`, ...), then its `value(i)`: borrowed where they lie, no value built; inside the crate a reader that takes every text or byte layout checks `is_string_storage`/`is_byte_storage` once and reads `Serie::value_bytes` per row |
| rows in order, unique, reversed, picked or kept | `serie.into_sorted(options)`, `into_unique()`, `into_reversed()`, `into_taken(&indices)`, `into_filtered(&mask)` for a new serie under the same field, the `as_*` forms to bring the serie into that state in place and chain; `sort_indices(options)` for the positions alone, `is_sorted(options)`, `is_unique()`, `unique_count()` to ask without building; a `ChunkedSerie` answers the same, chunk by chunk where a chunk alone can and through its one join where the rows must be seen together; `SortOptions` is the direction and the nulls placement, default ascending with nulls last |
| rows in the order `order by` keys state | `serie.sort_indices_by(by)`, `into_sort_by(by)`, `as_sort_by(by)` - `by` anything `IntoOrderings` reads: one text (`"venue, price desc nulls first"`), texts, `Ordering`s, a `Selector`, a `Scalar` of those shapes - each key a term bound once against `SerieReader::root_of`, compared key by key on the rung its cells answer, stable; a `ChunkedSerie`'s `into_sort_by` the merge, a `WindowSerie`'s over its rows, a `SerieReader`'s `into_sorted`/`into_sort_by` the stream drained, merged and streamed back; the result's root declares the keys |
| the order a record's rows are in, as a fact | `serie.declared_order()`: the `SORT:by` keys its root declares, proven - written by the sorts, kept by a slice, a filter, `into_unique`, an increasing take, flipped by a reversal, cleared by a take out of position or a write that breaks it (every row write compares the rows it touched against their neighbours; a computed key clears on any write), verified where foreign rows land under a declaring root (`from_scalars`, the Arrow doors, a cast onto a declaring target, a stream's batches and their edges, a chunked serie's chunk edges) and refused by row; `is_sorted`, `sort_indices`, `into_sorted` and the `_by` forms answer without a pass where it states what they ask |
| rows out of memory | `serie.spill(&SpillOptions)`, `chunked.spill`, `reader.spill`, with `as_spilled` answering `&mut Self` to chain and `into_spilled` a spilled copy (a reader's consuming it); `resident_size()` and `is_spilled()` to ask; `SpillOptions::install_env` to state the process default every door settles under; a result that must stay resident whatever the default takes its own bound - a join's `JoinOptions::with_spill`, a serie's `spill(&SpillOptions::new().with_byte_size(SpillOptions::NEVER))` after the fact |
| two record columns joined on keys | `serie.join_with(&other, by, how, &options)` - `by` anything `IntoJoinKeys` reads: a bare shared column (`using`, coalesced), `l = r` equalities joined by `and`, texts, `(Term, Term)` pairs, a `Scalar` - `how` a `JoinKind`, the probe side's rows in their own order (pin the build side where the order matters); `chunked.join_with` the batches apart; `reader.join_with(other, ...)` lazily, the held side built first; the plan's `from a join b using (k)` the same engine over two targets with the keys pushed down |
| rows grouped by a key | `serie.partition_by(&keys)` - one `(key, rows)` per distinct key in first-occurrence order, sorted keys cut as zero-copy slices - and `partition_by_paths(&paths)` for a record column keyed by the run of the cells its paths reach; a `ChunkedSerie`'s groups keep each chunk's contribution apart |
| a stretch of rows read or written where they stand | `serie.window(offset, len)` / `window_mut(offset, len)`: a `WindowSerie` / `WindowSerieMut` that reads and writes through the serie on the rebased range, never a second serie; `into_serie` when a serie of the window is wanted |
| rows grouped into windows by key | `serie.window_by(by, sorted)` - `by` a selector bound once, the key column computed once - and its `SerieWindows` walked with `iter()`: one `(key, WindowSerie)` per run of equal adjacent keys in row order, or with `sorted` each key once in key order, the rows gathered once only where the keys descend; each window's `static_values()` its key cells, `windownum` and `rownum`. The same on a `WindowSerie`; `chunked.window_by(by, sorted)` for zero-copy chunk pieces with no record; `reader.window_by(by, sorted)` for one lazy `SerieReader` per window of a stream, read in order, `sorted` verifying rather than reordering |
| the bytes rows occupy | `serie.memory_size()`, `chunked.memory_size()`, `window.memory_size()`: a column's buffers as its own slice counts them, a run's values as the row estimator charges them |

- **One plan per stream, bind or write session.** A `Serie` Arrow door or
  `Serie::cast` inside a batch loop compiles per batch and is a defect. The
  loop holds a `SerieReader`, an `ArrowCastPlan`, or a stage plan
  (the write session's shaping); a bound expression compiles its
  casts at bind; a loop whose batches may change schema holds the crate's
  `PlanCache`, which recompiles only when the source fields change. Its
  `allocations` row proves a thousand batches cost a thousand times one.
- **A column holds only rows its field accepts.** `Rows::row_at` panics on a
  row it was promised, so this is a safety boundary. The landing
  (`serie::arrow::column_of`) proves the projection and absence at every
  level, and reads each row of a leaf whose layout is not its datatype's
  contract once - unless the caller's `Proof` certifies it. Every public child
  remains readable on its own: ancestor-null rows mask aligned children, hidden
  narrow list and map spans are compacted, and dictionary or run-end values are
  masked by visible reachability. A value it refuses is named by the row of the
  landed array it lies in and the path below it, found again on the way out. A `Proof::Proven`
  is passed only for rows the crate laid out (`from_canonical_rows`), a
  selection of a landed column (`repeat`), or a plan node that certifies: an
  ingest that read every value under the target's rule, a contract target, a
  compiled default, or an exact node over a landed `Serie`. **An extension
  label is not a proof**: exact, bit, kernel and byte-bridge nodes that land
  foreign bytes in a non-contract leaf are read. Every `Proof::Proven` site is
  reviewed before a commit.
- **Transport is not landing.** A batch that is only moved - a decoder to a
  caller's reader, a stream to a writer - is reconciled by the plan's transport
  form, reads no row the engine did not already read, and makes no `Serie`
  claim. Rows land in a `Serie`, and are proven there, only where something
  reads their cells.
- **A cast is not a value check.** `DataType::scalar`/`Field::scalar` stay the
  one value contract. A column entering a value (`canonical_sequence`,
  `extend_from_serie`) is read through them, never cast.
- **Errors by side.** A door that takes or answers an Arrow type, or runs the
  engine, returns `crate::arrow::Result`; the `Scalar`-side verbs return
  `crate::Result`; `?` crosses both ways.
- **Other vocabularies keep their shapes.** `apply_arrow_batch`/
  `apply_arrow_reader` on `Field`, the protocols, the expression layer and
  `IORecordOptions`, and every media reader and writer, keep `RecordBatch`/
  `BatchReader` in their signatures and do their per-batch work through a held
  plan and `Serie`.
- **No second cast door.** There is no `cast_arrow_*` on `DataTypeValue` or
  `FieldValue`, no cast function in `arrow/`, and no typed-array trait.

## Public vocabulary

Names describe ownership and return type; alternate-verb aliases are forbidden.
Check a name in `.api-inventory.txt` before writing it; the change that adds or
retires one regenerates it ([Before you push](#before-you-push)).

| Verb | Contract |
| --- | --- |
| `new` | infallible construction from native parts |
| `from_*` | construct or parse a named representation; validate when needed |
| `into_*` | another representation, borrowing or consuming as useful |
| `as_<noun>` | borrowed, allocation-free view (`as_int64`, `as_serie`, `as_window`) |
| `as_<state>` | a participle - `as_sorted`, `as_unique`, `as_reversed`, `as_taken`, `as_filtered` - brings `self` into that state in place and answers `Result<&mut Self>`, so calls chain; it copies nothing it can transform where it stands (a uniquely held buffer is rewritten, a shared one copied once by `Arc::make_mut`), and a refusal leaves `self` as it was |
| `into_<state>` | a new value in that state, `self` untouched (`into_sorted`, `into_unique`, `into_reversed`, `into_taken`, `into_filtered`); the `into_*` rule above, where the representation is the same type in another state |
| `into_struct_<root>` | self when already a struct, else the one-child wrap: `DataType::into_struct_type`, `Field::into_struct_field`, `Scalar::into_struct_scalar` |
| `is_*` / `has_*` | side-effect-free predicate |
| `get*` | borrowed lookup; `get_mut` only where validation/caches cannot be bypassed |
| `set_*` | validated in-place update; failure leaves self unchanged |
| `with_*` | consuming update; `try_` only when the paired setter can fail |
| `clear_*` / `remove_*` | clear a category / remove one item |
| `*Value` | a trait over values; never a type name |
| `*Wire` | the private serde shadow of a type, where one is needed |

No project-defined plain `to_*`; foreign protocols (`ToString`, JS `toString`)
keep their spelling. Implement `From`, `TryFrom`, `FromStr`, `AsRef` where
coherent; bindings redirect through stable inherent methods. Exceptions:

- `field`, never `schema`, in options and accessors; the Python class accessor
  is `into_field`, leaving `field` to the caller's own members.
  `as_<protocol>`/`as_<protocol>_mut` borrow one protocol's view beside the
  runtime-scheme `protocol`/`protocol_mut` pair; `as_field_properties` and
  `as_arrow_properties` are the deliberate spellings in that family.
- A bare `Metadata` snapshot has no field behind it: it answers `ProtocolMetadata`
  and carries no protocol's typed vocabulary.
- `Uri`/`Url`/`Urn` share `from_str`, `from_path`, `from_uri`, `into_json`,
  `into_uri`; `Uri` adds `into_url`/`into_urn`, file values `into_path`.
- Structured text implements in the explicit forms (`from_utf8`, `from_bytes`,
  `from_reader` with their `_all`/iterator/inferred variants; `into_utf8`,
  `into_bytes`, `into_writer`), mirrored by JSON/YAML/TOML with no format
  argument. Each format and direction adds exactly one inferring entry point
  naming the `Scalar` it answers (`from_json_scalar`, `into_json_scalar`,
  field-directed `from_json_scalar_with_field`, the YAML/TOML/XML counterparts),
  re-exported beside `Scalar`; it only coerces and redirects, byte-like input and
  strings are content rather than paths, and it parses, renders, validates, and
  bounds nothing.
- `local::LocalFolder` roots `temporary`, `home`, `config`: `home` reads
  `HOME`, then `USERPROFILE`, failing and naming both when neither is set;
  `config` = `home` + `.config`; `temporary` wraps the platform temporary
  directory. All three construct a handle and create nothing; nothing else
  reaches these directories through `std::env` or concatenation.
- `FieldScalar<'_>` = one borrowed `Field` + the `Scalar` its `scalar` contract
  answered; `FieldRecord<'_>` = one borrowed Struct `Field` + one `FieldScalar`
  per child. `FieldScalar::infer` borrows `DataType::shared_field`, the one
  prebuilt nullable `value` field per leaf datatype - parameter-free leaves in
  a table by `DataTypeId`, parameterized leaves interned under a fixed bound,
  nested and geospatial datatypes never. `parse_str` is the one text-to-typed
  door. Arrow projection routes through the core scalar-array boundary under
  the real field.

## Generic scalar

- `yggdryl::Scalar` is the single cross-platform scalar: no parallel value tree, no
  retired alias.
- Variants are spelled as their datatype is: `Int8`..`Int64`, `UInt8`..`UInt64`,
  `Int128`, `UInt128`; `Float16`, `Float32`, `Float64`; `Decimal32`,
  `Decimal64`, `Decimal128`, `Decimal256` and the fixed `Decimal`, `BigDecimal`; `Date32`, `Date64`;
  `Time32`, `Time64`; `Duration32`, `Duration64`; one `DateTime64`; `Interval`;
  `Geometry`, `Geography`; `Serie`, `SerieView`, `FixedSizeSerie`, `LargeSerie`, `LargeSerieView`, `Map`, `SortedMap`, `Struct`. Temporals keep the `TimeUnit`/`TimeZone` their datatype needs;
  `DateTime64` always has a non-null `TimeZone`, naive spelled `TimeZone::Naive`.
  The wire vocabulary does not follow the spelling: `Scalar::kind()` and the
  serde tags keep the short `i8`, `d128` names they always wrote. The serie
  family is the one whose tags moved with its name: `kind()` and the tag are
  the layout's own name - `serie`, `serie_view`, `fixed_size_serie`,
  `large_serie`, `large_serie_view` - one tag per layout over a run's rows or
  a column's field beside its rows, the payload's shape saying which, and the
  tags it wrote before (`list`, `list_view`, `fixed_size_list`, `large_list`,
  `large_list_view`, a column's `serie`, `list_view_serie`,
  `fixed_size_list_serie`, `large_list_serie`, `large_list_view_serie`) are
  still read, never written ([legacy names](#datatypes-parsers-errors)).
- `Scalar::Struct` is a deterministic sorted name-to-`Scalar` map, resolved to an
  ordered sequence by Struct-field canonicalization; enum scalars keep generic
  enum identity in the smallest lossless integer representation.
- Implement `Clone`, `Debug`, canonical `Display`, `Eq`, total `Ord`, `Hash`,
  serde, arithmetic, conversions wherever semantics exist; float
  equality/order/hash stay mutually consistent; unsupported arithmetic is
  explicit, never a panic. Every immutable public wrapper - Rust and both
  extensions - is hashable when its state has stable equality; copy-on-write
  wrappers go unhashable after mutation only where the language demands it.
- Byte/text/JSON access uses the canonical core codec: borrowing methods never
  allocate, allocating projections are `into_*`, no JSON bridge for Arrow or
  records. Shared nesting uses immutable references, empty collections allocate
  no backing, caller input never reaches `unsafe`, `unwrap`, or panic.
- Rust keeps exact-width variants and constructors, every width a direct
  `Scalar` variant with nothing between, each geospatial reading and each
  registered code likewise - a code is `Scalar::Ccy`, the way its type
  is `DataType::Ccy`. A family is no type but the `DataTypeId` byte range
  its `DataTypeKind` owns: "is it an X" is `family()` or
  `DataTypeKind::X.contains(id)` (`is_integer`, `is_temporal`, ... are that
  check), never a narrowing. Shared logic goes through the cross-width
  readers `as_i128`/`as_u128`, `as_f64`, `as_decimal`, and
  `temporal_unit`/`temporal_timezone`/`temporal_count`, or reads the leaf -
  the matched variant, or its `Value::from_scalar` - through the leaf
  contract it answers (`IntegerValue`, `TemporalValue`, ...); a family
  constructor picks the physical width once.
- A typed view compares, orders, and hashes over `(dtype, value)` - never the
  field's name, nullability, or metadata - so a value is one value whichever
  column holds it, and its `stable_hash` is the value's own. A borrowing view
  implements `Serialize` and never `Deserialize`; `UncheckedFieldScalar` has no
  equality, hash, or serde because its state is unproven.

## Datatypes, parsers, errors

- `DataType::from_str` and `Field::from_str` are the recursive schema grammars;
  bindings pass expressions straight through. Accept canonical plus common
  Arrow/SQL/Hive/Spark/Iceberg forms under an explicit recursion limit; an
  Iceberg type string reads as the datatype `PrimitiveType::into_dtype` maps
  it to, which `rust/tests/iceberg/types.rs` pins.
- `DataType::LOGICAL_NAMES` is the one fallback registry: FIX Latest datatype
  vocabulary plus `mic`, each name resolving to the closest core datatype and
  displaying as it - no variant, no second spelling. Never register a word the
  Arrow/SQL grammar owns. `StringEnum::PREBUILT` keys the ISO code constants
  the registered names prebuild; `StringEnum::from_logical_name` builds the
  enum a field declares from one. A listing is a constant: every reader answers
  the same members.
- `DataTypeId::LEGACY_NAMES` is the one table of the names the serie family
  had before it took its own - `list`, `list_view`, `fixed_size_list`,
  `large_list`, `large_list_view` - and `DataTypeId::from_legacy_name` reads
  it, as `DataTypeId::from_str` and both bindings' datatype ids do. Every
  other door that reads a datatype's name accepts the same five: the type
  grammar (folded like every keyword, so `largelist` and `LARGE-LIST` are
  that spelling, beside the Hive/Spark `array` words), the `DataType`/`Field`
  serde tags, the `Scalar` wire tags and the pickles that re-parse through
  them; an expression document's `[a, b]` constructor, written under `list`
  before, reads under that tag too. Nothing writes one: `as_str`, `Display`, the pretty form,
  `Scalar::kind()` and every tag spell `serie`, `serie_view`,
  `fixed_size_serie`, `large_serie`, `large_serie_view`, so a schema,
  document or pickle written under an old name reads, and writes back under
  the new one. Each old spelling is an accepted one, listed and tested at
  every door.
- Split only at top-level separators, honoring balanced wrappers, quoting, and
  escapes; reject trailing tokens, duplicates, malformed numbers, and invalid
  nullability with byte position and context. `variant(...)` is dense-union input
  sugar; generic decimal/time constructors pick the fitting width, then use the
  explicit implementation.
- One core URI parser owns components and suffixes: platform-independent scheme
  and file-path canonicalization, validated percent escapes, byte offsets in
  errors; bindings never split identifiers. User info splits at the first `:`
  (passwords may contain `:`). S3 authority: the first path part is the hostname
  when it ends `.com`/`.io`, carries a port, is an IP literal, or is
  `localhost`; else it is the bucket. `key` = the path below the bucket as
  spelled, escapes and trailing slash kept; region infers lazily from known AWS
  hosts.
- `DataType::scalar` is the one value contract: check a value against the
  datatype, rewrite it into the representation that datatype declares - integer
  narrowed, decimal at its scale, temporal at its unit, ASCII trimmed of padding
  - return an unchanged value untouched. `Field::scalar` adds nullability and
  name. Every caller value becoming a stored one goes through them: never a
  synthetic row around one value, never a re-check of what `scalar` answered.
- Errors are typed variants carrying expected, actual, and location (nested path,
  byte position, batch index, URL), canonical formatting, bounded user text, the
  shared diff renderer; mutations fail atomically. `yggdryl::Error` = core
  failures, `yggdryl::arrow::Error` = runtime boundaries, external chains
  preserved.

## Storage: IOBase

- Positional: `pread`/`pwrite` are the primitives; whole reads, streams,
  compression, records, media derive from them. No second storage trait, no
  hidden cursor.
- **Call count is the cost model**, not bytes - one call = one object-store round
  trip, one syscall, one lock per wrapper. Issue the fewest that answer the
  operation: slice a later step's needs out of a read already returned; answer
  from an index, listing, or parsed footer; record what a write knows instead of
  reading it back; skip a call the state proves is a no-op.
- An answer only the store can give about one resource (a member's data offset, a
  footer's length) is read once and shared by every handle on it; a wrapper
  keeping its own copy hides the call.
- Every derived surface states its cost in call counts, pins it in
  `rust/tests/iobase_calls.rs`, and reports it in the `holder` benchmark beside
  the timing; adding a call means editing the assertion naming it.
  `holder::counted::Counted` tallies forwarded calls by name, the object
  backend's `Stats` counts the requests one call becomes.
- Derived reads and appends name the core type they answer, since the same verbs
  address rows: `read_all_bytes`, `read_range_bytes`, `write_all_bytes`,
  `append_bytes`, `read_scalar`, `read_arrow_reader`. Bare `read`, `write`,
  `append`, `read_range` are never core names. A binding may keep a runtime
  spelling that still names the type (`read_bytes`) plus one inferring entry
  point over the explicit method.
- Lazy construction: missing reads return empty/zero, writes create resource and
  parents on first mutation, `media_type` invalidates when bytes change.
  `IOKind` is authoritative - `is_container`, `is_atomic`, `is_tabular`, `is_io`
  derive from root kind/media behavior, never ad-hoc matching.
- `pstream_bytes(position, batch_size)` streams bounded chunks from an explicit
  start with no retained page cache; `stream_bytes(batch_size)` owns only its
  cursor. Both fuse after error and serve codecs, line reconstruction, parsers,
  Arrow readers. Compressed streams keep only decoder state and the current
  chunk; line readers keep only the fragment joining a line across chunks.
- `clear` empties and preserves, `remove(recursive)` deletes; both act directly,
  map only backend not-found to success, never pre-probe, and wrappers drop
  pending writes and caches so a flush cannot resurrect deleted content.
- `pwrite` stages, `flush`/`close` publish; whole byte writes and ordinary record
  overwrite/append flush on completion. `open` caches expensive metadata for its
  scope, `close` publishes and drops it, closed reads are fresh, wrappers use
  `delegate_iobase!` and override only changed behavior.
- `Buffered<H>` is idempotent, bounded by bytes and last-access TTL, writes
  through, invalidates touched pages, pins the first and current final page. No
  cache crate, no background thread.

### Existence and iteration

- EAFP: act once, use the typed result; never guard with `exists`, `is_dir`,
  `contains`, `mkdir`, `ensure`, or ancestry walks. Creation is a write
  consequence - normalize backend absence/conflict once at the boundary, repair
  absence and retry the original act at most once.
- `create` derives conflict from the attempt, `open_or_create` absorbs it, `get`
  raises absence; existence queries are public answers, never internal guards.
  No compare-and-swap: concurrent creation converges or returns typed conflict,
  never silently selects or corrupts.
- Listings are deterministic lazy `Result` iterators fused after first error;
  recursive walks hold a bounded frontier, not results; owned reports only where
  the operation bounds them. Object-safe traits return one named iterator type
  per item kind; bindings expose native lazy protocols without collecting;
  benchmarks measure time to first item and full drain.

### ZIP (`zip/`)

An archive is a file system inside one file, so it supplies the backend roles
under the names its own index has: `ZipNode` is a prefix of that index,
`ZipLeaf` is one entry in it, `ZipPath` resolves to whichever is there.

- Codings are `Codec::Identity`, `Codec::Deflate`, `Codec::Zstd` and nothing
  else spells one; the archive adds no second coding dispatcher, and gzip and
  zlib are refused by name because their framing wraps a whole resource.
- A member is addressed by the archive's URL with its path in the fragment.
  An archive inside an archive continues that fragment past a `//` marker,
  which is a spelling no canonical member path can claim; the marker with
  nothing after it is the mounted archive, and the same fragment without it is
  the member whose bytes hold it.
- A compressed member is written as units a stated stride apart, and its
  central record carries the map of where they begin under this crate's own
  extra field id `0x5967`. A read decodes one unit, not the whole prefix; a
  point is proven against the coding's own evidence before it is used, and a
  member another writer compressed maps nothing and says so.
- One member writer, and it streams: nothing holds a member whole, the header
  reserves the sizes a stream does not know yet, and the index learns about a
  member only once its bytes are in the handle.
- `ZipArchive::handle_reads`/`handle_writes` count what the backend asked of the
  handle beneath it; the cost model in the ZIP section of `docs/holder/index.md` is stated
  and asserted in those terms.

### HTTP (`http/`, non-default `http`, `http2` and `http3` features)

A resource an `http` or `https` URL names is a `Request` leaf answering every
`IOBase` verb, and each verb is a stated number of requests the accounting
tests in `rust/tests/http/` assert exactly:

| Operation | Requests |
| --- | --- |
| building a session or a request, resolving a child | 0 |
| `pread`, `read_range_bytes`, `read_range_digest` | 1 ranged `GET` |
| `read_all_bytes`, `read_digest`, a `pstream_bytes` drain | 1 `GET` + 1 per resume |
| `size`, `mtime`, `kind` while closed | 1 `HEAD`; 0 while open |
| `write_all_bytes`, `clear` | 1 `PUT` |
| `pwrite` then `flush`, `append_bytes` | 1 `GET` + 1 `PUT` |
| `remove` | 1 `DELETE`; a `404` is success |
| `send`, `stream` | 1 per attempt + 1 per redirect hop + 1 per resume |
| a closed `Response` or `Stream` read again, or `open` | 1 ranged `GET` at the cursor |
| `pages`, a paginated `read_arrow_reader` | 1 `GET` per page |

- A request goes out through `Session::send`, or `Request::send_reader` for
  a body read once from the caller's reader; everything it carries beyond
  its own headers - defaults, authorization, cookies, `Accept-Encoding`,
  `User-Agent` - is merged for both by `Session::headers_for` and nowhere
  else. The pool's knobs are the client's: a session over a client states
  none of them otherwise (`Session::with_client`). A ranged
  or streamed request asks for `identity`, so a byte offset means the same on
  both ends; content codings are decoded by the crate's `Codec`, never by the
  transport.
- A handle learns from the answers it already has: `Content-Type` once
  (a declared media type wins, the URL's suffix answers until then),
  `Content-Length` and `Content-Range` for the size while open,
  `Last-Modified` for `mtime`. It never asks a second question to learn one.
- Resume, never splice: a cut body re-issues `Range: bytes=<delivered>-` with
  `If-Range` naming the first answer's strong `ETag` or `Last-Modified`, only
  for a successful uncoded `GET` - nothing else is asked for twice - only
  when the first answer offered ranges, and only while consecutive failures
  stay under `max_attempts` and `Stream::MAX_RESUMES` re-opens in all; a
  resumed `206` must state in `Content-Range` that it starts at the cursor;
  a resource whose validator moved is `Error::Conflict`. Every whole-body
  read goes through that stream, under the same rules, and a `Response` or
  `Stream` stands alone - request and session carried - so `close` lets go
  of the transfer and keeps the cursor, and the next read re-opens there or
  is refused by name.
- Retry only what cannot do harm twice: a retryable status only for an
  idempotent request, a transport failure for an idempotent request or a
  request no connection took (`retry::is_unsent`); `Retry-After` (delta or HTTP-date) waited up
  to `max_pause` and never past it; one token budget per client. A redirect to
  another origin carries no credential the caller stated, and what a
  `with_attempt_headers` hook makes is one: the hook is not called for that
  hop. Idempotent is the
  method's word unless the request states its own (`Request::with_idempotent`),
  and a request states what else the client cannot know of it -
  `with_attempt_headers` (made fresh per attempt from the `Attempt` as it is
  about to go out: its number, the hop's method and URL, the headers already
  on it and the body, none for a streamed one), `with_max_attempts`,
  `with_connect_timeout`, `with_deadline`, `with_retry_on` (a bounded read of
  a failing body, asked only where a retry could follow) and `with_direct`
  (past any proxy) - each read by the one retry loop in `Client::execute`,
  never by a loop of the caller's. Beside the retry rule sits the
  crate-private resend rule (`with_resend_on`, what `with_sigv4` reads a
  refused key by): a `4xx` it says a second attempt would mend is sent again
  once per hop, whatever the method, with no pause and nothing drawn from
  the budget, and never to another origin. `is_unanswered` tells a transport failure
  in which no head was read from any answer.
- The proxy is chosen per request: a named `proxy` wins; else, reading the
  environment, `no_proxy`, then the scheme's `http_proxy`/`https_proxy`,
  then `all_proxy`, lower case first, upper-case `HTTP_PROXY` unread under
  CGI - read at send time so a changed environment is followed, the parse
  memoized by value (`http/proxy.rs`), a SOCKS proxy refused by name; a
  request naming no credential takes its host's `.netrc` entry where the
  options' `netrc` says so - `HttpOptions::with_netrc`, the property
  `netrc`, following `read_environment` until stated - the file parsed once
  per version (`http/netrc.rs`).
- `Client::attempt` is the one place a version is chosen (`http/framed.rs`):
  `http_version` against what the client learned of the origin - ALPN `h2`
  over TLS, `h2c` by prior knowledge only when asked, HTTP/3 when asked or
  advertised in `Alt-Svc` on the origin's own host - each refusal a fallback
  in the same attempt, remembered per origin in a bounded map, a proxied
  request always HTTP/1.1. Everything above it - retries, redirects, cookies,
  resume, pages, `send_all` - reads the same `Answer` whatever the version,
  and HTTP/2 and HTTP/3 failures are spelled in the retry rules' own
  vocabulary: a refused or never-run stream is unsent, a reset mid-body is a
  severed transfer a `Stream` resumes.
- HTTP/2 and HTTP/3 hold one multiplexed connection per origin, driven by the
  one private runtime in `http/runtime.rs`; a blocking call polls its own
  future on its own thread, parked between wakes, so nothing crosses a thread
  per chunk and no caller brings a runtime. Nothing else in the crate starts
  a runtime or blocks one of its workers. A QUIC stream's receive window is
  1 MiB on both ends (`h3::STREAM_WINDOW`), because the window is what bounds
  the gaps a lossy path leaves in a stream and quinn closes a connection
  whose stream lies in more than 1024 pieces; a constant assertion holds the
  two together.
- `Server` answers HTTP/2 wherever a connection opens with the preface, and
  with `http3` on, HTTP/3 on the UDP twin and TLS offering `h2` on the TCP
  port under a certificate it signs itself; every framed request reaches the
  same `dispatch` - routes, mounts, faults, the log - as an HTTP/1.1 one, a
  cut answer's stream reset where it stops.
- A route handler's answer is bytes in hand or a body `Response::with_writer`
  runs as it is sent, never held whole: chunked on HTTP/1.1
  (`Transfer-Encoding: chunked`, no `Content-Length`), as data frames with no
  `Content-Length` on HTTP/2 and HTTP/3, its writes gathered
  `DEFAULT_STREAM_BATCH_SIZE` at a time, the writer run once after the
  handler returns.
  A `HEAD` never runs it and states neither header; `Fault::CutBodyAt(n)`
  writes `n` body bytes then closes the connection (HTTP/1.1, no final
  chunk) or resets the stream (HTTP/2, HTTP/3), and a writer returning `Err`
  severs the transfer the same way. `respond` with a written response writes
  it once, at registration.
- Routing: a `HEAD` no route names answers as its `GET` route would, headers
  included and no body; a path with routes but none for the request's method
  is `405` with `Allow` listing the routed methods in `Method::ALL` order,
  `HEAD` inserted beside a routed `GET`, the text body naming the method,
  the path and what is routed there.
- `Expect: 100-continue` earns its interim `100 Continue` only for an
  HTTP/1.1 request whose body may carry a byte, and only once the head has
  passed every check the head alone refuses a body on - a bad
  `Transfer-Encoding` or `Content-Length` is `400`, a stated length over
  `max_body_size` is `413` - which answer their final status with no `100`;
  a chunked body states no length, so one past the bound is `413` as it is
  read, after the `100`. HTTP/1.0 ignores the header silently, and any other
  `Expect` value is `417` and closes the connection.
- `wire::parse_response` reads past every interim `1xx` answer other than
  `101` to the final one, so `Response::from_bytes` answers what the
  exchange ended on; `101` is final, and `into_bytes` renders only the last
  answer.
- `ServerOptions::with_trace(folder)` writes every exchange under `folder`
  as `NNNN-request.http`/`NNNN-response.http`, `message/http` documents
  numbered from `0000` in the order the requests' heads arrived, across
  every connection: an HTTP/1.1 exchange byte for byte as it crossed the wire -
  the interim `100 Continue` and the chunked framing included - and an
  HTTP/2 or HTTP/3 one rendered as the HTTP/1.1 message its frames carried,
  a request refused before it is routed included;
  a connection that sends nothing writes no files, a request cut short
  writes the request bytes that arrived and an empty response file, and a
  trace write that fails closes the connection after the answer.
- `Session::send_all` is a stream: the source is pulled only as far as the
  requests in flight and the walk is `Send + 'static`; a binding holds it and
  pulls one answer per step, never collecting.
- Pagination is one ladder, `Pagination::Auto`: the `Link` header, the
  next-page headers, a next URL in the body, a cursor sent back under the
  parameter the request already carries; a visited URL, `has_more` false or an
  empty page ends the walk, and a failed page resumes from its own request,
  never from the first. `Pages` lays every page out through `Serie` under the
  root the first page infers or the caller declares.
- `HttpOptions::from_properties` is the one property door, and
  `Holder::from_url` routes `http`/`https` through it; a name it does not know
  is ignored and a value it cannot read is refused naming the property.
- `Server` is the crate's own answer to its own client: every client feature
  is tested against it, a mounted leaf is never read whole - a child streams
  through `pstream_bytes`, the holder at the prefix one `read_range_bytes` per
  batch under its lock, ending the body where a write moves it - and faults are injected per path rather than faked by a
  second implementation. It stays bounded against peers it does not trust -
  `max_connections`, a head deadline, a write timeout, a capped request log,
  `TCP_NODELAY` - and `with_tunnel` makes it the forward proxy the client's
  proxy handling is tested through.

### Object stores (`s3/`, non-default `s3` feature)

One backend, one location/container/leaf trio, three stores: Amazon S3, Google
Cloud Storage, Azure Blob Storage. Each REST API is spoken directly - SigV4,
OAuth 2.0 bearer tokens, Azure Shared Key over a synchronous HTTP/1.1 client -
with no SDK, runtime, or object-store layer.

`Provider` is the sole dispatcher: one value says which store answers, and every
place the three differ reads it and nothing else. A dialect owns only what its
store spells for itself - `aws/` the S3 request knobs and the S3 XML; `google/`
the Application Default Credentials chain, the RS256 assertion, and the JSON
API; `azure/` the Shared Key signature, the SAS and bearer paths, and the Blob
XML. Who an S3 request signs as is not the dialect's: it is the root `aws/`
module's `Session`, carried by `S3Options::with_session`, narrowed by what the
options say explicitly (anonymous, a pair, a pair the location carries) and
sealed by `with_environment(false)`; the client reads the region, the endpoint
(`AWS_ENDPOINT_URL_S3`, the profile's `[services]` entry), the addressing
style, the FIPS and dual-stack hosts and the payload-signing policy off it.
When a store refuses the key an attempt was signed with - read off that
attempt's own `Authorization` - the client tells the session and signs once
more, once, only if the session then answers another set: `ExpiredToken` and
`TokenRefreshRequired` hold the key refused for good, `InvalidAccessKeyId`
and `InvalidToken` for a pause or until the shared files move, and a `HEAD`
answered a bodyless `400` or `403` while it carried a session token has the
sources read again with nothing held against the key. The S3 backend's
`xml.rs` holds the `<Error>` document both XML stores answer with over the
crate's `xml/scanner.rs`; `answer.rs` holds what an answer *says* in shapes no store
owns, so the transport, the retry, the staging model, the listing pipeline and
the three roles are written once.

`S3Options` holds what all three stores have; `AwsOptions`, `GoogleOptions`
and `AzureOptions` hold what one has, so a knob has exactly one owner. A
property name two stores both have is applied to both, because only the store
that answers reads its own options.

Each method states its request count and the accounting tests assert it exactly:

| Operation | S3 | Google | Azure |
| --- | --- | --- | --- |
| ranged read | 1 ranged `GET` | 1 `GET` with `alt=media` | 1 `GET` with `x-ms-range` |
| whole read, full stream drain | 1 `GET` | 1 `GET` | 1 `GET` |
| whole write | 1 `PUT` | 1 `multipart/related` `POST` | 1 `PUT` |
| large write | parts + 2 | chunks + 1 | blocks + 1 |
| listing, flat or recursive | 1 per 1000 entries | 1 per 1000 | 1 per 1000 |
| prefix removal | 1 listing + 1 bulk delete per 1000 keys | per 100 | per 256 |
| construction, child resolution, trailing-slash location | 0 | 0 | 0 |

A listing states every entry's size, so a listed object never re-asks; `open`
caches metadata, never bytes; pooled connections mean a body is always drained;
a 3xx is never followed, because the one redirect that matters corrects the
signing region here and Google reuses 308 for a chunk that landed. Payload
signing is AWS's alone: signed over plain HTTP, unsigned over HTTPS.

## IOMedia and records

- `IOMedia` owns field/datatype, record, Arrow, expression, applier, row/column
  size, specialized tabular behavior; `IOBase` implements it; no separate tabular
  trait.
- Arrow primitives: `read_arrow_reader(options)`, required
  `overwrite_arrow_reader(reader, options)`, default streamed
  `append_arrow_reader`/`merge_arrow_reader`, generic
  `write_arrow_reader(batches, mode, options)`, and `read_serie`/`write_serie`
  over them for the three shapes a `SerieSource` holds. Table, record-batch,
  row-record entry points infer or wrap input into that pipeline; nothing
  streamable takes or returns `Vec` batches.
- Record options are the split sections of one `Plan`, stored apart for
  isolation: `name` (default `media::DEFAULT_ROOT_NAME`) and the declared
  `field` (undeclared = inferred; the plan's `create` section), `filter` (its
  `where`), `select` (its `select`), `merge_by` (its `upsert by`), and the row
  bounds (its `limit`). `plan()` composes them and `set_plan` splits a plan, a
  clause, a field or text back into them; no `dtype`, `metadata`, name lists or
  partition pairs of their own. A media reads what it needs from the sections
  through pre-implemented methods - `partition_pairs` for pruning,
  `apply_columns` for projection pushdown, `apply_arrow_expressions` for the
  filter-then-select seam - never from a second property. Reads project in the
  encoding and cast each batch; writes cast once, pop the field before delegating
  to overwrite, never materialize the stream.
- `options.commit_batch_num`: `N` publishes every `N` whole batches of the shaped
  stream plus the remainder, holding at most one cadence and cutting no batch;
  unset is the destination's own cadence - a leaf or a partitioned folder
  publishes once when the source ends (a leaf append is a rewrite), an Iceberg
  table commits once when the source ends too, every partition's rows held as
  spilled chunks under the process spill bound until then, so an overwrite of
  any length is one atomic snapshot, the push-based write session every
  `DEFAULT_COMMIT_BYTE_SIZE`; what a cadence holds between publications is
  held under the process spill bound, its heaviest batches spilled first; the
  first overwrite commit overwrites, later ones append; failure leaves
  published prefixes visible.
- `options.num_threads`: the parts a write of several parts runs at once - an
  Iceberg commit's partition groups - the explicit layer over the table's
  `write.parallelism`, else `read.parallelism`, else the host; zero is refused
  by `require_num_threads` naming `$.num_threads` at every write door - the
  streamed primitives, the row doors, a table's - beside the zero-cadence
  refusal and before a write pulls its source, and the count is inside the
  options' identity.
- Overwrite replaces rows under the stored field - a leaf whole, a
  partitioned folder or table only the partitions the incoming rows reach
  and the ones its `where` pins, each the first time a commit of the write
  reaches it and appended to by every later one, so a partition nothing
  reaches keeps its leaves, a source with no row replaces nothing outside
  that scope, and `clear` is what empties; a folder holds each commit's
  rows split by partition under the process spill bound and writes every
  leaf it reaches once; append retains stored rows;
  merge needs a non-empty `merge_by` selector, updates matches, appends misses,
  and streams incoming batches - no positional upsert. A `Plan` names the same
  four writes as `insert overwrite`, `insert into`, `upsert into ... by (...)`
  and `delete from ... where`, with every common alias read and one canonical
  spelling printed; `Plan::execute` reads a source through `Holder::from_url`
  with the read sections pushed into the media. `row_size`/`column_size` are lazy
  cached metadata from cheap media answers, never a full read.
- Encoding comes from `MediaType` through `RecordOptions`, with no format
  argument; generic `write_*` takes an `IOMode` and redirects to specialized core
  paths.
- Plain-text rows are the fifteen element and event columns `ElementColumn::ALL` and `EventColumn::ALL` name -
  the line as the event it is, the six element columns from `curruuid`
  first, then the nine event columns from `currunix` to `state` last - then
  required `body: utf8`, then one column per row-header capture that feeds
  no event fact: a capture named `state`, `creaunix`, `recdunix`,
  `exprunix`, `prevunix`, `snapunix` or `prevuuid`, or `mtime` under
  `parse_mtime`, states that fact in the event's own column, and a capture
  named `execunix` is an ordinary one: a line is no market element. The event
  states every fact a column used to repeat and no column repeats one: the
  object a line came from is `crosscode`, so `crosshashcode` is the XXH3-64 of
  that URL string and `crossuuid` derives from it; the row number under
  `TextOptions.start_rownum` is `seqnum`, null at zero and refused where a
  count cannot hold it; when the record was written is `currunix`, the row
  header's `mtime` capture where `parse_mtime` declares one and `IOBase::mtime`
  where it does not or the header does not match the line; the instant the read
  dated the line cut before it by is `prevunix` - none for an object's first
  line or after an undated one, a `prevunix` capture standing, each object read
  starting again - and `prevuuid` is never filled.
  `body` is the line
  past what the header matched - the header comes off where the line is made,
  so the captures are the line's and the body is the payload, empty exactly
  where the header consumed the line - and a line is never empty as cut,
  refused at every door that sets one, a blank physical line being a
  separator rather than a record; its bytes are
  decoded at the transport in the charset the handle's media type declares
  other than UTF-8 or US-ASCII, and otherwise once where the line is made,
  each byte that is not UTF-8 read as the Windows-1252 character it is,
  `TextLine::decoded_byte_size` counting them. `currhashcode` is the XXH3-64
  of the body and nothing else - what anyone holding the `body` cell
  computes - so two lines of byte-identical bodies share it wherever they
  were read: the captures a header lifted, the state, the predecessor and the
  cross code stay out of it, and the instant, the row number and the cross
  hash reach `curruuid` beside it, through `Event::time_uuid`. The row header is the only thing that lifts a column out of a
  line. Flat `TextOptions` owns named `rowheader`
  captures, edge-only regex stripping, a line separator, and syntax-directed
  `autotype` via `DataType::from_regex`, so the full source field is known before
  a read. `timezone` stays a shared `RecordOptions` accessor over offset-free
  datetime captures; writes consume only non-null, non-empty `utf8` `body`.
  `read_text_lines` is the decode, not the query: it answers every line under
  the options it is given, and the `where`, `select` and row bounds are the
  record surface's - `apply_arrow_expressions` over the rows the lines become -
  because a `where` may name a column the `select` builds and no line states
  one. `into_arrow_batch`/`into_arrow_reader` sit on the same side of that seam.
- Content coding belongs to the handle: reject outer compression for formats that
  compress internally, such as Parquet.

### Paths and partitions

- Globs use `Url::is_glob`, `glob_parts`, `matches_glob`, descending fixed
  prefixes before listing. Hive paths use `Url::hive_partitions`,
  `hive_partitions_under`, lazy `children_where`; a table-format folder routes
  through its metadata before ordinary leaves.
- Stored `column=value` layout is authoritative, else marked root fields decide;
  contradictions are typed errors naming both declarations.
  `media::partition::partition_text` is the only partition renderer: partition
  columns move between paths and rows through one typed implementation.
- A struct declares how its rows partition as `PARTITION:by` and the order
  they keep as `SORT:by`, each a JSON array of expression texts in the one
  shape every `by` property has (`DIGEST:by`, `TRANSFORM:by` too), each entry
  read by its key's grammar - a projection, an `order by` key, a term - and
  stored as that grammar spells it. A bare `PARTITION:by` entry is an
  identity partition; any other is a derived one, named by its alias or
  `{source}_{function}` in Iceberg's singular (`ts_year`, `ts_minutes`,
  `name_truncate`). `Field::with_partition_by` is the one door from the
  declaration to the layout: it stores the key, marks the identity columns
  and materializes every derived entry as a marked `TRANSFORM:` column typed
  by its term; `with_partition_fields` is it over bare columns, and
  `partition_by` reads the key, else the marks. A mark the declaration does
  not name is refused naming both; a declared column may be unmarked or
  absent, because a leaf stores the rows minus the partition columns under
  the whole declaration and an Iceberg table keeps a derived value in its
  manifest. `apply_arrow_batch` is the one verb every declaring protocol
  answers - it walks the Structs that protocol declares and leaves a column
  holding anything but its canonical default alone - and a caller asks each
  protocol it wants, transform before digest. `Field`'s own
  `apply_arrow_batch` is the cast alone, as is every read and write a
  declared or stored field shapes: a derived or holder column the rows do
  not carry lands null, or is refused by path where it is required. The
  transform view fills a derived column only where it is absent or holds
  its canonical default in every row - a column with any written row is
  left whole - through `apply_arrow_batch`, or `apply_arrow_reader` under
  one plan for a stream. The one writer that derives is the table that owns
  the declaration: an Iceberg table computes the `TRANSFORM:` columns its
  stored schema declares for every row written to it - after the options'
  declared field, before their clauses - keeps each term as the table
  property `yggdryl.transform.<column>`, and reads a derived `PARTITION:by`
  entry no Iceberg transform spells as the identity partition of the column
  `with_partition_by` materialized.
  `Plan::from_field` moves `SORT:by` into its `order by` section and writes
  it back; an Iceberg spec and sort order read the two keys
  (`PartitionSpec::from_schema`, `SortOrder::from_schema`) and write them
  (`mark_partitions`), so `IcebergTable::schema()` reports both.

### Digests

- `stable_hash` = XXH3-64 over the canonical feed everywhere: no second hash
  family, no second spelling. The pinned xxHash dependency's types never appear in
  a public signature, doc example, error, or binding.
- Integer holders take signed or unsigned storage at the algorithm's exact width;
  signed is a bit-preserving view, normalized to unsigned before feed on nested
  reuse.
- A row digest reads direct Struct children in declaration order.
  `DIGEST:by` is the exact input, terms resolved against its own Struct: a
  bare column path feeds the column's own buffers, any other term is bound
  once per plan and evaluated per batch into a column fed beside them; `["*"]`
  and an absent list both select every field except a holder; `"*"` may not
  travel beside a term. A holder never feeds itself back; a selected Struct
  holding exactly one direct holder feeds that holder's value instead of being
  hashed again. Framing stays an ordered sequence, empty included.

## Media and table formats

- A media wrapper delegates raw bytes and implements the shared `IOMedia`
  primitives; metadata caches live only between `open` and `close`, and metadata
  reads never decode rows.
- Declared fields drive native projection and one shared cast plan; exact casts
  reuse arrays. Benchmarks run release builds against a trusted native
  implementation on the same payload and wire; regenerate results, never edit
  numbers.
- A skipped half of an exchange check is not a pass - wire protocols included: a
  hand-written fake store is written from the same reading of the API as the
  client, so both can agree and both be wrong.

### Iceberg

`docs/media/iceberg.md` documents the format surface; these bind a change to
`iceberg/`.

- A table is a folder reached only through `IOBase`: metadata = core JSON,
  manifests = core Avro, data = core Parquet, and no Iceberg/Avro/catalog
  dependency whose I/O or Arrow model conflicts.
- Plan snapshot -> manifest list -> manifest -> files from metadata, never by
  walking `data/`; prune on partition summaries and safe statistics, resolve
  residuals by row filtering, report read/skipped counts, and keep parallel scans
  in plan order - they differ from sequential only in speed.
- `SchemaUpdate` owns evolution: preserve field IDs and never reuse dropped ones;
  promotions are Int32->Int64, Float32->Float64, same-scale decimal widening,
  and v3's `unknown` - a variant its field declares `unknown` - to any type;
  validate loaded metadata and every commit.
- An overwrite replaces the partitions its rows fall in: the plan opens
  those partitions alone, drops their live files and carries every other
  file with its row lineage, on every format version; a write cut into
  cadences replaces a partition on the first commit that reaches it and
  appends after. A stated `where` scope, an unpartitioned table and
  `commit_overwrite_where(&[], ..)` replace that scope whole. No write
  compacts - `compact` is the one maintenance door - and a data directory
  spells its partition tuple with `[A-Za-z0-9._+-]` alone, any other
  character `_`, because the manifest is the authority on the value.
- The official manifest parser converts a partition tuple by its field
  types and reads no long as a nanosecond timestamp: a manifest whose spec
  holds an identity field over one is parsed through a view spelling that
  column `long`, and its tuple is read back under the header as written.
- `IcebergTable` answers the same `IOMedia` surface as a leaf - a table format is a
  media wrapper, not a second record API.
- A record read (`read_serie`, `read_arrow_reader` its transport) yields
  partition after partition in ascending tuple order, each sorted by the
  table's order through the two calls a write sorts a group by
  (`keeps_order`, else `ChunkedSerie::into_sort_by`), lazily - one partition
  held at once, under the process spill bound, none opened before the one
  before it is yielded - its files opened by their leading key's manifest
  bound so files that follow one another are never merged; its root, and
  `read_arrow_field`, declare the order that proves only where the spec's
  identity columns lead the order ascending and the metadata holds one spec,
  a transform the last key declared, and a key the stored values order (an
  identity partition column, a transform) only where the root reads its
  column as stored. With no partition to sort the transport is the scan
  itself, nothing landed; `read_serie` behind a `select` or a bound lands the
  shaped stream once and carries the declaration without reading it again.
  The scan doors, compaction and a merge's stored side keep plan order.
- A write through `IOMedia` commits once when its source ends unless
  `commit_batch_num` paces it: every partition's rows held as spilled chunks
  under the process spill bound, each group sorted as a whole by the table's
  sort order (stable, `ChunkedSerie::into_sort_by`) unless it is already in
  it - a proven `SORT:by` on the stream's root, or read once chunk by chunk -
  the groups written on `num_threads` threads at once, and
  `write.target-file-size-bytes` cutting files, never commits.
- Emit bounds only where Parquet and Iceberg encodings agree: missing bounds cost
  performance, wrong bounds violate correctness.
- Options resolve explicit -> table property -> default in `IcebergOptions`, one
  resolver per key. The retry gate rebases append and metadata-only commits only;
  overwrite/merge/compact restore state on conflict, and a failed commit may
  leave unreferenced files.

## Charsets

`docs/media/charsets.md` documents the surface; these bind a change to `charset.rs`,
`charset/`, the three charset files `utf8.rs`, `ascii.rs` and `cp1252.rs`, and
every byte that becomes text anywhere else.

- **Text crosses the boundary once.** A byte payload is decoded at intake -
  by a `Transcoded` handle, by `text::io::Plan`, by the record reader's
  transport from the handle's media type, by a `string(...)` column's own
  value contract, or by a direct `Charset::decode` - and everything past
  that point is `str` or UTF-8 bytes. Nothing re-decodes,
  and no cast, digest, or record layer branches on a charset per row. A layer
  that wants a charset argument wants the wrong seam: wrap the handle instead.
  A `string(...)` value does carry the charset it is *written* in, so it goes
  back out the way it came - but it holds UTF-8 while it is here, and `as_str`
  on it is infallible.
- `Charset` is the one vocabulary and the only place a name selects an
  implementation. Intake accepts every documented alias case-insensitively;
  past it nothing sees a string. Two refusals are deliberate and are contract,
  not omission: `iso-8859-1` is ISO 8859-1 and never `windows-1252`, and bare
  `utf-16` is refused because RFC 2781 and the WHATWG Encoding Standard
  disagree about it - `utf-16le`, `utf-16be`, and `Charset::from_bom` are the
  three unambiguous answers.
- Precedence where a caller did not say: an explicit argument, then the
  declared `MediaType`, then one bounded content read of a byte-order mark.
  `MediaType` carries `Option<Charset>`; absent is the media type saying
  nothing, and `Charset::from_media_type` is the one place that absence becomes
  `Utf8`. A mark is framing rather than content, so `from_bom` answers its
  length and the caller decides - only the structured-text plan takes it off.
- Every charset agrees with US-ASCII below `0x80` except the UTF-16 pair, and
  that is load-bearing: `decode` and `encode` borrow an all-ASCII payload
  rather than transcoding it, a line scan splits on `\n` before anything is
  transcribed, and after a declared charset is decoded, and the borrow is
  asserted in the counting allocator rather than argued. A charset that broke
  it would need its own scan and its own line splitter.
- Three doors, one verb: `decode` refuses and names the charset, the byte
  position, and the byte or scalar found there, through `Error::Codec` - no new
  error variant, and the charset's canonical name in `format`. `decode_lossy`
  marks each fault with `U+FFFD` and is what a capture of arbitrary wire bytes
  needs. `transcribe` reads every byte it can - an unassigned byte as its
  ISO 8859-1 scalar, and bytes offered as UTF-8 or as US-ASCII that are not
  what they were offered as by one rule written once in the layer, every
  valid UTF-8 run kept and every other byte read as WHATWG windows-1252 with
  the five holes as C1 controls, per invalid run - and is what a
  `string(...)` column's values arrive through. There is no lossy *encode*: a
  scalar a charset cannot spell is unrepresentable input, which fails.
  `encoded_len` answers the stored length without building the bytes, and it
  counts rather than judges - a scalar with no byte still occupies the one it
  would occupy - so a length bound costs a walk and `encode` stays the single
  authority on whether bytes can be written at all.
- Tables are generated from Python's codec registry by
  `scripts/generate_charset_tables.py` and never edited by hand; a wrong scalar
  is a silently corrupted column. `scripts/check_charset_interop.py` checks
  every table against that registry in both directions, because a table wrong
  both ways round trips perfectly.
- A decode resumes only at a sequence boundary, which `Decoder::is_pending`
  answers, and every charset here is stateless past one - so a byte offset is
  the whole of what `Transcoded` needs to seek. A shift-state encoding would
  have to carry that state into the resume index before it could be added.
- There is no charset option on the text record reader: the text medium in
  `text/` reads the charset a handle's media type declares, once, where it builds the
  transport, and a whole resource in one charset is `Transcoded`,
  not a second option on every reader.

## Strings

`string.rs` owns the family, and `utf8.rs`, `ascii.rs` and `cp1252.rs` each
hold one charset's six leaves beside that charset's codec; these bind a change
to any of the eighteen leaves or to what a string declares.

- **Eighteen leaves, each a variant of its own.** A string is one of
  eighteen leaves: six shapes - plain, large, view, large view,
  `Fixed*(u32)`, `Sized*(u32)` - in each of the three charsets that have a
  datatype, UTF-8, US-ASCII and windows-1252 (`Utf8String` through
  `SizedCp1252String`). Each leaf is a `DataType` variant, a `Field` variant
  and a `Scalar` variant of its own, one to one with its `DataTypeId`, the
  number a numbered leaf states carried inline. `StringType` is the view over
  the eighteen - what a `StringField` holds, what `string_parameters` answers
  on a datatype and on a value, and the owner of every rule a leaf has - and
  `DataType::from(leaf)` is the variant it names. `DataType::string` is the
  one validating constructor and `utf8()`,
  `large_utf8()`, `utf8_view()`, `large_utf8_view()`, `fixed_utf8(n)`,
  `sized_utf8(n)` and the same six for `ascii` and `cp1252` that constructor
  picking a leaf once. There is no layout beside a charset beside a bound, no
  `StringLayout` and no parameter struct: the leaf already says all three, so
  a reader never asks whether the number it carries is a width or a maximum.
  A leaf built by hand can state a width of zero, exactly as every other
  parameterized variant can hold what its constructor refuses, and `validate`
  is what catches it before a boundary. `Charset` stays the text-decoding
  vocabulary; a charset with no leaf is refused wherever a leaf is asked for
  (`with_charset`). `string_parameters` reads back for every string, which is
  what makes "which charset is this column in" one question. The twelve
  registered codes are not strings: a currency is an identity over ISO 4217
  codes and digital-asset tickers, at most eight bytes, that stores as the
  text it is, so it is `DataType::Ccy`, kind `Code`,
  answers `is_code` and `code_width`, and never `string_parameters`.
  `code_width` is a maximum rather than a layout, so `fixed_byte_width`
  answers `None` for a code.
- **The leaf is the name.** `DataTypeId::as_str` and `Display` are the leaf's
  canonical name - `utf8`, `large_utf8`, `utf8_view`, `large_utf8_view`,
  `fixed_utf8(n)`, `sized_utf8(n)`, and the same six under `ascii` and
  `cp1252` - and `DataTypeId` has one identifier per leaf, the eighteen in
  the text family's range (`0x51`-`0x62`), because `as_u8` is a wire contract
  laid out by family. The charset-free
  spellings `string`, `fixed_string`, `string_view`, `large_string`,
  `large_string_view`, `sized_string` and SQL's `varchar`, `text`, `char(n)`
  name the UTF-8 leaf of their shape, and they alone take a `(charset)`
  argument - `string(windows-1252,32)` is `sized_cp1252(32)` - because a
  charset-named spelling already answered that question: `utf8(windows-1252)`
  is refused. The grammar's fold drops case, underscores and hyphens, so
  `largeutf8` and `LARGE-UTF8` are that spelling too. `StringType::from_spelling`
  (every alias) and `general_spelling` (the charset-free ones) are the one
  table the grammar and both bindings read, so a spelling is never accepted in
  a datatype expression and refused under a `layout=` argument.
- **One number, one leaf.** A stated number is `with_bound`: the width on a
  `Fixed*` leaf, the maximum on a `Sized*` one, and a plain leaf takes a
  maximum and answers the sized leaf of its charset, because plain storage is
  exactly what a bounded column fills - `utf8(32)` is `sized_utf8(32)`,
  `ascii(4)` is `sized_ascii(4)`, `fixed_ascii(4)` the width. The large and
  the viewed leaves refuse a number rather than silently becoming something
  narrower: `large_utf8(64)` is an error. `with_declared_bound` is the other
  half - a leaf that *is* its number does not stand without one - and
  `from_declaration(layout, charset, bound)` is the sequence a binding's three
  arguments run, so a binding decides nothing. The grammar tracks what was
  *read*, never the placeholder `1` a numbered leaf carries in `ALL`.
- **`Str` is the characters; the variant is the leaf.** Every string
  `Scalar` variant holds a `Str` - the crate's compact string, inline to
  `INLINE_CAPACITY` bytes, static for free, one `Arc<str>` beyond - and the
  numbered ones their number beside it: `Scalar::FixedAsciiString(Str, u32)`.
  A value carries the leaf of the column it was read from, a maximum as well
  as a width, so a cell read out of `sized_utf8(32)` is
  `SizedUtf8String(text, 32)` and its `id`, `kind` and `dtype` say so.
  Equality, order and hash read the characters alone, so a value is one value
  whichever leaf holds it, and `Borrow<str>` is sound for that reason and no
  other. `StringType::scalar` is the value door - the NUL trim of a fixed
  slot, the US-ASCII repertoire, the width and the bound - and a canonical row
  holds the column's exact leaf: a value of another leaf, or of the same leaf
  under another number, is restated under the column's, and the validation
  that precedes it judges the text against the column's leaf, never the one a
  value names. `Str` is the holder every string-family API answers with -
  `ascii_value`, `StringEnum` members, `str_from_value` - and `SmolStr` stays
  the crate's utility string for names, keys and errors. `string_scalars!`
  and `string_dtypes!` are the or-patterns over the eighteen variants that
  every match in the crate spells them through.
- **The bound counts stored bytes**: the exact width on a `Fixed*` leaf, the
  maximum on a `Sized*` one. Bytes, because that is what the buffer holds and
  what Arrow's offsets measure; a scalar count would make a bound a walk of
  the value. `Charset::encoded_len` counts it without building the bytes.
  Whether those bytes can be written is the write seam's question, not the
  value door's: a value read back through `transcribe` carries scalars the
  charset does not assign - that is what recovering damage means - so refusing
  it here would make the permissive read useless, and the refusal names the
  scalar where it is written instead.
- **A value holds UTF-8 and remembers its charset.** Decoding happens at the
  seam, as everywhere else; what a value keeps is the leaf it is *written*
  under - its variant - so it goes back out the way it came without the column
  being read twice. `as_str` is therefore infallible on every string value
  there is.
- **Arrow gets the truth about the bytes.** UTF-8 and US-ASCII ride Arrow's
  string layouts - ASCII bytes are UTF-8 - and windows-1252 rides the matching
  *binary* layout, because the bytes are not UTF-8 and an Arrow reader told
  otherwise reads mojibake and calls it text; the fixed leaves ride
  `FixedSizeBinary` in every charset. `is_text_storage` is the one owner of
  that split and every writer, reader, digest and cast asks it. Plain `utf8`,
  `large_utf8` and `utf8_view` are Arrow's own datatypes and cross bare; every
  other leaf states something Arrow cannot - a charset, a number, the second
  view width - and rides the `yggdryl.string` extension document with the leaf
  written whole beside its charset, `{"layout":"sized_cp1252","charset":"windows-1252","max":32}`,
  because that document is what a foreign reader inspects and the charset is
  the one fact Arrow cannot state. A document over a storage it does not
  describe is a foreign field wearing our name and imports as its storage. The
  Arrow document, the schema document (`{"type":"string","layout":"sized_cp1252","max":32}`,
  the charset never written because the leaf says it) and a value's document
  all still read the pairing they replaced - a shape name beside a `charset`
  key, a `max` beside any unbounded shape - as the leaf it names.
- **A code's identity is its extension name, not its storage.** That split
  governs the string leaves; a registered code is outside it. A code is
  US-ASCII text held to one width, so it rides Arrow's `Utf8` whatever else
  is true, and what separates it from the text beside it is the *name*:
  `yggdryl.ccy` over `Utf8` with an empty document is a currency, the
  same storage under `yggdryl.string` is the string that document describes,
  and under no name at all it is plain text. `yggdryl.ccy` over any
  other storage is a foreign field wearing our name and imports as that
  storage, by the same rule a string document does; the retired
  `yggdryl.currency` name is refused before that foreign-extension
  fallback. The width no column enforces is enforced where values enter,
  which is what makes it a value rule rather than a layout - a shape rule:
  how real a code is (its check digit, its listing) is its rank, read on
  merge, never a refusal.
- **`StringType::scalar_from_bytes` is the one door bytes take, and the
  charset decides how strict it is.** UTF-8 and US-ASCII are validated repertoires - Arrow
  guarantees the first and the second rides Arrow's text storage - so bytes
  that are not what they claim are refused, and a US-ASCII value holds no NUL
  and no byte above `0x7F`. windows-1252 is a declaration that the column
  holds legacy bytes, and those are transcribed rather than refused:
  `Charset::transcribe` reads an unassigned byte as its ISO 8859-1 scalar.
  The row door, the Arrow cell reader and the cast all call that one function
  (or `read_text`, its text beneath it), so a batch and a row cannot read
  bytes differently; a text-storage cell is adopted as the column's leaf,
  checked when it was written and not again, through one reader per leaf
  chosen once per run. The value stream is input from outside, so every
  string and byte value it decodes crosses the leaf's door.
- **The value door judges US-ASCII and counts everything else.** A value
  restated under windows-1252 may hold scalars that charset cannot write -
  that is what recovering damage means - and the write seam (the Arrow array
  build, the cast) refuses them naming the scalar. A `StringEnum` packs its
  members into integers through `ascii_packed`, so it is accepted on a
  `fixed_ascii` leaf of at most sixteen bytes or a code and refused by name
  elsewhere. `ascii_packed` pads a value with trailing NUL to that width and
  reads it big-endian: the padding belongs to the packing, never to a column,
  so a code's integers are the same whatever its storage holds. A fixed
  string pads into `fixed_byte_width`, a code into `code_width`.

## Bytes

`bytes.rs` owns the family the same way `string.rs` owns strings;
these bind a change to any of the six leaves or to what a byte column
declares.

- **Six leaves, each a variant of its own.** A byte column is one of six
  leaves - `Binary`, `LargeBinary`, `BinaryView`, `LargeBinaryView`,
  `FixedBinary(u32)`, `SizedBinary(u32)` - and each is a `DataType`, `Field`
  and `Scalar` variant of its own. `BytesType` is the view over the six, what
  a `BytesField` holds and `bytes_parameters` answers, and
  `DataType::from(leaf)` the variant it names. `DataType::bytes` is the one
  validating constructor and `binary()`, `large_binary()`,
  `binary_view()`, `large_binary_view()`, `fixed_binary(n)`, `sized_binary(n)`
  are that constructor picking a leaf once. There is no `BytesLayout` and no
  parameter struct beside a bound. `bytes_parameters` reads back for every
  byte column. A UUID and a geospatial value are bytes with an identity, so
  they are their own datatypes and answer no `bytes_parameters`, exactly as a
  code answers no `string_parameters`. A UUID stays `FixedSizeBinary(16)`
  where a code moved to text, and the asymmetry is the point: `arrow.uuid` is
  the canonical Arrow extension and its storage is not ours to redefine, the
  value is 128 opaque bits with no repertoire to be text in, and sixteen
  bytes beat the thirty-six a spelling would take. A code's `yggdryl.*` name
  is ours, and a code's value *is* ASCII text. Both read into the other
  family through the one cast tier rather than through a renderer of their
  own.
- **One number, one leaf.** `fixed_binary(16)` is the exact width and
  `sized_binary(16)` a maximum of sixteen bytes; neither stands without its
  number, and the other four refuse one - `binary(16)` is `sized_binary(16)`
  written short because plain binary is exactly the storage a bounded column
  fills, while `large_binary(16)` says so rather than silently narrowing.
  Bytes are never padded, so a fixed value is exactly its width. `bytes`,
  `blob`, `bytea`, `varbinary(n)`, `fixed_size_binary(n)` and Iceberg's
  `fixed[n]` are accepted spellings and render as the canonical ones; `BytesType::from_spelling`,
  `with_bound` and `with_declared_bound` are the one table and the one rule
  the grammar and both bindings read.
- **`Bytes` is the payload; the variant is the leaf.** Every byte `Scalar`
  variant holds a `Bytes` - inline to `INLINE_BYTES` bytes with no heap behind
  it, static for free, one `Arc<[u8]>` beyond - and the numbered ones their
  number beside it. A value carries the leaf of the column it was read from, a
  maximum included. Equality, order and hash read the payload alone, and
  `Borrow<[u8]>` is sound for that reason and no other. `BytesType::scalar`
  is the value door; a cell of a column's own storage is adopted as the
  column's leaf; `bytes_from_value` is what a value spells as bytes.
  `as_bytes` reads the payload of a byte or a geospatial value, `as_binary`
  of a byte value alone.
- **Arrow already says the layout.** The storage is the leaf, and only what
  no Arrow type can state - a maximum, and the second view width - rides the
  `yggdryl.bytes` document; a fixed width is the storage itself. A document
  over a storage it does not describe imports as its storage. Avro and
  Iceberg have no maximum either, so a bounded column crosses them unbounded
  and the bound is enforced where the values enter.

## Structured codecs

The JSON, YAML, TOML, and XML pages under `docs/media/` document the
surface; these bind a change to `json/`, `toml/`, `yaml/`, `xml/` and the codec
machinery they share in `text/`.

- Parse bytes, slices, readers and emit bytes, writers over `Scalar`; string
  conveniences reuse the same parser with no intermediate serialization.
- Emit ordinary native shapes only - no tags, envelopes, version markers, or
  private wire representation - and reject kinds a format cannot represent. An
  optional `Field` types natural strings, orders records, and canonicalizes;
  without it, return only types the document proves.
- YAML ignores tags as annotations; TOML follows its native root/table, integer,
  date/time, and single-document limits; unsupported values fail.
- Limits bound bytes, depth, nodes, documents, aliases, and hard recursion;
  errors name format and byte position; streaming fails at the failing item under
  backpressure.
- Inference is deterministic: explicit format, then path suffix; byte-like is
  content, a string is a path only when it names an existing file; content parse
  order is JSON, XML when well-formed and opening with `<`, TOML when
  complete and non-empty, then YAML. Never infer JSONL
  from content.
- Placeholder substitution walks parsed `Scalar` under a closed grammar and needs
  separate opt-ins for substitution and environment access. Benchmark slice,
  stream, writer, field-directed, wide, and deep paths.

## Arrow and allocation

- One sealed zero-sized marker per datatype variant. `TypedField<K>` owns one
  `Field`; borrowed forms and `ProtocolField`/`ProtocolFieldMut` own one pointer
  (plus a `Scheme`); `FieldScalar<'_>` owns one pointer and one `Scalar`,
  `FieldRecord<'_>` one pointer and the cells' `Vec`. No duplicated state, no
  unchecked mutable path that can invalidate the marker or the pairing.
- Arrow schema parity is lossless; the C Data Interface routes only through core
  recursive `DataType`/`Field` exporters, and bindings never build schemas
  recursively. IPC dictionary IDs are transport-local: preserve native IDs in one
  reserved root sidecar, remove it on import, reject unsupported nested
  dictionary layouts.
- Cache complete Arrow projections: no-op mutations retain caches, effective ones
  invalidate once, cache state never affects equality, hash, serde, display.
  Root fields validate once and cache; rows use `validate_value` and
  `canonicalize_value`; named records reject missing, extra, duplicate,
  non-string keys before committing.
- Core scalar creation, getters, lookup, iteration setup, shared nesting
  clones, inferring a typed leaf, and reading a typed row's cells and names do
  not allocate; corpus sizes vary in the check, and timing alone never proves
  it. Preflight slot and fixed-buffer budgets before allocating.
- `Serie` owns every value crossing: `serie/value.rs` is the one codec
  between a row and an Arrow slot, named by nothing outside `serie/`,
  `cast.rs` and `temporal.rs`, and a column leaf reads its own typed buffer
  through the reading its field resolved where it landed - a cell read is
  one buffer read and one constructor, `Serie::scalar(i)` allocates nothing
  for a leaf value and a record row costs its run alone. `Serie::from_scalars`
  and `Serie::from_arrow_array(...).scalar(0)` are the scalar-array
  boundary, where the exact `Field` controls nullability, dictionaries,
  extension identity; `yggdryl::arrow` keeps the stream combinators, the
  IPC dictionary sidecar and the bounded row-to-batch reader, at most one
  source batch held, never JSON.
- `cast.rs` owns recursive casting and is reached through `Serie`: Struct
  casts reconcile names, reject ambiguous folds, follow target order, fill a
  missing nullable field with nulls, and preserve exact buffers; wrapper
  exposure propagates, so hidden child failures and nulls stay hidden. A null
  under a required field is absence and is refused by path - except where null
  is the datatype's own canonical default, the one exception, stated once in
  the engine and once in the landing.
- Hidden-span compaction and noncompact list-view gathering draw on the landing's
  one materialization budget. A root run-end gather searches each selected span
  once, and a nested run-end uses indexed take unless its tree also contains a
  zero-width fixed-size serie. That combined tree uses generic range extension
  to preserve the zero-width row count and may rescan the nested runs once per
  selected span; retain its explicit cost caveat until one recursive gather
  replaces it.
- `ArrowCastOptions` carries the two independent answers a cast needs and every
  entry point takes it: `safe` = may a present value convert, `Representation` =
  what a same-width pair carries. Whether a value may be absent is no option: it
  is the target field's nullability, one rule at every door. A nullable column
  takes a failed conversion as null under `safe`; a required column refuses a
  value it cannot convert by that value whatever `safe` says, and a null, an
  empty text cell entering a non-text column and a column the source does not
  carry by path, never writing its canonical default. The one repair is
  internal: a holder the digest fill writes after the cast that lands its
  batch may arrive absent for that fill. `Representation::Bits`
  shares the value buffer between two fixed-width layouts of one byte width;
  it is a preference, so an unlike pair or a rule-governed target converts as
  it always did.
- `ArrowCastPlan` is the schema-dependent half, compiled once from a source
  `Field` to a target `Field` (a foreign source is planned from its Arrow
  field, never imported): immutable, `Send + Sync`, `compile`/`preflight`/
  `apply`, with its certification compiled beside it. There is one plan per
  stream, bind or write session, and a `PlanCache` where the schema can
  change. Only masks, offsets, dictionary reachability and the proof of
  uncertified leaves vary per batch. `Serie::cast` compiles per call and is
  for one-off columns.

# 2. Validation - smoke locally, prove in CI

Local work is the [smoke loop](#smoke-loop). Proof is `.github/workflows/ci.yml`
on the pushed branch, plus `docs.yml` for the site. Do not rehearse the matrix
locally: two feature lanes, the MSRV toolchain, seven exchange jobs, a JVM,
both pyarrow legs, and every documentation example in three languages cost tens
of minutes on one machine and run in parallel there for nothing.

## Before you push

Five commands, all warm from the loop, that catch most of what CI would reject:

```bash
cargo fmt --all                                            # the formatter, not the check
cargo clippy -p yggdryl --all-targets --no-deps -- -D warnings
cargo test -p yggdryl --all-targets                        # default features; benches at smoke corpus
cargo test -p yggdryl --doc                                # rustdoc examples, the pages' examples
git status --short                                         # generated files and inventories committed
```

Then the pre-push block of each layer the change actually touched - §3 for
Python, §4 for Node, §5 for docs - and nothing at all for a layer it did not.

A file that is generated and was not regenerated is the cheapest CI failure to
prevent and the most common one. Each is regenerated by its own tool once its
input has settled, in this order, because some read an earlier one: the dump
and the hash read the dictionary, and the two manifests read the addon and the
dictionary:

| Generated | Regenerated by | Once |
| --- | --- | --- |
| `rust/src/charset/tables.rs` | `python scripts/generate_charset_tables.py` | a charset row changes |
| `config/fix/`, `provenance.json` and `rust/src/fix/constants.rs` | `python scripts/generate_fix_dictionary.py`, which fetches the FIX standard | a pinned source commit, `StringEnum::COUNTRIES` in `rust/src/string.rs` or the generator changes, the `FIX:` keys it writes included; never a crate field alone, which the generator neither writes nor checks |
| the crate's own documents under `config/fix/` - the crate's field shard, the fixed row and its `metadata` group | `YGGDRYL_FIX_DUMP_WRITE=1 cargo test --locked -p yggdryl --test fix the_committed_store_carries_the_crate_dump` | a crate field, the fixed row or its group changes (`rust/src/fix/crated.rs`), or the dictionary is regenerated |
| the dictionary hash in `rust/tests/fix/store.rs` | the `left` value `cargo test -p yggdryl --test fix the_committed_dictionary_hashes_to_one_pinned_value` reports, pinned in that test with the reason as the newest `It last moved when` sentence of its rustdoc, every earlier one kept; the census counts beside it move in the same edit | the dump is written |
| `node/index.js`, `node/index.d.ts` | `npm run --prefix node build:debug` | any Node binding or its doc comments change |
| `docs/assets/fix.json`, `docs/assets/playground.json` | `node scripts/build_docs_fix.js`, `node scripts/build_docs_playground.js` | the dictionary, the crate dump or the addon changes, the addon rebuilt first: `build_docs_fix.js` runs the addon over `config/fix` |
| `.api-inventory.txt`, `.api-bindings.txt` | by hand, in the same change; the inventories row of the [smoke loop](#smoke-loop) proves it | a public name is added or retired |

## What CI proves

Read this instead of running it. CI passes `--locked` to every cargo and
maturin command, so a `Cargo.lock` that would have to move is a failure there
and not a silent update.

| Job | Proves | The one command that reproduces it |
| --- | --- | --- |
| Rust quality (default features, all features) | `cargo fmt`; clippy at `-D warnings` on `-p yggdryl` and on `--workspace --all-features`; `cargo test --all-targets` in both lanes and the CLI's in the default one; rustdoc examples; `cargo doc` under `RUSTDOCFLAGS=-D warnings`; the optimized benchmark configuration | the failing step verbatim, with the lane's flags: nothing, or `--all-features` |
| Iceberg Rust 1.94 | the declared MSRV, the workspace's one: the core with every target and the official Iceberg boundary | `cargo +1.94.0 check --locked --manifest-path rust/Cargo.toml -p yggdryl --all-targets --features iceberg` |
| S3 / Azure / Google exchange | the object stores against MinIO with boto3, Azurite with azure-storage-blob, fake-gcs-server with google-cloud-storage - signatures and dialects against implementations that answer 403 | `python scripts/check_object_interop.py`, `check_azure_interop.py`, `check_gcs_interop.py`; each fetches its own server |
| ZIP / Avro exchange | `zipfile` and fastavro writing the archive and the container this crate then reads: the direction whose in-tree tests skip when nothing produced the input | `python scripts/check_zip_interop.py`, `python scripts/check_avro_interop.py` |
| PyIceberg exchange | v1, v2, and v3 tables against PyIceberg | `python scripts/check_iceberg_interop.py` |
| Spark interop | Iceberg against the format's reference implementation, behind its own marker | §3, and only for that boundary |
| Python binding wheel | `stage_cli.py --debug`, the maturin wheel at `--profile dev` (CI never measures; the release workflow builds what ships), and the assertion that it carries `yggdryl-<version>.data/scripts/yggdryl` | the wheel path in §3, with those two debug flags |
| Python binding (`pyarrow==18.*`, `pyarrow>=18`) | `pytest python/tests` and `mypy --strict` on both legs, with pandas, polars, tzdata, and xxhash installed so no suite skips silently | §3, with the leg's pyarrow pinned into `python/.venv` |
| Node.js binding | `test:package:debug`, the generated loader and declarations unchanged, the `yggdryl` command built so the book tests drive it rather than skip, `node --test` plus `tsc --noEmit`, and the two docs manifests | §4 |
| Documentation examples | every fenced block under `docs/` and `skills/` compiled and run in Rust, Python, and JavaScript | `python scripts/check_docs_examples.py --lang <the failing language>` |
| `docs.yml` build | `mkdocs build --strict` - nav, links, and strict warnings | `python -m mkdocs build --strict --config-file mkdocs.yml` |

## What CI never runs

These have no job, so a push cannot find them. They belong to the change that
makes them stale, and a skipped one is reported as skipped:

```bash
python scripts/generate_charset_tables.py --check   # rust/src/charset/tables.rs drift
python scripts/check_charset_interop.py             # every code page against Python's codecs
```

```bash
cargo bench -p yggdryl --bench <types|arrow|uri|text|coding|charset|media|holder|hashing|expression|fix|fix_allocations|logging>
npm run --prefix node bench:<coding|fix|graph|hashing:txhash|hashing:xxhash|holder|http|logging|media|text|types>
python python/benchmarks/<name>.py                  # boundary benchmarks, release wheel
YGGDRYL_S3TABLES_ARN=<table bucket ARN> python python/benchmarks/media/s3tables.py  # a real table bucket and pyiceberg; SKIPPED otherwise
YGGDRYL_SCALE_BYTES=21474836480 cargo test --release -p yggdryl --test scale_ulbridge --features iceberg -- --ignored --nocapture  # a 20 GB capture under a bounded RssAnon; SKIPPED otherwise
```

CI compiles the benchmark targets and executes them at smoke corpus; it never
measures. A Performance table on a page is regenerated by the release run above,
on the machine that table names, or it is not changed at all.
`scripts/generate_fix_dictionary.py --check` needs the upstream dictionaries, so
it runs with a regeneration ([Before you push](#before-you-push)) and never in
CI; `python -m unittest discover -s scripts/tests`, the generator's own suite,
runs in the change that edits the generator and has no job either.

## A red run

Read the failing job's log before touching anything: the matrix names the lane,
the feature set, the toolchain, and the interpreter, and a failure in one leg
only is usually a feature gate or a version floor rather than the behavior.
Reproduce it with the narrowest local command that can show it - that leg's
feature flags, the one interop script, the one pyarrow version - fix the cause,
smoke it, push again. Never re-run a job to see whether it passes this time,
never skip, relax, or quarantine a check to make it green, and never report a
run that has not been read.

# 3. Python

The core settles first (§1). Rules shared by both extensions:

- Reach every stable core domain; a missing binding is documented as Rust-only.
- The type side owns width, unit, scale and zone. `DataType::scalar` and
  `Field::scalar` already carry all four and reach strictly more than a family
  factory can - every decimal width including `decimal32` and `decimal64`, every
  float width, and a bare count read at the unit the column declares. So a
  caller who needs a width names it on the type, and `Scalar` keeps only the
  statics expressing something no other door does. This replaces the former
  rule, which mandated the `Scalar.float`/`decimal`/`date`/`time`/`datetime`/
  `duration` family factories and kept exact widths private: `DataType("float16")`
  and `DataType("time32(s)")` were always public, so the width was never private
  and the factories were a second door onto one verb.
- Infer or cast once at the boundary, then redirect to the most specific native
  method; duplicate no parser, schema, suffix, codec, scalar, or record logic. A
  value entering a datatype or field crosses `DataType::scalar` or
  `Field::scalar`, never the host runtime's casting - PyArrow and Arrow JS know
  none of the value rules this crate owns.
- Conversion pairs are Python `Scalar.from_`/`as_py` and JS
  `Scalar.from`/`asJs`, the inferring constructor and the native reading;
  native and Arrow values map through `Scalar` losslessly where the runtime
  allows.
- Coerce only documented wrappers, strings, path-like values, mappings, native
  scalars, enums, and Arrow values; never stringify arbitrary objects.
- Preserve argument order, defaults, error semantics, and native error messages
  across all three languages; map only exception type.
- Bind scope protocols to `open`/`close`, keep no binding-side cache, and list
  every public method in `.api-bindings.txt`.

Python-only:

- Python protocols and native types: immutable wrappers implement stable
  equality/hash/order/pickle/copy/repr, and a mutable wrapper with equality
  follows Python's hash contract. `IOBase`/`Url` are `pathlib`-shaped but
  core-backed, inventing no modes or cursor state.
- Annotation and dataclass inference builds native fields directly, never PyArrow
  schemas merely to import them again; the behavior is Python-only, while schema
  and scalar semantics stay native.
- Public decorator `@scalar` (beside the Python `Scalar` boundary), pure field
  builder `field(value, name=None)`, typed field factories at the package root,
  one module per type. `@scalar` forwards every stdlib dataclass option,
  installs one cached argument-free `staticmethod into_field()`, rejects a
  pre-existing `into_field` member, leaves every other member name - `field`
  included - to the caller, and reserves no static metadata constant.
- `Class.into_field()` returns one frozen non-null Struct `Field`, preserving
  dataclass order and metadata, excluding `ClassVar`/`InitVar`/private working
  annotations, resolving forward and generic annotations once, detecting
  recursion, synchronized on first access. Optionality defines default
  nullability, explicit annotation options win, defaults/factories affect
  construction rather than schema, and generated dataclasses derive annotations
  from the exact native field graph.
- No second row decorator or class, static field constant, library-installed
  `schema` or `field` alias beside `into_field`, or retired public surface.
- `pyarrow.RecordBatchReader` is the primitive record shape - table, batch, and
  dataclass row methods redirect through it over the C Stream interface, on the C
  Data Interface and PyArrow holders.
- Columnar host objects cross as `Serie` (held), `ChunkedSerie` (held apart:
  a `pyarrow.ChunkedArray`'s chunks, a table's batches) or `SerieReader`
  (streamed): `Serie.from_(value, field=None)`, `ChunkedSerie.from_(value,
  field=None)` and `SerieReader.from_(value, root=None)` share one recognition
  ladder for pyarrow, pandas, polars, NumPy and any Arrow
  C exporter. After it, a concrete scalar or container takes `Serie.from_`'s
  scalar boundary and becomes one held reader item. A Python sequence recognized
  as mapping records or columnar batches remains an incremental record stream and
  reuses its first batch import; an iterator or generic reusable iterable remains
  incremental too, and schema resolution may pull its first item. A held non-record
  column is wrapped as a record before the reader's root is applied.
  `SerieReader.from_serie` and `SerieReader.from_chunked` make a held column or
  a held chunked one a stream, `IOBase.read_serie` answers a `SerieReader`, and
  `write_serie`, `overwrite_serie`, `append_serie` and `merge_serie` take a
  `Serie`, a `ChunkedSerie`, a `SerieReader` or anything `SerieReader.from_`
  reads, off the GIL where the rows are native.
  A cast is `Serie.cast`, `ChunkedSerie.cast` or an `ArrowCastPlan`, passing
  the caller's `safe` and `representation`. No binding casts,
  rebuilds rows from, or walks an Arrow array itself.
- Structured codec facades stay byte-oriented and native, `cls=` is explicit
  reconstruction, and encoders never close caller-owned streams.

## Python checks

The loop is an in-place debug extension and one scoped suite; before pushing,
the same over the whole tree plus the type checker:

```bash
V=python/.venv/bin/python
VIRTUAL_ENV=python/.venv $V -m maturin develop -m python/Cargo.toml  # in place, debug, no wheel
$V -m pytest python/tests/<file> -x -q         # the loop
$V -m pytest python/tests                      # before pushing
$V -m mypy --strict --config-file python/pyproject.toml \
  python/yggdryl python/tests/typing_bindings.py python/tests/typing_fields.py
```

- `python/.venv` is where the extension is installed and where
  `scripts/check_docs_examples.py --lang python` looks for its interpreter, so
  every Python command is that interpreter and never the ambient one.
- Install `pandas`, `polars`, `tzdata`, and `xxhash` into it or those suites
  skip silently, locally and anywhere else. A silent skip is a failed check,
  never a pass.
- The wheel path - `python scripts/stage_cli.py`, `maturin build --locked
  --manifest-path python/Cargo.toml --interpreter python --out python/dist`,
  `python -m pip install --force-reinstall --no-deps python/dist/*.whl` - is
  what CI does and what a boundary benchmark needs. It is not the loop, and the
  wheel it builds must carry `yggdryl-<version>.data/scripts/yggdryl`.
- Both pyarrow legs (`pyarrow==18.*`, `pyarrow>=18`) are CI's; run one locally,
  and only pin a second interpreter when a failure names the version.
- Iceberg-with-Spark has its own CI job and is opt-in locally - `python
  scripts/setup_spark_interop.py`, then `python -m pytest
  python/tests/test_spark_interop.py -m spark_interop` - so run it only when
  changing that boundary.

# 4. Node

Python settles first; the shared binding rules in §3 hold here too.
JavaScript-only:

- camelCase at the boundary only, over native state: JS equality/comparison/hash
  helpers, cloning, child iteration, `Map`-like metadata.
- Record helpers close over one native Struct `Field` and nested structs reuse
  cached layouts; no per-row schema, map, or JSON bridge.
- `BatchReader` is the one-shot primitive: `BatchReader.from` accepts readers,
  Arrow JS tables/batches, batch arrays, or IPC bytes; `intoIpc`/`intoTable` drain
  it; one batch crosses as one self-contained IPC stream.
- `Serie`, `ChunkedSerie` and `SerieReader` are the Arrow doors:
  `Serie.fromArrowArray(vector, field?, options?)`,
  `Serie.fromArrowBatch(batchOrTable, root?, options?)`,
  `Serie.fromArrowReader(reader, root?, options?)`,
  `ChunkedSerie.fromArrowArray(vector, field?, options?)` (one chunk per
  `Data`), `ChunkedSerie.fromArrowBatch(batchOrTable, root?, options?)` (one
  chunk per batch), `ChunkedSerie.fromArrowReader(reader, root?, options?)`,
  `SerieReader.fromArrowReader(reader, root?, options?)`,
  `SerieReader.fromSerie(serie)`, `SerieReader.fromChunked(chunked)`,
  `serie.cast(field, options?)`, `chunked.cast(field, options?)` and
  `ArrowCastPlan`; `{ safe, representation }` reach the core.
  `IOBase.readSerie(options?)` answers a `SerieReader`, and `writeSerie`,
  `overwriteSerie`, `appendSerie` and `mergeSerie` take a `Serie`, a
  `ChunkedSerie`, a `SerieReader` (consumed) or anything `BatchReader.from`
  accepts. A
  `Scalar` has no Arrow door of its own: a value crosses as
  `Serie.fromScalars(field, rows)` and comes back as `scalar(0)` or
  `intoScalar()`. A chunked input is one reader door and one cast, never a
  cast followed by `extendFromSerie` per chunk, and a `ChunkedSerie` where its
  chunks stay apart.
- Arrow JS interop is copied IPC with bounded cursors and a validated cached
  schema - never claim zero-copy; public IDs are transport-local while native
  records keep canonical IDs.
- JSON/YAML/TOML/XML facades are byte-first over native `Scalar`, preserving
  `bigint`, bytes, `Date`, arrays, plain objects, maps, sets, class targets.
- Before N-API recursive conversion, build one bounded detached plain-data
  snapshot and reject cycles, proxies, accessors, symbols, depth, node overflow.
  Keep the recursive depth ceiling at 48 until traversal is iterative.
- Reserved identities `javascript:builtins.<Name>`, `javascript:<application>`,
  `yggdryl:<native>`; detect native identity, never `constructor.name`.

## Node checks

The loop is the debug addon and one test file:

```bash
npm ci --prefix node                                     # once
npm run --prefix node build:debug                        # after a Rust or binding edit
node --test node/tests/<file>.test.js
```

Before pushing a Node change, the audit and the files the build generates:

```bash
npm run --prefix node test:package:debug                 # build + loader/type audit + package files
git diff --exit-code -- node/index.js node/index.d.ts    # generated loader and declarations current
cargo build --locked -p yggdryl-cli                      # the command node/tests/book.test.js spawns
npm test --prefix node                                   # node --test plus tsc --noEmit
node scripts/build_docs_playground.js --check            # generated docs manifests not stale
node scripts/build_docs_fix.js --check
```

`npm run --prefix node bench:<...>` measures the release addon and has no CI
job - §2.

# 5. Documentation

Write for lookup - the readers are human scanners and LLM retrieval. Contract,
then the smallest runnable example, then non-obvious edges, then measured
performance. Canonical symbol names, stable headings, short paragraphs, tables
only for exact mappings, exact commands and results preserved. One fact in one
place: link instead of paraphrasing, and never narrate signatures in prose (a
signature block is code, see below), repeat examples in prose, add marketing
text, or create benchmark-only pages.

The layer tabs, the page skeleton, and the per-change docs rules are spelled out
in `docs/architecture.md` and `docs/contributing.md`; those pages and this
section change together. What binds every page:

- Root `mkdocs.yml` is authoritative - strict build, nav, and links change
  together, README stays a short landing page. A family page lives under
  `docs/<theme>/` for the theme owning the vocabulary, with
  `docs/<theme>/index.md` as its overview. There is no per-language page: a
  binding fact is documented on the page owning the vocabulary it belongs to,
  in that page's Python or JavaScript tab, so one operation is described once
  and every language spelling of it sits beside the others.
  `docs/media/` is a theme of one page per medium under one `Media` nav
  section: `index.md` is the overview - the table of every medium, then the
  surface they all share: Read, Write with its three intents, and
  `RecordOptions` - and each media type has its page -
  `ipc.md`, `parquet.md`, `avro.md`, `text.md`, `csv.md`, `json.md`,
  `yaml.md`, `toml.md`, `xml.md`, `xmla.md`, `excel.md`, `iceberg.md`,
  `http.md` - beside `compression.md` (gzip, zlib, zstd) and `charsets.md`.
  A media page reads Overview - a contract table: what declares it, its
  build, its doors in each language, its settings and refusals - then Read
  and Write, each a short paragraph and a tabbed example that carries the
  detail, then the sections only that medium has, then its own Performance
  section, never a shared one.
  `docs/holder/index.md` is one page for storage: Handles
  (`Holder`, roles, delegation), then the `IOBase` surfaces - Bytes, Values,
  Records, Partitions, Call counts - then one section per backend: Buffer,
  Local, Filesystems, Object stores, Buffered, ZIP. A section may open with a
  `text` block of the few public signatures a caller implements or reaches for
  first, each with a one-line comment on what it promises; it is never the
  whole surface, which rustdoc owns.
  `docs/types/` is a theme like Media: the Core pages - `datatype.md`,
  `field.md`, `scalar.md`, `cast.md`, `paths.md`, `protocol.md` - then one
  subsection per family (`numeric/`, `temporal/`, `text/`, `codes/`, `nested/`,
  `geospatial/`), each an `index.md` for what the family shares and one page per
  type in it. A type page reads in the order its core file is written -
  Contract, DataType, Field, Scalar, Arrow storage - then its features, its
  edges and its commands, with every example in the three languages.
- Every supported example uses tabs in Rust, Python, JavaScript order, the same
  operation expressed idiomatically; show Rust-only explicitly, never invent a
  binding. Every block is self-contained with an assertion and runs through
  `scripts/check_docs_examples.py`; ignored blocks use valid superfence syntax
  and are reported; shell commands use `bash` fences and name real targets.
- Interactive pages read the committed manifest under `docs/assets/` that the
  JavaScript extension generates from the published package: every package fact
  comes from it, the addon job checks it for drift, page scripts add no
  framework, CDN, or build step and reimplement nothing, an ungeneratable page
  stays an ordinary example block, and a page reading reader input against the
  manifest says which answers are the package's and which are the reading.
- A benchmark table lives in the Performance section of the page owning the
  measured method, names machine/runtime/build, compares a trusted baseline, and
  ends with its regenerate command; `docs/benchmarks.md` only indexes them.
- `skills/` holds the agent skills for code that *uses* the package, one folder
  per layer: a `SKILL.md` (the door table - task to Rust, Python and JavaScript
  spelling - then the rules and pitfalls) and `references/rust.md`,
  `python.md`, `javascript.md` of runnable recipes, linking the published pages
  for depth rather than restating them. `skills/yggdryl/` is the entry skill.
  A change to a public name, default, cost or refusal a skill teaches updates
  that skill in the same change; its blocks follow the example rules above and
  run under `scripts/check_docs_examples.py`, and `.claude-plugin/` publishes
  the folder as the `yggdryl` Claude Code plugin.

## Documentation checks

`mkdocs build --strict` is seconds and catches the nav and link breakage a page
move causes, so it runs on every push that touches `docs/`, `mkdocs.yml`, or
`README.md`:

```bash
python -m mkdocs build --strict --config-file mkdocs.yml
```

The example runner has no per-page filter: it is a whole-language pass, one
process per block on a pool one process wide per core (a block spends most of
its life importing the extension; `--jobs N` narrows it when the machine has to
stay responsive). Run the language whose examples were edited, once, before
pushing; CI runs all three.

```bash
python scripts/check_docs_examples.py --lang rust         # compiled against parquet iceberg s3 http3
python scripts/check_docs_examples.py --lang python       # runs under python/.venv
python scripts/check_docs_examples.py --lang javascript   # needs the built addon beside Arrow JS
```

## Handoff

- Sweep for dead code, duplicated logic, retired symbols, stale docs and
  skills, Rust-only bindings a stable core no longer justifies.
- Run the §2 local-only checks the change made stale - charset table drift,
  charset interop, and the benchmark behind any number a page now states.
- Push, then read the run. A branch whose CI has not been read is not handed
  off, and a red one is §2's "A red run", not a caveat.
- Remove only generated targets, site output, virtual environments, binaries,
  caches, and `node_modules` that validation created; preserve unrelated work.
- Report CI status per job that failed, the local-only checks run, exact skipped
  checks, material caveats - nothing else.

# 6. Releases

- `main` triggers `.github/workflows/release.yml`; `v<version>` is the receipt
  created after registry publication, and manual runs rehearse only.
- Publishes are idempotent, the tag is last, and a released version is never
  reused.
- A version is out on all three registries or on none. `preflight` reads
  crates.io, PyPI and npm before anything builds and refuses a branch
  push that would publish a version some of them already carry, because the
  tree under a branch is not the tree those artifacts were built from and one
  number would come to name two libraries. Such a version is finished from the
  commit it was built at - `git tag v<version> <commit> && git push origin
  v<version>`, a tag being what pins that tree - or it is left partial and
  every manifest bumps. Re-running on `main` repairs only a version no
  registry has yet.
- A release that was going to publish and did not files an issue naming the
  version, the run and what each registry holds, and comments on that issue
  rather than opening a second. A failed `preflight` files one too: three
  manifests disagreeing is the loudest failure there is and the one that
  publishes no version to name, so the report reads the tree instead.
- Root Cargo, Python, and Node versions match exactly. Publish crates.io, PyPI,
  npm only after platform smoke tests import and exercise the artifacts.
- One job per platform builds everything the platform ships: the `yggdryl`
  command, the wheels and, where npm carries the platform, the Node.js module,
  over one compile of the core for both bindings. That holds while the core
  resolves to one crate graph under both: a feature one binding's dependency
  switches on for a crate the core also uses is declared in the workspace
  manifest (`ffi` on the Arrow crates, `std` on `log`), and every build in the
  job sees the same `--target` and environment.
- A bump moves seven files together, and `preflight` reads three of them:
  `Cargo.toml` and `Cargo.lock`, `python/pyproject.toml`, `node/package.json`
  and `node/package-lock.json`, and the two generated documentation manifests
  `docs/assets/fix.json` and `docs/assets/playground.json`, which stamp
  `node/package.json`'s version.
- Credentials stay in repository configuration: Cargo and npm secrets, PyPI
  trusted publishing. No stored PyPI password, no fourth registry.
