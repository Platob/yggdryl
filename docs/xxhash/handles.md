# Handles

Digest an `IOBase` handle's bytes, and hash a stream while it moves.

## Contract

| key | value |
| --- | --- |
| Owns | `read_digest`, `read_range_digest`, `xxhash::reader`/`writer`, `Hashed<H>` |
| Derived | every backend and every wrapper inherits both handle methods unchanged |
| Memory | streams through [`pstream_bytes`](../holder/iobase/bytes.md), retains one bounded chunk, never calls `read_all_bytes` |
| Missing resource | digests as no bytes, per the laziness contract |
| Container | typed `Error::NotAtomic` naming the kind; folder and recursive digests are absent |
| Wrapper vs backing | a wrapper answers for the bytes it presents, `handle()` for the bytes it holds |
| `Hashed<H>` | answers from the running state; the bytes are never read back |
| Running state | covers writes strictly sequential from offset 0, counted after `flush` |
| Stale state | a positional write makes the next digest re-stream and re-arm, with an identical answer |
| Bindings | both handle methods everywhere; `DigestReader`, `DigestWriter`, `Hashed<H>` are Rust only |

## Use

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::{DigestAlgorithm, xxhash};

    let mut handle = Buffer::new();
    handle.write_all_bytes(b"symbol,price\nAAPL,187.23\n")?;

    assert_eq!(
        handle.read_digest(DigestAlgorithm::Xxh3)?,
        DigestAlgorithm::Xxh3.digest(&handle.read_all_bytes()?),
    );
    assert_eq!(
        handle.read_range_digest(0, 6, DigestAlgorithm::Xxh3)?,
        DigestAlgorithm::Xxh3.digest(b"symbol"),
    );

    // A resource that does not exist digests as no bytes, per the laziness
    // contract - absence is emptiness, not a third answer to branch on.
    assert_eq!(
        Buffer::new().read_digest(DigestAlgorithm::Xxh3)?.as_u64(),
        Some(xxhash::xxh3(b"")),
    );
    ```

=== "Python"

    ```python
    import tempfile
    from pathlib import Path

    from yggdryl import IOBase, xxhash

    with tempfile.TemporaryDirectory() as root:
        path = Path(root) / "trades.csv"
        path.write_bytes(b"symbol,price\nAAPL,187.23\n")

        handle = IOBase(path)
        assert handle.read_digest("xxh3-64") == xxhash.digest(handle.read_bytes(), "xxh3-64")
        assert handle.read_range_digest(0, 6) == xxhash.digest(b"symbol", "xxh3-64")

        missing = IOBase(Path(root) / "never-written.csv")
        assert missing.read_digest() == xxhash.digest(b"", "xxh3-64")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase, xxhash } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-digest-'))
    try {
      const file = path.join(root, 'trades.csv')
      const payload = Buffer.from('symbol,price\nAAPL,187.23\n')
      fs.writeFileSync(file, payload)

      const handle = new IOBase(file)
      assert.ok(handle.readDigest('xxh3-64').equals(xxhash.digest(payload, 'xxh3-64')))
      assert.ok(handle.readRangeDigest(0, 6).equals(xxhash.digest(Buffer.from('symbol'), 'xxh3-64')))

      const missing = new IOBase(path.join(root, 'never-written.csv'))
      assert.ok(missing.readDigest().equals(xxhash.digest(Buffer.alloc(0), 'xxh3-64')))
    } finally {
      fs.rmSync(root, { recursive: true, force: true })
    }
    ```

## Wrapped handles

A [coding](../coding/index.md) wrapper and its backing handle answer different questions.

```rust
use yggdryl::coding::gzip::Gzip;
use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::{DigestAlgorithm, xxhash};

let plain = b"symbol,price\nAAPL,187.23\n";
let mut handle = Gzip::new(Buffer::new());
handle.write_all_bytes(plain)?;
handle.flush()?;

// The wrapper answers for the bytes it presents.
assert_eq!(
    handle.read_digest(DigestAlgorithm::Xxh3)?.as_u64(),
    Some(xxhash::xxh3(plain)),
);
// The handle underneath answers for the bytes it holds.
let compressed = handle.handle().read_all_bytes()?;
assert_eq!(
    handle.handle().read_digest(DigestAlgorithm::Xxh3)?.as_u64(),
    Some(xxhash::xxh3(&compressed)),
);
assert_ne!(xxhash::xxh3(plain), xxhash::xxh3(&compressed));
```

## Reading and writing through a digest

Rust only: `DigestReader` and `DigestWriter` build on `Read` and `Write`.

```rust
use std::io::{Read, Write};

use yggdryl::{DigestAlgorithm, xxhash};

// Hash a payload that is already being moved, in the pass that was already
// happening, rather than reading it a second time.
let mut source = xxhash::reader(b"AAPL,187.23".as_slice(), DigestAlgorithm::Xxh3);
let mut moved = Vec::new();
source.read_to_end(&mut moved)?;
assert_eq!(moved, b"AAPL,187.23");
assert_eq!(source.as_digest(), DigestAlgorithm::Xxh3.digest(&moved));

let mut target = xxhash::writer(Vec::new(), DigestAlgorithm::Xxh64);
target.write_all(b"AAPL,187.23")?;
assert_eq!(target.as_digest(), DigestAlgorithm::Xxh64.digest(b"AAPL,187.23"));
assert_eq!(target.into_inner(), b"AAPL,187.23");
```

## Hashed handles

Rust only: `Hashed<H>` is generic over the handle it wraps.

```rust
use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::xxhash::Hashed;
use yggdryl::DigestAlgorithm;

let mut handle = Hashed::new(Buffer::new(), DigestAlgorithm::Xxh3);
handle.write_all_bytes(b"symbol,price\n")?;
handle.append_bytes(b"AAPL,187.23\n")?;
handle.flush()?;

// Answered from the running state; the bytes are never read back.
assert_eq!(
    handle.read_digest(DigestAlgorithm::Xxh3)?,
    DigestAlgorithm::Xxh3.digest(b"symbol,price\nAAPL,187.23\n"),
);

// A positional write the running state cannot follow makes it stale, which is
// not an error and not silent corruption: the next digest re-streams the
// handle and re-arms, and the answer is identical either way.
handle.pwrite_all(7, b"PRICE")?;
assert_eq!(
    handle.read_digest(DigestAlgorithm::Xxh3)?,
    DigestAlgorithm::Xxh3.digest(&handle.read_all_bytes()?),
);
```

## Edges

- Missing resource -> the digest of no bytes, never an error.
- Container handle -> `Error::NotAtomic`; folder and recursive digests do not exist.
- Positional write into `Hashed<H>` -> stale state, and the next digest re-streams.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib -- xxhash::tests::handles xxhash::tests::hashed
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/xxhash -k TestHandles
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="handle digests" node/tests/xxhash/xxhash.test.js
    ```

## Performance

One containerized x86_64 Linux run ([benchmarks](../benchmarks.md)): Intel Xeon @ 2.10 GHz, 4 cores, 16 GiB; rustc 1.94.1 release with thin LTO.

| case | time | throughput |
| --- | --- | --- |
| one-shot, 1 MiB | 0.040 ms | 26.00 GB/s |
| streamed in 64 KiB windows, 1 MiB | 0.040 ms | 26.42 GB/s |
| one-shot, 64 MiB | 9.153 ms | 7.33 GB/s |
| streamed in 64 KiB windows, 64 MiB | 8.938 ms | 7.51 GB/s |

Streaming in `pstream_bytes` windows costs nothing against hashing the payload whole.

| 64 MiB local file | time | peak resident after |
| --- | --- | --- |
| `read_digest` | 8.85 ms | 77.9 MiB (unchanged) |
| `read_all_bytes` then digest | 72.98 ms | 140.5 MiB |

The memory column is why `read_digest` exists: the streamed read left the high-water mark unchanged.

| case | time |
| --- | --- |
| `Hashed<H>` write-through, 4 MiB | 0.762 ms |
| plain write then a digest pass, 4 MiB | 0.948 ms |
| `std::io::copy`, 4 MiB | 0.322 ms |
| through `DigestReader` | 0.685 ms |
| through `DigestWriter` | 0.540 ms |

`Hashed<H>` saves the second pass, the 0.19 ms. The wrappers hash on a copy already happening.

```bash
cargo bench -p yggdryl --bench xxhash -- xxhash_streaming
cargo bench -p yggdryl --bench xxhash -- xxhash_handle
cargo bench -p yggdryl --bench xxhash -- xxhash_hashed
cargo bench -p yggdryl --bench xxhash -- xxhash_stream_wrappers
```
