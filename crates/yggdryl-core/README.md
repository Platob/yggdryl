# yggdryl-core

The Rust core foundations of **yggdryl**.

> **Project reset.** The implementation was removed and will be reimplemented
> around an Arrow-centralized design. See `CLAUDE.md` at the repository root for
> contributor rules.

## Serialization

The `Charset` trait encodes text to bytes and back through `encode_bytes` /
`decode_bytes`; `Utf8` and `Latin1` implement it. Behind the off-by-default
`json` feature, the `Base` trait gives every value type content-based JSON
serialization — as a string and as a canonical compact-UTF-8-JSON byte form
(encoded through the `Utf8` charset):

```rust
use serde::{Deserialize, Serialize};
use yggdryl_core::Base;

#[derive(Serialize, Deserialize)]
struct Point {
    x: i32,
    y: i32,
}
impl Base for Point {}

let p = Point { x: 1, y: 2 };
let _json = p.serialize_json()?;    // {"x":1,"y":2}
let _bytes = p.serialize_bytes()?;  // compact UTF-8 JSON
# Ok::<(), yggdryl_core::BaseError>(())
```

Enable it with `features = ["json"]`.

## Positioned I/O

`IOBase<T>` reads and writes `T` elements at a `position` measured from a `Whence`
(`Start`, `Current`, or `End`). Implement the two array primitives and the
single-element `pread_one` / `pwrite_one` come free from their defaults:

```rust
use yggdryl_core::{IOBase, Whence};

fn head<S: IOBase<u8>>(store: &mut S) -> Result<u8, yggdryl_core::IOError> {
    store.pwrite_array(0, Whence::Start, &[1, 2, 3])?;
    store.pread_one(0, Whence::Start)
}
```
