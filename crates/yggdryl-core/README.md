# yggdryl-core

The Rust core foundations of **yggdryl**.

> **Project reset.** The implementation was removed and will be reimplemented
> around an Arrow-centralized design. See `CLAUDE.md` at the repository root for
> contributor rules.

## Serialization

`Charset` encodes text to bytes and back (UTF-8 by default; also UTF-16 LE/BE,
Latin-1, and ASCII). Behind the off-by-default `json` feature, the `Base` trait
gives every value type content-based JSON serialization — as a string, as encoded
bytes, and as a canonical compact-UTF-8-JSON byte form:

```rust
use serde::{Deserialize, Serialize};
use yggdryl_core::{Base, Charset};

#[derive(Serialize, Deserialize)]
struct Point {
    x: i32,
    y: i32,
}
impl Base for Point {}

let p = Point { x: 1, y: 2 };
let _json = p.to_json()?;                        // {"x":1,"y":2}
let _pretty = p.to_bson(Some(2), Charset::Utf8)?; // indented JSON bytes
let _bytes = p.to_bytes()?;                       // compact UTF-8 JSON
# Ok::<(), yggdryl_core::BaseError>(())
```

Enable it with `features = ["json"]`.
