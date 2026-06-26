# HTTP

A blocking, `requests`-like HTTP client whose bodies **stream over [`Io`](io.md)**.
The transport is `ureq` (rustls TLS, trusting the **OS-native certificate store** by
default); decompression goes through [compression](compression.md), on by default.

## Decoding & content type

`text()` and `json()` (and `bytes()`) **transparently decompress** the body per its
`Content-Encoding` — `gzip` / `zstd` / `snappy` / `brotli` (`br`) — so you always read
the decoded payload. The response also exposes `compression()` (the codec named by
`Content-Encoding`), `mime_type()` / `media_type()` (from `Content-Type`), and
`content_type` / `content_encoding`.

```python
r = yggdryl.HttpSession().get("https://example.com/data.json")  # served `Content-Encoding: br`
r.compression   # "brotli"
r.mime_type     # "application/json"
data = r.json() # decompressed and parsed in Rust
```

## Request in, response out

A **request is the transaction**: build an `HttpRequest`, and it is self-sufficient
to fetch its `HttpResponse`.

```rust
let body = HttpRequest::get("https://example.com")?.send(true)?.text()?;  // raise on 4xx/5xx
```

`request.send(raise_error)` dispatches through the process-wide shared session.
`HttpSession` is the **defaulting factory + transport**: it carries the pool, TLS,
proxy, retry policy and default headers, builds requests with `prepare`, and runs
them with `session.send(request, raise_error)` — use it when you need a
custom-configured client. The body is **always streamed**: `HttpStream` itself
handles buffering and random access (a sliding cache, `Range` requests), so there
is no `stream` flag — drain it with `bytes()` / `text()` / `into_io()`.

```rust
session.send(request, raise_error) -> HttpResponse   // raise on a 4xx/5xx
```

`send_many(requests)` runs an iterator of requests concurrently in batches, lazily.

### Connection reuse & timeouts

Connection reuse is a per-request **keep-alive idle TTL** in seconds:
`request.with_keep_alive(seconds)` (default 300 — 5 minutes) pools the connection
so the next request skips the TLS handshake; `0` sends `Connection: close`,
releasing the socket the moment the body drains. A pooled connection idle past its
TTL is dropped.

`session.with_read_timeout(seconds)` (default 120) errors if the server sends no
data for that long, with a hint to raise it for genuinely slow endpoints; `0`
removes the bound.

Both `HttpRequest` and `HttpSession` have a `copy()` that returns an independent
instance (a session copy gets its own fresh pool and a snapshot of the cookie jar).

```python
import yggdryl
s = yggdryl.HttpSession(headers={"accept": "application/json"})
r = s.get("https://httpbin.org/get")
print(r.status, r.headers, r.sent_at, r.received_at)   # request/response timestamps
```

## Authentication

`with_basic_auth(username, password)` and `with_bearer_auth(token)` set the
`Authorization` header — HTTP Basic (`Basic base64(user:pass)`, RFC 7617) or
Bearer (`Bearer <token>`, RFC 6750). They exist on both `HttpRequest` (per
request) and `HttpSession` (a default on every request, like `requests`'
`Session.auth`). A session-level credential is a default header, so a per-request
`Authorization` overrides it and a **cross-origin redirect strips it** — credentials
never leak to another host.

```python
import yggdryl
# Session-wide credentials (Python kwargs / Node options).
s = yggdryl.HttpSession(basic_auth=("user", "pass"))      # or bearer_auth="token"
s.get("https://httpbin.org/basic-auth/user/pass")
```

```javascript
const { HttpSession } = require("yggdryl");
// basicAuth is a [username, password] pair; bearerAuth is a token.
const opts = Array(9).fill(undefined);
const s = new HttpSession(...opts, ["user", "pass"]); // or (...opts, undefined, "token")
```

## Streaming & random access

`HttpStream` streams straight off the held connection — sequential `read` pulls
bytes on demand, keeping a sliding 4 MiB cache for short seek-backs, while `pread`
(a footer, a column chunk) and seek-backs past the cache re-open a one-off `Range`
request on a pooled connection. A connection lost mid-stream is reconnected and
**resumed from the cursor** (each range request is idempotent, and a `206` is
verified to resume at the byte we asked for).

```rust
// Read a 16-byte footer of a multi-gigabyte object — one Range request.
let mut body = session.send(HttpRequest::get(url)?, false, true, true)?.into_io();
let mut footer = [0u8; 16];
body.pread(&mut footer, -16, Whence::End)?;
```

## Reliability

- **Retries** — 429 / 502 / 503 / 504 with capped exponential backoff (honouring
  `Retry-After`), plus a **single** retry of a `500`. Streamed bodies resume from
  the cursor; replayable bodies (none / bytes) replay in full.
- **Connection pool** — reused keep-alive connections skip the TLS handshake, with
  a saturation safeguard so streaming reads never starve the pool.
- **Cookies** — an RFC 6265 cookie jar parses `Set-Cookie` and sends matching
  `Cookie` headers (domain / path / `Secure`).
- **Redirects** — followed by default (301/302/303 → GET, 307/308 preserve the
  method), with hop limits and loop detection; per-request `allow_redirect`
  toggles it, and auth headers are stripped on a cross-host hop.

## Request bodies stream too

`with_body_io(handle)` uploads straight off any `Io` (a `LocalPath` is never
buffered): the handle's length frames `Content-Length` and the bytes flow off the
handle. The bindings pass `BytesIO` / `LocalPath` handles as bodies — never
serialized `bytes`.
