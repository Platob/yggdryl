# HTTP

A blocking, `requests`-like HTTP client whose bodies **stream over [`Io`](io.md)**.
The transport is `ureq` (rustls TLS); decompression goes through
[compression](compression.md), on by default.

## The session

`HttpSession` is a pooled client. **Every request funnels through one method:**

```rust
session.send(request, raise_error, stream) -> HttpResponse
```

- `raise_error` (`true` on the verb helpers `get`/`post`/…) turns a 4xx/5xx into an error.
- `stream` (`true` by default) keeps the body a **live, seekable `HttpStream`**; `false`
  drains it into memory during `send`, releasing the connection at once.

Connection reuse is a per-request knob: `request.with_keep_alive(true)` pools the
connection so the next request skips the TLS handshake (default `false` →
`Connection: close`, the socket released the moment the body is drained). The verb
helpers (`get`/`post`/…) opt in, so a Session loop reuses one warm connection; a
hand-built request sent through `send`/`request` closes by default unless it opts in.

`send_many(requests)` runs an iterator of requests concurrently in batches, lazily.

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
