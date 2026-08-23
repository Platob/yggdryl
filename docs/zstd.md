# Zstandard

Encode and decode RFC 8878 Zstandard frames as whole buffers, as streams, or through a handle that compresses transparently.

!!! note "Streams and handles are Rust only"
    The whole-buffer pair crosses to Python and JavaScript as `loads`/`dumps`. The streaming
    `reader`/`writer` and the transparent `Zstd<H>` handle are Rust only: both are built on
    `Read`/`Write`, which neither binding has a native spelling for.

=== "Rust"

    ```rust
    use yggdryl::zstd;

    let frame = zstd::dump(b"symbol,price\nAAPL,1\n")?;
    assert_eq!(zstd::load(&frame)?, b"symbol,price\nAAPL,1\n");
    ```

=== "Python"

    ```python
    from yggdryl import zstd

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

`dump` produces one complete frame and `load` consumes one. Both hold the whole input and the whole output in memory; the streaming forms below hold one window instead, whatever the payload size.

=== "Rust"

    ```rust
    use yggdryl::zstd;

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

    from yggdryl import zstd

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

The gain begins where the payload has repetition to remove; below that, framing dominates and the output grows. `load` decodes exactly one frame and rejects anything else, so a mis-routed payload fails at the codec rather than downstream - and it fails the same way in all three languages, as the binding's own error type.

## Streams

```rust
use std::io::{Read, Write};
use yggdryl::zstd;

let payload = "AAPL,1\n".repeat(64);

let mut encoded = Vec::new();
let mut encoder = zstd::writer(&mut encoded);
encoder.write_all(payload.as_bytes())?;
encoder.finish()?;

let mut decoded = Vec::new();
zstd::reader(encoded.as_slice()).read_to_end(&mut decoded)?;
assert_eq!(decoded, payload.as_bytes());
```

`writer` returns an [`Encoder`](enums.md), which **must** be finished. Dropping it without calling `finish` omits the frame epilogue, and what was written is then not a valid Zstandard frame.

`reader` returns `Box<dyn Read>` rather than a `Result`. The decoder can only fail to construct when it cannot allocate its window; that failure is held and surfaced on the first `read` instead of panicking, so the reader is always usable as a value.

## Levels

```rust
use yggdryl::{Level, zstd};

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
use yggdryl::{Level, zstd};

let payload = "AAPL,1\n".repeat(64);

let mut encoded = Vec::new();
let mut encoder = zstd::writer_with_level(&mut encoded, Level::BEST);
encoder.write_all(payload.as_bytes())?;
encoder.finish()?;

assert_eq!(zstd::load(&encoded)?, payload.as_bytes());
```

[`Level`](enums.md) is one 0-to-9 scale shared by every codec, so raising compression does not mean learning three numbering schemes. Zstandard's own range is 1 to 19, and the shared scale maps onto it by rounding `level * 19 / 9` up:

| `Level` | zstd |
| --- | --- |
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

`Level::NONE` is the one entry that does not mean what its name suggests everywhere else: zstd has no store-uncompressed setting, so level 0 maps to zstd 1, its cheapest real level. Under [gzip](gzip.md) and [zlib](zlib.md) the same `Level::NONE` does store uncompressed. Every level round-trips; the only difference is time spent and bytes produced.

## The transparent handle

```rust
use yggdryl::io::{Buffer, IOBase};
use yggdryl::zstd::{self, Zstd};

let mut handle = Zstd::new(Buffer::new());
handle.write_all_bytes(b"symbol,price\nAAPL,1\n")?;
handle.flush()?;

// The wrapper reads plain bytes and reports the decoded size.
assert_eq!(handle.read_all_bytes()?, b"symbol,price\nAAPL,1\n");
assert_eq!(handle.size(), 20);

// The wrapped handle holds the frame.
assert_eq!(
    zstd::load(handle.handle().as_slice())?,
    b"symbol,price\nAAPL,1\n"
);
```

`Zstd<H>` is an [`IOBase`](io.md) over another `IOBase`. Reads decompress and writes compress, so anything that takes a handle - a media reader, another coding - sees the decoded bytes while the wrapped handle keeps the Zstandard form. `size` follows the same rule: it is the decoded length, not the frame length.

A coding is not seekable, so writes and opened sessions materialize the decoded
value and publish pending changes on `flush` or `close`, not on every `pwrite`.

Sequential reads use [`IOBase::pstream_bytes`](io.md#streamed-bytes): Zstandard
is decoded directly from the wrapped handle into bounded arrays, without
opening the handle or retaining earlier decoded pages. A non-zero start decodes
and discards the prefix because a frame has no decoded seek. A surrounding
`Buffered` cache stays empty on this path. The
[stream benchmark](io.md#measured-streamed-byte-behavior) records first-chunk,
full-drain, and whole-value costs beside gzip and zlib.

```rust
use yggdryl::io::{Buffer, IOBase};
use yggdryl::zstd::Zstd;

// Wrapping touches nothing, and a handle with no bytes decodes to nothing.
let handle = Zstd::new(Buffer::new());
assert!(handle.read_all_bytes()?.is_empty());
assert_eq!(handle.size(), 0);
```

An empty wrapped handle decodes to an empty value rather than failing on an absent frame, which is the same laziness contract every handle follows: constructing touches nothing, reading something absent yields nothing, writing creates.

```rust
use yggdryl::io::{Buffer, IOBase};
use yggdryl::zstd::{self, Zstd};
use yggdryl::Level;

let payload = "AAPL,1\n".repeat(64);

let mut handle = Zstd::new(Buffer::new()).with_level(Level::BEST);
assert_eq!(handle.level(), Level::BEST);
handle.write_all_bytes(payload.as_bytes())?;

// Consuming the handle publishes the pending write first.
let inner = handle.into_handle()?;
assert_eq!(zstd::load(inner.as_slice())?, payload.as_bytes());
```

`with_level` sets the level writes encode at; it changes nothing about reads, since a frame carries what a decoder needs. `into_handle` returns the wrapped handle with any pending write already published, which is the way to hand the compressed bytes to something else without going through `flush` first.

When the coding is chosen at runtime rather than written into the type, [`Codec::Zstd`](enums.md) names it for whole-buffer and stream operations, and [`generic::Codec`](generic.md) wraps a handle in a coding decided at run time.

## Against the standard library

`python/benchmarks/compression.py` times the crate's codings beside the standard library's over
the same payload - the same wire format either way, so every row is an engine comparison.
`compression.py --min-time 0.2 --repeat 5`, one containerized x86_64 Linux run, over 1,080,000
bytes of JSON lines:

```text
zstd encode (yggdryl)     14.879 ms     69.2 MiB/s
zstd decode (yggdryl)      0.358 ms   2877.3 MiB/s
```

The standard library gains a `compression.zstd` module on Python 3.14+, which the benchmark times
beside these rows where it exists. The [gzip](gzip.md) and [zlib](zlib.md) pages carry their rows
from the same run.

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/rust/zstd.ipynb){ download },
[Python](notebooks/python/zstd.ipynb){ download },
[JavaScript](notebooks/javascript/zstd.ipynb){ download }.

<!-- /notebooks -->
