# Streaming body

The body of a response is never collected up front — it is a live, seekable
`HttpStream` that **holds the connection** and pulls bytes off the socket on demand.
Sequential reads stream straight through (keeping only a small sliding cache for
short seek-backs), while random access (`pread`) or a seek-back past the cache
re-opens a one-off `Range` request. So you can read a Parquet footer or a column
chunk without ever downloading the whole file, and a dropped connection resumes from
the cursor instead of starting over.

## What it is

A [Request & Response](request-response.md) hands back the body as an `Io` handle:
the `HttpResponse` *is itself* an `Io` (it delegates to its body), and `into_io()`
takes the body out for seekable access. Either way you get the full
[byte-IO surface](../core/io.md): `read` / `seek` / `stream_position` /
`stream_len` / `pread` / `read_to_end` / `close`.

!!! warning "Lazy streaming is a Rust-core capability"
    Partial download, seek-back via `Range`, and resume-on-drop are properties of
    the Rust **`HttpStream`** — it holds the live connection and fetches only what
    you read. The **Python and Node bindings buffer the body** (it is drained
    off-thread, then exposed as `response.io`, a Rust-backed `BytesIO`), so in the
    bindings `pread` / `seek` operate **in memory over the already-downloaded
    body** — the same `Io` surface and no native copy, but not a partial fetch. The
    footer-without-download trick below is therefore a Rust example; the binding
    tabs show the same calls over the buffered body.

!!! note "The canonical remote `Io`"
    `HttpStream` is the network counterpart of `LocalPath`: a `LocalPath` memory-maps
    a file lazily, an `HttpStream` streams a URL lazily. Both implement the one `Io`
    trait, so a reader (Arrow, Parquet) works over either unchanged.

## How it reads

- **Sequential `read`** pulls bytes off the held connection on demand, appending each
  chunk to a sliding 4 MiB cache so a short seek-back is served without a new request.
- **`pread`** (and a seek-back past the cache) issues a one-off `Range` request on a
  pooled connection, leaving the live reader and — for `Whence::Start` / `Whence::End`
  — the cursor untouched. This is the footer read.
- **Retry + resume** — transient statuses (429 / 502 / 503 / 504) are retried honouring
  `Retry-After`, and a connection lost mid-stream is reconnected and **resumed from the
  cursor** (each `Range` request is idempotent).
- **Release** — the connection goes back to the pool the moment the body reaches EOF,
  and `close()` drops it eagerly (idempotent; further reads return EOF).

## Read a footer without downloading the file

`pread` reads positional bytes. In **Rust** this fetches just the trailing window —
one `Range` request, no full download. In the **bindings** the body is already
buffered, so the same `pread` reads from memory. `whence` is `0` = start, `1` =
current, `2` = end (the `io.SEEK_*` convention); in Rust it is the `Whence` enum.

=== "Python"

    ```python
    import yggdryl

    response = yggdryl.get("https://example.com/big.parquet")
    body = response.io                  # Rust-backed BytesIO over the buffered body

    # The Parquet footer ends with an 8-byte trailer (4-byte length + "PAR1").
    trailer = body.pread(8, -8, 2)      # whence=2 (end); reads in memory here
    footer_len = int.from_bytes(trailer[:4], "little")

    footer = body.pread(footer_len, -8 - footer_len, 2)
    print(len(footer), "footer bytes")
    body.close()
    ```

=== "Node"

    ```javascript
    const yggdryl = require("yggdryl");

    const response = await yggdryl.get("https://example.com/big.parquet");
    const body = response.io;            // Rust-backed BytesIO over the buffered body

    // The Parquet footer ends with an 8-byte trailer (4-byte length + "PAR1").
    const trailer = body.pread(8, -8, 2);  // whence 2 = end; reads in memory here
    const footerLen = trailer.readUInt32LE(0);

    const footer = body.pread(footerLen, -8 - footerLen, 2);
    console.log(footer.length, "footer bytes");
    body.close();
    ```

=== "Rust"

    ```rust
    use yggdryl_http::HttpSession;
    use yggdryl_core::{Io, Whence};

    let mut body = HttpSession::new()
        .get("https://example.com/big.parquet", true)? // true = send now
        .into_io();                                    // take the live HttpStream

    // The Parquet footer ends with an 8-byte trailer (4-byte length + "PAR1").
    let mut trailer = [0u8; 8];
    body.pread(&mut trailer, -8, Whence::End)?;        // one Range request, no download
    let footer_len = u32::from_le_bytes(trailer[..4].try_into().unwrap()) as usize;

    let mut footer = vec![0u8; footer_len];
    body.pread(&mut footer, -8 - footer_len as i64, Whence::End)?;
    println!("{} footer bytes", footer.len());
    body.close()?;
    ```

!!! tip "`pread` does not move the cursor"
    A `pread` with whence start/end (`0`/`2`, or `Whence::Start`/`Whence::End`) reads
    positionally and leaves the sequential cursor where it was, so you can read a
    footer and then keep scanning from the front. Use whence current (`1` /
    `Whence::Current`) to read relative to — and advance — the cursor.

## Seeking and resuming

`seek` moves the cursor without I/O; the next `read` re-opens at that offset if it
fell outside the live reader and the cache. A `seek` from the end needs a known size
(the server must have sent `Content-Length`). Because every range request is
idempotent, a mid-stream disconnect is retried and resumed transparently — your read
loop never sees the drop.

=== "Python"

    ```python
    import yggdryl

    body = yggdryl.get("https://example.com/data.bin").io
    body.seek(1024, 0)              # whence 0 = start; jump ahead, cursor only
    chunk = body.read(256)         # reads from byte 1024 (in memory, body buffered)
    print(body.stream_position(), "of", body.stream_len())
    ```

=== "Node"

    ```javascript
    const yggdryl = require("yggdryl");

    const body = (await yggdryl.get("https://example.com/data.bin")).io;
    body.seek(1024, 0);            // whence 0 = start; jump ahead, cursor only
    const chunk = body.read(256);  // reads from byte 1024 (in memory, body buffered)
    console.log(body.streamPosition(), "of", body.streamLen());
    ```

=== "Rust"

    ```rust
    use yggdryl_http::HttpSession;
    use yggdryl_core::{Io, Whence};

    let mut body = HttpSession::new()
        .get("https://example.com/data.bin", true)?
        .into_io();
    body.seek(1024, Whence::Start)?;       // jump ahead; cursor only
    let mut chunk = [0u8; 256];
    body.read_exact(&mut chunk)?;          // re-opens a Range from byte 1024 (live stream)
    println!("{} of {:?}", body.stream_position(), body.stream_len());
    ```

## Draining the whole body

When you do want the entire payload, drain it through the [response](request-response.md)
accessors — `bytes()` / `text()` / `json()` / `into_bytesio()` (Rust) or
`response.content` / `.text()` / `.json()` (bindings). These read the stream to EOF
in one pass, decompress transparently per `Content-Encoding` (see
[Compression](../core/compression.md)), and release the connection. Reach for
`HttpStream` directly only when you need **partial** or **out-of-order** access.

## Notes

- The connection is held until EOF or `close()`; a pool-saturation safeguard forces
  `Connection: close` on extra concurrent streams so a fan-out of `send_many` cannot
  exhaust the pool. See [Session](session.md) for pooling and concurrency.
- A server that ignores `Range` (answers `200` to a non-zero offset) is rejected
  rather than silently corrupting the stream — the error names the cause.
- `stream_len()` reflects the server's reported size; it is learnt lazily for an
  unknown-size body once the stream reaches EOF.
