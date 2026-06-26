# Compression

Streamed byte compression — **none**, **gzip**, **deflate** (zlib), **Zstandard**,
**Snappy** and **Brotli** — layered on top of [Byte IO](io.md). A `Compression`
codec wraps any [`Io`](io.md) handle to compress and decompress **a chunk at a
time**, never buffering the whole payload. The codecs ship **on by default**; opt
out with `default-features = false` (Rust) for a codec-free build, where an unbuilt
codec still parses and names itself but reports `Unsupported` on encode/decode.

## The codec

`Compression` is `none` / `gzip` / `deflate` / `zstd` / `snappy` / `brotli`.
`from_str` accepts the canonical name or a short alias (`gz`, `zz`/`zlib`, `zst`,
`snap`/`sz`, `br`); `from_extension` infers from a file suffix. Each codec reports
its canonical `name` (`as_str` in Rust), conventional `extension`, and whether its
backend `is_available`.

=== "Python"

    ```python
    import yggdryl

    codec = yggdryl.Compression("gzip")     # also: gz, deflate/zlib, zstd, snappy, brotli
    assert codec.name == "gzip"
    assert codec.extension == "gz"
    assert codec.is_available is True

    packed = codec.compress(b"hello hello hello")
    assert codec.decompress(packed) == b"hello hello hello"
    ```

=== "Node"

    ```javascript
    const { Compression } = require("yggdryl");

    const codec = Compression.fromStr("gzip"); // or: new Compression("gzip")
    console.log(codec.name);        // "gzip"
    console.log(codec.extension);   // "gz"
    console.log(codec.isAvailable); // true

    const packed = codec.compress(Buffer.from("hello hello hello"));
    // codec.decompress(packed) -> the original bytes
    ```

=== "Rust"

    ```rust
    use yggdryl_core::Compression;

    let codec = Compression::from_str("gzip")?; // gz, zz/zlib, zst, snap/sz, br aliases
    assert_eq!(codec.as_str(), "gzip");
    assert_eq!(codec.extension(), Some("gz"));
    assert!(codec.is_available());

    let packed = codec.compress(b"hello hello hello")?;
    assert_eq!(codec.decompress(&packed)?, b"hello hello hello");
    ```

!!! note "deflate is the HTTP `Content-Encoding` token"
    The `deflate` / `zlib` / `zz` names all select the **zlib** codec (RFC 1950, a
    DEFLATE stream with a zlib wrapper) — which is what servers send for
    `Content-Encoding: deflate` in common practice. It names itself `deflate` with
    extension `zz`. gzip and deflate share the pure-Rust **zlib-rs** backend (via
    `flate2`), so there is no C dependency.

!!! tip "Brotli has no magic bytes"
    Brotli's HTTP token is `br`. Because it has no recognisable header, it is
    detected by the `.br` extension or `application/x-brotli` MIME only — never by
    content sniffing.

## One-shot compress / decompress

`compress` / `decompress` are the `&[u8] -> Vec<u8>` (bytes -> bytes) conveniences
built on the streaming encoder/decoder over an in-memory buffer. `none` is the
identity codec — bytes pass through unchanged.

=== "Python"

    ```python
    import yggdryl

    codec = yggdryl.Compression("zstd")
    payload = b"a long, very repetitive payload " * 64
    packed = codec.compress(payload)
    assert codec.decompress(packed) == payload

    # the identity codec
    assert yggdryl.Compression("none").compress(payload) == payload
    ```

=== "Node"

    ```javascript
    const { Compression } = require("yggdryl");

    const codec = Compression.fromStr("zstd");
    const payload = Buffer.from("a long, very repetitive payload ".repeat(64));
    const packed = codec.compress(payload);
    // codec.decompress(packed) -> payload
    ```

=== "Rust"

    ```rust
    use yggdryl_core::Compression;

    let codec = Compression::from_str("zstd")?;
    let payload = b"a long, very repetitive payload ".repeat(64);
    let packed = codec.compress(&payload)?;
    assert_eq!(codec.decompress(&packed)?, payload);
    ```

!!! warning "Decompressing untrusted input"
    The decoded size is unbounded — a small hostile input can expand greatly (a
    "zip bomb"). For untrusted data, cap or stream it through the streaming decoder
    below rather than decoding it whole.

## Streaming over `Io`

`encoder(sink) -> Encoder` and `decoder(source) -> Decoder` are themselves
**streamed [`Io`](io.md) handles**, so they compose straight into a file, a
[`BytesIO`](io.md), or an HTTP body without buffering the whole payload. Write
through the encoder to compress on the way out — then call `finish()` to flush the
trailer and recover the sink. Read through the decoder to decompress on the way in.
(The streamed path measures identical to the one-shot path — zero abstraction
overhead.)

The streaming `Encoder` / `Decoder` adapters are the Rust core surface; the
bindings expose streaming through the [`CompressIo`](#compressio-extension) handle
methods below.

=== "Python"

    ```python
    import yggdryl

    # Stream a BytesIO handle through the codec (see CompressIo below).
    src = yggdryl.BytesIO(b"a long, very repetitive payload " * 64)
    packed = src.compress("zstd")          # -> a fresh BytesIO
    assert packed.decompress("zstd").getvalue() == b"a long, very repetitive payload " * 64
    ```

=== "Node"

    ```javascript
    const { BytesIO } = require("yggdryl");

    const src = new BytesIO(Buffer.from("a long, very repetitive payload ".repeat(64)));
    const packed = src.compress("zstd");   // -> a fresh BytesIO
    // packed.decompress("zstd").getValue() -> the original bytes
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{BytesIO, Compression, Io, Whence};

    // Compress into a BytesIO sink, finish() to flush the trailer…
    let mut encoder = Compression::Zstd.encoder(BytesIO::new())?;
    encoder.write_all(b"a long, very repetitive payload ")?;
    let mut sink = encoder.finish()?;
    sink.seek(0, Whence::Start)?;

    // …then decompress straight out of that handle.
    let mut decoder = Compression::Zstd.decoder(sink)?;
    let mut out = Vec::new();
    decoder.read_to_end(&mut out)?;
    ```

## `CompressIo` extension

Every [`Io`](io.md) handle gains `compress(codec)` / `decompress(codec)`, each
returning a fresh in-memory [`BytesIO`](io.md). They stream the handle from its
cursor through the codec, so a large file is never materialised whole. Called with
**no codec**, `decompress` **infers** one — from the handle's URL extension first
(always available), then its discovered media type (magic-byte sniff for an
in-memory buffer, file name for a path), then its `stats()` content type.

=== "Python"

    ```python
    import yggdryl

    # Compress any handle (BytesIO or LocalPath) -> a fresh BytesIO.
    packed = yggdryl.BytesIO(b"data" * 1000).compress("gzip")

    # A `.gz` path: decompress() with no codec infers gzip from the extension.
    yggdryl.LocalPath("data.txt.gz").decompress().getvalue()

    # An in-memory buffer has no extension -> the codec is sniffed from magic bytes.
    yggdryl.BytesIO(packed.getvalue()).decompress().getvalue()
    ```

=== "Node"

    ```javascript
    const { BytesIO, LocalPath } = require("yggdryl");

    const packed = new BytesIO(Buffer.from("data".repeat(1000))).compress("gzip");

    // `.gz` path: decompress() infers gzip from the extension.
    new LocalPath("data.txt.gz").decompress().getValue();

    // In-memory buffer: codec sniffed from magic bytes.
    new BytesIO(packed.getValue()).decompress().getValue();
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{BytesIO, Compression, CompressIo};

    // `compress` / `decompress` are blanket-added to every Io handle.
    let mut data = BytesIO::from_bytes(b"a long, very repetitive payload ".repeat(64));
    let mut packed = data.compress(Compression::Zstd)?;  // -> a fresh BytesIO
    let out = packed.decompress(None)?;                  // None = infer the codec
    ```

## Inferring a codec

Beyond `from_str` / `from_extension`, a codec can be inferred from a single
[`MimeType`](media.md), a layered [`MediaType`](media.md) stack (its outermost,
container MIME), or an `Io`'s `stats()`. The inverse `mime()` names the MIME a
codec is carried as — use it to add an encoding layer to a media type (`null` for
`none` / `deflate` / `snappy`, which have no registered MIME). In Rust this group
(`from_mime` / `from_media` / `from_stats` / `mime`) lives behind the `media`
feature.

=== "Python"

    ```python
    import yggdryl

    yggdryl.Compression.from_mime(yggdryl.MimeType("application/gzip")).name  # "gzip"
    yggdryl.Compression.from_media(yggdryl.MediaType.from_str("csv.gz")).name # "gzip"
    yggdryl.Compression.from_stats(yggdryl.LocalPath("data.csv.gz").stats()).name # "gzip"
    yggdryl.Compression("gzip").mime().mime  # "application/gzip" (the inverse)
    ```

=== "Node"

    ```javascript
    const { Compression, MimeType, MediaType, LocalPath } = require("yggdryl");

    Compression.fromMime(new MimeType("application/gzip")).name;     // "gzip"
    Compression.fromMedia(MediaType.fromStr("csv.gz")).name;         // "gzip"
    Compression.fromStats(new LocalPath("data.csv.gz").stats()).name; // "gzip"
    new Compression("gzip").mime().mime;                             // "application/gzip"
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{Compression, MediaType, MimeType};

    assert_eq!(Compression::from_mime(&MimeType::Gzip), Some(Compression::Gzip));
    let media = MediaType::from_str("data.csv.gz")?;
    assert_eq!(Compression::from_media(&media), Some(Compression::Gzip));
    assert_eq!(Compression::Gzip.mime(), Some(MimeType::Gzip)); // the inverse
    ```

## See also

- [Byte IO](io.md) — the `Io` handle compression streams over.
- [Media types](media.md) — the `MimeType` / `MediaType` used to infer a codec.
- [Request & Response](../http/request-response.md) — `Content-Encoding` decoded
  transparently through these codecs.
