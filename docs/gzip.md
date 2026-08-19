# gzip

Encode and decode RFC 1952 gzip, as whole buffers, as streams, or as a handle that hides the coding entirely.

!!! note "Streams and handles are Rust only"
    The whole-buffer pair crosses to Python and JavaScript as `loads`/`dumps`. The streaming
    `reader`/`writer` and the transparent `Gzip<H>` handle are Rust only: both are built on
    `Read`/`Write`, which neither binding has a native spelling for.

=== "Rust"

    ```rust
    use yggdryl::gzip;

    let encoded = gzip::dump(b"symbol,price\nAAPL,1\n")?;
    assert_eq!(gzip::load(&encoded)?, b"symbol,price\nAAPL,1\n");
    ```

=== "Python"

    ```python
    import gzip as standard

    from yggdryl import gzip

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
[zlib](zlib.md) and [zstd](zstd.md) with identical signatures, and
[`Codec`](enums.md) selects between them at runtime.

## Levels

=== "Rust"

    ```rust
    use yggdryl::gzip;
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
    from yggdryl import gzip

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

## Streams

```rust
use std::io::{Read, Write};
use yggdryl::gzip;

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
use yggdryl::{gzip, Level};

let mut target = Vec::new();
let mut encoder = gzip::writer_with_level(&mut target, Level::BEST);
encoder.write_all(b"symbol,price\nAAPL,1\n")?;
encoder.finish()?;

let mut head = [0_u8; 6];
gzip::reader(target.as_slice()).read_exact(&mut head)?;
assert_eq!(&head, b"symbol");
```

## A handle that hides the coding

```rust
use yggdryl::gzip::{self, Gzip};
use yggdryl::io::{Buffer, IOBase};

let mut handle = Gzip::new(Buffer::new());
handle.write_all_bytes(b"symbol,price\nAAPL,1\n")?;
handle.flush()?;

// The wrapper reads and measures the plain bytes.
assert_eq!(handle.read_all_bytes()?, b"symbol,price\nAAPL,1\n");
assert_eq!(handle.size(), 20);

// The wrapped handle holds the gzip member.
let inner = handle.into_handle()?;
assert_eq!(gzip::load(inner.as_slice())?, b"symbol,price\nAAPL,1\n");
```

`Gzip<H>` is an [`IOBase`](io.md) over an `IOBase`: reads decompress, writes compress,
and everything downstream sees the decoded bytes. That is what lets a record encoding or
a text codec work over a compressed resource without knowing it is compressed.

A coding is not seekable, so positional reads and writes cannot go straight through. The
decoded value is materialized on first use and the encoded form is republished on
`flush`, on `close`, or by `into_handle` - not on every `pwrite`. Skipping that flush
loses the write.

A level set on the handle reaches the encoder that publishes it:

```rust
use yggdryl::gzip::Gzip;
use yggdryl::io::{Buffer, IOBase};
use yggdryl::Level;

let mut handle = Gzip::new(Buffer::new()).with_level(Level::BEST);
assert_eq!(handle.level(), Level::BEST);

handle.write_all_bytes(b"symbol,price\nAAPL,1\n")?;
handle.flush()?;
assert_eq!(handle.read_all_bytes()?, b"symbol,price\nAAPL,1\n");
```

## A `.gz` name is enough

```rust
use yggdryl::generic::Codec;
use yggdryl::io::{Buffer, IOBase};
use yggdryl::{MediaType, MimeType};

// `.gz` is the last suffix, so gzip is the outermost coding of a CSV.
let named = MediaType::from_file_name("trades.csv.gz");
assert_eq!(named.base(), &MimeType::CSV);
assert_eq!(yggdryl::Codec::from_media_type(&named), yggdryl::Codec::Gzip);

// A handle that declares that media type picks its own coding.
let mut handle = Codec::infer(Buffer::new().with_media_type(named));
assert_eq!(handle.codec(), yggdryl::Codec::Gzip);

handle.write_all_bytes(b"symbol,price\nAAPL,1\n")?;
handle.flush()?;
assert_eq!(yggdryl::gzip::load(handle.handle().as_slice())?, b"symbol,price\nAAPL,1\n");
```

A compound filename is the only place the coding needs to be written down.
`Codec::from_url` reads it off a location and `Codec::from_media_type` off a media type,
so `trades.csv.gz` decodes without anyone naming gzip in the call. The generic
[`Codec`](generic.md) enum is the value that holds any of these handles at once;
`Gzip<H>` is what it picks here.

The media type a coded handle reports is the *decoded* one, because that is what its
bytes now are:

```rust
use yggdryl::gzip::Gzip;
use yggdryl::io::{Buffer, IOBase};
use yggdryl::{MediaType, MimeType};

let buffer = Buffer::new().with_media_type(MediaType::from_file_name("trades.csv.gz"));
assert!(buffer.media_type().is_encoded());

let handle = Gzip::new(buffer);
assert_eq!(handle.media_type().base(), &MimeType::CSV);
assert!(!handle.media_type().is_encoded());
```

## Failures

```rust
use yggdryl::gzip;

assert!(gzip::load(b"definitely not a compressed payload").is_err());
```

Absence is not a failure, though. A handle constructed over a resource that does not
exist yet decodes to nothing, exactly as reading past the end does, so a location can be
probed without a separate existence check:

```rust
use yggdryl::gzip::Gzip;
use yggdryl::io::{Buffer, IOBase};

let handle = Gzip::new(Buffer::new());
assert_eq!(handle.size(), 0);
assert!(handle.read_all_bytes()?.is_empty());
```

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/gzip-rust.ipynb){ download },
[Python](notebooks/gzip-python.ipynb){ download },
[JavaScript](notebooks/gzip-javascript.ipynb){ download }.

<!-- /notebooks -->
