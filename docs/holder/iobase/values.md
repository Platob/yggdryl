# Values

Whole-value byte conveniences, digests, structured JSON, YAML, and TOML scalars, and the Rust `std::io` adapters.

## Contract

| surface | calls | behavior |
| --- | --- | --- |
| Whole bytes | `read_all_bytes`, `write_all_bytes`, `read_range_bytes`, `append_bytes`, `pwrite_all`, `clear` | derived from `pread`/`pwrite`; `append_bytes` answers the landing offset |
| Binding spelling | `read_bytes`/`read_text`, `write_bytes`/`write_text` | `read_range_bytes` and `append_bytes` keep the core name, camelCased in JavaScript |
| Copy | `copy_into`, `copyInto` in JavaScript | chunked, so neither side is buffered whole; carries the media type across |
| Digests | `read_digest`, `read_range_digest` | stream [`pstream_bytes`](bytes.md) and retain one bounded chunk |
| Structured | `read_scalar`, `write_scalar` | the media type selects JSON, YAML, or TOML and any outer gzip, zlib, or zstd coding |
| Field | optional on both scalar calls | directs native parsing and casting; omitted, it infers the natural value |
| Struct row | Rust `Scalar::Sequence` | Python and JavaScript restore field names; `cls=Scalar` and `{ scalar: true }` return the core value |
| Adapters | `reader_at`, `writer_at` | Rust only; each advances its own offset |

## Use

The inferring `read_range` and `append` entry points sit on the [Python](../../extensions/python.md) and [JavaScript](../../extensions/javascript.md) pages.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;

    let mut handle = Buffer::new();
    handle.write_all_bytes(b"symbol,price\n")?;

    // `append_bytes` reports the offset the bytes landed at.
    assert_eq!(handle.append_bytes(b"AAPL,1\n")?, 13);
    assert_eq!(handle.read_range_bytes(0, 6)?, b"symbol");
    // A range past the end yields what exists rather than failing.
    assert!(handle.read_range_bytes(100, 4)?.is_empty());
    assert_eq!(handle.read_all_bytes()?.len(), 20);
    ```

=== "Python"

    ```python
    from yggdryl import IOBase

    handle = IOBase.from_bytes()
    handle.write_bytes(b"symbol,price\n")

    # `append_bytes` reports the offset the bytes landed at.
    assert handle.append_bytes(b"AAPL,1\n") == 13
    assert handle.read_range_bytes(0, 6) == b"symbol"
    # A range past the end yields what exists rather than raising.
    assert handle.read_range_bytes(100, 4) == b""
    assert len(handle.read_bytes()) == 20
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase } = require('yggdryl')

    const handle = IOBase.fromBytes()
    handle.writeBytes(Buffer.from('symbol,price\n'))

    // `appendBytes` reports the offset the bytes landed at.
    assert.equal(handle.appendBytes(Buffer.from('AAPL,1\n')), 13)
    assert.equal(handle.readRangeBytes(0, 6).toString(), 'symbol')
    // A range past the end yields what exists rather than throwing.
    assert.equal(handle.readRangeBytes(100, 4).length, 0)
    assert.equal(handle.readBytes().length, 20)
    ```

## Digests

Both answer a [`Digest`](../../xxhash/index.md) rather than the bytes, and every backend and every wrapper inherits them. [Handles](../../xxhash/handles.md) carries `Hashed<H>` and the Rust-only pass-through pair.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::DigestAlgorithm;

    let mut handle = Buffer::new();
    handle.write_all_bytes(b"symbol,price\nAAPL,1\n")?;

    assert_eq!(
        handle.read_digest(DigestAlgorithm::Xxh3)?,
        DigestAlgorithm::Xxh3.digest(&handle.read_all_bytes()?),
    );
    assert_eq!(
        handle.read_range_digest(0, 6, DigestAlgorithm::Xxh3)?,
        DigestAlgorithm::Xxh3.digest(b"symbol"),
    );
    // Absence is emptiness here too: nothing written digests as no bytes.
    assert_eq!(
        Buffer::new().read_digest(DigestAlgorithm::Xxh32)?,
        DigestAlgorithm::Xxh32.digest(b""),
    );
    ```

=== "Python"

    ```python
    from yggdryl import IOBase, xxhash

    handle = IOBase.from_bytes()
    handle.write_bytes(b"symbol,price\nAAPL,1\n")

    assert handle.read_digest("xxh3-64") == xxhash.digest(handle.read_bytes(), "xxh3-64")
    assert handle.read_range_digest(0, 6) == xxhash.digest(b"symbol", "xxh3-64")
    assert IOBase.from_bytes().read_digest("xxh32") == xxhash.digest(b"", "xxh32")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase, xxhash } = require('yggdryl')

    const handle = IOBase.fromBytes()
    const payload = Buffer.from('symbol,price\nAAPL,1\n')
    handle.writeBytes(payload)

    assert.ok(handle.readDigest('xxh3-64').equals(xxhash.digest(payload, 'xxh3-64')))
    assert.ok(handle.readRangeDigest(0, 6).equals(xxhash.digest(Buffer.from('symbol'), 'xxh3-64')))
    ```

## Structured values

Reads feed the parser from `pstream_bytes`, so decoded pages are not retained. The codecs themselves are on the [structured text](../../text/index.md) layer.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::{Field, Url, Scalar};

    let media = Url::from_str("file:///trade.json.gz")?.media_type();
    let mut handle = Buffer::new().with_media_type(media);
    let value = Scalar::from_record([
        ("quantity", Scalar::from(2_i64)),
        ("symbol", Scalar::from("AAPL")),
    ])?;
    handle.write_scalar(&value)?;

    let field = Field::from_str(
        "trade: struct<quantity: int32 not null, symbol: utf8 not null> not null",
    )?;
    assert_eq!(handle.read_scalar(Some(&field))?[0], Scalar::from(2_i64));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase, Scalar

    path = pathlib.Path(tempfile.mkdtemp()) / "trade.json.gz"
    handle = IOBase(path)
    handle.write_scalar({"quantity": 2, "symbol": "AAPL"})
    field = "trade: struct<quantity: int32 not null, symbol: utf8 not null> not null"
    assert handle.read_scalar(field) == {"quantity": 2, "symbol": "AAPL"}
    value = handle.read_scalar(field, cls=Scalar)
    assert value.kind == "sequence"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase, Scalar } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-value-'))
    const handle = new IOBase(path.join(root, 'trade.json.gz'))
    handle.writeScalar({ quantity: 2, symbol: 'AAPL' })
    const field = 'trade: struct<quantity: int32 not null, symbol: utf8 not null> not null'
    assert.deepEqual(handle.readScalar(field), { quantity: 2, symbol: 'AAPL' })
    const value = handle.readScalar({ field, scalar: true })
    assert.ok(value instanceof Scalar)
    assert.equal(value.kind, 'sequence')
    ```

## Streaming adapters

Rust only.

```rust
use std::io::{Read, Write};

use yggdryl::IOBase;
use yggdryl::holder::Buffer;

let mut handle = Buffer::new();
handle.writer_at(0).write_all(b"symbol,price\n")?;
handle.append_bytes(b"AAPL,1\n")?;

let mut text = String::new();
handle.reader_at(13).read_to_string(&mut text)?;
assert_eq!(text, "AAPL,1\n");
```

`reader_at` and `writer_at` borrow the handle as a `Reader`/`Writer`. [Adding and removing a coding](bytes.md) moves bytes through a coding in all three languages.

## Edges

- `read_range_bytes` past the end -> what exists, never a failure.
- `pread_exact` on a value shorter than the buffer -> fails, naming the shortfall.
- `read_range_digest` -> clamps the range exactly as `read_range_bytes` clamps it.
- Digest of a resource that does not exist -> the digest of no bytes.
- Digest of a container -> typed failure naming the kind.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib iobase::tests::conformance
    cargo bench --bench media --features parquet -- io_scalar
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/holder/test_io.py -k StructuredValues
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern "structured values" "node/tests/holder/io.test.js"
    npm run --prefix node bench:holder:io
    ```

## Performance

Criterion measured one 16,384-record JSON value through `IOBase`; each compressed case includes coding and parsing or rendering. The host is Windows 11 x86_64, an AMD Ryzen 5 150 (6 cores/12 threads), rustc 1.96.1, on 2026-08-23.

| representation | `read_scalar` | read throughput | `write_scalar` | write throughput |
| --- | ---: | ---: | ---: | ---: |
| JSON | 71.445 ms | 11.880 MiB/s | 19.137 ms | 44.352 MiB/s |
| JSON + gzip | 81.427 ms | 10.424 MiB/s | 394.83 ms | 2.150 MiB/s |
| JSON + zlib | 79.588 ms | 10.665 MiB/s | 385.78 ms | 2.200 MiB/s |
| JSON + zstd | 78.580 ms | 10.801 MiB/s | 195.60 ms | 4.339 MiB/s |

```bash
cargo bench --bench media --features parquet -- io_scalar
```
