# Scalars

The `yggdryl-scalar` crate is the Apache Arrow-centralized **scalar layer**,
built on [`yggdryl-dtype`](dtype.md) and `yggdryl-core` — the third of the three
data layers, each concern its own crate, so the concrete types share one naming
convention across the layers (`yggdryl_scalar::Int64Scalar` holds one value of the
`yggdryl_dtype::Int64Type` type, whose column is a
[`yggdryl_field::Int64Field`](field.md)). A scalar is a **single, possibly-null
value** of a data type, with `as_*` accessors reading it as any
exactly-representable Rust target.

The bindings expose the layer as `yggdryl.scalar` (Python and Node), adapting to
idioms: Node carries 8–32 bit integers as `number` and the 64-bit integers as
`BigInt`, the floats `Float32Scalar` / `Float64Scalar` as `number`, byte values
cross as Python `bytes` / JS `Buffer`, the null-or-value scalars are concrete
per-type classes (`OptionalInt64Scalar`, `OptionalFloat64Scalar`,
`OptionalBinaryScalar`, …) built straight from the native value, the buffer-backed
serie scalars (`Int8Serie` … `UInt64Serie`, `Float16Serie` … `Float64Serie`) cross
(elements copy out through `to_pylist()` in Python / `toArray()` in Node — the
pyarrow / Arrow JS conversion names, kept for every future native-container accessor
such as a dict-shaped `to_pydict()` — as `int` / `number` for the 8–32 bit integer
widths, `BigInt` for the 64-bit ones, and `float` / `number` for the floats), and
the `as_*` accessors surface the core `DataError` as a raised `ValueError` (Python)
/ thrown `Error` (Node). Three
things stay **Rust-only**, stated here and in both binding module docs: the
[Arrow interop](#arrow-interop) surface (`to_arrow_scalar` / `from_arrow`, and the
`cast_dtype` / `cast_dtype_unchecked` casts which return a re-typed `arrow-array`
value — all exchange `arrow-array` values that cannot cross the FFI boundary), the
`FromScalar` /
`ScalarFactory` traits (generic Rust bounds; the bindings reach defaults through a
data type's `default_scalar()`), and — for the serie scalars — their
per-element-null construction, `to_arrow_array` / `nulls` Arrow-buffer surface and
`from_io` / `pwrite_io` two-resource bridge (which await C Data Interface interop
or borrow a second IO resource at once), so a serie built from a binding is a
dense (all-valid) serie. The dynamic bases and typed generics of the
[nested scalars](#nested-scalars-serie-map-and-struct) — `Serie` / `TypedSerie`,
`MapScalar` / `TypedMapScalar`, `StructScalar`, the struct-row series `StructSerie`
/ `TypedStructSerie` — and the [`AnySerie` / `AnyScalar` /
`NestedSerie`](#anyserie-anyscalar-and-nestedserie) type-erased holders behind them
have no concrete FFI shape yet; the struct family crosses as
[`RecordScalar`](#recordscalar), built from a `dict` / plain object with every
field inferred.

## Scalars hold a value or null

Scalars are built from their native value and read through the `as_*` accessors:
direct for the scalar's own type, exact conversion otherwise, and an actionable
error — Rust `DataError`, Python `ValueError`, a thrown JS `Error` — when the
scalar is null or the value is not exactly representable:

=== "Python"

    ```python
    from yggdryl import scalar

    answer = scalar.Int64Scalar(42)
    assert answer.value() == 42
    assert answer.as_i8() == 42          # converted access
    try:
        answer.as_str()                  # an int64 is not a string
    except ValueError as error:
        assert "no str conversion" in str(error)
    assert scalar.Int64Scalar.null().is_null()
    ```

=== "Node"

    ```js
    const { scalar } = require('yggdryl')

    const answer = new scalar.Int64Scalar(42n)
    assert.equal(answer.value(), 42n)
    assert.equal(answer.asI8(), 42)      // converted access
    assert.throws(() => answer.asStr(), /no str conversion/) // not a string
    assert.equal(scalar.Int64Scalar.null().isNull(), true)
    ```

=== "Rust"

    ```rust
    use yggdryl_scalar::{Int64Scalar, Scalar};

    fn main() {
        let answer = Int64Scalar::from(42);
        assert_eq!(answer.value(), Some(&42));
        assert_eq!(answer.as_i8().unwrap(), 42); // converted access
        assert!(answer.as_str(None).is_err()); // an int64 is not a string
        assert!(Int64Scalar::null().is_null());
    }
    ```

## The data type builds its scalar

A typed data type *is* the scalar factory: `data_type.scalar(value)` builds the
matching scalar and `data_type.default_scalar()` its default, so a value can be
made straight from the type (`ScalarFactory` in Rust, a method on every
`yggdryl.dtype` type in the bindings). In Rust, `null_scalar()` builds the null
scalar directly; the bindings reach it through the scalar class's `null()`.

=== "Python"

    ```python
    from yggdryl import dtype

    answer = dtype.Int64Type().scalar(42)
    assert answer.value() == 42
    assert dtype.Int64Type().default_scalar().value() == 0
    ```

=== "Node"

    ```js
    const { dtype } = require('yggdryl')

    const answer = new dtype.Int64Type().scalar(42n)
    assert.equal(answer.value(), 42n)
    assert.equal(new dtype.Int64Type().defaultScalar().value(), 0n)
    ```

=== "Rust"

    ```rust
    use yggdryl_scalar::yggdryl_dtype::Int64Type;
    use yggdryl_scalar::{Int64Scalar, Scalar, ScalarFactory};

    fn main() {
        assert_eq!(Int64Type.scalar(42), Int64Scalar::new(42));
        assert!(Int64Type.null_scalar().is_null());
        assert_eq!(Int64Type.default_scalar(), Int64Scalar::new(0));
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

The `utf8` **string** scalar (`StringScalar`) is the same idea one type up: a
`utf8` value is a **logical** type over `binary` storage, so `StringScalar` holds
its content as a core `StringBuffer` — the same UTF-8 bytes, plus a typed `char`
view (`IOBase<char>`) — and `io()` / `into_io()` hand it back for positioned byte
reads and char writes. `value` / `as_str` borrow the string, `as_bytes` its UTF-8
bytes.

=== "Python"

    ```python
    from yggdryl import core, scalar

    blob = scalar.BinaryScalar(b"\x01\x02\x03")
    assert blob.value() == b"\x01\x02\x03"
    assert blob.as_bytes() == b"\x01\x02\x03"
    assert scalar.BinaryScalar(b"hi").as_str() == "hi"  # valid UTF-8 only

    # The value doubles as a core positioned-IO ByteBuffer.
    io = blob.to_io()
    assert io.pread_byte_one(1, core.Whence.Start) == 2

    assert scalar.BinaryScalar.null().is_null()
    assert scalar.OptionalBinaryScalar(b"hi").as_bytes() == b"hi"
    ```

=== "Node"

    ```js
    const { core, scalar } = require('yggdryl')

    const blob = new scalar.BinaryScalar(Buffer.from([1, 2, 3]))
    assert.deepEqual(blob.value(), Buffer.from([1, 2, 3]))
    assert.deepEqual(blob.asBytes(), Buffer.from([1, 2, 3]))
    assert.equal(new scalar.BinaryScalar(Buffer.from('hi')).asStr(), 'hi') // valid UTF-8 only

    // The value doubles as a core positioned-IO ByteBuffer.
    const io = blob.toIo()
    assert.equal(io.preadByteOne(1, core.Whence.Start), 2)

    assert.equal(scalar.BinaryScalar.null().isNull(), true)
    assert.deepEqual(new scalar.OptionalBinaryScalar(Buffer.from('hi')).asBytes(), Buffer.from('hi'))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{RawIOBase, RawIOCursor, Whence};
    use yggdryl_scalar::{BinaryScalar, Scalar};

    fn main() {
        let blob = BinaryScalar::new(vec![1, 2, 3]);
        assert_eq!(blob.value(), Some(&[1, 2, 3][..]));
        assert_eq!(blob.as_bytes().unwrap(), &[1, 2, 3][..]); // borrowed, never copied
        assert_eq!(BinaryScalar::new(b"hi".to_vec()).as_str(None).unwrap(), "hi");

        // The value doubles as a core positioned-IO resource.
        let io = blob.io().unwrap();
        assert_eq!(io.pread_byte_one(1, Whence::Start).unwrap(), 2);
        let cursor = RawIOCursor::new(blob.clone().into_io().unwrap());
        assert_eq!(cursor.pread_byte_array(0, Whence::Start, 2).unwrap(), vec![1, 2]);

        assert!(BinaryScalar::null().is_null());
    }
    ```

## The optional scalar

`TypedOptionalScalar<D, S>` is the null-or-value scalar over union storage: an inner
scalar `S`, or the null variant. Access redirects to the inner scalar (`value` and
every `as_*` accessor answer through `S`), and so does the Arrow form: a
one-element `UnionArray` whose type id selects the variant. The bindings expose it
as concrete per-type classes built straight from the native value:

=== "Python"

    ```python
    from yggdryl import scalar

    answer = scalar.OptionalInt64Scalar(42)
    assert answer.as_i64() == 42  # redirected to the inner scalar
    assert answer.scalar().value() == 42
    assert not answer.is_null()

    missing = scalar.OptionalInt64Scalar.null()
    assert missing.is_null()
    assert missing.value() is None
    ```

=== "Node"

    ```js
    const { scalar } = require('yggdryl')

    const answer = new scalar.OptionalInt64Scalar(42n)
    assert.equal(answer.asI64(), 42n) // redirected to the inner scalar
    assert.equal(answer.scalar().value(), 42n)
    assert.equal(answer.isNull(), false)

    const missing = scalar.OptionalInt64Scalar.null()
    assert.equal(missing.isNull(), true)
    assert.equal(missing.value(), null)
    ```

=== "Rust"

    ```rust
    use yggdryl_scalar::yggdryl_dtype::Int64Type;
    use yggdryl_scalar::{Int64Scalar, Scalar, TypedOptionalScalar};

    fn main() {
        let answer = TypedOptionalScalar::new(Int64Scalar::new(42));
        assert_eq!(answer.as_i64().unwrap(), 42); // redirected to the inner scalar
        assert_eq!(answer.scalar(), Some(&Int64Scalar::new(42)));
        assert!(!answer.is_null());

        let missing: TypedOptionalScalar<Int64Type, Int64Scalar> = TypedOptionalScalar::null();
        assert!(missing.is_null());
        assert_eq!(missing.value(), None);
    }
    ```

## Arrow interop

!!! note "Rust only"
    `to_arrow_scalar` / `from_arrow` exchange `arrow-array` values, which cannot cross
    the FFI boundary — the bindings will gain this surface through the Arrow C
    Data Interface as it lands.

A scalar mirrors Arrow's own scalar representation — a one-element `arrow_array`
array, null when the scalar is null — with a `to_arrow_scalar` / `from_arrow` pair
(`from_arrow` is the exact inverse, refusing a mismatched Arrow value with
`DataError`). The `arrow-array` and `arrow-buffer` subset crates are re-exported
from the crate root so downstream code uses the exact versions the crate was
built against.

```rust
use yggdryl_scalar::arrow_array::Array;
use yggdryl_scalar::{Int64Scalar, Scalar};

fn main() {
    let arrow = Int64Scalar::new(42).to_arrow_scalar();
    assert_eq!((arrow.len(), arrow.null_count()), (1, 0));
    assert_eq!(Int64Scalar::from_arrow(arrow.as_ref()).unwrap(), Int64Scalar::new(42));
    assert!(Int64Scalar::null().to_arrow_scalar().is_null(0));
}
```

## Casting

!!! note "Rust only"
    `cast_dtype` / `cast_dtype_unchecked` return the value re-typed as an
    `arrow-array` value, which cannot cross the FFI boundary — Rust-only alongside
    the rest of the Arrow interop surface.

`cast_dtype(dtype)` re-types a scalar to any target data type, returning the value as
a one-element Arrow array of that type (rehydrate it with the target scalar's
`from_arrow`). It is **exact-or-error**, reusing the `as_*` contract: a null casts to
a null of the target, a numeric target reads the matching `as_*` accessor (erroring
when the value would not fit), a `utf8` target reads `as_str` (validated), and a
`binary` target reads `as_bytes`. The `unsafe` `cast_dtype_unchecked(dtype)`
reinterprets the value's raw little-endian bytes instead — bridging *all* fixed-width,
`binary` and `utf8` types (an `int64`'s eight bytes become a `binary`, or a raw
`utf8` read with no UTF-8 validation) — so it may lose meaning and must be called in
an `unsafe` block.

```rust
use yggdryl_scalar::yggdryl_dtype::{BinaryType, Int32Type};
use yggdryl_scalar::{BinaryScalar, Int32Scalar, Int64Scalar, Scalar};

fn main() {
    // Exact: int64 → int32 (and a value that would not fit errors).
    let narrowed = Int64Scalar::new(42).cast_dtype(&Int32Type).unwrap();
    assert_eq!(Int32Scalar::from_arrow(narrowed.as_ref()).unwrap(), Int32Scalar::new(42));
    assert!(Int64Scalar::new(1 << 40).cast_dtype(&Int32Type).is_err());

    // Unchecked reinterpret: int64 → its eight little-endian bytes as binary.
    let bytes = unsafe { Int64Scalar::new(1).cast_dtype_unchecked(&BinaryType) }.unwrap();
    assert_eq!(
        BinaryScalar::from_arrow(bytes.as_ref()).unwrap(),
        BinaryScalar::new(1i64.to_le_bytes().to_vec()),
    );
}
```

## Nested scalars: serie, map and struct

Every nested scalar holds **our own series** — [`AnySerie`](#anyserie-anyscalar-and-nestedserie)
columns, integer elements decomposed to their raw buffers, anything else zero-copy
Arrow — and reconstitutes Arrow arrays on demand: a list holds its *item serie*, a
map holds its *entries serie* (a serie of struct entries), and a struct holds an
*array of column series*. Its atomic counterpart is `AnyScalar`, the type-erased
single value behind a [`RecordScalar`](#recordscalar)'s fields. Constructing
`from_arrow` decomposes the incoming array; both directions are reference-count
bumps, never element copies.

The serie scalar is *our array*: `TypedSerie<D, S>` is backed by one zero-copy item
serie — construction assembles the elements once, `to_arrow_scalar` / `from_arrow`
are reference-count bumps, and the scalar accessors read elements back out
(`get_scalar_at(index)` redirects one element through the inner scalar's own
`from_arrow`, and the generic native accessor `get_at::<T>(index)` reads an
element as any native Rust target through the `as_*` contract — `i64` or any
exactly-representable number for an `int64` element, `Vec<u8>`, `String` or a
`yggdryl-core` `ByteBufferSlice` for a `binary` element (`FromScalar` names the
readable targets); `len` / `is_empty` describe the sequence). Every integer type
also has its concrete serie (`Int8Serie` … `UInt64Serie`), borrowing the raw
Arrow buffers themselves (a `ScalarBuffer` of native elements plus an optional
`NullBuffer`): `values()` borrows the whole element buffer as a native slice
without copying, `get_at::<T>(index)` reads one element null-aware straight from
the buffers, `get_scalar_at(index)` hands back the element scalar,
`to_arrow_array()` converts the elements out as the Arrow primitive array around
the same shared buffers, and `from_io` / `pwrite_io` bridge the elements to any
`yggdryl-core` positioned-IO resource in one bulk little-endian
`pread_byte_array` / `pwrite_byte_array` transfer.
`TypedMapScalar<K, V, SK, SV>` holds the entry sequence and `StructScalar` one row
of one-element column series, each round-tripping through a one-element Arrow array
whose children redirect to the inner scalars' own `to_arrow_scalar` / `from_arrow`.
A **serie of struct rows** is `StructSerie` / `TypedStructSerie<S>` — the struct
counterpart of `Serie` / `TypedSerie` that the generic serie cannot express (a
`StructType` has no compile-time default shape), reading each row back as its `S`
scalar (a `RecordScalar` row atom, or a `StructScalar`) and exposing the struct's
*field columns* as its children.
Each of `serie` / `map` / `optional` also has a dynamic base (`Serie`, `MapScalar`,
`OptionalScalar`) with the element type erased — the base `Scalar` surface only —
that the typed generics `erase()` back to.

### AnySerie, AnyScalar and NestedSerie

`AnySerie` is the type-erased column behind every nested scalar: the fixed-width
numeric element types are held **decomposed** as the concrete buffer-backed series
(the integers `Int8Serie` … `UInt64Serie` and the floats `Float32Serie` /
`Float64Serie`); any other element type keeps its Arrow array zero-copy in the
`Arrow` fallback (more decomposed variants land as concrete series do). `from_arrow`
decomposes, `to_arrow` reconstitutes, `slice` windows, and `get_scalar(index)` reads
one element out as an `AnyScalar` — all sharing buffers. `AnyScalar` is the atomic
counterpart one value down (a number decomposed to its concrete scalar, anything
else a one-element Arrow value), the crate's own holder behind a `RecordScalar`'s
fields.

Both **unwrap** back to the concrete typed value: the generic `unwrap::<S>()`
recovers *any* scalar / serie type through its `from_arrow` (erroring when the held
type does not match), while the zero-copy per-variant accessors (`int64()` … ,
`arrow()`) borrow a decomposed value whose type the caller already knows. The
erasure is verified lossless for every model type by a global coherence check — a
type is not coherent with the Any layer until it round-trips there.

Every nested scalar implements the `NestedSerie` trait for easy child access:
`child_serie_count()`, `child_serie_at(index)`, `child_serie_by(name)` and
`child_serie_name_at(index)` — a serie has one `"item"` child, a map one
`"entries"` child (plus the `"key"` / `"value"` projections by name), a struct /
record one child per field, and a struct serie one field column per field, by
position and by field name. Handles are zero-copy `AnySerie` clones; a null scalar
has no children.

### RecordScalar

`RecordScalar` is the **row-oriented struct atom**: an array of one `AnyScalar` per
field, sharing one `StructType`. Where `StructScalar` is the column-oriented row
(one one-element serie per field), `RecordScalar` materializes it field-by-field —
`scalar_at(index)` / `scalar_by(name)` hand back a field's atomic scalar directly,
and `StructScalar` converts to it with the base accessor `as_struct()`. In the
bindings a record is built straight from a `dict` (Python) / plain object (Node)
with every field inferred, and reads back out as an auto-generated **singleton
dataclass** (one frozen dataclass per schema, cached) in Python or a plain object
in JS.

### `as_serie` / `as_map` / `as_struct` and the native-value accessors

The base `Scalar` carries three nested accessors under the same exact-or-error
`as_*` contract: `as_serie()` hands back the dynamic `Serie`, `as_map()` the
dynamic `MapScalar`, and `as_struct()` the `RecordScalar` — zero-copy handles on
the scalars that have the shape, `UnsupportedConversion` on the ones that don't
(the optional redirects to its inner scalar). In the bindings, every scalar also
carries the general native-value accessor — `to_pyvalue()` (Python) /
`toJsValue()` (Node) — implemented in the core so the value crosses the FFI
boundary **once**, never through per-element scripting loops.

The integer series are the nested scalars exposed to the bindings — built dense
(all-valid) from a native sequence, the whole serie still nullable through
`null()`; `Int64Serie` shown here, the other widths identical modulo the element
type:

=== "Python"

    ```python
    from yggdryl import scalar

    numbers = scalar.Int64Serie([1, 2, 3])
    assert (numbers.is_null(), numbers.is_empty(), numbers.len()) == (False, False, 3)
    assert numbers.to_pylist() == [1, 2, 3]
    assert numbers.get_at(1) == 2                    # the native value
    assert numbers.get_scalar_at(2).value() == 3     # ... or the element scalar
    assert numbers.get_scalar_at(3) is None          # out of bounds
    assert numbers.data_type().name() == "list"

    # The empty serie and null are distinct states.
    assert scalar.Int64Serie([]).is_empty() is True
    assert scalar.Int64Serie.null().is_null() is True
    ```

=== "Node"

    ```js
    const { scalar } = require('yggdryl')

    const numbers = new scalar.Int64Serie([1n, 2n, 3n])
    assert.deepEqual([numbers.isNull(), numbers.isEmpty(), numbers.len()], [false, false, 3])
    assert.deepEqual(numbers.toArray(), [1n, 2n, 3n])
    assert.equal(numbers.getAt(1), 2n)                 // the native value
    assert.equal(numbers.getScalarAt(2).value(), 3n)   // ... or the element scalar
    assert.equal(numbers.getScalarAt(3), null)         // out of bounds
    assert.equal(numbers.dataType().name(), 'list')

    // The empty serie and null are distinct states.
    assert.equal(new scalar.Int64Serie([]).isEmpty(), true)
    assert.equal(scalar.Int64Serie.null().isNull(), true)
    ```

=== "Rust"

    ```rust
    use yggdryl_scalar::{Int64Scalar, Int64Serie, Scalar};

    fn main() {
        let numbers = Int64Serie::from(vec![1, 2, 3]);
        assert_eq!((numbers.is_null(), numbers.is_empty(), numbers.len()), (false, false, 3));
        assert_eq!(numbers.values(), Some(&[1, 2, 3][..])); // borrows the Arrow buffer
        assert_eq!(numbers.get_at::<i64>(1).unwrap(), 2);   // the native value
        assert_eq!(numbers.get_scalar_at(2), Some(Int64Scalar::new(3))); // the element scalar
        assert_eq!(numbers.data_type().name(), "list");

        // The empty serie and null are distinct states.
        assert!(Int64Serie::default().is_empty());
        assert!(Int64Serie::null().is_null());
    }
    ```

The generic `Serie`, per-element nulls and the Arrow / IO surface stay Rust-only:

!!! note "Rust only"
    The generic `Serie` / `MapScalar` / `StructScalar`, the struct-row series
    `StructSerie` / `TypedStructSerie` and the type-erased `AnySerie` / `AnyScalar`
    holders are generic over their child types (or carry dynamic Arrow fields), and
    the concrete series' `to_arrow_scalar` / `from_arrow`, `to_arrow_array` / `nulls`
    and `from_io` / `pwrite_io` share raw Arrow buffers or borrow a second IO
    resource at once — none crosses the FFI boundary yet, so from a binding a serie
    is a dense (all-valid) serie.

```rust
use yggdryl_scalar::yggdryl_dtype as dtype;
use yggdryl_scalar::{Int64Scalar, Int64Serie, Scalar, TypedSerie};

fn main() {
    // The generic TypedSerie carries per-element nulls and round-trips through Arrow.
    let numbers = TypedSerie::new(vec![Int64Scalar::new(1), Int64Scalar::null()]);
    assert_eq!(numbers.get_scalar_at(1), Some(Int64Scalar::null()));
    assert_eq!(numbers.get_at::<i64>(0).unwrap(), 1); // the native value, any target
    let arrow = numbers.to_arrow_scalar(); // a one-element ListArray sharing the elements
    assert_eq!(TypedSerie::from_arrow(arrow.as_ref()).unwrap(), numbers);

    // Int64Serie shares the same buffers across the Arrow boundary, zero-copy.
    let fast = Int64Serie::from(vec![1, 2, 3]);
    assert_eq!(Int64Serie::from_arrow(fast.to_arrow_scalar().as_ref()).unwrap(), fast);

    // The type parameters name the dtype-layer types.
    let _: TypedSerie<dtype::Int64Type, Int64Scalar> = TypedSerie::default();
}
```

## The trait layers

- **`Scalar`** — the untyped base: a single, possibly-null value carrying its
  data type as the associated `DataType` (`data_type`, `is_null`, `value` of an
  associated `Value: ?Sized`);
  `to_arrow_scalar` / `from_arrow` mirror a one-element `arrow_array` array, and
  `to_arrow_array` hands back the value's Arrow **array** form — for a plain scalar
  the same one-element array (a scalar *is* a length-1 array), for a serie its
  element array instead. The `as_*`
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
- **`TypedScalar<DT: DataType, T, ArrowScalar, ArrowArray = ArrowScalar>: Scalar<DataType = DT, Value = T>`**
  — the typed layer: a scalar whose value is `T` (possibly unsized: a string
  scalar exposes `Option<&str>`), naming the concrete Apache Arrow array types it
  produces — `ArrowScalar` (the `to_arrow_scalar` form) and `ArrowArray` (the
  `to_arrow_array` form, defaulting to `ArrowScalar`; a serie splits them, its
  scalar form a `ListArray` and its array form the element `PrimitiveArray`).
- **`FromScalar`** — the native Rust targets readable out of any scalar, behind
  the generic accessors such as `Serie::get_at::<T>`.
- **`ScalarFactory<T>: TypedDataType<T>`** — the factory: a typed data type builds
  its scalar. The scalar layer builds on the data types, never the other way
  around, so the "data type → scalar" factory lives here (implemented for every
  `yggdryl-dtype` type next to its scalar): `scalar(value)` holds a native value,
  `null_scalar()` the null scalar, and `default_scalar()` the type's default — a
  scalar holding `default_value`, except where the scalar models nullness (an
  optional defaults to its null variant, matching the scalar's own `Default`).
```
