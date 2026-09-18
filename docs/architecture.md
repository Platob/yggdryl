# Architecture

Shared vocabulary lives in root files; implementations live in eleven layer folders. Rust is the source of truth, and Python and JavaScript are native views of the same contracts.

```text
root traits, enums, and values
        │
        ├── types ─────────► arrow
        │      │                │
        │      └── expression ──┤
        │                       ▼
        ├── holder ◄── coding ◄── media
        │      ▲
        │   charset ── decoded bytes for media and text
        │                         │
        ├── uri ──────────────────┘
        ├── text
        ├── hashing: xxhash ──► txhash
        └── graph: element ──► event ──► iterator

fix ── protocol vocabulary over types + holder
```

## Root files and layer folders

Each `rust/src/<name>.rs` owns one shared trait, enum, or value (`iobase.rs` owns `IOBase`, `codec.rs` owns `Codec`, `media_type.rs` owns `MediaType`). Each folder owns one implementation family, and the site's top bar is that folder list - with one join: `text/` documents under [Media](media/index.md), because a reader picks a scheme by the name a handle carries, not by which crate module answers it.

| Layer | Owns |
| --- | --- |
| [`types`](types/index.md) | `DataType`, `Field`, `Scalar`, the datatype families, protocol views, validation, and casting |
| [`holder`](holder/index.md) | `Buffer`, local and generic filesystem handles, buffering, and every `IOBase` implementation |
| [`coding`](coding/index.md) | gzip, zlib/deflate, zstd, and transparent coded handles |
| [`charset`](charset/index.md) | UTF-8, UTF-16, US-ASCII, the ISO 8859 and Windows code pages, and transparent transcoded handles |
| [`media`](media/index.md) | record options, IPC, Parquet, Avro, plain-text records, and Iceberg |
| [`text`](media/structured.md) | structured `Scalar` codecs for JSON, YAML, and TOML, under the [Media](media/index.md) tab as three more schemes |
| [`uri`](uri/index.md) | URI, URL, URN, path, glob, and partition syntax |
| [`arrow`](arrow/index.md) | Arrow schema, scalar, array, batch, and reader boundaries |
| [`expression`](expression/index.md) | parsing, binding, row evaluation, Arrow evaluation, and pushdown |
| [`graph`](graph.md) | `graph/element.rs`: the `Element`, `Event`, `MarketElement` and `MarketEvent` traits - an element's `Uuid`, the identity it has elsewhere, its parents' UUIDs, an event's instant and code, a market element's price, quantity and side - as signatures a value implements; `graph/event.rs` the two holders and `graph/iterator.rs` the one walk |
| [`hashing`](hashing.md) | `hashing/xxhash/`: digest values, one-shot and resumable hashes, streams, handles, and row hashes; `hashing/txhash/`: an instant coupled with a digest - the sortable value, its instant intake, coupled columns, and the `digest:time` holder |
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
| Text speaks values | JSON, YAML, and TOML parse and render one [`Scalar`](media/structured.md); the exact field directs nullability, order, and dictionaries. |
| A scheme owns three pages | Every [Media](media/index.md) scheme answers the same two surfaces, so it documents them the same way: what it is, rows as native scalars, rows as Arrow batches. |
| One expression, three tiers | [`Expression`](expression/index.md) parses once, binds once, then evaluates a row, a batch, or container statistics; statistics answer `false` only when no row can match. |
| One shape per hierarchy level | Collections use `get`, `create`, `open_or_create`, `contains`, lazy iteration, `len`, `is_empty`; dotted names descend. |
| Bindings are views | Python and JavaScript coerce once at the boundary and call the core; parsing, validation, hashing, and conversion stay native. |

## Watching what the core does

The native core narrates its work through Rust's `log` facade, so a Rust caller
installs any `log` implementation. Python bridges it into `logging` under the
package's own logger: a record's name is the Rust module path it came from, so
`yggdryl.media.iceberg.table` and its siblings all hang off `yggdryl` and one
`setLevel` is the whole switch. JavaScript has no bridge.

Debug is an operation starting; info is one done, carrying the counts a monitor
watches. Nothing is reported per row, per batch, or per file: a commit is the
unit, so ten times the rows is the same handful of records. A dependency of the
build reaches `logging` only at warning and above, so enabling debug narrates
this project and nothing else.

=== "Python"

    ```python
    import logging
    import tempfile
    from pathlib import Path

    import pyarrow as pa

    from yggdryl import IOBase, refresh_logging
    from yggdryl.media.iceberg import Table, assign_field_ids

    said: list[str] = []


    class Collect(logging.Handler):
        def emit(self, record: logging.LogRecord) -> None:
            said.append(record.getMessage())


    watcher = logging.getLogger("yggdryl")
    watcher.addHandler(Collect())
    watcher.setLevel(logging.INFO)
    # The bridge caches each logger's effective level, so a level set after
    # import reaches it only through this call.
    refresh_logging()

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    with tempfile.TemporaryDirectory() as folder:
        table = Table.create(IOBase(Path(folder) / "trades"), assign_field_ids(schema))
        table.append(pa.record_batch({"id": [1, 2, 3]}, schema=schema))
        assert sum(batch.num_rows for batch in table.scan()) == 3

    assert any(message.startswith("created iceberg table at") for message in said)
    assert any("wrote 3 rows as" in message for message in said)
    ```

| Reported | Level | Carries |
| --- | --- | --- |
| a table created | info | location, format version, column count |
| a table opened | debug | location, metadata version |
| a scan planned | debug then info | manifests walked; files to open, files the filters excluded, manifests read and skipped |
| a snapshot written | debug then info | operation and snapshot id; rows, data files, bytes |
| a commit landed | info | metadata version and snapshot id |
| a commit beaten | debug | the version that won, the retry count, the wait |
| a compaction | debug then info | files and bytes rewritten, files produced |
| a schema evolved | info | the new schema id |
| snapshots expired | info | how many |
| an Arrow write session | debug then info | cadences published |

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
