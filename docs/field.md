# Fields

The `yggdryl-field` crate is the Apache Arrow-centralized **field layer**, built
on [`yggdryl-dtype`](dtype.md) — the second of the three data layers, each
concern its own crate, so the concrete types share one naming convention across
the layers (`yggdryl_field::Int64Field` names a column of the
`yggdryl_dtype::Int64Type` type, whose single value is a
[`yggdryl_scalar::Int64Scalar`](scalar.md)). A field pairs a **name** with a
**data type** and a **nullability flag** — exactly the three properties of an
Apache Arrow `Field` — so a schema is a sequence of fields.

The bindings expose the layer as `yggdryl.field` (Python and Node), adapting to
idioms: `nullable` defaults to `True` / `true` as a keyword / optional argument.
The concrete integer serie fields (`Int8SerieField` … `UInt64SerieField`, each a
column of its serie type) cross too. Two things stay **Rust-only**, stated here
and in both binding module docs: the [Arrow interop](#arrow-interop) surface
(`to_arrow` / `from_arrow`, and `cast_dtype` which returns a re-typed
`arrow-schema` field — all exchange `arrow-schema` values that cannot cross the
FFI boundary), and the dynamic-base and typed nested fields (`SerieField` /
`TypedSerieField` over a non-integer value type, `MapField` / `TypedMapField`,
`StructField`), which have no concrete FFI shape yet.

## Fields pair a name with a data type

The fixed-shape families default their data type; the optional fields wrap it
(`OptionalInt64Field` is a field of the logical `optional` of `int64`); the
parameterised `StructField` and `UnionField` take theirs at construction.

=== "Python"

    ```python
    from yggdryl import field

    id_field = field.Int64Field("id", False)
    assert (id_field.name(), id_field.is_nullable()) == ("id", False)
    assert id_field.data_type().name() == "int64"
    assert field.Int64Field("maybe").is_nullable() is True  # nullable by default

    score = field.OptionalInt64Field("score")
    assert score.data_type().name() == "optional"
    assert score.data_type().value_type().name() == "int64"

    payload = field.BinaryField("payload")
    assert payload.data_type().name() == "binary"

    scores = field.Int64SerieField("scores")
    assert scores.data_type().name() == "list"
    assert scores.data_type().value_type().name() == "int64"
    ```

=== "Node"

    ```js
    const { field } = require('yggdryl')

    const idField = new field.Int64Field('id', false)
    assert.deepEqual([idField.name(), idField.isNullable()], ['id', false])
    assert.equal(idField.dataType().name(), 'int64')
    assert.equal(new field.Int64Field('maybe').isNullable(), true) // nullable by default

    const score = new field.OptionalInt64Field('score')
    assert.equal(score.dataType().name(), 'optional')
    assert.equal(score.dataType().valueType().name(), 'int64')

    const payload = new field.BinaryField('payload')
    assert.equal(payload.dataType().name(), 'binary')

    const scores = new field.Int64SerieField('scores')
    assert.equal(scores.dataType().name(), 'list')
    assert.equal(scores.dataType().valueType().name(), 'int64')
    ```

=== "Rust"

    ```rust
    use yggdryl_field::yggdryl_dtype::{DataType, Int64Type, TypedSerie, TypedOptional};
    use yggdryl_field::{BinaryField, Field, Int64Field, TypedSerieField, TypedOptionalField};

    fn main() {
        let id = Int64Field::new("id", false);
        assert_eq!((id.name(), id.is_nullable()), ("id", false));
        assert_eq!(id.data_type().name(), "int64");

        let score = TypedOptionalField::<Int64Type>::new("score", true);
        assert_eq!(score.data_type().name(), "optional");
        assert_eq!(score.data_type().value_type().name(), "int64");

        let payload = BinaryField::new("payload", true);
        assert_eq!(payload.data_type().name(), "binary");

        let scores = TypedSerieField::<Int64Type>::new("scores", true);
        assert_eq!(scores.data_type().name(), "list");
        assert_eq!(scores.data_type().value_type().name(), "int64");
    }
    ```

## The data type builds its field

A typed data type *is* the field factory: `data_type.field(name, nullable)` builds
the matching field, so a schema can be assembled straight from the types
(`FieldFactory` in Rust, a method on every `yggdryl.dtype` type in the bindings).

=== "Python"

    ```python
    from yggdryl import dtype

    id_field = dtype.Int64Type().field("id", False)
    assert (id_field.name(), id_field.is_nullable()) == ("id", False)
    assert id_field.data_type().name() == "int64"
    ```

=== "Node"

    ```js
    const { dtype } = require('yggdryl')

    const idField = new dtype.Int64Type().field('id', false)
    assert.deepEqual([idField.name(), idField.isNullable()], ['id', false])
    assert.equal(idField.dataType().name(), 'int64')
    ```

=== "Rust"

    ```rust
    use yggdryl_field::yggdryl_dtype::{DataType, Int64Type};
    use yggdryl_field::{Field, FieldFactory};

    fn main() {
        let id = Int64Type.field("id", false);
        assert_eq!((id.name(), id.is_nullable()), ("id", false));
        assert_eq!(id.data_type().name(), "int64");
    }
    ```

In the bindings the `nullable` argument is optional and defaults to nullable —
`data_type.field(name)` builds a nullable field, matching the `Field`
constructor's own default — while Rust passes the flag explicitly.

The `union` field takes its parameterised data type (reached in the bindings
through an optional data type's `storage()`):

=== "Python"

    ```python
    from yggdryl import dtype, field

    union = dtype.Int64Type().optional().storage()
    value = field.UnionField("value", union)
    assert value.data_type().arrow_format() == "+us:0,1"
    ```

=== "Node"

    ```js
    const { dtype, field } = require('yggdryl')

    const union = new dtype.Int64Type().optional().storage()
    const value = new field.UnionField('value', union)
    assert.equal(value.dataType().arrowFormat(), '+us:0,1')
    ```

=== "Rust"

    ```rust
    use yggdryl_field::yggdryl_dtype::{self as dtype, DataType, Int64Type};
    use yggdryl_field::{Field, UnionField};

    fn main() {
        let value = UnionField::new("value", dtype::UnionType::optional(&Int64Type), true);
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

`cast_dtype(dtype)` re-types a field — a field carries no value, so casting only
swaps its data type, keeping the name and nullability — and returns the mirroring
`arrow_schema::Field` (rehydrate it with the target field's `from_arrow`), the
field-layer counterpart of `Scalar::cast_dtype`.

```rust
use yggdryl_field::yggdryl_dtype::UInt8Type;
use yggdryl_field::{arrow_schema, Field, Int64Field, UInt8Field};

fn main() {
    // Field ↔ arrow_schema::Field.
    let id = Int64Field::new("id", false);
    assert_eq!(Int64Field::from_arrow(&id.to_arrow()).unwrap(), id);

    // cast_dtype re-types the field, keeping name + nullability.
    let cast = id.cast_dtype(&UInt8Type);
    assert_eq!((cast.name(), cast.is_nullable()), ("id", false));
    assert_eq!(cast.data_type(), &arrow_schema::DataType::UInt8);

    // A heterogeneous set of fields converts straight into an Arrow schema.
    let schema = arrow_schema::Schema::new(vec![
        id.to_arrow(),
        UInt8Field::new("flags", true).to_arrow(),
    ]);
    assert_eq!(schema.field(0).data_type(), &arrow_schema::DataType::Int64);
}
```

## The trait layers

- **`Field`** — the untyped base: a named, nullable column
  (`name`, `data_type`, `is_nullable`); `to_arrow` / `from_arrow` mirror an
  `arrow_schema::Field`. Carries its data type as the associated `DataType` so the
  concrete type is preserved for zero-cost access; `Debug + Send + Sync`, no
  lifetime parameters.
- **`TypedField<DT: TypedDataType<T>, T>: Field<DataType = DT>`** — the typed layer: a field
  whose data type is a `yggdryl_dtype::TypedDataType<T>`, so the field's values
  have native Rust representation `T`.
- **`FieldFactory<T>: TypedDataType<T>`** — the factory: a typed data type builds
  its field (`Int64Type.field("id", false)` → `Int64Field`). The dynamic
  `StructType` and `UnionType`, which are not typed data types, have no factory —
  their fields are constructed directly from a data type instance.
```
