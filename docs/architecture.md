# Architecture

Shared vocabulary and every type live in root files; every implementation is a root file or folder of its own name, and a parent folder holds only what its implementations share. The docs tabs group those files by the vocabulary they answer to. Rust is the source of truth, and Python and JavaScript are native views of the same contracts.

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

## Root files, root folders and tabs

Each `rust/src/<name>.rs` owns one shared trait, enum, value or type (`iobase.rs` owns `IOBase`, `codec.rs` owns `Codec`, `media_type.rs` owns `MediaType`, `ccy.rs` owns the `ccy` datatype, its field marker and its value). Each implementation - a medium, a codec, a storage backend, a digest, a charset with string leaves - is a root folder or file of its own name (`parquet/`, `gzip.rs`, `zip/`, `xxhash/`, `utf8.rs`), and a parent folder (`media/`, `text/`, `coding/`, `holder/`, `hashing/`, `charset/`) holds only what its implementations share. The site's top bar groups those files by the vocabulary they answer to, with one join: every encoding, coding and charset - `ipc/`, `parquet/`, `avro/`, `csv/`, `text/`, `json/`, `toml/`, `yaml/`, `xml/`, `xmla/`, `excel/`, `iceberg/`, the codecs and the charsets - documents as one section of the single [Media](media/index.md) page, because a reader picks all three by the name a handle carries, not by which crate module answers it. Storage is joined the same way: `iobase.rs`, the `io*.rs` roles, `holder/` and every backend folder document as sections of the single [Holder](holder/index.md) page, because every backend answers the one `IOBase` contract.

| Tab | Root files and folders |
| --- | --- |
| [Types](types/index.md) | `datatype.rs`, `field.rs`, `scalar.rs`, `cast.rs`, `typed.rs`, `protocol.rs`, `metadata.rs` and one file per type - `string.rs`, `bytes.rs`, `integer.rs`, `floating.rs`, `decimal.rs` with `int256.rs`, `boolean.rs`, `date.rs`, `time.rs`, `datetime.rs`, `duration.rs`, `interval.rs` with `temporal.rs`, `timezone.rs`, `uuid.rs`, `geospatial.rs` with `wkb.rs`, `enums.rs` with the three enums `state.rs`, `side.rs` and `marketdatakind.rs`, `structure.rs`, `sequence.rs`, `mapping.rs`, `union.rs`, `runend.rs`, `version.rs`, `code.rs` with the thirteen registered codes, `forex.rs` among them, `uri/datatype.rs`, `mime_type/datatype.rs`, `media_type/datatype.rs`: `DataType`, `Field`, `Scalar`, the datatype families, protocol views, validation, and casting |
| [Holder](holder/index.md) | `iobase.rs` + `iobase/`, the `io*.rs` roles, `holder/` (`Holder`, `Buffer`, buffering, counting), one folder per backend - `local/`, `fs/`, `zip/`, `s3/`: every `IOBase` implementation - `aws/`, who this process is to AWS: the credential chain, the shared files, the STS and IAM Identity Center exchanges and Signature Version 4 every S3 request signs with; `auth/`, what every identity provider shares - a secret that never renders, an expiring value leased and refreshed in time, an environment or a stand-in, a walk's report; and `xml/scanner.rs`, the scanner the small documents those APIs answer are read by |
| [Media: compression](media/index.md#compression) | `codec.rs`, `coding/` (transparent coded handles), `gzip.rs`, `zlib.rs` (zlib and raw deflate), `zstd.rs` |
| [Media: charsets](media/index.md#charsets) | `charset.rs` + `charset/` (UTF-16, the ISO 8859 and Windows code pages, transparent transcoded handles) and the three charsets with string leaves: `utf8.rs`, `ascii.rs`, `cp1252.rs` |
| [Media](media/index.md) | `media_type.rs`, `mime_type.rs`, `media/` (record options, inference, magic, merge, partition) and one folder per medium - `ipc/`, `parquet/`, `avro/`, `csv/` (delimited text: CSV and TSV), `iceberg/`, `text/` (plain-text records), `xmla/` (XML for Analysis rowsets, and the provider serving them), `excel/` (Office Open XML workbooks: one worksheet as records, and the workbook, its sheets and cells for random access) |
| [Media: JSON, YAML, TOML, XML](media/index.md#json) | `json/`, `toml/`, `yaml/`, `xml/`: structured `Scalar` codecs over the machinery in `text/`, four more sections of the [Media](media/index.md) page |
| [URI](uri/index.md) | `uri/`, `scheme.rs`: URI, URL, URN, ARN, path, glob, and partition syntax |
| [Arrow](arrow/index.md) | `arrow/`: Arrow schema, scalar, array, batch, and reader boundaries |
| [Expression](expression/index.md) | `expression/`: parsing, binding, row evaluation, Arrow evaluation, and pushdown |
| [Graph](graph/index.md) | `graph/element.rs`: the `Element` and `Event` traits - an element's `Uuid`, the identity it has elsewhere, its sources' UUIDs, an event's instant, state and place among the events of its instant - as signatures a value implements; `graph/market.rs` the `Market` and `Operation` traits - the instrument, the side, the price and the quantity, the stated bid and ask, the FX rates, then the operation's time in force, whether it trades and its identifiers and party ids - with the readings of an `Event` that is one of them provided on the traits themselves; `graph/operation.rs` the operation leaves - `Order`, `Quote`, `Execution` and their dated `OrderEvent`, `QuoteEvent`, `ExecutionEvent`, one generic pair over a sealed `OperationKind` - and their book control, `graph/trade.rs` the `TradeEvent`, `graph/book.rs` the `BookEvent`, `SnapshotEvent` and the book fold, `graph/market_data.rs` the one `MarketData` enum over every leaf and `graph/kind.rs` its `MarketKind`, `graph/arrow.rs` the lifted Arrow row, `graph/view.rs` the six `MarketView` readings of it, each one `Plan`, `graph/candle.rs` the `Candle` a bucket of books folds into and the `CandleIterator` that folds them, `graph/serve.rs` the `BookService` that answers a market-data table as candles, books and audits over HTTP (the `http` feature; `yggdryl market serve` is its terminal), and `graph/iterator.rs` the one walk; the root `limit.rs` holds `Limit`, one price level of a book side, which a book states under `bidlimits` and `asklimits`, and the root `identifier.rs` holds `Identifier` and `Identifiers`, the sets a market element names its security (`securityids`), itself (`identifiers`) and its parties (`partyids`) by - each identifier a source, a type and a value, unique by `src:type`, its words the `IdSource` and `IdType` of the root `idsource.rs` and `idtype.rs`. |
| [Hashing](hashing.md) | `digest.rs` (`Digest`, `DigestAlgorithm`, `Digester`), `hashing/` (the private stable-hash adapters), `xxhash/`: digest values, one-shot and resumable hashes, streams, handles, and row hashes; `txhash/`: an instant coupled with a digest - the sortable value, its instant intake, coupled columns, and the `DIGEST:time` holder |
| [FIX](fix/index.md) | `fix/`: FIX vocabulary over core `Field` values and `IOBase` registry storage |

Documentation is grouped by these tab names - `docs/<tab>/` for a tab of several pages, `docs/<tab>.md` for a single-page tab such as Hashing - so one name finds a concept's contract, validation, boundary, and page, whichever root files answer it. Source and tests are not: the Python package and both binding crates repeat the crate's own layout, one file per type at the root and one per implementation beside it, and every test file sits at the path of the source file it pins.

## Rules the layers share

| Rule | Consequence |
| --- | --- |
| A schema is a field | A non-null Struct [`Field`](types/field.md) is the only row schema; `Scalar::Struct` is named input that canonicalizes to an ordered `Scalar::Serie`. |
| Protocol metadata is a view | `field.as_fix()` and `field.as_iceberg()` borrow the same field, `field.as_digest()` names row-digest roles, `field.as_identity()` and `field.as_partition()` give generic metadata; none copies state. |
| Storage is one trait | [`IOBase`](holder/index.md) is positional (`pread`, `pwrite`); construction touches nothing, absent reads are empty, writes create. |
| Listings are iterators | `ls`, `glob`, and predicate listings yield `Result` items lazily and fuse at the first failure. |
| Traits say what, enums say which | `Codec`, `MediaType`, `IOKind`, `IOMode` dispatch; `Holder` and `Media` carry one native implementation across bindings. |
| Arrow speaks batches | IPC, Parquet, text records, and Iceberg expose bounded `BatchReader` streams, never collected batches. |
| Text speaks values | JSON, YAML, TOML, and XML parse and render one [`Scalar`](media/index.md#json); the exact field directs nullability, order, and dictionaries. |
| A scheme owns a direction, not a surface | Every [Media](media/index.md) scheme answers the same two surfaces, so it documents them the same way: an overview, a read page, a write page, and one page per feature it alone has. Reading and writing each show native scalars first, then Arrow batches, in all three languages. |
| A family owns a subsection | Every [Types](types/index.md) family is a folder: `index.md` for what its leaves share, and one page per type, each presenting its datatype, its field, its scalar, its Arrow storage, then its features. |
| One expression, three tiers | [`Expression`](expression/index.md) parses once, binds once, then evaluates a row, a batch, or container statistics; statistics answer `false` only when no row can match. |
| One shape per hierarchy level | Collections use `get`, `create`, `open_or_create`, `contains`, lazy iteration, `len`, `is_empty`; dotted names descend. |
| Bindings are views | Python and JavaScript coerce once at the boundary and call the core; parsing, validation, hashing, and conversion stay native. |

## Watching what the core does

The native core narrates its work through Rust's `log` facade, so a Rust caller
installs any `log` implementation. Python bridges it into `logging` under the
package's own logger: a record's name is the Rust module path it came from, so
`yggdryl.iceberg.table` and its siblings all hang off `yggdryl` and one
`setLevel` is the whole switch. The Node addon writes warnings to standard
error as `yggdryl: <message>` when it loads, unless the process already
installed a logger, and the `yggdryl` command writes them to standard error
prefixed `!`. What a data door passes over is said once per kind and then
counted ([Warnings](fix/capture.md#warnings)).

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
    from yggdryl.iceberg import Table, assign_field_ids

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

No feature is on by default (`default = []`); Arrow arrays, batches, IPC and casting are always compiled in.

| Feature | Adds |
| --- | --- |
| `parquet` | the Parquet codec and its compression stack |
| `iceberg` | Iceberg tables, their metadata serde and validation owned by the official Iceberg 0.10.1 crate; implies `parquet`, needs Rust 1.94 or newer |
| `http` | the HTTP/1.1 client, sessions, requests, responses, resumable streams, paginated pages and the `Server`, behind `IOBase` ([HTTP](holder/index.md#http)) |
| `http2` | HTTP/2 under the same client - ALPN `h2` over TLS, `h2c` by prior knowledge; implies `http` |
| `http3` | HTTP/3 under the same client, once an origin advertises it in `Alt-Svc`; implies `http2` |
| `aws` | the AWS identity: credential chain, shared configuration, SSO, STS, Signature Version 4; implies `http` |
| `s3` | the Amazon S3, Google Cloud Storage and Azure Blob Storage backend; implies `aws` |
| `internals` | `yggdryl::internals`, reached by `rust/tests/` alone; no published build turns it on |

Every build - the default one, `s3`, `iceberg` and both bindings - compiles on Rust 1.94.

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
