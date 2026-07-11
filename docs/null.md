# The null type

`yggdryl` **scalars are always present** — a scalar carries a value, never a null.
Nullability is not a property of a scalar; it is represented by a distinct **null type**,
so "null" is a value like any other. (Column- and union-level nullability will build on
these null values; a scalar itself never goes missing.)

The null type is **sui generis** — it is not a primitive, logical, or nested type, so it
joins none of those category traits — but it appears at every data-model layer, mirroring
the primitives:

- `NullType` (a [data type](dtype.md)) — Arrow `Null`, name `"null"`, a value width of
  **zero bytes**.
- `NullField` (a [field](field.md)) — a named, nullable column of `NullType`.
- `NullScalar` (a [scalar](scalar.md)) — the single value of `NullType`; its value is
  "null" (Python `None` / JS `null`), and it serialises to **zero bytes**.

## The null data type

`NullType` carries the usual type-identity surface — `name`, `byte_width`, the byte codec,
and value semantics — with no core `primitive_tag` (it is outside the numeric tags).

=== "Python"

    ```python
    from yggdryl.dtype import NullType

    dt = NullType()
    assert dt.name == "null"
    assert dt.byte_width == 0          # a null value is zero bytes
    assert dt.primitive_tag is None    # sui generis
    assert NullType.deserialize_bytes(dt.serialize_bytes()) == dt
    ```

=== "Node"

    ```js
    const { NullType } = require('yggdryl').dtype

    const dt = new NullType()
    console.assert(dt.name === 'null')
    console.assert(dt.byteWidth === 0)       // a null value is zero bytes
    console.assert(dt.primitiveTag === null) // sui generis
    console.assert(NullType.deserializeBytes(dt.serializeBytes()).equals(dt))
    ```

=== "Rust"

    ```rust
    use yggdryl_dtype::{DataType, NullType, TypedDataType};
    use arrow_schema::DataType as ArrowDataType;

    let dt = NullType::new();
    assert_eq!(dt.name(), "null");
    assert_eq!(dt.byte_width(), Some(0));
    assert_eq!(dt.to_arrow(), ArrowDataType::Null);
    // The unit value encodes to zero bytes.
    assert!(dt.value_to_bytes(()).is_empty());
    assert_eq!(dt.value_from_bytes(&[]).unwrap(), ());
    ```

## The null field

`NullField` is a named, nullable field whose data type is `NullType` — the same
`name` / `nullable` / `data_type` / headers / byte-codec surface as a primitive field.

=== "Python"

    ```python
    from yggdryl.field import NullField
    from yggdryl.dtype import NullType

    f = NullField("maybe", True)
    assert f.name == "maybe"
    assert f.nullable is True
    assert f.data_type == NullType()
    assert NullField.deserialize_bytes(f.serialize_bytes()) == f
    ```

=== "Node"

    ```js
    const { NullField } = require('yggdryl').field
    const { NullType } = require('yggdryl').dtype

    const f = new NullField('maybe', true)
    console.assert(f.name === 'maybe')
    console.assert(f.nullable === true)
    console.assert(f.dataType.equals(new NullType()))
    console.assert(NullField.deserializeBytes(f.serializeBytes()).equals(f))
    ```

=== "Rust"

    ```rust
    use yggdryl_dtype::DataType;
    use yggdryl_field::{Field, NullField, TypedField};

    let f = NullField::new("maybe", true);
    assert_eq!(f.name(), "maybe");
    assert!(f.is_nullable());
    assert_eq!(TypedField::data_type(&f).name(), "null");
    ```

## The null scalar

`NullScalar` is the single value of the null type — a present scalar whose value is "null".
It takes no argument, its `value` is `None` / `null`, and it serialises to zero bytes.

=== "Python"

    ```python
    from yggdryl.scalar import NullScalar
    from yggdryl.dtype import NullType

    s = NullScalar()
    assert s.value is None          # the null value
    assert s.data_type == NullType()
    assert s.serialize_bytes() == b""
    assert NullScalar.deserialize_bytes(b"") == s
    assert s == NullScalar()         # all null scalars are equal
    ```

=== "Node"

    ```js
    const { NullScalar } = require('yggdryl').scalar
    const { NullType } = require('yggdryl').dtype

    const s = new NullScalar()
    console.assert(s.value === null)                 // the null value
    console.assert(s.dataType.equals(new NullType()))
    console.assert(s.serializeBytes().length === 0)
    console.assert(NullScalar.deserializeBytes(s.serializeBytes()).equals(s))
    console.assert(s.equals(new NullScalar()))        // all null scalars are equal
    ```

=== "Rust"

    ```rust
    use yggdryl_scalar::{NullScalar, Scalar, TypedScalar};

    let s = NullScalar::new();
    assert_eq!(s.value(), ());            // the unit value
    assert!(s.serialize_bytes().is_empty());
    assert_eq!(NullScalar::deserialize_bytes(&[]).unwrap(), s);
    assert_eq!(NullScalar::new(), NullScalar::new());
    ```
