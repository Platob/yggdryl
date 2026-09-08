# Yggdryl agent contract

Arrow-native core in Rust (`rust/`), two native views - Python (`python/`) and
Node (`node/`) - and the `ygg` CLI (`cli/`). Rust owns `DataType`, `Field`,
`Scalar`, identifiers, I/O, codecs, shared enums; a binding redirects into it and
implements nothing of its own.

One direction, every gate blocking:

**Rust core -> Gate 1 -> Python -> Gate 2 -> Node -> Gate 3 -> docs -> Gate 4.**

Never open the next stage, or report done, over a red or unrun gate. Report exact
results and exact skipped checks.

## Workflow

1. **Locate** the owning layer in [Layout](#layout); read its neighbours and the
   names in `.api-inventory.txt` (Rust) / `.api-bindings.txt` (Python, JS).
2. **Design against §1**: one owner per fact, one spelling per verb, no second
   schema, no second dispatcher - and the [Patterns](#patterns) the core already
   has for equivalences, the handle stack, row accessors, and what is zero copy.
3. **Implement in Rust**: behavior, edges, errors, `rust/tests/`,
   `rust/benchmarks/`, rustdoc examples, both directions of any exchange format.
   Delete what it replaces in the same commit.
4. **Gate 1** green and the core contract settled before a binding exists.
5. **Python** (§3): redirects, parity tests, boundary benchmarks -> **Gate 2**.
6. **Node** (§4): the same -> **Gate 3**.
7. **Docs** (§5): every layer touched, examples in all three languages ->
   **Gate 4**.
8. **Handoff** (§5): sweeps, inventories, cleanup, report.

Rust-only is complete work when the core is the requested scope; a missing
binding is documented as Rust-only. Never pin an unsettled design by writing a
binding first.

### Common changes, in order

| Change | Touch, in this order |
| --- | --- |
| datatype variant | `types/` family module, `DataTypeId`/`DataTypeKind`, parser, serde, comparison, Arrow, cast, `scalar` -> tests -> bindings -> `docs/types/` |
| logical name | `DataType::LOGICAL_NAMES` only; resolves to an existing datatype, adds no variant |
| codec | `coding/<name>.rs` (`load`, `dump`, `reader`, `writer`, `IOBase` wrapper) + a `Codec` variant -> bench -> bindings -> `docs/coding/` |
| storage backend | `holder/<name>/` with `Path`, `Folder`, `File` over the root traits; state and assert its call/request counts -> interop script -> docs |
| media format | `media/<name>/` free functions over `IOBase` + a stateful wrapper, reached through `MediaType`/`RecordOptions` -> interop both directions -> docs |
| metadata property | a protocol view keyed `<scheme>:<property>`; never a new `Field` accessor |
| binding method | core method first; the binding only infers, coerces, redirects - plus a parity test, a boundary benchmark, a docs entry |

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

**Compressed output.** Only what changes a decision, proves a result, names a
blocker, or enables the next action; each fact once; outcome first (state,
evidence, next action). No greeting, praise, throat-clearing, repeated context,
narrated tool use, reassurance, sign-off, restated request. Progress at start,
material change, blocker, gate result. Handoff keys: `Goal`, `Invariants`,
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
`default-members = ["rust"]`; features are `default = ["arrow"]`, `parquet`,
`iceberg` (implies `parquet`), `s3`. Examples live in docs - no `examples/` dir -
and tests, benchmarks, bindings, and docs mirror these layers. A root file is not
an implementation layer, a layer is not a facade over root-owned vocabulary, and
a module owns implementation rather than an empty facade.

Paths below are under `rust/src/` unless stated otherwise.

| Path | Owns |
| --- | --- |
| `<name>.rs` | one shared trait, enum, or value each, re-exported from the crate root |
| `iobase.rs` | the single `IOBase` trait and its behavior modules |
| `types/` | `Scalar`; schema behavior by category: state, parser, serde, comparison, Arrow, casting, value validation, typed markers, datatype families |
| `holder/` | `Buffer`, local handles, generic `fs` handles, `Buffered<H>`, `Counted<H>`, storage variants; each backend a sibling folder (`local/`, `s3/`, `zip/`) with `Path`, `Folder`, `File` |
| `holder/local/` | memory-mapped local storage; remote backends change neither it nor the root traits |
| `holder::fs::FileSystem` | Arrow's seven-method shape for interop; core contract and variants keep generic `FileSystem`/`Fs*` names |
| `coding/` | transparent `Coded` handles; `{gzip,zlib,zstd}.rs` each own `load`, `dump`, `reader`, `writer`, an `IOBase` wrapper |
| `media/` | record routing and settings; `{ipc,parquet,avro}/` each own free functions over `IOBase` plus a stateful wrapper |
| `media/text/` | `Text<H>`, flat `TextOptions`, bounded physical-line splitting, row-header capture, body rendering |
| `media/iceberg/` | separate modules: types, schema, partition, snapshots, metadata, manifests, statistics, scalar rendering, scan, table, options, catalog, evolution, inspection |
| `text/` | JSON/YAML/TOML over `Scalar` |
| `uri/` | URI, URL, URN |
| `arrow/` | Arrow interop; recursive cast planning stays with `Field` |
| `expression/` | expression grammar, bound statements |
| `xxhash/` | one-shot digests, four resumable states, `reader`/`writer`, `Hashed<H>`, the canonical `Scalar` byte feed, Arrow row digests |
| `fix/` | FIX protocol behavior |
| binding `lib.rs` | boundary helpers, exports, registration - nothing else |

Parquet is feature-gated; Avro's scalar codec is unconditional and its record
surface uses Arrow; Iceberg sits on these codecs. `Text<H>` keeps only options
and delegates ordinary `IOMedia` - no line value, custom iterator, schema
builder, or line-only read/write. Sole dispatchers, delegating complete contracts
with no variant-specific public vocabulary: `Codec` (coding), `DigestAlgorithm`
(digests), `MediaType` via `RecordOptions` (encoding).

## Ownership

- One row schema: a non-null Struct `Field`. Rows canonicalize to ordered
  `Scalar::Sequence`; `Scalar::Record` is a sorted name-to-scalar *input* shape.
  No second row/schema class or accessor.
- `Field` alone owns metadata and cache-aware mutation; `DataType` has none.
  Protocol metadata is inert `<scheme>:<property>` text in one map; a protocol
  view borrows a whole `Field` and derefs to it, and typed protocol vocabulary
  (`digest:role`, the `partition:` pair) lives there, never on `Field`. `Field`
  owns `field:init`, `field:partition`, `alias`, `comment`, `display`,
  `location` under any key; `PARQUET:field_id` is the reserved typed exception.
- `holder` is the only digest role: a declaration says what a field holds or
  derives, never what another contributes - mark one field, leave its sources
  ordinary columns.
- Shared and dispatch enums each live in their named root file, re-exported from
  the crate root: `Codec`, `DataTypeId`, `DataTypeKind`, `DigestAlgorithm`,
  `EdgeAlgorithm`, `IOKind`, `IOMode`, `Level`, `Magic`, `MediaType`, `MimeType`,
  `Scheme`, `TimeUnit`, `TimeZone`, `UnionMode`. No local copies, no `enums`
  module. `Digest`/`Digester` sit beside `DigestAlgorithm`, `Encoder` beside
  `Codec`; `Scalar` -> `types`, storage variants -> `holder`, record settings ->
  `media`.
- `IOMode` = `ReadOnly`, `Overwrite`, `Append`, `Merge`, `Random`; operations
  reject modes that do not apply; no alias.
- `DataTypeId` = exact variant, `DataTypeKind` = family. `TimeUnit` is the only
  temporal/interval unit parser and Arrow converter; `MimeType`/`MediaType` own
  MIME parsing, suffix and content-coding inference, preferred extensions;
  `Scheme` owns URI and compatibility scheme vocabulary.

## Patterns

### `DataType`, `Field`, `Scalar`

| Concern | Type | Holds |
| --- | --- | --- |
| shape | `DataType` | no name, no nullability, no metadata |
| schema | `Field` = name + `DataType` + nullable + metadata | a non-null Struct `Field` is the row schema |
| value | `Scalar` | one variant per physical width |
| checked value | `DataType::scalar(v)`, `Field::scalar(v)` | the only way a caller value becomes a stored one |
| narrowed view | `TypedField<K>`, `TypedFieldRef<'_, K>`, `TypedScalar<K>` | a marker validating the variant; parameters stay in the wrapped `Field` |
| Arrow value | `arrow::ArrowValue` | one scalar, array, batch, or stream under one `Field` |

Equivalences a change keeps lossless, in both directions:

- `DataType`/`Field` <-> Arrow, through `from_arrow`/`into_arrow` and the core
  recursive exporters - never a schema rebuilt in a binding.
- `Scalar` <-> Arrow array or scalar, through `arrow::scalar_array` and
  `arrow::scalar_value` under the exact `Field`, which decides nullability,
  dictionaries, extension identity.
- rows <-> ordered `Scalar::Sequence`; named input <-> sorted `Scalar::Record`
  (`from_record`), canonicalized against the Struct `Field`;
  `ArrowValue::from_rows` and `into_scalar` cross the same way.
- a datatype's canonical default is `default_value`/`is_default_value` - the
  value a declaring protocol's `apply_arrow_batch` leaves alone.
- widths: a family constructor picks the physical width once, and shared logic
  reads across widths with `as_integer`, `as_float`, `as_decimal`, `as_temporal`.

### Stack: holder -> media -> arrow

| Level | Surface | Answers |
| --- | --- | --- |
| bytes | `IOBase`: `pread`/`pwrite`, `read_all_bytes`, `read_range_bytes`, `append_bytes`, `pstream_bytes`, `read_digest` | positional bytes, digests, bounded streams |
| position | `IOCursor`, `Cursor<H>` | the only place a cursor is retained |
| records | `IOMedia`: `read_arrow_field`, `read_arrow_reader`, `read_arrow_value`, `write_arrow_*`, `*_records`, `row_size`, `column_size`, `record_options` | schema, rows, batches, statistics |
| values | `yggdryl::arrow`: `scalar_array`, `scalar_value`, `ArrowValue`, `cast_reader`, `combined` | the `Scalar`/Arrow boundary |

`IOBase: Send + IOMedia`, so every handle answers records; a media wrapper
implements `overwrite_arrow_reader` and inherits streamed append and merge.
Wrappers compose over a handle, never inside it - `Coded` (coding), `Buffered`
(holder), `Hashed` (xxhash), `Counted` (tests) - each forwarding through
`delegate_iobase!` and overriding only what it changes. Commit cadence belongs to
`RecordOptions` and the write session in `iobase/transfer.rs`
(`ArrowWriteSession::{overwrite,append,merge}` with `push` and `finish`/`abort`),
never to a wrapper's own buffer.

### Record and row accessors

- Whole value: `read_scalar(field)` / `write_scalar(value)`. Schema alone:
  `read_arrow_field(options)`.
- Rows out: `read_arrow_reader(options)` streams; `read_arrow_value(field)`
  answers an `ArrowValue` carrying its own shape.
- Rows in, by shape, each with `overwrite`/`append`/`merge` plus a generic
  `write_*` taking an `IOMode`: `*_arrow_reader` (the streamed primitive),
  `*_arrow_batch` (one batch), `*_records` (a row iterator), `write_arrow_value`.
- Navigate a row `Scalar` with `get`, `get_key_str`, `path`, `iter`,
  `sequence_iter`, `record_iter`, and update with `with_field`/`without_field`;
  a row is an ordered sequence, never a map.
- `ArrowValue` reports `shape`, `is_scalar`/`is_array`/`is_batch`/`is_stream`,
  `row_size`, `column_size`; borrows with `as_array`/`as_batch`; consumes with
  `into_array`/`into_batch`/`into_reader`/`into_scalar`; converts with `cast`.
- Add no row type, schema accessor, or per-row map/JSON bridge; a binding's row
  helper closes over one Struct `Field`.

### Zero copy

Holds, and is asserted with the counting allocator at several corpus sizes -
timing alone proves nothing:

- borrowed views allocate nothing: `as_*`, `as_array`, `as_batch`, `as_field`,
  `TypedFieldRef`, `ProtocolField`; `into_*` is the allocating counterpart.
- an exact cast returns the caller's own batch, and `Representation::Bits` shares
  the value buffer between two same-width layouts.
- `holder/local/` is memory-mapped, `Buffered<H>` pins pages instead of copying
  them forward, and shared nesting clones a reference while empty collections
  hold no backing.
- Python crosses the C Data Interface and PyArrow holders.

Does not hold, and is never claimed: JavaScript interop is copied IPC with
bounded cursors; `read_all_bytes`, any `Vec` return, `into_*`, and text or JSON
rendering allocate by contract.

## Public vocabulary

Names describe ownership and return type; alternate-verb aliases are forbidden.
Check a name in `.api-inventory.txt` before writing it; edit that file in the
change that adds or retires one.

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

No project-defined plain `to_*`; foreign protocols (`ToString`, JS `toString`)
keep their spelling. Implement `From`, `TryFrom`, `FromStr`, `AsRef` where
coherent; bindings redirect through stable inherent methods. Exceptions:

- `field`, never `schema`, in options and accessors.
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
- `holder::local::Folder` roots `temporary`, `home`, `config`: `home` reads
  `HOME`, then `USERPROFILE`, failing and naming both when neither is set;
  `config` = `home` + `.config`; `temporary` wraps the platform temporary
  directory. All three construct a handle and create nothing; nothing else
  reaches these directories through `std::env` or concatenation.
- `TypedScalar<K>` = one validated `Scalar` + a datatype marker, owning no
  `Field`; Arrow projection routes through the core scalar-array boundary.

## Generic scalar

- `types::Scalar` is the single cross-platform scalar: no parallel value tree, no
  retired alias.
- Variants match native/Arrow widths: `F16`, `F32`, `F64`; `D128`, `D256`;
  `Date32`, `Date64`; `Time32`, `Time64`; `Duration32`, `Duration64`; one
  `DateTime64`. Temporals keep the `TimeUnit`/`TimeZone` their datatype needs;
  `DateTime64` always has a non-null `TimeZone`, naive spelled `TimeZone::Naive`.
- `Scalar::Record` is a deterministic sorted name-to-`Scalar` map, resolved to an
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
- Rust keeps exact-width variants and constructors; shared logic goes through
  `as_integer`, `as_float`, `as_decimal`, `as_temporal`, and a family constructor
  picks the physical width once.

## Datatypes, parsers, errors

- `DataType::from_str` and `Field::from_str` are the recursive schema grammars;
  bindings pass expressions straight through. Accept canonical plus common
  Arrow/SQL/Hive/Spark forms under an explicit recursion limit.
- `DataType::LOGICAL_NAMES` is the one fallback registry: FIX Latest datatype
  vocabulary plus `mic`, each name resolving to the closest core datatype and
  displaying as it - no variant, no second spelling. Never register a word the
  Arrow/SQL grammar owns. `AsciiEnum::PREBUILT` keys the ISO code constants three
  of those names prebuild; `AsciiEnum::from_logical_name` builds the enum a field
  declares from one. A listing is a constant: every reader answers the same
  members.
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
  `holder::counted::Counted` tallies forwarded calls by name, S3 `Stats` counts
  the requests one call becomes.
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

### S3 (`holder/s3/`, non-default `s3` feature)

S3 REST spoken directly - SigV4 over a synchronous HTTP/1.1 client, no SDK,
runtime, or object-store layer. Each method states its request count and the
accounting tests assert it exactly:

| Operation | Requests |
| --- | --- |
| ranged read | 1 ranged `GET` |
| whole read, full stream drain | 1 `GET` |
| listing, flat or recursive | 1 per 1000 entries |
| prefix removal | 1 listing + 1 bulk delete per 1000 keys |
| construction, child resolution, trailing-slash location | 0 |

A listing states every entry's size, so a listed object never re-asks; `open`
caches metadata, never bytes; pooled connections mean a body is always drained;
payloads are signed over plain HTTP, unsigned over HTTPS.

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
- `options.field` is the only declared datatype/schema, rebuilt on every ask from
  three stored parts - `name` (default `types::DEFAULT_ROOT_NAME`), `dtype`
  (undeclared = inferred), `metadata` (empty unless declared) - so each mutates
  alone and equal declarations have one stored form. Reads project in the
  encoding and cast each batch; writes cast once, pop the field before delegating
  to overwrite, never materialize the stream.
- `options.commit_row_size`: unset = one commit; `N` publishes every `N` rows plus the
  remainder, holding at most one bounded commit; the first overwrite commit
  overwrites, later ones append; failure leaves published prefixes visible.
- Overwrite replaces rows under the stored field; append retains stored rows;
  merge needs non-empty keys, updates matches, appends misses, and streams
  incoming batches - no positional upsert. `row_size`/`column_size` are lazy
  cached metadata from cheap media answers, never a full read.
- Encoding comes from `MediaType` through `RecordOptions`, with no format
  argument; generic `write_*` takes an `IOMode` and redirects to specialized core
  paths.
- Plain-text rows start with required `url: utf8` and `body: binary`;
  `TextOptions.start_rownum: Option<i64>` inserts required `rownum: int64` between
  them and names its first value. `parse_mtime`, on by default, inserts nullable
  `mtime: datetime64(ns, UTC)` after it, filled by the row header's `mtime`
  capture when the expression declares one and by `IOBase::mtime` when it does
  not - one column whichever answered, and null when neither can. Flat `TextOptions` owns named `rowheader`
  captures, edge-only regex stripping, a line separator, and syntax-directed
  `autotype` via `DataType::from_regex`, so the full source field is known before
  a read. `timezone` stays a shared `RecordOptions` accessor over offset-free
  datetime captures; writes consume only non-null binary `body`.
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
- A derived column names its own input: `partition:transform` is an expression
  grammar function over the field paths in `partition:sources`, both stored on
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
  `digest:sources` is the exact input, resolved against its own Struct; `["*"]`
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

`docs/media/iceberg/` documents the format surface and its edges; these bind a
change to `media/iceberg/`.

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

## Structured codecs

`docs/text/` documents the surface; these bind a change to `text/`.

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
  (plus a `Scheme`). No duplicated state, no unchecked mutable path that can
  invalidate the marker.
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
- Core scalar creation, getters, lookup, iteration setup, and shared nesting
  clones do not allocate; corpus sizes vary in the check, and timing alone never
  proves it. Preflight slot and fixed-buffer budgets before allocating.
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

# 2. Gate 1 - Rust validation

Blocking; run from the repository root. Nothing below starts until it is green.

```bash
cargo fmt --all -- --check
cargo clippy --locked -p yggdryl --all-targets --no-deps -- -D warnings
cargo clippy --locked --workspace --all-targets --all-features --no-deps -- -D warnings
cargo test --locked -p yggdryl --all-targets                            # default features
cargo test --locked -p yggdryl --all-targets --features "parquet iceberg"
cargo test --locked -p yggdryl --doc                                    # rustdoc examples
RUSTDOCFLAGS="-D warnings" cargo doc --locked -p yggdryl --no-deps
cargo check --locked -p yggdryl --profile bench --benches
```

MSRV, and the feature-off builds a schema-only consumer gets:

```bash
cargo +1.85.0 check --locked --manifest-path rust/Cargo.toml -p yggdryl --all-targets
cargo +1.85.0 check --locked --manifest-path rust/Cargo.toml -p yggdryl --no-default-features --lib
cargo +1.85.0 check --locked --manifest-path rust/Cargo.toml -p yggdryl --no-default-features --features s3 --lib
cargo +1.94.0 check --locked --manifest-path rust/Cargo.toml -p yggdryl --all-targets --features iceberg
```

Cost-model and allocation claims are assertions, not arguments - every derived
`IOBase` surface touched re-runs its pinned count:

```bash
cargo test --locked -p yggdryl --test iobase_calls --test allocations
```

Exchange formats, both directions, against outside implementations; a skipped
half is a failure, not a pass:

```bash
python scripts/check_zip_interop.py       # Python zipfile
python scripts/check_avro_interop.py      # fastavro, plus the apache-avro probe
python scripts/check_s3_interop.py        # MinIO + boto3
python scripts/check_iceberg_interop.py   # PyIceberg, v1/v2/v3 tables
```

Benchmarks for every touched surface, release build, numbers regenerated:

```bash
cargo bench -p yggdryl --bench <types|arrow|uri|text|coding|media|holder|xxhash|expression|fix>
```

# 3. Python

Gate 1 first. Rules shared by both extensions:

- Reach every stable core domain; a missing binding is documented as Rust-only.
- Expose only the `Scalar.float`, `decimal`, `date`, `time`, `datetime`,
  `duration` family factories; exact widths stay private Arrow/transport identity.
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
  builder `field(value, name=None)`, typed field factories below
  `yggdryl/fields/`. `@scalar` forwards every stdlib dataclass option, installs
  one cached argument-free `staticmethod field()`, rejects a pre-existing `field`
  member, and reserves no static metadata constant.
- `Class.field()` returns one frozen non-null Struct `Field`, preserving
  dataclass order and metadata, excluding `ClassVar`/`InitVar`/private working
  annotations, resolving forward and generic annotations once, detecting
  recursion, synchronized on first access. Optionality defines default
  nullability, explicit annotation options win, defaults/factories affect
  construction rather than schema, and generated dataclasses derive annotations
  from the exact native field graph.
- No second row decorator or class, static field constant, schema/into-field
  alias, or retired public surface.
- `pyarrow.RecordBatchReader` is the primitive record shape - table, batch, and
  dataclass row methods redirect through it over the C Stream interface, on the C
  Data Interface and PyArrow holders.
- Structured codec facades stay byte-oriented and native, `cls=` is explicit
  reconstruction, and encoders never close caller-owned streams.

## Gate 2 - Python validation

Blocking.

```bash
python scripts/stage_cli.py               # maturin copies wheel-data; it builds no binary
maturin build --locked --manifest-path python/Cargo.toml --interpreter python --out python/dist
python -m pip install --force-reinstall --no-deps python/dist/*.whl
python -m pytest python/tests
python -m mypy --strict --config-file python/pyproject.toml \
  python/yggdryl python/tests/typing_bindings.py python/tests/types/typing_fields.py
python python/benchmarks/<name>.py        # boundary benchmarks, release wheel
```

- Run the tests under both `pyarrow==18.*` and `pyarrow>=18`.
- Install `pandas`, `polars`, `tzdata`, `xxhash` first or those suites skip
  silently; a silent skip is a failed check.
- Iceberg-with-Spark is opt-in: `python scripts/setup_spark_interop.py`, then
  `python -m pytest python/tests/media/test_spark_interop.py -m spark_interop`.
- The wheel must carry `yggdryl-<version>.data/scripts/ygg`.

# 4. Node

Gate 2 first; the shared binding rules in §3 hold here too. JavaScript-only:

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

## Gate 3 - Node validation

Blocking.

```bash
npm ci --prefix node
npm run --prefix node test:package:debug                 # build + loader/type audit + package files
git diff --exit-code -- node/index.js node/index.d.ts    # generated loader and declarations current
npm test --prefix node                                   # node --test plus tsc --noEmit
node scripts/build_docs_playground.js --check            # generated docs manifests not stale
node scripts/build_docs_fix.js --check
npm run --prefix node bench:<coding|fix|holder|media|text|types|xxhash>   # release addon
```

# 5. Documentation

Write for lookup - the readers are human scanners and LLM retrieval. Contract,
then the smallest runnable example, then non-obvious edges, then measured
performance. Canonical symbol names, stable headings, short paragraphs, tables
only for exact mappings, exact commands and results preserved. One fact in one
place: link instead of paraphrasing, and never narrate signatures, repeat
examples in prose, add marketing text, or create benchmark-only pages.

The layer tabs, the page skeleton, and the per-change docs rules are spelled out
in `docs/architecture.md` and `docs/contributing.md`; those pages and this
section change together. What binds every page:

- Root `mkdocs.yml` is authoritative - strict build, nav, and links change
  together, README stays a short landing page. A family page lives under
  `docs/<layer>/` for the layer owning the vocabulary, with
  `docs/<layer>/index.md` as its overview; extension pages document boundaries
  only.
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

## Gate 4 - Documentation validation

Blocking.

```bash
python scripts/check_docs_examples.py --lang rust         # compiled against parquet iceberg s3
python scripts/check_docs_examples.py --lang python       # runs under python/.venv
python scripts/check_docs_examples.py --lang javascript   # needs the built addon beside Arrow JS
python -m mkdocs build --strict --config-file mkdocs.yml
```

## Handoff

- Sweep for dead code, duplicated logic, retired symbols, stale docs, Rust-only
  bindings a stable core no longer justifies.
- `.api-inventory.txt` and `.api-bindings.txt` are hand-maintained: a change
  adding or retiring a public name edits them in the same change.
- Remove only generated targets, site output, virtual environments, binaries,
  caches, and `node_modules` that validation created; preserve unrelated work.
- Report gate results, failures, exact skipped checks, material caveats - nothing
  else.

# 6. Releases

- `main` triggers `.github/workflows/release.yml`; `v<version>` is the receipt
  created after registry publication, and manual runs rehearse only.
- Publishes are idempotent, the tag is last, partial publication is repaired by
  rerunning, and a released version is never reused.
- Root Cargo, Python, and Node versions match exactly. Publish crates.io, PyPI,
  npm only after platform smoke tests import and exercise the artifacts.
- Credentials stay in repository configuration: Cargo and npm secrets, PyPI
  trusted publishing. No stored PyPI password, no fourth registry.
