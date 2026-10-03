# yggdryl in Rust

The crate is flat: every type, trait and enum is at the root
(`yggdryl::DataType`, `yggdryl::IOBase`, `yggdryl::MimeType`), and each
implementation is a module of its own name (`yggdryl::holder::Buffer`,
`yggdryl::local::LocalFile`, `yggdryl::parquet`, `yggdryl::json`).

## Add the dependency

Everything beyond the core is a feature, and none is on by default. Turn on
only what the program reads or writes.

```toml
[dependencies]
yggdryl = "0.1"
# Parquet files, Iceberg tables (needs Rust 1.94), object stores:
# yggdryl = { version = "0.1", features = ["parquet", "iceberg", "s3"] }
```

| Feature | Adds | Implies |
| --- | --- | --- |
| `parquet` | Parquet reader/writer, Avro snappy blocks | - |
| `iceberg` | Iceberg tables over the crate's own Parquet | `parquet` |
| `http` | the HTTP/1.1 client, sessions, requests/responses, resumable streams, the `message/http` medium | - |
| `http2` | HTTP/2 beside HTTP/1.1 (ALPN `h2`, or prior-knowledge `h2c`) | `http` |
| `http3` | HTTP/3 over QUIC | `http2` |
| `aws` | the AWS credential chain, profiles, SSO, STS, SigV4 | `http` |
| `s3` | Amazon S3, Google Cloud Storage, Azure Blob Storage handles | `aws` |

Without `http`, a bare `http://`/`https://` URL through `Holder::from_url`
refuses at runtime naming the missing feature; `yggdryl::http` itself does
not exist in the build.

Arrow types come from the `arrow-*` 59 crates (`arrow-array`,
`arrow-schema`, ...); add the ones you name in your own code at the same
version so the types unify.

## Errors

Two error types, one per side of the Arrow boundary: `yggdryl::Error` for
values, schemas and storage, `yggdryl::arrow::Error` for anything that takes
or answers an Arrow type or runs the cast engine. `?` converts each into the
other and into `Box<dyn std::error::Error>`.

```rust
use yggdryl::{DataType, Error};

let refused: Error = DataType::Int8.scalar(1000_i64).unwrap_err();
// A refusal names what was expected, what arrived and where.
let message = refused.to_string();
assert!(message.contains("int8"), "{message}");
```

## End to end: schema, value, records, document

A non-null Struct `Field` is the schema. Values enter through `scalar`, rows
are `Scalar` sequences, and a record handle's media type picks the encoding.

```rust
use yggdryl::holder::Buffer;
use yggdryl::media::IORecordOptions;
use yggdryl::{
    DataType, IOMedia, MimeType, Scalar, StructType, from_json_scalar, into_json_scalar,
};

// The schema: a required struct field whose children are the columns.
let schema = DataType::from(StructType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::utf8().nullable_field("symbol"),
    DataType::decimal(18, 4)?.required_field("price"),
])?)
.required_field("trade");

// A value enters through its type: 12.50 at scale 2 lands at the column's scale 4.
let price = schema.fields()[2].dtype().scalar(Scalar::decimal128(1_250, 2))?;
assert_eq!(price, Scalar::decimal128(125_000, 4));

// Rows are ordered sequences, one value per child field.
let rows = [
    Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL"), price.clone()]),
    Scalar::from_sequence([Scalar::from(2_i64), Scalar::Null, price]),
];

// An in-memory handle; the media type decides the encoding (Arrow IPC stream).
let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
let options = handle.record_options()?.with_field(schema.clone());
handle.overwrite_records(rows, &options)?;

// Reads stream one record column per batch; a record lends its children by name.
let mut ids = Vec::new();
for records in handle.read_arrow(Some(&options))? {
    let id = records?.child("id").cloned().expect("an id column");
    for row in 0..id.len() {
        ids.push(id.scalar(row)?);
    }
}
assert_eq!(ids, [Scalar::from(1_i64), Scalar::from(2_i64)]);

// The same values render to and parse from JSON through one codec.
let document = from_json_scalar(br#"{"symbol":"MSFT","id":3}"#)?;
assert!(into_json_scalar(&document)?.contains("MSFT"));
```

## Logging

The core narrates operations (a table opened, a scan planned, a commit
landed) through the `log` facade and never per row; nothing is emitted until
a `log` backend is installed. Any backend works (`env_logger`, `tracing-log`),
and the core ships its own: `yggdryl::logging`, Python's `logging` in Rust -
dotted logger names, numeric levels on Python's scale (`NOTSET` 0, `TRACE` 5,
`DEBUG` 10, `INFO` 20, `WARNING` 30, `ERROR` 40, `CRITICAL` 50), level
inheritance, `propagate`, handlers and `%`-style formatters.

- `logging::install()` makes the tree the backend of the `log` facade, so a
  record's target `yggdryl::iceberg::table` is the logger
  `yggdryl.iceberg.table`; installing twice is one installation, and it
  refuses with `Error::Conflict` when another backend already is the facade's.
  `logging::basic_config(BasicConfig::new().with_level(Level::INFO))` is the
  one call an application makes: it installs and gives the root a handler on
  standard error (`with_handler(..)` states others) writing the terminal
  line: the time, the level's glyph and name, `[thread]`, the logger, the
  call site, the message -
  `2026-10-03 14:05:09,123 • INFO     [main] trades.feed open:42 › opened 3 venues`.
  Colour is on only where `logging::is_color_enabled` says: `NO_COLOR` off,
  else `FORCE_COLOR` or `CLICOLOR_FORCE` on, else `TERM=dumb` off, else a
  terminal.
- `logging::get_logger("trades.feed")` is the logger of that name, `""` the
  root. A record no handler takes is written to standard error at `WARNING`
  and above, as that line. A handler with no formatter writes it too, plain
  unless the handler is coloured; `Formatter::default()` is `%(message)s`
  and `Formatter::BASIC_FORMAT` Python's `WARNING:trades.book:crossed`.
- `[thread]` is the name `logging::set_thread_name("feed")` gave the logging
  thread, else its spawned name, else `Thread-N`.
- `FileHandler::new(handle)` writes through any storage handle (`LocalFile`,
  `Buffer`, an S3 object, a ZIP member): one `append_bytes` per publish, each
  record at once by default. An append is a whole publish on a remote store
  (one `PUT`), so a handler over one states `with_capacity(bytes)`, and
  `with_flush_level(level)` (`ERROR` by default) publishes what it holds as a
  severe record arrives. A record never waits on a publish in flight: one
  logged on another thread meanwhile is taken in by the publishing thread.
  `flush()` or `close()` on the thread holding `handler.io()`'s guard is
  refused - `Error::Io` of kind `std::io::ErrorKind::Deadlock` - rather than
  waiting on itself, so drop the guard first.
- The process ends without dropping what a static holds: call
  `logging::shutdown()` before `main` returns to publish and close every
  handler. A handler's own `flush()` and a dropped handler publish too.
- `logger.set_deduplicating(Some(true))` counts each record by its logger,
  level and message where it is logged: the first occurrence goes on as
  logged, the 10th, 100th, 1000th as `message (seen N times)`, and every
  other reaches no handler. A logger takes its nearest ancestor's choice
  (`None` states none), off at the root unless set.

```rust
use std::sync::Arc;

use yggdryl::holder::Buffer;
use yggdryl::local::{LocalFile, LocalFolder};
use yggdryl::logging::{self, FileHandler, Formatter, Handler, Level};
use yggdryl::IOBase;

let root = LocalFolder::temporary()?.path()?.join("yggdryl-docs-skill-logging");
let _ = std::fs::remove_dir_all(&root);
let location = root.join("logs").join("feed.log");

// The tree becomes the `log` facade's backend; a second call is a no-op.
logging::install()?;

// A handler is a storage handle plus a format: one append per record.
let handler = Arc::new(FileHandler::new(LocalFile::new(&location)?));
handler.set_formatter(Formatter::from_str("%(levelname)s %(name)s %(message)s")?);

// A handler on a logger takes the records of its children too.
let feed = logging::get_logger("docs.skills.yggdryl.logging");
feed.set_level(Level::INFO);
feed.set_propagating(false);
feed.add_handler(handler.clone());

feed.info("opened 3 venues");
feed.debug("under the level");
logging::get_logger("docs.skills.yggdryl.logging.book").warning("crossed");
// A `log` record reaches the logger named after its target.
log::info!(target: "docs::skills::yggdryl::logging", "from the facade");

assert_eq!(
    std::fs::read_to_string(&location)?,
    "INFO docs.skills.yggdryl.logging opened 3 venues\n\
     WARNING docs.skills.yggdryl.logging.book crossed\n\
     INFO docs.skills.yggdryl.logging from the facade\n"
);

// A handler stating no formatter writes the terminal line, plain.
let plain = Arc::new(FileHandler::new(Buffer::new()));
let book = logging::get_logger("docs.skills.yggdryl.plain");
book.set_level(Level::WARNING);
book.set_propagating(false);
book.add_handler(plain.clone());
book.warning("crossed");
let line = String::from_utf8(plain.io().read_all_bytes()?)?;
assert!(line.contains(" ! WARNING  ["), "{line}");
assert!(line.contains("] docs.skills.yggdryl.plain "), "{line}");
assert!(line.ends_with(" › crossed\n"), "{line}");

// Over a remote store, hold records back: nothing is published until the
// capacity is reached or a record at the flush level (`ERROR`) arrives.
let held = Arc::new(FileHandler::new(Buffer::new()).with_capacity(4096));
held.set_formatter(Formatter::default());
let batch = logging::get_logger("docs.skills.yggdryl.held");
batch.set_level(Level::INFO);
batch.set_propagating(false);
batch.add_handler(held.clone());
batch.info("one");
batch.info("two");
assert!(held.io().read_all_bytes()?.is_empty());
batch.error("three");
assert_eq!(held.io().read_all_bytes()?, b"one\ntwo\nthree\n");

// An application ends with `logging::shutdown();`, which does this for every handler.
handler.close()?;
std::fs::remove_dir_all(&root)?;
```

## Gotchas in Rust

- A datatype constructor that validates (`DataType::decimal`, `StructType::from_fields`) returns `Result`; the parameter-free ones (`DataType::Int64`, `DataType::utf8()`) do not.
- `required_field(name)` / `nullable_field(name)` turn a `DataType` into a `Field`; `Field::new(name, dtype, nullable)` is the same with the flag spelled.
- A `Field` is one variant per shape; descend with `fields()`, `field_at`, `get_field_by_path`, never by rebuilding a `DataType`.
- `Scalar::from(&str)` is `utf8`; `Scalar::from(7_i64)` is `int64` - pick the Rust literal type that matches the column, or pass it through `dtype.scalar(..)` to narrow.
- Iterating a reader yields `Result` items and fuses after the first error: use `?` inside the loop.
