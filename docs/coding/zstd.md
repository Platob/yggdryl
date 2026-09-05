# zstd

RFC 8878 Zstandard as whole buffers, Rust streams, and a transparent `Zstd<H>` handle.

## Contract

| | |
| --- | --- |
| Owns | `load`/`dump`, `dump_with_level`, `reader`/`writer`, `writer_with_level`, `Zstd<H>` |
| Bindings | `loads`/`dumps` only; streams and `Zstd<H>` are Rust only |
| Wire format | RFC 8878; `dump` writes one frame, `load` reads exactly one |
| Engine | `zstd` crate |
| Level | shared 0 to 9, clamped, default 6; maps to zstd 1 to 19 |
| Errors | non-frame input -> `Err`, `ValueError` in Python, a throw in JavaScript |
| Handle | `Zstd<H>` is an [`IOBase`](../holder/index.md) over an `IOBase`: reads decompress, writes compress, `size` is the decoded length, nothing seeks |
| Commit | `flush`, `close`, or `into_handle`; never `pwrite` |
| Streamed reads | [`pstream_bytes`](../holder/iobase/bytes.md), bounded arrays, handle never opened |
| Selection | `Codec::Zstd` names it; [`coding::Coded`](index.md) applies it at runtime |

## Use

`load` and `dump` hold both sides in memory; the bindings spell them `loads`/`dumps`.

=== "Rust"

    ```rust
    use yggdryl::coding::zstd;

    let frame = zstd::dump(b"symbol,price\nAAPL,1\n")?;
    assert_eq!(zstd::load(&frame)?, b"symbol,price\nAAPL,1\n");
    ```

=== "Python"

    ```python
    from yggdryl.coding import zstd

    frame = zstd.dumps(b"symbol,price\nAAPL,1\n")
    assert zstd.loads(frame) == b"symbol,price\nAAPL,1\n"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { zstd } = require('yggdryl')

    const frame = zstd.dumps(Buffer.from('symbol,price\nAAPL,1\n'))
    assert.deepEqual(zstd.loads(frame), Buffer.from('symbol,price\nAAPL,1\n'))
    ```

## Frames

Gain starts where the payload repeats; below that, framing grows the output. `load` rejects anything but one frame, as each binding's own error type.

=== "Rust"

    ```rust
    use yggdryl::coding::zstd;

    // Repetition is what zstd removes.
    let payload = "AAPL,1\n".repeat(64);
    let frame = zstd::dump(payload.as_bytes())?;
    assert!(frame.len() < payload.len());

    // Framing costs bytes, so a short payload comes out larger than it went in.
    assert!(zstd::dump(b"AAPL,1\n")?.len() > 7);

    // A payload that is not a frame is reported, not silently returned.
    assert!(zstd::load(b"definitely not a compressed payload").is_err());
    ```

=== "Python"

    ```python
    import pytest

    from yggdryl.coding import zstd

    # Repetition is what zstd removes.
    payload = b"AAPL,1\n" * 64
    frame = zstd.dumps(payload)
    assert len(frame) < len(payload)

    # Framing costs bytes, so a short payload comes out larger than it went in.
    assert len(zstd.dumps(b"AAPL,1\n")) > 7

    # A payload that is not a frame is reported, not silently returned.
    with pytest.raises(ValueError):
        zstd.loads(b"definitely not a compressed payload")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { zstd } = require('yggdryl')

    // Repetition is what zstd removes.
    const payload = Buffer.from('AAPL,1\n'.repeat(64))
    const frame = zstd.dumps(payload)
    assert.ok(frame.length < payload.length)

    // Framing costs bytes, so a short payload comes out larger than it went in.
    assert.ok(zstd.dumps(Buffer.from('AAPL,1\n')).length > 7)

    // A payload that is not a frame is reported, not silently returned.
    assert.throws(() => zstd.loads(Buffer.from('definitely not a compressed payload')))
    ```

## Streams

Rust only. The `Encoder` must be finished; `reader` returns `Box<dyn Read>`, never a `Result`.

```rust
use std::io::{Read, Write};
use yggdryl::coding::zstd;

let payload = "AAPL,1\n".repeat(64);

let mut encoded = Vec::new();
let mut encoder = zstd::writer(&mut encoded);
encoder.write_all(payload.as_bytes())?;
encoder.finish()?;

let mut decoded = Vec::new();
zstd::reader(encoded.as_slice()).read_to_end(&mut decoded)?;
assert_eq!(decoded, payload.as_bytes());
```

## Levels

Rust only. The shared scale rounds `level * 19 / 9` up onto zstd 1 to 19.

```rust
use yggdryl::{Level};
use yggdryl::coding::zstd;

let payload = "AAPL,1\n".repeat(64);

for level in [Level::NONE, Level::FAST, Level::DEFAULT, Level::BEST] {
    let frame = zstd::dump_with_level(payload.as_bytes(), level)?;
    assert_eq!(zstd::load(&frame)?, payload.as_bytes(), "{level}");
}

// The scale is 0 to 9, and anything above it clamps.
assert_eq!(Level::DEFAULT.get(), 6);
assert_eq!(Level::new(12), Level::BEST);
```

The streaming form takes a level the same way.

```rust
use std::io::Write;
use yggdryl::{Level};
use yggdryl::coding::zstd;

let payload = "AAPL,1\n".repeat(64);

let mut encoded = Vec::new();
let mut encoder = zstd::writer_with_level(&mut encoded, Level::BEST);
encoder.write_all(payload.as_bytes())?;
encoder.finish()?;

assert_eq!(zstd::load(&encoded)?, payload.as_bytes());
```

| `Level` | zstd |
| --- | ---: |
| 0 (`NONE`) | 1 |
| 1 (`FAST`) | 3 |
| 2 | 5 |
| 3 | 7 |
| 4 | 9 |
| 5 | 11 |
| 6 (`DEFAULT`) | 13 |
| 7 | 15 |
| 8 | 17 |
| 9 (`BEST`) | 19 |

`Level::NONE` maps to zstd 1, its cheapest real level; [gzip](gzip.md) and [zlib](zlib.md) store uncompressed instead.

## The transparent handle

Rust only. Anything that takes a handle sees decoded bytes; the wrapped handle keeps the frame.

```rust
use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::coding::zstd::{self, Zstd};

let mut handle = Zstd::new(Buffer::new());
handle.write_all_bytes(b"symbol,price\nAAPL,1\n")?;
handle.flush()?;

// The wrapper reads plain bytes and reports the decoded size.
assert_eq!(handle.read_all_bytes()?, b"symbol,price\nAAPL,1\n");
assert_eq!(handle.size(), 20);

// The wrapped handle holds the frame.
assert_eq!(
    zstd::load(&handle.handle().read_all_bytes()?)?,
    b"symbol,price\nAAPL,1\n"
);
```

Sequential reads decode from the wrapped handle into bounded arrays; the [stream benchmark](../holder/iobase/bytes.md) records their cost beside gzip and zlib.

```rust
use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::coding::zstd::Zstd;

// Wrapping touches nothing, and a handle with no bytes decodes to nothing.
let handle = Zstd::new(Buffer::new());
assert!(handle.read_all_bytes()?.is_empty());
assert_eq!(handle.size(), 0);
```

Constructing touches nothing, reading something absent yields nothing, writing creates.

```rust
use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::coding::zstd::{self, Zstd};
use yggdryl::Level;

let payload = "AAPL,1\n".repeat(64);

let mut handle = Zstd::new(Buffer::new()).with_level(Level::BEST);
assert_eq!(handle.level(), Level::BEST);
handle.write_all_bytes(payload.as_bytes())?;

// Consuming the handle publishes the pending write first.
let inner = handle.into_handle()?;
assert_eq!(zstd::load(&inner.read_all_bytes()?)?, payload.as_bytes());
```

`with_level` changes writes only; `into_handle` publishes the pending write, then returns the wrapped handle.

## Edges

- `zstd::load` on a non-frame payload -> `Err`; `ValueError` in Python; a throw in JavaScript.
- `zstd::dump` on a short payload -> longer than its input; framing costs bytes.
- `zstd::writer` dropped before `finish` -> no epilogue, not a valid frame.
- `zstd::reader` when the window cannot be allocated -> still a value; the error surfaces on the first `read`.
- `Level::NONE` -> zstd 1; zstd has no stored form.
- `Zstd::new` over an empty handle -> size 0, empty read, no error.
- `pwrite` on `Zstd<H>` -> pending until `flush`, `close`, or `into_handle`.
- `pstream_bytes` past offset zero -> the prefix is decoded and discarded; a surrounding [`Buffered`](../holder/backends/buffered.md) stays empty.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib coding::zstd::
    cargo bench -p yggdryl --bench coding -- zstd
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/coding/test_codings.py -k zstd
    python/.venv/bin/python python/benchmarks/coding.py --min-time 0.2 --repeat 5
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="byte codings" node/tests/text/codec.test.js
    npm run --prefix node bench:coding
    ```

## Performance

One containerized x86_64 Linux run of the Python binding (CPython 3.11) over 1,080,000 bytes of JSON lines.

```text
zstd encode (yggdryl)     14.879 ms     69.2 MiB/s
zstd decode (yggdryl)      0.358 ms   2877.3 MiB/s
```

Standard-library rows need `compression.zstd` on Python 3.14+; on 3.11 the script prints `stdlib compression.zstd unavailable on this interpreter; skipped`. [gzip](gzip.md) and [zlib](zlib.md) carry their rows from the same run.

```bash
python/.venv/bin/python python/benchmarks/coding.py --min-time 0.2 --repeat 5
```
