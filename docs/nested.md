# Nested types (struct)

`io::nested` is the third typed family, the sibling of [fixed](fixed.md) and
[variable](var.md) — for **composite** types whose values are built from *other* columns.
Phase one ships the **struct** family (an ordered, named set of heterogeneous child columns);
`list` (Arrow `List`) and `map` follow.

A nested column holds child columns of arbitrary type — including other nested columns — so the
module adds two recursive, type-erased carriers the flat leaf families do not need:

| type | role |
| --- | --- |
| `Column` | the erased **data** column — a **thin enum over the crate's existing Series** (`Serie<T>`, `DecimalSerie`, `ByteSerie`, `FixedSizeSerie`, `NullSerie`, `StructSerie`) that only wraps and delegates |
| `ColumnField` | the erased, recursive **field** descriptor — a leaf (the flat `Field`, reused as `var` does) or a nested field |
| `Value` | the erased **cell value** an erased `Column::get` yields (and a `StructScalar` row is built from) |

`Column` reimplements nothing: every operation — length, the byte codec, equality, and Arrow
conversion — calls the wrapped `Serie`'s own method, and the `Struct` variant recurses into a whole
`StructSerie`. So the core still builds **without** the `arrow` feature (the carriers are plain value
enums), and Arrow **recomposition is zero-copy** wherever the underlying `Serie` is: the fixed and
decimal columns hand back their shared `Arc` buffer through their own `to_arrow_array`.

## StructField — one schema, two Arrow faces

`StructField` is the **single source of truth** for a struct's shape: a name, nullability, ordered
child `ColumnField`s, and [`Headers`](headers.md) metadata. It maps to **both** an Arrow `Field`
(of `Struct` type) *and* an Arrow `Schema` (its children as a top-level schema) — the natural bridge
for a struct column ↔ a `RecordBatch`.

```rust
use yggdryl_core::io::FieldType;
use yggdryl_core::io::fixed::{Field, PrimitiveType};
use yggdryl_core::io::nested::{ColumnField, StructField};

let schema = StructField::new(
    "person",
    vec![
        ColumnField::leaf(Field::new("id", &PrimitiveType::<i64>::new(), false)),
        ColumnField::leaf(Field::new("name", &PrimitiveType::<i32>::new(), true)),
    ],
    true,
);
assert_eq!(schema.name(), "person");
assert_eq!(schema.type_name(), "struct");
assert!(schema.is_struct() && schema.nullable());
assert_eq!(schema.num_fields(), 2);
assert_eq!(schema.index_of("name"), Some(1));
```

It is a value type — equal and hashable by content, so a schema works as a map key — with
`with_field` / `with_metadata` / `with_nullable` builders for one-line immutable updates. A struct
field can itself be a child of another struct (`ColumnField::Struct`), so schemas nest arbitrarily.

## Column — the erased column

Any typed column erases into a `Column` with `From`, so a struct holds heterogeneous children:

```rust
use yggdryl_core::io::fixed::Serie;
use yggdryl_core::io::var::Utf8Serie;
use yggdryl_core::io::nested::Column;

let ids = Column::from(Serie::from_values(&[1i64, 2, 3]));
let names = Column::from(Utf8Serie::from_strs(&[Some("ann"), None, Some("cara")]));
```

A `Column` reports its `len` / `type_id` / `null_count`, hands back a `ColumnField` for a given
name (nullability inferred from whether it holds nulls), and — where a fixed primitive column is
logically a temporal type — `with_field` reinterprets its logical type without touching the bytes
(an `i32` column tagged `date32`, say). A leaf column stores its bytes *erased* (raw little-endian
values, or offsets + data) plus a nameless logical field, so numbers, decimals, temporal, and
fixed-size bytes all share one flat shape.

## StructSerie — a struct column

`StructSerie` is a nullable struct column: one child `Column` per field (all the same length), an
ordered schema, and an optional top-level validity mask (a null struct row).

```rust
use yggdryl_core::io::fixed::Serie;
use yggdryl_core::io::var::Utf8Serie;
use yggdryl_core::io::nested::{Column, StructSerie, Value};

let ids = Column::from(Serie::from_values(&[1i64, 2, 3]));
let names = Column::from(Utf8Serie::from_strs(&[Some("ann"), Some("bo"), None]));
let table = StructSerie::from_named(vec![("id", ids), ("name", names)]).unwrap();

assert_eq!(table.len(), 3);
assert_eq!(table.num_columns(), 2);
assert_eq!(table.field(1).unwrap().name(), "name");

// A row is a `Value::Struct` of per-field erased values; a null row is `Value::Null`.
let Value::Struct(row) = table.get_row(0) else { unreachable!() };
assert_eq!(row.value_named("name").unwrap().bytes(), Some(&b"ann"[..]));

// It round-trips byte-exactly through its own codec (schema + data are self-contained).
assert_eq!(
    StructSerie::deserialize_bytes(&table.serialize_bytes()).unwrap(),
    table,
);
```

Every child column may itself be a `StructSerie`, so struct-of-struct nests to any depth and
round-trips whole.

## Arrow interop

Behind the `arrow` feature, `StructSerie` bridges to Arrow's `StructArray` **and** `RecordBatch`,
recursively — each leaf converting through one generic `ArrayData` (buffers + `DataType`) path, so
numbers, decimals, temporal, fixed-size bytes, utf8, and binary all map with no per-type code. A
struct column *is* a batch of named columns (with `--features arrow`):

```rust
use yggdryl_core::io::fixed::Serie;
use yggdryl_core::io::var::Utf8Serie;
use yggdryl_core::io::nested::{Column, StructSerie};

let ids = Column::from(Serie::from_values(&[1i64, 2, 3]));
let names = Column::from(Utf8Serie::from_strs(&[Some("ann"), Some("bo"), None]));
let table = StructSerie::from_named(vec![("id", ids), ("name", names)]).unwrap();

// A RecordBatch (each field becomes a batch column) and back — byte-exact.
let batch = table.to_record_batch().unwrap();
assert_eq!(batch.num_rows(), 3);
assert_eq!(StructSerie::from_record_batch(&batch).unwrap(), table);

// Or a (nullable) StructArray, for a struct that has null rows.
let array = table.to_arrow_array();
```

A struct with **null rows** has no `RecordBatch` form (a batch has no top-level validity) — convert
it to a nullable `StructArray` instead; `to_record_batch` returns a guided error in that case. An
Arrow type this crate does not model surfaces the same guided error on import.

The same `arrow` feature also completes the leaf families' array interop — `Utf8Serie` /
`BinarySerie` ↔ `StringArray` / `BinaryArray`, and the fixed-size byte columns ↔
`FixedSizeBinaryArray` — so every column type now converts to and from an Arrow array, at both the
typed and the erased-`Column` level. As elsewhere, Arrow types never appear in a public signature;
the mapping is centralized on [`DataTypeId`](types.md).
