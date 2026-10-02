# Compression

Content codings over any handle - gzip, zlib and Zstandard, each declared by a suffix on the name - and the one-shot codecs over bytes beside them, raw deflate among them.

## Overview

| | |
| --- | --- |
| Declared by | a coding suffix after the medium's own: `.gz` gzip, `.zz` zlib, `.zst` Zstandard - `trades.arrows.gz`, `logs/*.log.zst` |
| Build | default |
| Rust | `Codec` - `Identity`, `Gzip`, `Zlib`, `Deflate`, `Zstd` - the one dispatcher, with `load`, `dump`, `dump_with_level`, `reader` and `writer`; `yggdryl::gzip`, `zlib` and `zstd` the same doors per codec beside a handle wrapper (`Gzip<H>`, `Zlib<H>`, `Zstd<H>`); `yggdryl::coding::Coded` a handle presenting the decoded bytes the name declares |
| Python | coded `IOBase` handles by name; `yggdryl.gzip`, `zlib`, `zstd`: `loads` and `dumps`, and `zlib.loads_raw`, `zlib.dumps_raw` for raw deflate |
| JavaScript | coded `IOBase` handles by name; `gzip`, `zlib`, `zstd`: `loads` and `dumps`, and `zlib.loadsRaw`, `zlib.dumpsRaw` for raw deflate |
| Settings | `level`, one 0-9 scale for every codec (`Level::FAST` 1, `DEFAULT` 6, `BEST` 9); a handle whose name declares no coding ignores it |
| Refused | an outer coding over [Parquet](parquet.md), which compresses its pages inside the file |

A coding suffix on the name wraps the encoding: the same calls, compressed bytes underneath. `level` is the one setting.

## Read

A coded handle presents the decoded bytes: a read takes the coding off as it streams, holding only the decoder's state and the current chunk, so `trades.arrows` and `trades.arrows.gz` answer the same calls, and a folder or a glob takes each leaf's coding off by its own name. One payload decodes whole through `load` / `loads`, and a stream through `reader`; bytes that are not a frame of the codec are refused rather than returned.

=== "Rust"

    ```rust
    use yggdryl::coding::Coded;
    use yggdryl::holder::Buffer;
    use yggdryl::{gzip, zstd, Codec, IOBase, Scalar, Url};

    // Another writer's gzip, under a name that declares it.
    let url = Url::from_str("file:///trade.json.gz")?;
    assert_eq!(Codec::from_url(&url), Codec::Gzip);
    let stored = Buffer::from_bytes(gzip::dump(br#"{"symbol":"AAPL","quantity":2}"#)?)
        .with_media_type(url.media_type());

    // The value calls take the coding off on their own...
    let value = stored.read_scalar(None)?;
    assert_eq!(value.get_key_str("symbol").and_then(Scalar::as_str), Some("AAPL"));
    // ...and a coded handle presents the decoded bytes to every byte call.
    let decoded = Coded::infer(stored);
    assert_eq!(decoded.read_all_bytes()?, br#"{"symbol":"AAPL","quantity":2}"#);

    // One payload at a time, each codec reads its own frames and refuses anything else.
    assert_eq!(Codec::Gzip.load(&gzip::dump(b"AAPL,1\n")?)?, b"AAPL,1\n");
    assert!(zstd::load(b"definitely not a compressed payload").is_err());
    ```

=== "Python"

    ```python
    import gzip as standard
    import pathlib
    import tempfile

    import pytest

    from yggdryl import IOBase, gzip, zstd
    from yggdryl.holder import LocalPath

    root = pathlib.Path(tempfile.mkdtemp())

    # Another writer's gzip, under a name that declares it: the handle reads
    # the decoded bytes through the same calls as an uncoded one.
    (root / "trade.json.gz").write_bytes(standard.compress(b'{"symbol":"AAPL","quantity":2}'))
    handle = IOBase(root / "trade.json.gz")
    assert handle.read_scalar() == {"quantity": 2, "symbol": "AAPL"}
    assert handle.read_bytes() == b'{"symbol":"AAPL","quantity":2}'
    # LocalPath addresses the stored bytes instead, coding and all.
    assert LocalPath(root / "trade.json.gz").read_bytes()[:2] == bytes.fromhex("1f8b")

    # One payload at a time, each codec reads its own frames and refuses anything else.
    assert gzip.loads(standard.compress(b"AAPL,1\n")) == b"AAPL,1\n"
    with pytest.raises(ValueError):
        zstd.loads(b"definitely not a compressed payload")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const standard = require('node:zlib')
    const { IOBase, gzip, zstd } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))

    // Another writer's gzip, under a name that declares it: the handle reads
    // the decoded value through the same calls as an uncoded one.
    fs.writeFileSync(path.join(root, 'trade.json.gz'), standard.gzipSync('{"symbol":"AAPL","quantity":2}'))
    assert.deepEqual(new IOBase(path.join(root, 'trade.json.gz')).readScalar(), { quantity: 2, symbol: 'AAPL' })

    // One payload at a time, each codec reads its own frames and refuses anything else.
    assert.equal(gzip.loads(standard.gzipSync('AAPL,1\n')).toString(), 'AAPL,1\n')
    assert.throws(() => zstd.loads(Buffer.from('definitely not a compressed payload')))

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Write

A write encodes as it streams, at the `level` the options carry, and publishes the frames the name declares; the same calls write a coded and an uncoded handle. One payload encodes through `dump` / `dumps` - at a level with `dump_with_level`, or the `level` argument in the bindings - and a stream through `writer`.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::arrow;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::ipc::Ipc;
    use yggdryl::{DataType, Level, StructType, Url};

    let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?).required_field("row");
    let arrow_schema = schema.clone().into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![Arc::new(Int64Array::from((0..512).collect::<Vec<i64>>()))],
    )?;

    let mut stored = Vec::new();
    for name in ["trades.arrows", "trades.arrows.gz", "trades.arrows.zst"] {
        let url = Url::from_str(&format!("file:///{name}"))?;
        // A handle whose name declares no coding ignores the level.
        let mut media = Ipc::new(Buffer::new().with_media_type(url.media_type()))
            .with_field(schema.clone())
            .with_level(Level::BEST);
        let options = media.record_options()?;
        media.overwrite_arrow_reader(
            arrow::batch_reader(Arc::clone(&arrow_schema), [batch.clone()]),
            &options,
        )?;

        // Identical calls on both sides, whatever the coding is.
        assert_eq!(media.read_arrow_reader(&options)?.count(), 1, "{name}");
        stored.push(media.handle().as_slice().to_vec());
    }

    // The bytes underneath are framed by the coding the name declared, and
    // each coded member is smaller than the stream it encodes.
    assert_eq!(&stored[1][..2], &[0x1F, 0x8B]);
    assert_eq!(&stored[2][..4], &[0x28, 0xB5, 0x2F, 0xFD]);
    assert!(stored[1].len() < stored[0].len() && stored[2].len() < stored[0].len());
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase
    from yggdryl.holder import LocalPath

    schema = pa.schema([pa.field("id", pa.int64(), nullable=False)])
    batch = pa.record_batch({"id": list(range(512))}, schema=schema)
    root = pathlib.Path(tempfile.mkdtemp())

    stored = []
    for name in ("trades.arrows", "trades.arrows.gz", "trades.arrows.zst"):
        handle = IOBase(root / name)
        # A handle whose name declares no coding ignores the level.
        options = handle.record_options()
        options.level = 9
        handle.overwrite_arrow_batch(batch, options=options)

        # Identical calls on both sides, whatever the coding is.
        assert handle.read_arrow_reader().read_all().num_rows == 512, name
        # The handle presents the decoded stream, so its bytes are the stream.
        assert handle.read_bytes()[:4] == bytes.fromhex("ffffffff"), name
        # LocalPath addresses the stored bytes instead, coding and all.
        stored.append(LocalPath(root / name).read_bytes())

    # The bytes underneath are framed by the coding the name declared, and
    # each coded member is smaller than the stream it encodes.
    assert stored[1][:2] == bytes.fromhex("1f8b")
    assert stored[2][:4] == bytes.fromhex("28b52ffd")
    assert len(stored[1]) < len(stored[0]) and len(stored[2]) < len(stored[0])
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const ids = Array.from({ length: 512 }, (_, index) => BigInt(index))
    const written = []
    for (const name of ['trades.arrows', 'trades.arrows.gz', 'trades.arrows.zst']) {
      const handle = new IOBase(path.join(root, name))
      // A handle whose name declares no coding ignores the level.
      handle.overwriteArrowTable(
        new arrow.Table({ id: arrow.vectorFromArray(ids, new arrow.Int64()) }),
        handle.recordOptions().withLevel(9),
      )

      // Identical calls on both sides, whatever the coding is.
      assert.equal(handle.readArrowReader().intoTable().numRows, 512, name)
      written.push(handle.readBytes())
    }

    // The bytes underneath are framed by the coding the name declared, and
    // each coded member is smaller than the stream it encodes.
    assert.deepEqual([...written[1].subarray(0, 2)], [0x1f, 0x8b])
    assert.deepEqual([...written[2].subarray(0, 4)], [0x28, 0xb5, 0x2f, 0xfd])
    assert.ok(written[1].length < written[0].length && written[2].length < written[0].length)

    fs.rmSync(root, { recursive: true, force: true })
    ```

## gzip

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

## zlib

`*_raw` is DEFLATE without the zlib header and trailer.

=== "Rust"

    ```rust
    use yggdryl::zlib;

    let text = "symbol,price\n".to_string() + &"AAPL,1\n".repeat(64);
    let plain = text.as_bytes();

    let framed = zlib::dump(plain)?;
    let raw = zlib::dump_raw(plain)?;

    assert_eq!(zlib::load(&framed)?, plain);
    assert_eq!(zlib::load_raw(&raw)?, plain);
    assert!(framed.len() < plain.len());

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
    assert len(framed) < len(plain)

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
    assert.ok(framed.length < plain.length)

    // The framing is a two-byte header plus a four-byte Adler-32 trailer.
    assert.equal(framed.length, raw.length + 6)

    // Neither decoder accepts the other's bytes.
    assert.throws(() => zlib.loads(raw))
    assert.throws(() => zlib.loadsRaw(framed))

    // The raw pair is what node:zlib spells inflateRaw.
    assert.deepEqual(standard.inflateRawSync(raw), plain)
    ```

## zstd

=== "Rust"

    ```rust
    use yggdryl::zstd;

    let frame = zstd::dump(b"symbol,price\nAAPL,1\n")?;
    assert_eq!(zstd::load(&frame)?, b"symbol,price\nAAPL,1\n");

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

    frame = zstd.dumps(b"symbol,price\nAAPL,1\n")
    assert zstd.loads(frame) == b"symbol,price\nAAPL,1\n"

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

    const line = Buffer.from('symbol,price\nAAPL,1\n')
    assert.deepEqual(zstd.loads(zstd.dumps(line)), line)

    // Repetition is what zstd removes.
    const payload = Buffer.from('AAPL,1\n'.repeat(64))
    const frame = zstd.dumps(payload)
    assert.ok(frame.length < payload.length)

    // Framing costs bytes, so a short payload comes out larger than it went in.
    assert.ok(zstd.dumps(Buffer.from('AAPL,1\n')).length > 7)

    // A payload that is not a frame is reported, not silently returned.
    assert.throws(() => zstd.loads(Buffer.from('definitely not a compressed payload')))
    ```

## Performance

### gzip performance

One containerized x86_64 Linux run of the Python binding against the standard library's `gzip`, over 1,080,000 bytes of JSON lines.

```text
gzip encode (yggdryl)      0.362 ms   2848.0 MiB/s
gzip encode (stdlib gzip)  3.003 ms    343.0 MiB/s
gzip decode (yggdryl)      0.253 ms   4065.4 MiB/s
gzip decode (stdlib gzip)  0.396 ms   2597.8 MiB/s
```

`zlib-rs` puts the encode 8x ahead; [zlib](#zlib-performance) and [zstd](#zstd-performance) carry their rows from the same run.

```bash
python/.venv/bin/python python/benchmarks/coding.py --min-time 0.2 --repeat 5
```

### zlib performance

`python/benchmarks/coding.py` times `zlib-rs` beside the standard library's zlib over 1,080,000 bytes of JSON lines, one containerized x86_64 Linux run, same wire format. `zlib-rs` puts the encode 9x ahead.

```text
zlib encode (yggdryl)      0.344 ms   2998.3 MiB/s
zlib encode (stdlib zlib)  3.177 ms    324.2 MiB/s
zlib decode (yggdryl)      0.234 ms   4401.1 MiB/s
zlib decode (stdlib zlib)  0.484 ms   2127.9 MiB/s
```

`zlib-rs` level 6 trades a little ratio for speed on repetitive payloads; raise the level when size matters. [gzip](#gzip-performance) and [zstd](#zstd-performance) share this run.

```bash
python/.venv/bin/python python/benchmarks/coding.py --min-time 0.2 --repeat 5
```

### zstd performance

One containerized x86_64 Linux run of the Python binding (CPython 3.11) over 1,080,000 bytes of JSON lines.

```text
zstd encode (yggdryl)     14.879 ms     69.2 MiB/s
zstd decode (yggdryl)      0.358 ms   2877.3 MiB/s
```

Standard-library rows need `compression.zstd` (Python 3.14+); on 3.11 the script prints `stdlib compression.zstd unavailable on this interpreter; skipped`.

```bash
python/.venv/bin/python python/benchmarks/coding.py --min-time 0.2 --repeat 5
```
