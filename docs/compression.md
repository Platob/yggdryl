# Compression

Streamed byte compression — **gzip**, **Zstandard** and **Snappy** — layered on
top of [`Io`](io.md). The codecs ship **on by default**; opt out with
`default-features = false` for a codec-free build.

## The codec

`Compression` is `None` / `Gzip` / `Zstd` / `Snappy`:

```rust
use yggdryl_compression::Compression;

let codec = Compression::from_str("zstd")?;      // "gzip"/"gz", "zstd"/"zst", "snappy"/"snap"/"sz"
assert_eq!(codec.as_str(), "zstd");
assert_eq!(codec.extension(), Some("zst"));
let packed = codec.compress(b"hello hello hello")?;
assert_eq!(codec.decompress(&packed)?, b"hello hello hello");
```

It also infers a codec from a file extension, a MIME type, a media stack, or an
`Io`'s `stats()` (`from_extension` / `from_mime` / `from_media` / `from_stats`,
the last three under the `media` feature).

## Streaming over `Io`

`encoder(sink) -> Encoder` and `decoder(source) -> Decoder` are themselves
**streamed `Io` handles**, so they compose straight into a file or an HTTP body
without buffering the whole payload — the `Io`-stream path measures identical to
the one-shot path (zero abstraction overhead).

```rust
use yggdryl_compression::{Compression, CompressIo};
use yggdryl_io::{BytesIO, Io};

// `compress` / `decompress` are added to *every* Io handle.
let mut data = BytesIO::from_bytes(b"a long, very repetitive payload ".repeat(64));
let packed = data.compress(Compression::Zstd)?;   // -> a fresh BytesIO
```

`decompress` with no codec infers one from the handle's URL extension, then its
discovered media / content type.

## Bindings

=== "Python"

    ```python
    gz = yggdryl.Compression.from_str("gzip")
    packed = gz.compress(b"data" * 1000)
    # every BytesIO / LocalPath also has .compress("gzip") / .decompress()
    ```

=== "Node"

    ```javascript
    const gz = Compression.fromStr("gzip");
    const packed = gz.compress(Buffer.from("data".repeat(1000)));
    ```
