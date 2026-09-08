# Arrow

Coupled columns: a batch beside an instant column becomes one `fixed_size_binary` column, the halves split and join, and a `digest:time` holder is filled by every `apply_arrow_batch`.

## Contract

| Key | Value |
| --- | --- |
| Owns | `txhash::arrow::unix_array`, `row_txhashes`, `column_txhashes`, `compose`, `decompose`; `TxHasher::row_txhashes`, `column_txhashes`, `apply_arrow_batch`; `digest:time` and `digest:unit` on a holder |
| Digest half | Exactly [`row_digests`](../xxhash/values.md#arrow-row-digests) or `column_digests` of the same rows under the same algorithm; this layer hashes nothing on its own |
| Instant column | A timestamp of any resolution and zone, a date, or an integer column; read once as `int64` unix counts at the declared unit, nulls kept |
| Answer | `fixed_size_binary(12|16|24)`, one coupled value per row, null where the instant is |
| `compose` / `decompose` | Inverses: an instant column plus a digest column of the algorithm's width, and back to a UTC `timestamp` of the unit plus that digest column |
| `digest:time` | The path of the field whose instant a holder stores in front of its digest, relative to the holder's Struct, spelled as `digest:sources` are |
| `digest:unit` | The clock resolution the holder counts its instant in; absent means microseconds; only beside `digest:time` |
| Holder storage | `fixed_size_binary[12]`, `[16]`, or `[24]`; sixteen bytes imply XXH3-64 when no `digest:algorithm` is declared |
| Feature flag | `txhash::arrow` needs the default `arrow` feature |
| Bindings | Python takes and answers `pyarrow` arrays for every column function; JavaScript fills coupled holders through `TxHasher.applyArrowBatch` and keeps the column functions Rust and Python only |

## Use

A row's coupled value is its row digest with the instant beside it, so a coupled column and a plain digest column agree wherever they overlap.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array as _, Int64Array, RecordBatch, StringArray, TimestampMicrosecondArray};
    use arrow_schema::{DataType, Field, Schema};
    use yggdryl::xxhash::arrow::row_digests;
    use yggdryl::{DigestAlgorithm, TimeUnit, txhash};

    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![
            Field::new("symbol", DataType::Utf8, false),
            Field::new("quantity", DataType::Int64, false),
        ])),
        vec![
            Arc::new(StringArray::from(vec!["AAPL", "MSFT", "AAPL"])),
            Arc::new(Int64Array::from(vec![100, 250, 100])),
        ],
    )?;
    let instants = TimestampMicrosecondArray::from(vec![
        1_700_000_000_000_000,
        1_700_000_000_000_001,
        1_700_000_000_000_002,
    ])
    .with_timezone("UTC");

    let coupled = txhash::arrow::row_txhashes(&batch, &instants, TimeUnit::Microsecond, DigestAlgorithm::Xxh3)?;
    assert_eq!(coupled.data_type(), &DataType::FixedSizeBinary(16));

    // The halves come back out: the instant column, and exactly the row digests.
    let (times, digests) = txhash::arrow::decompose(coupled.as_ref(), TimeUnit::Microsecond, DigestAlgorithm::Xxh3)?;
    assert_eq!(&digests, &row_digests(&batch, DigestAlgorithm::Xxh3)?);
    assert_eq!(times.as_ref(), &instants as &dyn arrow_array::Array);
    assert_eq!(&txhash::arrow::compose(times.as_ref(), digests.as_ref(), TimeUnit::Microsecond, DigestAlgorithm::Xxh3)?, &coupled);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import txhash, xxhash

    batch = pa.record_batch(
        {"symbol": pa.array(["AAPL", "MSFT", "AAPL"]), "quantity": pa.array([100, 250, 100])}
    )
    instants = pa.array(
        [1_700_000_000_000_000, 1_700_000_000_000_001, 1_700_000_000_000_002],
        pa.timestamp("us", tz="UTC"),
    )

    coupled = txhash.row_txhashes(batch, instants)
    assert coupled.type == pa.binary(16)

    # The halves come back out: the instant column, and exactly the row digests.
    times, digests = txhash.decompose(coupled)
    assert digests == xxhash.row_digests(batch)
    assert times == instants
    assert txhash.compose(times, digests) == coupled
    # Rows 0 and 2 hold the same content at different instants: same digest, different key.
    assert txhash.TxHash.from_bytes("us", "xxh3-64", coupled[0].as_py()).digest == txhash.TxHash.from_bytes("us", "xxh3-64", coupled[2].as_py()).digest
    assert coupled[0] != coupled[2]
    ```

=== "JavaScript"

    The coupled columns are Rust and Python only; JavaScript couples one value at a time and fills holders through [`TxHasher.applyArrowBatch`](#coupled-holders).

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, txhash } = require('yggdryl')

    const row = Scalar.fromJs(['AAPL', 100])
    const first = new txhash.TxHasher().digestScalar(row, 1_700_000_000_000_000n)
    const third = new txhash.TxHasher().digestScalar(row, 1_700_000_000_000_002n)
    assert.ok(first.digest.equals(third.digest))
    assert.ok(!first.equals(third))
    ```

## Instant columns

`unix_array` reads every instant column the same way a scalar is read: a timestamp of any resolution or zone, a date, or an integer, restated at the unit asked for.

=== "Rust"

    ```rust
    use arrow_array::{Array as _, Date32Array, Int64Array, TimestampNanosecondArray, TimestampSecondArray};
    use yggdryl::{TimeUnit, txhash};

    let micros = |array: &dyn arrow_array::Array| txhash::arrow::unix_array(array, TimeUnit::Microsecond);
    let seconds = TimestampSecondArray::from(vec![Some(1_700_000_000), None]).with_timezone("Asia/Kolkata");
    let read = micros(&seconds)?;
    // The zone moves nothing: the count already names the instant.
    assert_eq!(read.value(0), 1_700_000_000_000_000);
    assert!(read.is_null(1));
    // A finer count floors, a date is its midnight, an integer is the count already.
    assert_eq!(micros(&TimestampNanosecondArray::from(vec![1_999, -1]))?.values(), &[1, -1]);
    assert_eq!(micros(&Date32Array::from(vec![1]))?.values(), &[86_400_000_000]);
    assert_eq!(micros(&Int64Array::from(vec![42]))?.values(), &[42]);
    ```

=== "Python"

    ```python
    import datetime as dt

    import pyarrow as pa

    from yggdryl import txhash

    seconds = pa.array([1_700_000_000, None], pa.timestamp("s", tz="Asia/Kolkata"))
    # The zone moves nothing: the count already names the instant.
    assert txhash.unix_array(seconds).to_pylist() == [1_700_000_000_000_000, None]
    # A finer count floors, a date is its midnight, an integer is the count already.
    assert txhash.unix_array(pa.array([1_999, -1], pa.timestamp("ns"))).to_pylist() == [1, -1]
    assert txhash.unix_array(pa.array([dt.date(1970, 1, 2)])).to_pylist() == [86_400_000_000]
    assert txhash.unix_array(pa.array([42], pa.int64())).to_pylist() == [42]
    assert txhash.unix_array(seconds, "s").to_pylist() == [1_700_000_000, None]
    ```

=== "JavaScript"

    Rust and Python only; one instant at a time is [`txhash.unixOf`](index.md#instants).

    ```javascript
    const assert = require('node:assert/strict')
    const { txhash } = require('yggdryl')

    assert.equal(txhash.unixOf(new Date('2023-11-14T22:13:20Z')), 1_700_000_000_000_000n)
    ```

## Coupled holders

A holder naming `digest:time` stores the instant it names in front of its digest. Everything else about it is the [digest holder contract](../xxhash/values.md#filling-digest-holders): `digest:sources` narrows what the digest reads, the instant column included by default; `digest:algorithm` or the storage width picks the algorithm; a written cell is preserved unless forced.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Array as _, RecordBatch, StringArray, TimestampMicrosecondArray};
    use arrow_schema::Schema;
    use yggdryl::txhash::TxHash;
    use yggdryl::{ArrowCastOptions, DataType, DigestAlgorithm, Field, Scalar, TimeUnit, Timezone};

    let event = Field::new("event", DataType::DateTime64 { unit: TimeUnit::Microsecond, timezone: Timezone::UTC }, false);
    let symbol = Field::new("symbol", DataType::Utf8, false);
    let mut key = Field::new("key", DataType::FixedSizeBinary(16), false);
    key.as_digest_mut().set_holder()?;
    key.as_digest_mut().set_time("event")?;
    key.as_digest_mut().set_unit(TimeUnit::Second)?;
    let root = DataType::from_fields([event.clone(), symbol.clone(), key])?.required_field("row");

    let batch = RecordBatch::try_new(
        Arc::new(Schema::new(vec![event.into_arrow()?, symbol.into_arrow()?])),
        vec![
            Arc::new(TimestampMicrosecondArray::from(vec![1_700_000_000_000_000, 1_700_000_000_999_999]).with_timezone("UTC")),
            Arc::new(StringArray::from(vec!["AAPL", "MSFT"])),
        ],
    )?;

    // The schema pipeline fills the holder, and filling again changes nothing.
    let filled = root.apply_arrow_batch(&batch, true, true, true, ArrowCastOptions::new())?;
    assert_eq!(root.apply_arrow_batch(&filled, true, true, true, ArrowCastOptions::new())?, filled);

    let cells = filled.column(2).as_any().downcast_ref::<arrow_array::FixedSizeBinaryArray>().unwrap();
    let second = TxHash::from_bytes(TimeUnit::Second, DigestAlgorithm::Xxh3, cells.value(1))?;
    // The instant floors to the declared unit; the digest reads every field but the holder.
    assert_eq!(second.unix(), 1_700_000_000);
    let row = Scalar::from_sequence([
        Scalar::from_datetime(1_700_000_000_999_999, TimeUnit::Microsecond, Timezone::UTC)?,
        Scalar::from("MSFT"),
    ]);
    assert_eq!(second.digest(), row.digest(DigestAlgorithm::Xxh3));
    ```

=== "Python"

    ```python
    import datetime as dt

    import pyarrow as pa

    from yggdryl import DataType, Field, Scalar, txhash

    key = Field("key", "fixed_size_binary[16]", nullable=False)
    key.digest.set_holder()
    key.digest.time = "event"
    key.digest.unit = "s"
    root = Field(
        "row",
        DataType.from_fields([Field("event", "timestamp[us, UTC]", nullable=False), Field("symbol", "utf8", nullable=False), key]),
        nullable=False,
    )
    batch = pa.record_batch(
        {
            "event": pa.array([1_700_000_000_000_000, 1_700_000_000_999_999], pa.timestamp("us", tz="UTC")),
            "symbol": pa.array(["AAPL", "MSFT"]),
        }
    )

    # The schema pipeline fills the holder, and filling again changes nothing.
    filled = root.apply_arrow_batch(batch)
    assert root.apply_arrow_batch(filled) == filled

    second = txhash.TxHash.from_bytes("s", "xxh3-64", filled.column("key")[1].as_py())
    # The instant floors to the declared unit; the digest reads every field but the holder.
    assert second.unix == 1_700_000_000
    row = Scalar.from_py([dt.datetime(2023, 11, 14, 22, 13, 20, 999_999, tzinfo=dt.timezone.utc), "MSFT"])
    assert second.digest == row.digest()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { DataType, Field, Scalar, TxHash, TxHasher } = require('yggdryl')

    const key = new Field('key', 'fixed_size_binary[16]', false, {
      'digest:role': 'holder',
      'digest:time': 'event',
      'digest:unit': 's',
    })
    const root = new Field(
      'row',
      DataType.fromFields([new Field('event', 'int64', false), new Field('symbol', 'utf8', false), key]),
      false,
    )
    const batch = new arrow.Table({
      event: arrow.vectorFromArray([1_700_000_000n, 1_700_000_001n], new arrow.Int64()),
      symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
    }).batches[0]

    const filled = new TxHasher().applyArrowBatch(root, batch)
    const second = TxHash.fromBytes('s', 'xxh3-64', filled.getChild('key').get(1))
    // An integer instant is the count already; the digest reads every field but the holder.
    assert.equal(second.unix, 1_700_000_001n)
    assert.ok(second.digest.equals(Scalar.fromJs([1_700_000_001n, 'MSFT']).digest()))
    ```

| Holder setting | Effect |
| --- | --- |
| `digest:time` | one field path relative to the containing Struct, resolved as a source is; the field must be a datetime, a date, or an integer, never a holder |
| `digest:unit` | `s`, `ms`, `us`, or `ns`, canonicalized on write; refused without `digest:time`; absent means microseconds |
| storage | `fixed_size_binary[12]` for XXH32, `[16]` for XXH64 or XXH3-64, `[24]` for XXH3-128; `digest:algorithm` must fit it |
| the instant column | feeds the digest like any other column unless `digest:sources` leaves it out |
| a null instant | a nullable holder stores null; a required holder refuses, naming the row |
| removal order | `remove_unit`, then `remove_time`, then `remove_role`; each refuses while what depends on it stands |

## Edges

- An instant column shorter or longer than the batch -> refused by length.
- A text, boolean, float, list, or dictionary column as the instant -> refused by datatype; it is read as a column, never per cell.
- A `uint64` instant above `i64::MAX` -> refused naming the value.
- An empty batch -> an empty coupled column of the right width.
- A null instant -> a null coupled cell in every column function; the digest functions answer no nulls, so a coupled column carries exactly the instant column's nulls.
- `compose` with a digest column of another width -> refused naming the expected width; `int32` and `int64` are read as the same bits `uint32` and `uint64` hold.
- `decompose` on a `fixed_size_binary` of another width -> refused naming the coupled width.
- A `digest:time` path through a list, map, union, dictionary, or run-end layout -> refused by path, as a holder there is.
- `digest:time` or `digest:unit` off a holder -> refused as belonging only to a holder.
- Nested Structs -> the instant is read after nested holders are final, and a row null at any Struct above the leaf is null.
- A seeded `TxHasher` -> its seed and secret reach every holder sharing its algorithm's width, as `Digester::apply_arrow_batch` states; the holder's own `digest:unit` always wins over the hasher's.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib txhash::arrow::tests
    cargo bench -p yggdryl --bench txhash -- txhash_columns
    cargo bench -p yggdryl --bench txhash -- txhash_apply_arrow_batch
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/txhash -k "TestColumns or TestHolders"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="coupled holder" node/tests/txhash/txhash.test.js
    ```

## Performance

One containerized x86_64 Linux run (Intel Xeon 2.10 GHz, 4 cores, 16 GiB; rustc 1.94.1 release with thin LTO) measures `rust/benchmarks/txhash/arrow.rs`. Each case runs 65,536 rows.

The coupled column costs what the digest column costs plus one eight-byte copy per row; the instant column is free when it is already at the holder's unit and one pass of integer division otherwise.

| case | time | rows/s |
| --- | ---: | ---: |
| `xxhash::arrow::row_digests`, XXH3-64 | 14.02 ms | 4.68 M |
| `row_txhashes`, XXH3-64 | 13.65 ms | 4.80 M |
| `row_txhashes`, XXH128 | 14.57 ms | 4.50 M |
| `xxhash::arrow::column_digests`, one text column | 6.33 ms | 10.4 M |
| `column_txhashes`, one text column | 6.85 ms | 9.57 M |
| `unix_array`, microseconds read as microseconds | 72.7 ns | shares the buffer |
| `unix_array`, nanoseconds floored to microseconds | 574 µs | 114 M |
| `compose` | 724 µs | 90.5 M |
| `decompose` | 787 µs | 83.2 M |

The holder fill pays for the digest of every source; the coupling adds the instant read and the wider cell. Both rows fill one 65,536-row batch through `DigestField::apply_arrow_batch`, IPC excluded.

| holder | time | rows/s |
| --- | ---: | ---: |
| plain `uint64` holder | 28.6 ms | 2.29 M |
| coupled `fixed_size_binary[16]` holder | 32.5 ms | 2.01 M |

```bash
cargo bench -p yggdryl --bench txhash -- txhash_columns
cargo bench -p yggdryl --bench txhash -- txhash_apply_arrow_batch
```
