# Quickstart

The least code adaptation to get Rust-speed IO, compression and HTTP in your
language.

## Install

=== "Python"

    ```bash
    pip install yggdryl   # or, from this repo: (cd bindings/python && maturin develop)
    ```

=== "Node"

    ```bash
    npm install yggdryl   # or, from this repo: (cd bindings/node && npm run build)
    ```

=== "Rust"

    ```toml
    # Cargo.toml — pull only the crates you need
    yggdryl-core = "0.1"
    yggdryl-http = "0.1"
    ```

## HTTP — like `requests` / `fetch`

=== "Python"

    ```python
    import yggdryl
    s = yggdryl.HttpSession()
    r = s.get("https://httpbin.org/json")
    print(r.status, r.headers["content-type"])
    data = r.json()          # parsed in Rust
    ```

=== "Node"

    ```javascript
    const { HttpSession } = require("yggdryl");
    const s = new HttpSession();
    const r = await s.get("https://httpbin.org/json");
    console.log(r.status, r.header("content-type"));
    ```

=== "Rust"

    ```rust
    use yggdryl_http::HttpSession;
    let r = HttpSession::new().get("https://httpbin.org/json")?;
    println!("{} {:?}", r.status(), r.content_type());
    ```

## Compression — gzip / zstd / snappy

=== "Python"

    ```python
    import yggdryl
    gz = yggdryl.Compression.from_str("zstd")   # also "gzip", "snappy"
    packed = gz.compress(b"hello " * 1000)
    assert gz.decompress(packed) == b"hello " * 1000
    ```

=== "Node"

    ```javascript
    const { Compression } = require("yggdryl");
    const gz = Compression.fromStr("zstd");
    const packed = gz.compress(Buffer.from("hello ".repeat(1000)));
    ```

=== "Rust"

    ```rust
    use yggdryl_core::Compression;
    let gz = Compression::from_str("zstd")?;
    let packed = gz.compress(b"hello world")?;
    ```

## Byte IO — read a remote footer without downloading the file

=== "Python"

    ```python
    import yggdryl
    # A seekable HTTP body: read the last 16 bytes with one Range request.
    stream = yggdryl.HttpSession().get("https://example.com/big.parquet")
    # the Rust core also exposes BytesIO / LocalPath with the same Io surface
    ```

=== "Rust"

    ```rust
    use yggdryl_http::{HttpSession, HttpRequest};
    use yggdryl_core::{Io, Whence};
    let mut s = HttpSession::new()
        .send(HttpRequest::get("https://example.com/big.parquet")?, false, true, true)?
        .into_io();
    let mut footer = [0u8; 16];
    s.pread(&mut footer, -16, Whence::End)?;   // one Range request, no full download
    ```

See the per-component guides for the full surface:
[Byte IO](io.md) · [Compression](compression.md) · [HTTP](http.md).
