# yggdryl-media

Media (MIME) type detection for the
[**yggdryl**](https://github.com/Platob/yggdryl) project, built on the
[`yggdryl-core`](https://crates.io/crates/yggdryl-core) parsing traits
(`FromInput` / `ToOutput`).

- `MimeType` is an enum of common, individual MIME types (with an `Other`
  fallback). Each type's MIME string, file extensions and magic-byte signatures
  live in a **global registry** that can be extended or trimmed at runtime.
- `MediaType` is an ordered stack of `MimeType`s describing a layered file, so
  `data.csv.gz` becomes `MediaType([MimeType::Csv, MimeType::Gzip])`.

```rust
use yggdryl_media::{FromInput, MediaType, MimeType, Signature};

// A single MIME type, parsed from a full MIME, a short name, an extension, or
// sniffed from magic bytes.
assert_eq!(MimeType::from_str("application/json").unwrap(), MimeType::Json);
assert_eq!(MimeType::from_str("zstd").unwrap(), MimeType::Zstd); // short name
assert_eq!(MimeType::from_extension("parquet"), Some(MimeType::Parquet));
assert_eq!(MimeType::from_magic(b"ARROW1\x00\x00"), Some(MimeType::Arrow));

// A layered file is an ordered stack, innermost content first — build it from a
// path, a list of extensions, or an explicit list of types.
let stack = MediaType::from_path("data.csv.gz");
assert_eq!(stack.types(), [MimeType::Csv, MimeType::Gzip]);
assert_eq!(stack, MediaType::from_extensions(&["csv", "gz"]));
assert_eq!(stack.first(), Some(&MimeType::Csv));
assert_eq!(MimeType::from_path("data.csv.gz"), Some(MimeType::Gzip)); // outermost

// The registry is global and mutable.
MimeType::register("application/x-foo", &["foo"], &[Signature::prefix(b"FOO1")]);
assert_eq!(
    MimeType::from_extension("foo"),
    Some(MimeType::Other("application/x-foo".to_string()))
);
MimeType::unregister("application/x-foo");

// `application/octet-stream` is the default fallback when nothing is inferred.
assert_eq!(MimeType::default(), MimeType::OctetStream);
assert_eq!(MimeType::from_path("notes").unwrap_or_default(), MimeType::OctetStream);
```
