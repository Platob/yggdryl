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
        ├── warehouse ── catalogs, namespaces and tables over holder and media
        ├── text
        ├── hashing: xxhash ──► txhash
        └── graph: element ──► event

yggdryl-market ── market kinds, identifiers, instruments,
        │         graph: market ──► operation ──► book ──► iterator
        ▼
yggdryl-fix ── dictionary, codec, messages, lifecycle over types + holder
```

## Crates

The core is the `yggdryl` crate in `rust/`. Two crates stand over it, each laid out as the core is - a flat `src/`, a `tests/` mirroring it, `benchmarks/`: `yggdryl-market` in `rust/market/`, the four market enum kinds (`MarketDataKind`, `MarketDataType`, `Side`, `TimeInForce`), the identifiers an element states, the `Instrument` and its `Instruments`, `Limit`, `Eusipa` and the market half of the graph; and `yggdryl-fix` in `rust/fix/`, the FIX dictionary and its store, the codec, the message handle and the lifecycle, over both. The core depends on neither: a crate claims what it brings on the core's registers, and reaches the core's crate-private items through the hidden `yggdryl::implementer`, as `yggdryl-fix` reaches the market crate's through `yggdryl_market::implementer`. The Python and Node bindings and the `yggdryl` command link all three.

A pure-Rust caller installs what it links before the core reads a name a crate brings, in dependency order: `yggdryl_market::install()?` claims the four kinds - a reserved kind is claimed only at its reserved byte, name, extension name and ranks, by the crate that owns it, so every hash, order and wire form over them holds - and the `side` and `timeinforce` listings; `yggdryl_fix::install()?` installs the market crate and then claims the FIX Latest logical names. Until then a reserved kind's name is refused naming that install - ``unknown datatype "side": no registered datatype answers it; `side` is read only once the crate that claims it is installed (`yggdryl_market::install()`)`` - and any other word no claim answers is refused with nothing to install. Nothing installs lazily; the Python and Node packages and the `yggdryl` command install both when they load.

A test sits in the crate whose `src/` holds the file it pins, at the mirrored path, unless it names something only a crate above links: what a core file answers for a market kind is pinned in `rust/market/tests/` at that file's path, and what only a build of every crate can state in `cli/tests/`. Every test of a crate installs it first, through its `tests/support/install.rs`.

## Root files, root folders and tabs

Each `rust/src/<name>.rs` owns one shared trait, enum, value or type (`iobase.rs` owns `IOBase`, `codec.rs` owns `Codec`, `media_type.rs` owns `MediaType`, `ccy.rs` owns the `ccy` datatype, its field marker and its value). Each implementation - a medium, a codec, a storage backend, a digest, a charset with string leaves - is a root folder or file of its own name (`parquet/`, `gzip.rs`, `zip/`, `xxhash/`, `utf8.rs`), and a parent folder (`media/`, `text/`, `coding/`, `holder/`, `hashing/`, `charset/`) holds only what its implementations share. The site's top bar groups those files by the vocabulary they answer to, with one join: every encoding, coding and charset - `ipc/`, `parquet/`, `avro/`, `csv/`, `text/`, `json/`, `toml/`, `yaml/`, `xml/`, `xmla/`, `excel/`, `iceberg/`, the codecs and the charsets - documents under the one [Media](media/index.md) tab, because a reader picks all three by the name a handle carries, not by which crate module answers it: the tab's overview holds the calls every medium shares, and each medium, the content codings and the charsets have a page of their own. Storage is joined on one page instead: `iobase.rs`, the `io*.rs` roles, `holder/` and every backend folder document as sections of the single [Holder](holder/index.md) page, because every backend answers the one `IOBase` contract.

| Tab | Root files and folders |
| --- | --- |
| [Types](types/index.md) | `datatype.rs`, `field.rs`, `scalar.rs`, `cast.rs`, `typed.rs`, `protocol.rs`, `metadata.rs` and one file per type - `string.rs`, `bytes.rs`, `integer.rs`, `floating.rs`, `decimal.rs` with `int256.rs`, `boolean.rs`, `date.rs`, `time.rs`, `datetime.rs`, `duration.rs`, `interval.rs` with `temporal.rs`, `timezone.rs`, `uuid.rs`, `geospatial.rs` with `wkb.rs`, `enums.rs` with `state.rs` - the market kinds are [`yggdryl-market`](#crates)'s - `structure.rs`, `sequence.rs`, `mapping.rs`, `union.rs`, `runend.rs`, `version.rs`, `code.rs` with the thirteen registered codes, `forex.rs` among them, `uri/datatype.rs`, `mime_type/datatype.rs`, `media_type/datatype.rs`: `DataType`, `Field`, `Scalar`, the datatype families, protocol views, validation, and casting |
| [Holder](holder/index.md) | `iobase.rs` + `iobase/`, the `io*.rs` roles, `holder/` (`Holder`, `Buffer`, buffering, counting, the `Catalog`, `Namespace` and `Table` variants holding a [warehouse](warehouse/index.md) object as the handle it is, and the register of storage backends whose handles `Holder::Registered` holds), one folder per backend - `local/`, `fs/`, `zip/`, `s3/`, the last the backend the core claims: every `IOBase` implementation - `aws/`, who this process is to AWS: the credential chain, the shared files, the STS and IAM Identity Center exchanges and Signature Version 4 every S3 request signs with; `auth/`, what every identity provider shares - a secret that never renders, an expiring value leased and refreshed in time, an environment or a stand-in, a walk's report; and `xml/scanner.rs`, the scanner the small documents those APIs answer are read by |
| [Warehouse](warehouse/index.md) | `warehouse/`: the catalog, namespace and table abstraction - the traits `ObjectValue`, `NamespaceValue`, `CatalogValue`, `TableValue` and the enums `Object`, `Catalog`, `Namespace`, `Table`; `Properties`, the one ordered name/value bag a target's `with (...)` clause, `Holder::from_url` and every object read; the path intake `IntoObjectPath`; the lazy `Namespaces` and `Tables` views; `Warehouse` and the process's `SystemWarehouse` - with its generic implementations `MemoryCatalog`, `MemoryNamespace`, `FolderCatalog`, `FolderNamespace` and `MediaTable` |
| [Media: compression](media/compression.md) | `codec.rs`, `coding/` (transparent coded handles), `gzip.rs`, `zlib.rs` (zlib and raw deflate), `zstd.rs` |
| [Media: charsets](media/charsets.md) | `charset.rs` + `charset/` (UTF-16, the ISO 8859 and Windows code pages, transparent transcoded handles) and the three charsets with string leaves: `utf8.rs`, `ascii.rs`, `cp1252.rs` |
| [Media](media/index.md) | `media_type.rs`, `mime_type.rs` and `media/` (the `Media` value, record options, the registers of media and of table formats, inference, magic, merge, partition) on the overview, and one folder per medium, each with its page - `ipc/`, `parquet/`, `avro/`, `csv/` (delimited text: CSV and TSV), `iceberg/`, `text/` (plain-text records), `xmla/` (XML for Analysis rowsets, and the provider serving them), `excel/` (Office Open XML workbooks: one worksheet as records, and the workbook, its sheets and cells for random access), and `http/wire.rs` for HTTP messages |
| Media: [JSON](media/json.md), [YAML](media/yaml.md), [TOML](media/toml.md), [XML](media/xml.md) | `json/`, `toml/`, `yaml/`, `xml/`: structured `Scalar` codecs over the machinery in `text/`, a page each under [Media](media/index.md) |
| [URI](uri/index.md) | `uri/`, `scheme.rs`: URI, URL, URN, ARN, path, glob, and partition syntax |
| [Arrow](arrow/index.md) | `arrow/`: Arrow schema, scalar, array, batch, and reader boundaries |
| [Expression](expression/index.md) | `expression/`: parsing, binding, row evaluation, Arrow evaluation, and pushdown; a target's `with (...)` clause is the warehouse's one `Properties` bag |
| [Graph](graph/index.md) | `graph/element.rs`: the `Element` and `Event` traits - an element's `Uuid`, the identity it has elsewhere, its sources' UUIDs, an event's instant, state and place among the events of its instant - as signatures a value implements, and `graph/element_column.rs` and `graph/column.rs` the columns they state; in `yggdryl-market`, `graph/market.rs` the `Market` and `Operation` traits - the instrument, the side, the price and the quantity, the stated bid and ask, the FX rates, then the operation's time in force, whether it trades and its identifiers and party ids - with the readings of an `Event` that is one of them provided on the traits themselves; `graph/operation.rs` the operation leaves - `Order`, `Quote`, `Execution` and their dated `OrderEvent`, `QuoteEvent`, `ExecutionEvent`, one generic pair over a sealed `OperationKind` - and their book control, `graph/trade.rs` the `TradeEvent`, `graph/book.rs` the `BookEvent`, `SnapshotEvent` and the book fold, `graph/market_data.rs` the one `MarketData` enum over every leaf and `graph/kind.rs` its `MarketKind`, `graph/arrow.rs` the lifted Arrow row, `graph/view.rs` the six `MarketView` readings of it, each one `Plan`, `graph/candle.rs` the `Candle` a bucket of books folds into and the `CandleIterator` that folds them, and `graph/iterator.rs` the one walk; its root `limit.rs` holds `Limit`, one price level of a book side, which a book states under `bidlimits` and `asklimits`, and its root `identifier.rs` holds `Identifier` and `Identifiers`, the sets a market element names its security (`securityids`), itself (`identifiers`) and its parties (`partyids`) by - each identifier a source, a type and a value, unique by its `IdKey` (`src:type`, the root `idkey.rs`), its words the `IdSource` and `IdType` of its root `idsource.rs` and `idtype.rs`; its root `instrument.rs` holds `Instrument`, one element per instrument keyed by its cross code - a real ISIN, or a CFI class and the body its characteristics spell - its listings nested, and `Instruments`, the collection a lifecycle learns into and fills every market element's `instcode` from, with `characteristics.rs` and `listing.rs` beside it. |
| [Hashing](hashing.md) | `digest.rs` (`Digest`, `DigestAlgorithm`, `Digester`), `hashing/` (the private stable-hash adapters), `xxhash/`: digest values, one-shot and resumable hashes, streams, handles, and row hashes; `txhash/`: an instant coupled with a digest - the sortable value, its instant intake, coupled columns, and the `DIGEST:time` holder |
| [Logging](logging.md) | `logging/`: Python's `logging` owned by the core, behind the `log` facade - `Logger` and `get_logger`, `Level`, `Record`, `Formatter`, the `Handler` trait with `StreamHandler`, `NullHandler` and `FileHandler` over any `IOBase`, the `Host` a binding attaches its runtime's own logging with, `install`, `basic_config` and `shutdown`; `logging/warning.rs`, the deduplicated warnings the data doors raise |
| [FIX](fix/index.md) | `yggdryl-fix`, `rust/fix/src/`: FIX vocabulary over core `Field` values and `IOBase` registry storage; the FIX protocol view (`FixField`, `FixFieldMut`, `field.rs`), the FIX readings of a `State` (`state.rs`) and the FIX Latest logical names its `install()` claims |

Documentation is grouped by these tab names - `docs/<tab>/` for a tab of several pages, `docs/<tab>.md` for a single-page tab such as Hashing or Logging - so one name finds a concept's contract, validation, boundary, and page, whichever root files answer it. Source and tests are not: the Python package and both binding crates repeat the crate's own layout, one file per type at the root and one per implementation beside it, and every test file sits at the path of the source file it pins.

## Rules the layers share

| Rule | Consequence |
| --- | --- |
| A schema is a field | A non-null Struct [`Field`](types/field.md) is the only row schema; `Scalar::Struct` is named input that canonicalizes to an ordered `Scalar::Serie`. |
| Protocol metadata is a view | `field.as_iceberg()` and `FixField::new(&field)` borrow the same field, `field.as_digest()` names row-digest roles, `field.as_identity()` and `field.as_partition()` give generic metadata; none copies state. A protocol the core does not own mints its view with the exported `protocol_field_types!` builder, so `FixField` lives in `yggdryl-fix`'s `field.rs` and `Field` carries no accessor for it. |
| Storage is one trait | [`IOBase`](holder/index.md) is positional (`pread`, `pwrite`); construction touches nothing, absent reads are empty, writes create. |
| Listings are iterators | `ls`, `glob`, and predicate listings yield `Result` items lazily and fuse at the first failure. |
| Traits say what, enums say which | `Codec`, `MediaType`, `IOKind`, `IOMode` dispatch; `Holder` and `Media` carry one native implementation across bindings. |
| The core's private doors are one module | `yggdryl::implementer` is the one public door the workspace's own crates reach the core's crate-private items through: a `pub use`, an `#[inline]` forwarder, a moved definition or an exported macro per item, each documented, and no proven landing among them. It is no API for a caller outside the workspace and is hidden from the rendered docs, and its list is regenerated by the slice that moves a crate; `yggdryl_market::implementer` is the same door one crate up, for `yggdryl-fix`. |
| An extension point is a register | A market kind, a medium, a table format, a catalog factory, a location scheme and a storage backend are each a `static` claimed once with `claim(.., "crate")` - the four market kinds by `yggdryl_market::install()`; an intake door reads the claim once and a value in hand carries it ([Registering a medium](media/index.md#registering-a-medium), [Storage backends](holder/index.md#storage-backends)). |
| Arrow speaks batches | IPC, Parquet, text records, and Iceberg expose bounded `BatchReader` streams, never collected batches. |
| Text speaks values | JSON, YAML, TOML, and XML parse and render one [`Scalar`](media/json.md); the exact field directs nullability, order, and dictionaries. |
| A scheme owns a direction, not a surface | Every [Media](media/index.md) scheme answers the same two surfaces, so its page documents them the same way: an Overview, a Read section, a Write section, then one section per feature it alone has. Read and Write each lead with a runnable example in all three languages. |
| A family owns a subsection | Every [Types](types/index.md) family is a folder: `index.md` for what its leaves share, and one page per type, each presenting its datatype, its field, its scalar, its Arrow storage, then its features. |
| One expression, three tiers | [`Expression`](expression/index.md) parses once, binds once, then evaluates a row, a batch, or container statistics; statistics answer `false` only when no row can match. |
| One shape per hierarchy level | Collections use `get`, `create`, `open_or_create`, `contains`, lazy iteration, `len`, `is_empty`; dotted names descend. |
| Bindings are views | Python and JavaScript coerce once at the boundary and call the core; parsing, validation, hashing, and conversion stay native. Python reaches every stable domain, JavaScript its essential doors, a new one only when it is asked for by name. |

## Watching what the core does

The native core owns a Python-like logging tree, `yggdryl::logging`, behind
Rust's `log` facade: a record's logger is named after the Rust module path it
came from, whatever crate of the workspace holds the module, so `yggdryl.iceberg.table` and its siblings all hang off `yggdryl`
and one level on `yggdryl` is the whole switch. A Rust caller installs the tree
(`install`, or `basic_config` for a handler on standard error) or any other
`log` backend. Wherever no format is stated - `basic_config`, a handler with
no formatter, the last resort - a record is the
[terminal line](logging.md#terminal): the time, the level's glyph and name,
`[thread]`, the logger, the call site, ` › `, the message, coloured only where
the [colour rule](logging.md#colour) says - a colour terminal, `NO_COLOR`,
`FORCE_COLOR`, `CLICOLOR_FORCE` and `TERM=dumb` honoured. Python hosts the
tree in `logging`: each Rust logger is the Python logger of the same name, and
a level changed at any time applies to the next record
([Python: hosted by logging](logging.md#python-hosted-by-logging)). The Node
addon installs the tree when it loads, and its last resort writes warnings to
standard error as that line, from `[main]` or `[worker-N]`
([JavaScript](logging.md#javascript)). The `yggdryl` command keeps the core's
warnings and prints them on standard output once the command's progress line is
done: one `!` line counting them, then each as a `·` note. What a data door
passes over is said once per kind and then counted
([Warnings](fix/capture.md#warnings)). Loggers, levels, handlers and the
formatter are one contract, on [Logging](logging.md).

Debug is an operation starting; info is one done, carrying the counts a monitor
watches. Nothing is reported per row, per batch, or per file: a commit is the
unit, so ten times the rows is the same handful of records. A dependency of the
build reaches the tree only at warning and above, in Python and in JavaScript
([The log facade](logging.md#the-log-facade)), so enabling debug narrates this
project and nothing else.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::holder::Buffer;
    use yggdryl::iceberg::{FormatVersion, IcebergTable, PartitionSpec, assign_field_ids};
    use yggdryl::local::LocalFolder;
    use yggdryl::logging::{self, FileHandler, Formatter, Handler, Level};
    use yggdryl::{arrow, DataType, IOBase, StructType};

    // The tree is the `log` facade's backend. A handler on `yggdryl.iceberg`
    // holds the narration of every table, one line per record.
    logging::install()?;
    let held = Arc::new(FileHandler::new(Buffer::new()));
    held.set_formatter(Formatter::from_str("%(levelname)s %(name)s %(message)s")?);
    let narration: Arc<dyn Handler> = held.clone();
    let watcher = logging::get_logger("yggdryl.iceberg");
    watcher.set_level(Level::INFO);
    watcher.set_propagating(false);
    watcher.add_handler(narration.clone());

    let mut schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
        .required_field("row");
    assign_field_ids(&mut schema, 1)?;
    let path = LocalFolder::temporary()?.path()?.join("yggdryl-docs-architecture-logging");
    let _ = std::fs::remove_dir_all(&path);

    let mut table = IcebergTable::create(LocalFolder::new(&path)?, FormatVersion::V2, schema.clone(), PartitionSpec::unpartitioned())?;
    let batch = RecordBatch::try_new(schema.into_arrow_schema()?, vec![Arc::new(Int64Array::from(vec![1_i64, 2, 3]))])?;
    table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;

    // Other tables narrated meanwhile land in the same handler, so the lines
    // are read for this table's folder.
    let said = String::from_utf8(held.io().read_all_bytes()?)?;
    let folder = "yggdryl-docs-architecture-logging";
    assert!(said.lines().any(|line| {
        line.starts_with("INFO yggdryl.iceberg.table created iceberg table at") && line.contains(folder)
    }));
    assert!(said.lines().any(|line| {
        line.starts_with("INFO yggdryl.iceberg.table wrote 3 rows as") && line.contains(folder)
    }));

    watcher.remove_handler(&narration);
    let _ = std::fs::remove_dir_all(&path);
    ```

=== "Python"

    ```python
    import logging
    import tempfile
    from pathlib import Path

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.iceberg import IcebergTable, assign_field_ids

    said: list[str] = []


    class Collect(logging.Handler):
        def emit(self, record: logging.LogRecord) -> None:
            said.append(record.getMessage())


    watcher = logging.getLogger("yggdryl")
    watcher.addHandler(Collect())
    watcher.setLevel(logging.INFO)

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    with tempfile.TemporaryDirectory() as folder:
        table = IcebergTable.create(IOBase(Path(folder) / "trades"), assign_field_ids(schema))
        table.append(pa.record_batch({"id": [1, 2, 3]}, schema=schema))
        assert sum(batch.num_rows for batch in table.scan()) == 3

    assert any(message.startswith("created iceberg table at") for message in said)
    assert any("wrote 3 rows as" in message for message in said)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { Field, fields, iceberg, logging } = require('yggdryl')

    const folder = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const narration = path.join(folder, 'narration.log')

    // Every record hangs off `yggdryl`, so one level there is the whole switch;
    // a file handler holds the narration, one line per record.
    const handler = new logging.FileHandler(narration)
    handler.setFormatter(new logging.Formatter('%(levelname)s %(name)s %(message)s'))
    const watcher = logging.getLogger('yggdryl')
    watcher.addHandler(handler)
    watcher.setLevel(logging.INFO)

    const schema = fields.struct('row', [Field.from('id: int64')], { nullable: false })
    const table = iceberg.IcebergTable.create(path.join(folder, 'trades'), schema, iceberg.PartitionSpec.unpartitioned())
    table.append(new arrow.Table({ id: arrow.vectorFromArray([1n, 2n, 3n], new arrow.Int64()) }))
    assert.equal(table.scan().intoTable().numRows, 3)

    const said = fs.readFileSync(narration, 'utf8')
    assert.match(said, /^INFO yggdryl\.iceberg\.table created iceberg table at/m)
    assert.match(said, /^INFO yggdryl\.iceberg\.table wrote 3 rows as/m)

    watcher.removeHandler(handler)
    handler.close()
    fs.rmSync(folder, { recursive: true, force: true })
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
| `iceberg` | Iceberg tables, their metadata serde and validation owned by the official Iceberg 0.10.1 crate, and the `iceberg` table format and the `hadoop` catalog factory the core claims; implies `parquet`, needs Rust 1.94 or newer |
| `http` | the HTTP/1.1 client, sessions, requests, responses, resumable streams, paginated pages and the `Server`, behind `IOBase` ([HTTP](holder/index.md#http)) |
| `http2` | HTTP/2 under the same client - ALPN `h2` over TLS, `h2c` by prior knowledge; implies `http` |
| `http3` | HTTP/3 under the same client, once an origin advertises it in `Alt-Svc`; implies `http2` |
| `aws` | the AWS identity: credential chain, shared configuration, SSO, STS, Signature Version 4; implies `http` |
| `s3` | the Amazon S3, Google Cloud Storage and Azure Blob Storage backend; implies `aws` |
| `s3tables` | Amazon S3 Tables: a table bucket as a warehouse catalog whose Iceberg tables commit through the control plane ([`S3TablesCatalog`](media/iceberg.md#iceberg-on-amazon-s3-tables), what `Catalog::from_url` answers for an `s3tables://` location in every language), and the client of that control plane ([the table bucket's catalog](media/iceberg.md#the-table-buckets-catalog), Rust only); implies `s3` and `iceberg` |
| `internals` | `yggdryl::internals`, `yggdryl_market::internals` and `yggdryl_fix::internals`, reached by the crates' tests (`rust/tests/`, `rust/market/tests/`, `rust/fix/tests/`) alone; no published build turns it on |

`yggdryl-market` and `yggdryl-fix` take `http`, `parquet`, `iceberg` and `internals` too, each forwarding to the crates below it. Every build - the default one, `s3`, `iceberg` and both bindings - compiles on Rust 1.94.

## Page skeleton

Every family page follows one order, so a reader who learns one page can navigate all of them.

| Section | Holds |
| --- | --- |
| Contract | One compact table: what is owned, validated, lazy, cached, and refused |
| Use | The smallest runnable example, in Rust and Python tabs, and a JavaScript tab where the Node binding carries the door |
| Feature sections | One per behaviour, each at most two sentences before its code |
| Edges | Refusals, nulls, empties, overflows, and limits, one line each |
| Commands | The test and benchmark commands scoped to the page |
| Performance | The measured table, its host and toolchain, and the regenerate command |

A [Media](media/index.md) page keeps that order with its use split by direction: Overview - the contract table: what declares the medium, its build, its doors in each language, its settings and refusals - then Read and Write, each led by its example, then the sections only that medium has, then Performance.
