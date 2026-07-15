# Typed data — fixed-width

The core `io` module is organized to **serialize all types**, split by value shape:

- **`io::fixed`** — fixed-width primitives (`u8`, `i32`, `f64`, …), one byte width per value.
- **`io::var`** — variable-length types ([strings and binary](var.md)); the sibling that builds
  on the same foundation.

!!! note "Python & Node"

    The **`Scalar` and `Serie`** value/column types are mirrored in Python and Node — one class
    per primitive under `yggdryl.types` (`I32Scalar` / `I32Serie`, `U256Scalar` / `U256Serie`,
    `F64Scalar` / `F64Serie`, …); see [In Python and Node](#in-python-and-node) below for the
    three-language API. The **schema** layer they rest on — `DataType`, `Field`, and its
    [`Headers`](../guide/headers.md) metadata — is mirrored too (see [Schema layer](schema.md)). The typed
    `Buffer<T>` (`I32Buffer` … `F64Buffer`, a raw non-nullable values store) and numeric
    [casting](../guide/converter.md) are mirrored as well.

Each fixed-width type `T` gets the same Arrow-style stack of value types, all generic over `T`
and built on the byte-I/O [`Buffer`](../guide/io.md):

| Type | Role |
| --- | --- |
| `PrimitiveType<T>` / `DataType` | the typed / erased **type descriptor** (name + byte width) |
| `TypedField<T>` / `Field` | a **named, nullable column** descriptor |
| `Scalar<T>` | **one nullable value**, with an `IOCursor` byte codec |
| `Buffer<T>` | contiguous **storage** + byte I/O — `U8Buffer` is [`Bytes`](../guide/io.md) |
| `Serie<T>` | a **nullable column** — a validity bitmap over a values `Buffer` |

`Buffer<u8>` **is** the project's byte buffer: `U8Buffer`, aliased `Bytes`, the type the
Python/Node `yggdryl.io.Bytes` wraps. The typed columnar types `Scalar` / `Serie` (and the
`DataType` / `Field` descriptors) are mirrored in all three languages; see
[In Python and Node](#in-python-and-node).

## The generic trait hierarchy

Each concrete type above sits under a **generic trait hierarchy**. The **root traits** are
family-agnostic and live at the [`io`](../guide/io.md) root; each concrete family adds its own sub-trait
that pre-implements the shared logic as default methods — `Fixed*` here, `Var*` in
[`io::var`](var.md) — so a concrete type supplies only 2–3 primitives. The families depend
**downward** on the roots, never sideways on each other.

| root trait (`io`) | fixed sub-trait | var sub-trait (`io::var`) | concrete (fixed) |
| --- | --- | --- | --- |
| `DataType` / `TypedDataType` | `FixedDataType` | `VarDataType` | `PrimitiveType<T>` |
| `FieldType` | `FixedField` | `VarField` | `Field` / `TypedField<T>` |
| `ScalarType` | `FixedScalar` | `VarScalar` | `Scalar<T>` |
| `BufferType` | `FixedBuffer` | — | `Buffer<T>` |
| `SerieType` | `FixedSerie` | `VarSerie` | `Serie<T>` |

So a function can be generic over `impl SerieType`, erase a descriptor to `&dyn DataType` or a
field to `&dyn FieldType`, and the `Fixed*` defaults (`data_type()`, `serialized_width()`, …)
come for free. (There is deliberately no `VarBuffer`: a variable column's physical storage is
an offsets buffer + a data buffer held inside its `Serie`, not a standalone typed buffer.)

## Category drill-down — `is_integer()`, not `match`, via `DataTypeId`

The concrete types are enumerated in exactly one place: `DataTypeId`, a `#[repr(u16)]` enum laid
out so each category is a **contiguous integer range** (unsigned `0x10–0x1F`, signed `0x20–0x2F`,
float `0x30–0x3F`, …) with reserved gaps for future types. Every descriptor reports its
`type_id()`, and each `is_*` predicate is **one or two `u16` range checks** — so a caller
classifies a type, fixed or variable, with a cheap inlinable predicate instead of matching the
concrete type. The same predicates are on `Field` / `FieldType`, so an erased schema drills down
without the original descriptor; the coarse `DataTypeCategory` is *derived* from the id.

```rust
use yggdryl_core::io::DataType;
use yggdryl_core::io::fixed::PrimitiveType;

let i32 = PrimitiveType::<i32>::new();
assert!(i32.is_integer() && i32.is_signed() && i32.is_numeric());
assert!(i32.is_fixed_width() && !i32.is_variable_length());

let f64 = PrimitiveType::<f64>::new();
assert!(f64.is_floating() && f64.is_signed() && !f64.is_integer());
```

## The numeric family — widths 1 → 32 bytes

The full set ships: **unsigned** `u8`/`u16`/`u32`/`u64`/`u96`/`u128`/`u256`, **signed**
`i8`/`i16`/`i32`/`i64`/`i96`/`i128`/`i256`, and **floats** `f16`/`f32`/`f64` — each with its
`Buffer` / `Scalar` / `Serie` / `Field` / `DataType` (`U16Serie`, `I256Scalar`, `F16Buffer`, …).
The little-endian codec and the null-aware column work identically across every width.

- `u8`…`i64`, `u128`/`i128`, `f32`/`f64` use the Rust primitive; **`f16`** is `half::f16`,
  re-exported as `fixed::f16`.
- `u96`/`i96`/`u256`/`i256` have no Rust *or* Arrow primitive, so they are
  `#[repr(transparent)]` little-endian `[u8; N]` newtypes — byte-canonical `Eq`/`Hash`, **no
  `Ord`** (LE byte order isn't numeric order), constructed via `U256::from_le_bytes([…])`.

**Value identity is bit-canonical**: `Scalar`/`Serie` compare and hash their *canonical
little-endian bytes*, not the element's `==` — so the float types are fully `Eq` + `Hash` (usable
as map keys) even though `f16`/`f32`/`f64` are not. This deliberately diverges from IEEE `==`:
`NaN == NaN` when the bits match, and `+0.0 != -0.0`.

## Layout — a type is five thin files

A concrete type lives under `io/fixed/<type>/` as five macro-declared files over the shared
generics — so a new primitive is a handful of lines:

```text
io/fixed/
  dtype.rs field.rs scalar.rs serie.rs buffer.rs   # the generic building blocks (once)
  u8/   dtype.rs …   # fixed_native!(u8, "u8"); pub type U8Scalar = Scalar<u8>; …
  i32/ i64/ u16/ f64/ …   # one directory per primitive
```

```rust
// io/fixed/i32/dtype.rs — the whole per-type "definition" is the native codec + aliases
use crate::io::fixed::{fixed_dtype, fixed_native};
fixed_native!(i32, "i32");     // impl NativeType for i32 (little-endian codec)
fixed_dtype!(I32DataType, i32); // pub type I32DataType = PrimitiveType<i32>;
```

## Buffer and scalar

`Buffer<T>` holds `T` values contiguously (little-endian). It has two length notions —
`count()` (elements) and `len()` (bytes, the `IOBase` contract) — and the full byte-I/O
family, so it doubles as raw serialized bytes. `Scalar<T>` is one nullable value that reads
and writes through the [`IOCursor`](../guide/io.md) abstraction.

```rust
use yggdryl_core::io::fixed::{Buffer, Scalar};
use yggdryl_core::io::{Bytes, IOBase, IOCursor};

let mut b = Buffer::<i32>::from_vec(vec![1, 2, 3]);
assert_eq!(b.count(), 3);   // 3 elements
assert_eq!(b.len(), 12);    // 12 bytes
b.push(4);
assert_eq!(b.get(3), Some(4));
assert_eq!(b.as_slice(), &[1, 2, 3, 4]); // zero-copy typed view

// A scalar round-trips through any byte sink (a Bytes, a file, …).
let mut sink = Bytes::new();
Scalar::of(42i32).write_to(&mut sink).unwrap();
sink.rewind();
assert_eq!(Scalar::<i32>::read_from(&mut sink).unwrap().value(), Some(42));
```

## Serie — a nullable column

`Serie<T>` is an Arrow-style column: a values `Buffer` plus an **optional** validity bitmap
(absent when there are no nulls, so a dense column pays nothing). It serializes as
`[len][flags][validity?][values]` through an `IOCursor`.

```rust
use yggdryl_core::io::fixed::Serie;
use yggdryl_core::io::{Bytes, IOCursor};

// Build in one pass from options (a null lazily materializes the bitmap).
let col = Serie::from_options(&[Some(1i32), None, Some(3)]);
assert_eq!(col.len(), 3);
assert_eq!(col.null_count(), 1);
assert_eq!(col.get(1), None);
assert_eq!(col.to_options(), vec![Some(1), None, Some(3)]);

// The whole column round-trips through a byte sink.
let mut sink = Bytes::new();
col.write_to(&mut sink).unwrap();
sink.rewind();
assert_eq!(Serie::<i32>::read_from(&mut sink).unwrap(), col);
```

Reads are zero-copy (an element decode touches no heap), and bulk construction
(`from_values` / `from_options`) builds the values in a single allocation — prefer it to a
`push` loop, which re-seals the immutable buffer per element (see the
[benchmark notes](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/fixed.md)).

## In-place set — single and bulk

Every `Serie` family (fixed numeric, [decimal](decimal.md), the fixed-size and variable-length byte
columns) can **read and overwrite an existing element** — `get` / `get_scalar` to read, `set` /
`set_scalar` to write one index (a `Some` value or a `None` null, transitioning the validity mask),
and the **bulk** `set_range` (from another `Serie`), `set_scalars` (from `&[Scalar]`), and
`set_values` (from native values) to overwrite a contiguous run. Out-of-range indices return a
guided `IndexOutOfBounds` and leave the column unchanged (`set` overwrites; `push` grows).

```rust
use yggdryl_core::io::fixed::{Scalar, Serie};

let mut col = Serie::from_values(&[0i32; 6]);
col.set(0, Some(10)).unwrap();                                    // one value
col.set(1, None).unwrap();                                        // one null
col.set_range(2, &Serie::from_options(&[Some(7), None])).unwrap();// from another column
col.set_scalars(4, &[Scalar::of(40), Scalar::null()]).unwrap();   // from scalars
assert_eq!(col.to_options(), [Some(10), None, Some(7), None, Some(40), None]);
assert!(col.set(9, Some(0)).is_err());                            // out of bounds
```

The **bulk** setters are the fast path: they commit the whole run in a **single** copy-on-write of
the values buffer, not one re-seal per element — a 33–67× throughput win over a `set` loop on the
Arc-backed columns (`Serie<T>`, `DecimalSerie<B>`). The `Vec`-backed byte columns overwrite in
place. See the [access benchmark](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/access.md);
the variable-length case, whose `set` rewrites offsets, is covered in [Typed data — variable](var.md#in-place-set--the-offset-rewrite).

## A column is usable as a scalar

`Serie` and `Scalar` interoperate: a column hands out scalars (`get_scalar`), a length-1
column **is** a scalar (`as_scalar`), and a scalar broadcasts to a length-1 column.

```rust
use yggdryl_core::io::fixed::{Scalar, Serie};

let col = Serie::from_options(&[Some(1i32), None, Some(3)]);
assert_eq!(col.get_scalar(1), Scalar::null());          // a null element -> null scalar
assert_eq!(Serie::from_values(&[42i32]).as_scalar(), Some(Scalar::of(42)));
assert_eq!(col.as_scalar(), None);                       // length ≠ 1 -> not a scalar

let broadcast = Scalar::of(7i32).to_serie();             // scalar -> length-1 column
assert_eq!(broadcast.as_scalar(), Some(Scalar::of(7)));  // ...and back
```

## In Python and Node

Every primitive's `Scalar` and `Serie` is a class under `yggdryl.types` — `I32Scalar` /
`I32Serie`, `U256Scalar` / `U256Serie`, `F64Scalar` / `F64Serie`, and so on for all 17 widths. A
`Scalar` is an **immutable value** (hashable/equatable, pickles/`serializeBytes` through its byte
codec); a `Serie` is a **mutable column** (so, like `bytearray`/`dict`, it is unhashable) with
`len()`/indexing/iteration. A `None` / `null` element is a null throughout, and the byte codec
(`serialize_bytes` / `serializeBytes`) round-trips exactly — the same bytes the Rust `write_to`
produces.

**Values marshal by width.** The small integers (`u8`…`u32`, `i8`…`i32`) cross as a native
`int` / `number`; the wide integers (`u64`/`i64`/`u128`/`i128`) as an exact **decimal string**;
the 96/256-bit integers (`u96`/`i96`/`u256`/`i256`), which have no cross-language numeric form,
as their **little-endian bytes**; and the floats (`f16`/`f32`/`f64`) as a native `float` /
`number`.

=== "Python"

    ```python
    from yggdryl.types import I32Scalar, I32Serie, U64Scalar

    # A scalar: one nullable value.
    s = I32Scalar(-5)
    assert s.value == -5 and not s.is_null and s.type_name == "i32"
    assert I32Scalar().is_null and I32Scalar(None).is_null
    assert I32Scalar.deserialize_bytes(s.serialize_bytes()) == s   # byte codec
    assert U64Scalar(2**63).value == "9223372036854775808"         # wide int -> decimal string

    # A serie: one nullable column.
    col = I32Serie([1, None, 3])
    assert len(col) == 3 and col.null_count == 1
    assert col.to_options() == [1, None, 3] and list(col) == [1, None, 3]
    col.push(4); col.set(1, 20)
    assert col[1] == 20 and col[-1] == 4                           # indexing, negatives allowed
    assert I32Serie.deserialize_bytes(col.serialize_bytes()) == col

    # Scalar <-> column interop, and a Field descriptor.
    assert col.get_scalar(2) == I32Scalar(3)
    assert I32Serie([7]).as_scalar() == I32Scalar(7)
    assert col.to_field("id").nullable is False                    # inferred from the (now zero) nulls
    ```

=== "Node"

    ```js
    const { I32Scalar, I32Serie, U64Scalar } = require('yggdryl').types

    // A scalar: one nullable value.
    const s = new I32Scalar(-5)
    assert(s.value === -5 && !s.isNull && s.typeName === 'i32')
    assert(I32Scalar.null().isNull && new I32Scalar(null).isNull)
    assert(I32Scalar.deserializeBytes(s.serializeBytes()).equals(s))   // byte codec
    assert(new U64Scalar((2n ** 63n).toString()).value === '9223372036854775808') // wide int -> string

    // A serie: one nullable column.
    const col = new I32Serie([1, null, 3])
    assert(col.length === 3 && col.nullCount === 1)
    assert(JSON.stringify(col.toOptions()) === JSON.stringify([1, null, 3]))
    col.push(4); col.set(1, 20)
    assert(col.get(1) === 20)
    assert(I32Serie.deserializeBytes(col.serializeBytes()).equals(col))

    // Scalar <-> column interop, and a Field descriptor.
    assert(col.getScalar(2).equals(new I32Scalar(3)))
    assert(I32Serie.fromValues([7]).asScalar().equals(new I32Scalar(7)))
    assert(col.toField('id').nullable === false)                       // inferred from the nulls
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::fixed::{Scalar, Serie};

    // A scalar: one nullable value, with a Vec byte codec.
    let s = Scalar::of(-5i32);
    assert_eq!(s.value(), Some(-5));
    assert_eq!(Scalar::<i32>::deserialize_bytes(&s.serialize_bytes()).unwrap(), s);

    // A serie: one nullable column.
    let mut col = Serie::from_options(&[Some(1i32), None, Some(3)]);
    assert_eq!((col.len(), col.null_count()), (3, 1));
    col.push(Some(4));
    col.set(1, Some(20)).unwrap();
    assert_eq!(Serie::<i32>::deserialize_bytes(&col.serialize_bytes()).unwrap(), col);

    // Scalar <-> column interop.
    assert_eq!(col.get_scalar(2), Scalar::of(3));
    assert_eq!(Serie::from_values(&[7i32]).as_scalar(), Some(Scalar::of(7)));
    ```

## Optimized descriptors

Type descriptors are zero-sized, so the accessors are free. `PrimitiveType<T>` exposes its
name and width as **compile-time constants** and `const fn`s, and every value type hands back
its `data_type()` (`const`) and a `field(name, nullable)`; `Field::is::<T>()` is a
`&'static str` pointer compare.

```rust
use yggdryl_core::io::DataType;
use yggdryl_core::io::fixed::{Serie, PrimitiveType};

const WIDTH: usize = PrimitiveType::<i64>::BYTE_WIDTH; // 8, at compile time
assert_eq!(PrimitiveType::<i64>::NAME, "i64");

let col = Serie::from_options(&[Some(1i32), None]);
assert_eq!(col.data_type().name(), "i32");
assert!(col.to_field("c").nullable());                  // nullability inferred from the nulls
assert!(col.data_type().is_fixed_width());
let _ = col.field("c", false);                          // or set nullability explicitly
```

## Arrow interop

With the **`arrow`** feature the fixed family converts to and from Arrow by **sharing the
allocation** — a `Buffer` / `Serie` becomes an `arrow_array::PrimitiveArray` (and back) with an
`Arc` bump for the native subset (`u8`…`i64`, `f16`/`f32`/`f64`), while the wider widths take the
total closest-fit `to_arrow()` schema mapping (`Decimal128`/`Decimal256` for `i128`/`i256`,
`FixedSizeBinary(N)` for `u128`/`u96`/`i96`/`u256`). **Arrow interop →
[see Arrow interop → Primitives](../arrow/primitives.md)** for the zero-copy `ArrowNative`
capability, the closest-fit fallback, and the three-language reference.

## Fixed-size byte types — `FixedBinary` / `FixedUtf8`

Beside the compile-time-width primitives, `io::fixed` ships a **runtime-`N`** byte family
(Arrow's `FixedSizeBinary(N)`) in the `binary` and `string` sub-modules: every value is exactly
`N` bytes, `N` chosen at construction. Structurally it is the [variable-length family](var.md)
without the offsets — a flat `N`-byte-slot data buffer over a validity bitmap — so it implements
the root traits directly (over the shared `FixedSize*` generics) rather than `NativeType`. It is
**both** fixed-width and binary/utf8 — the `DataTypeId` ranges classify it correctly on both
axes.

```rust
use yggdryl_core::io::DataType;
use yggdryl_core::io::fixed::{FixedBinarySerie, FixedUtf8Scalar, FixedUtf8Type};

// A column of 2-byte values; `push` enforces the width and validates the kind.
let mut col = FixedBinarySerie::new(2);
col.push(Some(&[1, 2])).unwrap();
col.push(None).unwrap();               // a null keeps its slot
assert!(col.push(Some(&[9, 9, 9])).is_err()); // wrong width is rejected
assert_eq!(col.get_bytes(0), Some(&[1, 2][..]));

// Fixed-size UTF-8 validates each value and reads back as &str.
assert_eq!(FixedUtf8Scalar::of("ok").as_str(), Some("ok"));
let dt = FixedUtf8Type::new(4);
assert!(dt.is_fixed_width() && dt.is_utf8()); // dual classification
```

Arrow has no fixed-size UTF-8 type, so `FixedUtf8` maps to `FixedSizeBinary(N)` (the bytes
round-trip; only the schema tag is coarser).

## The null type — 0-width, all-null

Arrow's `Null` is a type whose *every* value is null, at **zero** storage: `NullType` is 0-width,
a `NullScalar`'s wire form is empty, and a `NullSerie` is just its length (no value buffer, no
validity mask). Like the fixed-size byte family it has no `NativeType`, so it implements the root
traits directly. It sits at the bottom of the type lattice — any type casts *to* and *from* it
(see [casting](../guide/converter.md)). The `NullScalar` / `NullSerie` value types and the `DataType.null()`
descriptor are reachable in all three languages.

=== "Python"

    ```python
    from yggdryl.types import NullScalar, NullSerie, DataType

    assert NullScalar().is_null and NullScalar().value is None
    assert NullScalar().data_type == DataType.null()
    assert NullScalar().serialize_bytes() == b""     # empty wire form

    col = NullSerie(2)
    col.push()                                        # one more null
    assert len(col) == 3 and col.null_count == 3      # all null, no storage
    assert col[0] is None and col.get_scalar(0) == NullScalar()
    ```

=== "Node"

    ```js
    const { NullScalar, NullSerie, DataType } = require('yggdryl').types

    assert(new NullScalar().isNull && new NullScalar().value === null)
    assert(new NullScalar().dataType.equals(DataType.null()))
    assert(new NullScalar().serializeBytes().length === 0)   // empty wire form

    const col = new NullSerie(2)
    col.push()                                               // one more null
    assert(col.length === 3 && col.nullCount === 3)          // all null, no storage
    assert(col.get(0) === null && col.getScalar(0).equals(new NullScalar()))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::DataType;
    use yggdryl_core::io::fixed::{NullScalar, NullSerie, NullType};

    assert_eq!(NullType::new().byte_width(), 0);
    assert!(NullScalar::null().is_null());
    assert!(NullScalar::null().serialize_bytes().is_empty());

    let mut col = NullSerie::with_len(2);
    col.push();                              // one more null
    assert_eq!((col.len(), col.null_count()), (3, 3)); // all null, no storage
    ```

## Field metadata — safe, lossless Arrow round-trips

Every field carries [`Headers`](../guide/headers.md) — the centralized, ordered, case-insensitive
key/value map — as its metadata, mirroring Arrow's `Field::metadata`. Attach it with
`with_metadata` / `with_metadata_entry`.

Metadata is also what makes `to_arrow` / `from_arrow` **safe**: because the Arrow data-type
mapping is lossy and *non-injective* (`u96`, `i96`, `FixedUtf8`, and a runtime-`N` `FixedBinary`
all collapse to `FixedSizeBinary(N)`), `to_arrow` records the exact logical type under a reserved
`"yggdryl.logical_type"` key when the plain mapping is ambiguous, and `from_arrow` uses it to
recover the precise type. **Arrow interop →
[see Arrow interop → Metadata & round-tripping](../arrow/metadata.md)** for the reserved keys and
the full round-trip story.
