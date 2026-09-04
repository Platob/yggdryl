# Content coding

The coding layer owns byte transformations and transparent coded handles.

## Codec: a coding over a handle


`Codec` names a content coding. `coding::Coded` applies it to a handle: reads decompress and writes compress while downstream code sees plain bytes.

`Coded::infer` takes the coding from the handle's media type.

```rust
use yggdryl::coding::Coded;
use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::Url;

let named = Buffer::new().with_media_type(Url::from_str("file:///trades.csv.zst")?.media_type());
let mut handle = Coded::infer(named);
assert_eq!(handle.codec(), yggdryl::Codec::Zstd);

handle.write_all_bytes(b"symbol,price\nAAPL,1\nAAPL,2\n")?;
handle.flush()?;

// The coded handle reads plain bytes; the handle underneath holds the frame.
assert_eq!(handle.read_all_bytes()?, b"symbol,price\nAAPL,1\nAAPL,2\n");
assert_ne!(handle.handle().read_all_bytes()?, b"symbol,price\nAAPL,1\nAAPL,2\n");
```

Use `Coded::wrap` when the handle does not declare its coding.

```rust
use yggdryl::coding::Coded;
use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::Level;

let mut handle = Coded::wrap(Buffer::new(), yggdryl::Codec::Gzip).with_level(Level::BEST);
handle.write_all_bytes(b"symbol,price\nAAPL,1\n")?;

// into_handle publishes the pending write, then gives back the compressed bytes.
let inner = handle.into_handle()?;
assert_eq!(yggdryl::coding::gzip::load(&inner.read_all_bytes()?)?, b"symbol,price\nAAPL,1\n");
```

There are four variants for five codings. Raw DEFLATE carries no framing to detect, so it has no transparent handle of its own and wraps as [`Zlib`](coding.md), the framed form of the same algorithm.

```rust
use yggdryl::coding::Coded;
use yggdryl::holder::Buffer;

let handle = Coded::wrap(Buffer::new(), yggdryl::Codec::Deflate);
assert_eq!(handle.codec(), yggdryl::Codec::Zlib);
```

`Coded<H>` composes over any `IOBase`, including `Holder` or another coded handle. Levels reach [`gzip`](coding.md), [`zlib`](coding.md), and [`zstd`](coding.md); `Identity` ignores them.


## gzip


Encode and decode RFC 1952 gzip, as whole buffers, as streams, or as a handle that hides the coding entirely.

!!! note "Streams and handles are Rust only"
    The whole-buffer pair crosses to Python and JavaScript as `loads`/`dumps`. The streaming
    `reader`/`writer` and the transparent `Gzip<H>` handle are Rust only: both are built on
    `Read`/`Write`, which neither binding has a native spelling for.

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

`load` and `dump` are the whole-buffer form: both sides are in memory at once, which is
what you want for a payload you already hold and not what you want for a file. The
streaming pair below costs one compression window instead.

The bindings name the pair `loads`/`dumps`, the plural spelling Python's own `json` and
`pickle` use for "from and to a value in memory", because that is the only form they
carry - a caller who sees `loads` there is never left wondering where `load` went.

The same four operations - `load`, `dump`, `reader`, `writer` - exist under
[zlib](coding.md) and [zstd](coding.md) with identical signatures, and
[`Codec`](types.md) selects between them at runtime.

### Levels

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

The level is one number on one scale in all three languages, because `Level` clamps rather
than validates: a caller who reaches for 12 gets the best this codec has instead of an
error about a range they had no reason to memorize.

`Level` is shared across every codec and mapped onto each one's native range, so raising
compression does not mean learning gzip's 0-to-9, zstd's 1-to-19, and a third scheme
after that. `Level::NONE` still emits a valid gzip member: the framing is there, the
DEFLATE blocks are stored rather than compressed.

### Streams

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

`writer` wraps any `Write` and `reader` wraps any `Read`, so neither side needs the
payload in memory. Finish the encoder explicitly - the trailer is written by `finish`,
and a dropped encoder is not a promise that it got there.

Decoding stops where the reader stops, which is what makes a header-only read cheap on a
large member:

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

### A handle that hides the coding

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

`Gzip<H>` is an [`IOBase`](holder.md) over an `IOBase`: reads decompress, writes compress,
and everything downstream sees the decoded bytes. That is what lets a record encoding or
a text codec work over a compressed resource without knowing it is compressed.

A coding is not seekable, so positional writes and opened sessions materialize the
decoded value. The encoded form is republished on `flush`, on `close`, or by
`into_handle` - not on every `pwrite`. Skipping that flush loses the write.

Sequential reads use [`IOBase::pstream_bytes`](holder.md#streamed-bytes): gzip is decoded
directly from the wrapped handle into bounded arrays, without opening the handle or
retaining earlier decoded pages. Starting after byte zero decodes and discards the
prefix because a gzip member has no decoded seek. A surrounding `Buffered` cache stays
empty on this path. The [stream benchmark](holder.md#measured-streamed-byte-behavior)
records first-chunk, full-drain, and whole-value costs beside zlib and zstd.

A level set on the handle reaches the encoder that publishes it:

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

### A `.gz` name is enough

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

A compound filename is the only place the coding needs to be written down.
`Codec::from_url` reads it off a location and `Codec::from_media_type` off a media type,
so `trades.csv.gz` decodes without anyone naming gzip in the call. The generic
[`Codec`](types.md) enum is the value that holds any of these handles at once;
`Gzip<H>` is what it picks here.

The media type a coded handle reports is the *decoded* one, because that is what its
bytes now are:

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

### Failures

```rust
use yggdryl::coding::gzip;

assert!(gzip::load(b"definitely not a compressed payload").is_err());
```

Absence is not a failure, though. A handle constructed over a resource that does not
exist yet decodes to nothing, exactly as reading past the end does, so a location can be
probed without a separate existence check:

```rust
use yggdryl::coding::gzip::Gzip;
use yggdryl::IOBase;
use yggdryl::holder::Buffer;

let handle = Gzip::new(Buffer::new());
assert_eq!(handle.size(), 0);
assert!(handle.read_all_bytes()?.is_empty());
```

### Against the standard library

`python/benchmarks/coding.py` times the crate's codings beside the standard library's over
the same payload - the same wire format either way, so every row is an engine comparison.
`compression.py --min-time 0.2 --repeat 5`, one containerized x86_64 Linux run, over 1,080,000
bytes of JSON lines:

```text
gzip encode (yggdryl)      0.362 ms   2848.0 MiB/s
gzip encode (stdlib gzip)  3.003 ms    343.0 MiB/s
gzip decode (yggdryl)      0.253 ms   4065.4 MiB/s
gzip decode (stdlib gzip)  0.396 ms   2597.8 MiB/s
```

The DEFLATE engine is `zlib-rs`, which is what puts the encode 8x ahead of the standard
library's; its level 6 trades a little ratio for that speed on highly repetitive payloads, so a
caller optimizing for size raises the level. The [zlib](coding.md) and [zstd](coding.md) pages carry
their rows from the same run.

## zlib and deflate


RFC 1950 zlib framing and raw RFC 1951 DEFLATE, as whole buffers, as streams, or as a handle that decodes on the way through.

!!! note "Streams and handles are Rust only"
    Both whole-buffer pairs - framed and raw - cross to Python and JavaScript as
    `loads`/`dumps` and `loads_raw`/`dumps_raw` (`loadsRaw`/`dumpsRaw` in JavaScript). The
    streaming `reader`/`writer` and the transparent `Zlib<H>` handle are Rust only: both are
    built on `Read`/`Write`, which neither binding has a native spelling for.

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

`dump` and `load` handle a complete buffer. Everything else on this page is the
same two operations under a different shape: a level, a stream, or a handle.

### Framed and raw

The `*_raw` pair encodes and decodes the DEFLATE stream alone, with no zlib
header and no Adler-32 trailer.

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

The two forms are not interchangeable and nothing sniffs between them, because
raw DEFLATE has no magic bytes to sniff. You have to know which one the payload
carries. Reach for the raw pair when the framing belongs to something else: ZIP
entries store raw DEFLATE members, and HTTP's `deflate` content coding is
widely sent raw despite nominally meaning zlib. Everywhere else -- a file you
control, a column you compress yourself -- use the framed pair and keep the
checksum.

### Levels

Every entry point has a `_with_level` twin. The plain form is `Level::DEFAULT`.

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

A level only affects encoding. A decoder reads whatever level produced the
stream, so `load` never needs to be told.

### Streams

`writer` and `reader` do the same work without holding the whole payload. The
encoder must be finished: that call writes the final block and the trailer, and
dropping the encoder instead loses them.

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

`raw_writer` and `raw_reader` are the unframed pair, with the same
`_with_level` twin on the writing side.

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

### The transparent handle

`Zlib` wraps one [byte handle](holder.md) and presents the decoded bytes. Reads
decompress and writes compress, so anything that takes an `IOBase` sees plain
bytes while the wrapped handle holds the zlib form.

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

A coding cannot be written positionally, so writes and opened sessions
materialize the decoded value and publish it on `flush` or `close`.
`into_handle` publishes the pending write and hands back what it wrapped.

Sequential reads use [`IOBase::pstream_bytes`](holder.md#streamed-bytes): zlib is
decoded directly from the wrapped handle into bounded arrays, without opening
the handle or retaining earlier decoded pages. A non-zero start decodes and
discards the prefix because a zlib stream has no decoded seek. A surrounding
`Buffered` cache stays empty on this path. The
[stream benchmark](holder.md#measured-streamed-byte-behavior) records first-chunk,
full-drain, and whole-value costs beside gzip and zstd.

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

### Why raw DEFLATE has no handle of its own

A transparent handle is chosen from what a payload declares -- a filename
suffix, a media type, a content coding. Raw DEFLATE declares nothing: it has no
framing to detect and no customary suffix. So it has no handle, and asking for
one through [`coding::Coded`](coding.md) gives the zlib handle, which is the
framed form of the same algorithm.

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

The buffer and stream operations keep the distinction, because there the caller
already knows which form the bytes are in. [`Codec`](types.md) dispatches to
this module for both.

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

### Related

* [gzip](coding.md) -- RFC 1952 framing over the same DEFLATE algorithm, with a
  filename suffix and a CRC-32.
* [zstd](coding.md) -- a faster codec at comparable ratios, when the reader is
  yours to choose.
* [generic](types.md) -- one enum over every coding, for when the coding is
  data rather than a decision in the source.

### Against the standard library

`python/benchmarks/coding.py` times the crate's codings beside the standard library's over
the same payload - the same wire format either way, so every row is an engine comparison.
`compression.py --min-time 0.2 --repeat 5`, one containerized x86_64 Linux run, over 1,080,000
bytes of JSON lines:

```text
zlib encode (yggdryl)      0.344 ms   2998.3 MiB/s
zlib encode (stdlib zlib)  3.177 ms    324.2 MiB/s
zlib decode (yggdryl)      0.234 ms   4401.1 MiB/s
zlib decode (stdlib zlib)  0.484 ms   2127.9 MiB/s
```

The DEFLATE engine is `zlib-rs`, which is what puts the encode 9x ahead of the standard
library's; its level 6 trades a little ratio for that speed on highly repetitive payloads, so a
caller optimizing for size raises the level. The [gzip](coding.md) and [zstd](coding.md) pages carry
their rows from the same run.

## Zstandard


Encode and decode RFC 8878 Zstandard frames as whole buffers, as streams, or through a handle that compresses transparently.

!!! note "Streams and handles are Rust only"
    The whole-buffer pair crosses to Python and JavaScript as `loads`/`dumps`. The streaming
    `reader`/`writer` and the transparent `Zstd<H>` handle are Rust only: both are built on
    `Read`/`Write`, which neither binding has a native spelling for.

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

`dump` produces one complete frame and `load` consumes one. Both hold the whole input and the whole output in memory; the streaming forms below hold one window instead, whatever the payload size.

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

The gain begins where the payload has repetition to remove; below that, framing dominates and the output grows. `load` decodes exactly one frame and rejects anything else, so a mis-routed payload fails at the codec rather than downstream - and it fails the same way in all three languages, as the binding's own error type.

### Streams

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

`writer` returns an [`Encoder`](types.md), which **must** be finished. Dropping it without calling `finish` omits the frame epilogue, and what was written is then not a valid Zstandard frame.

`reader` returns `Box<dyn Read>` rather than a `Result`. The decoder can only fail to construct when it cannot allocate its window; that failure is held and surfaced on the first `read` instead of panicking, so the reader is always usable as a value.

### Levels

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

[`Level`](types.md) is one 0-to-9 scale shared by every codec, so raising compression does not mean learning three numbering schemes. Zstandard's own range is 1 to 19, and the shared scale maps onto it by rounding `level * 19 / 9` up:

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

`Level::NONE` is the one entry that does not mean what its name suggests everywhere else: zstd has no store-uncompressed setting, so level 0 maps to zstd 1, its cheapest real level. Under [gzip](coding.md) and [zlib](coding.md) the same `Level::NONE` does store uncompressed. Every level round-trips; the only difference is time spent and bytes produced.

### The transparent handle

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

`Zstd<H>` is an [`IOBase`](holder.md) over another `IOBase`. Reads decompress and writes compress, so anything that takes a handle - a media reader, another coding - sees the decoded bytes while the wrapped handle keeps the Zstandard form. `size` follows the same rule: it is the decoded length, not the frame length.

A coding is not seekable, so writes and opened sessions materialize the decoded
value and publish pending changes on `flush` or `close`, not on every `pwrite`.

Sequential reads use [`IOBase::pstream_bytes`](holder.md#streamed-bytes): Zstandard
is decoded directly from the wrapped handle into bounded arrays, without
opening the handle or retaining earlier decoded pages. A non-zero start decodes
and discards the prefix because a frame has no decoded seek. A surrounding
`Buffered` cache stays empty on this path. The
[stream benchmark](holder.md#measured-streamed-byte-behavior) records first-chunk,
full-drain, and whole-value costs beside gzip and zlib.

```rust
use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::coding::zstd::Zstd;

// Wrapping touches nothing, and a handle with no bytes decodes to nothing.
let handle = Zstd::new(Buffer::new());
assert!(handle.read_all_bytes()?.is_empty());
assert_eq!(handle.size(), 0);
```

An empty wrapped handle decodes to an empty value rather than failing on an absent frame, which is the same laziness contract every handle follows: constructing touches nothing, reading something absent yields nothing, writing creates.

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

`with_level` sets the level writes encode at; it changes nothing about reads, since a frame carries what a decoder needs. `into_handle` returns the wrapped handle with any pending write already published, which is the way to hand the compressed bytes to something else without going through `flush` first.

When the coding is chosen at runtime, `Codec::Zstd` names it and [`coding::Coded`](coding.md) applies it to a handle.

### Against the standard library

`python/benchmarks/coding.py` times the crate's codings beside the standard library's over
the same payload - the same wire format either way, so every row is an engine comparison.
`compression.py --min-time 0.2 --repeat 5`, one containerized x86_64 Linux run, over 1,080,000
bytes of JSON lines:

```text
zstd encode (yggdryl)     14.879 ms     69.2 MiB/s
zstd decode (yggdryl)      0.358 ms   2877.3 MiB/s
```

The standard library gains a `compression.zstd` module on Python 3.14+, which the benchmark times
beside these rows where it exists. The [gzip](coding.md) and [zlib](coding.md) pages carry their rows
from the same run.
