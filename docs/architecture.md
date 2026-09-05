# Architecture

Shared vocabulary lives in root files; implementations live in ten layer folders. Rust is the source of truth, and Python and JavaScript are native views of the same contracts.

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

Each `rust/src/<name>.rs` owns one shared trait, enum, or value (`iobase.rs` owns `IOBase`, `codec.rs` owns `Codec`, `media_type.rs` owns `MediaType`). Each folder owns one implementation family, and the site's top bar is that folder list.

| Layer | Owns |
| --- | --- |
| [`types`](types/index.md) | `DataType`, `Field`, `Scalar`, the datatype families, protocol views, validation, and casting |
| [`holder`](holder/index.md) | `Buffer`, local and generic filesystem handles, buffering, and every `IOBase` implementation |
| [`coding`](coding/index.md) | gzip, zlib/deflate, zstd, and transparent coded handles |
| [`media`](media/index.md) | record options, IPC, Parquet, Avro, plain-text records, and Iceberg |
| [`text`](text/index.md) | structured `Scalar` codecs for JSON, YAML, and TOML |
| [`uri`](uri/index.md) | URI, URL, URN, path, glob, and partition syntax |
| [`arrow`](arrow/index.md) | Arrow schema, scalar, array, batch, and reader boundaries |
| [`expression`](expression/index.md) | parsing, binding, row evaluation, Arrow evaluation, and pushdown |
| [`xxhash`](xxhash/index.md) | digest values, one-shot and resumable hashes, streams, handles, and row hashes |
| [`fix`](fix/index.md) | FIX vocabulary over core `Field` values and `IOBase` registry storage |

Tests, benchmarks, Python modules, JavaScript source groups, and documentation mirror these names, so one path finds a concept's implementation, validation, boundary, and contract.

## Rules the layers share

| Rule | Consequence |
| --- | --- |
| A schema is a field | A non-null Struct [`Field`](types/field.md) is the only row schema; `Scalar::Record` is named input that canonicalizes to an ordered `Scalar::Sequence`. |
| Protocol metadata is a view | `field.as_fix()` and `field.as_iceberg()` borrow the same field, `field.as_digest()` names row-digest roles, `field.as_identity()` and `field.as_partition()` give generic metadata; none copies state. |
| Storage is one trait | [`IOBase`](holder/index.md) is positional (`pread`, `pwrite`); construction touches nothing, absent reads are empty, writes create. |
| Listings are iterators | `ls`, `glob`, and predicate listings yield `Result` items lazily and fuse at the first failure. |
| Traits say what, enums say which | `Codec`, `MediaType`, `IOKind`, `IOMode` dispatch; `Holder` and `Media` carry one native implementation across bindings. |
| Arrow speaks batches | IPC, Parquet, text records, and Iceberg expose bounded `BatchReader` streams, never collected batches. |
| Text speaks values | JSON, YAML, and TOML parse and render one [`Scalar`](text/index.md); the exact field directs nullability, order, and dictionaries. |
| One expression, three tiers | [`Expression`](expression/index.md) parses once, binds once, then evaluates a row, a batch, or container statistics; statistics answer `false` only when no row can match. |
| One shape per hierarchy level | Collections use `get`, `create`, `open_or_create`, `contains`, lazy iteration, `len`, `is_empty`; dotted names descend. |
| Bindings are views | Python and JavaScript coerce once at the boundary and call the core; parsing, validation, hashing, and conversion stay native. |

## Feature boundaries

| Feature | Default | Adds |
| --- | ---: | --- |
| `arrow` | on | arrays, batches, IPC, casting |
| `parquet` | off | the Parquet codec and its compression stack |
| `iceberg` | off | Iceberg 0.10.1 metadata (Rust 1.94 or newer) |

A schema, identifier, hashing, FIX, and structured-text consumer builds with `default-features = false` on Rust 1.85.

## Page skeleton

Every family page follows one order, so a reader who learns one page can navigate all of them.

| Section | Holds |
| --- | --- |
| Contract | One compact table: what is owned, validated, lazy, cached, and refused |
| Use | The smallest runnable example, in Rust, Python, and JavaScript tabs |
| Feature sections | One per behaviour, each at most two sentences before its code |
| Edges | Refusals, nulls, empties, overflows, and limits, one line each |
| Commands | The test and benchmark commands scoped to the page |
| Performance | The measured table, its host and toolchain, and the regenerate command |
