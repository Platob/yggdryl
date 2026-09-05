# Values

The canonical [`Scalar`](../types/scalar.md) byte feed, the single `stable_hash` contract, and Arrow row and column digest arrays.

## Contract

| Key | Value |
| --- | --- |
| Owns | `Scalar::as_value_bytes`, `Scalar::write_bytes`, `Scalar::digest`, `stable_hash`, `xxhash::arrow::row_digests`, `column_digests` |
| `as_value_bytes` | payload alone, no tag, no length; borrows, never allocates; `None` for `Null`, `Sequence`, `Mapping`, `Record` |
| `write_bytes` | total prefix-free feed: one [`DataTypeId`](../types/datatype.md) tag byte, then the family's canonical form; every integer little-endian |
| `stable_hash` | XXH3-64 over the feed, equal to `digest(Xxh3).as_u64()`; [`Field`](../types/field.md), [`Uri`](../uri/index.md), `DataType`, `MimeType`, and Iceberg values hash their canonical rendering the same way |
| Widths | `I8(1)`, `I64(1)`, `U8(1)` feed one form; so do `F32(1.5)`/`F64(1.5)` and `D128(100, 2)`/`D256(1, 0)` |
| Recursion | bounded by `DataType::PARSE_RECURSION_LIMIT`; a deeper subtree feeds one reserved `0xff` |
| Arrow column | `UInt32` for XXH32, `UInt64` for XXH64 and XXH3-64, `FixedSizeBinary(16)` big-endian for XXH3-128 |
| Feature flag | `xxhash::arrow` needs the default `arrow` feature |
| Bindings | `Scalar.digest`, `stable_hash`, and a [state's](streaming.md) `write_scalar` reach the feed from Python and JavaScript; `as_value_bytes` and Arrow digests are Rust only |

## Use

`as_value_bytes` is the payload alone, so hashing it agrees with any xxHash over the same UTF-8; `write_bytes` frames every variant.

=== "Rust"

    ```rust
    use yggdryl::{DigestAlgorithm, Scalar, xxhash};

    let symbol = Scalar::from("AAPL");
    assert_eq!(&*symbol.as_value_bytes().unwrap(), b"AAPL");
    assert_eq!(
        xxhash::xxh3(&symbol.as_value_bytes().unwrap()),
        xxhash::xxh3(b"AAPL"),
    );

    // Equal values answer one digest, across widths.
    assert_eq!(Scalar::from(1_i8), Scalar::from(1_i64));
    assert_eq!(
        Scalar::from(1_i8).digest(DigestAlgorithm::Xxh3),
        Scalar::from(1_i64).digest(DigestAlgorithm::Xxh3),
    );
    // Values that differ stay apart, across variant boundaries.
    assert_ne!(
        Scalar::from("1").digest(DigestAlgorithm::Xxh3),
        Scalar::from(0x31_u8).digest(DigestAlgorithm::Xxh3),
    );
    // A null and an empty string are not the same absence.
    assert_ne!(
        Scalar::Null.digest(DigestAlgorithm::Xxh3),
        Scalar::from("").digest(DigestAlgorithm::Xxh3),
    );
    ```

=== "Python"

    ```python
    from yggdryl import Scalar, xxhash

    assert Scalar.from_py("AAPL").digest() == Scalar.from_py("AAPL").digest("xxh3-64")
    assert int(Scalar.from_py("AAPL").digest()) == Scalar.from_py("AAPL").stable_hash()

    # Equal values answer one digest, across widths.
    assert Scalar.decimal(100, 2).digest() == Scalar.decimal(1, 0).digest()
    assert Scalar.float(1.5, 32).digest() == Scalar.float(1.5, 64).digest()
    # Values that differ stay apart, across variant boundaries.
    assert Scalar.from_py("1").digest() != Scalar.from_py(b"1").digest()
    assert Scalar.from_py(None).digest() != Scalar.from_py("").digest()

    state = xxhash.Xxh3()
    state.write_scalar(Scalar.from_py("AAPL"))
    assert state.as_digest() == Scalar.from_py("AAPL").digest()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, xxhash } = require('yggdryl')

    const symbol = Scalar.fromJs('AAPL')
    assert.ok(symbol.digest().equals(symbol.digest('xxh3-64')))
    assert.equal(symbol.digest().value(), symbol.stableHash())

    // Equal values answer one digest, across widths.
    assert.ok(Scalar.decimal(100n, 2).digest().equals(Scalar.decimal(1n, 0).digest()))
    // Values that differ stay apart, across variant boundaries.
    assert.ok(!Scalar.fromJs('1').digest().equals(Scalar.fromJs(Buffer.from('1')).digest()))

    const state = new xxhash.Xxh3()
    state.writeScalar(symbol)
    assert.ok(state.asDigest().equals(symbol.digest()))
    ```


## Encoding

The tag byte is a wire contract: inserting a `DataTypeId` variant anywhere but the end changes stored digests. A digest identifies the value, not its storage width.

| Variant | Tag | Feed after the tag |
| --- | --- | --- |
| `Null` | `null` | nothing |
| `Bool` | `boolean` | `0x00` or `0x01` |
| `I8`..`U128` | `uint128`, or `int128` when negative | magnitude as `u128` little-endian |
| `F16`/`F32`/`F64` | `float64` | the common `f64` reading's IEEE bits, little-endian |
| `D128`/`D256` | `decimal256` | normalized coefficient as `i256` little-endian, then scale as one signed byte |
| `String` | `utf8` | length `u64` little-endian, then UTF-8 |
| `Enum` | `dictionary` | length-prefixed enum identity, then the member ordinal |
| `Bytes` | `binary` | length `u64` little-endian, then the bytes |
| `Geospatial` | `geometry` | length `u64` little-endian, then the WKB |
| `Date32`/`Date64` | `date64` | unit class byte, normalized count as `i128` little-endian, length-prefixed timezone |
| `Time32`/`Time64` | `time64` | as above |
| `DateTime64` | `timestamp` | as above |
| `Duration32`/`Duration64` | `duration64` | as above |
| `Sequence` | `list` | element count `u64` little-endian, then each element's feed |
| `Mapping` | `map` | entry count `u64` little-endian, then each key feed and value feed in stored order |
| `Record` | `struct` | entry count `u64` little-endian, then per sorted entry a length-prefixed name and the value's feed |

## One hash contract

`stable_hash` is XXH3-64 over this feed everywhere; the tree has no second hash family and no second spelling.

Rust only.

```rust
use yggdryl::{DigestAlgorithm, Scalar, text};

let value = Scalar::from("AAPL");
assert_eq!(
    value.stable_hash(),
    value.digest(DigestAlgorithm::Xxh3).as_u64().unwrap(),
);
// Equal values hash equally across widths, which is the invariant every
// binding relies on.
assert_eq!(
    Scalar::from(1_i8).stable_hash(),
    Scalar::from(1_i64).stable_hash()
);
let _ = text::Format::Json;
```

`Field::stable_hash` and `xxhash::xxh3` of the field's canonical rendering are one number reached two ways.

## Arrow row digests

A row is the ordered sequence of its columns, so `row_digests` equals feeding each row's `Scalar` through `write_bytes` without building one. That equality is the test on every datatype family, nulls, nested structs, lists, maps, dictionaries, unions, and geospatial values.

Rust only.

```rust
use std::sync::Arc;

use arrow_array::cast::AsArray as _;
use arrow_array::types::UInt64Type;
use arrow_array::{Int64Array, RecordBatch, StringArray};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
use yggdryl::DigestAlgorithm;
use yggdryl::xxhash::arrow::row_digests;

let batch = RecordBatch::try_new(
    Arc::new(Schema::new(vec![
        ArrowField::new("symbol", ArrowDataType::Utf8, false),
        ArrowField::new("quantity", ArrowDataType::Int64, false),
    ])),
    vec![
        Arc::new(StringArray::from(vec!["AAPL", "MSFT", "AAPL"])),
        Arc::new(Int64Array::from(vec![100, 250, 100])),
    ],
)?;

let digests = row_digests(&batch, DigestAlgorithm::Xxh3)?;
let digests = digests.as_primitive::<UInt64Type>();
// Identical rows answer identical digests, which is what makes this a dedup
// key, a change-detection column, and a hash-join key.
assert_eq!(digests.value(0), digests.value(2));
assert_ne!(digests.value(0), digests.value(1));
```

`column_digests` is the single-column form: each answer is the cell's own value with no row framing.

## Edges

- `as_value_bytes` on `Null`, `Sequence`, `Mapping`, or `Record` -> `None`; these have no payload without a framing.
- `as_value_bytes` on a decimal or temporal -> coefficient or stored count at storage width; scale, unit, and zone are type, not payload.
- Subtree past `DataType::PARSE_RECURSION_LIMIT` -> one reserved `0xff` replaces it; no allocation, no panic; values differing only below that depth collide.
- `Scalar::Null` vs `Scalar::from("")` -> different digests; each feeds its own tag.
- `Scalar::from("1")` vs `Scalar::from(0x31_u8)` -> different digests; the tag byte separates variants.
- Null cell in `column_digests` -> feeds the null tag, so it never collides with an empty string.
- XXH3-128 over rows -> `FixedSizeBinary(16)` of big-endian bytes; no Arrow integer is wide enough.
- Dictionary-encoded text column -> no buffer arm; read through the scalar boundary, same digests.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib -- xxhash::tests::values xxhash::arrow::tests
    cargo test --features "parquet iceberg" -p yggdryl --test allocations -- the_canonical_value_feed_allocates_nothing borrowed_value_bytes_allocate_nothing
    cargo test --features "parquet iceberg" -p yggdryl --test types -- stable_hash
    cargo bench -p yggdryl --bench xxhash -- xxhash_value
    cargo bench -p yggdryl --bench xxhash -- xxhash_stable_hash
    cargo bench -p yggdryl --bench xxhash -- xxhash_row_digests
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/xxhash -k TestValues
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="value digests|feeds a value" node/tests/xxhash/xxhash.test.js
    ```

## Performance

One containerized x86_64 Linux run measures both groups: Intel Xeon @ 2.10 GHz, 4 cores, 16 GiB, rustc 1.94.1 release with thin LTO. The sources are `rust/benchmarks/xxhash/values.rs` and `rust/benchmarks/xxhash/arrow.rs`.

### The value feed

The feed row reuses one state, as an Arrow column does; the digest row builds a fresh state per value. XXH3 keeps its secret on the heap, so that construction is the gap; the feed itself allocates nothing, pinned by `rust/tests/allocations.rs`.

| value | feed into a reused state | `digest` | `stable_hash` |
| --- | ---: | ---: | ---: |
| leaf string | 58.4 ns | 64.7 ns | 54.7 ns |
| integer | 50.8 ns | 57.5 ns | 47.5 ns |
| decimal | 74.9 ns | 81.7 ns | 75.7 ns |
| four-column row | 157.0 ns | 168.7 ns | 158.3 ns |
| 64-field record | 2.32 µs | 2.29 µs | 2.28 µs |
| 32-deep nest | 1.43 µs | 1.43 µs | 1.39 µs |

`stable_hash` on the short canonical renderings it sees; the first column measures the rendering, the second the algorithm alone. At these lengths the hash is the small half by more than an order of magnitude.

| value | `stable_hash` | the same bytes through `xxh3` |
| --- | ---: | ---: |
| field name (19 chars) | 356.1 ns | 6.8 ns |
| URI (52 chars) | 98.1 ns | 7.1 ns |
| datatype expression (78 chars) | 725.5 ns | 13.9 ns |

```bash
cargo bench -p yggdryl --bench xxhash -- xxhash_value
cargo bench -p yggdryl --bench xxhash -- xxhash_stable_hash
```

### Arrow row digests

Each case digests 65,536 rows of four columns. Both paths answer the same digests; reading buffers directly is worth about 1.8x against the dictionary fallback and 2.6x against materializing rows.

| case | time | per row |
| --- | ---: | ---: |
| buffer path (`int64`, `utf8`, `float64`, `utf8`) | 11.47 ms | 175 ns |
| scalar fallback (same shape, text dictionary-encoded) | 20.84 ms | 318 ns |
| materializing each row as a `Scalar` first | 29.32 ms | 447 ns |
| buffer path, XXH3-128 | 11.24 ms | 172 ns |

Answering 128 bits instead of 64 costs nothing.

```bash
cargo bench -p yggdryl --bench xxhash -- xxhash_row_digests
```
