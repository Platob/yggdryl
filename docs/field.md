# Fields

The `yggdryl-field` crate is the Apache Arrow-centralized **field layer**, built
on [`yggdryl-dtype`](dtype.md) — the second of the three data layers, each
concern its own crate, so the concrete types share one bare name across the
layers (`yggdryl_field::Int64` names a column of the `yggdryl_dtype::Int64`
type, whose single value is a [`yggdryl_scalar::Int64`](scalar.md)). A field
pairs a **name** with a **data type** and a **nullability flag** — exactly the
three properties of an Apache Arrow `Field` — so a schema is a sequence of
fields.

The bindings expose the layer as `yggdryl.field` (Python and Node), adapting to
idioms: `nullable` defaults to `True` / `true` as a keyword / optional argument.
Two things stay **Rust-only**, stated here and in both binding module docs: the
[Arrow interop](#arrow-interop) surface (`to_arrow` / `from_arrow` exchange
`arrow-schema` values that cannot cross the FFI boundary), and the generic nested
fields (`List<D>` / `Map<K, V>` / `Struct`), which have no concrete FFI shape
yet.

## Fields pair a name with a data type

The fixed-shape families default their data type; the optional fields wrap it
(`OptionalInt64` is a field of the logical `optional` of `int64`); the
parameterised `Struct` and `Union` take theirs at construction.

=== "Python"

    ```python
    from yggdryl import field

    id_field = field.Int64("id", False)
    assert (id_field.name(), id_field.is_nullable()) == ("id", False)
    assert id_field.data_type().name() == "int64"
    assert field.Int64("maybe").is_nullable() is True  # nullable by default

    score = field.OptionalInt64("score")
    assert score.data_type().name() == "optional"
    assert score.data_type().value_type().name() == "int64"

    payload = field.Binary("payload")
    assert payload.data_type().name() == "binary"
    ```

=== "Node"

    ```js
    const { field } = require('yggdryl')

    const idField = new field.Int64('id', false)
    assert.deepEqual([idField.name(), idField.isNullable()], ['id', false])
    assert.equal(idField.dataType().name(), 'int64')
    assert.equal(new field.Int64('maybe').isNullable(), true) // nullable by default

    const score = new field.OptionalInt64('score')
    assert.equal(score.dataType().name(), 'optional')
    assert.equal(score.dataType().valueType().name(), 'int64')

    const payload = new field.Binary('payload')
    assert.equal(payload.dataType().name(), 'binary')
    ```

=== "Rust"

    ```rust
    use yggdryl_field::yggdryl_dtype::{Int64 as Int64Type, RawDataType, RawOptional};
    use yggdryl_field::{Binary, Int64, Optional, RawField};

    fn main() {
        let id = Int64::new("id", false);
        assert_eq!((id.name(), id.is_nullable()), ("id", false));
        assert_eq!(id.data_type().name(), "int64");

        let score = Optional::<Int64Type>::new("score", true);
        assert_eq!(score.data_type().name(), "optional");
        assert_eq!(score.data_type().value_type().name(), "int64");

        let payload = Binary::new("payload", true);
        assert_eq!(payload.data_type().name(), "binary");
    }
    ```

The `union` field takes its parameterised data type (reached in the bindings
through an optional data type's `storage()`):

=== "Python"

    ```python
    from yggdryl import dtype, field

    union = dtype.Int64().optional().storage()
    value = field.Union("value", union)
    assert value.data_type().arrow_format() == "+us:0,1"
    ```

=== "Node"

    ```js
    const { dtype, field } = require('yggdryl')

    const union = new dtype.Int64().optional().storage()
    const value = new field.Union('value', union)
    assert.equal(value.dataType().arrowFormat(), '+us:0,1')
    ```

=== "Rust"

    ```rust
    use yggdryl_field::yggdryl_dtype::{self as dtype, Int64, RawDataType};
    use yggdryl_field::{RawField, Union};

    fn main() {
        let value = Union::new("value", dtype::Union::optional(&Int64), true);
        assert_eq!(value.data_type().arrow_format(), "+us:0,1");
    }
    ```

## Arrow interop

!!! note "Rust only"
    `to_arrow` / `from_arrow` exchange `arrow-schema` values, which cannot cross
    the FFI boundary — the bindings will gain this surface through the Arrow C
    Data Interface as it lands.

Every field converts to and from the `arrow_schema::Field` it mirrors:
`to_arrow` (defaulted from the three accessors) and `from_arrow`, its exact
inverse. Field metadata is handled in two tiers: an extension-typed Arrow field
(one carrying an `ARROW:extension:name` metadata entry) is a *different* logical
type and is refused with `DataError::IncompatibleArrowType`, while any other
metadata is not part of the model — a field is exactly a name, a data type and a
nullability flag — and is deliberately dropped on the way in (logged as a `warn`
when the `log` cargo feature is on; `to_arrow` correspondingly always produces a
metadata-free field).

```rust
use yggdryl_field::{arrow_schema, Int64, RawField, UInt8};

fn main() {
    // Field ↔ arrow_schema::Field.
    let id = Int64::new("id", false);
    assert_eq!(Int64::from_arrow(&id.to_arrow()).unwrap(), id);

    // A heterogeneous set of fields converts straight into an Arrow schema.
    let schema = arrow_schema::Schema::new(vec![
        id.to_arrow(),
        UInt8::new("flags", true).to_arrow(),
    ]);
    assert_eq!(schema.field(0).data_type(), &arrow_schema::DataType::Int64);
}
```

## The trait layers

- **`RawField<D: RawDataType>`** — the untyped base: a named, nullable column
  (`name`, `data_type`, `is_nullable`); `to_arrow` / `from_arrow` mirror an
  `arrow_schema::Field`. Parameterised by the data type `D` so the concrete type
  is preserved for zero-cost access; `Debug + Send + Sync`, no lifetime
  parameters.
- **`Field<T>: RawField<Self::Type>`** — the typed layer: a field whose data type
  is a `yggdryl_dtype::DataType<T>`, so the field's values have native Rust
  representation `T`.
