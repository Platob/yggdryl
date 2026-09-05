# Architecture

Shared vocabulary lives in root files; implementations live in nine layer folders. Rust is the
source of truth, and Python and JavaScript are native views of the same contracts.

```text
root traits, enums, and values
        │
        ├── types ─────────► arrow
        │      │                │
        │      └── expression ──┤
        │                       ▼
        ├── holder ◄── coding ◄── media
        │                         │
        ├── uri ──────────────────┘
        ├── text
        └── xxhash

fix ── protocol vocabulary over types + holder
```

## Root files and layer folders

Each `rust/src/<name>.rs` owns one shared trait, enum, or value. For example, `iobase.rs` owns
`IOBase`, `codec.rs` owns `Codec`, and `media_type.rs` owns `MediaType`. The crate root declares
and re-exports them; it contains no second implementation.

Each folder owns one implementation family:

| Layer | Owns |
| --- | --- |
| [`types`](types.md) | `DataType`, `Field`, `Scalar`, type families, protocol views, validation, and casting |
| [`holder`](holder.md) | `Buffer`, local and generic filesystem handles, buffering, and storage implementations |
| [`coding`](coding.md) | gzip, zlib/deflate, zstd, and transparent coded handles |
| [`media`](media.md) | record options, IPC, Parquet, Avro, plain-text records, and Iceberg |
| [`text`](text.md) | structured `Scalar` codecs for JSON, YAML, and TOML |
| [`uri`](uri.md) | URI, URL, URN, path, glob, and partition syntax |
| [`arrow`](arrow.md) | Arrow schema, scalar, array, batch, and reader boundaries |
| [`expression`](expression.md) | parsing, binding, row evaluation, Arrow evaluation, and pushdown |
| [`xxhash`](xxhash.md) | digest values, one-shot and resumable hashes, streams, handles, and row hashes |

[`fix`](fix.md) is a protocol module over those layers: FIX fields remain core `Field` values and
registry storage remains `IOBase`.

Tests, benchmarks, Python modules, JavaScript source groups, and documentation mirror these layer
names. This makes a concept's implementation, validation, boundary, and contract discoverable by
the same path.

## A schema is a field

A non-null Struct [`Field`](types.md) is the only row schema. `Scalar::Record` is named input that
canonicalizes to an ordered `Scalar::Sequence` against that field; it is not another schema type.
Validation, canonicalization, metadata, comparison, Arrow projection, and casting therefore share
one source of truth.

Protocol metadata is a borrowed view of the same field. `field.as_fix()` and
`field.as_iceberg()` add typed foreign vocabulary without copying state or adding protocol fields
to the core type.

## Storage is one trait

[`IOBase`](holder.md) is positional: `pread` and `pwrite` take explicit offsets, so independent
readers never coordinate a hidden cursor. Root traits describe storage roles; implementations in
`holder` supply memory, local, generic filesystem, and buffered behavior. The
generic `FileSystem` trait preserves Arrow's synchronous method signatures so
existing Arrow-compatible backends map onto it directly.

Construction is lazy. Missing reads return empty data, writes create resources and parents as a
consequence, and internal code acts before handling typed absence or conflict. `clear` preserves a
resource, `remove` deletes it, and `open`/`close` delimit the only cache lifetime. Listings are
deterministic, lazy, fused iterators over `Result`, with a bounded traversal frontier.

[`coding`](coding.md) wraps handles without creating another storage trait. gzip, zlib, and zstd
retain only codec state and the current bounded chunk. [`media`](media.md) adds record behavior
through `IOMedia`: Arrow readers stream batches, writes cast once, and commit buffering is explicitly
bounded.

## Traits say what; enums say which

Root traits define behavior and root enums dispatch among implementations. `Codec` selects a
content coding, `MediaType` selects record behavior, `IOKind` describes a resource, and `IOMode`
selects a write operation. `Holder` and `Media` are layer-owned concrete sums that carry one native
implementation across listings and binding boundaries.

Adding a backend or format implements the existing trait and extends its one dispatcher. It does
not add a parallel trait, local enum, or binding implementation.

## Arrow speaks batches; structured text speaks values

[`arrow`](arrow.md) carries columnar arrays and batches. IPC, Parquet, plain-text records, and
Iceberg expose bounded `BatchReader` streams rather than collected batches. [`text`](text.md)
parses and renders the generic `Scalar` through JSON, YAML, and TOML.

Field-directed conversion is the bridge: the exact field controls nullability, nested order,
dictionaries, extension identity, and canonical scalar representation. Bindings use the same native
boundary rather than constructing schemas or values recursively themselves.

## One expression, three tiers

[`Expression`](expression.md) parses once and binds to a field once. The resolved tree evaluates a
row `Scalar`, an Arrow batch, or container statistics. Arrow kernels accelerate supported nodes;
fallback row evaluation cannot change their meaning. Statistics return `false` only when they prove
that no row can match, so uncertainty costs a read instead of losing data.

## One shape per hierarchy level

Collections use `get`, `create`, `open_or_create`, `contains`, lazy iteration, `len`, and
`is_empty`; resources expose identity, properties, and child collections. Dotted names descend
through collections. Iceberg catalogs, namespaces, and tables all use this shape, and storage
folders use the same lazy, typed absence/conflict rules.

## Bindings are views

Python and JavaScript infer or coerce once at their boundary and redirect to Rust. Parsing,
validation, comparison, hashing, storage routing, and recursive conversion remain native. Python
public modules use the layer names; JavaScript keeps one package entry point while its source,
tests, and benchmarks use the same groups.

## Feature boundaries

`arrow` is enabled by default. `parquet` and `iceberg` remain optional because they add their format
stacks. A schema, identifier, hashing, FIX, and structured-text consumer still builds with
`default-features = false` on Rust 1.85.
