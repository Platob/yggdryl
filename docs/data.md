# Data model

The `yggdryl-data` crate is the Apache Arrow-centralized **data-model layer**, built
on `yggdryl-core`. It defines the physical type system — data types, fields and
scalars — for zero-copy FFI and Arrow interop. The concrete families so far: the
`integer` module (every signed and unsigned integer), the `null` module (the
storage-free null type) and the `union` module (the union type, with the
null-or-value `OptionalScalar`); more land as the layer grows.

The bindings expose the layer as `yggdryl.data` (Python) and `yggdryl.data` (Node),
adapting to idioms: Node carries 8–32 bit values as `number` and the 64-bit types as
`BigInt`, and the null-or-value scalars are concrete per-type classes
(`OptionalInt64Scalar`, …) built straight from the native value. Three things stay
**Rust-only**, stated here and in both binding module docs: the [Arrow
interop](#arrow-interop) surface (`to_arrow` / `from_arrow` exchange `arrow-schema` /
`arrow-array` values that cannot cross the FFI boundary), construction of a `Union`
from arbitrary child fields (reached in the bindings through an optional data type's
`storage()`), and the [`DataTypeId`](#type-ids) classifier.

The type system is three layers of traits. None carries a lifetime parameter
(FFI-clean); the untyped base is `Debug + Send + Sync` so schemas are printable and
shareable across threads and FFI, and `RawDataType` is object-safe for
`Box<dyn RawDataType>` schemas.

## The concrete types: the `integer` module

The `integer` module holds every Apache Arrow signed and unsigned integer — `Int8` …
`Int64`, `UInt8` … `UInt64` — one module per type, one file per concern (`data_type`,
`field`, `scalar`). Each is a fixed-width [primitive](#categories) with a little-endian
byte codec, a nullable field and a possibly-null scalar; they share one shape, so a
single crate-internal macro generates each per-type file.

`Int64`, native Rust `i64`, is stored little-endian in eight bytes (Arrow C Data
Interface format `"l"`). Scalars are built from their native value and read through
the `as_*` accessors: direct for the scalar's own type, exact conversion otherwise,
and null when the scalar is null or the value is not exactly representable:

=== "Python"

    ```python
    from yggdryl import data

    int64 = data.Int64()
    assert int64.name() == "int64"
    assert int64.arrow_format() == "l"
    assert (int64.byte_width(), int64.bit_width()) == (8, 64)

    # The codec bridging a native value to and from Arrow bytes.
    assert int64.native_to_bytes(-1) == b"\xff" * 8
    assert int64.native_from_bytes(b"\xff" * 8) == -1

    # A named, nullable column of int64.
    id_field = data.Int64Field("id", False)
    assert (id_field.name(), id_field.is_nullable()) == ("id", False)

    # A single i64 value, or null, with exact-or-None accessors.
    scalar = data.Int64Scalar(42)
    assert scalar.value() == 42
    assert scalar.as_i8() == 42          # converted access
    assert scalar.as_str() is None       # an int64 is not a string
    assert data.Int64Scalar.null().is_null()
    ```

=== "Node"

    ```js
    const { data } = require('yggdryl')

    const int64 = new data.Int64()
    assert.equal(int64.name(), 'int64')
    assert.equal(int64.arrowFormat(), 'l')
    assert.deepEqual([int64.byteWidth(), int64.bitWidth()], [8, 64])

    // The codec bridging a native value to and from Arrow bytes (BigInt for 64-bit).
    assert.deepEqual([...int64.nativeToBytes(-1n)], Array(8).fill(0xff))
    assert.equal(int64.nativeFromBytes(Buffer.alloc(8, 0xff)), -1n)

    // A named, nullable column of int64.
    const idField = new data.Int64Field('id', false)
    assert.deepEqual([idField.name(), idField.isNullable()], ['id', false])

    // A single i64 value, or null, with exact-or-null accessors.
    const scalar = new data.Int64Scalar(42n)
    assert.equal(scalar.value(), 42n)
    assert.equal(scalar.asI8(), 42)      // converted access
    assert.equal(scalar.asStr(), null)   // an int64 is not a string
    assert.equal(data.Int64Scalar.null().isNull(), true)
    ```

=== "Rust"

    ```rust
    use yggdryl_data::{DataType, Int64, Int64Field, Int64Scalar, RawDataType, RawField, RawScalar};

    fn main() {
        assert_eq!(Int64.name(), "int64");
        assert_eq!(Int64.arrow_format(), "l");
        assert_eq!((Int64.byte_width(), Int64.bit_width()), (Some(8), Some(64)));

        // The codec bridging a native value to and from Arrow bytes.
        assert_eq!(Int64.native_to_bytes(&-1), vec![0xFF; 8]);
        assert_eq!(Int64.native_from_bytes(&[0xFF; 8]).unwrap(), -1);

        // A named, nullable column of int64.
        let id = Int64Field::new("id", false);
        assert_eq!((id.name(), id.is_nullable()), ("id", false));

        // A single i64 value, or null, with exact-or-None accessors.
        let scalar = Int64Scalar::from(42);
        assert_eq!(scalar.value(), Some(&42));
        assert_eq!(scalar.as_i8(), Some(42)); // converted access
        assert_eq!(scalar.as_str(), None); // an int64 is not a string
        assert!(Int64Scalar::null().is_null());
    }
    ```

The other widths follow the same surface — swap `Int64` / `i64` / `"l"` for
`Int8` / `i8` / `"c"`, `UInt32` / `u32` / `"I"`, and so on. In Rust, `Int64::ID`
names the matching [`DataTypeId`](#type-ids) classifier.

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
use yggdryl_data::{arrow_array::Array, arrow_schema, Int64, Int64Field, Int64Scalar};
use yggdryl_data::{RawDataType, RawField, RawScalar};

fn main() {
    // Data type ↔ arrow_schema::DataType.
    assert_eq!(Int64.to_arrow(), arrow_schema::DataType::Int64);
    assert!(Int64::from_arrow(&arrow_schema::DataType::Utf8).is_err());

    // Field ↔ arrow_schema::Field.
    let id = Int64Field::new("id", false);
    assert_eq!(Int64Field::from_arrow(&id.to_arrow()).unwrap(), id);

    // Scalar ↔ a one-element arrow_array array.
    let arrow = Int64Scalar::new(42).to_arrow();
    assert_eq!((arrow.len(), arrow.null_count()), (1, 0));
    assert_eq!(Int64Scalar::from_arrow(arrow.as_ref()).unwrap(), Int64Scalar::new(42));
    assert!(Int64Scalar::null().to_arrow().is_null(0));
}
```

## The null, union and optional types

The `null` module holds `Null` — the storage-free type whose every value is null —
with its `NullField` and `NullScalar`. The `union` module holds `Union`, Apache
Arrow's union type: a value is exactly one of several child types, discriminated by
a type id. `Union` carries its `UnionFields` and `UnionMode` exactly as Arrow models
them, so `to_arrow` / `from_arrow` round-trip *any* union losslessly.

The `optional` module builds on both: `Optional<D>` is the first concrete
[Logical](#categories) type — a value of the value type `D`, or null, physically
stored as `Union::optional(&D)` (the sparse two-variant union between null and the
value type; `storage()` returns it). Its Arrow surface delegates to the storage,
while its typed byte codec delegates to the value type. `OptionalField<D>` is its
field, and `OptionalScalar<D, S>` its scalar — an inner scalar `S`, or the null
variant. Access redirects to the inner scalar (`value` and every `as_*` accessor
answer through `S`), and so does the Arrow form: a one-element `UnionArray` whose
type id selects the variant, `from_arrow` handing the value child back to `S`'s own
`from_arrow`. The bindings expose the optional family as concrete per-type classes
(`OptionalInt64`, `OptionalInt64Field`, `OptionalInt64Scalar`, …), the scalars built
straight from the native value, and reach `Union` through an optional data type's
`storage()` (arbitrary child fields stay Rust-only).

=== "Python"

    ```python
    from yggdryl import data

    optional = data.Int64().optional()
    assert (optional.name(), optional.value_type().name()) == ("optional", "int64")
    assert optional.arrow_format() == "+us:0,1"  # sparse, type ids 0 and 1
    assert (optional.storage().name(), optional.storage().mode()) == ("union", "sparse")

    score = data.OptionalInt64Field("score")
    assert score.data_type().name() == "optional"

    answer = data.OptionalInt64Scalar(42)
    assert answer.as_i64() == 42  # redirected to the inner scalar
    assert answer.scalar().value() == 42
    assert not answer.is_null()

    missing = data.OptionalInt64Scalar.null()
    assert missing.is_null()
    assert missing.value() is None
    ```

=== "Node"

    ```js
    const { data } = require('yggdryl')

    const optional = new data.Int64().optional()
    assert.deepEqual([optional.name(), optional.valueType().name()], ['optional', 'int64'])
    assert.equal(optional.arrowFormat(), '+us:0,1') // sparse, type ids 0 and 1
    assert.deepEqual([optional.storage().name(), optional.storage().mode()], ['union', 'sparse'])

    const score = new data.OptionalInt64Field('score')
    assert.equal(score.dataType().name(), 'optional')

    const answer = new data.OptionalInt64Scalar(42n)
    assert.equal(answer.asI64(), 42n) // redirected to the inner scalar
    assert.equal(answer.scalar().value(), 42n)
    assert.equal(answer.isNull(), false)

    const missing = data.OptionalInt64Scalar.null()
    assert.equal(missing.isNull(), true)
    assert.equal(missing.value(), null)
    ```

=== "Rust"

    ```rust
    use yggdryl_data::{
        Int64, Int64Scalar, Logical, Optional, OptionalField, OptionalScalar, RawDataType,
        RawField, RawScalar,
    };

    fn main() {
        let optional = Optional::new(Int64);
        assert_eq!((optional.name(), optional.value_type().name()), ("optional", "int64"));
        assert_eq!(optional.arrow_format(), "+us:0,1"); // sparse, type ids 0 and 1
        assert_eq!(optional.storage().name(), "union");

        let score = OptionalField::<Int64>::new("score", true);
        assert_eq!(score.data_type().name(), "optional");

        let answer = OptionalScalar::new(Int64Scalar::new(42));
        assert_eq!(answer.as_i64(), Some(42)); // redirected to the inner scalar
        assert_eq!(answer.scalar(), Some(&Int64Scalar::new(42)));
        assert!(!answer.is_null());

        let missing: OptionalScalar<Int64, Int64Scalar> = OptionalScalar::null();
        assert!(missing.is_null());
        assert_eq!(missing.value(), None);
    }
    ```

In Rust, the Arrow form round-trips too: `missing.to_arrow()` is a one-element union
array whose type id selects the null variant, and `OptionalScalar::from_arrow` is
its exact inverse; the typed byte codec of `Optional<Int64>` reads and writes plain
`i64` bytes (the value type's codec).

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
  (`as_i8` … `as_u64`, `as_f32` / `as_f64`, `as_bool`, `as_str`) read the value as a
  chosen Rust type under one contract: direct for the scalar's own type (`as_str`
  borrows, never copies), exact conversion otherwise, and `None` when the scalar is
  null or the value is not exactly representable (a narrowing out of range or a
  float that would round). Every accessor defaults to `None`, so a concrete scalar
  overrides only the targets its value converts to.

### Typed

The same, tied to a native Rust type `T`:

- **`DataType<T>: RawDataType`** — adds the byte codec `native_to_bytes` /
  `native_from_bytes` (a length mismatch on decode returns
  `DataError::InvalidByteLength`).
- **`Field<T>: RawField<Self::Type>`** — a field whose data type is a `DataType<T>`.
- **`Scalar<T>: RawScalar<Self::Type, Value = T>`** — a scalar whose value is `T`.

```rust
use yggdryl_data::{DataType, Int64, Primitive, RawDataType, RawScalar, Scalar};

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
    assert_eq!(first_byte(&Int64, 5), 5);
    assert_eq!(width(&Int64), Some(8));
}
```

### Categories

How a type is shaped (each refines `RawDataType`):

- **`Primitive`** — a fixed-width, childless physical type (integers, floats, boolean).
- **`Logical`** — a type layered over a physical `Storage` type, e.g. a timestamp over
  `int64`; `storage()` returns the backing `RawDataType`. The
  [`Optional` type](#the-null-union-and-optional-types) — a value or null, stored as
  the null-or-value union — is the first concrete one.
- **`Nested`** — a type composed of child fields (`struct`, `list`, `map`);
  `child_count()` reports how many.

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
