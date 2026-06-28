# Request & Response

The two halves of an HTTP exchange. An `HttpRequest` is a builder — a method, a URL,
headers and an optional body — that you can dispatch on its own; an `HttpResponse` is
what comes back, modelled on `requests.Response`: a status, headers and a body you read
as bytes, text or JSON. The verb helpers ([Session](session.md)) build *and* send a
request in one call, but you can also build a request, inspect it, and send it later.

## Building a request

Each builder method returns the request, so they chain. Set headers and query
parameters, attach auth, and choose a body.

=== "Python"

    ```python
    import yggdryl

    request = yggdryl.HttpRequest(
        "POST",
        "https://api.example.com/items",
        headers={"accept": "application/json"},
        params={"dry_run": "true"},
        bearer_auth="s3cr3t-token",
        body=b'{"name": "widget"}',
    )
    print(request.method, request.url)          # "POST" "https://api.example.com/items?dry_run=true"
    print(request.header("authorization"))      # "Bearer s3cr3t-token"
    ```

=== "Node"

    ```javascript
    const { HttpRequest } = require("yggdryl");

    const request = new HttpRequest(
        "POST",
        "https://api.example.com/items",
        { accept: "application/json" },          // headers
        Buffer.from('{"name": "widget"}'),       // body
        { dry_run: "true" },                     // params
        undefined,                               // basicAuth
        "s3cr3t-token",                          // bearerAuth
    );
    console.log(request.method, request.url);
    console.log(request.header("authorization")); // "Bearer s3cr3t-token"
    ```

=== "Rust"

    ```rust
    use yggdryl_http::HttpRequest;

    let request = HttpRequest::post("https://api.example.com/items")?
        .with_header("accept", "application/json")
        .with_param("dry_run", "true")
        .with_bearer_auth("s3cr3t-token")
        .with_body(b"{\"name\": \"widget\"}".to_vec());
    assert_eq!(request.method().as_str(), "POST");
    assert_eq!(request.headers().get("authorization"), Some("Bearer s3cr3t-token"));
    ```

!!! note "Builder methods (Rust)"
    `with_header` / `with_param` / `with_basic_auth` / `with_bearer_auth` /
    `with_body` / `with_body_io` / `with_allow_redirect` / `with_keep_alive` /
    `with_http_version` all consume and return `self`, mirroring the rest of the
    project's builders. The bindings fold the same options into the constructor's
    keyword (Python) / positional (Node) arguments.

## Auth and redirects

`with_basic_auth` (RFC 7617) and `with_bearer_auth` (RFC 6750) set the
`Authorization` header; a cross-origin redirect strips it. `with_allow_redirect(false)`
opts the request out of the redirect loop, returning the 3xx response itself.
`with_keep_alive(seconds)` sets the connection's keep-alive idle TTL (default 300; `0`
sends `Connection: close`).

=== "Python"

    ```python
    import yggdryl

    request = yggdryl.HttpRequest(
        "GET",
        "https://example.com/protected",
        basic_auth=("aladdin", "open sesame"),
        allow_redirect=False,
        keep_alive=0,            # Connection: close
    )
    ```

=== "Node"

    ```javascript
    const { HttpRequest } = require("yggdryl");

    const request = new HttpRequest(
        "GET",
        "https://example.com/protected",
        undefined,               // headers
        undefined,               // body
        undefined,               // params
        ["aladdin", "open sesame"], // basicAuth
        undefined,               // bearerAuth
        false,                   // allowRedirect
        0,                       // keepAlive -> Connection: close
    );
    ```

=== "Rust"

    ```rust
    use yggdryl_http::HttpRequest;

    let request = HttpRequest::get("https://example.com/protected")?
        .with_basic_auth("aladdin", "open sesame")
        .with_allow_redirect(false)
        .with_keep_alive(0.0); // Connection: close
    ```

## Streaming an upload off disk

The preferred upload path streams the body straight off an [`Io`](../core/io.md)
handle, so a large file is never buffered into memory. In Rust that is `with_body_io`;
the bindings accept a [`LocalPath`](../core/io.md) anywhere a body is expected and
stream it for you.

=== "Python"

    ```python
    import yggdryl

    upload = yggdryl.LocalPath("./big.parquet")          # streamed, not buffered
    response = yggdryl.HttpSession().post("https://example.com/upload", upload)
    response.raise_for_status()
    ```

=== "Node"

    ```javascript
    const { HttpSession, LocalPath } = require("yggdryl");

    const upload = new LocalPath("./big.parquet");        // streamed, not buffered
    const response = await new HttpSession().post("https://example.com/upload", upload);
    response.raiseForStatus();
    ```

=== "Rust"

    ```rust
    use yggdryl_http::HttpRequest;
    use yggdryl_core::LocalPath;

    let response = HttpRequest::post("https://example.com/upload")?
        .with_body_io(LocalPath::open("./big.parquet"))   // Content-Length framed, streamed
        .send(true)?;
    response.raise_for_status()?;
    ```

!!! tip "Pass `Io`, never raw bytes"
    `with_body_io` reads the handle's `stream_len` to set `Content-Length` and reads the
    bytes straight off it — a file is never loaded into memory. `with_body` takes an
    in-memory `Vec<u8>` (replayable, so it can be retried); a streamed body is single-shot.

## Sending — and building without sending

A request is self-sufficient: `send` dispatches it through the shared per-host session
and returns the response, no session reference needed. The verb helpers take a trailing
`send` flag (Rust positional, bindings keyword/optional). With `send = false` they return
an **unsent** `HttpResponse` — `is_sent()` is `false`, the status is `0`, the body empty —
carrying only the prepared request. You inspect it, then dispatch it later with the
response's own `send`.

=== "Python"

    ```python
    import yggdryl

    session = yggdryl.HttpSession()

    # Build without dispatching: an unsent response carrying the prepared request.
    unsent = session.get("https://httpbin.org/get", send=False)
    assert not unsent.is_sent and unsent.status == 0
    assert unsent.request.method == "GET"

    # Dispatch it later.
    response = unsent.send()            # raise_error=True by default
    assert response.is_sent and response.ok

    # Or send a hand-built request directly.
    response = yggdryl.HttpRequest("GET", "https://httpbin.org/get").send()
    ```

=== "Node"

    ```javascript
    const { HttpSession, HttpRequest } = require("yggdryl");

    const session = new HttpSession();

    // Build without dispatching: an unsent response carrying the prepared request.
    const unsent = await session.get(
        "https://httpbin.org/get",
        undefined, undefined, undefined, undefined, // headers/params/basicAuth/bearerAuth
        undefined, undefined, undefined,            // allowRedirect/keepAlive/httpVersion
        undefined,                                  // raiseError
        false,                                      // send
    );
    console.log(unsent.isSent, unsent.status);      // false 0
    console.log(unsent.request.method);             // "GET"

    // Dispatch it later (returns a Promise).
    const response = await unsent.send();           // raiseError=true by default

    // Or send a hand-built request directly.
    const direct = await new HttpRequest("GET", "https://httpbin.org/get").send();
    ```

=== "Rust"

    ```rust
    use yggdryl_http::HttpSession;

    let session = HttpSession::new();

    // Build without dispatching: an unsent response carrying the prepared request.
    let unsent = session.get("https://httpbin.org/get", false)?;
    assert!(!unsent.is_sent() && unsent.status() == 0);
    assert_eq!(unsent.request().map(|r| r.method().as_str()), Some("GET"));

    // Dispatch it later.
    let response = unsent.send(true)?;  // true = raise on 4xx/5xx
    assert!(response.is_sent() && response.ok());
    ```

!!! note "`send` flag vs. the dispatch primitive"
    The verb helpers carry the `send` flag; `session.send(request, raise_error)` (Rust)
    / `session.send(request, raiseError)` (bindings) is the dispatch primitive — it
    always sends and takes no flag. `raise_error` (the verb default) turns a 4xx/5xx
    status into an error.

## Reading the response

`status` / `ok` / `is_sent` / `raise_for_status` describe the outcome; `headers` /
`header(name)` (case-insensitive) read the response headers; `request()` returns the
originating prepared request and `session()` (Rust) the shared session it belongs to.
After a redirect chain `request()` is the *original* request, so its method/URL may
differ from the response's final `url`. `negotiated_version` reports the HTTP version
actually spoken — `"HTTP/1.1"` / `"HTTP/2.0"` / `"HTTP/3.0"` — which may differ from
the session's `with_http_version` pin when `Auto` negotiation picks a version at
runtime.

=== "Python"

    ```python
    import yggdryl

    response = yggdryl.get("https://httpbin.org/json")
    print(response.status, response.ok)         # 200 True
    print(response.header("content-type"))      # "application/json"
    print(response.request.url)                 # the originating request URL
    print(response.http_version)                # "HTTP/1.1" / "HTTP/2.0" / "HTTP/3.0"
    ```

=== "Node"

    ```javascript
    const yggdryl = require("yggdryl");

    const response = await yggdryl.get("https://httpbin.org/json");
    console.log(response.status, response.ok);    // 200 true
    console.log(response.header("content-type")); // "application/json"
    console.log(response.request.url);            // the originating request URL
    console.log(response.httpVersion);            // "HTTP/1.1" / "HTTP/2.0" / "HTTP/3.0"
    ```

=== "Rust"

    ```rust
    use yggdryl_http::{HttpSession, HttpVersion};

    let response = HttpSession::new().get("https://httpbin.org/json", true)?;
    println!("{} {}", response.status(), response.ok());     // 200 true
    println!("{:?}", response.header("content-type"));       // Some("application/json")
    println!("{:?}", response.request().map(|r| r.url().to_string()));
    println!("{:?}", response.negotiated_version());         // HttpVersion::Http11
    ```

## The body — bytes, text, JSON

The body is read lazily off the connection and `Content-Encoding` is decoded
transparently. In Rust the accessors **consume** the response: `reader()` (a decoded
`Io`), `bytes()` / `text()` / `json()` (drain it), `into_bytesio()` / `into_io()` (take
the whole body for seekable access). In the bindings the body is drained and decompressed
once when the response resolves, then exposed three ways: `io` (a Rust-backed
[`BytesIO`](../core/io.md), no native copy — the performant accessor), `content` (native
`bytes` / `Buffer`), and `text()` / `json()`.

=== "Python"

    ```python
    import yggdryl

    response = yggdryl.get("https://httpbin.org/gzip")  # Content-Encoding: gzip
    data = response.json()          # decoded, parsed in Rust — no FFI copy
    text = response.text()          # decoded UTF-8
    raw = response.content          # native bytes (decompressed)
    handle = response.io            # a Rust-backed BytesIO (seek / pread / json)
    ```

=== "Node"

    ```javascript
    const yggdryl = require("yggdryl");

    const response = await yggdryl.get("https://httpbin.org/gzip"); // Content-Encoding: gzip
    const data = response.json();    // decoded, parsed in Rust
    const text = response.text();    // decoded UTF-8
    const raw = response.content;    // native Buffer (decompressed)
    const handle = response.io;      // a Rust-backed BytesIO (seek / pread / json)
    ```

=== "Rust"

    ```rust
    use yggdryl_http::HttpSession;

    let response = HttpSession::new().get("https://httpbin.org/gzip", true)?; // gzip
    let data = response.json()?;     // decoded off the stream, parsed in Rust
    // (each of these consumes the response — pick one)
    // let text = response.text()?;
    // let body = response.into_bytesio()?; // seekable BytesIO
    ```

!!! tip "Prefer `io` for further Rust-side work"
    In the bindings `response.io` stays a Rust-backed, seekable byte buffer, so you can
    `json()` / `decompress()` / `read` it (or hand it to another yggdryl call) without
    copying the payload into the host language. Reach for `content` only when a native
    API demands `bytes` / `Buffer`. See [Byte IO](../core/io.md).

## A response is an `Io`

In the Rust core an `HttpResponse` **is itself** an [`Io`](../core/io.md) handle: it
delegates `read` / `seek` / `pread` to its body (the live [`HttpStream`](stream.md)), so a
returned response reads and seeks like any other byte source — and a `pread` on the live
stream is a true partial fetch (one `Range`, no full download). Use `reader()` instead for
transparent `Content-Encoding` decoding. The bindings **buffer** the body and surface it
as `response.io`, a seekable [`BytesIO`](../core/io.md): `seek` / `pread` there work
**in memory** over the already-downloaded body (see [Streaming body](stream.md) for the
distinction).

=== "Python"

    ```python
    import yggdryl

    response = yggdryl.HttpSession().get("https://example.com/big.parquet")
    body = response.io           # a Rust-backed BytesIO over the buffered body
    tail = body.pread(8, -8, 2)  # whence 2 = end; reads in memory (body buffered)
    ```

=== "Node"

    ```javascript
    const { HttpSession } = require("yggdryl");

    const response = await new HttpSession().get("https://example.com/big.parquet");
    const body = response.io;       // a Rust-backed BytesIO over the buffered body
    const tail = body.pread(8, -8, 2); // whence 2 = end; reads in memory (body buffered)
    ```

=== "Rust"

    ```rust
    use yggdryl_http::HttpSession;
    use yggdryl_core::{Io, Whence};

    // A response *is* an Io: seek/pread it directly, or take the body with into_io().
    let mut body = HttpSession::new()
        .get("https://example.com/big.parquet", true)?
        .into_io();
    let mut footer = [0u8; 8];
    body.pread(&mut footer, -8, Whence::End)?; // one Range request, no full download
    ```

## Typed content reads

Under the `media` / `compression` features the response infers types from its headers:
`mime_type` (from `Content-Type`), `media_type` (a layered stack **combining
`Content-Type` with `Content-Encoding`** — a gzipped CSV reads as `["text/csv",
"application/gzip"]`, like a `data.csv.gz` path), and `compression` (the codec named by
`Content-Encoding`). See [Media types](../core/media.md) and
[Compression](../core/compression.md).

=== "Python"

    ```python
    import yggdryl

    response = yggdryl.get("https://example.com/data.csv.gz")
    print(response.mime_type)     # "text/csv"
    print(response.media_type)    # ["text/csv", "application/gzip"]
    print(response.compression)   # "gzip"  (already decoded in content / text / json)
    ```

=== "Node"

    ```javascript
    const yggdryl = require("yggdryl");

    const response = await yggdryl.get("https://example.com/data.csv.gz");
    console.log(response.mimeType);    // "text/csv"
    console.log(response.mediaType);   // ["text/csv", "application/gzip"]
    console.log(response.compression); // "gzip" (already decoded in content/text/json)
    ```

=== "Rust"

    ```rust
    use yggdryl_http::HttpSession;

    let response = HttpSession::new().get("https://example.com/data.csv.gz", true)?;
    // Under the `media` / `compression` features:
    // response.mime_type()   -> Some(MimeType::Csv)
    // response.media_type()  -> Some([Csv, Gzip])
    // response.compression() -> Some(Compression::Gzip)
    ```

## See also

- [Session](session.md) — the client that builds and runs requests, with pooling,
  cookies, redirects and the shared singleton behind the module-level verbs.
- [Streaming body](stream.md) — the live `HttpStream` a response holds.
- [Cookies](cookies.md) — the RFC 6265 jar the session feeds each exchange.
- [Byte IO](../core/io.md) — the `Io` surface a response and `response.io` expose.
