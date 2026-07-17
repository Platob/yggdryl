# Schema layer — DataType and Field

The **schema layer** describes columns without holding their data: a [`DataType`](#datatype)
(the runtime type descriptor + category drill-down) and a [`Field`](#field) (a named, nullable
column with metadata). A field's metadata is the centralized [`Headers`](headers.md) map — the
single, shared key/value holder (there is no separate `Metadata` type). It is mirrored,
method-for-method, in the Rust core and the **Python** and **Node** extensions.

The value/column layer that carries the actual data — `Scalar` / `Serie` / `Buffer` over the
[fixed](fixed/index.md) and [variable](var.md) families, plus the [decimal](fixed/decimal.md) and
[temporal](fixed/temporal.md) types — is mirrored in both bindings too, one class per type
(`I32Serie`, `Utf8Serie`, `D64Serie`, `Ts64Serie`, …).

## DataType

A runtime type descriptor. Build one with a factory (`i32`, `utf8`, `fixed_binary(16)`, or
`by_name`), then read its `name` / `byte_width` / `category` or drill down with the `is_*`
predicates — each a couple of integer comparisons on the underlying `DataTypeId`, never a match.
A fixed-size byte type is classified on **both** axes (fixed-width *and* binary/utf8).

=== "Python"

    ```python
    from yggdryl.types import DataType

    dt = DataType.i32()
    assert dt.name == "i32" and dt.byte_width == 4
    assert dt.is_integer() and dt.is_signed() and dt.is_fixed_width()

    assert DataType.f64().category == "float"
    assert DataType.by_name("u96").is_unsigned_integer()   # every type by name

    fb = DataType.fixed_binary(16)                          # runtime width N
    assert fb.byte_width == 16 and fb.is_binary() and fb.is_fixed_width()
    assert DataType.fixed_utf8(4).is_utf8()                 # dual classification
    ```

=== "Node"

    ```js
    const { DataType } = require('yggdryl').types

    const dt = DataType.i32()
    assert(dt.name === 'i32' && dt.byteWidth === 4)
    assert(dt.isInteger() && dt.isSigned() && dt.isFixedWidth())

    assert(DataType.f64().category === 'float')
    assert(DataType.byName('u96').isUnsignedInteger())      // every type by name

    const fb = DataType.fixedBinary(16)                     // runtime width N
    assert(fb.byteWidth === 16 && fb.isBinary() && fb.isFixedWidth())
    assert(DataType.fixedUtf8(4).isUtf8())                  // dual classification
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::DataType;
    use yggdryl_core::io::fixed::PrimitiveType;

    let dt = <PrimitiveType<i32>>::new();
    assert_eq!(dt.name(), "i32");
    assert_eq!(dt.byte_width(), 4);
    assert!(dt.is_integer() && dt.is_signed() && dt.is_fixed_width());
    // The runtime identity + predicates live on `DataTypeId` (what the bindings wrap).
    use yggdryl_core::io::DataTypeId;
    assert!(DataTypeId::U96.is_unsigned_integer());
    assert_eq!(DataTypeId::FixedBinary.name(), "fixed_binary");
    ```

## Field

A named, nullable column descriptor: a name, its `DataType`, a nullable flag, and a
[`Headers`](headers.md) metadata map. It is a **value type** — it compares (and, where the
language has it, hashes) by content, metadata included — so it works as a schema entry, a map
key, and in a set.

=== "Python"

    ```python
    from yggdryl.types import DataType, Field

    field = Field("price", DataType.f64(), nullable=True, metadata={"unit": "USD"})
    assert field.name == "price"
    assert field.data_type == DataType.f64()
    assert field.byte_width == 8 and field.is_numeric()
    assert field.metadata.get("unit") == "USD"

    # Immutable builders return a fresh field.
    tagged = field.with_metadata_entry("scale", "cents")
    assert tagged.metadata.items() == [("unit", "USD"), ("scale", "cents")]  # insertion order
    assert field.metadata.get("scale") is None        # original untouched

    schema = {field: 0, tagged: 1}                     # hashable -> a dict key
    assert len(schema) == 2
    ```

=== "Node"

    ```js
    const { DataType, Field } = require('yggdryl').types

    const field = new Field('price', DataType.f64(), true, { unit: 'USD' })
    assert(field.name === 'price')
    assert(field.dataType.equals(DataType.f64()))
    assert(field.byteWidth === 8 && field.isNumeric())
    assert(field.metadata.get('unit') === 'USD')

    // Immutable builders return a fresh field.
    const tagged = field.withMetadataEntry('scale', 'cents')
    assert.deepEqual(tagged.metadata.keys(), ['unit', 'scale'])  // insertion order
    assert(field.metadata.get('scale') === null)       // original untouched
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::{DataType, DataTypeId, FieldType, Headers};
    use yggdryl_core::io::fixed::Field;

    let field = Field::of("price", DataTypeId::F64, 8, true)
        .with_metadata(Headers::new().with("unit", "USD"));
    assert_eq!(field.name(), "price");
    assert_eq!(FieldType::type_id(&field), DataTypeId::F64);
    assert!(field.is_numeric());
    assert_eq!(field.metadata().get("unit"), Some("USD"));
    ```

## Metadata

A field's metadata **is** the centralized [`Headers`](headers.md) map — the same ordered,
case-insensitive, multi-value key/value type that backs HTTP headers. Construct a field's
metadata from a plain `dict` / object (as above) or from a `Headers` value; read it back with
the `Headers` accessors (`get`, `items` / `toObject`, `keys`, …). There is no separate
`Metadata` type — see the [Headers](headers.md) page for the full surface.

The Rust core additionally records the exact logical type under a reserved metadata key so a
lossy Arrow conversion round-trips exactly — see [Arrow metadata & round-tripping](arrow/metadata.md)
below (that Arrow interop is behind the core's `arrow` feature and is not part of the bindings).

## Arrow metadata & round-tripping

`to_arrow` is **lossy** and *non-injective*: `u96`, `i96`, `FixedUtf8`, and a runtime-`N`
`FixedBinary` all collapse to the same `FixedSizeBinary(N)`, and `(precision, scale)` /
`(unit, timezone)` are not expressible in a byte width. A naive `from_arrow` would have to *guess*.
Instead the exact logical type is recorded in **field metadata** — Arrow's `Field::metadata`, which
yggdryl models as the centralized [`Headers`](headers.md) map — and `from_arrow` prefers it
to recover the precise type. This is the mechanism the per-type Arrow sections rely on.

### The reserved keys

`to_arrow` writes these `yggdryl`-namespaced keys **only when the plain mapping is ambiguous**;
`from_arrow` reads them, then **strips** them from the user-visible metadata:

| key | written for | example value |
| --- | --- | --- |
| `yggdryl.logical_type` | a type that shares an Arrow representation with others (`u96`, `i96`, `u128`, `u256`, `fixed_utf8`, and the `Decimal`-backed `d*`) | `"u96"`, `"d128"` |
| `precision` | decimal columns | `"20"` |
| `scale` | decimal columns | `"2"` |
| `unit` | temporal columns | `"microsecond"` |
| `timezone` | timestamp columns | `"UTC"`, `"Europe/Paris"`, `""` (naive) |

Exact primitives (`i32` → `Int32`) and the `Decimal`-backed integers (`i128`/`i256`, which reverse
unambiguously to `Decimal128(38,0)` / `Decimal256(76,0)`) add **no** `yggdryl.logical_type` tag —
the plain mapping already reverses to them. A foreign `FixedSizeBinary(N)` with no yggdryl tag
decodes to the **safe default** — `fixed_binary` of that width — never a guessed wide integer.

### It round-trips exactly

```rust
# #[cfg(feature = "arrow")]
# fn demo() {
use yggdryl_core::io::fixed::{FixedUtf8Field, TypedField, I96, U96};

// u96 -> Arrow FixedSizeBinary(12) (lossy) + a "yggdryl.logical_type" = "u96" tag...
let field = TypedField::<U96>::new("hash", false);
let arrow = field.to_arrow();
assert_eq!(TypedField::<U96>::from_arrow(&arrow), Some(field)); // ...so it round-trips exactly
assert_eq!(TypedField::<I96>::from_arrow(&arrow), None);        // a same-width sibling: not i96

// FixedBinary vs FixedUtf8 (both FixedSizeBinary(N)) are disambiguated by metadata, and any
// user metadata is preserved through the round-trip.
let fu = FixedUtf8Field::new("code", 4, true).with_metadata_entry("charset", "ascii");
let back = FixedUtf8Field::from_arrow(&fu.to_arrow()).unwrap();
assert_eq!(back.metadata().get("charset"), Some("ascii"));
# }
```

### It survives IPC and Parquet

Arrow carries **unknown metadata keys** through its IPC and Parquet serializations, so the
`yggdryl`-namespaced discriminator survives an external round-trip: write a yggdryl column to a
Parquet file with a third-party tool and read it back, and `from_arrow` still recovers `u96` /
`d128` / a naive `ts32` at second resolution. The keys are ordinary string metadata, so they are
visible (as a field's `metadata`) in Python and Node too — the recovery logic itself lives in the
Rust core behind the `arrow` feature. The per-family Arrow sections that rely on this mechanism are
[Fixed-width primitives](fixed/index.md#arrow-interop) (the `FixedSizeBinary` / scale-0 `Decimal`
widths), [Decimals](arrow/decimal.md) (`(precision, scale)`), and
[Temporal](fixed/temporal.md#arrow-interop) (`(unit, timezone)` and the `Ts32`/`Duration32`/`Ts96`
lossy cases).
