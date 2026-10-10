# yggdryl-records in Rust

`use yggdryl::{IOBase, IOMedia}` for the record verbs and `use yggdryl::media::IORecordOptions` for the option builders; Arrow batches are `arrow_array` 59. Parquet needs `features = ["parquet"]`, Iceberg `features = ["iceberg"]` (implies `parquet`).

## Which encoding will this handle use?

The handle's media type decides; `record_options()` answers that encoding's `RecordOptions` and refuses one no medium claims, naming the crate to install; an absent resource reads as no batches. A medium's own settings are its options struct, reached as `options.settings::<ParquetOptions>()`.

```rust
use yggdryl::holder::Buffer;
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::parquet::ParquetOptions;
use yggdryl::{IOBase, IOMedia, MimeType, Url};

let options = RecordOptions::for_media_type(&Url::from_str("file:///trades.parquet")?.media_type())?;
assert_eq!(options.mime_type(), MimeType::PARQUET);
assert_eq!(options.settings::<ParquetOptions>().map(|parquet| parquet.max_row_group_size), Some(1_048_576));
assert!(RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?.settings::<ParquetOptions>().is_none());

// Absent reads as empty; an encoding no claim answers is named, never guessed.
let empty = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
assert_eq!(empty.read_arrow_reader(&empty.record_options()?)?.count(), 0);
let orc = Buffer::new().with_media_type(MimeType::ORC.into());
assert!(orc.record_options().unwrap_err().to_string().contains("application/vnd.apache.orc"));
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

// A row offset skips leading rows before the row bound counts.
let page = plain.clone().with_select(["id"])?.with_row_offset(3).with_max_row_size(2);
assert_eq!(ids_of(&page)?, [3, 4]);

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

## Refuse a value the declared field cannot convert

A nullable declared column takes a value it cannot convert as null while `safe` holds, and `safe` is the default; `with_safe(false)` (or a required column) refuses it by path.

```rust
use std::sync::Arc;

use arrow_array::{Array, Int32Array, RecordBatch, StringArray};
use yggdryl::holder::Buffer;
use yggdryl::media::IORecordOptions;
use yggdryl::{arrow, DataType, IOBase, IOMedia, MimeType, StructType};

let stored = DataType::from(StructType::from_fields([DataType::utf8().nullable_field("v")])?)
    .required_field("row");
let arrow_schema = stored.into_arrow_schema()?;
let batch = RecordBatch::try_new(Arc::clone(&arrow_schema), vec![Arc::new(StringArray::from(vec!["1", "x"]))])?;
let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
let options = handle.record_options()?;
handle.overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &options)?;

let declared = options.with_field(
    DataType::from(StructType::from_fields([DataType::Int32.nullable_field("v")])?).required_field("row"),
);

// Default: the unconvertible "x" becomes null without a word.
let read = handle.read_arrow_reader(&declared)?.next().expect("a batch")?;
let v = read.column(0).as_any().downcast_ref::<Int32Array>().expect("int32");
assert_eq!((v.value(0), v.is_null(1)), (1, true));

// with_safe(false) names the column and the value instead.
let strict = declared.with_safe(false);
let refused = match handle.read_arrow_reader(&strict) {
    Ok(reader) => reader.collect::<Result<Vec<_>, _>>().map(|_| ()).unwrap_err().to_string(),
    Err(error) => error.to_string(),
};
assert!(refused.contains("$.v"), "{refused}");
```

## Write and read rows as values

`*_records` takes anything `Into<Scalar>` in the field's column order; `read_serie` answers a `StreamChunkedSerie`, one record `Serie` per batch, whose children are the columns.

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
for records in handle.read_serie(Some(&options.clone().with_filter("id >= 2")?))?.into_chunked_stream(None, None)?.into_chunks() {
    let records = records?;
    let venue = records.child("venue").expect("a venue column");
    for row in 0..venue.len() {
        venues.push(venue.scalar(row)?);
    }
}
assert_eq!(venues, [Scalar::from("XNYS"), Scalar::from("XLON")]);
```

## Append, and upsert by key

Overwrite replaces, append keeps the stored rows, merge updates rows whose `merge_by` key matches and appends the rest; it holds only the stored side in memory, and replaces a stored row only where the last arrival for its key differs from it, so a merge changing nothing leaves the leaf unwritten. Merge without a key is refused, and so is a boolean `false` key.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use yggdryl::holder::Buffer;
use yggdryl::media::IORecordOptions;
use yggdryl::{arrow, DataType, IOBase, IOMedia, IOResult, MimeType, Scalar, StructType};

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
let keyed = options.clone().with_merge_by(["id"])?;
handle.merge_arrow_reader(rows(vec![2, 9], vec!["MSFT.O", "AMD"]), &keyed)?;
assert_eq!(handle.row_size()?, 4);

// A merge whose rows equal the stored ones still counts them, and writes nothing.
let before = handle.read_all_bytes()?;
assert_eq!(handle.merge_arrow_reader(rows(vec![2], vec!["MSFT.O"]), &keyed)?, IOResult::new(1, 1));
assert_eq!(handle.read_all_bytes()?, before);

let refused = handle.merge_arrow_reader(rows(vec![1], vec!["X"]), &options).unwrap_err();
assert!(refused.to_string().contains("merge_by"), "{refused}");
// `true` is the destination's own key - none on a leaf - and `false` names no key.
assert_eq!(options.clone().with_merge_by_scalar(&Scalar::from(true))?, options);
assert!(options.clone().with_merge_by_scalar(&Scalar::from(false)).is_err());
```

## Choose the write mode at run time

`write_arrow_reader`/`write_arrow_batch`/`write_records` take an `IOMode`, and so does `write_serie`, whose `overwrite_serie`/`append_serie`/`merge_serie` name it: they take a `Serie`, a `ChunkedSerie` or a `StreamChunkedSerie` as one `Serie` (`.into()`), written as the batches it already is, and with `read_serie` are also the record door of JSON, JSON Lines, YAML, TOML and XML handles. `None` options are the handle's own.

```rust
use yggdryl::holder::Buffer;
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::{ChunkedSerie, DataType, IOBase, IOMedia, IOMode, MimeType, Scalar, Serie, StreamChunkedSerie, StructType, Url};

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
    stream.write_arrow_reader(StreamChunkedSerie::from_serie(rows.clone())?.into_arrow_reader(), mode, &options)?;
}
assert_eq!(stream.row_size()?, 2);

// The Serie doors take a held column, held chunks or a stream; None options are the handle's own.
stream.write_serie(rows.clone().into(), IOMode::Append, None)?;
stream.append_serie(ChunkedSerie::from_serie(rows.clone())?.into(), None)?;
assert_eq!(stream.row_size()?, 4);

// A JSON Lines handle takes rows as documents through overwrite_serie.
let mut lines = Buffer::new().with_media_type(Url::from_str("file:///quotes.jsonl")?.media_type());
lines.overwrite_serie(StreamChunkedSerie::from_serie(rows.clone())?.into(), None)?;
let declared = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?.with_field(root);
assert_eq!(lines.read_serie(Some(&declared))?.into_chunked_stream(None, None)?.into_chunks().collect::<Result<Vec<_>, _>>()?, vec![rows.clone()]);

// A document is written whole: write_serie on it takes IOMode::Overwrite only.
let refused = lines.append_serie(rows.into(), None).unwrap_err();
assert!(refused.to_string().contains("expected overwrite, got append"), "{refused}");
```

## Bound memory on large writes

`with_commit_batch_num(N)` publishes every N whole batches, then the remainder (a committed prefix survives a later failure); a cadence never cuts a batch. Unset is the destination's own cadence - a leaf, a folder and an Iceberg table commit once, the table holding every partition's rows under the process spill bound until the source ends; what any cadence holds between publications is held under that bound, heaviest batches spilled first; `0` is refused before any input is pulled. `with_num_threads(n)` is how many partition groups an Iceberg commit writes at once (unset: `write.parallelism`, else `read.parallelism`, else the host), `0` refused naming `$.num_threads`. `with_batch_row_size` bounds the batches any record read yields - Parquet, Arrow IPC, Avro, and plain text alike.

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
// Three batches at two batches a commit: rows 0-7 publish, then rows 8-9.
let batches = [batch.slice(0, 4), batch.slice(4, 4), batch.slice(8, 2)];
handle.overwrite_arrow_reader(
    arrow::batch_reader(Arc::clone(&arrow_schema), batches),
    &options.clone().with_commit_batch_num(2),
)?;
assert_eq!(handle.row_size()?, 10);

let sizes: Vec<usize> = handle
    .read_arrow_reader(&options.clone().with_batch_row_size(4))?
    .map(|batch| batch.map(|batch| batch.num_rows()))
    .collect::<Result<_, _>>()?;
assert_eq!(sizes.iter().sum::<usize>(), 10);
assert!(sizes.iter().all(|rows| *rows <= 4));

let refused = handle
    .overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &options.with_commit_batch_num(0))
    .unwrap_err();
assert!(refused.to_string().contains("commit_batch_num"), "{refused}");
```

## Parquet: compression, pruning, footer answers

Pages compress inside the file (default `zstd(1)`); a read never names it. A `filter` skips row groups the footer rules out. An outer `.gz`/`.zst` coding on the name is refused.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch};
use yggdryl::holder::Buffer;
use yggdryl::media::IORecordOptions;
use yggdryl::parquet::ParquetOptions;
use yggdryl::{arrow, DataType, IOBase, IOMedia, MimeType, StructType, Url};

let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
    .required_field("row");
let arrow_schema = schema.into_arrow_schema()?;
let batch = RecordBatch::try_new(Arc::clone(&arrow_schema), vec![Arc::new(Int64Array::from_iter_values(0..1_000))])?;

let mut handle = Buffer::new().with_media_type(MimeType::PARQUET.into());
let mut options = handle.record_options()?;
let parquet = options.require_settings_mut::<ParquetOptions>("$.compression", "a page compression")?;
parquet.set_compression_name("snappy")?;
parquet.set_max_row_group_size(250);
handle.overwrite_arrow_reader(arrow::batch_reader(Arc::clone(&arrow_schema), [batch.clone()]), &options)?;

// row_size and the statistics come from the footer, never from decoding rows.
assert_eq!(handle.row_size()?, 1_000);
assert_eq!(yggdryl::parquet::read_media_statistics(&handle)?.row_groups.len(), 4);
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

A `.avro` handle is a record medium (`AvroOptions::set_block_codec`, reached as `options.require_settings_mut::<AvroOptions>(..)`: `null`, `deflate`, `snappy`, `zstandard`). `yggdryl::avro` is the scalar codec; a reader schema resolves renames, promotions and defaults.

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

## Excel: one worksheet as records, the workbook as cells

A `.xlsx` handle is a record medium over one worksheet - `ExcelOptions::set_sheet` and `set_range`, reached through `require_settings_mut::<ExcelOptions>(..)`, and `RecordOptions::set_header` pick which cells - and `yggdryl::excel::Workbook` is the same package cell by cell.

```rust
use yggdryl::excel::{CellRef, ExcelOptions, Sheet, Workbook};
use yggdryl::holder::Buffer;
use yggdryl::media::IORecordOptions;
use yggdryl::{DataType, IOBase, IOMedia, MimeType, Scalar, Serie, StructType};

let field = DataType::from(StructType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::utf8().nullable_field("symbol"),
])?)
.required_field("row");
let rows = Serie::from_scalars(field.clone(), [
    Scalar::from_sequence([Scalar::from(1_i64), Scalar::from("AAPL")]),
    Scalar::from_sequence([Scalar::from(2_i64), Scalar::Null]),
])?;

// One worksheet as records, under the declared field.
let mut handle = Buffer::new().with_media_type(MimeType::XLSX.into());
let mut options = handle.record_options()?.with_field(field.clone());
options.require_settings_mut::<ExcelOptions>("$.sheet", "a worksheet")?.set_sheet(Some("Trades"))?;
handle.overwrite_arrow_batch(rows.clone().into_arrow_batch()?, &options)?;
let read: usize = handle.read_arrow_reader(&options)?.map(|batch| batch.unwrap().num_rows()).sum();
assert_eq!(read, 2);
// Inferred, a number column is the float64 the file stores.
let mut inferred = handle.record_options()?;
inferred.require_settings_mut::<ExcelOptions>("$.sheet", "a worksheet")?.set_sheet(Some("Trades"))?;
assert_eq!(handle.read_arrow_field(&inferred)?.fields()[0].dtype(), &DataType::Float64);

// The workbook: any cell by its A1 reference, a sheet as a Serie and back.
let mut workbook = Workbook::from_bytes(handle.read_all_bytes()?)?;
let sheet = workbook.sheet_mut("Trades")?;
assert_eq!(sheet.scalar("B2".parse()?), Scalar::from("AAPL"));
sheet.set_cell(CellRef::new(2, 1), "MSFT")?;
assert_eq!(sheet.clone().into_serie(Some(&field), true, Default::default())?.len(), 2);
workbook.insert_sheet(Sheet::from_serie("Copy", &rows, true)?)?;
let reopened = Workbook::from_bytes(workbook.into_bytes()?)?;
assert_eq!(reopened.sheet_names(), ["Trades", "Copy"]);
```

## Read a log file as typed rows

`into_text_with(TextOptions)` reads one record per line (or per framed chain with `framing`): the fifteen event columns, `body`, then one column per named `rowheader` capture, typed by `autotype`.

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

let records = text.read_serie(None)?.into_chunked_stream(None, None)?.next_chunk().expect("one batch")?;
let body = records.child("body").expect("the body column");
let id = records.child("id").expect("a capture column");
assert_eq!(body.scalar(0)?, Scalar::from("first\n detail A"));
assert_eq!(id.scalar(1)?, Scalar::from(9_i64));
```

## CSV and TSV: the dialect on the options

A `.csv` handle reads RFC 4180 records under its header and a sample of the rows, or under the declared `field`; a `.tsv` name is the same medium under a tab. The dialect is `RecordOptions` properties (`csv_separator`, `set_csv_separator`, ..., or `CsvOptions` directly), never a format argument; compression is the name's (`trades.csv.gz`).

```rust
use yggdryl::csv::CsvOptions;
use yggdryl::holder::Buffer;
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::{DataType, IOBase, IOMedia, MimeType, Scalar, StructType};

struct Trade(i64, Option<&'static str>);

impl From<Trade> for Scalar {
    fn from(row: Trade) -> Self {
        Scalar::from_sequence([Scalar::from(row.0), row.1.map_or(Scalar::Null, Scalar::from)])
    }
}

let field = DataType::from(StructType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::utf8().nullable_field("symbol"),
])?)
.required_field("trade");
let mut handle = Buffer::new().with_media_type(MimeType::CSV.into());
let declared = handle.record_options()?.with_field(field.clone());
handle.overwrite_records([Trade(1, Some("AAPL")), Trade(2, None), Trade(3, Some(""))], &declared)?;
// A null is the empty cell; the empty text is quoted, so the two read back apart.
assert_eq!(handle.read_all_bytes()?, b"id,symbol\n1,AAPL\n2,\n3,\"\"\n".to_vec());

// Undeclared, the header names the columns and the sample types them, every one nullable.
let inferred = handle.read_arrow_field(&handle.record_options()?)?;
assert_eq!(inferred.dtype(), &DataType::from_str("struct<id: int64, symbol: utf8>")?);
assert_eq!((handle.row_size()?, handle.column_size()?), (3, 2));

// Declared, every cell crosses the column's contract; a null and "" stay apart.
let mut symbols = Vec::new();
for records in handle.read_serie(Some(&declared))?.into_chunked_stream(None, None)?.into_chunks() {
    let records = records?;
    let symbol = records.child("symbol").expect("a symbol column");
    for row in 0..symbol.len() {
        symbols.push(symbol.scalar(row)?);
    }
}
assert_eq!(symbols, [Scalar::from("AAPL"), Scalar::Null, Scalar::from("")]);

// A `;` document another writer saved: the separator is a property of the read.
let semicolon = Buffer::from_bytes(b"id;symbol\n1;AAPL\n2;\n".to_vec()).with_media_type(MimeType::CSV.into());
let mut dialect = semicolon.record_options()?;
dialect.set_csv_separator(b';')?;
assert_eq!(dialect.csv_separator(), Some(b';'));
let batch = semicolon.read_arrow_reader(&dialect)?.next().expect("one batch")?;
assert_eq!((batch.num_rows(), batch.num_columns()), (2, 2));
// Under the default dialect the same header is one column.
assert_eq!(semicolon.read_arrow_field(&semicolon.record_options()?)?.field_len(), 1);

// A tab separator is what a `.tsv` name reads and writes.
assert_eq!(CsvOptions::tsv().separator(), b'\t');
assert_eq!(RecordOptions::for_mime_type(&MimeType::TSV)?.csv_separator(), Some(b'\t'));
assert_eq!(RecordOptions::for_mime_type(&MimeType::PARQUET)?.csv_separator(), None);

// A record with the wrong number of cells is refused by row, never widened.
let ragged = Buffer::from_bytes(b"a,b\n1,2\n3\n".to_vec()).with_media_type(MimeType::CSV.into());
let refused = match ragged.read_arrow_reader(&ragged.record_options()?) {
    Ok(reader) => reader.collect::<Result<Vec<_>, _>>().map(|_| ()).unwrap_err().to_string(),
    Err(error) => error.to_string(),
};
assert!(refused.contains("expected 2 cells, got 1 in row 3"), "{refused}");
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

`PARTITION:by` declares it - a bare column an identity partition, a term a derived one (`years(event)`, `truncate(name, 4) as prefix`) - and `with_partition_by` marks the identity columns and adds each derived entry as a marked column carrying its term as `TRANSFORM:` metadata; `apply_arrow_batch` on the transform view of the root fills it where absent or all null and leaves values alone. A write only casts, so fill the column through `root.as_transform().apply_arrow_batch` before a partitioned write: a required derived column the rows lack is refused by path, a nullable one lands null.

```rust
use std::sync::Arc;

use arrow_array::{ArrayRef, Date32Array, Int32Array, RecordBatch};
use yggdryl::{DataType, StructType};

let root = DataType::from(StructType::from_fields([DataType::date32().required_field("event")])?)
    .required_field("row")
    .with_partition_by(["year(event) as year".parse()?])?;
assert_eq!(root.partition_field_names().collect::<Vec<_>>(), ["year"]);

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
use yggdryl::iceberg::{assign_field_ids, FormatVersion, PartitionSpec, IcebergTable};
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
let mut table = IcebergTable::create(LocalFolder::new(&path)?, FormatVersion::V2, schema.clone(), spec)?;
assert!(table.current_snapshot()?.is_none());

table.commit_append(rows(vec![1, 2, 3], vec!["XNAS", "XNYS", "XNAS"], vec![1.0, 2.0, 3.0]))?;
let first = table.current_snapshot()?.expect("a snapshot").snapshot_id;
table.commit_merge(rows(vec![3, 4], vec!["XNAS", "XNAS"], vec![30.0, 4.0]), &"id".parse::<Selector>()?, true)?;

assert_eq!(count(table.scan(None)?)?, 4);
assert_eq!(count(table.scan_matching("px > 2.5", None)?)?, 2);
assert_eq!(table.plan_matching("venue = 'XNYS'")?.tasks.len(), 1);
assert_eq!(count(table.scan_at(first, &[], None)?)?, 3); // time travel

let reopened = IcebergTable::open(LocalFolder::new(&path)?)?;
assert_eq!(reopened.current_snapshot()?.expect("a snapshot").operation(), "overwrite");
let _ = std::fs::remove_dir_all(&path);
```

## Iceberg: the table's own key

A table whose schema states `identifier-field-ids` keys every write by it: a merge naming no key matches on it (`with_merge_by_scalar(&Scalar::from(true))` states it outright), and an append writes only the rows whose key neither the table nor an earlier row of the write holds - the first arrival kept, the rest in `skipped_rows`, no stored file rewritten. A merge or an append that changes nothing commits no snapshot. A keyed append beaten by a concurrent commit fails with `CommitConflict` instead of rebasing.

```rust
use yggdryl::iceberg::{assign_field_ids, FormatVersion, IcebergTable, PartitionSpec};
use yggdryl::local::LocalFolder;
use yggdryl::media::IORecordOptions;
use yggdryl::{DataType, IOMedia, IOResult, Scalar, Serie, StructType};

let row = DataType::from(StructType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::utf8().nullable_field("venue"),
])?)
.required_field("row");
let mut schema = row.clone();
assign_field_ids(&mut schema, 1)?;
// The table's own key: `id`, named by the field id the numbering gave it.
schema.as_iceberg_mut().set_identifier_field_ids(&[1])?;

let path = LocalFolder::temporary()?.path()?.join("yggdryl-skill-records-iceberg-keyed");
let _ = std::fs::remove_dir_all(&path);
let mut table = IcebergTable::create(LocalFolder::new(&path)?, FormatVersion::V2, schema, PartitionSpec::unpartitioned())?;
let trade = |id: i64, venue: &str| Scalar::from_sequence([Scalar::from(id), Scalar::from(venue)]);
let trades = |rows: Vec<Scalar>| Serie::from_scalars(row.clone(), rows);

assert_eq!(table.append_serie(trades(vec![trade(1, "XNAS"), trade(2, "XNYS")])?.into(), None)?, IOResult::new(2, 2));
// 2 is stored and 3 arrives twice: one row written, two skipped.
let appended = table.append_serie(trades(vec![trade(2, "XLON"), trade(3, "XPAR"), trade(3, "XAMS")])?.into(), None)?;
assert_eq!((appended.read_rows, appended.written_rows, appended.skipped_rows), (3, 1, 2));

// A merge naming no key matches on the table's own; `true` states it.
let own = table.record_options()?.with_merge_by_scalar(&Scalar::from(true))?;
table.merge_serie(trades(vec![trade(1, "XAMS")])?.into(), Some(&own))?;
// Replaying it changes no row: counted as written, committed nowhere.
let snapshots = table.metadata()?.snapshots().len();
assert_eq!(table.merge_serie(trades(vec![trade(1, "XAMS")])?.into(), None)?, IOResult::new(1, 1));
assert_eq!(table.metadata()?.snapshots().len(), snapshots);
assert!(own.with_merge_by_scalar(&Scalar::from(false)).is_err());

let mut rows: Vec<Scalar> = Vec::new();
for batch in table.read_serie(None)?.into_chunked_stream(None, None)?.into_chunks() {
    rows.extend(batch?.rows().into_owned());
}
rows.sort();
assert_eq!(rows, [trade(1, "XAMS"), trade(2, "XNYS"), trade(3, "XPAR")]);
let _ = std::fs::remove_dir_all(&path);
```

## Evolve an Iceberg schema

`SchemaUpdate` records column operations; `IcebergTable::update_schema` replays them onto the schema each commit attempt reads - so a commit beaten by another writer rebases rather than overwrites - keeps field IDs, never reuses a dropped one, and answers the schema id it made current. `evolve_schema(field)` replaces the schema whole.

```rust
use std::sync::Arc;

use arrow_array::{Int32Array, RecordBatch};
use yggdryl::iceberg::{assign_field_ids, FormatVersion, PartitionSpec, SchemaUpdate, IcebergTable};
use yggdryl::local::LocalFolder;
use yggdryl::{arrow, DataType, StructType};

let mut schema = DataType::from(StructType::from_fields([DataType::Int32.required_field("id")])?)
    .required_field("row");
assign_field_ids(&mut schema, 1)?;
let path = LocalFolder::temporary()?.path()?.join("yggdryl-skill-records-evolve");
let _ = std::fs::remove_dir_all(&path);
let mut table = IcebergTable::create(LocalFolder::new(&path)?, FormatVersion::V2, schema.clone(), PartitionSpec::unpartitioned())?;
let batch = RecordBatch::try_new(schema.into_arrow_schema()?, vec![Arc::new(Int32Array::from(vec![1]))])?;
table.commit_append(arrow::batch_reader(batch.schema(), [batch]))?;

let mut update = SchemaUpdate::from_metadata(table.metadata()?)?;
update.add_column("", DataType::utf8().nullable_field("note"));
update.update_type("id", DataType::Int64);
let schema_id = table.update_schema(&update)?;
assert_eq!(schema_id, table.metadata()?.current_schema_id());

assert_eq!(table.schema()?.field_len(), 2);
let first = table.scan(None)?.next().expect("one batch")?;
assert_eq!(first.column(0).data_type(), &arrow_schema::DataType::Int64);
assert_eq!(first.column(1).null_count(), 1);
let _ = std::fs::remove_dir_all(&path);
```

## Write a pandas or polars frame to a file, and read one back

Python only (`overwrite_pandas_frame`, `read_polars_frame`, ...). In Rust, write the `RecordBatch`es in hand with `overwrite_arrow_reader` and read with `read_arrow_reader`.

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
- Every verb that decodes, casts, or writes rows takes `&RecordOptions`; get it from `handle.record_options()?` so the variant matches the encoding. `row_size()`, `column_size()`, `record_options()` and `parquet::read_media_statistics(&handle)` take none: they derive their own options internally. `read_serie` and the `*_serie` writes take `Option<&RecordOptions>`, `None` the handle's own.
- `with_select`, `with_filter`, `with_merge_by` and `with_plan` parse and return `Result`; `with_field`, `with_max_row_size`, `with_commit_batch_num` do not.
- `with_plan` keeps a plan's `limit` as `max_row_size` and its `offset` as `row_offset`; a merge with a `row_offset` is refused.
- `write_serie` on a JSON, JSON Lines, YAML, TOML or XML handle takes `IOMode::Overwrite` only - `overwrite_serie` - and reads only the declared `field` off the options: a document is written whole. A run, or a record holding an absent row, is refused before any handle is touched.
- A declared nullable column reads a value it cannot convert as null under the default `safe`; `with_safe(false)` refuses it.
- There is no `read_records` in Rust: rows out are `read_serie` columns (`child`, `scalar(i)`) or the `RecordBatch`es themselves.
- A CSV byte role is a `u8` (`b';'`), one ASCII byte that is no line break and no other role's; `set_csv_*` on another encoding's options is an error, and `csv_*` on them answers `None`. `linesep` is `CsvOptions::with_linesep` only.
- Parquet, Iceberg and S3 do not exist without their Cargo features; a Parquet setting on another encoding's options is an error, not a no-op - `require_settings_mut::<ParquetOptions>` refuses it naming both encodings.
