# xxHash

`yggdryl::xxhash` digests bytes, values, handles, and Arrow rows with XXH32, XXH64, XXH3-64, and XXH3-128.

## Pages

| Page | Purpose |
| --- | --- |
| [Streaming](streaming.md) | Resumable states, `clear()`, `Hasher`, and the XXH3 seed-and-secret rules. |
| [Values](values.md) | The canonical `Scalar` byte feed, `stable_hash`, digest holders, and Arrow row and column digests. |
| [Handles](handles.md) | Digesting a handle, `DigestReader` / `DigestWriter`, and the `Hashed<H>` write-through wrapper. |

## Contract

| | |
| --- | --- |
| Owns | `xxh32`, `xxh64`, `xxh3`, `xxh128` and their `_with_seed` forms; `Digest`; `DigestAlgorithm`. |
| Algorithms | `DigestAlgorithm::ALL`: `xxh32`, `xxh64`, `xxh3-64`, `xxh3-128`; `width()` 4, 8, 8, 16 bytes. |
| Arguments | Input first, seed second, everywhere. |
| Default | XXH3-64; what `stable_hash` answers ([Values](values.md)). |
| `Digest` | The algorithm carried with the number; `DigestAlgorithm` dispatches at runtime, as [`Codec`](../coding/index.md) does for [gzip](../coding/gzip.md). |
| Spelling | `<algorithm>:<hex>`; `from_str` is the exact inverse; two algorithms are never equal. |
| Bytes | `into_bytes` is the canonical big-endian form, the reference's `XXH*_canonicalFromHash`. |
| Secret | Custom secret: the XXH3 pair only (`is_secretable`); a seed: every algorithm. |
| Not | A cryptographic hash or adversarial integrity check; not Iceberg `bucket[N]`, which is murmur3 x86_32 ([Iceberg](../media/iceberg/index.md) never calls this module). |
| Bindings | Python: `bytes`, `bytearray`, `memoryview`, any buffer, `str` as UTF-8. JavaScript: `Buffer`, `Uint8Array`, `ArrayBuffer`, string as UTF-8. |

## Use

The four one-shot functions answer their native widths with nothing wrapped around the number.

=== "Rust"

    ```rust
    use yggdryl::xxhash;

    assert_eq!(xxhash::xxh32(b"abc"), 0x32d1_53ff);
    assert_eq!(xxhash::xxh64(b"abc"), 0x44bc_2cf5_ad77_0999);
    assert_eq!(xxhash::xxh3(b"abc"), 0x78af_5f94_892f_3950);
    assert_eq!(
        xxhash::xxh128(b"abc"),
        0x06b0_5ab6_733a_6185_78af_5f94_892f_3950,
    );

    // The input comes first and the seed second, here and everywhere.
    assert_ne!(xxhash::xxh64_with_seed(b"abc", 42), xxhash::xxh64(b"abc"));
    ```

=== "Python"

    ```python
    from yggdryl import xxhash

    assert xxhash.xxh32(b"abc") == 0x32D153FF
    assert xxhash.xxh64(b"abc") == 0x44BC2CF5AD770999
    assert xxhash.xxh3(b"abc") == 0x78AF5F94892F3950
    assert xxhash.xxh128(b"abc") == 0x06B05AB6733A618578AF5F94892F3950

    # bytes, bytearray, memoryview, any buffer, or a str as its UTF-8.
    assert xxhash.xxh3("abc") == xxhash.xxh3(bytearray(b"abc"))
    assert xxhash.xxh64(b"abc", seed=42) != xxhash.xxh64(b"abc")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { xxhash } = require('yggdryl')

    const payload = Buffer.from('abc')
    // XXH32 answers a number - 32 bits always fit one exactly - and the wider
    // algorithms answer bigints.
    assert.equal(xxhash.xxh32(payload), 0x32d153ff)
    assert.equal(xxhash.xxh64(payload), 0x44bc2cf5ad770999n)
    assert.equal(xxhash.xxh3(payload), 0x78af5f94892f3950n)
    assert.equal(xxhash.xxh128(payload), 0x06b05ab6733a618578af5f94892f3950n)

    // Buffer, Uint8Array, ArrayBuffer, or a string as its UTF-8.
    assert.equal(xxhash.xxh3('abc'), xxhash.xxh3(new Uint8Array(payload)))
    assert.notEqual(xxhash.xxh64(payload, { seed: 42n }), xxhash.xxh64(payload))
    ```

## Digest values

=== "Rust"

    ```rust
    use yggdryl::{Digest, DigestAlgorithm};

    let digest = DigestAlgorithm::Xxh3.digest(b"abc");
    assert_eq!(digest.as_u64(), Some(0x78af_5f94_892f_3950));
    assert_eq!(digest.to_string(), "xxh3-64:78af5f94892f3950");
    assert_eq!(Digest::from_str(&digest.to_string())?, digest);
    assert_eq!(digest.into_bytes().len(), 8);

    // Two algorithms are never equal, whatever their payloads: `xxh64` and
    // `xxh3-64` are both 64 bits wide and answer different values.
    assert_ne!(
        Digest::new(DigestAlgorithm::Xxh64, 7),
        Digest::new(DigestAlgorithm::Xxh3, 7),
    );
    ```

=== "Python"

    ```python
    from yggdryl import xxhash

    digest = xxhash.digest(b"abc", "xxh3-64")
    assert int(digest) == 0x78AF5F94892F3950
    assert str(digest) == "xxh3-64:78af5f94892f3950"
    assert xxhash.Digest(str(digest)) == digest
    assert len(bytes(digest)) == digest.width == 8

    assert xxhash.Digest.from_int("xxh64", 7) != xxhash.Digest.from_int("xxh3-64", 7)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { xxhash } = require('yggdryl')

    const digest = xxhash.digest(Buffer.from('abc'), 'xxh3-64')
    assert.equal(digest.value(), 0x78af5f94892f3950n)
    assert.equal(digest.toString(), 'xxh3-64:78af5f94892f3950')
    assert.ok(xxhash.Digest.from(digest.toString()).equals(digest))
    assert.equal(digest.bytes().length, digest.width)

    const wide = xxhash.digest(Buffer.from('abc'), 'xxh3-128')
    assert.ok(!wide.equals(digest))
    ```

### DigestAlgorithm

`Digester` is the runtime-selected state, what `Encoder` is to `Codec` ([Streaming](streaming.md)).

Rust only.

```rust
use yggdryl::{Digest, DigestAlgorithm};

let digest = DigestAlgorithm::Xxh3.digest(b"AAPL");
assert_eq!(digest.algorithm(), DigestAlgorithm::Xxh3);
assert_eq!(Digest::from_str(&digest.to_string())?, digest);
assert_eq!(digest.into_bytes().len(), DigestAlgorithm::Xxh3.width());

// A caller who knows the algorithm at compile time uses the concrete state in
// `yggdryl::xxhash` and pays no dispatch; this is the form for one held in a
// variable.
let mut digester = DigestAlgorithm::Xxh3.digester();
digester.write_bytes(b"AA");
digester.write_bytes(b"PL");
assert_eq!(digester.as_digest(), digest);

// Only the XXH3 pair takes a custom secret; every algorithm takes a seed.
assert!(!DigestAlgorithm::Xxh64.is_secretable());
assert!(DigestAlgorithm::Xxh3.is_secretable());
assert_eq!(
    DigestAlgorithm::ALL.map(DigestAlgorithm::as_str),
    ["xxh32", "xxh64", "xxh3-64", "xxh3-128"],
);
```

## Edges

- Same payload, two algorithms -> never equal (`xxh64` vs `xxh3-64`).
- JavaScript `xxh32` -> a number; the wider three -> bigints; seed as `{ seed: 42n }`.
- Python `bytearray` / `memoryview` -> a bounded window, never borrowed; 1.7x slower than `bytes`.
- JavaScript string -> UTF-8 encoded on the way in; 7.5x slower than `Buffer`.
- One-byte call -> 166 ns Python, 496 ns Node of binding overhead; gone by 64 KiB.
- `python/benchmarks/xxhash.py` without `-P` -> `python/benchmarks/types.py` shadows stdlib `types`; `import argparse` crashes.
- No `xxhash` C package in `python/.venv` -> `(C libxxhash)` rows skipped; `python/tests/xxhash` skipped by `pytest.importorskip`.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib xxhash::
    cargo test --features "parquet iceberg" -p yggdryl --lib -- xxhash::tests::published_vectors xxhash::tests::xxh128_carries xxhash::tests::the_module_digest_helper
    cargo bench -p yggdryl --bench xxhash -- xxhash_size
    cargo bench -p yggdryl --bench xxhash -- xxhash_wrapper
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/xxhash -k "TestVectors or TestOutsideImplementation or TestContent or TestDigest"
    python/.venv/bin/python -P python/benchmarks/xxhash.py --min-time 0.2 --repeat 5
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="published vectors|32-bit answer|byte shape|digest carries|two algorithms|unknown algorithm" node/tests/xxhash/xxhash.test.js
    npm run --prefix node bench:xxhash
    ```

## Performance

`rust/benchmarks/xxhash.rs`, `python/benchmarks/xxhash.py`, and `node/benchmarks/xxhash.js` measure one protocol from three sides, fixtures built outside every measured loop ([benchmarks](../benchmarks.md)). One containerized x86_64 Linux run (Intel Xeon 2.10 GHz, 4 cores, 16 GiB) produced the numbers: rustc 1.94.1 release, thin LTO, CPython 3.11.15, Node 22.22.2.

The Arrow groups report rows per second for missing or default holders, preserved populated holders, and forced recomputation. The JavaScript rows include the IPC copy that binding requires.

### Throughput per algorithm and size

Bytes per second, higher is better; the 64 MiB row is memory-bound rather than hash-bound. XXH3 is roughly four times XXH32 and twice XXH64 once a payload is worth vectorizing.

| payload | xxh32 | xxh64 | xxh3-64 | xxh3-128 |
| --- | --- | --- | --- | --- |
| 1 B | 0.25 GB/s | 0.16 GB/s | 0.21 GB/s | 0.18 GB/s |
| 4 B | 0.70 GB/s | 0.73 GB/s | 0.88 GB/s | 0.78 GB/s |
| 16 B | 2.70 GB/s | 2.81 GB/s | 3.96 GB/s | 2.58 GB/s |
| 64 B | 5.12 GB/s | 4.87 GB/s | 9.39 GB/s | 5.28 GB/s |
| 128 B | 5.86 GB/s | 6.52 GB/s | 11.61 GB/s | 7.41 GB/s |
| 240 B | 6.15 GB/s | 7.86 GB/s | 13.57 GB/s | 8.74 GB/s |
| 1 KiB | 6.48 GB/s | 11.57 GB/s | 20.52 GB/s | 16.44 GB/s |
| 64 KiB | 6.55 GB/s | 12.99 GB/s | 28.13 GB/s | 27.55 GB/s |
| 1 MiB | 6.43 GB/s | 12.74 GB/s | 25.84 GB/s | 25.88 GB/s |
| 64 MiB | 4.58 GB/s | 5.97 GB/s | 6.54 GB/s | 6.64 GB/s |

```bash
cargo bench -p yggdryl --bench xxhash -- xxhash_size
```

### What this module costs over the protocol it wraps

All three columns hash the same bytes with the same implementation; the difference is argument normalization at this module's boundary.

| payload | `xxhash::xxh3` | direct `twox-hash` call | `DigestAlgorithm::digest` |
| --- | --- | --- | --- |
| 1 B | 4.9 ns | 4.1 ns | 4.9 ns |
| 64 B | 7.4 ns | 8.1 ns | 7.3 ns |
| 240 B | 19.0 ns | 22.5 ns | 19.3 ns |
| 4 KiB | 141.9 ns | 124.1 ns | 145.6 ns |
| 1 MiB | 37.8 µs | 37.0 µs | 39.1 µs |

From 4 KiB up the columns sit inside each other's run-to-run spread. Carrying the algorithm in a `Digest` costs nothing measurable.

```bash
cargo bench -p yggdryl --bench xxhash -- xxhash_wrapper
```

### At the bindings

The Python rows ran a release wheel (`--min-time 0.1 --repeat 5`) against the C `libxxhash` binding on one 1,080,000-byte payload.

```text
xxh3 payload                     44773.2 ns    24.12 GB/s
xxh3 payload (C libxxhash)       66506.4 ns    16.24 GB/s
xxh64 payload                       87825.4 ns    12.30 GB/s
xxh64 payload (C libxxhash)         85947.4 ns    12.57 GB/s
xxh3 payload (bytearray)         75534.4 ns    14.30 GB/s
xxh3 payload (memoryview)        75835.4 ns    14.24 GB/s
xxh3 payload (str)               44825.5 ns    24.09 GB/s
xxh3        1 B                    165.8 ns     0.01 GB/s
xxh3        1 B (C libxxhash)       78.0 ns     0.01 GB/s
```

```bash
python/.venv/bin/python -P python/benchmarks/xxhash.py --min-time 0.2 --repeat 5
```

The Node rows ran a release addon on the same payload; `streamed 64 KiB` and `scalar leaf digest` belong to [Streaming](streaming.md) and [Values](values.md).

```text
xxh3         1 B                                 495.9 ns     0.00 GB/s
xxh3      1024 B                                 505.0 ns     2.03 GB/s
xxh3     65536 B                                2756.8 ns    23.77 GB/s
xxh32 payload                                    170537.4 ns     6.33 GB/s
xxh64 payload                                     87036.2 ns    12.41 GB/s
xxh3 payload                                   48139.1 ns    22.43 GB/s
xxh128 payload                                  51937.5 ns    20.79 GB/s
xxh3 payload (Uint8Array)                      50711.2 ns    21.30 GB/s
xxh3 payload (string)                         364511.2 ns     2.96 GB/s
xxh3 payload (streamed 64 KiB)                 71816.4 ns    15.04 GB/s
scalar leaf digest                                 2399.2 ns     0.00 GB/s
```

```bash
npm run --prefix node bench:xxhash
```
