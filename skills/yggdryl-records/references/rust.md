# yggdryl-records in Rust

`use yggdryl::{IOBase, IOMedia}` for the record verbs and `use yggdryl::media::IORecordOptions` for the option builders; Arrow batches are `arrow_array` 59. Parquet needs `features = ["parquet"]`, Iceberg `features = ["iceberg"]` (implies `parquet`).

## Which encoding will this handle use?

The handle's media type decides; `record_options()` answers that encoding's `RecordOptions` and refuses one the build does not implement; an absent resource reads as no batches.

```rust
use yggdryl::holder::Buffer;
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::{IOBase, IOMedia, MimeType, Url};

let options = RecordOptions::for_media_type(&Url::from_str("file:///trades.parquet")?.media_type())?;
assert_eq!(options.mime_type(), MimeType::PARQUET);
assert_eq!(options.parquet_max_row_group_size(), Some(1_048_576));
assert_eq!(RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?.parquet_max_row_group_size(), None);

// Absent reads as empty; an unimplemented encoding is named, never guessed.
let empty = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
assert_eq!(empty.read_arrow_reader(&empty.record_options()?)?.count(), 0);
let csv = Buffer::new().with_media_type(MimeType::CSV.into());
assert!(csv.record_options().unwrap_err().to_string().contains("text/csv"));
```

## Write batches and stream them back

`read_arrow_reader` answers an `arrow::BatchReader` (an iterator of `RecordBatch`); pass it straight to another handle's `overwrite_arrow_reader` so nothing is collected.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use yggdryl::holder::Buffer;
use yggdryl::{arrow, DataType, IOBase, IOMedia, MimeType, StructType};

let schema = DataType::from(StructType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::utf8().nullable_field("venue"),
])?)
.required_field("row");
let arrow_schema = schema.clone().into_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
    vec![
        Arc::new(Int64Array::from(vec![1, 2, 3])),
        Arc::new(StringArray::from(vec![Some("XNAS"), Some("XNYS"), None])),
    ],
)?;

let mut source = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
let options = source.record_options()?;
source.overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &options)?;
assert_eq!(source.read_arrow_field(&options)?, schema);
assert_eq!((source.row_size()?, source.column_size()?), (3, 2));

// Stream from one handle into another: only the current batch is alive.
let mut target = Buffer::new().with_media_type(MimeType::PARQUET.into());
let target_options = target.record_options()?;
target.overwrite_arrow_reader(source.read_arrow_reader(&options)?, &target_options)?;

let mut rows = 0;
for batch in target.read_arrow_reader(&target_options)? {
    rows += batch?.num_rows();
}
assert_eq!(rows, 3);
```

## Read only the columns and rows I need

Put `select`, `filter`, a declared `field` and `max_row_size` on the options: the medium projects and prunes, the field casts in the same pass, and the limit stops pulling.

```rust
use std::sync::Arc;

use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
use yggdryl::holder::Buffer;
use yggdryl::media::IORecordOptions;
use yggdryl::{arrow, DataType, IOBase, IOMedia, MimeType, StructType};

let stored = DataType::from(StructType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::utf8().required_field("venue"),
])?)
.required_field("row");
let arrow_schema = stored.into_arrow_schema()?;
let ids: Vec<i64> = (0..10).collect();
let venues: Vec<&str> = ids.iter().map(|id| if id % 2 == 0 { "XNAS" } else { "XNYS" }).collect();
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
    vec![Arc::new(Int64Array::from(ids)), Arc::new(StringArray::from(venues))],
)?;
let mut handle = Buffer::new().with_media_type(MimeType::PARQUET.into());
let plain = handle.record_options()?;
handle.overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &plain)?;

let ids_of = |options| -> Result<Vec<i64>, Box<dyn std::error::Error>> {
    let mut ids = Vec::new();
    for batch in handle.read_arrow_reader(options)? {
        let batch = batch?;
        let column = batch.column(0).as_any().downcast_ref::<Int64Array>().expect("int64 ids");
        ids.extend(column.values().iter().copied());
    }
    Ok(ids)
};

// Sections on one options value: projection, predicate, row bound.
let picked = plain.clone().with_select(["id"])?.with_filter("id > 3")?.with_max_row_size(2);
assert_eq!(ids_of(&picked)?, [4, 5]);

// The same sections spelled as one plan.
let planned = plain.clone().with_plan("select id where venue = 'XNYS' limit 3")?;
assert_eq!(ids_of(&planned)?, [1, 3, 5]);

// A declared field projects and casts in one pass.
let narrow = DataType::from(StructType::from_fields([DataType::Int32.required_field("id")])?)
    .required_field("row");
let first = handle.read_arrow_reader(&plain.with_field(narrow))?.next().expect("a batch")?;
assert_eq!(first.num_columns(), 1);
assert_eq!(first.column(0).data_type(), &arrow_schema::DataType::Int32);
```

## Write and read rows as values

`*_records` takes anything `Into<Scalar>` in the field's column order; `read_arrow` answers a `SerieReader`, one record `Serie` per batch, whose children are the columns.

```rust
use yggdryl::holder::Buffer;
use yggdryl::media::IORecordOptions;
use yggdryl::{DataType, IOBase, IOMedia, MimeType, Scalar, StructType};

struct Trade(i64, &'static str);

impl From<Trade> for Scalar {
    fn from(row: Trade) -> Self {
        Scalar::from_sequence([Scalar::from(row.0), Scalar::from(row.1)])
    }
}

let field = DataType::from(StructType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::utf8().required_field("venue"),
])?)
.required_field("row");
let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
let options = handle.record_options()?.with_field(field);

handle.overwrite_records([Trade(1, "XNAS"), Trade(2, "XNYS")], &options)?;
handle.append_records([Trade(3, "XLON")], &options)?;

let mut venues = Vec::new();
for records in handle.read_arrow(Some(&options.clone().with_filter("id >= 2")?))? {
    let records = records?;
    let venue = records.child("venue").expect("a venue column");
    for row in 0..venue.len() {
        venues.push(venue.scalar(row)?);
    }
}
assert_eq!(venues, [Scalar::from("XNYS"), Scalar::from("XLON")]);
```

## Append, and upsert by key

Overwrite replaces, append keeps the stored rows, merge updates rows whose `merge_by` key matches and appends the rest; it holds only the stored side in memory. Merge without a key is refused.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use yggdryl::holder::Buffer;
use yggdryl::media::IORecordOptions;
use yggdryl::{arrow, DataType, IOBase, IOMedia, MimeType, StructType};

let schema = DataType::from(StructType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::utf8().nullable_field("symbol"),
])?)
.required_field("row");
let arrow_schema = schema.clone().into_arrow_schema()?;
let rows = |ids: Vec<i64>, symbols: Vec<&'static str>| {
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![Arc::new(Int64Array::from(ids)), Arc::new(StringArray::from(symbols))],
    )
    .expect("a batch matching the root");
    arrow::batch_reader(batch.schema(), [batch])
};

let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
let options = handle.record_options()?.with_field(schema);
handle.overwrite_arrow_reader(rows(vec![1, 2], vec!["AAPL", "MSFT"]), &options)?;
handle.append_arrow_reader(rows(vec![3], vec!["NVDA"]), &options)?;
handle.merge_arrow_reader(rows(vec![2, 9], vec!["MSFT.O", "AMD"]), &options.clone().with_merge_by(["id"])?)?;
assert_eq!(handle.row_size()?, 4);

let refused = handle.merge_arrow_reader(rows(vec![1], vec!["X"]), &options).unwrap_err();
assert!(refused.to_string().contains("merge_by"), "{refused}");
```

## Choose the write mode at run time

`write_arrow_reader`/`write_arrow_batch`/`write_records` take an `IOMode`; `write_arrow`/`read_arrow` take and answer a `SerieReader` and are also the record door of JSON, JSON Lines, YAML, TOML and XML handles.

```rust
use yggdryl::holder::Buffer;
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::{DataType, IOBase, IOMedia, IOMode, MimeType, Scalar, Serie, SerieReader, StructType, Url};

let root = DataType::from(StructType::from_fields([
    DataType::utf8().required_field("symbol"),
    DataType::Int64.required_field("size"),
])?)
.required_field("row");
let rows = Serie::from_scalars(
    root.clone(),
    [Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(100_i64)])],
)?;

let mut stream = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
let options = stream.record_options()?;
for mode in [IOMode::Overwrite, IOMode::Append] {
    stream.write_arrow_reader(SerieReader::from_serie(rows.clone())?.into_arrow_reader(), mode, &options)?;
}
assert_eq!(stream.row_size()?, 2);

// A JSON Lines handle takes rows as documents through write_arrow.
let mut lines = Buffer::new().with_media_type(Url::from_str("file:///quotes.jsonl")?.media_type());
lines.write_arrow(SerieReader::from_serie(rows.clone())?, IOMode::Overwrite, None)?;
let declared = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?.with_field(root);
assert_eq!(lines.read_arrow(Some(&declared))?.collect::<Result<Vec<_>, _>>()?, vec![rows]);
```

## Bound memory on large writes

`with_commit_row_size(N)` publishes every N rows (a committed prefix survives a later failure); unset commits once; `0` is refused before any input is pulled. `with_batch_row_size` bounds the batches a Parquet read yields.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch};
use yggdryl::holder::Buffer;
use yggdryl::media::IORecordOptions;
use yggdryl::{arrow, DataType, IOBase, IOMedia, MimeType, StructType};

let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
    .required_field("row");
let arrow_schema = schema.into_arrow_schema()?;
let batch = RecordBatch::try_new(Arc::clone(&arrow_schema), vec![Arc::new(Int64Array::from_iter_values(0..10))])?;

let mut handle = Buffer::new().with_media_type(MimeType::PARQUET.into());
let options = handle.record_options()?;
handle.overwrite_arrow_reader(
    arrow::batch_reader(Arc::clone(&arrow_schema), [batch.clone()]),
    &options.clone().with_commit_row_size(4),
)?;
assert_eq!(handle.row_size()?, 10);

let sizes: Vec<usize> = handle
    .read_arrow_reader(&options.clone().with_batch_row_size(4))?
    .map(|batch| batch.map(|batch| batch.num_rows()))
    .collect::<Result<_, _>>()?;
assert_eq!(sizes.iter().sum::<usize>(), 10);
assert!(sizes.iter().all(|rows| *rows <= 4));

let refused = handle
    .overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &options.with_commit_row_size(0))
    .unwrap_err();
assert!(refused.to_string().contains("commit_row_size"), "{refused}");
```

## Parquet: compression, pruning, footer answers

Pages compress inside the file (default `zstd(1)`); a read never names it. A `filter` skips row groups the footer rules out. An outer `.gz`/`.zst` coding on the name is refused.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch};
use yggdryl::holder::Buffer;
use yggdryl::media::IORecordOptions;
use yggdryl::{arrow, DataType, IOBase, IOMedia, MimeType, StructType, Url};

let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
    .required_field("row");
let arrow_schema = schema.into_arrow_schema()?;
let batch = RecordBatch::try_new(Arc::clone(&arrow_schema), vec![Arc::new(Int64Array::from_iter_values(0..1_000))])?;

let mut handle = Buffer::new().with_media_type(MimeType::PARQUET.into());
let mut options = handle.record_options()?;
options.set_parquet_compression_name("snappy")?;
options.set_parquet_max_row_group_size(250)?;
handle.overwrite_arrow_reader(arrow::batch_reader(Arc::clone(&arrow_schema), [batch.clone()]), &options)?;

// row_size and the statistics come from the footer, never from decoding rows.
assert_eq!(handle.row_size()?, 1_000);
assert_eq!(handle.read_parquet_statistics()?.row_groups.len(), 4);
let kept: usize = handle
    .read_arrow_reader(&options.clone().with_filter("id >= 900")?)?
    .map(|batch| batch.map(|batch| batch.num_rows()))
    .sum::<Result<_, _>>()?;
assert_eq!(kept, 100);

let mut coded = Buffer::new().with_media_type(Url::from_str("file:///trades.parquet.gz")?.media_type());
let coded_options = coded.record_options()?;
let refused = coded
    .overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &coded_options)
    .unwrap_err();
assert!(refused.to_string().contains("parquet compresses"), "{refused}");
```

## Avro: a container file, or bytes with a reader schema

A `.avro` handle is a record medium (`set_avro_block_codec`: `null`, `deflate`, `snappy`, `zstandard`). `yggdryl::avro` is the scalar codec; a reader schema resolves renames, promotions and defaults.

```rust
use yggdryl::avro::{self, Schema};
use yggdryl::holder::Buffer;
use yggdryl::{json, Scalar};

let writer = json::from_utf8(
    r#"{"type":"record","name":"trade","fields":[
        {"name":"symbol","type":"string"},
        {"name":"qty","type":"int"}]}"#,
)?;
let reader = Schema::from_str(
    r#"{"type":"record","name":"trade","fields":[
        {"name":"quantity","aliases":["qty"],"type":"long"},
        {"name":"note","type":"string","default":"none"}]}"#,
)?;
let row = json::from_utf8(r#"{"symbol":"AAPL","qty":100}"#)?;
let mut handle = Buffer::new();
avro::write_container(&mut handle, &writer, &[], &[row])?;

let decoded = avro::read_container_resolved(&handle, &reader)?;
assert_eq!(decoded.rows[0].get_key_str("quantity").and_then(Scalar::as_i64), Some(100));
assert_eq!(decoded.rows[0].get_key_str("note").and_then(Scalar::as_str), Some("none"));
```

## Read a log file as typed rows

`into_text_with(TextOptions)` reads one record per line (or per framed chain with `framing`): the sixteen event columns, `body`, then one column per named `rowheader` capture, typed by `autotype`.

```rust
use yggdryl::holder::Buffer;
use yggdryl::media::IORecordOptions as _;
use yggdryl::text::TextOptions;
use yggdryl::{IOBase as _, IOMedia as _, Scalar, Url};

let source = Buffer::from_bytes(b"[INFO] id=7 first\r\n detail A\r[WARN] id=9 second\n detail B".to_vec())
    .with_media_type(Url::from_str("file:///app.log")?.media_type());

let mut text_options = TextOptions::new();
text_options.start_rownum = Some(1);
text_options.set_rowheader(Some(r"^\[(?<level>[A-Z]+)\] id=(?<id>\d+) "))?;
text_options.set_framing(true);
let text = source.into_text_with(text_options);

let records = text.read_arrow(Some(&text.record_options()?))?.next().expect("one batch")?;
let body = records.child("body").expect("the body column");
let id = records.child("id").expect("a capture column");
assert_eq!(body.scalar(0)?, Scalar::from("first\n detail A"));
assert_eq!(id.scalar(1)?, Scalar::from(9_i64));
```

## Partitioned folders: route on write, prune on read

A folder handle writes each row to its `column=value` leaf (the path carries the partition columns, the leaf does not) and reads them back typed. A `filter` equality prunes leaves by path before anything is decoded.

```rust
use std::sync::Arc;

use arrow_array::{Int32Array, Int64Array, RecordBatch, StringArray};
use yggdryl::holder::Holder;
use yggdryl::local::LocalFolder;
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::{arrow, DataType, IOBase, IOMedia, MimeType, StructType};

let root = LocalFolder::temporary()?.path()?.join("yggdryl-skill-records-lake");
let _ = std::fs::remove_dir_all(&root);
std::fs::create_dir_all(root.join("year=2024").join("month=01"))?;

let schema = DataType::from(StructType::from_fields([
    DataType::Int64.required_field("price"),
    DataType::Int32.required_field("year"),
    DataType::utf8().required_field("month"),
])?)
.required_field("row");
let arrow_schema = schema.clone().into_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
    vec![
        Arc::new(Int64Array::from(vec![10, 20])),
        Arc::new(Int32Array::from(vec![2024, 2024])),
        Arc::new(StringArray::from(vec!["01", "01"])),
    ],
)?;

let mut lake = Holder::folder(&root)?;
let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?.with_field(schema);
lake.overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &options)?;

let leaf = lake.child_by_path("year=2024/month=01/part-0.arrows")?;
assert_eq!(leaf.read_arrow_field(&RecordOptions::for_media_type(leaf.media_type())?)?.field_len(), 1);

let pruned = options.with_filter("year = 2024 and month = '01'")?;
assert_eq!(pruned.partition_pairs(), [("year".to_owned(), "2024".to_owned()), ("month".to_owned(), "01".to_owned())]);
let restored = lake.read_arrow_reader(&pruned)?.next().expect("one batch")?;
assert_eq!((restored.num_rows(), restored.num_columns()), (2, 3));

let _ = std::fs::remove_dir_all(&root);
```

## Derive a partition column from another column

`PARTITION:sources` and `PARTITION:transform` on the derived field; `apply_arrow_batch` on the root fills it where absent or all null and leaves values alone.

```rust
use std::sync::Arc;

use arrow_array::{ArrayRef, Date32Array, Int32Array, RecordBatch};
use yggdryl::expression::Function;
use yggdryl::{DataType, StructType};

let mut year = DataType::Int32.nullable_field("year");
year.as_partition_mut().set_sources(["event"])?;
year.as_partition_mut().set_transform(Function::Year)?;
let root = DataType::from(StructType::from_fields([DataType::date32().required_field("event"), year])?)
    .required_field("row");

let batch = RecordBatch::try_from_iter([(
    "event",
    Arc::new(Date32Array::from(vec![19_723, 20_089])) as ArrayRef,
)])?;
let filled = root.as_transform().apply_arrow_batch(&batch)?;

assert_eq!(filled.num_columns(), 2);
assert_eq!(filled.column(1).as_ref(), &Int32Array::from(vec![2024, 2025]) as &dyn arrow_array::Array);
```

## Iceberg: create, append, upsert, scan

A table is a folder reached through one handle; no catalog is required. Scans are `BatchReader`s planned from metadata; `commit_merge` keys are the identity partition columns plus `merge_by`.

```rust
use std::sync::Arc;

use arrow_array::{Float64Array, Int64Array, RecordBatch, StringArray};
use yggdryl::iceberg::{assign_field_ids, FormatVersion, PartitionSpec, Table};
use yggdryl::local::LocalFolder;
use yggdryl::{arrow, DataType, Selector, StructType};

let mut schema = DataType::from(StructType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::utf8().nullable_field("venue"),
    DataType::Float64.nullable_field("px"),
])?)
.required_field("row");
assign_field_ids(&mut schema, 1)?;
let arrow_schema = schema.clone().into_arrow_schema()?;
let rows = |ids: Vec<i64>, venues: Vec<&str>, prices: Vec<f64>| {
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(ids)),
            Arc::new(StringArray::from(venues)),
            Arc::new(Float64Array::from(prices)),
        ],
    )
    .expect("a batch matching the schema");
    arrow::batch_reader(batch.schema(), [batch])
};
let count = |reader: arrow::BatchReader| reader.map(|batch| batch.map(|batch| batch.num_rows())).sum::<Result<usize, _>>();

let path = LocalFolder::temporary()?.path()?.join("yggdryl-skill-records-iceberg");
let _ = std::fs::remove_dir_all(&path);
let spec = PartitionSpec::identity(1, &schema, &["venue"])?;
let mut table = Table::create(LocalFolder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;
assert!(table.current_snapshot().is_none());

table.commit_append(rows(vec![1, 2, 3], vec!["XNAS", "XNYS", "XNAS"], vec![1.0, 2.0, 3.0]))?;
let first = table.current_snapshot().expect("a snapshot").snapshot_id;
table.commit_merge(rows(vec![3, 4], vec!["XNAS", "XNAS"], vec![30.0, 4.0]), &"id".parse::<Selector>()?, true)?;

assert_eq!(count(table.scan(None)?)?, 4);
assert_eq!(count(table.scan_matching("px > 2.5", None)?)?, 2);
assert_eq!(table.plan_matching("venue = 'XNYS'")?.tasks.len(), 1);
assert_eq!(count(table.scan_at(first, &[], None)?)?, 3); // time travel

let reopened = Table::open(LocalFolder::new(&path)?)?;
assert_eq!(reopened.current_snapshot().expect("a snapshot").operation(), "overwrite");
let _ = std::fs::remove_dir_all(&path);
```

## Evolve an Iceberg schema

`SchemaUpdate` replays column operations onto the current schema, keeping field IDs and never reusing a dropped one; `evolve_schema` commits the result as one metadata document.

```rust
use std::sync::Arc;

use arrow_array::{Int32Array, RecordBatch};
use yggdryl::iceberg::{assign_field_ids, FormatVersion, PartitionSpec, SchemaUpdate, Table};
use yggdryl::local::LocalFolder;
use yggdryl::{arrow, DataType, StructType};

let mut schema = DataType::from(StructType::from_fields([DataType::Int32.required_field("id")])?)
    .required_field("row");
assign_field_ids(&mut schema, 1)?;
let path = LocalFolder::temporary()?.path()?.join("yggdryl-skill-records-evolve");
let _ = std::fs::remove_dir_all(&path);
let mut table = Table::create(LocalFolder::new(&path)?, FormatVersion::V2, schema.clone(), PartitionSpec::unpartitioned())?;
let batch = RecordBatch::try_new(schema.into_arrow_schema()?, vec![Arc::new(Int32Array::from(vec![1]))])?;
table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;

let mut update = SchemaUpdate::from_metadata(table.metadata())?;
update.add_column("", DataType::utf8().nullable_field("note"));
update.update_type("id", DataType::Int64);
table.evolve_schema(update.into_field()?)?;

assert_eq!(table.schema()?.field_len(), 2);
let first = table.scan(None)?.next().expect("one batch")?;
assert_eq!(first.column(0).data_type(), &arrow_schema::DataType::Int64);
assert_eq!(first.column(1).null_count(), 1);
let _ = std::fs::remove_dir_all(&path);
```

## Hand the file to polars or a pyarrow dataset lazily

Python only (`scan_polars`, `scan_arrow`). In Rust, iterate `read_arrow_reader` with the pushdown options above.

## Run a SQL-like write or read plan

`Plan` spells `insert into`, `insert overwrite`, `upsert into ... by (...)`, `delete from ... where`, and reads with `select ... from ... where ... limit ... offset`; `execute()` pushes the read sections into the source. Grammar: `yggdryl-expressions`.

```rust
use std::sync::Arc;

use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
use yggdryl::expression::Plan;
use yggdryl::local::LocalFolder;
use yggdryl::{DataType, StructType, Url};

let root = LocalFolder::temporary()?.path()?.join("yggdryl-skill-records-plan");
std::fs::create_dir_all(&root)?;
let url = Url::from_path(root.join("trades.arrows"))?;

let created: Plan = format!("create '{url}' (id int64 not null, name utf8)").parse()?;
created.execute()?.count();

let schema = DataType::from(StructType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::utf8().nullable_field("name"),
])?)
.required_field("trades");
let rows = RecordBatch::try_new(
    schema.into_arrow_schema()?,
    vec![Arc::new(Int64Array::from(vec![1_i64, 2, 3])), Arc::new(StringArray::from(vec!["a", "b", "c"]))],
)?;
let insert: Plan = format!("insert into '{url}'").parse()?;
insert.apply_arrow_reader(yggdryl::arrow::batch_reader(rows.schema(), [rows]))?.count();

let read: Plan = format!("select name from '{url}' where id > 1 order by id desc limit 1 offset 1").parse()?;
let batch = read.execute()?.next().expect("one batch")?;
let names = batch.column(0).as_any().downcast_ref::<StringArray>().expect("utf8 names");
assert_eq!((names.len(), names.value(0)), (1, "b"));
std::fs::remove_dir_all(&root)?;
```

## Gotchas in Rust

- `IOBase`, `IOMedia` and `IORecordOptions` are traits: import them or the methods do not resolve.
- Every record verb takes `&RecordOptions`; get it from `handle.record_options()?` so the variant matches the encoding. `read_arrow`/`write_arrow` take `Option<&RecordOptions>`.
- `with_select`, `with_filter`, `with_merge_by` and `with_plan` parse and return `Result`; `with_field`, `with_max_row_size`, `with_commit_row_size` do not.
- `RecordOptions` has no offset: `with_plan` keeps a plan's `limit` as `max_row_size` and drops its `offset`. Use `Plan::apply_arrow_reader` or `Plan::execute`.
- There is no `read_records` in Rust: rows out are `read_arrow` columns (`child`, `scalar(i)`) or the `RecordBatch`es themselves.
- Parquet, Iceberg and S3 do not exist without their Cargo features; a Parquet-only setter on another encoding's options is an error, not a no-op.
