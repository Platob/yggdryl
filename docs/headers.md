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

    assert_eq!(h.get("content-type"), Some("application/json")); // case-insensitive
    assert_eq!(h.get_all("set-cookie"), vec!["a=1", "b=2"]);      // multi-value
    assert_eq!(Headers::deserialize_bytes(&h.serialize_bytes()).unwrap(), h);
    ```

Every byte source carries one — see
[`headers()` on the memory contract](io/memory.md#metadata-mode-and-kind).
