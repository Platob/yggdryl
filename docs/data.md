# Data model

The `yggdryl-data` crate is the Apache Arrow-centralized **data-model layer**, built
on `yggdryl-core`. It defines the physical type system — data types, fields and
scalars — for zero-copy FFI and Arrow interop. The concrete families so far: the
`integer` module (every signed and unsigned integer), the
[`binary` module](#the-binary-module) (the variable-size byte type, its scalar a
core positioned-IO resource), the `null` module (the storage-free null type), the
`union` module, the `optional` module (the logical null-or-value type over union
storage) and the nested `list`, `map` and `struct` modules; more land as the
layer grows.

The bindings expose the layer as `yggdryl.data` (Python) and `yggdryl.data` (Node),
adapting to idioms: Node carries 8–32 bit values as `number` and the 64-bit types as
`BigInt`, byte values cross as Python `bytes` / JS `Buffer`, the null-or-value
scalars are concrete per-type classes (`OptionalInt64`,
`OptionalBinary`, …) built straight from the native value, and the `as_*`
accessors surface the core `DataError` as a raised `ValueError` (Python) / thrown
`Error` (Node). Four things stay
**Rust-only**, stated here and in both binding module docs: the [Arrow
interop](#arrow-interop) surface (`to_arrow` / `from_arrow` exchange `arrow-schema` /
`arrow-array` values that cannot cross the FFI boundary), construction of a `UnionType`
from arbitrary child fields (reached in the bindings through an optional data type's
`storage()`), the [`DataTypeId`](#type-ids) classifier, and the
[nested families](#nested-types-list-map-and-struct) (the generic `ListType` /
`MapType` / `StructType` with their scalars, the per-family trait pairs, and the
buffer-backed `Int64Serie`, whose zero-copy Arrow buffers await C Data Interface
interop), which have no concrete FFI shape yet.

The type system is three layers of traits. None carries a lifetime parameter
(FFI-clean); the untyped base is `Debug + Send + Sync` so schemas are printable and
shareable across threads and FFI, and `RawDataType` is object-safe for
`Box<dyn RawDataType>` schemas.

## The concrete types: the `integer` module

The `integer` module holds every Apache Arrow signed and unsigned integer — `Int8Type` …
`Int64Type`, `UInt8Type` … `UInt64Type` — one module per type, one file per concern (`data_type`,
`field`, `scalar`). Each is a fixed-width [primitive](#categories) with a little-endian
byte codec, a nullable field and a possibly-null scalar; they share one shape, so a
single crate-internal macro generates each per-type file.

`Int64Type`, native Rust `i64`, is stored little-endian in eight bytes (Arrow C Data
Interface format `"l"`). Scalars are built from their native value and read through
the `as_*` accessors: direct for the scalar's own type, exact conversion otherwise,
and an actionable error — Rust `DataError`, Python `ValueError`, a thrown JS
`Error` — when the scalar is null or the value is not exactly representable:

=== "Python"

    ```python
    from yggdryl import data

    int64 = data.Int64Type()
    assert int64.name() == "int64"
    assert int64.arrow_format() == "l"
    assert (int64.byte_width(), int64.bit_width()) == (8, 64)

    # The codec bridging a native value to and from Arrow bytes.
    assert int64.native_to_bytes(-1) == b"\xff" * 8
    assert int64.native_from_bytes(b"\xff" * 8) == -1

    # A named, nullable column of int64.
    id_field = data.Int64Field("id", False)
    assert (id_field.name(), id_field.is_nullable()) == ("id", False)

    # A single i64 value, or null, with exact-or-raise accessors.
    scalar = data.Int64(42)
    assert scalar.value() == 42
    assert scalar.as_i8() == 42          # converted access
    try:
        scalar.as_str()                  # an int64 is not a string
    except ValueError as error:
        assert "no str conversion" in str(error)
    assert data.Int64.null().is_null()
    ```

=== "Node"

    ```js
    const { data } = require('yggdryl')

    const int64 = new data.Int64Type()
    assert.equal(int64.name(), 'int64')
    assert.equal(int64.arrowFormat(), 'l')
    assert.deepEqual([int64.byteWidth(), int64.bitWidth()], [8, 64])

    // The codec bridging a native value to and from Arrow bytes (BigInt for 64-bit).
    assert.deepEqual([...int64.nativeToBytes(-1n)], Array(8).fill(0xff))
    assert.equal(int64.nativeFromBytes(Buffer.alloc(8, 0xff)), -1n)

    // A named, nullable column of int64.
    const idField = new data.Int64Field('id', false)
    assert.deepEqual([idField.name(), idField.isNullable()], ['id', false])

    // A single i64 value, or null, with exact-or-throw accessors.
    const scalar = new data.Int64(42n)
    assert.equal(scalar.value(), 42n)
    assert.equal(scalar.asI8(), 42)      // converted access
    assert.throws(() => scalar.asStr(), /no str conversion/) // not a string
    assert.equal(data.Int64.null().isNull(), true)
    ```

=== "Rust"

    ```rust
    use yggdryl_data::{DataType, Int64Type, Int64Field, Int64, RawDataType, RawField, RawScalar};

    fn main() {
        assert_eq!(Int64Type.name(), "int64");
        assert_eq!(Int64Type.arrow_format(), "l");
        assert_eq!((Int64Type.byte_width(), Int64Type.bit_width()), (Some(8), Some(64)));

        // The codec bridging a native value to and from Arrow bytes.
        assert_eq!(Int64Type.native_to_bytes(&-1), vec![0xFF; 8]);
        assert_eq!(Int64Type.native_from_bytes(&[0xFF; 8]).unwrap(), -1);

        // A named, nullable column of int64.
        let id = Int64Field::new("id", false);
        assert_eq!((id.name(), id.is_nullable()), ("id", false));

        // A single i64 value, or null, with exact-or-error accessors.
        let scalar = Int64::from(42);
        assert_eq!(scalar.value(), Some(&42));
        assert_eq!(scalar.as_i8().unwrap(), 42); // converted access
        assert!(scalar.as_str().is_err()); // an int64 is not a string
        assert!(Int64::null().is_null());
    }
    ```

The other widths follow the same surface — swap `Int64Type` / `i64` / `"l"` for
`Int8Type` / `i8` / `"c"`, `UInt32Type` / `u32` / `"I"`, and so on. In Rust, `Int64Type::ID`
names the matching [`DataTypeId`](#type-ids) classifier.

## The `binary` module

The `binary` module holds the variable-size byte type: `BinaryType` (Arrow C Data
Interface format `"z"`, no fixed width), `BinaryField` and `Binary`. The
typed codec is the identity — any byte sequence is a valid `binary` value — and
the scalar holds its bytes as a `yggdryl-core` positioned-IO `ByteBuffer`, so a
value plugs straight into the core IO layer: in Rust, `io()` borrows the resource
for `RawIOBase` reads and `into_io()` moves it out to wrap in the `RawIOCursor` /
`RawIOSlice` adapters; the bindings hand back a `yggdryl.core` `ByteBuffer`
through `to_io()` (one copy at the FFI boundary, like strings). `as_bytes`
answers the native type directly and `as_str` converts when the bytes are valid
UTF-8.

=== "Python"

    ```python
    from yggdryl import core, data

    binary = data.BinaryType()
    assert (binary.name(), binary.arrow_format()) == ("binary", "z")
    assert binary.byte_width() is None  # variable width
    assert binary.native_from_bytes(binary.native_to_bytes(b"\x01\x02")) == b"\x01\x02"

    blob = data.Binary(b"\x01\x02\x03")
    assert blob.value() == b"\x01\x02\x03"
    assert blob.as_bytes() == b"\x01\x02\x03"
    assert data.Binary(b"hi").as_str() == "hi"  # valid UTF-8 only

    # The value doubles as a core positioned-IO ByteBuffer.
    io = blob.to_io()
    assert io.pread_byte_one(1, core.Whence.Start) == 2

    assert data.Binary.null().is_null()
    assert data.OptionalBinary(b"hi").as_bytes() == b"hi"
    ```

=== "Node"

    ```js
    const { core, data } = require('yggdryl')

    const binary = new data.BinaryType()
    assert.deepEqual([binary.name(), binary.arrowFormat()], ['binary', 'z'])
    assert.equal(binary.byteWidth(), null) // variable width
    assert.deepEqual(
      binary.nativeFromBytes(binary.nativeToBytes(Buffer.from([1, 2]))),
      Buffer.from([1, 2]),
    )

    const blob = new data.Binary(Buffer.from([1, 2, 3]))
    assert.deepEqual(blob.value(), Buffer.from([1, 2, 3]))
    assert.deepEqual(blob.asBytes(), Buffer.from([1, 2, 3]))
    assert.equal(new data.Binary(Buffer.from('hi')).asStr(), 'hi') // valid UTF-8 only

    // The value doubles as a core positioned-IO ByteBuffer.
    const io = blob.toIo()
    assert.equal(io.preadByteOne(1, core.Whence.Start), 2)

    assert.equal(data.Binary.null().isNull(), true)
    assert.deepEqual(new data.OptionalBinary(Buffer.from('hi')).asBytes(), Buffer.from('hi'))
    ```

=== "Rust"

    ```rust
    use yggdryl_data::yggdryl_core::{RawIOBase, RawIOCursor, Whence};
    use yggdryl_data::{BinaryType, Binary, DataType, RawDataType, RawScalar};

    fn main() {
        assert_eq!((BinaryType.name(), BinaryType.arrow_format().as_str()), ("binary", "z"));
        assert_eq!(BinaryType.byte_width(), None); // variable width
        let bytes = BinaryType.native_to_bytes(&vec![1, 2]);
        assert_eq!(BinaryType.native_from_bytes(&bytes).unwrap(), vec![1, 2]);

        let blob = Binary::new(vec![1, 2, 3]);
        assert_eq!(blob.value(), Some(&[1, 2, 3][..]));
        assert_eq!(blob.as_bytes().unwrap(), &[1, 2, 3][..]); // borrowed, never copied
        assert_eq!(Binary::new(b"hi".to_vec()).as_str().unwrap(), "hi");

        // The value doubles as a core positioned-IO resource.
        let io = blob.io().unwrap();
        assert_eq!(io.pread_byte_one(1, Whence::Start).unwrap(), 2);
        let cursor = RawIOCursor::new(blob.clone().into_io().unwrap());
        assert_eq!(cursor.pread_byte_array(0, Whence::Start, 2).unwrap(), vec![1, 2]);

        assert!(Binary::null().is_null());
    }
    ```

In Rust the Arrow round trip is exact — `to_arrow` builds a one-element
`BinaryArray`, `from_arrow` is its inverse — and `BinaryType::ID` is
`DataTypeId::Binary`. BinaryType is *not* a `Primitive` in this model's fixed-width
sense: it is Arrow's variable-size binary layout, childless but without a fixed
width. `yggdryl-core` is re-exported as `yggdryl_data::yggdryl_core`, so the IO
surface is reachable at the exact version the crate was built against.

## Arrow interop

!!! note "Rust only"
    `to_arrow` / `from_arrow` exchange `arrow-schema` / `arrow-array` values, which
    cannot cross the FFI boundary — the bindings will gain this surface through the
    Arrow C Data Interface as it lands.

Every layer converts to and from its Apache Arrow equivalent with a `to_arrow` /
`from_arrow` pair (`from_arrow` is the exact inverse of what `to_arrow` produces,
refusing a mismatched Arrow value with `DataError`): a data type mirrors an
`arrow_schema::DataType`, a field an `arrow_schema::Field`, and a scalar Arrow's own
scalar representation — a one-element `arrow_array` array, null when the scalar is
null. The `arrow-schema` and `arrow-array` subset crates are re-exported from the
crate root so downstream code uses the exact versions the crate was built against.

Field metadata is handled in two tiers: an extension-typed Arrow field (one carrying
an `ARROW:extension:name` metadata entry) is a *different* logical type and is
refused with `DataError::IncompatibleArrowType`, while any other metadata is not part
of the model — a field is exactly a name, a data type and a nullability flag — and is
deliberately dropped on the way in (logged as a `warn` when the `log` cargo feature
is on; `to_arrow` correspondingly always produces a metadata-free field).

```rust
use yggdryl_data::{arrow_array::Array, arrow_schema, Int64Type, Int64Field, Int64};
use yggdryl_data::{RawDataType, RawField, RawScalar};

fn main() {
    // Data type ↔ arrow_schema::DataType.
    assert_eq!(Int64Type.to_arrow(), arrow_schema::DataType::Int64);
    assert!(Int64Type::from_arrow(&arrow_schema::DataType::Utf8).is_err());

    // Field ↔ arrow_schema::Field.
    let id = Int64Field::new("id", false);
    assert_eq!(Int64Field::from_arrow(&id.to_arrow()).unwrap(), id);

    // Scalar ↔ a one-element arrow_array array.
    let arrow = Int64::new(42).to_arrow();
    assert_eq!((arrow.len(), arrow.null_count()), (1, 0));
    assert_eq!(Int64::from_arrow(arrow.as_ref()).unwrap(), Int64::new(42));
    assert!(Int64::null().to_arrow().is_null(0));
}
```

## The null, union and optional types

The `null` module holds `NullType` — the storage-free type whose every value is null —
with its `NullField` and `Null`. The `union` module holds `UnionType`, Apache
Arrow's union type: a value is exactly one of several child types, discriminated by
a type id. `UnionType` carries its `UnionFields` and `UnionMode` exactly as Arrow models
them, so `to_arrow` / `from_arrow` round-trip *any* union losslessly.

The `optional` module builds on both: `OptionalType<D>` is the first concrete
[Logical](#categories) type — a value of the value type `D`, or null, physically
stored as `UnionType::optional(&D)` (the sparse two-variant union between null and the
value type; `storage()` returns it). Its Arrow surface delegates to the storage,
while its typed byte codec delegates to the value type. `OptionalField<D>` is its
field, and `Optional<D, S>` its scalar — an inner scalar `S`, or the null
variant. Access redirects to the inner scalar (`value` and every `as_*` accessor
answer through `S`), and so does the Arrow form: a one-element `UnionArray` whose
type id selects the variant, `from_arrow` handing the value child back to `S`'s own
`from_arrow`. The bindings expose the optional family as concrete per-type classes
(`OptionalInt64Type`, `OptionalInt64Field`, `OptionalInt64`, …), the scalars built
straight from the native value, and reach `UnionType` through an optional data type's
`storage()` (arbitrary child fields stay Rust-only).

=== "Python"

    ```python
    from yggdryl import data

    optional = data.Int64Type().optional()
    assert (optional.name(), optional.value_type().name()) == ("optional", "int64")
    assert optional.arrow_format() == "+us:0,1"  # sparse, type ids 0 and 1
    assert (optional.storage().name(), optional.storage().mode()) == ("union", "sparse")

    score = data.OptionalInt64Field("score")
    assert score.data_type().name() == "optional"

    answer = data.OptionalInt64(42)
    assert answer.as_i64() == 42  # redirected to the inner scalar
    assert answer.scalar().value() == 42
    assert not answer.is_null()

    missing = data.OptionalInt64.null()
    assert missing.is_null()
    assert missing.value() is None
    ```

=== "Node"

    ```js
    const { data } = require('yggdryl')

    const optional = new data.Int64Type().optional()
    assert.deepEqual([optional.name(), optional.valueType().name()], ['optional', 'int64'])
    assert.equal(optional.arrowFormat(), '+us:0,1') // sparse, type ids 0 and 1
    assert.deepEqual([optional.storage().name(), optional.storage().mode()], ['union', 'sparse'])

    const score = new data.OptionalInt64Field('score')
    assert.equal(score.dataType().name(), 'optional')

    const answer = new data.OptionalInt64(42n)
    assert.equal(answer.asI64(), 42n) // redirected to the inner scalar
    assert.equal(answer.scalar().value(), 42n)
    assert.equal(answer.isNull(), false)

    const missing = data.OptionalInt64.null()
    assert.equal(missing.isNull(), true)
    assert.equal(missing.value(), null)
    ```

=== "Rust"

    ```rust
    use yggdryl_data::{
        Int64Type, Int64, Logical, TypedOptional, OptionalField, Optional, RawDataType,
        RawField, RawScalar,
    };

    fn main() {
        let optional = TypedOptional::new(Int64Type);
        assert_eq!((optional.name(), optional.value_type().name()), ("optional", "int64"));
        assert_eq!(optional.arrow_format(), "+us:0,1"); // sparse, type ids 0 and 1
        assert_eq!(optional.storage().name(), "union");

        let score = OptionalField::<Int64Type>::new("score", true);
        assert_eq!(score.data_type().name(), "optional");

        let answer = Optional::new(Int64::new(42));
        assert_eq!(answer.as_i64().unwrap(), 42); // redirected to the inner scalar
        assert_eq!(answer.scalar(), Some(&Int64::new(42)));
        assert!(!answer.is_null());

        let missing: Optional<Int64Type, Int64> = Optional::null();
        assert!(missing.is_null());
        assert_eq!(missing.value(), None);
    }
    ```

In Rust, the Arrow form round-trips too: `missing.to_arrow()` is a one-element union
array whose type id selects the null variant, and `Optional::from_arrow` is
its exact inverse; the typed byte codec of `OptionalType<Int64Type>` reads and writes plain
`i64` bytes (the value type's codec).

## Nested types: list, map and struct

!!! note "Rust only"
    The nested families are generic over their child types (or carry dynamic Arrow
    fields), and the buffer-backed `Int64Serie` shares raw Arrow buffers that await
    C Data Interface interop — none has a concrete FFI shape yet, so they are not
    exposed to Python or Node.

The `list`, `map` and `struct` modules follow the family pattern. `ListType<D>` is
the variable-length sequence of one value type (single nullable `"item"` child);
`MapType<K, V>` the sequence of key–value entries (single `"entries"` struct child);
`StructType` the dynamic ordered set of named fields, carried losslessly like
`UnionType`.

The list scalar is *our array*: `Serie<D, S>` is backed by one zero-copy Arrow
child array — construction assembles the elements once, `to_arrow` / `from_arrow`
are reference-count bumps, and the scalar accessors read elements back out
(`get_scalar_at(index)` redirects one element through the inner scalar's own
`from_arrow`, `get_value_at(index)` hands back an element's owned native value —
`i64` for an `int64` list, `Vec<u8>` for a `binary` list; `len` / `is_empty`
describe the sequence). `Int64Serie` is the
concrete list of `int64`, borrowing the raw Arrow buffers themselves
(`ScalarBuffer<i64>` elements plus an optional `NullBuffer`): `values()` borrows
the whole element buffer as `&[i64]` without copying, `get_value_at(index)` reads
one element null-aware, and `get_scalar_at(index)` hands back an `Int64`.
`Map<K, V, SK, SV>` holds the entry sequence and `Struct` one row of
one-element Arrow columns, each round-tripping through a one-element Arrow array
whose children redirect to the inner scalars' own `to_arrow` / `from_arrow`. The
typed byte codecs concatenate the child codecs and split them back by fixed width
(a variable-width child errors with `DataError::IndeterminateElementWidth` —
decode those from Arrow).

```rust
use yggdryl_data::{
    DataType, Int64Type, Int64Serie, Int64, Serie, ListType, MapType, RawDataType,
    RawScalar, UInt8Type,
};

fn main() {
    let list = ListType::new(Int64Type);
    assert_eq!((list.name(), list.arrow_format().as_str()), ("list", "+l"));
    assert_eq!(list.native_from_bytes(&list.native_to_bytes(&vec![1, 2])).unwrap(), vec![1, 2]);
    assert_eq!(list.default_value(), Vec::<i64>::new()); // sequences default to empty

    let numbers = Serie::new(vec![Int64::new(1), Int64::null()]);
    assert_eq!(numbers.get_scalar_at(1), Some(Int64::null()));
    assert_eq!(numbers.get_value_at(0), Some(1)); // the owned native value
    let arrow = numbers.to_arrow(); // a one-element ListArray sharing the elements
    assert_eq!(Serie::from_arrow(arrow.as_ref()).unwrap(), numbers);

    // The buffer-backed list of int64: native, zero-copy access.
    let fast = Int64Serie::from(vec![1, 2, 3]);
    assert_eq!(fast.values(), Some(&[1, 2, 3][..])); // borrows the Arrow buffer
    assert_eq!(fast.get_value_at(1), Some(2));
    assert_eq!(Int64Serie::from_arrow(fast.to_arrow().as_ref()).unwrap(), fast);

    let map = MapType::new(UInt8Type, Int64Type);
    assert_eq!((map.name(), map.arrow_format().as_str()), ("map", "+m"));
}
```

## The trait layers

### Untyped base

- **`RawDataType`** — a physical type descriptor: `name`, the Arrow C Data Interface
  `arrow_format` string, and fixed `byte_width` / `bit_width` (`None` for variable or
  nested types); `to_arrow` / `from_arrow` mirror an `arrow_schema::DataType`
  (`from_arrow`, returning `Self`, is `Self: Sized` so the trait stays object-safe).
- **`RawField<D: RawDataType>`** — a named, nullable column: `name`, `data_type`,
  `is_nullable`; `to_arrow` (defaulted from those three accessors) / `from_arrow`
  mirror an `arrow_schema::Field`.
- **`RawScalar<D: RawDataType>`** — a single, possibly-null value: `data_type`,
  `is_null`, `value` of an associated `Value: ?Sized`; `to_arrow` / `from_arrow`
  mirror a one-element `arrow_array` array. The typed and category traits inherit the
  whole Arrow surface — it is defined once, on the base. The `as_*` accessors
  (`as_i8` … `as_u64`, `as_f32` / `as_f64`, `as_bool`, `as_str`, `as_bytes`) read
  the value as a chosen Rust type under one contract: the value whenever the target
  represents it exactly — direct for the scalar's own type (`as_str` / `as_bytes`
  borrow, never copy), exact conversion otherwise — and an actionable `DataError`
  when not: `NullValue` for a null scalar, `InexactConversion` when converting
  would change the value (a narrowing out of range, a float that would round,
  non-UTF-8 bytes read as `str`), `UnsupportedConversion` when the type has no
  conversion to the target at all. Every accessor defaults to that error, so a
  concrete scalar overrides only the targets its value converts to; the bindings
  raise `ValueError` (Python) / throw (Node).

### Typed

The same, tied to a native Rust type `T`:

- **`DataType<T>: RawDataType`** — adds the byte codec `native_to_bytes` /
  `native_from_bytes` (a length mismatch on decode returns
  `DataError::InvalidByteLength`), the associated `Scalar` type, and the
  defaults: `default_value` (the type's default native value — `0` for the
  integers, an empty sequence for lists and maps, the *first* data type's
  default for a union) and `default_scalar` (a scalar holding it, except where
  the scalar models nullness — an optional's default scalar is its null
  variant).
- **`Field<T>: RawField<Self::Type>`** — a field whose data type is a `DataType<T>`.
- **`Scalar<T>: RawScalar<Self::Type, Value = T>`** — a scalar whose value is `T`.

```rust
use yggdryl_data::{DataType, Int64Type, Primitive, RawDataType, RawScalar, Scalar};

// Generic code composes across the layers.
fn first_byte<D: DataType<i64>>(data_type: &D, value: i64) -> u8 {
    data_type.native_to_bytes(&value)[0]
}
fn is_null<S: Scalar<i64>>(scalar: &S) -> bool {
    scalar.is_null()
}
fn width<P: Primitive>(primitive: &P) -> Option<usize> {
    primitive.byte_width()
}

fn main() {
    assert_eq!(first_byte(&Int64Type, 5), 5);
    assert_eq!(width(&Int64Type), Some(8));
}
```

### Categories

How a type is shaped (each refines `RawDataType`):

- **`Primitive`** — a fixed-width, childless physical type (integers, floats, boolean).
- **`RawLogical<S>` / `Logical<T>`** — a type layered over a physical storage type
  `S`, e.g. a timestamp over `int64`: the raw side's `storage()` returns it, the
  typed side pins it as the associated `Storage` and adds the native codec. The
  generic holder is `OptionalType<D>` — a value or null over the null-or-value
  union.
- **`RawNested` / `Nested<T>`** — a type composed of child fields (`struct`,
  `list`, `map`, `union`): the raw side's `child_count()` reports how many, the
  typed side adds the native codec (a sequence, a row). The generic holders are
  `ListType<D>` (`Nested<Vec<T>>`) and `MapType<K, V>` (`Nested<Vec<(TK, TV)>>`);
  the dynamic `StructType` and `UnionType` stay raw-only.

Each composite family also carries its own raw/typed trait pair, mirroring the
base layers: `RawOptional` / `TypedOptional`, `RawUnion` / `UnionType` (a typed union's
defaults are its *first* data type's), `RawList` / `TypedList`, `RawMap` / `TypedMap` and
`RawStruct` / `TypedStruct` — the concrete `OptionalType<D>`, `UnionType`,
`ListType<D>`, `MapType<K, V>` and `StructType` implement the raw side, and the
typed side wherever the child types have codecs (the dynamic `UnionType` and
`StructType`, whose children are only known at runtime, stay raw-only).

## Type ids

!!! note "Rust only"
    `DataTypeId` is a method-bearing enum the bindings cannot model uniformly; it is
    not yet exposed to Python or Node.

`DataTypeId` is a `Copy` tag with one variant per Arrow type — independent of any
parameters — used to switch on or group types cheaply. `DataTypeId::ALL` lists every
id; each carries its `name`, its Arrow C Data Interface `arrow_format` (static for
parameterless types, `None` for parameterized/logical ones), and the `is_primitive` /
`is_nested` classification.

```rust
use yggdryl_data::DataTypeId;

fn main() {
    assert_eq!(DataTypeId::Int64.name(), "int64");
    assert_eq!(DataTypeId::Int64.arrow_format(), Some("l"));
    assert!(DataTypeId::Int64.is_primitive());
    assert!(DataTypeId::Struct.is_nested());
    assert_eq!(DataTypeId::Decimal128.arrow_format(), None); // parameterized
    assert!(DataTypeId::ALL.contains(&DataTypeId::Utf8));
}
```
