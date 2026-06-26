# URI & URL

Two value types for addressing resources: `Uri` is the generic
[RFC 3986](https://www.rfc-editor.org/rfc/rfc3986) URI
(`scheme:[//authority]path[?query][#fragment]`), and `Url` is a `Uri` that always
has an authority, with that authority broken out into `username` / `password` /
`host` / `port`. Both parse, decompose, edit query parameters, resolve relative
references, and infer a [media type](media.md) from their path — and both are
immutable: every editor returns a new value.

## Parse and decompose

`from_str` validates the input (scheme and any `%XX` escapes) and raises on
malformed input — there is no lenient mode. The component accessors then read each
piece back out.

=== "Python"

    ```python
    import yggdryl

    uri = yggdryl.Uri("https://example.com/docs?page=2#intro")
    assert uri.scheme == "https"
    assert uri.authority == "example.com"
    assert uri.path == "/docs"
    assert uri.query == "page=2"
    assert uri.fragment == "intro"

    url = yggdryl.Url("https://user:pw@example.com:8443/api?v=1#top")
    assert url.host == "example.com"
    assert url.port == 8443
    assert url.username == "user"
    ```

=== "Node"

    ```javascript
    const { Uri, Url } = require("yggdryl");

    const uri = new Uri("https://example.com/docs?page=2#intro");
    // uri.scheme "https", uri.authority "example.com", uri.path "/docs"
    // uri.query "page=2", uri.fragment "intro"

    const url = new Url("https://user:pw@example.com:8443/api?v=1#top");
    // url.host "example.com", url.port 8443, url.username "user"
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{Uri, Url};

    let uri = Uri::from_str("https://example.com/docs?page=2#intro")?;
    assert_eq!(uri.scheme(), "https");
    assert_eq!(uri.authority(), Some("example.com"));
    assert_eq!(uri.path(), "/docs");

    let url = Url::from_str("https://user:pw@example.com:8443/api?v=1#top")?;
    assert_eq!(url.host(), "example.com");
    assert_eq!(url.port(), Some(8443));
    assert_eq!(url.username(), Some("user"));
    ```

!!! note
    A `Url` needs an authority with a non-empty host: `mailto:alice@example.com`
    parses as a `Uri` but raises as a `Url`. A scheme-less input defaults to the
    `file` scheme (`relative/path` → `file:relative/path`), and `\` is normalised
    to `/` so Windows paths parse.

## Query parameters

The query is a multimap: a key may appear more than once, so every read returns a
list of values. `params` parses the whole query; `get_param` reads one key. The
editors (`set_param` / `add_param` / `set_params` / `remove_param` /
`remove_params` / `clear_params` / `with_params`) each return a **new** value —
the original is untouched.

=== "Python"

    ```python
    import yggdryl

    url = yggdryl.Url("https://h/p?a=1&a=2&b=hi")
    assert url.params() == {"a": ["1", "2"], "b": ["hi"]}
    assert url.get_param("a") == ["1", "2"]
    assert url.get_param("z") is None

    # Each editor returns a new Url (values are percent-encoded by default).
    updated = url.set_param("a", ["x"]).add_param("c", ["1", "2"])
    assert updated.get_param("a") == ["x"]
    assert updated.remove_param("b").get_param("b") is None

    built = yggdryl.Uri("https://h/p").with_params({"q": ["a b"]})
    assert built.query == "q=a%20b"
    ```

=== "Node"

    ```javascript
    const { Uri, Url } = require("yggdryl");

    const url = new Url("https://h/p?a=1&a=2&b=hi");
    // url.params() -> { a: ['1', '2'], b: ['hi'] }
    // url.getParam('a') -> ['1', '2'], url.getParam('z') -> null

    const updated = url.setParam("a", ["x"]).addParam("c", ["1", "2"]);
    // updated.getParam('a') -> ['x']

    const built = new Uri("https://h/p").withParams({ q: ["a b"] });
    // built.query -> "q=a%20b"
    ```

=== "Rust"

    ```rust
    use yggdryl_core::Url;

    let url = Url::from_str("https://h/p?a=1&a=2&b=hi")?;
    assert_eq!(url.get_param("a"), Some(vec!["1".into(), "2".into()]));
    assert_eq!(url.get_param("z"), None);

    // Each editor returns a new Url; values are percent-encoded when `encode`.
    let updated = url
        .set_param("a", vec!["x".into()], true)
        .add_param("c", vec!["1".into(), "2".into()], true);
    assert_eq!(updated.get_param("a"), Some(vec!["x".into()]));
    ```

!!! tip
    `params(decode)` and `with_params(map, encode)` take a flag (default: decode /
    encode). Pass `decode=false` to keep raw percent-encoded values, or
    `encode=false` to store a query verbatim — the same toggle the string
    renderers use below.

## Join — RFC 3986 reference resolution

`join` resolves a reference against the current path (RFC 3986 §5.2.4 dot-segment
removal): a leading `/` replaces the path, `.` / `..` climb relative to the last
`/`. The reference may be a path **string** (kept verbatim — already encoded), a
**list of segments** (each percent-encoded, `/` inside a segment is data), or
another `Uri` / `Url` (its path is used). The authority is preserved; the query
and fragment are dropped (the location has changed).

=== "Python"

    ```python
    import yggdryl

    base = yggdryl.Url("https://h/a/b/c")
    assert base.join("d").path == "/a/b/d"
    assert base.join("../x").path == "/a/x"
    assert base.join("../../x").path == "/x"
    assert base.join("/abs/y").path == "/abs/y"
    # A list is percent-encoded and '/'-joined; '/' inside an element is data.
    assert base.join(["d", "e f"]).path == "/a/b/d/e%20f"
    # Joining drops self's query/fragment.
    assert str(yggdryl.Url("https://h/a/b/c?k=v#f").join("../x")) == "https://h/a/x"
    ```

=== "Node"

    ```javascript
    const { Url } = require("yggdryl");

    const base = new Url("https://h/a/b/c");
    // base.join('d').path        -> "/a/b/d"
    // base.join('../x').path     -> "/a/x"
    // base.join('/abs').path     -> "/abs"
    // base.join(['d', 'e f']).path -> "/a/b/d/e%20f"
    // new Url('https://h/a/b/c?k=v#f').join('../x').toString() -> "https://h/a/x"
    ```

=== "Rust"

    ```rust
    use yggdryl_core::Url;

    let base = Url::from_str("https://h/a/b/c")?;
    assert_eq!(base.join("d").path(), "/a/b/d");
    assert_eq!(base.join("../x").path(), "/a/x");
    assert_eq!(base.join("/abs/y").path(), "/abs/y");
    // A slice of segments is percent-encoded and '/'-joined.
    assert_eq!(base.join(["d", "e f"]).path(), "/a/b/d/e%20f");
    ```

## Media type from the path

The path's file extensions infer a [media type](media.md). `media_type()` returns
the layered `MediaType` stack (e.g. `data.csv.gz` → `[Csv, Gzip]`), and
`mime_type()` returns the outermost `MimeType` — both `None` when no known
extension is found.

=== "Python"

    ```python
    import yggdryl

    url = yggdryl.Url("https://h/data/report.csv.gz")
    assert [t.mime for t in url.media_type().types] == [
        "text/csv", "application/gzip",
    ]
    assert url.mime_type().mime == "application/gzip"
    assert url.extensions() == ["csv", "gz"]
    ```

=== "Node"

    ```javascript
    const { Url } = require("yggdryl");

    const url = new Url("https://h/data/report.csv.gz");
    url.mediaType().types.map((t) => t.mime);
    // ["text/csv", "application/gzip"]
    url.mimeType().mime; // "application/gzip"
    url.extensions();    // ["csv", "gz"]
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{MimeType, Url};

    let url = Url::from_str("https://h/data/report.csv.gz")?;
    assert_eq!(
        url.media_type().unwrap().types(),
        [MimeType::Csv, MimeType::Gzip],
    );
    assert_eq!(url.mime_type(), Some(MimeType::Gzip));
    ```

## Render and convert

`to_str(encode)` renders the address as a string — encoded for transport (the
default, also what `str()` / `toString()` / `Display` produce) or decoded for
display. `to_uri` / `to_url` convert between the two types (`to_url` requires an
authority with a host).

=== "Python"

    ```python
    import yggdryl

    url = yggdryl.Url("https://h/a%20b?q=x%20y")
    assert url.to_string() == "https://h/a%20b?q=x%20y"          # encoded (default)
    assert url.to_string(encode=False) == "https://h/a b?q=x y"  # decoded

    uri = url.to_uri()
    assert uri.authority == "h"
    # to_url() requires a host: Uri("mailto:a@b").to_url() raises ValueError.
    ```

=== "Node"

    ```javascript
    const { Url } = require("yggdryl");

    const url = new Url("https://h/a%20b?q=x%20y");
    url.toString();      // "https://h/a%20b?q=x%20y"  (encoded, default)
    url.toString(false); // "https://h/a b?q=x y"      (decoded)

    const uri = url.toUri(); // uri.authority -> "h"
    ```

=== "Rust"

    ```rust
    use yggdryl_core::Url;

    let url = Url::from_str("https://h/a%20b?q=x%20y")?;
    assert_eq!(url.to_str(true), "https://h/a%20b?q=x%20y");  // encoded
    assert_eq!(url.to_str(false), "https://h/a b?q=x y");     // decoded

    let uri = url.to_uri();
    assert_eq!(uri.authority(), Some("h"));
    ```

## See also

- [Media types](media.md) — the `MimeType` / `MediaType` the path accessors infer.
- [Byte IO](io.md) — `from_str(location)` opens the backend a URL points at.
- [Request & Response](../http/request-response.md) — where a `Url` becomes a
  live HTTP request.
