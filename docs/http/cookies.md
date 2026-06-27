# Cookies

Every [session](session.md) keeps an RFC 6265 cookie jar. It ingests `Set-Cookie`
from each response and adds the matching `Cookie` header before each dispatch — no
configuration needed. You inspect the jar with `cookies` and seed your own with
`set_cookie`. The jar is shared across requests on a session (and across the
module-level verbs, which run on the shared session).

## Automatic ingest and send

The jar works on its own: a login response's `Set-Cookie` is stored, then sent
back on the next matching request. Nothing to wire up.

=== "Python"

    ```python
    import yggdryl

    session = yggdryl.HttpSession()
    session.post("https://example.com/login", b"user=me&pass=secret")
    # the session stored the response's Set-Cookie; it is now sent automatically
    me = session.get("https://example.com/account")
    ```

=== "Node"

    ```javascript
    const { HttpSession } = require("yggdryl");

    const session = new HttpSession();
    await session.post("https://example.com/login", Buffer.from("user=me&pass=secret"));
    // the session stored the response's Set-Cookie; it is now sent automatically
    const me = await session.get("https://example.com/account");
    ```

=== "Rust"

    ```rust
    use yggdryl_http::HttpSession;

    let session = HttpSession::new();
    session.post("https://example.com/login", "user=me&pass=secret", true)?;
    // the session stored the response's Set-Cookie; it is now sent automatically
    let me = session.get("https://example.com/account", true)?;
    ```

!!! note
    A per-request `Cookie` header you set yourself wins — the jar only adds its
    `Cookie` when the request does not already carry one.

## Inspecting the jar

`cookies` is a **snapshot** of the stored cookies. In the bindings it is a plain
name → value map; in Rust it is an `HttpCookies` you iterate (`iter`) or look up
(`get`).

=== "Python"

    ```python
    session = yggdryl.HttpSession()
    session.set_cookie("https://example.com/", "sid", "abc123")
    cookies = session.cookies          # dict of name -> value
    assert cookies["sid"] == "abc123"
    ```

=== "Node"

    ```javascript
    const session = new HttpSession();
    session.setCookie("https://example.com/", "sid", "abc123");
    const cookies = session.cookies;   // object of name -> value
    // cookies.sid === "abc123"
    ```

=== "Rust"

    ```rust
    use yggdryl_core::Url;

    let session = HttpSession::new();
    session.set_cookie(&Url::from_str("https://example.com/")?, "sid", "abc123");
    let jar = session.cookies();        // an HttpCookies snapshot
    assert_eq!(jar.get("sid").map(|c| c.value()), Some("abc123"));
    ```

## Seeding a cookie

`set_cookie(url, name, value)` adds a cookie scoped to `url`'s host (host-only)
and path `"/"`, so it rides along on matching requests. An empty `name` is ignored.

=== "Python"

    ```python
    session = yggdryl.HttpSession()
    session.set_cookie("https://api.example.com/", "token", "xyz")
    # `token` is now sent on every https://api.example.com/... request
    session.get("https://api.example.com/me")
    ```

=== "Node"

    ```javascript
    const session = new HttpSession();
    session.setCookie("https://api.example.com/", "token", "xyz");
    // `token` is now sent on every https://api.example.com/... request
    await session.get("https://api.example.com/me");
    ```

=== "Rust"

    ```rust
    use yggdryl_core::Url;

    let session = HttpSession::new();
    let url = Url::from_str("https://api.example.com/")?;
    session.set_cookie(&url, "token", "xyz");
    // `token` is now sent on every https://api.example.com/... request
    session.get("https://api.example.com/me", true)?;
    ```

## Matching rules

A stored cookie is sent only when it applies to the request URL, following RFC 6265:

- **Domain** (§5.1.3) — a host-only cookie (no `Domain` attribute) matches only the
  exact host; a `Domain` cookie matches that host or any `.`-suffixed subdomain. An
  IP literal never subdomain-matches. A cross-domain `Domain` (cookie injection) and
  a single-label / public-suffix `Domain` (e.g. `Domain=com`) are rejected on ingest.
- **Path** (§5.1.4) — the request path equals the cookie path, or the cookie path is
  a prefix ending in `/`, or the next request-path byte is `/`. Longer-path cookies
  are listed first in the `Cookie` header.
- **Secure** — a `Secure` cookie is withheld over plain `http`.
- **Expiry** — `Max-Age` (which wins over `Expires`) sets the absolute expiry; expired
  cookies are dropped on access. A cookie with no expiry is a session cookie.

A later `Set-Cookie` with the same `(name, domain, path)` replaces the earlier one.

!!! tip
    A redirect to another origin (scheme, host or port differ) strips the per-request
    `Cookie` for that hop — see the redirect handling on the [Session](session.md).
    Cross-origin cookie leakage is prevented for you.

## Related

- [Session](session.md) — where the jar lives and the redirect rules that guard it.
- [Request & Response](request-response.md) — setting a per-request `Cookie` header.
