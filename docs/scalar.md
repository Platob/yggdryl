# Scalars

A **scalar** is a single, possibly-null value of a [data type](dtype.md) — the third
Arrow data-model layer (data types → [fields](field.md) → scalars). `yggdryl-scalar`
mirrors the layers below: the FFI-opaque `Scalar`, the Rust-only `TypedScalar<DT, T>`,
and the category traits `PrimitiveScalar` (+ `LogicalScalar` / `NestedScalar`
scaffolding).

The concrete **primitive** scalars are the ten native numerics (`I8Scalar` …
`F64Scalar`) plus `BooleanScalar`. Each holds a value or is null, reports its
`data_type`, and round-trips through bytes. Equality and hashing are **by the serialised
bytes**, so the float scalars behave bitwise (`0.0 != -0.0`, two `NaN`s with the same
bits are equal) and a present value never equals a null.

!!! note "Node value marshalling"
    `I64Scalar` / `U64Scalar` values marshal as `bigint` (so 64-bit integers keep
    full precision), and `F32Scalar` marshals its value over an `f64` boundary — the
    same idioms as the buffer layer. Every primitive scalar is present.

## Construct, value, and null

Passing no value (or `None` / `null`) builds a null scalar; `null()` is an explicit
factory.

=== "Python"

    ```python
    from yggdryl.scalar import I64Scalar
    from yggdryl.dtype import I64Type

    present = I64Scalar(7)
    assert present.value == 7
    assert present.is_null is False
    assert present.data_type == I64Type()

    null = I64Scalar(None)          # or I64Scalar(), or I64Scalar.null()
    assert null.value is None
    assert null.is_null is True
    ```

=== "Node"

    ```js
    const { I64Scalar } = require('yggdryl').scalar
    const { I64Type } = require('yggdryl').dtype

    const present = new I64Scalar(7n)   // i64 marshals as bigint
    console.assert(present.value === 7n)
    console.assert(present.isNull === false)
    console.assert(present.dataType.equals(new I64Type()))

    const nul = new I64Scalar()          // or new I64Scalar(null), or I64Scalar.null()
    console.assert(nul.value === null)
    console.assert(nul.isNull === true)
    ```

=== "Rust"

    ```rust
    use yggdryl_dtype::DataType;
    use yggdryl_scalar::{I64Scalar, Scalar, TypedScalar};

    let present = I64Scalar::new(7);
    assert_eq!(present.value(), Some(7));
    assert!(!present.is_null());
    assert_eq!(TypedScalar::data_type(&present).name(), "int64");

    assert_eq!(I64Scalar::null().value(), None);
    ```

## Byte round-trip

A scalar serialises to a 1-byte null flag followed by the value's little-endian bytes
when present; `deserialize_bytes` gives guided errors for a bad flag or a stray payload.

=== "Python"

    ```python
    from yggdryl.scalar import I64Scalar

    present = I64Scalar(7)
    assert I64Scalar.deserialize_bytes(present.serialize_bytes()) == present
    assert I64Scalar.null().serialize_bytes() == bytes([0])

    try:
        I64Scalar.deserialize_bytes(bytes([2]))   # flag neither 0 nor 1
    except ValueError as error:
        assert "expected 0" in str(error)
    ```

=== "Node"

    ```js
    const { I64Scalar } = require('yggdryl').scalar

    const present = new I64Scalar(7n)
    console.assert(I64Scalar.deserializeBytes(present.serializeBytes()).equals(present))
    console.assert(I64Scalar.null().serializeBytes().equals(Buffer.from([0])))

    try {
      I64Scalar.deserializeBytes(Buffer.from([2]))
    } catch (error) {
      console.assert(/expected 0/.test(error.message))
    }
    ```

=== "Rust"

    ```rust
    use yggdryl_scalar::{I64Scalar, Scalar};

    let present = I64Scalar::new(7);
    assert_eq!(I64Scalar::deserialize_bytes(&present.serialize_bytes()).unwrap(), present);
    assert_eq!(I64Scalar::null().serialize_bytes(), vec![0]);
    assert!(I64Scalar::deserialize_bytes(&[2]).is_err());
    ```

## Value semantics

Scalars compare and hash by their serialised bytes, so they work as dict / map keys and
set members; the float scalars are bitwise and a present value never equals a null.

=== "Python"

    ```python
    import math, pickle
    from yggdryl.scalar import F64Scalar, I64Scalar

    assert I64Scalar(5) == I64Scalar(5)
    assert I64Scalar(5) != I64Scalar.null()
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
    assert_ne!(I64Scalar::new(5), I64Scalar::null());
    assert_ne!(F64Scalar::new(0.0), F64Scalar::new(-0.0));
    assert_eq!(F64Scalar::new(f64::NAN), F64Scalar::new(f64::NAN));
    ```
