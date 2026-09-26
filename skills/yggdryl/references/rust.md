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
| `aws` | the AWS credential chain, profiles, SSO, STS, SigV4 | - |
| `s3` | Amazon S3, Google Cloud Storage, Azure Blob Storage handles | `aws` |

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
let price = schema.fields()[2].dtype().scalar(Scalar::d128(1_250, 2))?;
assert_eq!(price, Scalar::d128(125_000, 4));

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
landed) through the `log` facade and never per row; install any `log`
implementation (`env_logger`, `tracing-log`) to see it. Nothing is emitted
when none is installed.

## Gotchas in Rust

- A datatype constructor that validates (`DataType::decimal`, `StructType::from_fields`) returns `Result`; the parameter-free ones (`DataType::Int64`, `DataType::utf8()`) do not.
- `required_field(name)` / `nullable_field(name)` turn a `DataType` into a `Field`; `Field::new(name, dtype, nullable)` is the same with the flag spelled.
- A `Field` is one variant per shape; descend with `fields()`, `field_at`, `get_field_by_path`, never by rebuilding a `DataType`.
- `Scalar::from(&str)` is `utf8`; `Scalar::from(7_i64)` is `int64` - pick the Rust literal type that matches the column, or pass it through `dtype.scalar(..)` to narrow.
- Iterating a reader yields `Result` items and fuses after the first error: use `?` inside the loop.
