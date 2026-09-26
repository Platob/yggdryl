# yggdryl-arrow in Rust

No feature flag is needed for this layer. Name Arrow types through
`arrow-array` and `arrow-schema` at the release yggdryl is built on (`59`);
yggdryl does not re-export them. The core names are at the crate root:
`yggdryl::{Serie, ChunkedSerie, SerieReader, ArrowCastPlan, ArrowCastOptions,
Representation}`; stream helpers are in `yggdryl::arrow`.

## Build a column from values

`Serie::from_scalars` sends every row through `Field::scalar` once and lays
the buffers out once; `from_default` repeats the field's canonical default.

```rust
use yggdryl::{DataType, Field, Scalar, Serie};

let price = Field::new("price", DataType::Int64, false);

// Three widths in, one width out: each row narrowed by the field.
let serie = Serie::from_scalars(
    price.clone(),
    [Scalar::from(125_i32), Scalar::from(126_u8), Scalar::from(127_i64)],
)?;
assert_eq!(serie.len(), 3);
assert_eq!(serie.scalar(0)?, Scalar::from(125_i64));

// A required field has no room for a null: the whole column is refused.
assert!(Serie::from_scalars(price.clone(), [Scalar::Null]).is_err());

// Defaults: a required field repeats its present default, a nullable one null.
assert_eq!(Serie::from_default(price.clone(), 2)?.scalar(1)?, Scalar::from(0_i64));
assert_eq!(Serie::from_default(Field::new("symbol", DataType::utf8(), true), 2)?.null_count(), 2);
assert!(Serie::empty(price.clone())?.is_empty());
assert!(Serie::with_capacity(price, 1024)?.is_empty());
```

## Land an Arrow array, sharing its buffers

`Serie::from_arrow_array(Some(&field), array, options)` compiles one plan
from the array's layout to the field. An exact layout is the identity: the
same buffers, no row read. Any other layout is cast.

```rust
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Int64Array, StringArray};
use yggdryl::{ArrowCastOptions, DataType, Field, Serie};

let options = ArrowCastOptions::new();
let field = Field::new("price", DataType::Int64, false);
let source: ArrayRef = Arc::new(Int64Array::from(vec![125, 126, 127]));

let held = Serie::from_arrow_array(Some(&field), Arc::clone(&source), options)?;
let out = held.require_arrow_array()?;
assert_eq!(out.to_data().buffers()[0].as_ptr(), source.to_data().buffers()[0].as_ptr());

// No field: the column of its own layout, named `item`, nullable where it holds a null.
let absent: ArrayRef = Arc::new(Int64Array::from(vec![Some(1), None]));
let own = Serie::from_arrow_array(None, absent, options)?;
assert_eq!(own.field(), Some(&Field::new("item", DataType::Int64, true)));

// Another layout is cast once, on the way in.
let text: ArrayRef = Arc::new(StringArray::from(vec!["12", "1234"]));
let sizes = Serie::from_arrow_array(Some(&field), text, options)?;
assert_eq!(sizes.as_int64().expect("an int64 column").values(), &[12, 1234]);
```

## Land a record batch as a record column

A `RecordBatch` lands under a non-null struct root: children matched by name
(ASCII case-insensitive), in the root's order, extra columns dropped, missing
nullable ones all-null.

```rust
use std::sync::Arc;

use arrow_array::{ArrayRef, Float64Array, Int32Array, RecordBatch, StringArray};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
use yggdryl::{ArrowCastOptions, DataType, Field, Serie, StructType};

let root = DataType::from(StructType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::utf8().nullable_field("symbol"),
    DataType::utf8().nullable_field("venue"),
])?)
.required_field("trade");
let batch = RecordBatch::try_new(
    Arc::new(Schema::new(vec![
        ArrowField::new("SYMBOL", ArrowDataType::Utf8, true),
        ArrowField::new("id", ArrowDataType::Int32, false),
        ArrowField::new("extra", ArrowDataType::Float64, true),
    ])),
    vec![
        Arc::new(StringArray::from(vec!["ACME"])) as ArrayRef,
        Arc::new(Int32Array::from(vec![7])) as ArrayRef,
        Arc::new(Float64Array::from(vec![1.5])) as ArrayRef,
    ],
)?;

let trades = Serie::from_arrow_batch(Some(&root), &batch, ArrowCastOptions::new())?;
let names: Vec<&str> = trades.children().iter().filter_map(Serie::field).map(Field::name).collect();
assert_eq!(names, ["id", "symbol", "venue"]);
assert_eq!(trades.child("venue").map(Serie::null_count), Some(1));
assert_eq!(trades.into_arrow_batch()?.schema().field(0).data_type(), &ArrowDataType::Int64);

// With no root the batch is the record `row` of its own schema, columns shared.
let own = Serie::from_arrow_batch(None, &batch, ArrowCastOptions::new())?;
assert_eq!(own.field().map(Field::name), Some("row"));
```

## Build a stream from batches you produce

Rust has no foreign runtime to recognize (Python's `Serie.from_` ladder has
no Rust counterpart): a producer hands batches to
`yggdryl::arrow::batch_reader(schema, batches)`, which takes any
`IntoIterator` and pulls lazily. Drain it into one column only when the
whole stream fits.

```rust
use std::sync::Arc;

use arrow_array::{ArrayRef, Int64Array, RecordBatch, RecordBatchReader};
use yggdryl::arrow::batch_reader;
use yggdryl::{ArrowCastOptions, DataType, Serie, StructType};

let root = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
    .required_field("row");
let schema = root.clone().into_arrow_schema()?;
let produce = {
    let schema = Arc::clone(&schema);
    (0..3_i64).map(move |id| {
        RecordBatch::try_new(Arc::clone(&schema), vec![Arc::new(Int64Array::from(vec![id])) as ArrayRef])
            .expect("a batch of the schema")
    })
};

let reader = batch_reader(Arc::clone(&schema), produce);
assert_eq!(reader.schema(), schema); // known before a batch is pulled

// Eager: the whole stream held as one column.
let held = Serie::from_arrow_reader(Some(&root), reader, ArrowCastOptions::new())?;
assert_eq!(held.len(), 3);
```

## Stream a reader under one plan

`SerieReader::from_arrow_reader(root, reader, options)` compiles one plan
from the stream's schema before a batch is pulled and holds at most one
source batch. A bad batch fails at its pull, and the reader is fused after
it. `into_arrow_reader()` is the transport face, never landed.

```rust
use std::sync::Arc;

use arrow_array::{ArrayRef, Int32Array, RecordBatch, RecordBatchReader, StringArray};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
use yggdryl::arrow::batch_reader;
use yggdryl::{ArrowCastOptions, DataType, Serie, SerieReader, StructType};

let root = DataType::from(StructType::from_fields([
    DataType::Int64.nullable_field("id"),
    DataType::utf8().required_field("symbol"),
])?)
.required_field("row");
let schema = Arc::new(Schema::new(vec![
    ArrowField::new("id", ArrowDataType::Int32, false),
    ArrowField::new("symbol", ArrowDataType::Utf8, true),
]));
let batch = |id: i32, symbol: Option<&str>| {
    RecordBatch::try_new(
        Arc::clone(&schema),
        vec![
            Arc::new(Int32Array::from(vec![id])) as ArrayRef,
            Arc::new(StringArray::from(vec![symbol])) as ArrayRef,
        ],
    )
};

let stream = batch_reader(Arc::clone(&schema), [batch(1, Some("A"))?, batch(2, Some("B"))?, batch(3, None)?]);
let mut series = SerieReader::from_arrow_reader(Some(&root), stream, ArrowCastOptions::new())?;
assert_eq!(series.field(), &root);
let first = series.next().transpose()?.expect("a first batch");
assert_eq!(first.child("id").and_then(Serie::as_int64).map(|ids| ids.values().to_vec()), Some(vec![1]));
assert!(series.next().is_some_and(|second| second.is_ok()));
let refusal = series.next().expect("a third batch").unwrap_err();
assert!(refusal.to_string().contains("$.symbol"), "{refusal}");
assert!(series.next().is_none());

// Transport: the root's schema up front, batches cast as they are pulled.
let good = batch_reader(Arc::clone(&schema), [batch(1, Some("A"))?, batch(2, Some("B"))?]);
let reader = SerieReader::from_arrow_reader(Some(&root), good, ArrowCastOptions::new())?.into_arrow_reader();
assert_eq!(reader.schema().field(0).data_type(), &ArrowDataType::Int64);
let rows = reader.map(|batch| batch.map(|batch| batch.num_rows())).sum::<Result<usize, _>>()?;
assert_eq!(rows, 2);
```

## Compile a cast once and apply it to many batches

`ArrowCastPlan::compile(&source, &target, options)` does every
schema-dependent decision once; it is immutable and `Send + Sync`, so one
plan serves every batch and every thread. A batch schema is a source as
`Field::from_arrow_schema("row", &schema)`.

```rust
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Int32Array, Int64Array, RecordBatch};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
use yggdryl::{ArrowCastOptions, ArrowCastPlan, ChunkedSerie, DataType, Field, Serie, StructType};

let options = ArrowCastOptions::new();
let root = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
    .required_field("row");
let schema = Arc::new(Schema::new(vec![ArrowField::new("id", ArrowDataType::Int32, false)]));

let source = Field::from_arrow_schema("row", &schema)?;
let plan = ArrowCastPlan::compile(&source, &root, options)?;
plan.preflight()?; // the whole recursion over no rows
assert_eq!(plan.as_target(), &root);
assert!(!plan.is_identity());

let mut landed = Vec::new();
for offset in 0..3 {
    let batch = RecordBatch::try_new(
        Arc::clone(&schema),
        vec![Arc::new(Int32Array::from(vec![offset])) as ArrayRef],
    )?;
    // The batch lands as its own layout, sharing its columns; the held plan casts it.
    let column = Serie::from_arrow_batch(None, &batch, options)?;
    let cast = plan.apply(&column)?;
    assert_eq!(cast.child("id").and_then(Serie::as_int64).map(|ids| ids.values()[0]), Some(i64::from(offset)));
    landed.push(column);
}

// One plan over every chunk, the chunks kept apart.
let chunks = ChunkedSerie::from_series(None, landed, options)?;
assert_eq!(plan.apply_chunked(&chunks)?.num_chunks(), 3);

// An identity plan hands back the same buffers.
let price = Field::new("price", DataType::Int64, false);
let identity = ArrowCastPlan::compile(&price, &price, options)?;
assert!(identity.is_identity());
let column = Serie::from_arrow_array(Some(&price), Arc::new(Int64Array::from(vec![1, 2])), options)?;
let same = identity.apply(&column)?.require_arrow_array()?;
assert_eq!(same.to_data().buffers()[0].as_ptr(), column.require_arrow_array()?.to_data().buffers()[0].as_ptr());

// A missing required column is refused when the plan is compiled.
let other = Field::from_arrow_schema("row", &Schema::new(vec![ArrowField::new("other", ArrowDataType::Int32, true)]))?;
let message = ArrowCastPlan::compile(&other, &root, options).unwrap_err().to_string();
assert_eq!(message, "required Arrow field $.id is missing from the source");
```

## Cast a column in hand

`serie.cast(&target, options)` compiles one plan for one column; a column
already under the target is itself. A bare datatype is its required field
named `value`.

```rust
use yggdryl::{ArrowCastOptions, DataType, Field, Scalar, Serie};

let options = ArrowCastOptions::new();
let ids = Serie::from_scalars(Field::new("id", DataType::Int32, true), [Scalar::from(1_i32), Scalar::Null])?;

let wide = ids.cast(&Field::new("id", DataType::Int64, true), options)?;
assert_eq!(wide.as_int64().expect("an int64 column").value(0), Some(1));
assert!(wide.is_null(1)?);
assert_eq!(ids.cast(ids.require_field()?, options)?, ids);

let value = DataType::Int64.required_field("value");
let message = ids.cast(&value, options).unwrap_err().to_string();
assert_eq!(message, "required Arrow field $.value holds 1 null values");

// A schema-free run has no buffers to cast: type its rows instead.
let run = Serie::new(vec![Scalar::from(1_i32)]);
assert!(run.cast(&value, options).is_err());
assert_eq!(Serie::from_scalars(value, run.rows().into_owned())?.len(), 1);
```

## Decide what a bad or missing value becomes

Nullability of the target field decides absence; `safe` decides only whether
a present value that fails to convert becomes null, and only where the column
may hold null. An empty text cell is null before `safe` is asked.

```rust
use std::sync::Arc;

use arrow_array::{ArrayRef, StringArray};
use yggdryl::{ArrowCastOptions, DataType, Serie};

let broken: ArrayRef = Arc::new(StringArray::from(vec!["1", "not a number", ""]));
let nullable = DataType::Int64.nullable_field("n");
let required = DataType::Int64.required_field("n");
let lenient = ArrowCastOptions::new();
let strict = lenient.with_safe(false);

// Nullable + safe (default): the failure and the empty cell are null.
let column = Serie::from_arrow_array(Some(&nullable), Arc::clone(&broken), lenient)?;
assert_eq!(column.null_count(), 2);

// Nullable + safe = false: the failed conversion is refused by value.
let refused = Serie::from_arrow_array(Some(&nullable), Arc::clone(&broken), strict).unwrap_err();
assert!(refused.to_string().contains("not a number"), "{refused}");

// Required: refused whatever safe says; never filled with a default.
for options in [lenient, strict] {
    let refused = Serie::from_arrow_array(Some(&required), Arc::clone(&broken), options).unwrap_err();
    assert!(refused.to_string().contains("not a number"), "{refused}");
}

// An empty cell into a required column is a null, refused by path.
let empty: ArrayRef = Arc::new(StringArray::from(vec![""]));
let refused = Serie::from_arrow_array(Some(&required), empty, strict).unwrap_err();
assert_eq!(refused.to_string(), "required Arrow field $.n holds 1 null values");
```

## Reinterpret bits between same-width types

`Representation::Bits` shares the value buffer between two layouts of one
byte width; other pairs convert as usual.

```rust
use std::sync::Arc;

use arrow_array::{ArrayRef, Int32Array, UInt64Array};
use yggdryl::{ArrowCastOptions, DataType, Field, Representation, Serie};

let bits = ArrowCastOptions::new().with_representation(Representation::Bits);
let source: ArrayRef = Arc::new(UInt64Array::from(vec![0, u64::MAX]));

let signed = Serie::from_arrow_array(Some(&Field::new("digest", DataType::Int64, true)), Arc::clone(&source), bits)?;
assert_eq!(signed.as_int64().expect("an int64 column").values(), &[0, -1]);

let raw = Serie::from_arrow_array(Some(&Field::new("digest", DataType::fixed_binary(8)?, true)), source, bits)?;
assert_eq!(raw.as_fixed_bytes().expect("a fixed byte column").value(1), Some(&[0xff_u8; 8][..]));
let back = raw.cast(&Field::new("digest", DataType::UInt64, true), bits)?;
assert_eq!(back.as_uint64().expect("a uint64 column").values(), &[0, u64::MAX]);

// Four bytes are not eight: an ordinary widening.
let narrow: ArrayRef = Arc::new(Int32Array::from(vec![7]));
let widened = Serie::from_arrow_array(Some(&Field::new("id", DataType::Int64, true)), narrow, bits)?;
assert_eq!(widened.as_int64().expect("an int64 column").values(), &[7]);
```

## Read values fast

Narrow once with `as_<leaf>()` - the leaf names the storage layout - then
read each row off the buffer: `values()` is the slice itself, `value(i)` an
`Option` honouring the validity bitmap. In a stream, narrow once per batch.
The typed writers (`get_<leaf>_mut`, `push_value`) store straight into the
buffer and validate nullability.

```rust
use std::sync::Arc;

use arrow_array::{ArrayRef, Int64Array, StringArray};
use yggdryl::{ArrowCastOptions, DataType, Field, Serie};

let options = ArrowCastOptions::new();
let price = Field::new("price", DataType::Int64, true);
let array: ArrayRef = Arc::new(Int64Array::from(vec![Some(125), None, Some(127)]));
let mut prices = Serie::from_arrow_array(Some(&price), array, options)?;

// One narrowing, then a bounds check and a load per row; no Scalar built.
let leaf = prices.as_int64().ok_or("an int64 column")?;
assert_eq!(leaf.values().len(), 3); // includes the slot under the null
assert!(leaf.nulls().is_some());
let total: i64 = (0..prices.len()).filter_map(|i| leaf.value(i)).sum();
assert_eq!(total, 252);

// A typed write lands in the same buffer; `None` is refused by a required field.
let writer = prices.get_int64_mut().ok_or("an int64 column")?;
writer.push_value(Some(128))?;
writer.push_value(None)?;
assert_eq!((prices.len(), prices.null_count()), (5, 2));

// Text lends its bytes where they lie: `as_utf8` covers every UTF-8 string leaf.
let symbols: ArrayRef = Arc::new(StringArray::from(vec!["AAPL", "MSFT"]));
let text = Serie::from_arrow_array(Some(&Field::new("symbol", DataType::utf8(), false)), symbols, options)?;
let leaf = text.as_utf8().ok_or("a utf8 column")?;
assert_eq!(leaf.value(1), Some("MSFT"));
assert_eq!(leaf.offsets().as_ref(), &[0, 4, 8]);
```

## Read and write rows, children and slices

`splice` is the one mutation and every other write is spelled over it; a
refusal leaves the column unchanged. `scalar`, `get`, `rows` and `iter`
build values per call. Nested cells are written by `FieldPath`; no child is
handed out mutably.

```rust
use yggdryl::{DataType, Field, FieldPath, Scalar, Serie, StructType};

let mut prices = Serie::from_scalars(Field::new("price", DataType::Int64, true), [Scalar::from(1_i64)])?;
prices.push(Scalar::from(2_i64))?;
prices.extend(vec![Scalar::from(3_i64), Scalar::Null])?;
prices.set(0, Scalar::from(10_i32))?; // narrowed through the field
prices.insert(1, Scalar::from(5_i64))?;
assert_eq!(prices.remove(1)?, Scalar::from(5_i64));
assert_eq!(prices.pop()?, Some(Scalar::Null));
prices.splice(0..1, vec![Scalar::from(7_i64), Scalar::from(8_i64)])?;
assert_eq!(prices.rows().into_owned(), [7_i64, 8, 2, 3].map(Scalar::from).to_vec());
assert_eq!(prices.scalar(1)?, Scalar::from(8_i64));
assert!(prices.get(99).is_none());
assert_eq!(prices.slice(1, 2)?.len(), 2); // zero copy
prices.truncate(2)?;
assert_eq!(prices.iter().count(), 2);

let root = Field::new(
    "row",
    DataType::from(StructType::from_fields([
        Field::new("id", DataType::Int64, false),
        Field::new("tags", DataType::serie(Field::new("item", DataType::utf8(), false)), true),
    ])?),
    false,
);
let mut rows = Serie::from_scalars(
    root,
    [
        Scalar::from_sequence([Scalar::from(1_i64), Scalar::from_sequence([Scalar::from("a"), Scalar::from("b")])]),
        Scalar::from_sequence([Scalar::from(2_i64), Scalar::Null]),
    ],
)?;
rows.set_cell(&FieldPath::from_str("id")?, 1, Scalar::from(20_i8))?;
assert_eq!(rows.child("id").and_then(Serie::as_int64).map(|ids| ids.values().to_vec()), Some(vec![1, 20]));
let items = rows.get_child_by_path(&FieldPath::from_str("tags.item")?).ok_or("a column at the path")?;
assert_eq!(items.as_utf8().ok_or("a utf8 column")?.value(1), Some("b"));

let refused = rows.push(Scalar::from_sequence([Scalar::from("x"), Scalar::Null]));
assert!(refused.is_err());
assert_eq!(rows.len(), 2); // unchanged
```

## Keep chunks and batches apart

`ChunkedSerie` holds arrays or batches without concatenating: a row is a
binary search over the chunk ends, a clone or window moves chunk pointers.
`into_serie` is the one join.

```rust
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Int64Array};
use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar, Serie};

let options = ArrowCastOptions::new();
let field = Field::new("price", DataType::Int64, false);
let first: ArrayRef = Arc::new(Int64Array::from(vec![125, 126]));
let second: ArrayRef = Arc::new(Int64Array::from(vec![127]));

let prices = ChunkedSerie::from_arrow_arrays(Some(&field), [Arc::clone(&first), second], options)?;
assert_eq!((prices.len(), prices.num_chunks()), (3, 2));
assert_eq!(prices.scalar(2)?, Scalar::from(127_i64));
assert_eq!(prices.slice(1, 2)?.num_chunks(), 2);
let arrays = prices.into_arrow_arrays();
assert_eq!(arrays[0].to_data().buffers()[0].as_ptr(), first.to_data().buffers()[0].as_ptr());

// One plan, every chunk; a foreign chunk is cast in as it is appended.
let wide = prices.cast(&Field::new("price", DataType::Float64, true), options)?;
assert_eq!(wide.num_chunks(), 2);
let mut grown = prices.clone();
grown.push_chunk(Serie::from_scalars(Field::new("p", DataType::Int32, true), [Scalar::from(128_i32)])?, options)?;
assert_eq!(grown.scalar(3)?, Scalar::from(128_i64));

// The one join; identity is the rows, however they are cut.
let joined = prices.into_serie()?;
assert!(prices == joined);

// A table: one chunk per batch, and back out one batch per chunk.
let reader = ChunkedSerie::from_serie(Serie::from_scalars(field, [Scalar::from(1_i64)])?)?.into_arrow_reader()?;
let table = ChunkedSerie::from_arrow_reader(None, reader, options)?;
assert_eq!(table.num_chunks(), 1);
assert_eq!(table.child("price").map(|column| column.len()), Some(1));
```

## Hand a held column on as a stream

`SerieReader::from_serie` and `from_chunked` read held data as a stream with
no plan and no copy - what `IOMedia::write_arrow` takes. `reader.cast`
re-roots the stream under one plan and consumes the reader.

```rust
use yggdryl::{ArrowCastOptions, ChunkedSerie, DataType, Field, Scalar, Serie, SerieReader, StructType};

let options = ArrowCastOptions::new();
let root = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
    .required_field("row");
let rows = Serie::from_scalars(
    root.clone(),
    [Scalar::from_sequence([Scalar::from(1_i64)]), Scalar::from_sequence([Scalar::from(2_i64)])],
)?;
let held = SerieReader::from_serie(rows.clone())?;
assert_eq!(held.field(), &root);
assert_eq!(held.collect::<Result<Vec<_>, _>>()?, vec![rows.clone()]);

// A leaf column is the one child of a `row` record.
let price = Serie::from_scalars(Field::new("price", DataType::Int64, true), [Scalar::from(1_i64)])?;
let record = SerieReader::from_serie(price.clone())?.next().expect("one batch")?;
assert_eq!(record.child("price").map(Serie::len), Some(1));

let chunks = ChunkedSerie::from_series(None, [price.clone(), price], options)?;
assert_eq!(SerieReader::from_chunked(chunks)?.count(), 2);

// Re-root under a wider field: one plan, cast as each record is pulled.
let wide = DataType::from(StructType::from_fields([DataType::Float64.required_field("id")])?)
    .required_field("row");
let cast = SerieReader::from_serie(rows)?.cast(&wide, options)?;
let records = cast.collect::<Result<Vec<_>, _>>()?;
assert_eq!(records[0].scalar(1)?, Scalar::from_sequence([Scalar::from(2.0_f64)]));
```

## Hold a column as one value

`Scalar::from(serie)` makes a column one serie value and `Scalar::as_serie`
borrows it back; neither reads a row. `Serie::as_serie` is a different verb:
it narrows a column to its `SerieSerie` (list) leaf.

```rust
use yggdryl::{DataType, Field, Scalar, Serie};

let column = Serie::from_scalars(
    Field::new("price", DataType::Int64, false),
    [Scalar::from(125_i64), Scalar::from(126_i64)],
)?;
let value = Scalar::from(column.clone());
assert_eq!(value.kind(), "serie");
assert!(value.as_serie().is_some_and(|held| held == &column));
assert!(column.as_serie().is_none()); // an int64 column is no list leaf

// Identity is the rows alone: not the field, not the width.
assert_eq!(column, Serie::new(vec![Scalar::from(125_i64), Scalar::from(126_i64)]));
```

## Convert schemas and types

A non-null struct `Field` is the schema: `Field::from_arrow_schema` and
`into_arrow_schema` cross whole schemas, `from_arrow_field` /
`into_arrow_field` and `from_arrow_datatype` / `into_arrow_datatype` single
fields and types. Every conversion re-checks parameters.

```rust
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
use yggdryl::{DataType, Field, StructType};

let schema = Schema::new(vec![
    ArrowField::new("id", ArrowDataType::Int64, false),
    ArrowField::new("symbol", ArrowDataType::Utf8, true),
]);
let root = Field::from_arrow_schema("row", &schema)?;
let expected = DataType::from(StructType::from_fields([
    DataType::Int64.required_field("id"),
    DataType::utf8().nullable_field("symbol"),
])?)
.required_field("row");
assert_eq!(root, expected);
assert_eq!(root.clone().into_arrow_schema()?.as_ref(), &schema);

assert_eq!(DataType::from_arrow_datatype(&ArrowDataType::Int32)?, DataType::Int32);
assert_eq!(DataType::Int64.into_arrow_datatype()?, ArrowDataType::Int64);
let arrow_field = ArrowField::new("x", ArrowDataType::Int32, false);
let field = Field::from_arrow_field(&arrow_field)?;
assert_eq!(field, Field::new("x", DataType::Int32, false));
assert_eq!(field.into_arrow_field()?, arrow_field);

// A schema root must be a non-null struct: refused, never coerced.
assert!(Field::new("row", DataType::Int64, false).into_arrow_schema().is_err());
```

## Merge two streams

`yggdryl::arrow::combined(left, right)` chains two readers onto the root
their schemas merge into, pulling no row to decide it; `combined_as` casts
both onto a root you declare.

```rust
use std::sync::Arc;

use arrow_array::{Array, Int64Array, RecordBatch, RecordBatchReader, StringArray};
use yggdryl::arrow::{batch_reader, combined};
use yggdryl::{DataType, StructType};

let left_schema = DataType::from(StructType::from_fields([DataType::Int64.nullable_field("id")])?)
    .required_field("row")
    .into_arrow_schema()?;
let right_schema = DataType::from(StructType::from_fields([
    DataType::Int64.nullable_field("id"),
    DataType::utf8().nullable_field("venue"),
])?)
.required_field("row")
.into_arrow_schema()?;
let left = batch_reader(
    Arc::clone(&left_schema),
    [RecordBatch::try_new(left_schema, vec![Arc::new(Int64Array::from(vec![1_i64]))])?],
);
let right = batch_reader(
    Arc::clone(&right_schema),
    [RecordBatch::try_new(
        right_schema,
        vec![Arc::new(Int64Array::from(vec![2_i64])), Arc::new(StringArray::from(vec!["XPAR"]))],
    )?],
);

let joined = combined(left, right)?;
assert_eq!(joined.schema().fields().len(), 2);
let batches: Vec<RecordBatch> = joined.collect::<Result<_, _>>()?;
assert!(batches[0].column(1).is_null(0)); // the left side has no `venue`
assert_eq!(batches.iter().map(RecordBatch::num_rows).sum::<usize>(), 2);
```

## Gotchas in Rust

- The Arrow doors, `cast` and the readers return `yggdryl::arrow::Result`;
  the row verbs return `yggdryl::Result`. `?` converts between them.
- `into_arrow_array()` answers `None` for a schema-free run; use
  `require_arrow_array()?` to get an error naming it instead.
- `as_<leaf>()` names the storage layout, not the datatype: `as_utf8()`
  answers for `ascii`, `sized_utf8(n)` and the codes laid out as UTF-8, and
  `SerieValue::id` on the leaf says which datatype it is.
- `values()` includes the slots under nulls; read `value(i)` or check
  `nulls()`.
- A clone shares buffers; the first write to either copies them once, and
  every later write is in place.
- `SerieReader::cast` takes the reader by value; a refusal is returned at the
  call for held records and at the pull for a stream's batches.
- A plan compiled for one source layout refuses another by name; key plans by
  source schema when a loop's batches can change schema.
