# Values

The canonical [`Scalar`](../types/scalar.md) byte feed, the single `stable_hash` contract, digest holders, and Arrow row and column digest arrays.

## Contract

| Key | Value |
| --- | --- |
| Owns | `as_value_bytes`, `write_bytes`, `Scalar::digest`, `stable_hash`, `apply_arrow_batch` on a state and on `DigestField`, `xxhash::arrow::row_digests`, `column_digests` |
| `as_value_bytes` | payload alone, no tag, no length; borrows, never allocates; `None` for `Null`, `Sequence`, `Mapping`, `Record` |
| `write_bytes` | total prefix-free feed: one [`DataTypeId`](../types/datatype.md) tag byte, then the family's canonical form; integers little-endian |
| `stable_hash` | XXH3-64 over the feed; [`Field`](../types/field.md), [`Uri`](../uri/index.md), `DataType`, `MimeType`, and Iceberg values hash their canonical rendering the same way |
| `row_digests` | the selected values as one `Scalar::Sequence` through `write_bytes`, on every datatype family except `variant`: nulls, nesting, dictionaries, unions, run-end encodings, geospatial |
| Selection | a holder's `digest:sources`, relative to its own Struct; `["*"]` and absence both mean every field except a `digest:role=holder` |
| `apply_arrow_batch` | fills every holder in a batch under a non-null Struct root; the state's running digest is untouched |
| Arrow column | `UInt32` for XXH32, `UInt64` for XXH64 and XXH3-64, `FixedSizeBinary(16)` big-endian for XXH3-128 |
| Feature flag | `xxhash::arrow` needs the default `arrow` feature |
| Bindings | `Scalar.digest`, `stable_hash`, a [state's](streaming.md) `write_scalar`, and `apply_arrow_batch`; `as_value_bytes` and the digest arrays are Rust only |

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

The tag is the value's own [`DataTypeId`](../types/datatype.md), except where a family compares equal across its members and one member's tag then stands for all of them: integers feed `int128` or `uint128` by sign, floats and decimals feed their widest member, the six ASCII datatypes - `ascii`, `ascii(n)`, `country`, `currency`, `mic`, `cfi` - all feed `ascii`, and a geography feeds `geometry`.

| Variant | Tag | Feed after the tag |
| --- | --- | --- |
| `Null` | `null` | nothing |
| `Bool` | `boolean` | `0x00` or `0x01` |
| `I8`..`U128` | `uint128`, or `int128` when negative | magnitude as `u128` little-endian |
| `F16`/`F32`/`F64` | `float64` | the common `f64` reading's IEEE bits, little-endian |
| `D32`..`D256` | `decimal256` | normalized coefficient as `i256` little-endian, then scale as one signed byte |
| `Text` | `utf8` | length `u64` little-endian, then UTF-8 |
| `Ascii` | `ascii` | length `u64` little-endian, then the trimmed text |
| `Uuid` | `uuid` | the 16 big-endian bytes, with no length |
| `Version` | `version` | rendered length `u64` little-endian, then the canonical rendering |
| `Enum` | `dictionary` | length-prefixed enum identity, then the member ordinal |
| `Bytes` | `binary` | length `u64` little-endian, then the bytes |
| `Geospatial` | `geometry` | length `u64` little-endian, then the WKB |
| `Date32`/`Date64` | `date64` | unit class byte, normalized count as `i128` little-endian, length-prefixed timezone |
| `Time32`/`Time64` | `time64` | as above |
| `DateTime64` | `datetime64` | as above |
| `Duration32`/`Duration64` | `duration64` | as above |
| `Interval` | `interval` | months and days as `i32` little-endian, nanoseconds as `i64` little-endian, then the layout unit as one byte |
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

## Filling digest holders

A digest holder is a field carrying `digest:role=holder`. `root.as_digest().apply_arrow_batch(&batch)`
fills every holder the root declares, walking each declared Struct beneath it. That view carries no
seed or secret, so its answer is the same [`stable_hash`](../types/scalar.md) every other reader
computes; a state does the same with its own configuration and a `force` switch.
[`Field::apply_arrow_batch`](../types/field.md#applying-a-schemas-declarations) is the entry point
that runs this step beside the partition one.

```rust
use std::sync::Arc;

use arrow_array::cast::AsArray as _;
use arrow_array::types::UInt64Type;
use arrow_array::{RecordBatch, StringArray};
use arrow_schema::{DataType as ArrowDataType, Field as ArrowField, Schema};
use yggdryl::xxhash::Xxh3;
use yggdryl::{DataType, DigestAlgorithm, Field, Scalar};

let symbol = Field::new("symbol", DataType::Utf8, false);
let mut holder = Field::new("row_digest", DataType::UInt64, false);
holder.as_digest_mut().set_holder()?;
holder.as_digest_mut().set_sources(["symbol"])?;
holder
    .as_digest_mut()
    .set_algorithm(DigestAlgorithm::Xxh3)?;
let root = DataType::from_fields([symbol, holder])?.required_field("row");

// The target root adds the missing holder in its declared position.
let batch = RecordBatch::try_new(
    Arc::new(Schema::new(vec![ArrowField::new(
        "symbol",
        ArrowDataType::Utf8,
        false,
    )])),
    vec![Arc::new(StringArray::from(vec!["AAPL"]))],
)?;

let mut state = Xxh3::with_seed(7);
state.write_bytes(b"an unrelated running stream");
let running = state.as_u64();
let filled = state.apply_arrow_batch(&root, batch, false)?;

let mut expected = Xxh3::with_seed(7);
expected.write_scalar(&Scalar::from_sequence([Scalar::from("AAPL")]));
assert_eq!(
    filled.column(1).as_primitive::<UInt64Type>().value(0),
    expected.as_u64(),
);
assert_eq!(state.as_u64(), running, "filling does not consume the state");
```

Each visible row is framed as an ordered `Scalar::Sequence` and streamed through the canonical value feed. Nested Struct holders are filled deepest first.

| Holder setting | Effect |
| --- | --- |
| `digest:sources` | canonical JSON array of unique non-empty paths, for example `["id","line.price"]`; its order is the feed order, and it states nothing on the fields it names |
| Source syntax | relative to the containing Struct: an exact whole field name wins, then dots descend through Struct fields only |
| `["*"]` or no `digest:sources` | every field of the containing Struct except a holder; `[]` hashes an empty sequence, and `"*"` beside a path is refused |
| Selected nested Struct with one direct holder | feeds that holder's digest payload instead of hashing the Struct again, which is the bypass a nested holder earns |
| `digest:algorithm` | `xxh32`, `xxh64`, `xxh3-64`, or `xxh3-128`; it must fit the holder's storage mapping |
| No `digest:algorithm` | a receiver whose output width fits the holder, with its seed and secret |
| Fresh default by holder type | `int32`/`uint32` picks XXH32, `int64`/`uint64` picks XXH3-64, `fixed_size_binary(16)` picks XXH3-128 |
| `force=false` | a cell equal to the holder `Field`'s default is computed, every non-default value preserved |
| `force=true` | every visible holder is recomputed |
| `root.as_digest().apply_arrow_batch` | the seedless form: no configuration crosses into it, and `force` is never on |
| Bindings | `field.digest.apply_arrow_batch(batch)` and `field.apply_arrow_batch(batch)` in Python; `state.apply_arrow_batch(root, batch, force=True)`; `state.applyArrowBatch(root, batch, true)` copies the batch through Arrow IPC |

## Arrow row digests

A row is the ordered `Scalar::Sequence` of its selected columns in schema order, element count included. The answer never builds one.

Rust only.

```rust
use std::sync::Arc;

use arrow_array::cast::AsArray as _;
use arrow_array::types::UInt64Type;
use arrow_array::{Int64Array, RecordBatch, StringArray, UInt64Array};
use arrow_schema::Schema;
use yggdryl::{DataType, DigestAlgorithm, Field};
use yggdryl::xxhash::arrow::row_digests;

let symbol = Field::new("symbol", DataType::Utf8, false);
let quantity = Field::new("quantity", DataType::Int64, false);
let mut stored = Field::new("row_digest", DataType::UInt64, false);
stored.as_digest_mut().set_holder()?;

let batch = RecordBatch::try_new(
    Arc::new(Schema::new(vec![
        symbol.into_arrow()?,
        quantity.into_arrow()?,
        stored.into_arrow()?,
    ])),
    vec![
        Arc::new(StringArray::from(vec!["AAPL", "MSFT", "AAPL"])),
        Arc::new(Int64Array::from(vec![100, 250, 999])),
        Arc::new(UInt64Array::from(vec![11, 22, 33])),
    ],
)?;

let digests = row_digests(&batch, DigestAlgorithm::Xxh3)?;
let digests = digests.as_primitive::<UInt64Type>();
// Every column but the holder feeds the digest. Rows 0 and 2 differ only in
// the quantity, so they differ here; the prior holder value never feeds.
assert_ne!(digests.value(0), digests.value(2));
assert_ne!(digests.value(0), digests.value(1));
```

| Schema | Selected values |
| --- | --- |
| Any schema | every field except a `digest:role=holder`, in schema order |
| Only holders | the empty sequence, for every row |

`row_digests` reads a batch, not a holder, so it always takes that whole selection; narrowing is a holder's `digest:sources` and belongs to [`apply_arrow_batch`](#filling-digest-holders). Names, roles, and other metadata choose the values but never enter the byte feed. The [`DigestField` selection helpers](../types/protocol.md) answer the same set without hashing a batch.

`row_digests` always uses its `algorithm` argument and the selection above. `column_digests` is the single-column form, each answer the cell's own value with no row framing.

## Edges

- `as_value_bytes` on a decimal or temporal -> coefficient or stored count at storage width; scale, unit, and zone are type, not payload.
- Subtree past `DataType::PARSE_RECURSION_LIMIT` -> one reserved `0xff` replaces it; no allocation, no panic; values differing only below that depth collide.
- Null cell in `column_digests` -> feeds the null tag, so it never collides with an empty string.
- A `variant` column -> refused by name; its binary encoding lands with the Iceberg v3 layer, so there is no value to feed.
- A `field` whose datatype does not describe the array given to `column_digests` -> `IncompatibleSchema`, never a panic.
- The same value on a big-endian machine -> the same digest; every integer in the feed is little-endian.
- A `country`, `currency`, `mic`, `cfi`, `ascii(n)`, or `ascii` cell holding the same text -> one digest; the six compare equal and all feed the `ascii` tag.
- A `geometry` and a `geography` cell over the same WKB -> one digest; both feed the `geometry` tag.
- Holder-local `digest:sources` or `digest:algorithm` -> ignored by `row_digests`; they configure [`apply_arrow_batch`](#filling-digest-holders) only.
- A path through a list, map, or union -> that value is selected whole, never traversed.
- A holder, or `digest:sources`/`digest:algorithm`, under a list, map, union, dictionary, or run-end layout -> refused by path; a fill descends into Struct children only.
- A holder that selects itself or another holder in the same Struct -> refused.
- A Struct with several direct holders -> ambiguous; name the intended nested holder by a path.
- A holder column missing from the batch -> added in the position the root declares.
- Children below a null Struct -> not read and not changed.
- A non-default holder under `force=false` -> preserved, though nothing proves which algorithm, seed, or secret produced it.
- A nullable holder -> null is unfilled and a present zero is kept; a required integer holder reads zero as unfilled.
- An algorithm differing from the receiver's, declared or resolved from a holder's width -> a fresh unseeded state, so a seed and a secret reach only the holders sharing that receiver's width.
- `DigestField::apply_arrow_batch` -> the seedless state's answer, never forcing; a seed, a secret, or a recompute is a state's `apply_arrow_batch`.
- A signed holder -> bit-cast to the same-width unsigned payload, so `int32`/`uint32` and `int64`/`uint64` schemas answer one digest.
- A set high bit in a signed holder -> reads as a negative integer, with no overflow and no loss.

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

One containerized x86_64 Linux run (Intel Xeon @ 2.10 GHz, 4 cores, 16 GiB; rustc 1.94.1 release with thin LTO) measures `rust/benchmarks/xxhash/values.rs` and `arrow.rs`.

### The value feed

The feed row reuses one state, as an Arrow column does; the digest row builds a fresh state per value. The feed allocates nothing, which `rust/tests/allocations.rs` pins with a counting allocator.

| value | feed into a reused state | `digest` | `stable_hash` |
| --- | ---: | ---: | ---: |
| leaf string | 58.4 ns | 64.7 ns | 54.7 ns |
| integer | 50.8 ns | 57.5 ns | 47.5 ns |
| decimal | 74.9 ns | 81.7 ns | 75.7 ns |
| four-column row | 157.0 ns | 168.7 ns | 158.3 ns |
| 64-field record | 2.32 µs | 2.29 µs | 2.28 µs |
| 32-deep nest | 1.43 µs | 1.43 µs | 1.39 µs |

`stable_hash` on the short canonical renderings it sees measures the rendering; the second column is the algorithm alone.

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

Each case digests 65,536 rows of four columns. Both paths answer the same digests; the fallback's dictionary-encoded text has no buffer arm and reads through the scalar boundary.

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
