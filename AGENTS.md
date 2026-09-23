# Yggdryl agent contract

Arrow-native core in Rust (`rust/`), two native views - Python (`python/`) and
Node (`node/`) - and the `ygg` CLI (`cli/`). Rust owns `DataType`, `Field`,
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
- **CI proves the change.** The exhaustive matrix - two feature lanes, two MSRV
  toolchains, seven exchange jobs against outside implementations, both pyarrow
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
7. **Docs** (§5): every layer touched, examples in all three languages.
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
| codec | `<name>.rs` at the root (`load`, `dump`, `reader`, `writer`, `IOBase` wrapper) + a `Codec` variant -> bench -> bindings -> the Compression section of `docs/media/index.md` |
| string leaf | a `StringType` variant + `DataTypeId` appended + `string.rs` (spellings, the number rule, Arrow storage, grammar, value) + the charset's own root file - `utf8.rs`, `ascii.rs` or `cp1252.rs` - for the leaf's constructor, validation and reading -> tests -> bindings -> `docs/types/` |
| byte leaf | a `BytesType` variant + `DataTypeId` appended + `bytes.rs` (spellings, the number rule, Arrow storage, grammar, value) -> tests -> bindings -> `docs/types/` |
| charset | a row in `scripts/generate_charset_tables.py` + a regenerated `charset/tables.rs` + a `Charset` variant; a charset that gets string leaves is a root file of its own beside `utf8.rs`, `ascii.rs` and `cp1252.rs`, holding its codec and those leaves -> interop both directions -> bench -> bindings -> the Charsets section of `docs/media/index.md` |
| storage backend | `<name>/` at the root with a location/container/leaf trio over the root traits - `<Name>Path`, `<Name>Folder`, `<Name>File` over a host tree; `<Name>Path`, `<Name>Node`, `<Name>Leaf` where the store has no tree to promise (`zip/`); state and assert its call/request counts -> interop script -> docs |
| media format | `<name>/` at the root, free functions over `IOBase` + a stateful wrapper, reached through `MediaType`/`RecordOptions` -> interop both directions -> docs |
| metadata property | a protocol view keyed `<SCHEME>:<property>`, the scheme upper case; never a new `Field` accessor |
| binding method | core method first; the binding only infers, coerces, redirects - plus a parity test, a boundary benchmark, a docs entry |

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
| the Python view redirects | `python/.venv/bin/python -m maturin develop -m python/Cargo.toml`, then the same interpreter's `-m pytest python/tests/<file> -x -q` | the binding against the core it redirects to, with no wheel built |
| the Node view redirects | `npm run --prefix node build:debug`, then `node --test node/tests/<file>.test.js` | the same, with no package audit |
| the inventories are not stale | `python scripts/check_api_inventory.py` | every section header names a file or folder that exists; a Rust name still occurs somewhere in that crate's `src/`, and so does every type the signature beside it names; a binding entry's dotted key still resolves through the tree its section names - each segment a module beside its parent or a name that parent binds. What is omitted is counted - source files with no section, `pub` names the inventory never spells - never failed |
| a page example runs | `python scripts/check_docs_examples.py --lang rust`, or `python`, or `javascript` | every block in that language - there is no per-page filter, so this is a pre-push check, not a loop |
| the installed wheel works | `python scripts/check_wheel_smoke.py` | what `pip install yggdryl` gives a reader: the extension loads and an Iceberg table round-trips. It reads `yggdryl` from the environment, never `python/yggdryl`, so install a wheel (or `maturin develop`) first - the release runs it against every wheel it publishes |

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
it skipped a check.

**Best effort, then a named refusal.** An expression, a cast, a read or a write
does what the text asks whenever one reading does it, and refuses only what no
reading can, naming the column, the value, or the section it could not honour.
A constant coerces into the operand it meets, operands with no common type
compare as text, a declared column casts safely unless it is `not null`, an
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
columnar object lands as `Scalar::Arrow` with its buffers) and the core reads
them with `from_scalar` - a binding never re-implements a parser, and the
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
`iceberg` (implies `parquet`), `s3`. Examples live in docs - no `examples/`
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
| `datatype.rs` | `DataType`, the shared logical datatype enum and its cross-family value contract; `datatype_id.rs` and `datatype_kind.rs` the exact-variant and family enums, `parser.rs` the canonical display and the Arrow, SQL, Hive and Spark parsing, `serde.rs` the structural document, `compatibility.rs` the concrete targets, `vocabulary.rs` the logical names, `default.rs` the canonical defaults, `diff.rs` schema equality and its differences, `merge.rs` the one place two schemas become one |
| `field.rs` | `Field`, one variant per `DataType` shape, each carrying name, nullability, metadata and the Arrow projection cache; `metadata.rs` + `metadata/` the `<SCHEME>:<property>` map and its validation, `protocol.rs` the borrowed protocol views |
| `scalar.rs` | `Scalar`, the one value every part of the project speaks; `arithmetic.rs` checked arithmetic over exact natives, `path.rs` the one allocation-free value path every recursive walk uses, `pretty.rs` the indented rendering of a schema |
| `value/` | what a datatype, a field and a value each owe the root that holds them, and what a family owes its leaves: the `Value` and `FamilyValue` contracts, `DataTypeValue`, `FieldValue`, `FieldSidecar` and the payload datatypes `GeometryType`, `GeographyType`, `UnionType`, `RunEndType`, the family traits `IntegerValue`, `FloatingValue`, `DecimalValue`, `TemporalValue`, `GeospatialValue`, `CodeValue` and `NestedValue` - declared here, implemented beside each leaf - and the `Nested` value enum; the other six family value enums, `Integer`, `Floating`, `Decimal`, `Temporal`, `Code` and `Geospatial`, live in their family files; `canonical.rs` the schema-directed validation and canonicalization of row values. The module is private, and every name is `yggdryl::<Name>` at the crate root |
| `typed.rs` | the typed markers and the field-borrowing values: `TypedField<K>`, `FieldScalar<'_>`, `UncheckedFieldScalar<'_>`, `FieldRecord<'_>` and the prebuilt shared fields |
| `cast.rs` | casting an Arrow array into the exact array a typed field describes: `ArrowCastPlan` and the strict rules; `budget.rs` the bounded scratch and output reservations it draws on |
| `integer.rs`, `floating.rs`, `decimal.rs`, `boolean.rs`, `bytes.rs`, `uuid.rs`, `geospatial.rs`, `enums.rs`, `structure.rs`, `sequence.rs`, `mapping.rs`, `union.rs`, `runend.rs`, `version.rs` | one family per file, each the whole of its datatype, field and scalar; `int256.rs` holds the `i256`/`u256` pair the exact decimals compute in, the one type file not named for its type because a module and a struct share one namespace at the root; `wkb.rs` the Well-Known Binary reader three types need; `regex.rs` the Struct inference from named captures |
| `code.rs` | the contract every registered code answers - the trait and the two builders; the twelve codes are one file each: `currency.rs`, `country.rs`, `mic_code.rs`, `cfi_code.rs`, `isin_code.rs`, `cusip_code.rs`, `sedol_code.rs`, `bloomberg_code.rs`, `figi_code.rs`, `side.rs`, `state.rs`, `timeinforce.rs` |
| `temporal.rs` | what the five temporal families share and nothing any one of them owns: the `Temporal` value enum over the eight leaves, whose `family()` answers `date`, `time`, `datetime`, `duration` or `interval`, the `temporal_leaf!` macro the family files build their count-unit-zone values with, the unit validators the constructors call, the ISO 8601 spellings every text codec and the scalar renderer write through, the Arrow casts that take any temporal, and the `Scalar` readers that answer across the families (`as_temporal`, `temporal_unit`, `temporal_timezone`, `temporal_count`); `TemporalValue`, the contract every leaf answers, is in `value/`; no datatype, no field and no leaf value live here |
| `date.rs` | the date family: `DateType` - `Date32`, `Date64`, no parameter, the unit being what the leaf is - `DataType::Date(DateType)` with `date32()`, `date64()` and `date_type()`, the `Date32` and `Date64` values with their `Scalar` constructors, one Arrow projection (`Date32`, `Date64`) |
| `time.rs` | the time family: `TimeType` - `Time32(unit)`, `Time64(unit)`, the resolution a parameter of the leaf and `for_unit` the one rule `DataType::time` picks a width by - `DataType::Time(TimeType)` with `time`, `time32`, `time64`, `time_of` and `time_type`, SQL's `time(p)` grammar, the `Time32` and `Time64` values, one Arrow projection (`Time32`, `Time64`) |
| `datetime.rs` | the datetime family: `DateTimeType` - one leaf, `DateTime64 { unit, timezone }` - `DataType::DateTime(DateTimeType)` with `datetime64` and `datetime_type`, every `timestamp` spelling of the grammar, the `DateTime64` value, one Arrow projection (`Timestamp`, carrying the zone only when the datatype states one) |
| `duration.rs` | the duration family: `DurationType` - `Duration32(unit)`, `Duration64(unit)` - `DataType::Duration(DurationType)` with `duration32`, `duration64`, `duration_of` and `duration_type`, the `Duration32` and `Duration64` values, one Arrow projection (`Duration`, which imports back as `duration64` because Arrow has one width) |
| `interval.rs` | the interval family: `IntervalType` - one leaf, `Interval(layout)`, the layout a `TimeUnit` interval member - `DataType::Interval(IntervalType)` with the validating `interval(unit)` and `interval_type`, the `interval` grammar with SQL's bare `interval day`, the `Interval` value holding every component of every layout, one Arrow projection (`Interval`) |
| `timezone.rs` | the `Timezone` value, its bundled IANA registry, and the `timezone` datatype a column of zones declares |
| `mime_type.rs` + `mime_type/`, `media_type.rs` + `media_type/` | the root `MimeType` and `MediaType` values, which stay the media routing vocabulary, each with a `datatype.rs` beneath it for the `mimetype` and `mediatype` datatypes a column declares; `mime_type/` also holds the extension registry and the line classifier |
| `string.rs` | every string the crate has, one family: the `StringType` enum of eighteen leaves - six shapes in each of UTF-8, US-ASCII and windows-1252 - the one string datatype `DataType::String(StringType)`, the one string value `Str`, the `FIELD:enum` dictionary `StringEnum` and its ISO listings, one Arrow projection, one cast tier, one grammar, one set of field markers. The twelve registered codes are not strings and are not here: each is its own file - `currency.rs`, `country.rs`, the seven `*_code.rs` leaves, `side.rs`, `state.rs` and `timeinforce.rs` - over the contract in `code.rs`. `utf8`, `large_utf8`, `sized_ascii(4)`, `fixed_cp1252(8)` and the spelling `string(windows-1252,32)` are all leaves of `DataType::String` and all answer `DataType::string_parameters`; a code answers `DataType::code_width` and `is_code` instead, because it is an identity over a registry rather than a charset, and rides `Utf8` under its own extension name. The per-charset arms - `charset()`, the fixed and sized leaf constructors, `with_charset`, the decode and encode behind `Str::from_bytes` and `encode`, a value's repertoire check - dispatch to `utf8.rs`, `ascii.rs` and `cp1252.rs`; the eighteen-variant enum itself stays here, because a variant is not a type of its own |
| `utf8.rs`, `ascii.rs`, `cp1252.rs` | one root file per charset that has string leaves, each holding that charset's codec and its six leaves together. `utf8.rs`: the UTF-8 decode, transcribe, pending and fault rules under the `utf-8` name, and `Utf8String` through `SizedUtf8String` with `utf8()`, `large_utf8()`, `utf8_view()`, `large_utf8_view()`, `fixed_utf8(w)`, `sized_utf8(n)`. `ascii.rs`: the `ascii_len` scan, `decode`/`encode` and their `_into` forms, `text`, the `us-ascii` name, the `ascii_text`/`ascii_bytes`/`ascii_repertoire` helpers, the `ascii_packed`/`ascii_value`/`packed_width` pair the codes and `StringEnum` ride on, and the six ASCII leaves. `cp1252.rs`: a thin codec over `charset::single_byte` with `tables::CP1252` under the `windows-1252` name, and the six windows-1252 leaves. Each owns its leaves' `DataType` constructors, its `LEAVES` list, and the decode and encode that `Str::from_bytes` and `Str::encode` in `string.rs` dispatch to; only `ascii.rs` judges a repertoire (`ascii_repertoire`) and holds the `i128` packing; `Charset` and `StringType` dispatch to them and duplicate nothing |
| `charset.rs` + `charset/` | the `Charset` vocabulary beside what every code page shares: `single_byte` and the generated `tables.rs` own the code pages, `utf16` owns UTF-16, `bom` the byte-order mark, `Decoder`/`Reader`/`Writer`/`sink` the chunked doors, `Transcoded` the decoding handle. The three charsets with string leaves are root files; every other code page reaches `single_byte` through `Charset` and is not a public module of its own |
| `holder/` | what every backend shares: `Holder`, the one concrete handle unifying every backend, `Buffer`, `Buffered<H>`, `Counted<H>`. The root traits follow no backend: `IOPath`/`IOFolder`/`IOFile` and their `path_*`/`folder_*`/`file_*` methods are the same on every one |
| `local/`, `fs/`, `zip/`, `s3/` | one root folder per storage backend, each a location/container/leaf trio over the root traits: `LocalPath`, `LocalFolder`, `LocalFile`, `FsPath`, `FsFolder`, `FsFile` and `S3Path`, `S3Folder`, `S3File` in `local/`, `fs/` and `s3/`; `ZipPath`, `ZipNode`, `ZipLeaf` in `zip/`, which indexes names and has no directories or files to name after. `local/` is memory-mapped local storage, and remote backends change neither it nor the root traits; `fs::FileSystem` is Arrow's seven-method shape for interop, while the core contract and variants keep generic `FileSystem`/`Fs*` names; `s3/` holds Amazon S3, Google Cloud Storage and Azure Blob Storage inside it, since all three answer that dialect, under the non-default `s3` feature |
| `coding/` | what every codec shares: the transparent `Coded<H>` handle and the `Codec` dispatch helpers |
| `gzip.rs`, `zlib.rs`, `zstd.rs` | one root file per codec; each owns `load`, `dump`, `reader`, `writer`, an `IOBase` wrapper |
| `media/` | what every medium shares: the `Media` value naming every implementation, record options, inference, magic, merge, partition, structured routing |
| `ipc/`, `parquet/`, `avro/` | one root folder per record medium; each owns free functions over `IOBase` plus a stateful wrapper |
| `iceberg/` | separate modules: types, schema, partition, snapshots, metadata, manifests, statistics, scalar rendering, scan, table, options, catalog, evolution, inspection |
| `text/` | the plain-text medium - `Text<H>`, flat `TextOptions`, bounded physical-line splitting, row-header capture, body rendering, `TextBytes`/`TextLine`/`TextEntries` - `TextLine` an `Event` of the graph holding the whole line, row header included, and the `Arc<TextOptions>` it reads itself by, every reading resolved once on its first ask and a `set_` stated over it - beside what the structured codecs share: `Format`, `Limits`, `Formatting`, `Loading`, placeholders, `TextCodec`, io, wire, typed |
| `json/`, `toml/`, `yaml/` | one root folder per structured codec over `Scalar`, each its own parser over the machinery in `text/` |
| `uri/` | the URI, URL, URN and ARN values and, in `datatype.rs`, the `uri` family - `UriType` with its `url` and `urn` leaves - and the fields and scalars over them |
| `arrow/` | Arrow interop; recursive cast planning stays with `Field` |
| `expression/` | one term grammar and one plan grammar: `Term`/`Bound`, `Filter`, `Selector`/`BoundSelector`, `Plan` (create, write verbs, `select`, `from`, `where`, `order by`, `limit`, `offset`), `Expression` (clause, plan, or `;` sequence), `Records`, `Attribute`, `Bounds`, `explain`, `FieldPath`/`FieldSegment`, `user` (registered `namespace.name` functions, `FunctionSignature` as a struct field, `Function::User`), `transform` (`TRANSFORM:function`/`TRANSFORM:sources`, else `TRANSFORM:expression`); every application (`apply_datatype` first and `apply_field` derived from it, `apply_scalar`, `apply_arrow_reader` first and `apply_arrow_batch` derived from it, `apply_records`, `from_scalar` readers) lives here and nowhere else |
| `graph/` | the graph vocabulary: `element.rs` holds `Element` - an element's `Uuid`, its cross identity and code, its codes, its names, its canonically sorted parent UUIDs (the whole lineage) and source UUIDs (the elements it was read from: provenance, never carried along a chain), read and written - `Event`, an element with an instant (`currunix`, `i64` nanoseconds since the epoch, UTC), precise optional execution and recording instants plus the persisted recording clock of its merge reference, a state and a place in its chain, and `MarketElement`/`MarketEvent`; `event.rs` the two holders, `iterator.rs` the one walk, `column.rs` the nineteen event columns (`EventColumn`) every generated event schema states under one name and one datatype each - a text line's batch opens with them in `EventColumn::ALL` order, while a FIX row contains the same fields through the crate's protocol-oriented bands and lifecycle rows retain them; `instrument.rs` the lifecycle-local association registry: a conservative 32 MiB reserve charges 1 KiB per valid ISIN, admits at most 32,768 under that reserve and 65,536 in all, keeps learning known entries at the cap, and has no global mapper; signatures and provided readings, no storage |
| `hashing/` | the private structural/display stable-hash adapters the digests share; shared dispatch vocabulary is `digest.rs` |
| `xxhash/` | one-shot digests, four resumable states, `reader`/`writer`, `Hashed<H>`, the canonical `Scalar` byte feed, Arrow row digests |
| `variant.rs` | the Apache Parquet Variant binary encoding, version 1: `Variant` - one metadata dictionary and one value payload - `Scalar::Variant`, the encode and decode doors, the canonical `arrow.parquet.variant` projection, and what every medium writes for a `variant` column |
| `valuestream.rs` | the crate's own byte stream: any `Scalar` as version, `DataTypeId`, payload, children - and back, leaf for leaf; the stream, the whole, the `DataType` doors; what pickle carries |
| `txhash/` | raw Unix-count/digest pairs, clock-unit conversion, configured hashing and Arrow coupling; not RFC UUIDs |
| `parallel.rs` | the one ordered map over persistent stream workers the FIX doors read on: line, message-row and write doors use 64-item chunks at lane depth two; Arrow capture parsing holds at most one whole input batch per worker; answers stay in input order and one thread is the lazy sequential map |
| `fix/` | FIX protocol behavior |
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
own target.

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
  `Scalar::Sequence`; `Scalar::Struct` is a sorted name-to-scalar *input* shape.
  No second row/schema class or accessor; `FieldRecord<'_>` is a borrowed view
  of one row under that field, never a class of its own.
- Codec parsing and local per-event enrichment depend only on that event. They
  may use `parallel.rs`'s persistent ordered workers when parallelism helps,
  preserving input order with bounded concurrency. Cross-event state and
  prior-event order belong only to lifecycle, which performs deeper logical
  enrichment after parsing.
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
  `DigestAlgorithm`, `EdgeAlgorithm`, `IOKind`, `IOMode`, `Level`, `Magic`,
  `MediaType`, `MimeType`, `Scheme`, `TimeUnit`, `UnionMode`. `Timezone` is a
  datatype of its own, so `timezone.rs` holds it as a type file and the crate
  root re-exports it like `Scalar`. No local copies, no `enums` module.
  `Digest`/`Digester` sit beside `DigestAlgorithm`, `Encoder` beside `Codec`;
  `Scalar` -> `scalar.rs`, the `Holder` variants -> `holder`, record settings ->
  `media`, `FieldPath`/`FieldSegment` -> `expression`, whose grammar already
  writes their steps.
- `IOMode` = `ReadOnly`, `Overwrite`, `Append`, `Merge`, `Random`; operations
  reject modes that do not apply; no alias.
- `DataTypeId` = exact variant, `DataTypeKind` = family. `TimeUnit` is the only
  temporal/interval unit parser and Arrow converter; `MimeType`/`MediaType` own
  MIME parsing, suffix and content-coding inference, preferred extensions;
  `Scheme` owns URI and compatibility scheme vocabulary.

## Patterns

### One type, one file

A type lives in `rust/src/<type>.rs`, and that one file holds the whole
of it in this order: the **datatype** - the `DataType` constructors and
predicates naming it - then the **field** - its `FieldType` marker and
`TypedField` alias - then the **scalar** - its value, its `Value` impl and the
`Scalar` variant that carries it. Its Arrow reading belongs there too: the
storage it projects to, the extension name it is recognised by, and the cast
that reads it back. Nothing about a currency is anywhere but `currency.rs`.
A type too big for one file keeps the file and adds a folder of its own name
beside it, never a folder under another type: `mime_type.rs` holds the
`MimeType` value and `mime_type/` its datatype, its registry and its line
classifier; `media_type.rs` and `media_type/datatype.rs` the same. There is no
`types/` folder: a type is a root file, and the root is where a caller finds
it as `yggdryl::<Type>`.

`<type>` is the vocabulary, not the width. Every integer width is one
`IntegerValue` over one set of rules, so `integer.rs` holds all ten; every
registered code is its own standard with its own validity, so `country.rs`,
`currency.rs`, `isin.rs` and the eight beside them are eight more files.

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
| Arrow value | `arrow::ArrowScalar` | one scalar, array, batch, or stream under one `Field` |

Equivalences a change keeps lossless, in both directions:

- `DataType`/`Field` <-> Arrow, through `from_arrow`/`into_arrow` and the core
  recursive exporters - never a schema rebuilt in a binding.
- `Scalar` <-> Arrow array or scalar, through `arrow::scalar_array` and
  `arrow::scalar_value` under the exact `Field`, which decides nullability,
  dictionaries, extension identity.
- rows <-> ordered `Scalar::Sequence`; named input <-> sorted `Scalar::Struct`
  (`from_record`), canonicalized against the Struct `Field`;
  `ArrowScalar::from_rows` and `into_scalar` cross the same way.
- a datatype's canonical default is `default_value`/`is_default_value` - the
  value a declaring protocol's `apply_arrow_batch` leaves alone.
- widths: a family constructor picks the physical width once, and shared logic
  reads across widths with `as_i128`/`as_u128`, `as_f64`, `as_decimal`, and
  `as_temporal`/`temporal_unit`/`temporal_timezone`/`temporal_count`.

### Stack: holder -> media -> arrow

| Level | Surface | Answers |
| --- | --- | --- |
| bytes | `IOBase`: `pread`/`pwrite`, `read_all_bytes`, `read_range_bytes`, `append_bytes`, `pstream_bytes`, `read_digest` | positional bytes, digests, bounded streams |
| position | `IOCursor`, `Cursor<H>` | the only place a cursor is retained |
| records | `IOMedia`: `read_arrow_field`, `read_arrow_reader`, `read_arrow`, `write_arrow_*`, `*_records`, `row_size`, `column_size`, `record_options` | schema, rows, batches, statistics |
| values | `yggdryl::arrow`: `scalar_array`, `scalar_value`, `ArrowScalar`, `cast_reader`, `combined` | the `Scalar`/Arrow boundary |

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
- Rows out: `read_arrow_reader(options)` streams; `read_arrow(field)`
  answers an `ArrowScalar` carrying its own shape.
- Rows in, by shape, each with `overwrite`/`append`/`merge` plus a generic
  `write_*` taking an `IOMode`: `*_arrow_reader` (the streamed primitive),
  `*_arrow_batch` (one batch), `*_records` (a row iterator), `write_arrow`.
- Navigate a row `Scalar` with `get`, `get_key_str`, `path`, `iter`,
  `sequence_iter`, `record_iter`, and update with `with_field`/`without_field`;
  a row is an ordered sequence, never a map.
- `ArrowScalar` reports `shape`, `is_scalar`/`is_array`/`is_batch`/`is_stream`,
  `row_size`, `column_size`; borrows with `as_array`/`as_batch`; consumes with
  `into_array`/`into_batch`/`into_reader`/`into_scalar`; converts with `cast`.
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
| exploit | `ArrowCastPlan::compile`/`preflight`/`apply`, `scalar_array`/`scalar_value`, cached Arrow projections, `as_i128`/`as_u128`/`as_f64`/`as_decimal`/`temporal_*`, `default_value` | schema-dependent work leaves the per-item path |

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
  `.name` for a struct child, `[0]` and `[-1]` for a list element, `['key']` for
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

- borrowed views allocate nothing: `as_*`, `as_array`, `as_batch`, `as_field`,
  `TypedFieldRef`, `ProtocolField`; `into_*` is the allocating counterpart.
- typing allocates nothing where the proof already exists: `FieldScalar::infer`
  borrows the prebuilt shared field of a leaf datatype, `FieldScalar::new`
  answers a canonical value untouched, and `FieldRecord::get*`/`names`/`iter`
  borrow; `FieldRecord::new` (the cells' `Vec`), `into_scalar`, and `into_str`
  allocate by contract.
- an exact cast returns the caller's own batch, and `Representation::Bits` shares
  the value buffer between two same-width layouts.
- `local/` is memory-mapped, `Buffered<H>` pins pages instead of copying
  them forward, and shared nesting clones a reference while empty collections
  hold no backing.
- Python crosses the C Data Interface and PyArrow holders.

Does not hold, and is never claimed: JavaScript interop is copied IPC with
bounded cursors; `read_all_bytes`, any `Vec` return, `into_*`, and text or JSON
rendering allocate by contract.

## Public vocabulary

Names describe ownership and return type; alternate-verb aliases are forbidden.
Check a name in `.api-inventory.txt` before writing it; the change that adds or
retires one regenerates it ([Before you push](#before-you-push)).

| Verb | Contract |
| --- | --- |
| `new` | infallible construction from native parts |
| `from_*` | construct or parse a named representation; validate when needed |
| `into_*` | another representation, borrowing or consuming as useful |
| `as_*` | borrowed, allocation-free view |
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
  field-directed `from_json_scalar_with_field`, the YAML/TOML counterparts),
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
  `Decimal64`, `Decimal128`, `Decimal256`; `Date32`, `Date64`;
  `Time32`, `Time64`; `Duration32`, `Duration64`; one `DateTime64`; `Interval`;
  `Geometry`, `Geography`; `Sequence`, `Mapping`, `Struct`. Temporals keep the `TimeUnit`/`TimeZone` their datatype needs;
  `DateTime64` always has a non-null `TimeZone`, naive spelled `TimeZone::Naive`.
  The wire vocabulary does not follow the spelling: `Scalar::kind()` and the
  serde tags keep the short `i8`, `d128` names they always wrote.
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
  `Scalar` variant with no family enum between, each geospatial reading and
  each registered code likewise - a code is `Scalar::Currency`, the way its
  type is `DataType::Currency`; shared logic goes through the
  cross-width readers `as_i128`/`as_u128`, `as_f64`, `as_decimal`, and
  `as_temporal`/`temporal_unit`/`temporal_timezone`/`temporal_count`, or
  narrows to a family value enum - `as_integer`, `as_floating`,
  `Decimal::from_scalar`, `as_temporal`, `as_code`, `as_geospatial`,
  `as_nested` - and a family constructor picks the physical width once.
- A typed view compares, orders, and hashes over `(dtype, value)` - never the
  field's name, nullability, or metadata - so a value is one value whichever
  column holds it, and its `stable_hash` is the value's own. A borrowing view
  implements `Serialize` and never `Deserialize`; `UncheckedFieldScalar` has no
  equality, hash, or serde because its state is unproven.

## Datatypes, parsers, errors

- `DataType::from_str` and `Field::from_str` are the recursive schema grammars;
  bindings pass expressions straight through. Accept canonical plus common
  Arrow/SQL/Hive/Spark forms under an explicit recursion limit.
- `DataType::LOGICAL_NAMES` is the one fallback registry: FIX Latest datatype
  vocabulary plus `mic`, each name resolving to the closest core datatype and
  displaying as it - no variant, no second spelling. Never register a word the
  Arrow/SQL grammar owns. `StringEnum::PREBUILT` keys the ISO code constants
  the registered names prebuild; `StringEnum::from_logical_name` builds the
  enum a field declares from one. A listing is a constant: every reader answers
  the same members.
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

### Object stores (`s3/`, non-default `s3` feature)

One backend, one location/container/leaf trio, three stores: Amazon S3, Google
Cloud Storage, Azure Blob Storage. Each REST API is spoken directly - SigV4,
OAuth 2.0 bearer tokens, Azure Shared Key over a synchronous HTTP/1.1 client -
with no SDK, runtime, or object-store layer.

`Provider` is the sole dispatcher: one value says which store answers, and every
place the three differ reads it and nothing else. A dialect owns only what its
store spells for itself - `aws/` the credential chain, the STS exchange, the
shared files and the S3 XML; `google/` the Application Default Credentials
chain, the RS256 assertion, and the JSON API; `azure/` the Shared Key signature,
the SAS and bearer paths, and the Blob XML. `sigv4.rs` and `xml.rs` are shared
because Signature Version 4 and the `<Error>` document are not one store's
alone; `answer.rs` holds what an answer *says* in shapes no store owns, so the
transport, the retry, the staging model, the listing pipeline and the three
roles are written once.

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
  `write_arrow_reader(reader, options, mode)`. Table, record-batch, row-record
  entry points infer or wrap input into that pipeline; nothing streamable takes or
  returns `Vec` batches.
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
- `options.commit_row_size`: unset = one commit; `N` publishes every `N` rows plus the
  remainder, holding at most one bounded commit; the first overwrite commit
  overwrites, later ones append; failure leaves published prefixes visible.
- Overwrite replaces rows under the stored field; append retains stored rows;
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
- Plain-text rows are the nineteen event columns `EventColumn::ALL` names -
  the line as the event it is, `currunix` first and `state` last - then
  required `body: utf8`, then one column per row-header capture. The event
  states every fact a column used to repeat and no column repeats one: the
  object a line came from is `crosscode`, so `crosshashcode` is the XXH3-64 of
  that URL string and `crossuuid` derives from it; the row number under
  `TextOptions.start_rownum` is `seqnum`, null at zero and refused where a
  count cannot hold it; when the record was written is `currunix`, the row
  header's `mtime` capture where `parse_mtime` declares one and `IOBase::mtime`
  where it does not. `body` is the line past what the header matched - the
  header comes off where the line is made, so the captures are the line's and
  the body is the payload, empty exactly where the header consumed the line -
  and a line is never empty as cut, refused at every door that sets one, a
  blank physical line being a separator rather than a record; its bytes are
  decoded at the transport in the charset the handle's media type declares
  other than UTF-8 or US-ASCII, and otherwise once where the line is made,
  each byte that is not UTF-8 read as the Windows-1252 character it is,
  `TextLine::decoded_byte_size` counting them. `currhashcode` is the shared
  event digest - the captures a header lifted, the parents, the state, the
  place and the predecessor - and then the body, with the capture that dates
  the line left out, because `currunix` is coupled with the code rather than
  fed into it. The row header is the only thing that lifts a column out of a
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
- A derived column names its own input: `PARTITION:transform` is an expression
  grammar function over the field paths in `PARTITION:sources`, both stored on
  the derived column in the one shape every `sources` property has. One source
  per transform today; a longer list is stored and refused on apply.
  `apply_arrow_batch` is the one verb every declaring protocol answers - it walks
  the Structs that protocol declares and leaves a column holding anything but its
  canonical default alone. `Field` runs them in dependency order: cast,
  partition, digest.

### Digests

- `stable_hash` = XXH3-64 over the canonical feed everywhere: no second hash
  family, no second spelling. The pinned xxHash dependency's types never appear in
  a public signature, doc example, error, or binding.
- Integer holders take signed or unsigned storage at the algorithm's exact width;
  signed is a bit-preserving view, normalized to unsigned before feed on nested
  reuse.
- A row digest reads direct Struct children in declaration order.
  `DIGEST:sources` is the exact input, resolved against its own Struct; `["*"]`
  and an absent list both select every field except a holder; `"*"` may not
  travel beside a named path. A holder never feeds itself back; a selected Struct
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

The Iceberg section of `docs/media/index.md` documents the format surface; these bind a
change to `iceberg/`.

- A table is a folder reached only through `IOBase`: metadata = core JSON,
  manifests = core Avro, data = core Parquet, and no Iceberg/Avro/catalog
  dependency whose I/O or Arrow model conflicts.
- Plan snapshot -> manifest list -> manifest -> files from metadata, never by
  walking `data/`; prune on partition summaries and safe statistics, resolve
  residuals by row filtering, report read/skipped counts, and keep parallel scans
  in plan order - they differ from sequential only in speed.
- `SchemaUpdate` owns evolution: preserve field IDs and never reuse dropped ones;
  promotions are Int32->Int64, Float32->Float64, and same-scale decimal widening;
  validate loaded metadata and every commit.
- `Table` answers the same `IOMedia` surface as a leaf - a table format is a
  media wrapper, not a second record API.
- Emit bounds only where Parquet and Iceberg encodings agree: missing bounds cost
  performance, wrong bounds violate correctness.
- Options resolve explicit -> table property -> default in `IcebergOptions`, one
  resolver per key. The retry gate rebases append and metadata-only commits only;
  overwrite/merge/compact restore state on conflict, and a failed commit may
  leave unreferenced files.

## Charsets

The Charsets section of `docs/media/index.md` documents the surface; these bind a change to `charset.rs`,
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

- **One datatype, one value, eighteen leaves.** `StringType` is an enum whose
  leaves are the columns: six shapes - plain, large, view, large view,
  `Fixed*(u32)`, `Sized*(u32)` - in each of the three charsets that have a
  datatype, UTF-8, US-ASCII and windows-1252 (`Utf8String` through
  `SizedCp1252String`). `DataType::String(StringType)` is every string the
  crate has, `DataType::string` its one constructor and `utf8()`,
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
  that stores as the text it is, so it is `DataType::Currency`, kind `Code`,
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
- **`Str` is the string.** `Scalar::String(Str)` holds the characters in the
  crate's compact string - inline to `INLINE_CAPACITY` bytes, static for free,
  one `Arc<str>` beyond - beside the leaf they are stored under. A maximum is
  the column's rule and never the value's: `storage()` is what a value in a
  sized column carries, the plain leaf of its charset, so a cell read out of
  `sized_utf8(32)` is a `utf8`. Equality, order and hash read the characters
  alone, so a value is one value whichever column holds it, and `Borrow<str>`
  is sound for that reason and no other. `Str` is the holder every
  string-family API answers with - `ascii_value`, `StringEnum` members,
  `str_from_value` - and `SmolStr` stays the crate's utility string for names,
  keys and errors.
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
  seam, as everywhere else; what a `Str` keeps is the leaf it is *written*
  under, so it goes back out the way it came without the column being read
  twice. `as_str` is therefore infallible on every string value there is.
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
  governs `DataType::String`; a registered code is outside it. A code is
  US-ASCII text held to one width, so it rides Arrow's `Utf8` whatever else
  is true, and what separates it from the text beside it is the *name*:
  `yggdryl.currency` over `Utf8` with an empty document is a currency, the
  same storage under `yggdryl.string` is the string that document describes,
  and under no name at all it is plain text. `yggdryl.currency` over any
  other storage is a foreign field wearing our name and imports as that
  storage, by the same rule a string document does. The width no column
  enforces is enforced where values enter, which is what makes it a value
  rule rather than a layout.
- **`Str::from_bytes` is the one door bytes take, and the charset decides how
  strict it is.** UTF-8 and US-ASCII are validated repertoires - Arrow
  guarantees the first and the second rides Arrow's text storage - so bytes
  that are not what they claim are refused, and a US-ASCII value holds no NUL
  and no byte above `0x7F`. windows-1252 is a declaration that the column
  holds legacy bytes, and those are transcribed rather than refused:
  `Charset::transcribe` reads an unassigned byte as its ISO 8859-1 scalar.
  The row door, the Arrow cell reader and the cast all call that one function,
  so a batch and a row cannot read bytes differently; a text-storage cell is
  `Str::from_storage`, checked when it was written and not again.
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

- **One datatype, one value, six leaves.** `BytesType` is an enum whose
  leaves are the columns - `Binary`, `LargeBinary`, `BinaryView`,
  `LargeBinaryView`, `FixedBinary(u32)`, `SizedBinary(u32)` -
  `DataType::Bytes(BytesType)` is every byte column the crate has,
  `DataType::bytes` is its one constructor and `binary()`, `large_binary()`,
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
  `blob`, `bytea`, `varbinary(n)` and `fixed_size_binary(n)` are accepted
  spellings and render as the canonical ones; `BytesType::from_spelling`,
  `with_bound` and `with_declared_bound` are the one table and the one rule
  the grammar and both bindings read.
- **`Bytes` is the byte string.** `Scalar::Bytes(Bytes)` holds the payload
  inline to `INLINE_BYTES` bytes with no heap behind it, static for free, one
  `Arc<[u8]>` beyond, beside the leaf it is stored under. A maximum is the
  column's rule and never the value's: `storage()` is the plain leaf a value
  in a sized column carries. Equality, order and hash read the payload alone,
  and `Borrow<[u8]>` is sound for that reason and no other. `Bytes::from_storage`
  adopts a cell of a column's own storage; `bytes_from_value` is what a value
  spells as bytes.
- **Arrow already says the layout.** The storage is the leaf, and only what
  no Arrow type can state - a maximum, and the second view width - rides the
  `yggdryl.bytes` document; a fixed width is the storage itself. A document
  over a storage it does not describe imports as its storage. Avro and
  Iceberg have no maximum either, so a bounded column crosses them unbounded
  and the bound is enforced where the values enter.

## Structured codecs

The JSON, YAML, and TOML sections of `docs/media/index.md` document the
surface; these bind a change to `json/`, `toml/`, `yaml/` and the codec
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
  order is JSON, TOML when complete and non-empty, then YAML. Never infer JSONL
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
- `yggdryl::arrow` owns Struct scalar/array, batch, reader, IPC conversion:
  exhaustive, field-directed, at most one source batch held, never JSON.
  `arrow::scalar_array`/`arrow::scalar_value` are the single scalar-array
  boundary, where the exact `Field` controls nullability, dictionaries, extension
  identity.
- `ArrowCast` owns recursive array/batch casting: Struct casts reconcile names,
  reject ambiguous folds, follow target order, fill valid missing fields, preserve
  exact arrays after logical validation. Wrapper exposure propagates: hidden child
  failures and nulls stay hidden.
- `ArrowCastOptions` carries the three independent answers a cast needs and every
  entry point takes it: `safe` = may a present value convert, `Nullability` = may
  a declared value be absent, `Representation` = what a same-width pair carries.
  `Representation::Bits` shares the value buffer between two fixed-width layouts
  of one byte width; it is a preference, so an unlike pair or a rule-governed
  target converts as it always did.
- `ArrowCastPlan` is the schema-dependent half compiled once: immutable,
  `Send + Sync`, `compile`/`preflight`/`apply`, one plan per reader. Only masks,
  offsets, dictionary reachability vary per batch; an exact cast returns the
  caller's own batch.

# 2. Validation - smoke locally, prove in CI

Local work is the [smoke loop](#smoke-loop). Proof is `.github/workflows/ci.yml`
on the pushed branch, plus `docs.yml` for the site. Do not rehearse the matrix
locally: two feature lanes, two MSRV toolchains, seven exchange jobs, a JVM,
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
| the crate's own documents under `config/fix/` - the crate's field shard, the fixed row and its two groups | `YGGDRYL_FIX_DUMP_WRITE=1 cargo test --locked -p yggdryl --test fix the_committed_store_carries_the_crate_dump` | a crate field, the fixed row or one of its two groups changes (`rust/src/fix/crated.rs`), or the dictionary is regenerated |
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
| Rust quality (default features, all features) | `cargo fmt`; clippy at `-D warnings` on `-p yggdryl` and on `--workspace --all-features`; `cargo test --all-targets` in both lanes; rustdoc examples; `cargo doc` under `RUSTDOCFLAGS=-D warnings`; the optimized benchmark configuration | the failing step verbatim, with the lane's flags: nothing, or `--all-features` |
| Core Rust 1.85 | the declared MSRV: `--all-targets`, `--no-default-features --lib`, and `--no-default-features --features s3 --lib` - the build a schema-only consumer gets | `rustup toolchain install 1.85.0`, then `cargo +1.85.0 check --locked --manifest-path rust/Cargo.toml -p yggdryl <the failing flags>` |
| Iceberg Rust 1.94 | the official Iceberg boundary at its own, later MSRV | `cargo +1.94.0 check --locked --manifest-path rust/Cargo.toml -p yggdryl --all-targets --features iceberg` |
| S3 / Azure / Google exchange | the object stores against MinIO with boto3, Azurite with azure-storage-blob, fake-gcs-server with google-cloud-storage - signatures and dialects against implementations that answer 403 | `python scripts/check_object_interop.py`, `check_azure_interop.py`, `check_gcs_interop.py`; each fetches its own server |
| ZIP / Avro exchange | `zipfile` and fastavro writing the archive and the container this crate then reads: the direction whose in-tree tests skip when nothing produced the input | `python scripts/check_zip_interop.py`, `python scripts/check_avro_interop.py` |
| PyIceberg exchange | v1, v2, and v3 tables against PyIceberg | `python scripts/check_iceberg_interop.py` |
| Spark interop | Iceberg against the format's reference implementation, behind its own marker | §3, and only for that boundary |
| Python binding wheel | `stage_cli.py --debug`, the maturin wheel at `--profile dev` (CI never measures; the release workflow builds what ships), and the assertion that it carries `yggdryl-<version>.data/scripts/ygg` | the wheel path in §3, with those two debug flags |
| Python binding (`pyarrow==18.*`, `pyarrow>=18`) | `pytest python/tests` and `mypy --strict` on both legs, with pandas, polars, tzdata, and xxhash installed so no suite skips silently | §3, with the leg's pyarrow pinned into `python/.venv` |
| Node.js binding | `test:package:debug`, the generated loader and declarations unchanged, `node --test` plus `tsc --noEmit`, and the two docs manifests | §4 |
| Documentation examples | every fenced block under `docs/` compiled and run in Rust, Python, and JavaScript | `python scripts/check_docs_examples.py --lang <the failing language>` |
| `docs.yml` build | `mkdocs build --strict` - nav, links, and strict warnings | `python -m mkdocs build --strict --config-file mkdocs.yml` |

## What CI never runs

These have no job, so a push cannot find them. They belong to the change that
makes them stale, and a skipped one is reported as skipped:

```bash
python scripts/generate_charset_tables.py --check   # rust/src/charset/tables.rs drift
python scripts/check_charset_interop.py             # every code page against Python's codecs
```

```bash
cargo bench -p yggdryl --bench <types|arrow|uri|text|coding|charset|media|holder|hashing|expression|fix>
npm run --prefix node bench:<coding|fix|hashing:txhash|hashing:xxhash|holder|media|text|types>
python python/benchmarks/<name>.py                  # boundary benchmarks, release wheel
YGGDRYL_S3TABLES_ARN=<table bucket ARN> python python/benchmarks/media/s3tables.py  # a real table bucket and pyiceberg; SKIPPED otherwise
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
- Conversion pairs are Python `as_py`/`from_py` and JS `asJs`/`fromJs`; native
  and Arrow values map through `Scalar` losslessly where the runtime allows.
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
- Structured codec facades stay byte-oriented and native, `cls=` is explicit
  reconstruction, and encoders never close caller-owned streams.

## Python checks

The loop is an in-place debug extension and one scoped suite; before pushing,
the same over the whole tree plus the type checker:

```bash
V=python/.venv/bin/python
$V -m maturin develop -m python/Cargo.toml     # in place, debug, no wheel
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
  wheel it builds must carry `yggdryl-<version>.data/scripts/ygg`.
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
- Arrow JS interop is copied IPC with bounded cursors and a validated cached
  schema - never claim zero-copy; public IDs are transport-local while native
  records keep canonical IDs.
- JSON/YAML/TOML facades are byte-first over native `Scalar`, preserving
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
  `docs/media/index.md` is the one page for every media type, content coding
  and charset: a Read and write overview (native rows, then Arrow batches, then
  `RecordOptions`), then one short section per media type - IPC, Parquet, Avro,
  plain text, JSON, YAML, TOML, Iceberg - then Compression (gzip, zlib, zstd)
  and Charsets. Each section is a sentence or two and a tabbed example; the
  example carries the detail, not the prose. A section's benchmarks sit in its
  own `<section> performance` subsection, never in a shared one.
  `docs/holder/index.md` is the same kind of page for storage: Handles
  (`Holder`, roles, delegation), then the `IOBase` surfaces - Bytes, Values,
  Records, Partitions, Call counts - then one section per backend: Buffer,
  Local, Filesystems, Object stores, Buffered, ZIP. A section may open with a
  `text` block of the few public signatures a caller implements or reaches for
  first, each with a one-line comment on what it promises; it is never the
  whole surface, which rustdoc owns.
  `docs/types/` is a theme of the same kind: the Core pages - `datatype.md`,
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
python scripts/check_docs_examples.py --lang rust         # compiled against parquet iceberg s3
python scripts/check_docs_examples.py --lang python       # runs under python/.venv
python scripts/check_docs_examples.py --lang javascript   # needs the built addon beside Arrow JS
```

## Handoff

- Sweep for dead code, duplicated logic, retired symbols, stale docs, Rust-only
  bindings a stable core no longer justifies.
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
- A version is out on all three registries or on none. The `consistency` job
  reads crates.io, PyPI and npm before anything builds and refuses a branch
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
- A bump moves seven files together, and `preflight` reads three of them:
  `Cargo.toml` and `Cargo.lock`, `python/pyproject.toml`, `node/package.json`
  and `node/package-lock.json`, and the two generated documentation manifests
  `docs/assets/fix.json` and `docs/assets/playground.json`, which stamp
  `node/package.json`'s version.
- Credentials stay in repository configuration: Cargo and npm secrets, PyPI
  trusted publishing. No stored PyPI password, no fourth registry.
