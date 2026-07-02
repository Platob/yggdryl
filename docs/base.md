# Serialization

!!! note "Rust core only, behind the `json` feature"
    The `Base` trait lives in the Rust core behind the off-by-default `json` cargo
    feature (`features = ["json"]`), which pulls in `serde` and `serde_json`. It gains
    Python and Node tabs when the bindings expose a value type that implements it.

`Base` is the trait every value type implements. The JSON string form is free from
`serde`, so a type gets it by deriving `Serialize` / `Deserialize`; the canonical
**byte** form is a compact binary layout the type defines itself (never JSON).

| Method | Kind | Result |
| --- | --- | --- |
| `serialize_json` / `deserialize_json` | default (serde) | a JSON string |
| `serialize_bytes` / `deserialize_bytes` | **required** | the type's compact byte form |

```rust
use serde::{Deserialize, Serialize};
use yggdryl_core::{Base, BaseError};

#[derive(Debug, PartialEq, Serialize, Deserialize)]
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

fn main() {
    let p = Point { x: 1, y: 2 };

    // Content JSON is free from serde.
    assert_eq!(p.serialize_json().unwrap(), r#"{"x":1,"y":2}"#);
    assert_eq!(Point::deserialize_json(&p.serialize_json().unwrap()).unwrap(), p);

    // The byte form is the type's own compact binary layout — not JSON.
    assert_eq!(p.serialize_bytes().unwrap(), vec![1, 0, 0, 0, 2, 0, 0, 0]);
    assert_eq!(Point::deserialize_bytes(&p.serialize_bytes().unwrap()).unwrap(), p);
}
```

`deserialize_bytes` validates its input fully and is the exact inverse of
`serialize_bytes`; a decode error surfaces as `BaseError::InvalidBytes`.
