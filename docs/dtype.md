# Data types

The `yggdryl-dtype` crate is the Apache Arrow-centralized **data-type layer**,
built on `yggdryl-core` — the first of the three data layers (`yggdryl-dtype`,
[`yggdryl-field`](field.md), [`yggdryl-scalar`](scalar.md)), each concern its own
crate, so the concrete types share one naming convention across the layers
(`yggdryl_dtype::Int64Type` describes the type, `yggdryl_field::Int64Field` names a
column of it, `yggdryl_scalar::Int64Scalar` holds one value of it). It defines the
physical and logical data types for zero-copy FFI and Arrow interop. The concrete
families so far: the `integer` module (every signed and unsigned integer), the
`binary` module (the variable-size byte type), the `null` module (the storage-free
null type), the `union` module, the `optional` module (the logical null-or-value
type over union storage) and the nested `list`, `map` and `struct` modules; more
land as the layer grows.

The bindings expose the layer as `yggdryl.dtype` (Python and Node), adapting to
idioms: Node carries 8–32 bit codec values as `number` and the 64-bit types as
`BigInt`, byte values cross as Python `bytes` / JS `Buffer`, and errors surface
the core `DataError` as a raised `ValueError` (Python) / thrown `Error` (Node).
Four things stay **Rust-only**, stated here and in both binding module docs: the
[Arrow interop](#arrow-interop) surface (`to_arrow` / `from_arrow` exchange
`arrow-schema` values that cannot cross the FFI boundary), construction of a
`UnionType` from arbitrary child fields (reached in the bindings through an
optional data type's `storage()`), the [`DataTypeId`](#type-ids) classifier, and
the [nested types](#nested-types-list-map-and-struct) (the generic `ListType` /
`MapType` / `StructType` and the per-family trait pairs), which have no concrete
FFI shape yet.

The trait layers carry no lifetime parameter (FFI-clean); the untyped base is
`Debug + Send + Sync` so schemas are printable and shareable across threads and
FFI, and `DataType` is object-safe for `Box<dyn DataType>` schemas.

## The concrete types: the `integer` module

The `integer` module holds every Apache Arrow signed and unsigned integer —
`Int8Type` … `Int64Type`, `UInt8Type` … `UInt64Type` — one file per type. Each is
a fixed-width [primitive](#categories) with a little-endian byte codec; the eight
share one shape, so a single crate-internal macro generates each per-type file.

`Int64Type`, native Rust `i64`, is stored little-endian in eight bytes (Arrow C
Data Interface format `"l"`):

=== "Python"

    ```python
    from yggdryl import dtype

    int64 = dtype.Int64Type()
    assert int64.name() == "int64"
    assert int64.arrow_format() == "l"
    assert (int64.byte_width(), int64.bit_width()) == (8, 64)

    # The codec bridging a native value to and from Arrow bytes.
    assert int64.native_to_bytes(-1) == b"\xff" * 8
    assert int64.native_from_bytes(b"\xff" * 8) == -1

    # The type knows its defaults (the scalar comes from yggdryl.scalar).
    assert int64.default_value() == 0
    assert int64.default_scalar().value() == 0
    ```

=== "Node"

    ```js
    const { dtype } = require('yggdryl')

    const int64 = new dtype.Int64Type()
    assert.equal(int64.name(), 'int64')
    assert.equal(int64.arrowFormat(), 'l')
    assert.deepEqual([int64.byteWidth(), int64.bitWidth()], [8, 64])

    // The codec bridging a native value to and from Arrow bytes (BigInt for 64-bit).
    assert.deepEqual([...int64.nativeToBytes(-1n)], Array(8).fill(0xff))
    assert.equal(int64.nativeFromBytes(Buffer.alloc(8, 0xff)), -1n)

    // The type knows its defaults (the scalar comes from yggdryl.scalar).
    assert.equal(int64.defaultValue(), 0n)
    assert.equal(int64.defaultScalar().value(), 0n)
    ```

=== "Rust"

    ```rust
    use yggdryl_dtype::{DataType, Int64Type, TypedDataType};

    fn main() {
        assert_eq!(Int64Type.name(), "int64");
        assert_eq!(Int64Type.arrow_format(), "l");
        assert_eq!((Int64Type.byte_width(), Int64Type.bit_width()), (Some(8), Some(64)));

        // The codec bridging a native value to and from Arrow bytes.
        assert_eq!(Int64Type.native_to_bytes(&-1), vec![0xFF; 8]);
        assert_eq!(Int64Type.native_from_bytes(&[0xFF; 8]).unwrap(), -1);

        // The type knows its default value (the default *scalar* lives on
        // yggdryl-scalar's ScalarFactory trait).
        assert_eq!(Int64Type.default_value(), 0);
    }
    ```

The other widths follow the same surface — swap `Int64Type` / `i64` / `"l"` for
`Int8Type` / `i8` / `"c"`, `UInt32Type` / `u32` / `"I"`, and so on. In Rust,
`Int64Type::ID` names the matching [`DataTypeId`](#type-ids) classifier.

## The `binary` type

`BinaryType` is the variable-size byte type (Arrow C Data Interface format `"z"`,
no fixed width). The typed codec is the identity — any byte sequence is a valid
`binary` value.

=== "Python"

    ```python
    from yggdryl import dtype

    binary = dtype.BinaryType()
    assert (binary.name(), binary.arrow_format()) == ("binary", "z")
    assert binary.byte_width() is None  # variable width
    assert binary.native_from_bytes(binary.native_to_bytes(b"\x01\x02")) == b"\x01\x02"
    assert binary.default_value() == b""
    ```

=== "Node"

    ```js
    const { dtype } = require('yggdryl')

    const binary = new dtype.BinaryType()
    assert.deepEqual([binary.name(), binary.arrowFormat()], ['binary', 'z'])
    assert.equal(binary.byteWidth(), null) // variable width
    assert.deepEqual(
      binary.nativeFromBytes(binary.nativeToBytes(Buffer.from([1, 2]))),
      Buffer.from([1, 2]),
    )
    assert.deepEqual(binary.defaultValue(), Buffer.alloc(0))
    ```

=== "Rust"

    ```rust
    use yggdryl_dtype::{BinaryType, DataType, TypedDataType};

    fn main() {
        assert_eq!((BinaryType.name(), BinaryType.arrow_format().as_str()), ("binary", "z"));
        assert_eq!(BinaryType.byte_width(), None); // variable width
        let bytes = BinaryType.native_to_bytes(&vec![1, 2]);
        assert_eq!(BinaryType.native_from_bytes(&bytes).unwrap(), vec![1, 2]);
        assert_eq!(BinaryType.default_value(), Vec::<u8>::new());
    }
    ```

In Rust, `BinaryType::ID` is `DataTypeId::Binary`. `BinaryType` is *not* a
`Primitive` in this model's fixed-width sense: it is Arrow's variable-size binary
layout, childless but without a fixed width.

## Arrow interop

!!! note "Rust only"
    `to_arrow` / `from_arrow` exchange `arrow-schema` values, which cannot cross
    the FFI boundary — the bindings will gain this surface through the Arrow C
    Data Interface as it lands.

Every data type converts to and from the [`arrow_schema::DataType`] it mirrors
with a `to_arrow` / `from_arrow` pair (`from_arrow` is the exact inverse of what
`to_arrow` produces, refusing a mismatched Arrow value with `DataError`). The
`arrow-schema` subset crate is re-exported from the crate root so downstream code
uses the exact version the crate was built against.

```rust
use yggdryl_dtype::{arrow_schema, DataType, Int64Type};

fn main() {
    assert_eq!(Int64Type.to_arrow(), arrow_schema::DataType::Int64);
    assert!(Int64Type::from_arrow(&arrow_schema::DataType::Utf8).is_err());
}
```

## The null, union and optional types

`NullType` is the storage-free type whose every value is null. `UnionType` is
Apache Arrow's union type: a value is exactly one of several child types,
discriminated by a type id. `UnionType` carries its `UnionFields` and `UnionMode`
exactly as Arrow models them, so `to_arrow` / `from_arrow` round-trip *any* union
losslessly.

The `optional` module builds on both: `OptionalType<D>` is the first concrete
[Logical](#categories) type — a value of the value type `D`, or null, physically
stored as `UnionType::optional(&D)` (the sparse two-variant union between null and
the value type; `storage()` returns it). Its Arrow surface delegates to the
storage, while its typed byte codec delegates to the value type. The bindings
expose the optional family as concrete per-type classes (`OptionalInt64Type`,
`OptionalBinaryType`, …) and reach `UnionType` through an optional data type's
`storage()` (arbitrary child fields stay Rust-only).

=== "Python"

    ```python
    from yggdryl import dtype

    optional = dtype.Int64Type().optional()
    assert (optional.name(), optional.value_type().name()) == ("optional", "int64")
    assert optional.arrow_format() == "+us:0,1"  # sparse, type ids 0 and 1
    assert (optional.storage().name(), optional.storage().mode()) == ("union", "sparse")

    # The optional's codec is the value type's.
    assert optional.native_from_bytes(optional.native_to_bytes(42)) == 42
    ```

=== "Node"

    ```js
    const { dtype } = require('yggdryl')

    const optional = new dtype.Int64Type().optional()
    assert.deepEqual([optional.name(), optional.valueType().name()], ['optional', 'int64'])
    assert.equal(optional.arrowFormat(), '+us:0,1') // sparse, type ids 0 and 1
    assert.deepEqual([optional.storage().name(), optional.storage().mode()], ['union', 'sparse'])

    // The optional's codec is the value type's.
    assert.equal(optional.nativeFromBytes(optional.nativeToBytes(42n)), 42n)
    ```

=== "Rust"

    ```rust
    use yggdryl_dtype::{DataType, Int64Type, Logical, Optional, OptionalType, TypedDataType};

    fn main() {
        let optional = OptionalType::new(Int64Type);
        assert_eq!((optional.name(), optional.value_type().name()), ("optional", "int64"));
        assert_eq!(optional.arrow_format(), "+us:0,1"); // sparse, type ids 0 and 1
        assert_eq!(optional.storage().name(), "union");

        // The optional's codec is the value type's.
        assert_eq!(optional.native_from_bytes(&42i64.to_le_bytes()).unwrap(), 42);
    }
    ```

## Nested types: list, map and struct

!!! note "Rust only"
    The nested types are generic over their child types (or carry dynamic Arrow
    fields) — none has a concrete FFI shape yet, so they are not exposed to
    Python or Node.

The `list`, `map` and `struct` modules follow the family pattern. `ListType<D>` is
the variable-length sequence of one value type (single nullable `"item"` child);
`MapType<K, V>` the sequence of key–value entries (single `"entries"` struct
child); `StructType` the dynamic ordered set of named fields, carried losslessly
like `UnionType`. The typed byte codecs concatenate the child codecs and split
them back by fixed width (a variable-width child errors with
`DataError::IndeterminateElementWidth` — decode those from Arrow).

```rust
use yggdryl_dtype::{DataType, Int64Type, ListType, MapType, TypedDataType, UInt8Type};

fn main() {
    let list = ListType::new(Int64Type);
    assert_eq!((list.name(), list.arrow_format().as_str()), ("list", "+l"));
    assert_eq!(list.native_from_bytes(&list.native_to_bytes(&vec![1, 2])).unwrap(), vec![1, 2]);
    assert_eq!(list.default_value(), Vec::<i64>::new()); // sequences default to empty

    let map = MapType::new(UInt8Type, Int64Type);
    assert_eq!((map.name(), map.arrow_format().as_str()), ("map", "+m"));
}
```

## The trait layers

### Untyped base

- **`DataType`** — a physical type descriptor: `name`, the Arrow C Data Interface
  `arrow_format` string, and fixed `byte_width` / `bit_width` (`None` for variable or
  nested types); `to_arrow` / `from_arrow` mirror an `arrow_schema::DataType`
  (`from_arrow`, returning `Self`, is `Self: Sized` so the trait stays object-safe).

### Typed

- **`TypedDataType<T>: DataType`** — adds the byte codec `native_to_bytes` /
  `native_from_bytes` (a length mismatch on decode returns
  `DataError::InvalidByteLength`) and `default_value` (the type's default native
  value — `0` for the integers, an empty sequence for lists and maps, the *first*
  data type's default for a union). The default *scalar* of a type lives
  upstream, on [`yggdryl-scalar`](scalar.md)'s `ScalarFactory` trait — the scalar
  layer builds on this one, never the other way around.

### Categories

How a type is shaped (each refines `DataType`):

- **`Primitive`** — a fixed-width, childless physical type (integers, floats, boolean).
- **`Logical` / `TypedLogical<T>`** — a type layered over a physical storage
  type, e.g. a timestamp over `int64`: the base side carries it as the associated
  `Storage` (returned by `storage()`), the typed side pins the same `Storage` and
  adds the native codec. The generic holder is `OptionalType<D>` — a value or null
  over the null-or-value union.
- **`Nested` / `TypedNested<T>`** — a type composed of child fields (`struct`,
  `list`, `map`, `union`): the base side's `child_count()` reports how many, the
  typed side adds the native codec (a sequence, a row). The generic holders are
  `ListType<D>` (`TypedNested<Vec<T>>`) and `MapType<K, V>`
  (`TypedNested<Vec<(TK, TV)>>`); the dynamic `StructType` and `UnionType` stay
  base-only.

Each composite family also carries its own base/typed trait pair, mirroring the
base layers: `Optional` / `TypedOptional`, `Union` / `TypedUnion` (a typed
union's defaults are its *first* data type's), `List` / `TypedList`, `Map`
/ `TypedMap` and `Struct` / `TypedStruct` — the concrete `OptionalType<D>`,
`UnionType`, `ListType<D>`, `MapType<K, V>` and `StructType` implement the base
side, and the typed side wherever the child types have codecs (the dynamic
`UnionType` and `StructType`, whose children are only known at runtime, stay
base-only).

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
use yggdryl_dtype::DataTypeId;

fn main() {
    assert_eq!(DataTypeId::Int64.name(), "int64");
    assert_eq!(DataTypeId::Int64.arrow_format(), Some("l"));
    assert!(DataTypeId::Int64.is_primitive());
    assert!(DataTypeId::Struct.is_nested());
    assert_eq!(DataTypeId::Decimal128.arrow_format(), None); // parameterized
    assert!(DataTypeId::ALL.contains(&DataTypeId::Utf8));
}
```
