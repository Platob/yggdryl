# zlib and deflate

RFC 1950 zlib framing and raw RFC 1951 DEFLATE, as whole buffers, as streams, or as a handle that decodes on the way through.

!!! note "Streams and handles are Rust only"
    Both whole-buffer pairs - framed and raw - cross to Python and JavaScript as
    `loads`/`dumps` and `loads_raw`/`dumps_raw` (`loadsRaw`/`dumpsRaw` in JavaScript). The
    streaming `reader`/`writer` and the transparent `Zlib<H>` handle are Rust only: both are
    built on `Read`/`Write`, which neither binding has a native spelling for.

=== "Rust"

    ```rust
    use yggdryl::zlib;

    let text = "symbol,price\n".to_string() + &"AAPL,1\n".repeat(64);
    let plain = text.as_bytes();

    let encoded = zlib::dump(plain)?;
    assert_eq!(zlib::load(&encoded)?, plain);
    assert!(encoded.len() < plain.len());
    ```

=== "Python"

    ```python
    from yggdryl import zlib

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

`dump` and `load` handle a complete buffer. Everything else on this page is the
same two operations under a different shape: a level, a stream, or a handle.

## Framed and raw

The `*_raw` pair encodes and decodes the DEFLATE stream alone, with no zlib
header and no Adler-32 trailer.

=== "Rust"

    ```rust
    use yggdryl::zlib;

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

    from yggdryl import zlib

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

The two forms are not interchangeable and nothing sniffs between them, because
raw DEFLATE has no magic bytes to sniff. You have to know which one the payload
carries. Reach for the raw pair when the framing belongs to something else: ZIP
entries store raw DEFLATE members, and HTTP's `deflate` content coding is
widely sent raw despite nominally meaning zlib. Everywhere else -- a file you
control, a column you compress yourself -- use the framed pair and keep the
checksum.

## Levels

Every entry point has a `_with_level` twin. The plain form is `Level::DEFAULT`.

```rust
use yggdryl::{Level, zlib};

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

A level only affects encoding. A decoder reads whatever level produced the
stream, so `load` never needs to be told.

## Streams

`writer` and `reader` do the same work without holding the whole payload. The
encoder must be finished: that call writes the final block and the trailer, and
dropping the encoder instead loses them.

```rust
use std::io::{Read, Write};
use yggdryl::zlib;

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

`raw_writer` and `raw_reader` are the unframed pair, with the same
`_with_level` twin on the writing side.

```rust
use std::io::{Read, Write};
use yggdryl::{Level, zlib};

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

`Zlib` wraps one [byte handle](io.md) and presents the decoded bytes. Reads
decompress and writes compress, so anything that takes an `IOBase` sees plain
bytes while the wrapped handle holds the zlib form.

```rust
use yggdryl::io::{Buffer, IOBase};
use yggdryl::zlib::{self, Zlib};

let text = "symbol,price\n".to_string() + &"AAPL,1\n".repeat(64);
let plain = text.as_bytes();

let mut handle = Zlib::new(Buffer::new());
handle.write_all_bytes(plain)?;
handle.flush()?;

// The wrapper reads plain text and reports the decoded size.
assert_eq!(handle.read_all_bytes()?, plain);
assert_eq!(handle.size(), plain.len() as u64);

// The wrapped handle holds the compressed stream.
assert_eq!(zlib::load(handle.handle().as_slice())?, plain);
```

A coding cannot be written positionally, so the decoded value is materialized
on first use and published on `flush` or `close`. `into_handle` publishes the
pending write and hands back what it wrapped.

```rust
use yggdryl::io::{Buffer, IOBase};
use yggdryl::zlib::{self, Zlib};
use yggdryl::Level;

let plain: &[u8] = b"symbol,price\nAAPL,1\n";

let mut handle = Zlib::new(Buffer::new()).with_level(Level::BEST);
assert_eq!(handle.level(), Level::BEST);

handle.write_all_bytes(plain)?;

// No flush: into_handle publishes first.
let inner = handle.into_handle()?;
assert_eq!(zlib::load(inner.as_slice())?, plain);
```

## Why raw DEFLATE has no handle of its own

A transparent handle is chosen from what a payload declares -- a filename
suffix, a media type, a content coding. Raw DEFLATE declares nothing: it has no
framing to detect and no customary suffix. So it has no handle, and asking for
one through [`generic::Codec`](generic.md) gives the zlib handle, which is the
framed form of the same algorithm.

```rust
use yggdryl::generic::Codec;
use yggdryl::io::{Buffer, IOBase};
use yggdryl::zlib;

let plain: &[u8] = b"symbol,price\nAAPL,1\n";

let mut handle = Codec::wrap(Buffer::new(), yggdryl::Codec::Deflate);
assert_eq!(handle.codec(), yggdryl::Codec::Zlib);

handle.write_all_bytes(plain)?;
handle.flush()?;
assert_eq!(handle.read_all_bytes()?, plain);
assert_eq!(zlib::load(handle.handle().as_slice())?, plain);

// Nothing to detect: a coding, not a file format.
assert_eq!(yggdryl::Codec::Deflate.extension(), None);
assert_eq!(yggdryl::Codec::Zlib.extension(), Some("zz"));
```

The buffer and stream operations keep the distinction, because there the caller
already knows which form the bytes are in. [`Codec`](enums.md) dispatches to
this module for both.

```rust
use yggdryl::{Codec, zlib};

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

## Related

* [gzip](gzip.md) -- RFC 1952 framing over the same DEFLATE algorithm, with a
  filename suffix and a CRC-32.
* [zstd](zstd.md) -- a faster codec at comparable ratios, when the reader is
  yours to choose.
* [generic](generic.md) -- one enum over every coding, for when the coding is
  data rather than a decision in the source.

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/rust/zlib.ipynb){ download },
[Python](notebooks/python/zlib.ipynb){ download },
[JavaScript](notebooks/javascript/zlib.ipynb){ download }.

<!-- /notebooks -->
