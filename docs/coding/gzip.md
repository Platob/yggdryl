# gzip

RFC 1952 gzip as whole buffers, Rust streams, and a transparent `Gzip<H>` handle.

## Contract

| | |
| --- | --- |
| Owns | `load`/`dump`, `dump_with_level`, `reader`/`writer`, `writer_with_level`, `Gzip<H>` |
| Bindings | `loads`/`dumps` only; streams and `Gzip<H>` are Rust only |
| Wire format | RFC 1952, shared with Python `gzip` and `node:zlib` |
| Engine | `zlib-rs` |
| Level | 0 to 9, clamped, default 6 |
| Handle | `Gzip<H>` is an [`IOBase`](../holder/index.md) over an `IOBase`: reads decompress, writes compress, nothing seeks |
| Commit | `flush`, `close`, or `into_handle`; never `pwrite` |
| Streams | `writer` over any `Write`, `reader` over any `Read`; one compression window, not the whole payload |
| Streamed reads | [`pstream_bytes`](../holder/iobase/bytes.md), bounded arrays, handle never opened |
| Inference | `.gz` as the last suffix selects [`Codec::Gzip`](index.md); `Codec::from_url` off a location, `Codec::from_media_type` off a media type |

## Use

`load` and `dump` hold both sides in memory; the bindings spell them `loads`/`dumps`.

=== "Rust"

    ```rust
    use yggdryl::coding::gzip;

    let encoded = gzip::dump(b"symbol,price\nAAPL,1\n")?;
    assert_eq!(gzip::load(&encoded)?, b"symbol,price\nAAPL,1\n");
    ```

=== "Python"

    ```python
    import gzip as standard

    from yggdryl.coding import gzip

    encoded = gzip.dumps(b"symbol,price\nAAPL,1\n")
    assert gzip.loads(encoded) == b"symbol,price\nAAPL,1\n"

    # One wire format, so the standard library reads what this wrote and back.
    assert standard.decompress(encoded) == b"symbol,price\nAAPL,1\n"
    assert gzip.loads(standard.compress(b"symbol,price\nAAPL,1\n")) == (
        b"symbol,price\nAAPL,1\n"
    )
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const standard = require('node:zlib')
    const { gzip } = require('yggdryl')

    const payload = Buffer.from('symbol,price\nAAPL,1\n')
    const encoded = gzip.dumps(payload)
    assert.deepEqual(gzip.loads(encoded), payload)

    // One wire format, so node:zlib reads what this wrote and back.
    assert.deepEqual(standard.gunzipSync(encoded), payload)
    assert.deepEqual(gzip.loads(standard.gzipSync(payload)), payload)
    ```

[zlib](zlib.md) and [zstd](zstd.md) share the signatures; [`Codec`](index.md) selects at runtime.

## Levels

`Level` clamps rather than validates.

=== "Rust"

    ```rust
    use yggdryl::coding::gzip;
    use yggdryl::Level;

    let payload = b"AAPL,1\nAAPL,2\nAAPL,3\nAAPL,4\nAAPL,5\nAAPL,6\nAAPL,7\nAAPL,8\n";

    let stored = gzip::dump_with_level(payload, Level::NONE)?;
    let smallest = gzip::dump_with_level(payload, Level::BEST)?;
    assert!(smallest.len() < stored.len());
    assert_eq!(gzip::load(&stored)?, payload);
    assert_eq!(gzip::load(&smallest)?, payload);

    // One 0-to-9 scale, clamped at both ends, with 6 when nobody chooses.
    assert_eq!(Level::DEFAULT.get(), 6);
    assert_eq!(Level::new(12), Level::BEST);
    assert_eq!(gzip::dump(payload)?, gzip::dump_with_level(payload, Level::DEFAULT)?);
    ```

=== "Python"

    ```python
    from yggdryl.coding import gzip

    payload = b"AAPL,1\nAAPL,2\nAAPL,3\nAAPL,4\nAAPL,5\nAAPL,6\nAAPL,7\nAAPL,8\n"

    stored = gzip.dumps(payload, level=0)
    smallest = gzip.dumps(payload, level=9)
    assert len(smallest) < len(stored)
    assert gzip.loads(stored) == payload
    assert gzip.loads(smallest) == payload

    # The same 0-to-9 scale, clamped at both ends, with 6 when nobody chooses.
    assert gzip.dumps(payload) == gzip.dumps(payload, level=6)
    assert gzip.dumps(payload, level=12) == smallest
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { gzip } = require('yggdryl')

    const payload = Buffer.from('AAPL,1\nAAPL,2\nAAPL,3\nAAPL,4\nAAPL,5\nAAPL,6\n')

    const stored = gzip.dumps(payload, 0)
    const smallest = gzip.dumps(payload, 9)
    assert.ok(smallest.length < stored.length)
    assert.deepEqual(gzip.loads(stored), payload)
    assert.deepEqual(gzip.loads(smallest), payload)

    // The same 0-to-9 scale, clamped at both ends, with 6 when nobody chooses.
    assert.deepEqual(gzip.dumps(payload), gzip.dumps(payload, 6))
    assert.deepEqual(gzip.dumps(payload, 12), smallest)
    ```

## Streams

Rust only. `finish` writes the trailer.

```rust
use std::io::{Read, Write};
use yggdryl::coding::gzip;

let mut target = Vec::new();
let mut encoder = gzip::writer(&mut target);
encoder.write_all(b"symbol,price\nAAPL,1\n")?;
encoder.finish()?;

let mut decoded = Vec::new();
gzip::reader(target.as_slice()).read_to_end(&mut decoded)?;
assert_eq!(decoded, b"symbol,price\nAAPL,1\n");
```

Decoding stops where the reader stops:

```rust
use std::io::{Read, Write};
use yggdryl::{Level};
use yggdryl::coding::gzip;

let mut target = Vec::new();
let mut encoder = gzip::writer_with_level(&mut target, Level::BEST);
encoder.write_all(b"symbol,price\nAAPL,1\n")?;
encoder.finish()?;

let mut head = [0_u8; 6];
gzip::reader(target.as_slice()).read_exact(&mut head)?;
assert_eq!(&head, b"symbol");
```

## A handle that hides the coding

Rust only. Downstream encodings and codecs never see the coding.

```rust
use yggdryl::coding::gzip::{self, Gzip};
use yggdryl::IOBase;
use yggdryl::holder::Buffer;

let mut handle = Gzip::new(Buffer::new());
handle.write_all_bytes(b"symbol,price\nAAPL,1\n")?;
handle.flush()?;

// The wrapper reads and measures the plain bytes.
assert_eq!(handle.read_all_bytes()?, b"symbol,price\nAAPL,1\n");
assert_eq!(handle.size(), 20);

// The wrapped handle holds the gzip member.
let inner = handle.into_handle()?;
assert_eq!(gzip::load(&inner.read_all_bytes()?)?, b"symbol,price\nAAPL,1\n");
```

A level set on the handle reaches its encoder:

```rust
use yggdryl::coding::gzip::Gzip;
use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::Level;

let mut handle = Gzip::new(Buffer::new()).with_level(Level::BEST);
assert_eq!(handle.level(), Level::BEST);

handle.write_all_bytes(b"symbol,price\nAAPL,1\n")?;
handle.flush()?;
assert_eq!(handle.read_all_bytes()?, b"symbol,price\nAAPL,1\n");
```

## A `.gz` name is enough

Rust only. A compound [filename](../uri/path.md) names the coding.

```rust
use yggdryl::coding::Coded;
use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::{MediaType, MimeType};

// `.gz` is the last suffix, so gzip is the outermost coding of a CSV.
let named = MediaType::from_file_name("trades.csv.gz");
assert_eq!(named.base(), &MimeType::CSV);
assert_eq!(yggdryl::Codec::from_media_type(&named), yggdryl::Codec::Gzip);

// A handle that declares that media type picks its own coding.
let mut handle = Coded::infer(Buffer::new().with_media_type(named));
assert_eq!(handle.codec(), yggdryl::Codec::Gzip);

handle.write_all_bytes(b"symbol,price\nAAPL,1\n")?;
handle.flush()?;
assert_eq!(yggdryl::coding::gzip::load(&handle.handle().read_all_bytes()?)?, b"symbol,price\nAAPL,1\n");
```

A coded handle reports the *decoded* media type:

```rust
use yggdryl::coding::gzip::Gzip;
use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::{MediaType, MimeType};

let buffer = Buffer::new().with_media_type(MediaType::from_file_name("trades.csv.gz"));
assert!(buffer.media_type().is_encoded());

let handle = Gzip::new(buffer);
assert_eq!(handle.media_type().base(), &MimeType::CSV);
assert!(!handle.media_type().is_encoded());
```

## Failures

Rust only.

```rust
use yggdryl::coding::gzip;

assert!(gzip::load(b"definitely not a compressed payload").is_err());
```

Absence is not a failure:

```rust
use yggdryl::coding::gzip::Gzip;
use yggdryl::IOBase;
use yggdryl::holder::Buffer;

let handle = Gzip::new(Buffer::new());
assert_eq!(handle.size(), 0);
assert!(handle.read_all_bytes()?.is_empty());
```

## Edges

- `gzip::load` on a non-gzip payload -> `Err`.
- `Gzip::new` over a missing resource -> size 0, empty read, no error.
- `pwrite` on `Gzip<H>` with no later `flush` -> the write is lost.
- `gzip::writer` dropped before `finish` -> no trailer.
- `pstream_bytes` past offset zero -> the prefix is decoded and discarded; a surrounding [`Buffered`](../holder/backends/buffered.md) stays empty.
- `Level::new(12)` -> `Level::BEST`; `Level::NONE` -> a valid member with stored DEFLATE blocks.
- Repetitive payloads at level 6 -> some ratio traded for speed; raise the level for size.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib coding::gzip::
    cargo bench -p yggdryl --bench coding -- gzip
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/coding/test_codings.py -k gzip
    python/.venv/bin/python python/benchmarks/coding.py --min-time 0.2 --repeat 5
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="byte codings" node/tests/text/codec.test.js
    npm run --prefix node bench:coding
    ```

## Performance

One containerized x86_64 Linux run of the Python binding against the standard library's `gzip`, over 1,080,000 bytes of JSON lines.

```text
gzip encode (yggdryl)      0.362 ms   2848.0 MiB/s
gzip encode (stdlib gzip)  3.003 ms    343.0 MiB/s
gzip decode (yggdryl)      0.253 ms   4065.4 MiB/s
gzip decode (stdlib gzip)  0.396 ms   2597.8 MiB/s
```

`zlib-rs` puts the encode 8x ahead; [zlib](zlib.md) and [zstd](zstd.md) carry their rows from the same run.

```bash
python/.venv/bin/python python/benchmarks/coding.py --min-time 0.2 --repeat 5
```
