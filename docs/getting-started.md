# Getting started

The least code to get Rust-speed IO, media detection, compression and HTTP in your
language. Every snippet below comes in **Python / Node / Rust** — pick one tab and
the whole page follows.

## Install

=== "Python"

    ```bash
    pip install yggdryl
    # from this repo: (cd bindings/python && maturin develop)
    ```

=== "Node"

    ```bash
    npm install yggdryl
    # from this repo: (cd bindings/node && npm run build)
    ```

=== "Rust"

    ```toml
    # Cargo.toml — pull only the crates you need
    [dependencies]
    yggdryl-core = "0.1"   # Io, media, url, compression
    yggdryl-http = "0.1"   # the HTTP client
    ```

## HTTP — like `requests` / `fetch`

=== "Python"

    ```python
    import yggdryl

    session = yggdryl.HttpSession()
    response = session.get("https://httpbin.org/json")
    print(response.status, response.header("content-type"))
    data = response.json()      # parsed in Rust, no FFI copy
    ```

=== "Node"

    ```javascript
    const { HttpSession } = require("yggdryl");

    const session = new HttpSession();
    const response = await session.get("https://httpbin.org/json");
    console.log(response.status, response.header("content-type"));
    const data = response.json();   // parsed in Rust
    ```

=== "Rust"

    ```rust
    use yggdryl_http::HttpSession;

    let session = HttpSession::new();
    let response = session.get("https://httpbin.org/json", true)?; // true = send now
    println!("{} {:?}", response.status(), response.content_type());
    let data = response.json()?;    // parsed off the streamed body
    ```

The verb helpers (`get` / `post` / `delete` / `request` / …) build and send a
request in one call. Pass **`send = false`** to build it *without* dispatching and
get an **unsent** response back, carrying the prepared request — see
[Request & Response](http/request-response.md).

## Compression — gzip / deflate / zstd / snappy / brotli

=== "Python"

    ```python
    import yggdryl

    codec = yggdryl.Compression("zstd")     # also gzip, deflate, snappy, brotli
    packed = codec.compress(b"hello " * 1000)
    assert codec.decompress(packed) == b"hello " * 1000
    ```

=== "Node"

    ```javascript
    const { Compression } = require("yggdryl");

    const codec = Compression.fromStr("zstd");
    const packed = codec.compress(Buffer.from("hello ".repeat(1000)));
    // codec.decompress(packed) -> the original bytes
    ```

=== "Rust"

    ```rust
    use yggdryl_core::Compression;

    let codec = Compression::from_str("zstd")?;
    let packed = codec.compress(b"hello world")?;
    assert_eq!(codec.decompress(&packed)?, b"hello world");
    ```

## Media types — from an extension or magic bytes

=== "Python"

    ```python
    import yggdryl

    assert yggdryl.MimeType.from_extension("parquet").mime == "application/vnd.apache.parquet"
    # A layered stack: app.tgz is tar + gzip.
    assert [t.mime for t in yggdryl.MediaType.from_path("app.tgz").types] == [
        "application/x-tar", "application/gzip",
    ]
    ```

=== "Node"

    ```javascript
    const { MimeType, MediaType } = require("yggdryl");

    MimeType.fromExtension("parquet").mime; // "application/vnd.apache.parquet"
    MediaType.fromPath("app.tgz").types.map((t) => t.mime);
    // ["application/x-tar", "application/gzip"]
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{MediaType, MimeType};

    assert_eq!(MimeType::from_extension("parquet"), Some(MimeType::Parquet));
    assert_eq!(
        MediaType::from_path("app.tgz").types(),
        [MimeType::Tar, MimeType::Gzip],
    );
    ```

## Byte IO — read a remote footer without downloading the file

=== "Python"

    ```python
    import yggdryl

    # The HTTP response is a seekable Io: read the last 8 bytes with one Range request.
    response = yggdryl.HttpSession().get("https://example.com/big.parquet")
    handle = response.io          # a Rust-backed BytesIO (no native copy)
    # BytesIO / LocalPath expose the same Io surface (read / seek / pread).
    ```

=== "Node"

    ```javascript
    const { HttpSession } = require("yggdryl");

    const response = await new HttpSession().get("https://example.com/big.parquet");
    const handle = response.io;   // a Rust-backed BytesIO (no native copy)
    ```

=== "Rust"

    ```rust
    use yggdryl_http::HttpSession;
    use yggdryl_core::{Io, Whence};

    let mut body = HttpSession::new()
        .get("https://example.com/big.parquet", true)?
        .into_io();
    let mut footer = [0u8; 8];
    body.pread(&mut footer, -8, Whence::End)?;  // one Range request, no full download
    ```

## Next

Read the per-component guides — each mirrors a module of the code:

- **Core** — [Version](core/version.md) · [Media types](core/media.md) ·
  [URI & URL](core/url.md) · [Byte IO](core/io.md) · [Compression](core/compression.md)
- **HTTP** — [Session](http/session.md) · [Request & Response](http/request-response.md)
  · [Streaming body](http/stream.md) · [Cookies](http/cookies.md)
