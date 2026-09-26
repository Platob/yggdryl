# yggdryl-expressions in Rust

`Filter`, `Selector`, `Expression`, `FieldPath` and `FieldSegment` are at the
crate root; `Term`, `Plan`, `Bounds`, `col`, `lit` and the user-function
registry are under `yggdryl::expression`. `Plan::execute` over Parquet or
Iceberg needs those crate features.

## Parse a predicate, bind it once, answer rows

`bind` resolves names, converts each literal into its column's type and
orders `and` cheapest-first - once. `matches` then answers any number of rows.

```rust
use yggdryl::{Field, Filter, Scalar};

let schema: Field = "trades:struct<ccy:utf8,price:decimal(9,2),size:bigint>".parse()?;
let filter: Filter = "ccy = 'EUR' and price > 100".parse()?;
assert_eq!(filter.columns(), vec!["ccy".to_owned(), "price".to_owned()]);

let bound = filter.bind(&schema)?;
// The literal was converted once, into the column's exact type.
assert_eq!(bound.term().to_string(), "ccy = 'EUR' and price > decimal32(9,2) '100.00'");

let row = Scalar::from_sequence([Scalar::from("EUR"), Scalar::d128(15_000, 2), Scalar::from(5_i64)]);
assert!(bound.matches(&row)?);

// A null makes the answer unknown: `eval` says so, `matches` does not keep it.
let missing = Scalar::from_sequence([Scalar::from("EUR"), Scalar::Null, Scalar::from(5_i64)]);
assert_eq!(bound.eval(&missing)?, Scalar::Null);
assert!(!bound.matches(&missing)?);
```

## Build a term without text, with a parameter

`col` and `lit` compose the same tree the parser builds; a `:name` parameter
is supplied once at `bind_with`.

```rust
use yggdryl::expression::{Term, col, lit};
use yggdryl::{Field, Scalar};

let schema: Field = "trades:struct<ccy:utf8,price:decimal(9,2),size:bigint>".parse()?;

let composed = col("price").gt(lit(100_i64)).and(col("ccy").eq(lit("EUR")));
assert_eq!(composed, "price > 100 and ccy = 'EUR'".parse::<Term>()?);
assert_eq!(col("size").add(lit(1_i64)).to_string(), "size + 1");
// A literal keeps its Rust width: an untyped `100` is an `i32`.
assert_eq!(lit(100).to_string(), "int32 '100'");

let late: Term = "size >= :floor".parse()?;
assert_eq!(late.parameters(), vec!["floor".to_owned()]);
assert!(late.bind(&schema).is_err(), "a named parameter must be supplied");
let bound = late.bind_with(&schema, &[("floor", Scalar::from(10_i64))])?;
assert_eq!(bound.term().to_string(), "size >= 10");
// An extra entry is ignored and a repeated one takes its first value.
let loose = late.bind_with(
    &schema,
    &[("floor", Scalar::from(10_i64)), ("floor", Scalar::from(99_i64)), ("typo", Scalar::from(1_i64))],
)?;
assert_eq!(loose.term().to_string(), "size >= 10");

// Simplification keeps the answer in fewer nodes.
assert_eq!("a = 1 or a = 2".parse::<Term>()?.simplify().to_string(), "a in (1, 2)");
```

## Filter and project an Arrow batch

`Filter::apply_arrow_batch` keeps rows; a filter that keeps every row hands the
same batch back. `Selector` computes, renames, casts and excludes.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use yggdryl::{DataType, Filter, Selector, StructType};

let root = DataType::from(StructType::from_fields([
    DataType::utf8().nullable_field("ccy"),
    DataType::Int64.nullable_field("size"),
    DataType::utf8().nullable_field("secret"),
])?)
.required_field("rows");
let batch = RecordBatch::try_new(
    root.clone().into_arrow_schema()?,
    vec![
        Arc::new(StringArray::from(vec!["EUR", "USD", "EUR"])),
        Arc::new(Int64Array::from(vec![5_i64, 5, 0])),
        Arc::new(StringArray::from(vec!["a", "b", "c"])),
    ],
)?;

let kept = "ccy = 'EUR' and size > 1".parse::<Filter>()?.apply_arrow_batch(&batch)?;
assert_eq!(kept.num_rows(), 1);
assert_eq!(Filter::always_true().apply_arrow_batch(&batch)?.num_rows(), 3);

let selector: Selector = "* exclude (secret), size * 2 as doubled int32".parse()?;
let projected = selector.apply_arrow_batch(&batch)?;
let names: Vec<String> = projected.schema().fields().iter().map(|field| field.name().clone()).collect();
assert_eq!(names, ["ccy", "size", "doubled"]);
// The published schema is known without data.
assert_eq!(selector.apply_field(&root)?.fields()[2].dtype(), &DataType::Int32);
assert!(Selector::all().bind(&root)?.is_identity());
```

## Shape a stream with a plan

`apply_arrow_reader` binds once and wraps the stream lazily; only `order by`
collects. A `;` sequence applies its steps in order.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use yggdryl::expression::Plan;
use yggdryl::{DataType, Expression, StructType};

let root = DataType::from(StructType::from_fields([
    DataType::utf8().nullable_field("ccy"),
    DataType::Int64.nullable_field("size"),
])?)
.required_field("rows");
let batch = RecordBatch::try_new(
    root.clone().into_arrow_schema()?,
    vec![
        Arc::new(StringArray::from(vec!["A", "B", "C", "D"])),
        Arc::new(Int64Array::from(vec![Some(1_i64), Some(4), Some(3), None])),
    ],
)?;

let plan: Plan = "select ccy, size as quantity where size >= 2 order by size desc limit 1".parse()?;
let reader = plan.apply_arrow_reader(yggdryl::arrow::batch_reader(batch.schema(), [batch.clone()]))?;
// The output schema is known before the first batch is pulled.
assert_eq!(reader.schema().field(1).name(), "quantity");
let kept: usize = reader.map(|batch| batch.map(|batch| batch.num_rows())).sum::<Result<usize, _>>()?;
assert_eq!(kept, 1);

let steps: Expression = "where size is not null; select upper(ccy) as ccy".parse()?;
assert_eq!(steps.steps().len(), 2);
assert_eq!(steps.apply_arrow_batch(&batch)?.num_rows(), 3);
```

## Evaluate native rows

`apply_records` binds once against the schema (or the first record's own
datatype) and streams canonical rows; named input is a sorted `Scalar::Struct`.

```rust
use yggdryl::{DataType, Expression, Scalar, StructType};

let root = DataType::from(StructType::from_fields([
    DataType::utf8().nullable_field("ccy"),
    DataType::Int64.nullable_field("size"),
])?)
.required_field("rows");
let rows = [
    Scalar::from_struct([("ccy", Scalar::from("a")), ("size", Scalar::from(1_i64))])?,
    Scalar::from_struct([("ccy", Scalar::from("b")), ("size", Scalar::from(2_i64))])?,
];

let steps: Expression = "where size > 1; select upper(ccy) as ccy".parse()?;
let records = steps.apply_records(Some(&root), rows)?;
assert_eq!(records.field().fields()[0].name(), "ccy");
assert_eq!(records.collect_rows()?, vec![Scalar::from_sequence([Scalar::from("B")])]);
```

## Use the bound tiers directly: mask, filter, reader

A `Bound` answers one row (`eval`, `matches`), one batch (`evaluate`,
`filter_mask`, `filter`) and one stream (`filter_reader`) from the same
compiled tree - the shape for a hand-written batch loop.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use yggdryl::expression::Term;
use yggdryl::Field;

let schema = "trades:struct<ccy:utf8,size:bigint>".parse::<Field>()?.with_nullable(false);
let bound = "ccy = 'EUR' and size > 10".parse::<Term>()?.bind(&schema)?;
assert_eq!(bound.column_indices(), vec![0, 1]);

let arrow_schema = schema.into_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
    vec![
        Arc::new(StringArray::from(vec!["EUR", "USD"])),
        Arc::new(Int64Array::from(vec![25_i64, 25])),
    ],
)?;
assert_eq!(bound.evaluate(&batch)?.len(), 2);
assert_eq!(bound.filter_mask(&batch)?.true_count(), 1);
assert_eq!(bound.filter(&batch)?.num_rows(), 1);

let filtered = bound.filter_reader(yggdryl::arrow::batch_reader(arrow_schema, [batch.clone(), batch]));
let kept: usize = filtered.map(|batch| batch.map(|batch| batch.num_rows())).sum::<Result<usize, _>>()?;
assert_eq!(kept, 2);
```

## Skip a file from its statistics

`statistics_prune` answers `false` only when no row can match the minimums,
maximums and null counts a footer or manifest carries; anything unprovable
answers `true` and the file is read.

```rust
use yggdryl::expression::{Bounds, Term};
use yggdryl::{Field, Scalar};

let schema: Field = "trades:struct<ccy:utf8,size:bigint>".parse()?;
let bounds = Bounds::new(Some(1_000))
    .with_column("ccy", Some(Scalar::from("EUR")), Some(Scalar::from("USD")), Some(0))
    .with_column("size", Some(Scalar::from(1_i64)), Some(Scalar::from(99_i64)), Some(4));

assert!(!"size > 1000".parse::<Term>()?.bind(&schema)?.statistics_prune(&bounds));
assert!("size > 50".parse::<Term>()?.bind(&schema)?.statistics_prune(&bounds));
assert_eq!("size > 1000".parse::<Term>()?.bind(&schema)?.statistics_certainty(&bounds), Some(false));
assert!("size is null".parse::<Term>()?.bind(&schema)?.statistics_prune(&bounds));
```

## Split a predicate into partition and row halves

`partition_split` separates the conjuncts that read only partition columns
from the residual over rows; dropping a conjunct only widens what is kept.

```rust
use yggdryl::expression::Term;
use yggdryl::{DataType, Field, StructType};

let mut schema: Field = "trades:struct<year:int32,price:decimal(9,2)>".parse()?;
let mut children = schema.fields().to_vec();
children[0].set_partition(true);
schema.set_dtype(DataType::from(StructType::from_fields(children)?))?;

let residual = "year = 2024 and price > 100".parse::<Term>()?.bind(&schema)?.partition_split();
assert_eq!(residual.answerable().to_string(), "year = int32 '2024'");
assert_eq!(residual.remaining().to_string(), "price > decimal32(9,2) '100.00'");
assert!(!residual.is_complete());
```

## Push the filter and projection into a read

`with_filter` and `with_select` are sections of the record options: the media
prunes by path, statistics and columns, and the rows answer the rest - no
batch the filter drops is handed to you.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use yggdryl::holder::Buffer;
use yggdryl::media::IORecordOptions;
use yggdryl::{DataType, IOBase, IOMedia, MimeType, StructType};

let field = DataType::from(StructType::from_fields([
    DataType::utf8().nullable_field("ccy"),
    DataType::Int64.nullable_field("size"),
])?)
.required_field("row");
let schema = field.into_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&schema),
    vec![
        Arc::new(StringArray::from(vec!["EUR", "USD", "EUR"])),
        Arc::new(Int64Array::from(vec![1_i64, 5, 9])),
    ],
)?;
let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
let options = handle.record_options()?;
handle.overwrite_arrow_reader(yggdryl::arrow::batch_reader(schema, [batch]), &options)?;

let pushed = options.clone().with_filter("size > 2")?.with_select("ccy, size * 2 as doubled")?;
assert_eq!(pushed.plan().to_string(), "select ccy, size * 2 as doubled where size > 2");
let mut rows = 0;
for batch in handle.read_arrow_reader(&pushed)? {
    let batch = batch?;
    assert_eq!(batch.schema().field(1).name(), "doubled");
    rows += batch.num_rows();
}
assert_eq!(rows, 2);
```

## Run a plan against storage

`execute` reads the `from` target through `Holder::from_url` with the read
sections pushed down, and a write verb sends the shaped stream to its target.
A store that is not there yet reads as the empty stream.

```rust
use std::sync::Arc;

use arrow_array::{Array, Int64Array, RecordBatch, StringArray};
use yggdryl::expression::Plan;
use yggdryl::{DataType, StructType, Url};

let root = std::env::temp_dir().join(format!("ygg-skill-expr-plan-{}", std::process::id()));
std::fs::create_dir_all(&root)?;
let url = Url::from_path(root.join("trades.arrow"))?;

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

let read: Plan = format!("select name from '{url}' where id > 1 order by id desc").parse()?;
let mut names = Vec::new();
for batch in read.execute()? {
    let batch = batch?;
    let column = batch.column(0).as_any().downcast_ref::<StringArray>().expect("utf8");
    names.extend((0..column.len()).map(|row| column.value(row).to_owned()));
}
assert_eq!(names, ["c", "b"]);

// Every alias prints its canonical verb.
let upsert: Plan = "merge into t on (id) select id from s".parse()?;
assert_eq!(upsert.to_string(), "upsert into t by (id) select id from s");
// A plan is also built section by section.
let built = Plan::new().select("id, name")?.filter("id > 1")?.limit(Some(10));
assert_eq!(built.to_string(), "select id, name where id > 1 limit 10");
std::fs::remove_dir_all(&root)?;
```

## Address a nested value by path

A `FieldPath` is parsed once at its boundary and applied many times; quoting
makes a dotted name one child. It holds every step - child, position, key,
slice `[1:3]` (`FieldSegment::range`) and predicate segment `[ccy = 'EUR']`
(`FieldSegment::filter`) - and inside a term the same steps are accessors.

```rust
use yggdryl::expression::Term;
use yggdryl::{DataType, Field, FieldPath, FieldSegment, Scalar};

let root: Field = "orders:struct<line:serie<struct<ccy:utf8,price:int64>>>".parse()?;
let line = |ccy: &str, price: i64| Scalar::from_sequence([Scalar::from(ccy), Scalar::from(price)]);
let row = Scalar::from_sequence([Scalar::from_sequence([line("EUR", 10), line("USD", 12), line("EUR", 14)])]);

let path = FieldPath::from_str("line[-1].price as last_price")?;
assert_eq!(path.len(), 3);
assert_eq!(path.column_name(), Some("last_price"));
assert_eq!(path.apply_field(&root)?.dtype(), &DataType::Int64);
assert_eq!(path.apply_scalar(&root, &row)?, Scalar::from(14_i64));

let built = FieldPath::new([FieldSegment::field("line"), FieldSegment::index(0)]);
assert_eq!(built.to_string(), "line[0]");
assert_eq!(FieldPath::from_str("\"a.b\"")?.len(), 1);
assert_eq!(FieldPath::from_str("a.b")?.len(), 2);
assert_eq!(FieldPath::from_str("line[0:2]")?.len(), 2);
let eur = FieldPath::new([FieldSegment::field("line"), FieldSegment::filter("ccy = 'EUR'".parse()?)]);
assert_eq!(eur.to_string(), "line[ccy = 'EUR']");

let first_eur = "line[ccy = 'EUR'][0].price".parse::<Term>()?.bind(&root)?;
assert_eq!(first_eur.eval(&row)?, Scalar::from(10_i64));
```

## Register a user-defined function

A `namespace.name(...)` call is typed and called through its registered
`FunctionSignature`; the statistics tier never learns it, so a `where` over it
reads rows. Registration is process-wide and the latest one wins.

```rust
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Int64Array, RecordBatch};
use yggdryl::expression::{FunctionSignature, UserFunction, UserRef, register_function, unregister_function};
use yggdryl::{DataType, Filter, Result, Scalar, Selector, StructType};

struct Triple(FunctionSignature);

impl UserFunction for Triple {
    fn signature(&self) -> &FunctionSignature {
        &self.0
    }

    fn call(&self, arguments: &[Scalar]) -> Result<Scalar> {
        Ok(Scalar::from(arguments[0].as_i64().unwrap_or_default() * 3))
    }
}

let signature = FunctionSignature::new(
    UserRef::new("skills", "triple")?,
    [DataType::Int64.required_field("value")],
    DataType::Int64.nullable_field("returns"),
)?;
// The signature is a struct field, and reads back losslessly.
assert_eq!(FunctionSignature::from_field(&signature.as_field()?)?.reference().as_str(), "skills.triple");
register_function(Arc::new(Triple(signature)))?;

let rows = DataType::from(StructType::from_fields([DataType::Int64.nullable_field("size")])?).required_field("rows");
let batch = RecordBatch::try_from_iter([(
    "size",
    Arc::new(Int64Array::from(vec![Some(1), None, Some(3)])) as ArrayRef,
)])?;
let tripled = "skills.triple(size) as tripled".parse::<Selector>()?.apply_arrow_batch(&batch)?;
assert_eq!(tripled.column(0).as_ref(), &Int64Array::from(vec![Some(3), None, Some(9)]) as &dyn Array);
assert_eq!("skills.triple(size) > 3".parse::<Filter>()?.apply_arrow_batch(&batch)?.num_rows(), 1);

// A stored column records the derivation as TRANSFORM:function over TRANSFORM:sources.
let stored = "skills.triple(size) as tripled".parse::<Selector>()?.into_field(&rows)?;
assert_eq!(stored.fields()[0].get_metadata("TRANSFORM:function"), Some("skills.triple"));
assert_eq!(stored.fields()[0].get_metadata("TRANSFORM:sources"), Some(r#"["size"]"#));

assert!(unregister_function(&UserRef::parse("skills.triple")?));
assert!("skills.triple(size)".parse::<yggdryl::expression::Term>()?.field(&rows).is_err());
```

## Keep a derivation on the schema

`into_field` writes a selector as the declaration it is - each computed
column carrying `TRANSFORM:` metadata - and `from_field` reads it back, so a
`Field` is a plan holder. `Field::apply_arrow_batch` (and
`apply_arrow_reader`) recomputes the derivations on a batch; keep the source
columns in the selector, because the stored field is what the recompute reads.

```rust
use std::sync::Arc;

use arrow_array::{Array, ArrayRef, Int32Array, Int64Array, RecordBatch, StringArray};
use yggdryl::{ArrowCastOptions, DataType, Selector, StructType};

let root = DataType::from(StructType::from_fields([
    DataType::utf8().nullable_field("ccy"),
    DataType::Int64.nullable_field("size"),
])?)
.required_field("rows");
let selector: Selector = "ccy, size * 2 as doubled int32".parse()?;
let stored = selector.into_field(&root)?;
assert_eq!(stored.fields()[1].get_metadata("TRANSFORM:expression"), Some("size * 2"));
let declared: Selector = "ccy utf8 null, size * 2 as doubled int32 null".parse()?;
assert_eq!(Selector::from_field(&stored), declared);

// Recompute: the derived column may arrive absent; the transform fills it.
let holder = "ccy, size, size * 2 as doubled int32".parse::<Selector>()?.into_field(&root)?;
let batch = RecordBatch::try_from_iter([
    ("ccy", Arc::new(StringArray::from(vec!["EUR", "USD"])) as ArrayRef),
    ("size", Arc::new(Int64Array::from(vec![3_i64, 4])) as ArrayRef),
])?;
let applied = holder.apply_arrow_batch(&batch, true, true, true, ArrowCastOptions::new())?;
let doubled = applied.column(2).as_any().downcast_ref::<Int32Array>().ok_or("int32")?;
assert_eq!(doubled.values().to_vec(), vec![6, 8]);
```

## Read the plan, the text and the document

Every layer prints canonical text that re-parses to the same tree, serializes
to a tagged JSON document, and draws its bound plan with `explain`.

```rust
use yggdryl::expression::{Plan, Term};
use yggdryl::{Expression, Field, Selector};

let schema: Field = "trades:struct<ccy:utf8,price:decimal(9,2)>".parse()?;
let drawn = "price > 100".parse::<Term>()?.bind(&schema)?.explain();
assert!(drawn.starts_with('>') && drawn.contains("column price"));

let plan: Plan = "select ccy from t where price > 0 limit 10 offset 5".parse()?;
assert_eq!(plan.to_string().parse::<Plan>()?, plan);
assert_eq!(plan.read_columns(), Some(vec!["price".to_owned(), "ccy".to_owned()]));
assert_eq!("select * exclude (price)".parse::<Plan>()?.read_columns(), None);

assert_eq!(Selector::all().into_json()?, r#"{"star":true}"#);
assert_eq!(Selector::from_json("{}")?, Selector::all());
assert!("price > 1".parse::<Expression>().is_err(), "an Expression names its clause");
```

## Gotchas in Rust

- `Filter::apply_arrow_batch`, `Selector::apply_arrow_batch` and
  `Filter::apply_scalar` bind on every call. In a loop, call
  `apply_arrow_reader` once, or hold `bind(&schema)?` and call
  `Bound::filter` / `BoundSelector::apply_arrow_batch` per batch.
- `lit(100)` is an `i32` literal and prints `int32 '100'`; use `lit(100_i64)`
  for the bare `100` the parser reads.
- `"a > 1".parse::<Expression>()` fails - an `Expression` names its clause;
  parse a `Filter` or a `Term` for a bare predicate.
- A row `Scalar` is ordered in schema order (`Scalar::from_sequence`); a
  `Scalar::from_struct` is named input canonicalized by `apply_records`.
- `Plan::execute` needs the `parquet` feature to read `.parquet` sources and
  `s3` for object-store URLs.
- `statistics_prune` returning `true` means "must read", never "matches".
- A `Plan` answers only `apply_arrow_reader`, `execute`, `field_from` and
  `apply_datatype`; `Expression::from(plan)` reaches `apply_arrow_batch`,
  `apply_records` and `apply_field`.
- `bind_with` ignores an entry no `:name` reads; compare the keys against
  `parameters()` when a typo must fail.
- An untyped `9.5` literal against a `decimal` column shares no type with
  it and compares as text; write `decimal(9,2) '9.50'` or bind a decimal
  `Scalar`.
