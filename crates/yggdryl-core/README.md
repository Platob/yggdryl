# yggdryl-core

The Rust core foundations of **yggdryl**.

> **Project reset.** The implementation was removed and will be reimplemented
> around an Arrow-centralized design. See `CLAUDE.md` at the repository root for
> contributor rules.

## Serialization

The `Charset` trait encodes text to bytes and back through `encode_bytes` /
`decode_bytes`; `Utf8` and `Latin1` implement it. Behind the off-by-default
`json` feature, the `Base` trait gives every value type content-based JSON
serialization — as a string, as encoded bytes (with any `Charset`), and as a
canonical compact-UTF-8-JSON byte form:

```rust
use serde::{Deserialize, Serialize};
use yggdryl_core::{Base, Latin1, Utf8};

#[derive(Serialize, Deserialize)]
struct Point {
    x: i32,
    y: i32,
}
impl Base for Point {}

let p = Point { x: 1, y: 2 };
let _json = p.to_json()?;                  // {"x":1,"y":2}
let _pretty = p.to_bson(Some(2), Utf8)?;   // indented JSON bytes, UTF-8
let _latin1 = p.to_bson(None, Latin1)?;    // compact JSON bytes, Latin-1
let _bytes = p.serialize_bytes()?;         // compact UTF-8 JSON
# Ok::<(), yggdryl_core::BaseError>(())
```

Enable it with `features = ["json"]`.
