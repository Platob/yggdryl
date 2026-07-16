# Headers

`Headers` is the project's **one** string key/value map — an ordered, **case-insensitive**,
**multi-value** (a name may repeat) collection of byte-string keys and values, kept in insertion
order and following HTTP header conventions. It plays two roles with a single type:

- an **HTTP header block** — with the common field constants, `Content-Type` / `Content-Length`
  helpers, and an HTTP text render/parse; and
- the **centralized metadata holder** — the map a [`Field`](schema.md) carries (there is no
  separate `Metadata` type), mirroring Arrow's `Field::metadata`.

It lives in the Rust core (`yggdryl_core::io::Headers`) and is mirrored, method-for-method, in
**Python** (`yggdryl.io.Headers`) and **Node** (`yggdryl.io.Headers`). Like `dict` / `bytearray`
it is a mutable container, so it compares by content but is not hashable (a `Field` that embeds
it still hashes, via the core's byte-canonical field hash).

## Reading and writing

`insert` **replaces** every entry with a name (HTTP set semantics); `append` **keeps** them
(multi-value); `remove` drops **all** matches and returns how many. Names are matched
case-insensitively.

=== "Python"

    ```python
    from yggdryl.io import Headers

    h = Headers()
    h.insert("Content-Type", "application/json")   # replace-set
    h.append("Set-Cookie", "a=1")                  # multi-value append
    h.append("set-cookie", "b=2")                  # name is case-insensitive

    assert h.get("content-type") == "application/json"
    assert h.get_all("Set-Cookie") == ["a=1", "b=2"]
    assert "CONTENT-TYPE" in h

    h.insert("Set-Cookie", "c=3")                  # replaces both
    assert h.get_all("set-cookie") == ["c=3"]
    assert h.remove("set-cookie") == 1             # count removed
    ```

=== "Node"

    ```js
    const { Headers } = require('yggdryl').io

    const h = new Headers()
    h.insert('Content-Type', 'application/json')   // replace-set
    h.append('Set-Cookie', 'a=1')                  // multi-value append
    h.append('set-cookie', 'b=2')                  // name is case-insensitive

    assert(h.get('content-type') === 'application/json')
    assert.deepEqual(h.getAll('Set-Cookie'), ['a=1', 'b=2'])
    assert(h.has('CONTENT-TYPE'))

    h.insert('Set-Cookie', 'c=3')                  // replaces both
    assert.deepEqual(h.getAll('set-cookie'), ['c=3'])
    assert(h.remove('set-cookie') === 1)           // count removed
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::Headers;

    let mut h = Headers::new();
    h.insert(Headers::CONTENT_TYPE, "application/json"); // replace-set
    h.append("Set-Cookie", "a=1");                       // multi-value append
    h.append("set-cookie", "b=2");                       // name is case-insensitive

    assert_eq!(h.get("content-type"), Some("application/json"));
    assert_eq!(h.get_all("Set-Cookie"), vec!["a=1", "b=2"]);
    assert!(h.contains("CONTENT-TYPE"));

    h.insert("Set-Cookie", "c=3");                       // replaces both
    assert_eq!(h.get_all("set-cookie"), vec!["c=3"]);
    assert_eq!(h.remove("set-cookie"), 1);               // count removed
    ```

Equality is **order-significant** (insertion order is part of the value), so a `Headers` built
from a map/object preserves that literal's key order — `{a, b}` and `{b, a}` are *not* equal.

In the Rust core, values that are not UTF-8 are reached through the `*_bytes` accessors, and the
map stays aware of them (`append_bytes` / `get_bytes`); the `&str` accessors return `None` for a
non-UTF-8 value. The lossless byte codec in the Serialization section below round-trips those
bytes in every language.

## Common fields

Constants name the common HTTP headers (canonical casing, matched case-insensitively), and a
couple of typed helpers parse them:

=== "Python"

    ```python
    from yggdryl.io import Headers

    h = Headers()
    h.insert(Headers.CONTENT_TYPE, "text/html; charset=utf-8")
    h.insert(Headers.CONTENT_LENGTH, "1024")
    assert h.content_type == "text/html; charset=utf-8"
    assert h.content_length == 1024
    ```

=== "Node"

    ```js
    const { Headers } = require('yggdryl').io

    const h = new Headers()
    h.insert('Content-Type', 'text/html; charset=utf-8')
    h.insert('Content-Length', '1024')
    assert(h.contentType === 'text/html; charset=utf-8')
    assert(h.contentLength === 1024)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::Headers;

    let mut h = Headers::new();
    h.insert(Headers::CONTENT_TYPE, "text/html; charset=utf-8");
    h.insert(Headers::CONTENT_LENGTH, "1024");
    assert_eq!(h.content_type(), Some("text/html; charset=utf-8"));
    assert_eq!(h.content_length(), Some(1024));
    ```

Available names (as `Headers.CONTENT_TYPE` in Rust / Python; plain string literals in Node)
include `CONTENT_TYPE`, `CONTENT_LENGTH`, `CONTENT_ENCODING`, `HOST`, `ACCEPT`,
`ACCEPT_ENCODING`, `AUTHORIZATION`, `USER_AGENT`, `LOCATION`, `CONNECTION`, `CACHE_CONTROL`,
`COOKIE`, and `SET_COOKIE`.

## Serialization — HTTP text and binary

`to_http_bytes` / `parse_http` render and read the HTTP wire form (`Name: Value\r\n` per entry,
tolerating `\n` or `\r\n`, trimming optional whitespace, stopping at the blank line). For a
robust round-trip of **arbitrary** bytes and multi-value entries — names or values that contain
`:` or `\r\n` — `serialize_bytes` / `deserialize_bytes` use a length-prefixed binary form (the
exact inverse pair, and the codec pickle / `Field` metadata rides on).

=== "Python"

    ```python
    from yggdryl.io import Headers

    h = Headers()
    h.insert("Host", "example.com")
    h.append("Accept", "text/html")

    # HTTP text form.
    assert h.to_http_bytes() == b"Host: example.com\r\nAccept: text/html\r\n"
    assert Headers.parse_http(h.to_http_bytes()) == h

    # Binary form — round-trips arbitrary bytes and multi-value entries.
    assert Headers.deserialize_bytes(h.serialize_bytes()) == h
    ```

=== "Node"

    ```js
    const { Headers } = require('yggdryl').io

    const h = new Headers()
    h.insert('Host', 'example.com')
    h.append('Accept', 'text/html')

    // HTTP text form.
    assert(h.toHttpBytes().toString('utf8') === 'Host: example.com\r\nAccept: text/html\r\n')
    assert(Headers.parseHttp(h.toHttpBytes()).equals(h))

    // Binary form — round-trips arbitrary bytes and multi-value entries.
    assert(Headers.deserializeBytes(h.serializeBytes()).equals(h))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::Headers;

    let mut h = Headers::new();
    h.insert("Host", "example.com");
    h.append("Accept", "text/html");

    // HTTP text form.
    assert_eq!(h.to_http_bytes(), b"Host: example.com\r\nAccept: text/html\r\n");
    assert_eq!(Headers::parse_http(&h.to_http_bytes()), h);

    // Binary form — round-trips arbitrary bytes and multi-value entries.
    assert_eq!(Headers::deserialize_bytes(&h.serialize_bytes()).unwrap(), h);
    ```

The core also exposes streaming `write_to` / `read_from` over the [`IOCursor`](bytes.md)
abstraction (`serialize_bytes` is the convenience over a `Bytes` sink), so a header map
serializes to any byte sink.

## As field metadata

A [`Field`](schema.md)'s metadata **is** a `Headers`. Build it from a plain map/object or a
`Headers` value, and read it back with these same accessors:

```python
from yggdryl.io import Headers
from yggdryl.types import DataType, Field

field = Field("t", DataType.f64(), metadata={"unit": "seconds"})
assert field.metadata.get("unit") == "seconds"
assert field.metadata == Headers({"unit": "seconds"})
```

Lookups are a **zero-allocation** case-insensitive scan of the compact, insertion-ordered
entries — for the small `n` of a real header (or metadata) set that beats hashing and preserves
order and duplicates exactly (see the
[benchmark notes](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/headers.md)).
