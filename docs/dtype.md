# Data types

A **data type** describes an Apache Arrow type. `yggdryl-dtype` is the first of the
Arrow data-model layers (data types → [fields](field.md) → [scalars](scalar.md)), built as a small
trait hierarchy — the FFI-opaque `DataType`, the Rust-only value-typed
`TypedDataType<T>`, and the category traits `PrimitiveType` (+ `LogicalType` /
`NestedType` scaffolding for the types still to come).

The concrete **primitive** types are the ten native numerics (`I8Type` …
`F64Type`) plus the bit-packed `BooleanType`. Each reports its `name` and
`byte_width`, round-trips through bytes, and — in Rust — converts to and from its Arrow
`DataType`.

!!! note "Canonical typing"
    The `PrimitiveType` trait here is the canonical primitive-typing API for every layer
    above the core. The core's `PrimitiveType` *enum* (the converter's runtime tag) stays
    as the low-level FFI tag and interoperates through `primitive_tag` — e.g.
    `I64Type` ↔ `"i64"`. `BooleanType` has no core tag (it is bit-packed, outside the
    core enum's ten numerics), so its `primitive_tag` is `None`.

!!! note "Arrow interop is Rust-only"
    `to_arrow` / `from_arrow` exchange `arrow_schema::DataType` values, which do not cross
    the FFI boundary, so they are **not** replicated in the Python and Node bindings
    (exactly like the buffers' `from_arrow` / `to_arrow`). The bindings expose the name,
    width, tag, byte codec, and value semantics.

## Construct and inspect

=== "Python"

    ```python
    from yggdryl.dtype import I64Type, BooleanType

    dt = I64Type()
    assert dt.name == "int64"
    assert dt.byte_width == 8
    assert dt.primitive_tag == "i64"

    assert BooleanType().byte_width is None      # bit-packed
    assert BooleanType().primitive_tag is None
    ```

=== "Node"

    ```js
    const { I64Type, BooleanType } = require('yggdryl').dtype

    const dt = new I64Type()
    console.assert(dt.name === 'int64')
    console.assert(dt.byteWidth === 8)
    console.assert(dt.primitiveTag === 'i64')

    console.assert(new BooleanType().byteWidth === null) // bit-packed
    console.assert(new BooleanType().primitiveTag === null)
    ```

=== "Rust"

    ```rust
    use yggdryl_dtype::{BooleanType, DataType, I64Type, PrimitiveType};

    let dt = I64Type::new();
    assert_eq!(dt.name(), "int64");
    assert_eq!(dt.byte_width(), Some(8));
    assert_eq!(dt.primitive_tag(), Some(yggdryl_core::PrimitiveType::I64));

    assert_eq!(BooleanType::new().byte_width(), None); // bit-packed
    assert_eq!(BooleanType::new().primitive_tag(), None);
    ```

## Byte round-trip

A primitive data type is a value-free marker, so its serialised payload is empty;
`deserialize_bytes` rejects any non-empty payload with a guided error.

=== "Python"

    ```python
    from yggdryl.dtype import I32Type

    dt = I32Type()
    assert dt.serialize_bytes() == b""
    assert I32Type.deserialize_bytes(dt.serialize_bytes()) == dt

    try:
        I32Type.deserialize_bytes(b"\x01")
    except ValueError as error:
        assert "carries no parameters" in str(error)
    ```

=== "Node"

    ```js
    const { I32Type } = require('yggdryl').dtype

    const dt = new I32Type()
    console.assert(dt.serializeBytes().length === 0)
    console.assert(I32Type.deserializeBytes(dt.serializeBytes()).equals(dt))

    try {
      I32Type.deserializeBytes(Buffer.from([1]))
    } catch (error) {
      console.assert(/carries no parameters/.test(error.message))
    }
    ```

=== "Rust"

    ```rust
    use yggdryl_dtype::{DataType, I32Type};

    let dt = I32Type::new();
    assert!(dt.serialize_bytes().is_empty());
    assert_eq!(I32Type::deserialize_bytes(&dt.serialize_bytes()).unwrap(), dt);
    assert!(I32Type::deserialize_bytes(&[1]).is_err());
    ```

## Value semantics

Data types compare and hash by content (all instances of a type are equal), so they work
as dict / map keys and set members; Python also pickles them.

=== "Python"

    ```python
    import pickle
    from yggdryl.dtype import I64Type, F64Type

    assert I64Type() == I64Type()
    assert hash(I64Type()) == hash(I64Type())
    assert I64Type() != F64Type()
    assert len({I64Type(), I64Type(), F64Type()}) == 2
    assert pickle.loads(pickle.dumps(I64Type())) == I64Type()
    ```

=== "Node"

    ```js
    const { I64Type } = require('yggdryl').dtype

    const a = new I64Type()
    const b = new I64Type()
    console.assert(a.equals(b))
    console.assert(a.hashCode() === b.hashCode())
    ```

=== "Rust"

    ```rust
    use std::collections::HashSet;
    use yggdryl_dtype::{F64Type, I64Type};

    assert_eq!(I64Type::new(), I64Type::default());
    let mut set = HashSet::new();
    set.insert(I64Type::new());
    set.insert(I64Type::new());
    assert_eq!(set.len(), 1);
    assert_ne!(I64Type::new().name(), F64Type::new().name());
    ```

## Arrow interop (Rust-only)

=== "Rust"

    ```rust
    use yggdryl_dtype::{DataType, I64Type};
    use arrow_schema::DataType as ArrowDataType;

    let dt = I64Type::new();
    assert_eq!(dt.to_arrow(), ArrowDataType::Int64);
    assert_eq!(I64Type::from_arrow(&ArrowDataType::Int64).unwrap(), dt);

    // A mismatch is a guided error.
    let err = I64Type::from_arrow(&ArrowDataType::Utf8).unwrap_err();
    assert!(err.to_string().contains("int64"));
    ```
