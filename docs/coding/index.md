# Coding

`Codec` names a content coding; `coding::Coded` applies it over any `IOBase` handle so downstream code sees plain bytes.

## Contract

| | |
| --- | --- |
| Owns | `Codec`, `Level`, `coding::Coded`; the bytes live in [gzip](gzip.md), [zlib](zlib.md), [zstd](zstd.md) |
| Codings | `Identity`, `Gzip`, `Zlib`, `Deflate`, `Zstd`; four `Coded` variants |
| Select | `Coded::infer` from the media type; `Coded::wrap` from a `Codec` |
| Deflate | No framing to detect, so it wraps as the zlib handle |
| Level | `with_level`; `Identity` ignores it |
| Composes | Any [`IOBase`](../holder/index.md), `Holder` or another coded handle; `Coded` is itself an `IOBase` |
| Seek | None; the decoded value is materialized once and held until `close` |
| Commit | `flush`, `close`, or `into_handle`; never `pwrite` |
| Media type | Decoded, coding removed |
| Bindings | Rust only; Python and JavaScript use [`IOBase.codec`, `compress_into`, `decompress_into`](../holder/iobase/bytes.md) and per-codec `loads`/`dumps` |

## Use

Rust only. A compound [filename](../uri/path.md) declares the coding, so `Coded::infer` needs nothing else.

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

## Pages

| Page | Purpose |
| --- | --- |
| [gzip](gzip.md) | RFC 1952 gzip |
| [zlib](zlib.md) | RFC 1950 zlib and raw DEFLATE |
| [zstd](zstd.md) | RFC 8878 Zstandard |

## Wrap and publish

`Coded::wrap` names the coding when the handle does not.

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

## Raw DEFLATE

Four variants serve five codings.

=== "Rust"

    ```rust
    use yggdryl::coding::Coded;
    use yggdryl::holder::Buffer;

    let handle = Coded::wrap(Buffer::new(), yggdryl::Codec::Deflate);
    assert_eq!(handle.codec(), yggdryl::Codec::Zlib);
    ```

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

## Edges

- `Codec::Deflate` -> `Coded::wrap` answers `Codec::Zlib`; no raw handle exists.
- `Codec::Identity` -> pass-through; `with_level` changes nothing.
- `into_handle` -> publishes first; an encode or write failure is the `Err`.

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
