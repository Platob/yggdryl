# Scalars

A **scalar** is a single value of a [data type](dtype.md) — the third Arrow data-model
layer (data types → [fields](field.md) → scalars). `yggdryl-scalar` mirrors the layers
below: the FFI-opaque `Scalar`, the Rust-only `TypedScalar<DT, T>`, and the category traits
`PrimitiveScalar` (+ `LogicalScalar` / `NestedScalar` scaffolding).

A scalar is **always present** — it carries a value, never a null. Nullability is not a
scalar property; it is modelled separately (a `NullType` value and, later, union types), so
a scalar stays a plain value that always serialises.

The concrete **primitive** scalars are the ten native numerics (`I8Scalar` … `F64Scalar`)
plus `BooleanScalar`. Each holds a value, reports its `data_type`, and round-trips through
its value's little-endian bytes. Equality and hashing are **by the serialised bytes**, so
the float scalars behave bitwise (`0.0 != -0.0`, two `NaN`s with the same bits are equal).

!!! note "Node value marshalling"
    `I64Scalar` / `U64Scalar` values marshal as `bigint` (so 64-bit integers keep full
    precision), and `F32Scalar` marshals its value over an `f64` boundary — the same idioms
    as the buffer layer. A 64-bit value outside the range throws a guided error rather than
    truncating. Every primitive scalar is present.

## Construct and value

A scalar takes its value directly; the value is always available.

=== "Python"

    ```python
    from yggdryl.scalar import I64Scalar
    from yggdryl.dtype import I64Type

    present = I64Scalar(7)
    assert present.value == 7
    assert present.data_type == I64Type()
    ```

=== "Node"

    ```js
    const { I64Scalar } = require('yggdryl').scalar
    const { I64Type } = require('yggdryl').dtype

    const present = new I64Scalar(7n)   // i64 marshals as bigint
    console.assert(present.value === 7n)
    console.assert(present.dataType.equals(new I64Type()))
    ```

=== "Rust"

    ```rust
    use yggdryl_dtype::DataType;
    use yggdryl_scalar::{I64Scalar, Scalar, TypedScalar};

    let present = I64Scalar::new(7);
    assert_eq!(present.value(), 7);
    assert_eq!(TypedScalar::data_type(&present).name(), "int64");
    ```

## Default scalar

`default_scalar()` builds the scalar of the type's default value (`0` / `0.0` / `False`) —
the same default a null substitutes to when [building a buffer](infer.md).

=== "Python"

    ```python
    from yggdryl.scalar import I64Scalar

    assert I64Scalar.default_scalar() == I64Scalar(0)
    assert I64Scalar.default_scalar().value == 0
    ```

=== "Node"

    ```js
    const { I64Scalar } = require('yggdryl').scalar

    console.assert(I64Scalar.defaultScalar().equals(new I64Scalar(0n)))
    console.assert(I64Scalar.defaultScalar().value === 0n)
    ```

=== "Rust"

    ```rust
    use yggdryl_scalar::{I64Scalar, TypedScalar};

    assert_eq!(I64Scalar::default_scalar(), I64Scalar::new(0));
    ```

## Byte round-trip

A scalar serialises to just its value's little-endian bytes; `deserialize_bytes` gives a
guided error when the bytes do not decode for the data type (e.g. a wrong length).

=== "Python"

    ```python
    from yggdryl.scalar import I64Scalar

    present = I64Scalar(7)
    assert len(present.serialize_bytes()) == 8
    assert I64Scalar.deserialize_bytes(present.serialize_bytes()) == present

    try:
        I64Scalar.deserialize_bytes(bytes([0, 0, 0]))   # wrong length for int64
    except ValueError as error:
        assert "byte" in str(error)
    ```

=== "Node"

    ```js
    const { I64Scalar } = require('yggdryl').scalar

    const present = new I64Scalar(7n)
    console.assert(present.serializeBytes().length === 8)
    console.assert(I64Scalar.deserializeBytes(present.serializeBytes()).equals(present))

    try {
      I64Scalar.deserializeBytes(Buffer.from([0, 0, 0]))  // wrong length for int64
    } catch (error) {
      console.assert(/byte/.test(error.message))
    }
    ```

=== "Rust"

    ```rust
    use yggdryl_scalar::{I64Scalar, Scalar};

    let present = I64Scalar::new(7);
    assert_eq!(I64Scalar::deserialize_bytes(&present.serialize_bytes()).unwrap(), present);
    assert!(I64Scalar::deserialize_bytes(&[0, 0, 0]).is_err());
    ```

## Value semantics

Scalars compare and hash by their serialised bytes, so they work as dict / map keys and set
members; the float scalars are bitwise.

=== "Python"

    ```python
    import math, pickle
    from yggdryl.scalar import F64Scalar, I64Scalar

    assert I64Scalar(5) == I64Scalar(5)
    assert I64Scalar(5) != I64Scalar(6)
    assert F64Scalar(0.0) != F64Scalar(-0.0)          # distinct bits
    assert F64Scalar(math.nan) == F64Scalar(math.nan) # same bit pattern
    assert pickle.loads(pickle.dumps(I64Scalar(5))) == I64Scalar(5)
    ```

=== "Node"

    ```js
    const { F64Scalar, I64Scalar } = require('yggdryl').scalar

    console.assert(new I64Scalar(5n).equals(new I64Scalar(5n)))
    console.assert(!new F64Scalar(0.0).equals(new F64Scalar(-0.0)))
    console.assert(new F64Scalar(NaN).equals(new F64Scalar(NaN)))
    ```

=== "Rust"

    ```rust
    use yggdryl_scalar::{F64Scalar, I64Scalar};

    assert_eq!(I64Scalar::new(5), I64Scalar::new(5));
    assert_ne!(I64Scalar::new(5), I64Scalar::new(6));
    assert_ne!(F64Scalar::new(0.0), F64Scalar::new(-0.0));
    assert_eq!(F64Scalar::new(f64::NAN), F64Scalar::new(f64::NAN));
    ```
