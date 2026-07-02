# yggdryl-core

The Rust core foundations of **yggdryl**.

> **Project reset.** The implementation was removed and will be reimplemented
> around an Arrow-centralized design. See `CLAUDE.md` at the repository root for
> contributor rules.

## Serialization

The `Charset` trait encodes text to bytes and back through `encode_bytes` /
`decode_bytes`; `Utf8` and `Latin1` implement it.

Behind the off-by-default `json` feature, the `Base` trait gives every value type a
JSON string form for free from `serde` (`serialize_json` / `deserialize_json`) plus a
required compact binary byte form (`serialize_bytes` / `deserialize_bytes`) that each
type defines itself — never JSON:

```rust
use serde::{Deserialize, Serialize};
use yggdryl_core::{Base, BaseError};

#[derive(Serialize, Deserialize)]
struct Point {
    x: i32,
    y: i32,
}

impl Base for Point {
    // JSON is free from serde; define the compact byte layout yourself.
    fn serialize_bytes(&self) -> Result<Vec<u8>, BaseError> {
        let mut out = Vec::with_capacity(8);
        out.extend_from_slice(&self.x.to_le_bytes());
        out.extend_from_slice(&self.y.to_le_bytes());
        Ok(out)
    }
    fn deserialize_bytes(bytes: &[u8]) -> Result<Self, BaseError> {
        let a: [u8; 8] = bytes.try_into().map_err(|_| BaseError::InvalidBytes {
            reason: format!("expected 8 bytes, got {}", bytes.len()),
        })?;
        Ok(Point {
            x: i32::from_le_bytes([a[0], a[1], a[2], a[3]]),
            y: i32::from_le_bytes([a[4], a[5], a[6], a[7]]),
        })
    }
}

// p.serialize_json()?  -> {"x":1,"y":2}       (free, JSON text)
// p.serialize_bytes()? -> [1, 0, 0, 0, 2, 0, 0, 0]  (your binary layout)
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
