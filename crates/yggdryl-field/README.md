# yggdryl-field

The Apache Arrow-centralized **field layer** for yggdryl, built on
`yggdryl-dtype`. It defines the fields of the model — named, nullable columns of a
data type — designed for zero-copy FFI and Arrow interop. It is the second of the
three data layers (`yggdryl-dtype`, `yggdryl-field`, `yggdryl-scalar`), each
concern its own crate, so the concrete types share one naming convention across the
layers (a `yggdryl_field::Int64Field` names a column of the
`yggdryl_dtype::Int64Type` type, whose single value is a
`yggdryl_scalar::Int64Scalar`).

The layer is two traits plus a factory (one file per item at the crate root), plus
concrete fields grouped into per-family modules mirroring `yggdryl-dtype` (one file
per type): [`integer`](src/integer) holds every signed and unsigned integer field,
and [`binary`](src/binary.rs), [`null`](src/null.rs), [`union`](src/union.rs),
[`optional`](src/optional.rs), [`list`](src/list.rs), [`map`](src/map.rs) and
[`struct`](src/struct.rs) the rest.

## Untyped base

- **`Field<D: DataType>`** — a named, nullable column (`name`, `data_type`,
  `is_nullable`); mirrors an `arrow_schema::Field` (`to_arrow` is defaulted from the
  three accessors). The model carries exactly those three properties: `from_arrow`
  refuses an extension-typed field (`ARROW:extension:name` metadata is a different
  logical type) and deliberately drops any other Arrow metadata, logging a `warn`
  when the `log` cargo feature is on. All fields are `Debug + Send + Sync`
  (schemas are printed and shared across threads / FFI); no lifetime parameters.

## Typed

- **`TypedField<DT: TypedDataType<T>, T>: Field<DT>`** — a field whose data type is
  a `yggdryl_dtype::TypedDataType<T>`, so the field's values have native Rust
  representation `T`.

## Factory

- **`FieldFactory<T>: TypedDataType<T>`** — a typed data type builds its field
  (`Int64Type.field("id", false)` → `Int64Field`), the counterpart of
  `yggdryl-scalar`'s `ScalarFactory`. The dynamic `StructType` and `UnionType` are
  not typed data types and have no factory; their fields are constructed directly.

## The concrete fields

Every field pairs a name with its `yggdryl-dtype` data type under the same naming
convention. The fixed-shape families default their data type
(`Int64Field::new("id", false)`); the parameterised `StructField` and `UnionField`
take theirs (`StructField::new("point", shape, false)`); the generic `ListField<D>`,
`MapField<K, V>` and `OptionalField<D>` carry both trait layers, the typed side
whenever the child types have codecs.

```rust
use yggdryl_field::yggdryl_dtype::Int64Type;
use yggdryl_field::{arrow_schema, Field, FieldFactory, Int64Field};

// A named, nullable column of int64; to_arrow / from_arrow mirror an
// arrow_schema::Field.
let id = Int64Field::new("id", false);
assert_eq!((id.name(), id.is_nullable()), ("id", false));
assert_eq!(Int64Field::from_arrow(&id.to_arrow()).unwrap(), id);

// The data type is the factory: it builds the same field.
assert_eq!(Int64Type.field("id", false), id);

// A heterogeneous set of fields converts straight into an Arrow schema.
let schema = arrow_schema::Schema::new(vec![id.to_arrow()]);
assert_eq!(schema.field(0).data_type(), &arrow_schema::DataType::Int64);
```
