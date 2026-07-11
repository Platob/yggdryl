# Fields

A **field** is a named, nullable [data type](dtype.md) — the second Arrow data-model
layer (data types → fields → [scalars](scalar.md)). `yggdryl-field` mirrors the dtype layer's trait
shape: the FFI-opaque `Field`, the Rust-only `TypedField<DT, T>`, and the category traits
`PrimitiveField` (+ `LogicalField` / `NestedField` scaffolding).

The concrete **primitive** fields are the ten native numerics (`I8Field` …
`F64Field`) plus `BooleanField`. A field never touches a value, so `Boolean` is not a
special case here — every field is stamped from the same macro. Each reports its `name`,
`nullable` flag, `data_type`, and optional **headers**, round-trips through bytes, and —
in Rust — converts to and from an Arrow `Field`. A [buffer](buffer.md) hands out its
matching field via `field(name, nullable)`.

!!! note "Headers is bytes→bytes and yggdryl-side"
    A field may carry optional headers — a bytes→bytes map (`with_headers` /
    `headers`), marshalled as a Python `dict[bytes, bytes]` or a Node
    `Array<{key: Buffer, value: Buffer}>`. It is part of the field's identity (equality,
    hashing, and the byte codec include it) but is **not** carried into Arrow's `Field`
    (arbitrary bytes are not valid UTF-8), so `to_arrow` omits it.

!!! note "Arrow interop is Rust-only"
    `to_arrow` / `from_arrow` exchange `arrow_schema::Field` values, which do not cross the
    FFI boundary, so they are **not** replicated in the Python and Node bindings (which
    expose the name, nullability, data type, headers, byte codec, and value semantics).

## Construct and inspect

`nullable` defaults to `false`.

=== "Python"

    ```python
    from yggdryl.field import I64Field
    from yggdryl.dtype import I64Type

    field = I64Field("id", True)
    assert field.name == "id"
    assert field.nullable is True
    assert field.data_type == I64Type()
    assert field.data_type.name == "int64"
    ```

=== "Node"

    ```js
    const { I64Field } = require('yggdryl').field
    const { I64Type } = require('yggdryl').dtype

    const field = new I64Field('id', true)
    console.assert(field.name === 'id')
    console.assert(field.nullable === true)
    console.assert(field.dataType.equals(new I64Type()))
    console.assert(field.dataType.name === 'int64')
    ```

=== "Rust"

    ```rust
    use yggdryl_dtype::DataType;
    use yggdryl_field::{Field, I64Field, TypedField};

    let field = I64Field::new("id", true);
    assert_eq!(field.name(), "id");
    assert!(field.is_nullable());
    assert_eq!(TypedField::data_type(&field).name(), "int64");
    ```

## Byte round-trip

A field serialises to a 1-byte nullable flag followed by its UTF-8 name;
`deserialize_bytes` rejects an empty payload with a guided error.

=== "Python"

    ```python
    from yggdryl.field import I64Field

    field = I64Field("mesure_€", True)   # non-ASCII names are fine
    assert I64Field.deserialize_bytes(field.serialize_bytes()) == field

    try:
        I64Field.deserialize_bytes(b"")
    except ValueError as error:
        assert "nullable flag" in str(error)
    ```

=== "Node"

    ```js
    const { I64Field } = require('yggdryl').field

    const field = new I64Field('mesure_€', true)
    console.assert(I64Field.deserializeBytes(field.serializeBytes()).equals(field))

    try {
      I64Field.deserializeBytes(Buffer.alloc(0))
    } catch (error) {
      console.assert(/nullable flag/.test(error.message))
    }
    ```

=== "Rust"

    ```rust
    use yggdryl_field::{Field, I64Field};

    let field = I64Field::new("mesure_€", true);
    assert_eq!(I64Field::deserialize_bytes(&field.serialize_bytes()).unwrap(), field);
    assert!(I64Field::deserialize_bytes(&[]).is_err());
    ```

## Value semantics

Fields compare and hash by content (name + nullability + data type), so they work as
dict / map keys and set members; Python also pickles them.

=== "Python"

    ```python
    import pickle
    from yggdryl.field import I64Field

    a = I64Field("a", True)
    assert a == I64Field("a", True)
    assert a != I64Field("a", False)
    assert hash(a) == hash(I64Field("a", True))
    assert pickle.loads(pickle.dumps(a)) == a
    ```

=== "Node"

    ```js
    const { I64Field } = require('yggdryl').field

    const a = new I64Field('a', true)
    console.assert(a.equals(new I64Field('a', true)))
    console.assert(!a.equals(new I64Field('a', false)))
    console.assert(a.hashCode() === new I64Field('a', true).hashCode())
    ```

=== "Rust"

    ```rust
    use yggdryl_field::I64Field;

    assert_eq!(I64Field::new("a", true), I64Field::new("a", true));
    assert_ne!(I64Field::new("a", true), I64Field::new("a", false));
    ```

## Arrow interop (Rust-only)

=== "Rust"

    ```rust
    use yggdryl_field::{Field, I64Field};
    use arrow_schema::{DataType as ArrowDataType, Field as ArrowField};

    let field = I64Field::new("id", true);
    let arrow: ArrowField = field.to_arrow();
    assert_eq!(arrow.name(), "id");
    assert_eq!(arrow.data_type(), &ArrowDataType::Int64);
    assert_eq!(I64Field::from_arrow(&arrow).unwrap(), field);
    ```
