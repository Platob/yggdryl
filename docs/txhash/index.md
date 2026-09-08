# TxHash

`yggdryl::txhash` couples a UTC instant with an xxHash digest into one sortable value: the unix count first, the digest after it.

## Pages

| Page | Purpose |
| --- | --- |
| [Arrow](arrow.md) | Coupled columns from a batch and an instant column, splitting and joining the halves, and the `digest:time` holder. |

## Contract

| | |
| --- | --- |
| Owns | `TxHash`, `TxHasher`, `txh32`, `txh64`, `txh3`, `txh128`, `digest`, `restate_unix`, `unix_from_scalar`, `unix_now`, `width`, `dtype`, `Scalar::txhash`. |
| Value | `unit`, `unix`, `digest`; the digest is exactly what [xxHash](../xxhash/index.md) answers for the same bytes, so this module defines no second hash. |
| Bytes | The instant as a big-endian `i64`, then the digest's canonical bytes: 12, 16, or 24 bytes for XXH32, the two 64-bit algorithms, XXH3-128. |
| Order | By unit, then instant, then digest; the bytes sort the same way from the epoch on. |
| Instant | UTC always: a zoned datetime already counts from the epoch, a naive one reads as if it were UTC, a date is its midnight. |
| Unit | A clock resolution - `s`, `ms`, `us`, `ns` - microseconds when none is named; a coarser restatement floors, a finer one scales exactly. |
| Spelling | `<unix>@<unit>:<algorithm>:<hex>`; `from_str` is the exact inverse; two units or two algorithms are never equal. |
| Storage | `fixed_size_binary[12|16|24]`; sixteen bytes imply XXH3-64, the project default. |
| Arguments | Input first, instant second, as the seed sits second in every seeded digest call. |
| Bindings | Every `unix` argument is an `int` / `bigint`, a `datetime` / `Date`, timestamp text, or a `Scalar`; JavaScript keeps the coupled columns Rust and Python only. |

## Use

The four one-shot functions couple a microsecond instant with the plain digest of a buffer.

=== "Rust"

    ```rust
    use yggdryl::{DigestAlgorithm, txhash, xxhash};

    let instant = 1_700_000_000_000_000; // 2023-11-14T22:13:20Z in microseconds
    let value = txhash::txh3(b"AAPL", instant);

    assert_eq!(value.unix(), instant);
    assert_eq!(value.digest(), DigestAlgorithm::Xxh3.digest(b"AAPL"));
    assert_eq!(value.width(), 16);

    // The bytes are the instant, then the digest, so they sort by time first.
    let bytes = value.into_bytes();
    assert_eq!(&bytes[..8], &instant.to_be_bytes());
    assert_eq!(&bytes[8..], &xxhash::xxh3(b"AAPL").to_be_bytes());
    assert!(txhash::txh3(b"AAPL", instant + 1).into_bytes() > bytes);
    ```

=== "Python"

    ```python
    from yggdryl import txhash, xxhash

    instant = 1_700_000_000_000_000  # 2023-11-14T22:13:20Z in microseconds
    value = txhash.txh3(b"AAPL", instant)

    assert value.unix == instant
    assert int(value.digest) == xxhash.xxh3(b"AAPL")
    assert value.width == len(bytes(value)) == 16

    # The bytes are the instant, then the digest, so they sort by time first.
    assert bytes(value)[:8] == instant.to_bytes(8, "big", signed=True)
    assert bytes(value)[8:] == bytes(value.digest)
    assert bytes(txhash.txh3(b"AAPL", instant + 1)) > bytes(value)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { txhash, xxhash } = require('yggdryl')

    const instant = 1_700_000_000_000_000n // 2023-11-14T22:13:20Z in microseconds
    const value = txhash.txh3('AAPL', instant)

    assert.equal(value.unix, instant)
    assert.equal(value.digest.value(), xxhash.xxh3('AAPL'))
    assert.equal(value.width, 16)

    // The bytes are the instant, then the digest, so they sort by time first.
    const bytes = Buffer.from(value.bytes())
    assert.equal(bytes.readBigInt64BE(0), instant)
    assert.ok(Buffer.compare(Buffer.from(txhash.txh3('AAPL', instant + 1n).bytes()), bytes) > 0)
    ```

## The value

The spelling names the unit and the algorithm, because both are part of the value: without them a microsecond and a nanosecond count of the same digits would share one rendering.

=== "Rust"

    ```rust
    use yggdryl::txhash::TxHash;
    use yggdryl::{DigestAlgorithm, Scalar, TimeUnit, Timezone, txhash};

    let value = txhash::digest(b"abc", 0, DigestAlgorithm::Xxh128);
    assert_eq!(value.to_string(), "0@us:xxh3-128:06b05ab6733a618578af5f94892f3950");
    assert_eq!(TxHash::from_str(&value.to_string())?, value);
    assert_eq!(TxHash::from_bytes(TimeUnit::Microsecond, DigestAlgorithm::Xxh128, &value.into_bytes())?, value);

    // A coarser unit floors the instant and keeps the digest.
    let seconds = txhash::txh3(b"AAPL", 1_700_000_000_999_999).with_unit(TimeUnit::Second)?;
    assert_eq!(seconds.unix(), 1_700_000_000);
    assert_eq!(seconds.digest(), DigestAlgorithm::Xxh3.digest(b"AAPL"));
    assert_eq!(
        seconds.into_datetime(),
        Scalar::from_datetime(1_700_000_000, TimeUnit::Second, Timezone::UTC)?,
    );
    // Two units, like two algorithms, are never equal.
    assert_ne!(seconds, seconds.with_unit(TimeUnit::Millisecond)?);
    ```

=== "Python"

    ```python
    import datetime as dt

    from yggdryl import txhash

    value = txhash.digest(b"abc", 0, "xxh3-128")
    assert str(value) == "0@us:xxh3-128:06b05ab6733a618578af5f94892f3950"
    assert txhash.TxHash(str(value)) == value
    assert txhash.TxHash.from_bytes("us", "xxh3-128", bytes(value)) == value

    # A coarser unit floors the instant and keeps the digest.
    seconds = txhash.txh3(b"AAPL", 1_700_000_000_999_999).with_unit("s")
    assert seconds.unix == 1_700_000_000
    assert seconds.digest == txhash.txh3(b"AAPL", 0).digest
    assert seconds.into_datetime().as_py() == dt.datetime(2023, 11, 14, 22, 13, 20, tzinfo=dt.timezone.utc)
    # Two units, like two algorithms, are never equal.
    assert seconds != seconds.with_unit("ms")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, TxHash, txhash } = require('yggdryl')

    const value = txhash.digest('abc', 0, 'xxh3-128')
    assert.equal(value.toString(), '0@us:xxh3-128:06b05ab6733a618578af5f94892f3950')
    assert.ok(TxHash.from(value.toString()).equals(value))
    assert.ok(TxHash.fromBytes('us', 'xxh3-128', value.bytes()).equals(value))

    // A coarser unit floors the instant and keeps the digest.
    const seconds = txhash.txh3('AAPL', 1_700_000_000_999_999n).withUnit('s')
    assert.equal(seconds.unix, 1_700_000_000n)
    assert.ok(seconds.digest.equals(txhash.txh3('AAPL', 0).digest))
    assert.ok(seconds.intoDatetime().equals(Scalar.datetime(1_700_000_000n, 's', 'UTC')))
    // Two units, like two algorithms, are never equal.
    assert.ok(!seconds.equals(seconds.withUnit('ms')))
    ```

## Instants

Every spelling of an instant resolves to one unix count: an integer is the count already, a datetime is its instant whatever its zone, a date is its midnight, and text reads as a timestamp with or without an offset, or as a date.

=== "Rust"

    ```rust
    use yggdryl::{Scalar, TimeUnit, Timezone, txhash};

    let unit = TimeUnit::Microsecond;
    let kolkata = Timezone::from_str("Asia/Kolkata")?;
    // A zoned datetime already counts from the epoch; the zone moves nothing.
    assert_eq!(
        txhash::unix_from_scalar(&Scalar::from_datetime(1_700_000_000, TimeUnit::Second, kolkata)?, unit)?,
        1_700_000_000_000_000,
    );
    assert_eq!(txhash::unix_from_scalar(&Scalar::from("1970-01-01T00:00:01+01:00"), unit)?, -3_599_000_000);
    assert_eq!(txhash::unix_from_scalar(&Scalar::date32(1), unit)?, 86_400_000_000);

    // A finer count floors, so two instants keep their order across units.
    assert_eq!(txhash::restate_unix(1_999, TimeUnit::Nanosecond, unit)?, 1);
    assert_eq!(txhash::restate_unix(-1, TimeUnit::Nanosecond, unit)?, -1);
    assert!(txhash::restate_unix(i64::MAX, TimeUnit::Second, unit).is_err());

    // The clock, at the unit asked for.
    assert!(txhash::unix_now(TimeUnit::Second)? > 1_700_000_000);
    ```

=== "Python"

    ```python
    import datetime as dt

    from yggdryl import txhash

    aware = dt.datetime(2023, 11, 14, 22, 13, 20, tzinfo=dt.timezone.utc)
    kolkata = aware.astimezone(dt.timezone(dt.timedelta(hours=5, minutes=30)))
    # A zoned datetime already counts from the epoch; the zone moves nothing.
    assert txhash.unix_of(kolkata) == txhash.unix_of(aware) == 1_700_000_000_000_000
    assert txhash.unix_of("1970-01-01T00:00:01+01:00") == -3_599_000_000
    assert txhash.unix_of(dt.date(1970, 1, 2)) == 86_400_000_000
    assert txhash.unix_of(aware, unit="s") == 1_700_000_000

    # A finer count floors, so two instants keep their order across units.
    assert txhash.restate_unix(1_999, "ns", "us") == 1
    assert txhash.restate_unix(-1, "ns", "us") == -1

    # The clock, at the unit asked for.
    assert txhash.unix_now("s") > 1_700_000_000
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { txhash } = require('yggdryl')

    const aware = new Date('2023-11-14T22:13:20Z')
    // A Date is its UTC instant; text with an offset already counts from the epoch.
    assert.equal(txhash.unixOf(aware), 1_700_000_000_000_000n)
    assert.equal(txhash.unixOf('2023-11-15T03:43:20+05:30'), 1_700_000_000_000_000n)
    assert.equal(txhash.unixOf('1970-01-01T00:00:01+01:00'), -3_599_000_000n)
    assert.equal(txhash.unixOf(aware, 's'), 1_700_000_000n)

    // A finer count floors, so two instants keep their order across units.
    assert.equal(txhash.restateUnix(1_999n, 'ns', 'us'), 1n)
    assert.equal(txhash.restateUnix(-1, 'ns', 'us'), -1n)

    // The clock, at the unit asked for.
    assert.ok(txhash.unixNow('s') > 1_700_000_000n)
    ```

## Hasher

`TxHasher` settles the resolution, algorithm, seed, and secret once; a secret reaches it through the configured state, as the states carry one.

=== "Rust"

    ```rust
    use yggdryl::txhash::TxHasher;
    use yggdryl::xxhash::{self, Xxh3};
    use yggdryl::{DigestAlgorithm, Scalar, TimeUnit};

    let seconds = TxHasher::new_in(TimeUnit::Second, DigestAlgorithm::Xxh64)?.with_seed(7);
    let value = seconds.digest(b"AAPL", 1_700_000_000);
    assert_eq!(value.unit(), TimeUnit::Second);
    assert_eq!(value.digest().as_u64(), Some(xxhash::xxh64_with_seed(b"AAPL", 7)));
    assert_eq!(seconds.unix_of(&Scalar::from("2023-11-14T22:13:20Z"))?, 1_700_000_000);

    // A value's canonical feed, under the same configuration.
    let row = seconds.digest_scalar(&Scalar::from_sequence([Scalar::from("AAPL"), Scalar::from(100)]), 1_700_000_000);
    assert_ne!(row.digest(), value.digest());

    // A secret travels through the state that holds it.
    let secret = vec![0x5a_u8; xxhash::SECRET_MINIMUM_LENGTH];
    let secretive = TxHasher::from_digester(TimeUnit::Millisecond, Xxh3::from_secret(&secret)?.into())?;
    let long = vec![0x11_u8; 241];
    assert_eq!(secretive.digest(&long, 5).digest().as_u64(), Some(xxhash::xxh3_with_secret(&long, &secret)?));
    ```

=== "Python"

    ```python
    from yggdryl import Scalar, txhash, xxhash

    seconds = txhash.TxHasher("xxh64", unit="s", seed=7)
    value = seconds.digest(b"AAPL", 1_700_000_000)
    assert value.unit == "s"
    assert int(value.digest) == xxhash.xxh64(b"AAPL", seed=7)
    assert seconds.unix_of("2023-11-14T22:13:20Z") == 1_700_000_000

    # A value's canonical feed, under the same configuration.
    row = seconds.digest_scalar(Scalar.from_py(["AAPL", 100]), 1_700_000_000)
    assert row.digest != value.digest

    # A secret travels through the state that holds it.
    secret = bytes(xxhash.SECRET_MINIMUM_LENGTH)
    secretive = txhash.TxHasher.from_state(xxhash.Xxh3(secret=secret), unit="ms")
    long = b"\x11" * 241
    assert int(secretive.digest(long, 5).digest) == xxhash.xxh3(long, secret=secret)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Scalar, TxHasher, xxhash } = require('yggdryl')

    const seconds = new TxHasher('xxh64', 's', 7n)
    const value = seconds.digest('AAPL', 1_700_000_000n)
    assert.equal(value.unit, 's')
    assert.equal(value.digest.value(), xxhash.xxh64('AAPL', { seed: 7n }))
    assert.equal(seconds.unixOf('2023-11-14T22:13:20Z'), 1_700_000_000n)

    // A value's canonical feed, under the same configuration.
    const row = seconds.digestScalar(Scalar.fromJs(['AAPL', 100]), 1_700_000_000n)
    assert.ok(!row.digest.equals(value.digest))

    // A secret travels through the state that holds it.
    const secret = new Uint8Array(xxhash.SECRET_MINIMUM_LENGTH)
    const secretive = TxHasher.fromState(new xxhash.Xxh3(0n, secret), 'ms')
    const long = Buffer.alloc(241, 0x11)
    assert.equal(secretive.digest(long, 5n).digest.value(), xxhash.xxh3(long, { secret }))
    ```

## Edges

- An instant before the epoch -> orders first as a value and last as bytes; a big-endian two's complement count sorts every instant from the epoch on and no other.
- A count that does not fit the finer unit -> `ArithmeticOverflow`, never a wrapped count.
- `d`, `year_month`, `day_time`, `month_day_nano` as a unit -> refused; a unix count is a clock resolution.
- A time of day, a duration, an interval, a null, a boolean, or a float as an instant -> refused by kind.
- Timestamp text -> read at the resolution its own digits spell, then restated like every other intake; `1970-01-01T00:00:00.0000019` at microseconds is `1`.
- Date-only text -> that day's midnight, as a date scalar is.
- Timestamp text past the year 2262 with seven or more fractional digits -> refused as out of range; those digits name nanoseconds, and a signed 64-bit count of them ends there.
- A spelling naming another unit or algorithm than `from_scalar` asks for -> refused rather than restated.
- The same sixteen bytes read under XXH64 and XXH3-64 -> two values that render and compare apart, because the algorithm is part of the value.
- `TxHasher::from_digester` -> bytes already fed to the state are discarded; a hasher is a configuration, not a running digest.
- `TxHasher::with_seed` -> drops a secret; a seed and a secret are built as one state and given to `from_digester`.
- Python `True` or JavaScript `true` as an instant -> `TypeError`, never `1`.
- JavaScript `number` instants -> safe integers only; a fraction or a wider number is refused.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib txhash::tests
    cargo test --features "parquet iceberg" -p yggdryl --test allocations -- coupled_value_bytes_allocate_nothing reading_an_instant_out_of_a_value_allocates_nothing
    cargo bench -p yggdryl --bench txhash -- txhash_coupling
    cargo bench -p yggdryl --bench txhash -- txhash_value
    cargo bench -p yggdryl --bench txhash -- txhash_instant
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/txhash -k "TestValues or TestHasher"
    python/.venv/bin/python python/benchmarks/txhash.py --min-time 0.2 --repeat 5
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="one-shots|spelling|order|instant spelling|hasher" node/tests/txhash/txhash.test.js
    npm run --prefix node bench:txhash
    ```

## Performance

`rust/benchmarks/txhash.rs`, `python/benchmarks/txhash.py`, and `node/benchmarks/txhash.js` measure the coupling beside the digest it wraps, fixtures built outside every measured loop ([benchmarks](../benchmarks.md)). One containerized x86_64 Linux run (Intel Xeon 2.10 GHz, 4 cores, 16 GiB) produced the numbers: rustc 1.94.1 release, thin LTO, CPython 3.11.15, Node 22.22.2.

### The coupling beside the digest

All three columns hash the same bytes with the same implementation; the difference is laying the instant beside the answer, which is one copy of eight bytes.

| payload | `xxhash::xxh3` | `txhash::txh3` | `txhash::txh128` |
| --- | ---: | ---: | ---: |
| 16 B | 3.55 ns | 3.74 ns | 6.20 ns |
| 240 B | 18.1 ns | 19.1 ns | 28.1 ns |
| 4 KiB | 148.7 ns | 147.2 ns | 162.7 ns |
| 64 KiB | 2.38 µs | 2.33 µs | 2.43 µs |

From 4 KiB up the columns sit inside each other's run-to-run spread; below it the coupling is the eight-byte copy it is. A `TxHasher` clones its configured state per answer, and XXH3 keeps its secret on the heap, which is the algorithm's cost rather than the coupling's:

| case | `TxHasher` | the one-shot beside it |
| --- | ---: | ---: |
| `digest`, 240 B | 72.2 ns | 19.1 ns (`txh3`) |
| `digest_scalar`, four-column row | 157.9 ns | 151.5 ns (`Scalar::txhash`) |

```bash
cargo bench -p yggdryl --bench txhash -- txhash_coupling
```

### The value and the instant

The value's projections are inline work; the two that allocate are the spelling, by contract, and `into_scalar`, whose byte payload is shared storage.

| operation | time |
| --- | ---: |
| `into_bytes` | 2.26 ns |
| `from_bytes` | 18.4 ns |
| `with_unit` | 20.6 ns |
| `into_datetime` | 11.0 ns |
| `into_scalar` | 29.9 ns |
| `to_string` | 204 ns |
| `from_str` | 186 ns |

Reading an instant out of a value costs the unit arithmetic for an integer or a datetime and the timestamp parser for text.

| instant | `unix_from_scalar` |
| --- | ---: |
| integer | 10.1 ns |
| zoned datetime | 15.4 ns |
| timestamp text | 75.9 ns |
| `restate_unix`, nanoseconds to microseconds | 11.1 ns |
| `unix_now` | 29.3 ns |

```bash
cargo bench -p yggdryl --bench txhash -- txhash_value
cargo bench -p yggdryl --bench txhash -- txhash_instant
```

### At the bindings

Every coupled row pairs with the plain digest row for the same bytes, so the difference is reading the instant, laying it beside the digest, and the value object that crosses back. The Python rows ran a release wheel (`--min-time 0.2 --repeat 5`). A `datetime` instant costs the conversion every `Scalar` intake shares, minus the zone lookup a unix count does not need; the batch rows fill 4,096 rows through PyArrow.

```text
xxh3         16 B                                     171.5 ns     0.09 GB/s
txh3         16 B                                     236.3 ns     0.07 GB/s
xxh3        240 B                                     187.3 ns     1.28 GB/s
txh3        240 B                                     258.4 ns     0.93 GB/s
xxh3       4096 B                                     316.3 ns    12.95 GB/s
txh3       4096 B                                     393.6 ns    10.41 GB/s
xxh3      65536 B                                    2386.6 ns    27.46 GB/s
txh3      65536 B                                    2493.5 ns    26.28 GB/s
txh3 240 B (datetime instant)                        1871.9 ns     0.13 GB/s
hasher.digest 240 B                                   353.2 ns     0.68 GB/s
hasher.digest_scalar (four-column row)                373.0 ns     0.00 GB/s
Scalar.digest (four-column row)                       295.8 ns     0.00 GB/s
bytes(value)                                          155.5 ns     0.10 GB/s
TxHash.from_bytes                                     279.5 ns     0.06 GB/s
TxHash(str)                                           348.6 ns     0.05 GB/s
unix_of(datetime)                                    1675.5 ns     0.00 GB/s
xxhash.row_digests                                    757.6 us     5.41 M row/s
txhash.row_txhashes                                   790.6 us     5.18 M row/s
txhash.compose                                         47.5 us    86.17 M row/s
txhash.decompose                                       54.4 us    75.23 M row/s
txhash.unix_array                                       4.4 us   931.07 M row/s
plain holder apply_arrow_batch                       2183.0 us     1.88 M row/s
coupled holder apply_arrow_batch                     2403.8 us     1.70 M row/s
```

The Node rows ran a release addon. A digest crosses back as a `bigint` and a coupled value as a `TxHash` instance, which is the fixed cost every `txh3` row shows beside `xxh3`; a `Date` and a string cross as themselves and are read natively, so an instant that is not a `bigint` costs the parser, not a `Scalar` object. The batch rows fill 4,096 rows through Arrow IPC, both copies included.

```text
xxh3        16 B                                    522.3 ns     0.03 GB/s
txh3        16 B                                   2621.2 ns     0.01 GB/s
xxh3       240 B                                    516.9 ns     0.46 GB/s
txh3       240 B                                   2770.6 ns     0.09 GB/s
xxh3      4096 B                                    646.5 ns     6.34 GB/s
txh3      4096 B                                   3105.4 ns     1.32 GB/s
xxh3     65536 B                                   3188.1 ns    20.56 GB/s
txh3     65536 B                                   5332.3 ns    12.29 GB/s
txh3 240 B (Date instant)                          4967.5 ns     0.05 GB/s
hasher.digest 240 B                                3727.8 ns     0.06 GB/s
hasher.digestScalar (four-column row)              3105.0 ns     0.00 GB/s
Scalar.digest (four-column row)                    2867.0 ns     0.00 GB/s
value.bytes()                                      2519.6 ns     0.01 GB/s
TxHash.fromBytes                                   3305.5 ns     0.00 GB/s
TxHash.from(string)                                3172.8 ns     0.01 GB/s
txhash.unixOf(Date)                                1781.3 ns     0.00 GB/s
plain holder applyArrowBatch                       2625.7 us     1.56 M row/s
coupled holder applyArrowBatch                     2821.7 us     1.45 M row/s
```

```bash
python/.venv/bin/python python/benchmarks/txhash.py --min-time 0.2 --repeat 5
npm run --prefix node bench:txhash
```
