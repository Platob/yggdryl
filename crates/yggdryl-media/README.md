# yggdryl-media

Media (MIME) type detection for the
[**yggdryl**](https://github.com/Platob/yggdryl) project, built on the
[`yggdryl-core`](https://crates.io/crates/yggdryl-core) parsing traits
(`FromInput` / `ToOutput`).

`MediaType` is an enum of common media types (with an `Other` fallback for
anything else). It parses from a MIME string, infers from a file extension, or
sniffs a file's leading bytes — recognising container and columnar formats such
as Apache Arrow IPC, Parquet, ZIP and gzip.

```rust
use yggdryl_media::{FromInput, MediaType, ToOutput};

// From a MIME string (parameters are dropped, case-insensitive).
assert_eq!(MediaType::from_str("text/html; charset=utf-8", true).unwrap(), MediaType::Html);

// From a file extension.
assert_eq!(MediaType::from_extension("parquet"), Some(MediaType::Parquet));

// From magic bytes (content sniffing).
assert_eq!(MediaType::from_magic(b"PK\x03\x04..."), Some(MediaType::Zip));
assert_eq!(MediaType::from_magic(b"ARROW1\x00\x00"), Some(MediaType::Arrow));

// Components and rendering.
let png = MediaType::Png;
assert_eq!((png.type_(), png.subtype()), ("image", "png"));
assert_eq!(png.extension(), Some("png"));
assert_eq!(png.to_mapping().get("subtype"), Some(&"png".to_string()));
```
