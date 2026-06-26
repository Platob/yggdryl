# Session

`HttpSession` is the connection-pooling, defaulting HTTP client — the equivalent of
`requests.Session`. It carries default headers, auth, a cookie jar, a retry policy,
timeouts and a `base_url`, builds requests with `prepare`, and runs every one through
a single `send` path. The verb helpers (`get` / `head` / `delete` / `post` / `put` /
`patch` / `request`) build and dispatch a request in one call; the module-level verbs
do the same through a process-wide shared singleton, the `requests.get(...)` equivalent.

## Verb helpers

Each verb resolves its target against the session's [`base_url`](#base-url), builds
an [`HttpRequest`](request-response.md) (merging the session defaults), dispatches it,
and returns an [`HttpResponse`](request-response.md). They raise on a 4xx/5xx status.

!!! note "The `send` flag (Rust / Python)"
    The Rust verbs take a trailing `send: bool`; in Python it is the `send=` keyword.
    With `send=false` the verb builds the request **without** dispatching and returns
    an **unsent** response carrying it (status `0`, `is_sent() == false`) — dispatch it
    later with `response.send(raise_error)`. See [Request & Response](request-response.md).

=== "Python"

    ```python
    import yggdryl

    session = yggdryl.HttpSession(user_agent="my-app/1.0")
    response = session.get("https://httpbin.org/json")
    print(response.status, response.header("content-type"))

    session.post("https://httpbin.org/post", b"payload")
    session.delete("https://httpbin.org/anything")
    session.request("PATCH", "https://httpbin.org/patch", body=b"{}")
    ```

=== "Node"

    ```javascript
    const { HttpSession } = require("yggdryl");

    const session = new HttpSession("my-app/1.0");
    const response = await session.get("https://httpbin.org/json"); // returns a Promise
    console.log(response.status, response.header("content-type"));

    await session.post("https://httpbin.org/post", Buffer.from("payload"));
    // No `delete` verb (JS reserved word) — use request("DELETE", url).
    await session.request("DELETE", "https://httpbin.org/anything");
    await session.request("PATCH", "https://httpbin.org/patch", undefined, Buffer.from("{}"));
    ```

=== "Rust"

    ```rust
    use yggdryl_http::HttpSession;

    let session = HttpSession::new().with_user_agent("my-app/1.0");
    let response = session.get("https://httpbin.org/json", true)?; // true = send now
    println!("{} {:?}", response.status(), response.content_type());

    session.post("https://httpbin.org/post", b"payload".to_vec(), true)?;
    session.delete("https://httpbin.org/anything", true)?;
    ```

!!! tip "Node is async, Python and Rust are sync"
    Node's verbs return a `Promise` (`await` them) and take **positional** optional
    args. Python's are synchronous with **keyword** args (`headers=`, `params=`,
    `basic_auth=`, `bearer_auth=`, `allow_redirect=`, `keep_alive=`, `http_version=`,
    `raise_error=`, `send=`). Rust's are synchronous with a trailing `send: bool`.

## Default headers, user-agent & auth

A session merges its default headers into every request (a per-request header wins,
case-insensitively). `with_user_agent` sets the `User-Agent`; `with_basic_auth` /
`with_bearer_auth` set a default `Authorization` (HTTP Basic RFC 7617 / Bearer RFC
6750). Session-level auth is a default header, so a per-request value overrides it and
a cross-origin redirect strips it — credentials never leak to another host.

=== "Python"

    ```python
    session = yggdryl.HttpSession(
        headers={"Accept": "application/json"},
        bearer_auth="tok-123",          # or basic_auth=("user", "pass")
    )
    session.get("https://httpbin.org/bearer")   # sends Authorization: Bearer tok-123
    ```

=== "Node"

    ```javascript
    // Constructor: (userAgent, headers, maxRedirects, baseUrl, httpVersion, verify,
    //              proxy, caCert, caCertFile, basicAuth, bearerAuth, readTimeout).
    // bearerAuth is the 11th argument.
    const session = new HttpSession(
        undefined, { Accept: "application/json" }, undefined, undefined, undefined,
        undefined, undefined, undefined, undefined, undefined, "tok-123",
    );
    await session.get("https://httpbin.org/bearer");
    ```

=== "Rust"

    ```rust
    let session = HttpSession::new()
        .with_header("accept", "application/json")
        .with_bearer_auth("tok-123");   // or .with_basic_auth("user", "pass")
    session.get("https://httpbin.org/bearer", true)?;
    ```

## Base URL

`with_base_url` sets a prefix that relative request targets resolve against (like
`requests`' session prefix or `httpx`'s `base_url`). The verb helpers run their target
through `resolve_url`: a relative reference (`/path`, `name`) joins onto the base by
RFC 3986 rules, while an absolute URL is used unchanged. With no base URL, targets
must be absolute.

=== "Python"

    ```python
    session = yggdryl.HttpSession(base_url="https://api.example.com/")
    session.get("users/42")              # -> https://api.example.com/users/42
    session.get("https://other.example") # absolute URL bypasses the base
    ```

=== "Node"

    ```javascript
    const session = new HttpSession(
        undefined, undefined, undefined, "https://api.example.com/",
    );
    await session.get("users/42");       // -> https://api.example.com/users/42
    ```

=== "Rust"

    ```rust
    use yggdryl_core::Url;

    let session = HttpSession::new()
        .with_base_url(Url::from_str("https://api.example.com/")?);
    let url = session.resolve_url("users/42")?; // https://api.example.com/users/42
    session.get("users/42", true)?;
    ```

## Retries, timeouts & redirects

A session retries transient statuses (429 / 502 / 503 / 504, honouring `Retry-After`,
plus a single retry of a 500) with capped exponential backoff via its `RetryConfig`.
`with_read_timeout(seconds)` (default 120) errors if the server sends no data for that
long; `0` removes the bound. `with_max_redirects` (default 10) caps the 3xx hops
`send` follows before raising; a per-request opt-out is `allow_redirect`.

=== "Python"

    ```python
    session = yggdryl.HttpSession(
        read_timeout=30,        # seconds; 0 = unbounded
        max_redirects=5,
    )
    assert session.read_timeout == 30.0
    ```

=== "Node"

    ```javascript
    // readTimeout is the 12th option; maxRedirects the 3rd.
    const opts = Array(12).fill(undefined);
    opts[2] = 5;       // maxRedirects (3rd arg)
    opts[11] = 30;     // readTimeout (12th arg, seconds)
    const session = new HttpSession(...opts);
    // session.readTimeout === 30
    ```

=== "Rust"

    ```rust
    use yggdryl_http::RetryConfig;

    let session = HttpSession::new()
        .with_retry(RetryConfig::default())
        .with_read_timeout(30.0)   // seconds; 0.0 = unbounded
        .with_max_redirects(5);
    ```

!!! note "TLS, proxy & connection pool"
    The default trust store is the **OS-native** certificate store. `with_verify(false)`
    disables verification (insecure); `with_ca_cert` / `with_ca_cert_file` install
    custom CA certificates (the secure alternative to disabling verification). A session
    adopts the environment proxy by default (`HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY`,
    honouring `NO_PROXY`); `with_proxy` overrides it. `with_pool_size` tunes the idle
    connection pool — reused keep-alive connections skip the TLS handshake.

## Concurrency — `send_many` (Rust)

`send_many(requests)` runs an iterator of requests concurrently, **lazily** in batches
of `batch_size`, each batch running up to `max_concurrency` at a time (scoped threads).
Only one batch is in flight, so an unbounded request stream uses bounded memory. Each
result is returned whatever its status (transport/parse failures are `Err` entries).
This is a Rust-core API; the bindings issue requests one at a time (Python is
synchronous, Node returns per-call `Promise`s you can `Promise.all`).

=== "Python"

    ```python
    # No send_many in the binding — issue requests directly (or use a thread pool).
    session = yggdryl.HttpSession()
    results = [session.get(url, raise_error=False) for url in urls]
    ```

=== "Node"

    ```javascript
    // No sendMany in the binding — fan out with Promise.all over the verb Promises.
    const session = new HttpSession();
    const results = await Promise.all(urls.map((u) => session.get(u)));
    ```

=== "Rust"

    ```rust
    let session = HttpSession::new()
        .with_max_concurrency(8)   // requests in flight per wave
        .with_batch_size(80);      // requests pulled per batch
    let requests = urls.iter().map(|u| HttpRequest::get(u).unwrap());
    for batch in session.send_many(requests) {
        for result in batch.into_results() {
            let response = result?;
            println!("{}", response.status());
        }
    }
    ```

## The shared singleton & module-level verbs

A process-wide **shared** session backs the module-level verbs (`get` / `head` /
`post` / `put` / `patch` / `delete` / `request`), the `requests.get(...)` equivalent.
`shared()` returns it (created on first use, with its own cookie jar); `set_shared`
replaces it — the way to give the module verbs a `base_url` or default headers. The
bindings expose `set_base_url` to point the singleton at a host.

`shared_for(host)` returns a **per-host** pooled singleton — one session per hostname,
inheriting the shared session's configuration with its own connection pool. This is
the session a request is dispatched through when none is given explicitly (it backs
`HttpRequest::send`, the `http`/`https` [`Io`](../core/io.md) factory, and the session
a returned response carries). The registry is bounded, evicting idle entries past its
cap.

=== "Python"

    ```python
    import yggdryl

    # Module-level verbs dispatch through the shared singleton.
    response = yggdryl.get("https://httpbin.org/get")
    yggdryl.post("https://httpbin.org/post", b"ping")
    yggdryl.request("DELETE", "https://httpbin.org/anything")

    # Give the singleton a base URL, then call verbs with relative targets.
    yggdryl.set_base_url("https://api.example.com/")
    yggdryl.get("/users")
    ```

=== "Node"

    ```javascript
    const yggdryl = require("yggdryl");

    const response = await yggdryl.get("https://httpbin.org/get");
    await yggdryl.post("https://httpbin.org/post", Buffer.from("ping"));
    // No module-level `delete` (JS reserved word) — use request("DELETE", url).
    await yggdryl.request("DELETE", "https://httpbin.org/anything");

    yggdryl.setBaseUrl("https://api.example.com/");
    await yggdryl.get("/users");
    ```

=== "Rust"

    ```rust
    use yggdryl_http::{self, HttpSession};
    use yggdryl_core::Url;

    // Module functions go through HttpSession::shared().
    let response = yggdryl_http::get("https://httpbin.org/get", true)?;
    yggdryl_http::post("https://httpbin.org/post", b"ping".to_vec(), true)?;

    // Reconfigure the singleton (e.g. a base URL) with set_shared.
    HttpSession::set_shared(
        HttpSession::new().with_base_url(Url::from_str("https://api.example.com/")?),
    );
    yggdryl_http::get("/users", true)?;
    ```

## See also

- [Request & Response](request-response.md) — building requests, the unsent-response
  flow, and reading bodies.
- [Streaming body](stream.md) — `HttpStream`, the seekable body with `Range` reads.
- [Cookies](cookies.md) — the RFC 6265 jar the session feeds on every dispatch.
- [Byte IO](../core/io.md) · [Compression](../core/compression.md) — what response
  bodies build on.
