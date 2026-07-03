# Scalars

The `yggdryl-scalar` crate is the Apache Arrow-centralized **scalar layer**,
built on [`yggdryl-dtype`](dtype.md) and `yggdryl-core` — the third of the three
data layers, each concern its own crate, so the concrete types share one bare
name across the layers (`yggdryl_scalar::Int64` holds one value of the
`yggdryl_dtype::Int64` type, whose column is a
[`yggdryl_field::Int64`](field.md)). A scalar is a **single, possibly-null
value** of a data type, with `as_*` accessors reading it as any
exactly-representable Rust target.

The bindings expose the layer as `yggdryl.scalar` (Python and Node), adapting to
idioms: Node carries 8–32 bit values as `number` and the 64-bit types as
`BigInt`, byte values cross as Python `bytes` / JS `Buffer`, the null-or-value
scalars are concrete per-type classes (`OptionalInt64`, `OptionalBinary`, …)
built straight from the native value, and the `as_*` accessors surface the core
`DataError` as a raised `ValueError` (Python) / thrown `Error` (Node). Three
things stay **Rust-only**, stated here and in both binding module docs: the
[Arrow interop](#arrow-interop) surface (`to_arrow` / `from_arrow` exchange
`arrow-array` values that cannot cross the FFI boundary), the `FromScalar` /
`DefaultScalar` traits (generic Rust bounds; the bindings reach defaults through
a data type's `default_scalar()`), and the
[nested scalars](#nested-scalars-serie-map-and-struct) — the generic `Serie` /
`Map` / `Struct` and the buffer-backed `Int64Serie`, whose zero-copy Arrow
buffers await C Data Interface interop — which have no concrete FFI shape yet.

## Scalars hold a value or null

Scalars are built from their native value and read through the `as_*` accessors:
direct for the scalar's own type, exact conversion otherwise, and an actionable
error — Rust `DataError`, Python `ValueError`, a thrown JS `Error` — when the
scalar is null or the value is not exactly representable:

=== "Python"

    ```python
    from yggdryl import scalar

    answer = scalar.Int64(42)
    assert answer.value() == 42
    assert answer.as_i8() == 42          # converted access
    try:
        answer.as_str()                  # an int64 is not a string
    except ValueError as error:
        assert "no str conversion" in str(error)
    assert scalar.Int64.null().is_null()
    ```

=== "Node"

    ```js
    const { scalar } = require('yggdryl')

    const answer = new scalar.Int64(42n)
    assert.equal(answer.value(), 42n)
    assert.equal(answer.asI8(), 42)      // converted access
    assert.throws(() => answer.asStr(), /no str conversion/) // not a string
    assert.equal(scalar.Int64.null().isNull(), true)
    ```

=== "Rust"

    ```rust
    use yggdryl_scalar::{Int64, RawScalar};

    fn main() {
        let answer = Int64::from(42);
        assert_eq!(answer.value(), Some(&42));
        assert_eq!(answer.as_i8().unwrap(), 42); // converted access
        assert!(answer.as_str(None).is_err()); // an int64 is not a string
        assert!(Int64::null().is_null());
    }
    ```

## The `binary` scalar is a positioned-IO resource

The `binary` scalar holds its bytes as a `yggdryl-core` positioned-IO
`ByteBuffer`, so a value plugs straight into the core IO layer: in Rust, `io()`
borrows the resource for `RawIOBase` reads and `into_io()` moves it out to wrap
in the `RawIOCursor` / `RawIOSlice` adapters; the bindings hand back a
`yggdryl.core` `ByteBuffer` through `to_io()` (one copy at the FFI boundary,
like strings). `as_bytes` answers the native type directly, `as_str` decodes —
UTF-8 borrowed by default, or any core `Charset` passed explicitly (the bindings
take an optional charset name, `"utf8"` or `"latin1"`) — and `into_io_slice`
(bindings: `to_io_slice`) hands the value out as a full-window core
`ByteBufferSlice` for window-relative positioned reads.

=== "Python"

    ```python
    from yggdryl import core, scalar

    blob = scalar.Binary(b"\x01\x02\x03")
    assert blob.value() == b"\x01\x02\x03"
    assert blob.as_bytes() == b"\x01\x02\x03"
    assert scalar.Binary(b"hi").as_str() == "hi"  # valid UTF-8 only

    # The value doubles as a core positioned-IO ByteBuffer.
    io = blob.to_io()
    assert io.pread_byte_one(1, core.Whence.Start) == 2

    assert scalar.Binary.null().is_null()
    assert scalar.OptionalBinary(b"hi").as_bytes() == b"hi"
    ```

=== "Node"

    ```js
    const { core, scalar } = require('yggdryl')

    const blob = new scalar.Binary(Buffer.from([1, 2, 3]))
    assert.deepEqual(blob.value(), Buffer.from([1, 2, 3]))
    assert.deepEqual(blob.asBytes(), Buffer.from([1, 2, 3]))
    assert.equal(new scalar.Binary(Buffer.from('hi')).asStr(), 'hi') // valid UTF-8 only

    // The value doubles as a core positioned-IO ByteBuffer.
    const io = blob.toIo()
    assert.equal(io.preadByteOne(1, core.Whence.Start), 2)

    assert.equal(scalar.Binary.null().isNull(), true)
    assert.deepEqual(new scalar.OptionalBinary(Buffer.from('hi')).asBytes(), Buffer.from('hi'))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{RawIOBase, RawIOCursor, Whence};
    use yggdryl_scalar::{Binary, RawScalar};

    fn main() {
        let blob = Binary::new(vec![1, 2, 3]);
        assert_eq!(blob.value(), Some(&[1, 2, 3][..]));
        assert_eq!(blob.as_bytes().unwrap(), &[1, 2, 3][..]); // borrowed, never copied
        assert_eq!(Binary::new(b"hi".to_vec()).as_str(None).unwrap(), "hi");

        // The value doubles as a core positioned-IO resource.
        let io = blob.io().unwrap();
        assert_eq!(io.pread_byte_one(1, Whence::Start).unwrap(), 2);
        let cursor = RawIOCursor::new(blob.clone().into_io().unwrap());
        assert_eq!(cursor.pread_byte_array(0, Whence::Start, 2).unwrap(), vec![1, 2]);

        assert!(Binary::null().is_null());
    }
    ```

## The optional scalar

`Optional<D, S>` is the null-or-value scalar over union storage: an inner scalar
`S`, or the null variant. Access redirects to the inner scalar (`value` and every
`as_*` accessor answer through `S`), and so does the Arrow form: a one-element
`UnionArray` whose type id selects the variant. The bindings expose it as
concrete per-type classes built straight from the native value:

=== "Python"

    ```python
    from yggdryl import scalar

    answer = scalar.OptionalInt64(42)
    assert answer.as_i64() == 42  # redirected to the inner scalar
    assert answer.scalar().value() == 42
    assert not answer.is_null()

    missing = scalar.OptionalInt64.null()
    assert missing.is_null()
    assert missing.value() is None
    ```

=== "Node"

    ```js
    const { scalar } = require('yggdryl')

    const answer = new scalar.OptionalInt64(42n)
    assert.equal(answer.asI64(), 42n) // redirected to the inner scalar
    assert.equal(answer.scalar().value(), 42n)
    assert.equal(answer.isNull(), false)

    const missing = scalar.OptionalInt64.null()
    assert.equal(missing.isNull(), true)
    assert.equal(missing.value(), null)
    ```

=== "Rust"

    ```rust
    use yggdryl_scalar::yggdryl_dtype::Int64 as Int64Type;
    use yggdryl_scalar::{Int64, Optional, RawScalar};

    fn main() {
        let answer = Optional::new(Int64::new(42));
        assert_eq!(answer.as_i64().unwrap(), 42); // redirected to the inner scalar
        assert_eq!(answer.scalar(), Some(&Int64::new(42)));
        assert!(!answer.is_null());

        let missing: Optional<Int64Type, Int64> = Optional::null();
        assert!(missing.is_null());
        assert_eq!(missing.value(), None);
    }
    ```

## Arrow interop

!!! note "Rust only"
    `to_arrow` / `from_arrow` exchange `arrow-array` values, which cannot cross
    the FFI boundary — the bindings will gain this surface through the Arrow C
    Data Interface as it lands.

A scalar mirrors Arrow's own scalar representation — a one-element `arrow_array`
array, null when the scalar is null — with a `to_arrow` / `from_arrow` pair
(`from_arrow` is the exact inverse, refusing a mismatched Arrow value with
`DataError`). The `arrow-array` and `arrow-buffer` subset crates are re-exported
from the crate root so downstream code uses the exact versions the crate was
built against.

```rust
use yggdryl_scalar::arrow_array::Array;
use yggdryl_scalar::{Int64, RawScalar};

fn main() {
    let arrow = Int64::new(42).to_arrow();
    assert_eq!((arrow.len(), arrow.null_count()), (1, 0));
    assert_eq!(Int64::from_arrow(arrow.as_ref()).unwrap(), Int64::new(42));
    assert!(Int64::null().to_arrow().is_null(0));
}
```

## Nested scalars: serie, map and struct

!!! note "Rust only"
    The nested scalars are generic over their child types (or carry dynamic Arrow
    fields), and the buffer-backed `Int64Serie` shares raw Arrow buffers that await
    C Data Interface interop — none has a concrete FFI shape yet, so they are not
    exposed to Python or Node.

The list scalar is *our array*: `Serie<D, S>` is backed by one zero-copy Arrow
child array — construction assembles the elements once, `to_arrow` / `from_arrow`
are reference-count bumps, and the scalar accessors read elements back out
(`get_scalar_at(index)` redirects one element through the inner scalar's own
`from_arrow`, and the generic native accessor `get_at::<T>(index)` reads an
element as any native Rust target through the `as_*` contract — `i64` or any
exactly-representable number for an `int64` element, `Vec<u8>`, `String` or a
`yggdryl-core` `ByteBufferSlice` for a `binary` element (`FromScalar` names the
readable targets); `len` / `is_empty` describe the sequence). `Int64Serie` is the
concrete list of `int64`, borrowing the raw Arrow buffers themselves
(`ScalarBuffer<i64>` elements plus an optional `NullBuffer`): `values()` borrows
the whole element buffer as `&[i64]` without copying, `get_at::<T>(index)` reads
one element null-aware straight from the buffers, `get_scalar_at(index)` hands
back an `Int64`, and `from_io` / `pwrite_io` bridge the elements to any
`yggdryl-core` positioned-IO resource through the little-endian `pread_i64` /
`pwrite_i64` primitive helpers.
`Map<K, V, SK, SV>` holds the entry sequence and `Struct` one row of
one-element Arrow columns, each round-tripping through a one-element Arrow array
whose children redirect to the inner scalars' own `to_arrow` / `from_arrow`.

```rust
use yggdryl_scalar::yggdryl_dtype as dtype;
use yggdryl_scalar::{Int64, Int64Serie, RawScalar, Serie};

fn main() {
    let numbers = Serie::new(vec![Int64::new(1), Int64::null()]);
    assert_eq!(numbers.get_scalar_at(1), Some(Int64::null()));
    assert_eq!(numbers.get_at::<i64>(0).unwrap(), 1); // the native value, any target
    let arrow = numbers.to_arrow(); // a one-element ListArray sharing the elements
    assert_eq!(Serie::from_arrow(arrow.as_ref()).unwrap(), numbers);

    // The buffer-backed list of int64: native, zero-copy access.
    let fast = Int64Serie::from(vec![1, 2, 3]);
    assert_eq!(fast.values(), Some(&[1, 2, 3][..])); // borrows the Arrow buffer
    assert_eq!(fast.get_at::<i64>(1).unwrap(), 2);
    assert_eq!(Int64Serie::from_arrow(fast.to_arrow().as_ref()).unwrap(), fast);

    // The type parameters name the dtype-layer types.
    let _: Serie<dtype::Int64, Int64> = Serie::default();
}
```

## The trait layers

- **`RawScalar<D: RawDataType>`** — the untyped base: a single, possibly-null
  value (`data_type`, `is_null`, `value` of an associated `Value: ?Sized`);
  `to_arrow` / `from_arrow` mirror a one-element `arrow_array` array. The `as_*`
  accessors (`as_i8` … `as_u64`, `as_f32` / `as_f64`, `as_bool`, `as_str`,
  `as_bytes`) read the value as a chosen Rust type under one contract: the value
  whenever the target represents it exactly — direct for the scalar's own type
  (`as_str` / `as_bytes` borrow, never copy), exact conversion otherwise — and an
  actionable `DataError` when not: `NullValue` for a null scalar,
  `InexactConversion` when converting would change the value (a narrowing out of
  range, a float that would round, non-UTF-8 bytes read as `str`),
  `UnsupportedConversion` when the type has no conversion to the target at all.
  Every accessor defaults to that error, so a concrete scalar overrides only the
  targets its value converts to; the bindings raise `ValueError` (Python) / throw
  (Node).
- **`Scalar<T>: RawScalar<Self::Type, Value = T>`** — the typed layer: a scalar
  whose value is `T` (possibly unsized: a string scalar exposes `Option<&str>`).
- **`FromScalar`** — the native Rust targets readable out of any scalar, behind
  the generic accessors such as `Serie::get_at::<T>`.
- **`DefaultScalar<T>: DataType<T>`** — the scalar a data type defaults to. The
  scalar layer builds on the data types, never the other way around, so the
  default *scalar* of a type lives here (implemented for every `yggdryl-dtype`
  type next to its scalar): a scalar holding `default_value`, except where the
  scalar models nullness (an optional defaults to its null variant, matching the
  scalar's own `Default`).
