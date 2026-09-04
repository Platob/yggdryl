# Architecture

Five decisions shape this codebase; everything else follows from them.

```text
                      DataType ──────────────► arrow-schema
                          │
                          ▼
   metadata ──────────► Field ◄──── a non-null Struct Field is the schema
                          │
          ┌───────────────┴───────────────┐
          ▼                               ▼
   field::cast                      arrow (scalars,
   (ArrowCast, typed                 schema projection,
    per-datatype casts)              BatchReader)
          │                               │
          └───────────────┬───────────────┘
                          ▼
                  ipc  /  parquet  /  iceberg
                          │
                          ▼
                       IOBase ◄──── one storage trait: positional and lazy
                          │
     ┌────────────┬───────┴───────┬────────────────┬──────────────┐
     ▼            ▼               ▼                ▼              ▼
   Buffer       Coded         local::Path    arrowfs::Path   (your backend)
  (memory)   (gzip/zlib/      ├─ Folder      ├─ Folder       via IOPath/
              zstd view)      └─ File        └─ File         IOFile/IOFolder
                                             over any Arrow
                                             filesystem

   Holder │ Media │ Codec │ RecordOptions │ Scalar ──── generic/

   Uri ──► Url / Urn ──► MimeType + MediaType ──► Codec vocabulary
```

## A schema is a field

There is no record type and no schema type. A [field](field.md) whose datatype is a non-null
`Struct` describes rows, and everything that would take a schema takes that field.

A schema therefore cannot disagree with the field it came from, because it *is* that field.
Validation, canonicalization, Arrow projection, and comparison are all field operations, and every
encoding, cast target, and Iceberg table reads the same value.

A protocol namespace is a view of that one value, not a second one. `field.as_iceberg()` borrows the
whole field and dereferences to it, so a foreign protocol's typed vocabulary lives on its view -
[`HttpField`](field.md#one-protocol-at-a-time), `IcebergField`, and the rest - while the field keeps
only its own state.

## Storage is one trait

[`IOBase`](io.md) is positional rather than cursor-based: `pread` and `pwrite` take an explicit
offset, so a footer-first container reads its index without seeking a shared cursor and two readers
share one handle without coordinating.

Handles are lazy by contract. Constructing one touches nothing; a read of something absent yields
zero bytes; a write creates the resource and any parent it needs. That rule is why a location can be
described, passed around, and inspected before anything is there.

Lifecycle is stated rather than implied. `clear` empties, `remove` deletes, and each reaches the
backend once - absence is a completed removal, not something to check for first, so neither ever
issues a probe. A `remove` is the *complete* removal: pending writes and caches go with it.
`open`/`close` bracket the only window in which a handle may cache, so a *complete* write publishes
when it finishes and only a positional one stages.

## A role implements the boilerplate

Every storage backend has the same three roles, and the role traits pre-implement what follows from
each: a folder holds no bytes and refuses byte writes, a leaf contains nothing and lists nothing,
and a [generic location](local.md) answers by looking. `local` is the reference
implementation; a remote store is a sibling module supplying the same three roles.
[`arrowfs`](arrowfs.md) is that module for every filesystem the Arrow project already
implements - S3, GCS, Azure, or a caller's own - so reaching a bucket costs a handle, not a
change to `io` or `local`.

## One enum per contract

A trait says what an implementation must do; the enums in [`generic`](generic.md) say which
implementations exist. `Holder` names every `IOBase`, `Media` every record encoding, `Codec` every
content coding, `RecordOptions` every encoding's settings. Holding one means holding "some handle"
as a concrete value, which is what lets a listing or a binding pass an implementation around without
knowing which one it is.

`generic` also owns [`Scalar`](text.md), the one native value the whole project speaks: every
codec parses into it, every field validates it, and every binding converts its own objects to it.

## Arrow speaks batches; codecs speak values

Two currencies, deliberately separate. [Arrow](arrow.md) carries columnar batches, and reading
[IPC](ipc.md) or [Parquet](parquet.md) returns a reader that streams them rather than
collecting. `Scalar` is what [JSON](json.md), [YAML](yaml.md), and [TOML](toml.md)
read and write.

There is no row-to-Arrow conversion layer between them: converting a batch to values is a caller's
decision with a caller's cost, not something the storage path does implicitly.

## One filter, three tiers

There is exactly one way to say which rows: [`Expression`](expression.md). It parses through one
grammar, types itself against a schema, compiles once, and then answers three ways - row at a time
over a `Scalar`, vectorized over an Arrow batch, and three-valued over container statistics.

The three tiers share one resolved tree, which is the mechanism rather than the intention behind
their agreeing. Where Arrow owns a kernel the kernel runs; where it does not, the row evaluator runs
and its answers are gathered, which is slower and cannot disagree. The statistics tier answers
`false` only when it can prove no row matches, so everything it cannot prove costs a read rather than
a lost row.

The `(column, value)` pairs the older surfaces take are sugar that builds an expression. There is no
second implementation behind them, in the listing, in the record options, or in the table scan.

## One shape per hierarchy level

The Iceberg catalog is three levels - catalogs of namespaces of tables - and every level reads the
same way. A *collection* is a lazy map-oriented view whose construction touches nothing: `get`
raises a typed absence, `create` raises a typed conflict, `open_or_create` absorbs both as the same
attempt, `contains` answers the caller's own question, and `iter` yields names one at a time. A
*resource* is one addressed thing: its dotted name, its kind, its properties, and its child
collections. Dotted names resolve inside the collections - `tables.get("sales.eu.orders")`
descends - so the rule lives in one place, and no level invents a verb the others lack.

The shape exists to be learned once and reused twice - and to be leaned on: the Doris bridge and
any future catalog-shaped surface address `catalog.namespaces()...tables()` rather than growing a
flat method per operation.

## Listings yield, they do not collect

A listing says what is there, and it must never require holding all of it. `IOBase::ls`, `glob`,
`children_matching`, and `children_where` all answer with one named iterator type, `Listing`, over
`Result<Holder>`: the walk runs as the caller drains it, a failure arrives as an entry and fuses the
iterator, and order is deterministic. The argument is the same one the three record methods make
about batches - a shape that has to materialize cannot describe a resource larger than memory - and
it applies to entries for the same reason.

`IOBase` stays object-safe, which is why this is one named type rather than `impl Iterator` or a
`Box<dyn Iterator<..>>` in a public signature: a `dyn IOBase` keeps working and a binding can name
what it wraps. There is one such type because there is one item kind. What a recursive walk retains
is its frontier - one level per open depth - and everything that stays owned says what bounds it.

## Bindings are views

The Rust crate is the only implementation. [Python](extensions/python.md) and
[JavaScript](extensions/javascript.md) infer their inputs at the boundary and call the core;
recursive parsing, validation, comparison, hashing, and conversion never happen twice.

## Module map

| Module | Owns |
| --- | --- |
| `datatype` | [The logical type tree](datatype.md) |
| `field` | [Names, nullability, metadata, borrowed protocol views, partition marks, validation, casting](field.md) |
| `fix` | [The `fix:` vocabulary, the tag-and-name registry, its shards, the process default, and the message value](fix.md) |
| `arrow` | [Scalars, schema projection, batch readers](arrow.md) |
| `io` | [`IOBase`, `Buffer`, `Coded`, the role traits](io.md) |
| `expression` | [The one filter and projection tree, its grammar, its bind, and its three tiers](expression.md) |
| `generic` | [`Scalar`, `TypedScalar`, enums, `Holder`, `Coded`, `Media`, `RecordOptions`](generic.md) |
| `local` | [`Path`, `Folder`, `File`](local.md) |
| `gzip`, `zlib`, `zstd` | [Content codings and transparent handles](gzip.md) |
| `ipc`, `parquet`, `iceberg` | [Batches and tables on disk](ipc.md) |
| `uri` | [URI, URL, URN, and std path interop](uri.md) |
| `text`, `json`, `yaml`, `toml` | [Structured codecs](text.md) over `Scalar` |
| `text::line` | [`Text<H>`, the text-record handle](io.md#text-records): record splitting, the extractor options, and their Arrow projection - the one place lines are split |
| `buffered` | [`Buffered<H>`, the page cache](buffered.md): fixed-size pages under a byte budget and a time to live, both ends pinned, write-through and invalidating |

## Feature boundaries

`arrow` is on by default. `parquet` and `iceberg` are not: Parquet version-locks to the pinned Arrow
release and pulls a thrift and compression stack. Iceberg adds official
Iceberg 0.10.1 behind a private Arrow 58 adapter and requires Rust 1.94; the
public runtime remains Arrow 59. A consumer that needs only schemas,
identifiers, and text codecs builds with `default-features = false` on Rust
1.85.
