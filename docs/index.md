# yggdryl

**One streaming byte-IO core — `Io`, media types, compression and HTTP — for
Python, Node and Rust.** Write the same high-level code in any of the three
languages and get near-Rust throughput: all the work happens in a dependency-light
Rust core, and the bindings are thin wrappers that never copy your bytes through
the host language.

=== "Python"

    ```python
    # Looks like requests, runs in Rust.
    import yggdryl
    session = yggdryl.HttpSession()
    data = session.get("https://example.com/data.csv.gz").content
    ```

=== "Node"

    ```javascript
    // Looks like fetch/axios, runs in Rust.
    const { HttpSession } = require("yggdryl");
    const data = (await new HttpSession().get("https://example.com/data.csv.gz")).content;
    ```

=== "Rust"

    ```rust
    // The core.
    use yggdryl_http::HttpSession;
    let data = HttpSession::new().get("https://example.com/data.csv.gz", true)?.bytes()?;
    ```

!!! tip "Pick your language once"
    The **Python / Node / Rust** tabs on every page are linked — choose a language
    in any snippet and the whole site follows, so you only ever read your own
    language.

## Why

A reader (think Arrow / Parquet / CSV / JSON) should not care **where** its bytes
live — an in-memory buffer, a memory-mapped file, or a remote HTTP object — nor
whether they arrive **random** (read a footer, a column chunk) or **streamed**
(scan record batches). yggdryl unifies all of that behind **one trait, `Io`**, and
builds media-type detection, compression and a `requests`-like HTTP client on top
of it, each handle composing with the next with **at most one copy** of the data.

- **One trait for all bytes** — [`Io`](core/io.md) carries reads, writes, a cursor
  and positional `pread`/`pwrite`. A memory backend gets zero-copy reads for free;
  a streamed backend (an HTTP body, a decompressor) just implements `read`.
- **Media types** — [`MimeType`/`MediaType`](core/media.md) map extensions and
  magic bytes to a layered type stack (`data.csv.gz` → `[Csv, Gzip]`, `app.tgz` →
  `[Tar, Gzip]`).
- **Streamed compression over `Io`** — gzip / deflate / Zstandard / Snappy /
  Brotli encoders and decoders are themselves [`Io`](core/compression.md) handles,
  so they slot straight into an HTTP body or a file with no buffering of the whole
  payload.
- **A `requests`-like HTTP client** — [connection pooling](http/session.md),
  retries with resume-on-drop, a [seekable response body](http/stream.md) (read a
  footer without downloading the object), concurrent `send_many`,
  [cookies](http/cookies.md) and redirects.
- **Three languages, one surface** — the Rust core is the source of truth; the
  Python (PyO3) and Node (napi-rs) bindings expose the same names and semantics.

## Headline numbers

Measured on one developer machine (localhost, no real network) — see
[Benchmarks](benchmarks.md) for the full tables, organized by theme, and how to
reproduce them.

| workload | yggdryl | baseline | gain |
| --- | --- | --- | --- |
| HTTP GET (8 MiB, throughput, Node) | 1093 MiB/s | `node:http` 770 MiB/s | **1.42×** |
| gzip compress (Node) | 67 MiB/s | `node:zlib` 31 MiB/s | **2.2×** |
| gzip decompress (Node) | 491 MiB/s | `node:zlib` 450 MiB/s | **1.09×** |

…plus `zstd` / `snappy` / `brotli` codecs the Node/Python standard libraries don't
ship at all.

## Next

- **[Getting started](getting-started.md)** — install and the first lines in each language.
- **[Core](core/io.md)** — the `Io` trait, media types, URLs and compression.
- **[HTTP](http/session.md)** — the client, streaming, cookies, redirects.
