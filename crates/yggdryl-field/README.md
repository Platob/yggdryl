# yggdryl-field

The Apache Arrow-centralized **field layer** for yggdryl, built on
`yggdryl-dtype`. It defines the fields of the model — named, nullable columns of a
data type — designed for zero-copy FFI and Arrow interop. It is the second of the
three data layers (`yggdryl-dtype`, `yggdryl-field`, `yggdryl-scalar`), each
concern its own crate, so the concrete types share one bare name across the
layers (`yggdryl_field::Int64` names a column of the `yggdryl_dtype::Int64`
type, whose single value is a `yggdryl_scalar::Int64`).

The layer is two traits (one file per trait at the crate root), plus concrete
fields grouped into per-family modules mirroring `yggdryl-dtype` (one file per
type): [`integer`](src/integer) holds every signed and unsigned integer field,
and [`binary`](src/binary.rs), [`null`](src/null.rs), [`union`](src/union.rs),
[`optional`](src/optional.rs), [`list`](src/list.rs), [`map`](src/map.rs) and
[`struct`](src/struct.rs) the rest.

## Untyped base

- **`RawField<D: RawDataType>`** — a named, nullable column (`name`, `data_type`,
  `is_nullable`); mirrors an `arrow_schema::Field` (`to_arrow` is defaulted from the
  three accessors). The model carries exactly those three properties: `from_arrow`
  refuses an extension-typed field (`ARROW:extension:name` metadata is a different
  logical type) and deliberately drops any other Arrow metadata, logging a `warn`
  when the `log` cargo feature is on. All fields are `Debug + Send + Sync`
  (schemas are printed and shared across threads / FFI); no lifetime parameters.

## Typed

- **`Field<T>: RawField<Self::Type>`** — a field whose data type is a
  `yggdryl_dtype::DataType<T>`, so the field's values have native Rust
  representation `T`.

## The concrete fields

Every field pairs a name with its `yggdryl-dtype` data type under the same bare
name. The fixed-shape families default their data type (`Int64::new("id", false)`);
the parameterised `Struct` and `Union` take theirs (`Struct::new("point", shape,
false)`); the generic `List<D>`, `Map<K, V>` and `Optional<D>` carry both trait
layers, the typed side whenever the child types have codecs.

```rust
use yggdryl_field::{arrow_schema, Int64, RawField};

// A named, nullable column of int64; to_arrow / from_arrow mirror an
// arrow_schema::Field.
let id = Int64::new("id", false);
assert_eq!((id.name(), id.is_nullable()), ("id", false));
assert_eq!(Int64::from_arrow(&id.to_arrow()).unwrap(), id);

// A heterogeneous set of fields converts straight into an Arrow schema.
let schema = arrow_schema::Schema::new(vec![id.to_arrow()]);
assert_eq!(schema.field(0).data_type(), &arrow_schema::DataType::Int64);
```
