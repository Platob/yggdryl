# Charset

!!! note "Rust core only"
    `Charset` is currently available in the **Rust core** (`yggdryl-core`). It gains
    Python and Node tabs when the bindings expose it.

The `Charset` trait converts text to bytes and back through `encode_bytes` /
`decode_bytes`. Two encodings implement it:

- **`Utf8`** — the native encoding; encoding is a copy of the string's bytes and
  decoding validates UTF-8.
- **`Latin1`** — ISO-8859-1: one byte per code point in `U+0000..=U+00FF`; encoding
  rejects any character above `U+00FF`, decoding always succeeds.

```rust
use yggdryl_core::{Charset, Latin1, Utf8};

fn main() {
    // UTF-8 encodes 'é' as two bytes; Latin-1 as one.
    assert_eq!(Utf8.encode_bytes("é").unwrap(), vec![0xC3, 0xA9]);
    assert_eq!(Latin1.encode_bytes("é").unwrap(), vec![0xE9]);
    assert_eq!(Latin1.decode_bytes(&[0xE9]).unwrap(), "é");

    // Each charset carries its name, used in diagnostics.
    assert_eq!(Utf8.name(), "UTF-8");
    assert_eq!(Latin1.name(), "ISO-8859-1");
}
```

A conversion that cannot be represented (or invalid bytes on decode) returns a
`CharsetError` naming the offending character or bytes:

```rust
use yggdryl_core::{Charset, CharsetError, Latin1};

fn main() {
    // 'Ω' (U+03A9) is above the Latin-1 range.
    let error = Latin1.encode_bytes("Ω").unwrap_err();
    assert!(matches!(error, CharsetError::Unrepresentable { ch: 'Ω', .. }));
}
```

## Implementing a charset

`Charset` has three methods — `name`, `encode_bytes`, and `decode_bytes`:

```rust
use yggdryl_core::{Charset, CharsetError};

struct Ascii;

impl Charset for Ascii {
    fn name(&self) -> &'static str {
        "US-ASCII"
    }
    fn encode_bytes(&self, text: &str) -> Result<Vec<u8>, CharsetError> {
        for (index, ch) in text.chars().enumerate() {
            if !ch.is_ascii() {
                return Err(CharsetError::Unrepresentable { charset: self.name(), index, ch });
            }
        }
        Ok(text.as_bytes().to_vec())
    }
    fn decode_bytes(&self, bytes: &[u8]) -> Result<String, CharsetError> {
        if let Some(index) = bytes.iter().position(|&b| b > 0x7F) {
            return Err(CharsetError::InvalidBytes {
                charset: self.name(),
                reason: format!("byte {:#04x} at index {index} is not US-ASCII", bytes[index]),
            });
        }
        Ok(String::from_utf8(bytes.to_vec()).expect("ascii is valid utf-8"))
    }
}

fn main() {
    assert_eq!(Ascii.encode_bytes("ok").unwrap(), b"ok".to_vec());
    assert!(Ascii.encode_bytes("é").is_err());
}
```
