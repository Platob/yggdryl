# Headers

`yggdryl-http` provides **`Headers`** — a generic, ordered **bytes → bytes** map modelled
on an HTTP header block. A [field](field.md) or a [buffer](buffer.md) carries an optional
`Headers` as its annotations, and hands it out via the `HeadersBased` trait.

`Headers` offers, over one `BTreeMap<Vec<u8>, Vec<u8>>`:

- **byte** and UTF-8 **string** accessors/mutators (`get` / `insert` / `remove`,
  `get_str` / `set_str` / `remove_str`);
- **zero-copy** in-place value mutation via `get_mut` — extend or patch a value's bytes
  without cloning the map or re-inserting;
- pre-built accessors for the common keys `name`, `comment`, `content-type`,
  `content-encoding`;
- a deterministic byte round-trip (`serialize_bytes` / `deserialize_bytes`), so two maps
  are equal **iff** their serialised bytes are equal.

Because the keys and values are arbitrary bytes (not valid-UTF-8-only like Arrow's field
metadata), headers stay **yggdryl-side** — they are not written into Arrow's `Field`.

!!! note "Binding surface"
    In the bindings, a buffer's / field's headers are exposed as the whole-map `headers`
    (Python `dict[bytes, bytes]`; Node `Array<{key: Buffer, value: Buffer}>` — JS cannot
    key a map by bytes) and `with_headers`. The full per-key / string / common-key surface
    (`get_header`, `set_content_type`, zero-copy `get_mut`, …) lives on the Rust
    `Headers` / `HeadersBased` types.

## The header map (Rust)

```rust
use yggdryl_http::Headers;

let mut headers = Headers::new();

// String and common-key mutators.
headers.set_str("unit", "ms");
headers.set_content_type("text/plain");
assert_eq!(headers.content_type(), Some(b"text/plain".as_slice()));

// Zero-copy: patch a value's bytes in place — no map clone, no re-insert.
headers.get_mut(Headers::CONTENT_TYPE).unwrap().extend_from_slice(b"; charset=utf-8");
assert_eq!(headers.content_type(), Some(b"text/plain; charset=utf-8".as_slice()));

// Deterministic byte round-trip.
assert_eq!(Headers::deserialize_bytes(&headers.serialize_bytes()).unwrap(), headers);
```

## Carried by fields and buffers

A header-carrying type implements `HeadersBased`, which delegates the whole
get / add / update / delete surface (plus the common-key conveniences and the builder) to
the stored map.

=== "Python"

    ```python
    from yggdryl.buffer import I64Buffer

    buf = I64Buffer([1, 2, 3]).with_headers({b"content-type": b"application/x.int64"})
    assert buf.headers == {b"content-type": b"application/x.int64"}

    field = buf.field("v", True)            # an I64Field, carrying the headers
    assert field.headers == {b"content-type": b"application/x.int64"}
    ```

=== "Node"

    ```js
    const { I64Buffer } = require('yggdryl').buffer

    const entries = [{ key: Buffer.from('content-type'), value: Buffer.from('application/x.int64') }]
    const buf = new I64Buffer([1, 2, 3]).withHeaders(entries)

    const field = buf.field('v', true)      // an I64Field, carrying the headers
    console.assert(field.headers[0].value.equals(Buffer.from('application/x.int64')))
    ```

=== "Rust"

    ```rust
    use yggdryl_buffer::I64Buffer;
    use yggdryl_http::HeadersBased;

    let mut buf = I64Buffer::from_slice(&[1, 2, 3]);
    buf.set_content_type("application/x.int64");   // pre-built common-key mutator
    // Carried into the field the buffer hands out.
    assert_eq!(
        buf.field("v", true).content_type(),
        Some(b"application/x.int64".as_slice()),
    );
    ```

## Benchmarks

`Headers` serialize / deserialize and the get / set / in-place-mutate hot paths have a
throughput bench (`cargo bench -p yggdryl-http --bench headers`). The **in-place
`get_mut` mutation is markedly faster than a re-insert** (no map lookup churn / value
re-allocation), which is the point of the zero-copy path. See the
[report](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-http/http/headers.md).
