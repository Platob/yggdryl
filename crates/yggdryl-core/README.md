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

`RawIOBase` reads and writes bytes (`u8`) or bits (`bool`), one or many at a time, at
a `position` measured from a `Whence` (`Start`, `Current`, or `End`) — counted in
bytes for the `*_byte_*` methods and in bits (MSB-first) for the `*_bit_*` methods.
Every resource is `Seekable` (`RawIOBase: Seekable`): `tell` reports the cursor and
`seek` moves it, and `Whence::Current` addresses relative to that cursor. Implement
the four array primitives and the `*_one` methods come free from their defaults:

```rust
use yggdryl_core::{RawIOBase, Whence};

fn first_byte<S: RawIOBase>(store: &mut S) -> Result<u8, yggdryl_core::IOError> {
    store.pwrite_byte_array(0, Whence::Start, &[1, 2, 3])?;
    store.pread_byte_one(0, Whence::Start)
}
```

`IOBase<T>: RawIOBase` layers typed values on top: implement `value_to_bytes` (how a
`T` becomes bytes), `size` and `resize` (in items), and the typed writes
`pwrite_one` / `pwrite_array` come free, serializing through it into the raw byte
methods. Both traits report sizes and capacities (`byte_size` / `bit_size`,
`byte_capacity` / `bit_capacity`, `IOBase::size` / `capacity`), support resizing
(`resize_bytes` / `resize_bits` / `IOBase::resize`, plus capacity hints), and stream
between resources in chunks with `pread_io` / `pwrite_io` — a large transfer never
materializes in full.

`ByteBuffer` (byte-granular) and `BitBuffer` (exact bit length) are the concrete
in-memory resources; both are exposed in the Python and Node bindings. Benchmarks
live in `benches/buffers.rs` (`cargo bench`).

```rust
use yggdryl_core::{IOBase, RawIOBase, Whence};

fn store_u32<S: IOBase<u32>>(store: &mut S, value: u32) -> Result<(), yggdryl_core::IOError> {
    store.pwrite_one(0, Whence::Start, &value)
}
```
