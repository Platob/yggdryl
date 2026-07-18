# Headers — the one metadata map

`Headers` is the project's **single** metadata/annotation map, a root module beside
[`uri`](uri.md): HTTP headers, schema metadata, and source annotations all live here — never a
second map type. It follows HTTP conventions: insertion-ordered, name-matching is
ASCII-case-insensitive, a name may repeat (multi-value), and both names and values may be raw
bytes with `&str` conveniences on top. `insert` replaces, `append` keeps. It round-trips through
an HTTP text form (`parse_http` / `to_http_bytes`) and a byte codec (`serialize_bytes` /
`deserialize_bytes`) that preserves arbitrary bytes, order, and duplicates. In Python it is a
mutable mapping (dict protocol, unhashable like `dict`); in Node the same capability is named
methods.

=== "Python"

    ```python
    from yggdryl.headers import Headers

    h = Headers()
    h.insert("Content-Type", "application/json")
    h.append("Set-Cookie", "a=1")
    h.append("Set-Cookie", "b=2")

    assert h.get("content-type") == "application/json"   # case-insensitive
    assert h.get_all("set-cookie") == ["a=1", "b=2"]      # multi-value
    assert "Content-Type" in h and len(h) == 3            # dict protocol
    h["X-Trace"] = "abc"                                  # insert via dunder
    del h["Set-Cookie"]                                   # removes every occurrence

    round = Headers.deserialize_bytes(h.serialize_bytes())
    assert round == h
    ```

=== "Node"

    ```js
    const { Headers } = require('yggdryl').headers

    const h = new Headers()
    h.insert('Content-Type', 'application/json')
    h.append('Set-Cookie', 'a=1')
    h.append('Set-Cookie', 'b=2')

    console.assert(h.get('content-type') === 'application/json')  // case-insensitive
    console.assert(h.getAll('set-cookie').length === 2)           // multi-value
    console.assert(h.contains('Content-Type') && h.len() === 3)

    const round = Headers.deserializeBytes(h.serializeBytes())
    console.assert(round.equals(h))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::headers::Headers;

    let mut h = Headers::new();
    h.insert(Headers::CONTENT_TYPE, "application/json");
    h.append("Set-Cookie", "a=1");
    h.append("Set-Cookie", "b=2");

    assert_eq!(h.get("content-type").as_deref(), Some("application/json")); // case-insensitive
    assert_eq!(h.get_all("set-cookie"), vec!["a=1", "b=2"]);      // multi-value
    assert_eq!(Headers::deserialize_bytes(&h.serialize_bytes()).unwrap(), h);
    ```

Every byte source carries one — see
[`headers()` on the memory contract](io/memory.md#metadata-mode-and-kind).

## Media type and modification time

`Headers` is the one place `Content-Type` / `Content-Encoding` are interpreted, so the io layer
and [`Uri`](uri.md) share one reading. `mime_type()` is the primary [`MimeType`](mediatype.md)
of `Content-Type`; `media_type()` folds the `Content-Encoding` layers in (`application/x-tar` +
`gzip` → `[application/x-tar, application/gzip]`); the `set_*` mutators write them back.
`mtime()` / `set_mtime()` carry the modification time as **total epoch microseconds** (signed),
rendered compactly with no intermediate string.

=== "Python"

    ```python
    from yggdryl.headers import Headers

    h = Headers()
    h.set_content_type("application/x-tar")
    h.set_content_encoding("gzip")
    assert h.mime_type().essence == "application/x-tar"          # primary
    assert h.media_type().essences() == ["application/x-tar", "application/gzip"]

    h.set_mtime(1_600_000_000_000_000)                           # epoch microseconds
    assert h.mtime() == 1_600_000_000_000_000
    ```

=== "Node"

    ```javascript
    const { Headers } = require('yggdryl').headers

    const h = new Headers()
    h.setContentType('application/x-tar')
    h.setContentEncoding('gzip')
    console.assert(h.mimeType().essence === 'application/x-tar')  // primary
    console.assert(JSON.stringify(h.mediaType().essences()) ===
      '["application/x-tar","application/gzip"]')

    h.setMtime(1600000000000000)                                 // epoch microseconds
    console.assert(h.mtime() === 1600000000000000)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::headers::Headers;

    let mut h = Headers::new();
    h.set_content_type("application/x-tar");
    h.set_content_encoding("gzip");
    assert_eq!(h.mime_type().unwrap().essence(), "application/x-tar"); // primary
    assert_eq!(h.media_type().unwrap().essences(),
               vec!["application/x-tar", "application/gzip"]);

    h.set_mtime(1_600_000_000_000_000); // epoch microseconds
    assert_eq!(h.mtime(), Some(1_600_000_000_000_000));
    ```

## Size and mtime sync

Any write that changes a source's bytes keeps its metadata in step in the same pass:
`set_content_length` renders the new byte length as a decimal straight into the `Content-Length`
entry (alloc-free, no `String` temporary), `content_length` reads it back (whitespace-trimmed,
absent when the value is non-numeric), and `touch_mtime` stamps the current time as total epoch
microseconds into the `mtime` header — the size and timestamp halves of the same header sync.

=== "Python"

    ```python
    from yggdryl.headers import Headers

    h = Headers()
    h.set_content_length(1024)                    # decimal rendered into Content-Length
    assert h.get("content-length") == "1024"
    assert h.content_length() == 1024             # read back, whitespace-trimmed

    h.touch_mtime()                               # stamp now as epoch microseconds
    assert h.mtime() > 0
    ```

=== "Node"

    ```javascript
    const { Headers } = require('yggdryl').headers

    const h = new Headers()
    h.setContentLength(1024)                       // decimal rendered into Content-Length
    console.assert(h.get('content-length') === '1024')
    console.assert(h.contentLength() === 1024)     // read back, whitespace-trimmed

    h.touchMtime()                                 // stamp now as epoch microseconds
    console.assert(h.mtime() > 0)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::headers::Headers;

    let mut h = Headers::new();
    h.set_content_length(1024); // decimal rendered in-place
    assert_eq!(h.get("content-length").as_deref(), Some("1024")); // rendered from the u64 field
    assert_eq!(h.content_length(), Some(1024)); // read back, whitespace-trimmed

    h.touch_mtime(); // stamp now as epoch microseconds
    assert!(h.mtime().unwrap() > 0);
    ```
