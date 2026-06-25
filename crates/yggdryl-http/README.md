# yggdryl-http

A small **blocking HTTP client** for the
[**yggdryl**](https://github.com/Platob/yggdryl) project, shaped after Python's
[`requests`](https://requests.readthedocs.io/) — a connection-pooling
`HttpSession`, a builder `HttpRequest`, and an `HttpResponse` whose body
**streams over the [`yggdryl-io`](../yggdryl-io) abstraction** instead of being
eagerly buffered.

## What it offers

- `HttpSession` — like `requests.Session`: reuses connections, carries default
  headers, and exposes `get` / `head` / `delete` / `post` / `put` / `patch` plus
  `request` for full control. A 4xx/5xx status comes back as a normal response
  (call `raise_for_status` to opt into raising).
- `HttpRequest` — a builder: `with_header` / `with_headers` / `with_param`
  (query string) / `with_body` (bytes) / `with_body_reader` (stream the body
  from any `Io` handle, so a large upload is never buffered).
- `HttpResponse` — `status` / `ok` / `raise_for_status` / `headers` / `header` /
  `content_type` / `content_length`. The body is read lazily: `reader()` is a
  `ReadBytes` stream, while `bytes()` / `text()` / `into_bytesio()` drain it.
- `Method` — `GET` / `POST` / `PUT` / `PATCH` / `DELETE` / `HEAD` / `OPTIONS`.

```rust,no_run
use yggdryl_http::{HttpSession, HttpRequest};

let session = HttpSession::new().with_user_agent("yggdryl-http/0.1");

// requests-style one-liner.
let body = session.get("https://example.com").unwrap().text().unwrap();

// Full control, streaming an upload straight from a file handle.
use yggdryl_io::LocalPath;
let upload = LocalPath::open("big.bin");
let response = session
    .request(HttpRequest::put("https://example.com/up").unwrap().with_body_reader(upload))
    .unwrap()
    .raise_for_status()
    .unwrap();
```

The response body is a `ReadBytes` source, so it composes with the rest of the
ecosystem — `copy` it into a `BytesIO`/`LocalPath`, feed it through a `Frames`
codec, or (under the `compression` feature) let a `Content-Encoding` decode
transparently.

## Features (off by default)

- `compression` — transparently decode a gzip / zstd / snappy `Content-Encoding`
  response body via `yggdryl-compression`, the way `requests` auto-decompresses.
- `media` — expose the response's `mime_type()` (a `yggdryl-media` `MimeType`).
- `log` — structured `log` events on the request path.

The transport is [`ureq`](https://docs.rs/ureq) (blocking, `rustls` TLS); its own
gzip/brotli are left off so decompression goes through our abstraction.
