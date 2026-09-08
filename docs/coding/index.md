# Coding

`Codec` names a content coding; `coding::Coded` applies it over any `IOBase` handle so downstream code sees plain bytes.

## Pages

| Page | Purpose |
| --- | --- |
| [gzip](gzip.md) | RFC 1952 gzip |
| [zlib](zlib.md) | RFC 1950 zlib and raw DEFLATE |
| [zstd](zstd.md) | RFC 8878 Zstandard |

## Contract

| | |
| --- | --- |
| Owns | `Codec`, `Level`, `coding::Coded`; the bytes live in [gzip](gzip.md), [zlib](zlib.md), [zstd](zstd.md) |
| Codings | `Identity`, `Gzip`, `Zlib`, `Deflate`, `Zstd`; four `Coded` variants |
| Select | `Coded::infer` from the media type; `Coded::wrap` from a `Codec`; `Holder::into_coded` retains either as the handle |
| Deflate | No framing to detect, so it wraps as the zlib handle |
| Level | `with_level`; `Identity` ignores it |
| Composes | Any [`IOBase`](../holder/index.md), `Holder` or another coded handle; `Coded` is itself an `IOBase` |
| Seek | None through `Coded`; the decoded value is materialized once and held until `close` |
| Restart | `Encoder::restart` ends a unit so a decoder can begin at the next byte, `Codec::restarts` scans encoded bytes for those offsets, `Codec::has_restarts` says which codings have any: `Identity` everywhere, `Deflate` after a full flush, `Zstd` at each frame. Gzip and zlib have none - their framing wraps the whole payload - and refuse a restart by name |
| Commit | `flush`, `close`, or `into_handle`; never `pwrite` |
| Media type | Decoded, coding removed |
| Python | `yggdryl.coding.Coded`, with `Identity`, `Gzip`, `Zlib`, `Zstd` under it - the same four handles. A coded name composes one on construction, [`IOBase.into_coded`](../holder/iobase/bytes.md) names the coding otherwise; they are answers, never constructors |
| JavaScript | [`IOBase.codec`, `compressInto`, `decompressInto`](../holder/iobase/bytes.md) and the per-codec `loads`/`dumps` pairs; no coded handle |

## Use

A compound [filename](../uri/path.md) declares the coding, so `Coded::infer` - and the Python construction that composes it - needs nothing else.

=== "Rust"

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

=== "Python"

    ```python
    import gzip
    import pathlib
    import tempfile

    from yggdryl import IOBase
    from yggdryl.coding import Coded, Gzip
    from yggdryl.holder import Path

    root = pathlib.Path(tempfile.mkdtemp())
    path = root / "app.log.gz"
    path.write_bytes(gzip.compress(b"[INFO] alpha\n[WARN] beta\n"))

    # The coding goes underneath the text rows, which read the decoded bytes.
    source = IOBase(path)
    assert repr(source) == f'Text(Gzip(Path("{path.as_uri()}")))'
    assert source.codec == "gzip"
    assert str(source.media_type) == "text/plain"
    assert source.read_text() == "[INFO] alpha\n[WARN] beta\n"

    # Records read the same way, decoding as the batches are pulled.
    assert [row["body"] for row in source.read_records()] == [
        b"[INFO] alpha",
        b"[WARN] beta",
    ]

    # A name declaring nothing but the coding answers the coded handle itself.
    blob = IOBase(root / "archive.bin.gz")
    assert isinstance(blob, Gzip)
    assert isinstance(blob, Coded)

    # `Path` commits to the stored bytes instead, coding and all.
    assert Path(path).read_bytes()[:2] == b"\x1f\x8b"
    ```

## Wrap and publish

`Coded::wrap` - `into_coded` in Python - names the coding when the handle does not.

=== "Rust"

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

=== "Python"

    ```python
    from yggdryl import IOBase
    from yggdryl.coding import gzip
    from yggdryl.holder import Buffer

    # The conversion answers the coded handle and spends the one it took.
    handle = IOBase.from_bytes(b"").into_coded("gzip", level=9)
    handle.write_bytes(b"symbol,price\nAAPL,1\n")

    # into_handle publishes the pending write, then gives back the compressed bytes.
    inner = handle.into_handle()
    assert isinstance(inner, Buffer)
    assert gzip.loads(inner.read_bytes()) == b"symbol,price\nAAPL,1\n"
    ```

## Raw DEFLATE

Four handles serve five codings, in both languages.

=== "Rust"

    ```rust
    use yggdryl::coding::Coded;
    use yggdryl::holder::Buffer;

    let handle = Coded::wrap(Buffer::new(), yggdryl::Codec::Deflate);
    assert_eq!(handle.codec(), yggdryl::Codec::Zlib);
    ```

=== "Python"

    ```python
    from yggdryl import IOBase
    from yggdryl.coding import Zlib

    handle = IOBase.from_bytes(b"").into_coded("deflate")
    assert isinstance(handle, Zlib)
    assert handle.codec == "zlib"
    ```

## Restart points

A stream that only decodes from its first byte cannot answer a read at an offset without decoding everything before it. Two of these codings can do better, and say so: a raw DEFLATE stream restarts after a full flush, which closes the block, aligns to a byte, and drops the window; a Zstandard stream restarts at every frame. What follows a restart decodes on its own, so a map of restart offsets turns a positional read into a decode of one unit.

Restarting costs compression - each unit starts with no history of the one before it - so a caller restarts on a stride it chose, never per write. The [ZIP backend](../holder/backends/zip.md) is what this exists for: it writes members as units and states the map in their records.

```rust
use yggdryl::{Codec, Level};
use std::io::Write;

let mut encoded = Vec::new();
let mut encoder = Codec::Deflate.writer_with_level(&mut encoded, Level::DEFAULT);
encoder.write_all(b"symbol,price\n")?;
encoder.restart()?;
encoder.write_all(b"AAPL,187.23\n")?;
encoder.finish()?;

// The whole stream still decodes as one payload.
assert_eq!(Codec::Deflate.load(&encoded)?, b"symbol,price\nAAPL,187.23\n");

// And the scan finds the one offset a decoder may begin at instead.
let mut offsets = Vec::new();
Codec::Deflate.restart_scan().push(&encoded, &mut offsets);
assert_eq!(offsets.len(), 1);
let start = usize::try_from(offsets[0])?;
assert_eq!(Codec::Deflate.load(&encoded[start..])?, b"AAPL,187.23\n");

// A coding whose framing wraps the payload has none, and says which.
assert!(!Codec::Gzip.has_restarts());
let mut framed = Vec::new();
let mut gzip = Codec::Gzip.writer(&mut framed);
assert!(gzip.restart().is_err());
gzip.finish()?;
```

The scan answers *candidates*: the pattern a coding restarts after can also occur inside compressed data, so a caller that depends on the answer decodes a probe from each offset before trusting it. The stream's own start is never reported - a decoder may always begin there.

## Decoded media type

The wrapper's media type drops the coding; the wrapped handle holds the frame.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::coding::Coded;
    use yggdryl::{Codec, Level, MimeType, Url};

    let inner = Buffer::new().with_media_type(Url::from_str("file:///trades.arrows.gz")?.media_type());
    let mut handle = Coded::wrap(inner, Codec::Gzip).with_level(Level::BEST);

    // The wrapper's bytes are decoded, so its media type has the coding removed.
    assert_eq!(handle.media_type().base(), &MimeType::ARROW_STREAM);
    assert_eq!(handle.media_type().encoding_len(), 0);

    let payload = "symbol,price\n".repeat(64).into_bytes();
    handle.write_all_bytes(&payload)?;
    handle.flush()?;

    // Reads decompress; the wrapped handle only ever holds the encoded form.
    assert_eq!(handle.read_all_bytes()?, payload);
    assert!(handle.handle().size() < payload.len() as u64);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl.holder import Path

    root = pathlib.Path(tempfile.mkdtemp())

    # `Path` skips the composition, so the coding is the only layer retained.
    handle = Path(root / "trades.arrows.gz").into_coded()

    # The handle's bytes are decoded, so its media type has the coding removed.
    assert str(handle.media_type) == "application/vnd.apache.arrow.stream"
    assert not handle.media_type.is_encoded()

    payload = b"symbol,price\n" * 64
    handle.write_bytes(payload)
    handle.flush()

    # Reads decompress; the stored bytes only ever hold the encoded form.
    assert handle.read_bytes() == payload
    assert Path(root / "trades.arrows.gz").size < len(payload)
    ```

## Edges

- `Codec::Deflate` -> `Coded::wrap` answers `Codec::Zlib`; no raw handle exists, and `yggdryl.coding` has no `Deflate`.
- `Codec::Identity` -> pass-through; `with_level` changes nothing, and every offset already begins a unit, so `restart` is a flush and the scan reports nothing.
- `into_handle` -> publishes first; an encode or write failure is the `Err`.
- Python `into_coded` -> the coded handle; the handle it took is spent and raises `ValueError`.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib coding::tests::
    cargo test --features "parquet iceberg" -p yggdryl --lib coding::
    cargo bench -p yggdryl --bench coding -- io_pstream_first
    cargo bench -p yggdryl --bench coding -- io_pstream_drain
    cargo bench -p yggdryl --bench coding -- io_pstream_repeated_pread
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/coding/test_io_codings.py
    python/.venv/bin/python -m pytest python/tests/coding
    python/.venv/bin/python python/benchmarks/coding.py --min-time 0.2 --repeat 5
    ```

=== "JavaScript"

    ```bash
    npm run --prefix node bench:coding
    ```
