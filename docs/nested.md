# Nested types (struct)

`io::nested` is the third typed family, the sibling of [fixed](fixed.md) and
[variable](var.md) — for **composite** types whose values are built from *other* columns.
Phase one ships the **struct** family (an ordered, named set of heterogeneous child columns);
`list` (Arrow `List`) and `map` follow.

A nested column holds child columns of arbitrary type — including other nested columns — so
the crate adds three recursive, type-erased carriers at the **`io` root** (not inside
`nested`), because they describe *any* column, leaf or nested:

| type | role |
| --- | --- |
| `AnySerie` | the erased **data** trait — every concrete series (`Serie<T>`, `DecimalSerie`, `ByteSerie`, `FixedSizeSerie`, `NullSerie`, and `StructSerie` itself) implements it, so a nested column holds its children as `Box<dyn AnySerie>` |
| `AnyField` | the erased, recursive **field** descriptor — a `Leaf` (the flat `Field`, reused as `var` does) or a `Struct` carrying its child fields inline |
| `AnyScalar` | the erased **cell value** an `AnySerie::value` yields (and a `StructScalar` row is built from) — `Null`, a `Leaf`, or a `Struct` of child values |

`AnySerie` reimplements nothing: every operation — length, the byte codec, equality, and Arrow
conversion — delegates to the wrapped series' own method, and `StructSerie` (itself an
`AnySerie`) recurses into its children. So the core still builds **without** the `arrow`
feature (the carriers are plain traits/enums over the existing series), and Arrow
**recomposition is zero-copy** wherever the underlying series is: the fixed and decimal columns
hand back their shared `Arc` buffer through their own `to_arrow_array`.

## `boxed` — erase a series, `as_serie::<T>` — recover it

Any concrete series erases into a `Box<dyn AnySerie>` with the free function `boxed`, and comes
back out with a field-keyed safe downcast — no `Column` enum, no per-type wrapper:

```rust
use yggdryl_core::io::fixed::Serie;
use yggdryl_core::io::var::Utf8Serie;
use yggdryl_core::io::boxed;

let ids = boxed(Serie::from_values(&[1i64, 2, 3]));
let names = boxed(Utf8Serie::from_strs(&[Some("ann"), None, Some("cara")]));

// Recover the concrete series by asserting the element type.
let ids: &Serie<i64> = ids.as_serie::<i64>().unwrap();
assert_eq!(ids.get(0), Some(1));
```

`as_serie::<T>` (and its siblings `as_decimal::<B>` / `as_bytes_serie::<E>`) is the `as_ref<T>`
assumption keyed on the linked field: it downcasts to the concrete series if the element type
matches, else `None` — so a caller that knows a column's `AnyField` can safely assume its type.

## `AnyField` — the recursive schema leaf

`AnyField` is the erased field that both a leaf and a struct share. A leaf reuses the flat
`Field` (as `var` does); a struct carries its ordered child `AnyField`s inline, so schemas nest
to any depth without a root→nested dependency:

```rust
use yggdryl_core::io::FieldType;
use yggdryl_core::io::fixed::{Field, PrimitiveType};
use yggdryl_core::io::AnyField;

let id = AnyField::leaf(Field::new("id", &PrimitiveType::<i64>::new(), false));
let person = AnyField::struct_("person", vec![id.clone()], true);
assert!(person.is_struct() && person.nullable());
assert_eq!(person.children()[0].name(), "id");
```

## StructField — one schema, two Arrow faces

`StructField` is the **single source of truth** for a struct's shape — a validated
struct-shaped `AnyField` (its children hold the ordered child fields). It maps to **both** an
Arrow `Field` (of `Struct` type) *and* an Arrow `Schema` (its children as a top-level schema) —
the natural bridge for a struct column ↔ a `RecordBatch`.

```rust
use yggdryl_core::io::FieldType;
use yggdryl_core::io::fixed::{Field, PrimitiveType};
use yggdryl_core::io::AnyField;
use yggdryl_core::io::nested::StructField;

let schema = StructField::new(
    "person",
    vec![
        AnyField::leaf(Field::new("id", &PrimitiveType::<i64>::new(), false)),
        AnyField::leaf(Field::new("name", &PrimitiveType::<i32>::new(), true)),
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
`with_field` / `with_metadata` / `with_nullable` builders for one-line immutable updates. A
struct field can itself be a child of another struct (an `AnyField::Struct` child), so schemas
nest arbitrarily.

## StructSerie — a struct column

`StructSerie` is a nullable struct column: one child `Box<dyn AnySerie>` per field (all the same
length), an ordered schema of `AnyField`s, and an optional top-level validity mask (a null
struct row). It is itself an `AnySerie`, so struct-of-struct nests to any depth.

```rust
use yggdryl_core::io::fixed::Serie;
use yggdryl_core::io::var::Utf8Serie;
use yggdryl_core::io::{boxed, AnyScalar};
use yggdryl_core::io::nested::StructSerie;

let ids = boxed(Serie::from_values(&[1i64, 2, 3]));
let names = boxed(Utf8Serie::from_strs(&[Some("ann"), Some("bo"), None]));
let table = StructSerie::from_named(vec![("id", ids), ("name", names)]).unwrap();

assert_eq!(table.len(), 3);
assert_eq!(table.num_columns(), 2);
assert_eq!(table.field(1).unwrap().name(), "name");

// Downcast a child back to its concrete series, keyed on the field's type.
let ids: &Serie<i64> = table.column(0).unwrap().as_serie::<i64>().unwrap();
assert_eq!(ids.get(1), Some(2));

// A row is an `AnyScalar::Struct` of per-field erased values; a null row is `AnyScalar::Null`.
let AnyScalar::Struct(row) = table.row(0) else { unreachable!() };
assert_eq!(row.len(), 2);

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
use yggdryl_core::io::boxed;
use yggdryl_core::io::nested::StructSerie;

let ids = boxed(Serie::from_values(&[1i64, 2, 3]));
let names = boxed(Utf8Serie::from_strs(&[Some("ann"), Some("bo"), None]));
let table = StructSerie::from_named(vec![("id", ids), ("name", names)]).unwrap();

// A RecordBatch (each field becomes a batch column) and back — byte-exact.
let batch = table.to_record_batch().unwrap();
assert_eq!(batch.num_rows(), 3);
assert_eq!(StructSerie::from_record_batch(&batch).unwrap(), table);

// Or a (nullable) StructArray, for a struct that has null rows.
let array = table.to_arrow_array().unwrap();
```

A struct with **null rows** has no `RecordBatch` form (a batch has no top-level validity) — convert
it to a nullable `StructArray` instead; `to_record_batch` returns a guided error in that case. An
Arrow type this crate does not model surfaces the same guided error on import.

The same `arrow` feature also completes the leaf families' array interop — `Utf8Serie` /
`BinarySerie` ↔ `StringArray` / `BinaryArray`, and the fixed-size byte columns ↔
`FixedSizeBinaryArray` — so every column type now converts to and from an Arrow array, at both the
typed and the erased-`AnySerie` level. As elsewhere, Arrow types never appear in a public signature;
the mapping is centralized on [`DataTypeId`](types.md).

## In Python and Node

Both extensions mirror the nested surface — `StructField` (the schema) and `StructSerie` (the
column) — over the same core, so a struct built in one language **serializes to identical bytes** in
the others. The one platform difference is how a heterogeneous child column is passed in: Python
takes the live `Serie` objects directly, while Node (napi cannot accept an arbitrary one-of-many
class instance) takes a schema plus each child's `serializeBytes()` frame. Both round-trip
byte-for-byte with the Rust core.

=== "Python"

    ```python
    from yggdryl.types import StructField, StructSerie, I64Serie, Utf8Serie

    ids = I64Serie([1, 2, 3])
    names = Utf8Serie(["ann", None, "cara"])
    table = StructSerie([("id", ids), ("name", names)])          # live Serie children
    assert len(table) == 3 and table.num_columns == 2
    assert table.column_named("name").get(0) == "ann"            # re-wrapped, typed child
    assert StructSerie.deserialize_bytes(table.serialize_bytes()) == table

    schema = table.to_field("person")                            # StructField schema
    assert schema.num_fields == 2 and schema.field(1).name == "name"
    ```

=== "Node"

    ```js
    const { StructField, StructSerie, I32Serie, Utf8Serie } = require('yggdryl').types

    const ids = new I32Serie([1, 2, 3])
    const names = new Utf8Serie(['ann', null, 'cara'])
    // A child crosses as a schema field + its serialized bytes.
    const schema = new StructField('person', [ids.toField('id'), names.toField('name')], false)
    const table = StructSerie.fromColumns(schema, [ids.serializeBytes(), names.serializeBytes()])
    assert(table.length === 3 && table.numColumns === 2)
    assert(Utf8Serie.deserializeBytes(table.columnBytesNamed('name')).get(0) === 'ann')
    assert(StructSerie.deserializeBytes(table.serializeBytes()).equals(table))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::boxed;
    use yggdryl_core::io::fixed::Serie;
    use yggdryl_core::io::var::Utf8Serie;
    use yggdryl_core::io::nested::StructSerie;

    let ids = boxed(Serie::from_values(&[1i64, 2, 3]));
    let names = boxed(Utf8Serie::from_strs(&[Some("ann"), None, Some("cara")]));
    let table = StructSerie::from_named(vec![("id", ids), ("name", names)]).unwrap();
    assert_eq!(table.len(), 3);
    let ids: &Serie<i64> = table.column(0).unwrap().as_serie::<i64>().unwrap();
    assert_eq!(ids.get(0), Some(1));
    assert_eq!(StructSerie::deserialize_bytes(&table.serialize_bytes()).unwrap(), table);
    ```

### pyarrow — zero-copy via the Arrow C Data Interface

The Python extension implements the **Arrow PyCapsule protocol** (`__arrow_c_array__` /
`__arrow_c_schema__`), so a `StructSerie` hands its columns to `pyarrow` with **no payload copy**
(the child buffers are shared through the C Data Interface), and `StructSerie.from_arrow(...)` pulls
any C-Data-exposing pyarrow object back the same way.

```python
import pyarrow as pa
from yggdryl.types import StructSerie, I32Serie, Utf8Serie

table = StructSerie([("id", I32Serie([1, 2, 3])), ("name", Utf8Serie(["ann", None, "cara"]))])

arr = pa.array(table)                 # zero-copy import into pyarrow (a StructArray)
assert arr.field(0).to_pylist() == [1, 2, 3]
assert StructSerie.from_arrow(arr) == table          # and back, zero-copy
assert StructSerie.from_arrow(pa.RecordBatch.from_struct_array(arr)) == table
```

The Node extension carries the structural surface and the byte codec; an equivalent Arrow-ecosystem
bridge for apache-arrow JS (which has no C Data Interface consumer in its standard build) is a
separate follow-up.
