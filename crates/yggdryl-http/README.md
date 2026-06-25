# yggdryl-http

A small **blocking HTTP client** for the
[**yggdryl**](https://github.com/Platob/yggdryl) project, shaped after Python's
[`requests`](https://requests.readthedocs.io/) — a connection-pooling
`HttpSession`, a builder `HttpRequest`, and an `HttpResponse` whose body
**streams over the [`yggdryl-io`](../yggdryl-io) abstraction** instead of being
eagerly buffered.

## What it offers

- `HttpSession` — like `requests.Session`: reuses connections, carries default
  headers, a retry policy, `max_concurrency` (8) and `batch_size` (80). Every send
  goes through `prepare` (merge defaults; per-request headers win) then
  `request(req, raise_error)` — the verbs (`get` / `post` / …) raise on a 4xx/5xx
  by default. `stream` opens an `HttpStream`; `send_many` runs an iterator of
  requests concurrently in batches.
- `HttpRequest` — a builder: `with_header` / `with_param` (query string) /
  `with_body` (bytes) / `with_body_reader` / `with_body_io`. `with_body_io`
  streams the upload from any `Io` handle and frames it with `Content-Length` from
  the handle's length — a file is never buffered.
- `HttpResponse` — `status` / `ok` / `raise_for_status` / `headers` / `header` /
  `content_type`. The body is lazy: `reader()` is a `ReadBytes` stream, while
  `bytes()` / `text()` / `into_bytesio()` drain it.
- `HttpStream: Io` — a **seekable, lazily-fetched remote handle**. A `HEAD` makes
  its size / content type discoverable, then bytes come in 4 MiB windows via
  `Range` requests (`read` for sequential, `pread` for a one-off footer read).
  Reads retry transient statuses (`Retry-After`-aware) and **resume from the
  cursor** on a dropped connection.
- `RetryConfig` — `max_retries` / `base_delay` / `max_delay`, retrying 429 / 502 /
  503 / 504 and lost connections with capped exponential backoff.

```rust,no_run
use yggdryl_http::{HttpSession, HttpRequest};
use yggdryl_io::{Io, LocalPath, Whence};

let session = HttpSession::new().with_user_agent("yggdryl-http/0.1");

// Verb helpers raise on a 4xx/5xx; pass raise_error=false to keep the response.
let body = session.get("https://example.com").unwrap().text().unwrap();

// Stream an upload straight from a file (Content-Length framed, never buffered).
let response = session
    .request(
        HttpRequest::put("https://example.com/up")
            .unwrap()
            .with_body_io(LocalPath::open("big.bin")),
        false,
    )
    .unwrap();

// A seekable remote Io: read a footer with a single range request.
let mut stream = session.stream(HttpRequest::get("https://example.com/data.parquet").unwrap()).unwrap();
let mut footer = [0u8; 8];
stream.pread(&mut footer, -8, Whence::End).unwrap();
```

The response body is a `ReadBytes` source, so it composes with the rest of the
ecosystem — `copy` it into a `BytesIO`/`LocalPath`, feed it through a `Frames`
codec, parse it with `Io::json()`, or (under `compression`) let a
`Content-Encoding` decode transparently.

## Features (off by default)

- `compression` — transparently decode a gzip / zstd / snappy `Content-Encoding`
  response body via `yggdryl-compression`, the way `requests` auto-decompresses.
- `media` — expose the response's `mime_type()` (a `yggdryl-media` `MimeType`).
- `log` — structured `log` events on the request path.

The transport is [`ureq`](https://docs.rs/ureq) (blocking, `rustls` TLS); its own
gzip/brotli are left off so decompression goes through our abstraction.
