# zlib

RFC 1950 zlib framing and raw RFC 1951 DEFLATE, as whole buffers, as streams, or as a transparent handle.

## Contract

| | |
| --- | --- |
| Owns | `dump`/`load` (framed), `dump_raw`/`load_raw` (raw DEFLATE), their `_with_level` twins, `writer`/`reader`, `raw_writer`/`raw_reader`, `Zlib<H>` |
| Bindings | Whole-buffer pairs only: `loads`/`dumps`, `loads_raw`/`dumps_raw` (`loadsRaw`/`dumpsRaw` in JavaScript); `dumps` takes `level` 0 to 9 |
| Rust only | Streams and `Zlib<H>`, both built on `Read`/`Write` |
| Framing | Two-byte header plus four-byte Adler-32 trailer; raw carries neither, and nothing sniffs between them |
| Level | `Level::DEFAULT` for every plain form; affects encoding only |
| Streams | The writer must be finished; dropping it loses the final block and trailer |
| Handle | Writes materialize the decoded value and publish on `flush`, `close`, or `into_handle`; raw DEFLATE has no handle |
| Errors | Rust `Err`, Python `ValueError`, JavaScript throws |
| Engine | `zlib-rs` |

## Use

`dump` and `load` handle a complete buffer; everything else is the same pair under another shape.

=== "Rust"

    ```rust
    use yggdryl::coding::zlib;

    let text = "symbol,price\n".to_string() + &"AAPL,1\n".repeat(64);
    let plain = text.as_bytes();

    let encoded = zlib::dump(plain)?;
    assert_eq!(zlib::load(&encoded)?, plain);
    assert!(encoded.len() < plain.len());
    ```

=== "Python"

    ```python
    from yggdryl.coding import zlib

    plain = b"symbol,price\n" + b"AAPL,1\n" * 64

    encoded = zlib.dumps(plain)
    assert zlib.loads(encoded) == plain
    assert len(encoded) < len(plain)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { zlib } = require('yggdryl')

    const plain = Buffer.from('symbol,price\n' + 'AAPL,1\n'.repeat(64))

    const encoded = zlib.dumps(plain)
    assert.deepEqual(zlib.loads(encoded), plain)
    assert.ok(encoded.length < plain.length)
    ```

## Framed and raw

The `*_raw` pair carries the DEFLATE stream alone: no zlib header, no Adler-32 trailer.

=== "Rust"

    ```rust
    use yggdryl::coding::zlib;

    let text = "symbol,price\n".to_string() + &"AAPL,1\n".repeat(64);
    let plain = text.as_bytes();

    let framed = zlib::dump(plain)?;
    let raw = zlib::dump_raw(plain)?;

    assert_eq!(zlib::load(&framed)?, plain);
    assert_eq!(zlib::load_raw(&raw)?, plain);

    // The framing is a two-byte header plus a four-byte Adler-32 trailer.
    assert_eq!(framed.len(), raw.len() + 6);

    // Neither decoder accepts the other's bytes.
    assert!(zlib::load(&raw).is_err());
    assert!(zlib::load_raw(&framed).is_err());
    ```

=== "Python"

    ```python
    import zlib as standard

    import pytest

    from yggdryl.coding import zlib

    plain = b"symbol,price\n" + b"AAPL,1\n" * 64

    framed = zlib.dumps(plain)
    raw = zlib.dumps_raw(plain)

    assert zlib.loads(framed) == plain
    assert zlib.loads_raw(raw) == plain

    # The framing is a two-byte header plus a four-byte Adler-32 trailer.
    assert len(framed) == len(raw) + 6

    # Neither decoder accepts the other's bytes.
    with pytest.raises(ValueError):
        zlib.loads(raw)
    with pytest.raises(ValueError):
        zlib.loads_raw(framed)

    # The raw pair is what the standard library spells with a negative window.
    assert standard.decompress(raw, -standard.MAX_WBITS) == plain
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const standard = require('node:zlib')
    const { zlib } = require('yggdryl')

    const plain = Buffer.from('symbol,price\n' + 'AAPL,1\n'.repeat(64))

    const framed = zlib.dumps(plain)
    const raw = zlib.dumpsRaw(plain)

    assert.deepEqual(zlib.loads(framed), plain)
    assert.deepEqual(zlib.loadsRaw(raw), plain)

    // The framing is a two-byte header plus a four-byte Adler-32 trailer.
    assert.equal(framed.length, raw.length + 6)

    // Neither decoder accepts the other's bytes.
    assert.throws(() => zlib.loads(raw))
    assert.throws(() => zlib.loadsRaw(framed))

    // The raw pair is what node:zlib spells inflateRaw.
    assert.deepEqual(standard.inflateRawSync(raw), plain)
    ```

Use the raw pair where the framing belongs to something else: ZIP entries and HTTP's `deflate` content coding. Elsewhere keep the framed pair and its checksum.

## Levels

Every Rust entry point has a `_with_level` twin; the plain form is `Level::DEFAULT`.

```rust
use yggdryl::{Level};
use yggdryl::coding::zlib;

let text = "symbol,price\n".to_string() + &"AAPL,1\n".repeat(64);
let plain = text.as_bytes();

for level in [Level::NONE, Level::FAST, Level::DEFAULT, Level::BEST] {
    assert_eq!(zlib::load(&zlib::dump_with_level(plain, level)?)?, plain);
}

// NONE stores the payload, so framing makes it bigger.
assert!(zlib::dump_with_level(plain, Level::NONE)?.len() > plain.len());
assert!(zlib::dump_with_level(plain, Level::BEST)?.len() < plain.len());

// Raw DEFLATE takes the same levels.
let raw = zlib::dump_raw_with_level(plain, Level::BEST)?;
assert_eq!(zlib::load_raw(&raw)?, plain);
```

## Streams

Rust only. `writer` and `reader` do the same work without holding the whole payload.

```rust
use std::io::{Read, Write};
use yggdryl::coding::zlib;

let text = "symbol,price\n".to_string() + &"AAPL,1\n".repeat(64);
let plain = text.as_bytes();

let mut target = Vec::new();
let mut encoder = zlib::writer(&mut target);
encoder.write_all(plain)?;
encoder.finish()?;

let mut decoded = Vec::new();
zlib::reader(target.as_slice()).read_to_end(&mut decoded)?;
assert_eq!(decoded, plain);
```

`raw_writer` and `raw_reader` are the unframed pair, with `_with_level` on the writing side.

```rust
use std::io::{Read, Write};
use yggdryl::{Level};
use yggdryl::coding::zlib;

let text = "symbol,price\n".to_string() + &"AAPL,1\n".repeat(64);
let plain = text.as_bytes();

let mut target = Vec::new();
let mut encoder = zlib::raw_writer_with_level(&mut target, Level::BEST);
encoder.write_all(plain)?;
encoder.finish()?;

let mut decoded = Vec::new();
zlib::raw_reader(target.as_slice()).read_to_end(&mut decoded)?;
assert_eq!(decoded, plain);

// A raw stream is exactly what the buffer form would have produced.
assert_eq!(zlib::load_raw(&target)?, plain);
```

## The transparent handle

Rust only. `Zlib` wraps one [byte handle](../holder/index.md): reads decompress and writes compress, so an `IOBase` consumer sees plain bytes.

```rust
use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::coding::zlib::{self, Zlib};

let text = "symbol,price\n".to_string() + &"AAPL,1\n".repeat(64);
let plain = text.as_bytes();

let mut handle = Zlib::new(Buffer::new());
handle.write_all_bytes(plain)?;
handle.flush()?;

// The wrapper reads plain text and reports the decoded size.
assert_eq!(handle.read_all_bytes()?, plain);
assert_eq!(handle.size(), plain.len() as u64);

// The wrapped handle holds the compressed stream.
assert_eq!(zlib::load(&handle.handle().read_all_bytes()?)?, plain);
```

Sequential reads use [`IOBase::pstream_bytes`](../holder/iobase/bytes.md), decoding from the wrapped handle into bounded arrays.

```rust
use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::coding::zlib::{self, Zlib};
use yggdryl::Level;

let plain: &[u8] = b"symbol,price\nAAPL,1\n";

let mut handle = Zlib::new(Buffer::new()).with_level(Level::BEST);
assert_eq!(handle.level(), Level::BEST);

handle.write_all_bytes(plain)?;

// No flush: into_handle publishes first.
let inner = handle.into_handle()?;
assert_eq!(zlib::load(&inner.read_all_bytes()?)?, plain);
```

## Why raw DEFLATE has no handle

Rust only. A handle is chosen from what a payload declares, and raw DEFLATE declares nothing, so [`coding::Coded`](index.md) answers with the zlib handle.

```rust
use yggdryl::coding::Coded;
use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::coding::zlib;

let plain: &[u8] = b"symbol,price\nAAPL,1\n";

let mut handle = Coded::wrap(Buffer::new(), yggdryl::Codec::Deflate);
assert_eq!(handle.codec(), yggdryl::Codec::Zlib);

handle.write_all_bytes(plain)?;
handle.flush()?;
assert_eq!(handle.read_all_bytes()?, plain);
assert_eq!(zlib::load(&handle.handle().read_all_bytes()?)?, plain);

// Nothing to detect: a coding, not a file format.
assert_eq!(yggdryl::Codec::Deflate.extension(), None);
assert_eq!(yggdryl::Codec::Zlib.extension(), Some("zz"));
```

Buffer and stream operations keep the distinction; [`Codec`](index.md) dispatches to this module for both.

```rust
use yggdryl::{Codec};
use yggdryl::coding::zlib;

// An HTTP Content-Encoding header value parses directly.
let coding = Codec::from_str("deflate")?;
assert_eq!(coding, Codec::Deflate);

let plain: &[u8] = b"symbol,price\nAAPL,1\n";
let body = coding.dump(plain)?;

assert_eq!(coding.load(&body)?, plain);
// It really is unframed.
assert_eq!(zlib::load_raw(&body)?, plain);
assert!(zlib::load(&body).is_err());
```

## Edges

- `load(raw)` or `load_raw(framed)` -> refused: Rust `Err`, Python `ValueError`, JavaScript throw.
- `Level::NONE` -> stores the payload, so framed output is larger than the input.
- `writer` dropped without `finish` -> the final block and the Adler-32 trailer are never written.
- `Zlib<H>` written without `flush` -> the wrapped handle stays stale until `flush`, `close`, or `into_handle`.
- `pstream_bytes` with a non-zero start -> decodes and discards the prefix; a zlib stream has no decoded seek.
- [`Buffered`](../holder/backends/buffered.md) around `Zlib<H>` -> its cache stays empty on the streamed path.
- `Coded::wrap(_, Codec::Deflate)` -> the handle reports `Codec::Zlib` and writes framed bytes.
- `Codec::Deflate.extension()` -> `None`; `Codec::Zlib.extension()` -> `Some("zz")`.
- `Codec::from_str("deflate").dump` -> unframed bytes; `zlib::load` refuses them, `load_raw` reads them.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib coding::zlib::
    cargo bench -p yggdryl --bench coding -- zlib
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/coding/test_codings.py -k "zlib or RawDeflate"
    python/.venv/bin/python python/benchmarks/coding.py --min-time 0.2 --repeat 5
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="byte codings|raw DEFLATE" node/tests/text/codec.test.js
    npm run --prefix node bench:coding
    ```

## Performance

`python/benchmarks/coding.py` times `zlib-rs` beside the standard library's zlib over 1,080,000 bytes of JSON lines, one containerized x86_64 Linux run, same wire format.

```text
zlib encode (yggdryl)      0.344 ms   2998.3 MiB/s
zlib encode (stdlib zlib)  3.177 ms    324.2 MiB/s
zlib decode (yggdryl)      0.234 ms   4401.1 MiB/s
zlib decode (stdlib zlib)  0.484 ms   2127.9 MiB/s
```

`zlib-rs` level 6 trades a little ratio for speed on repetitive payloads; raise the level when size matters. [gzip](gzip.md) and [zstd](zstd.md) share this run.

```bash
python/.venv/bin/python python/benchmarks/coding.py --min-time 0.2 --repeat 5
```
