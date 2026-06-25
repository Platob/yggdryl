# yggdryl-core

The consolidated foundations of the [**yggdryl**](https://github.com/Platob/yggdryl)
project. One crate now holds what used to be five separate ones
(`yggdryl-io`, `yggdryl-url`, `yggdryl-media`, `yggdryl-compression` and
`yggdryl-version`). The blocking HTTP client lives in the separate
[`yggdryl-http`](https://crates.io/crates/yggdryl-http) crate, which depends on
this one.

It provides:

- the `Mapping` / `Params` component maps and the `ToOutput` rendering trait,
  plus URL-safe percent-encoding (`percent_encode` / `percent_decode`) and the
  lower-level component helpers — parsing validates its input and returns an
  error on malformed data;
- the `Version` (`major.minor.patch`) value type, numerically ordered;
- the `MimeType` enum (backed by a mutable global registry of extensions/magic
  bytes) and the `MediaType` extension stack (e.g. `csv.gz` → `[Csv, Gzip]`);
- the `Uri` / `Url` value types (RFC 3986), with query-parameter CRUD and an
  inferred `media_type()` accessor;
- the **byte-IO foundation** — the `Io` handle trait, the in-memory `BytesIO`,
  the filesystem `LocalPath`, the `Codec` / `Frames` value coders, and the
  `from_str` / `register_scheme` scheme-dispatch factory;
- streamed byte `Compression` (gzip / Zstandard / Snappy / identity) over any
  `Io` handle, plus the `CompressIo` extension trait.

```rust
use yggdryl_core::{percent_encode, percent_decode, Version, Url, BytesIO, Io, Whence};

assert_eq!(percent_encode("a b"), "a%20b");
assert_eq!(percent_decode("a%20b").unwrap(), "a b");

assert!(Version::from_str("1.4.2").unwrap() < Version::from_str("1.10.0").unwrap());

let url = Url::from_str("https://example.com/data.csv.gz").unwrap();
assert_eq!(url.host(), "example.com");

let mut io = BytesIO::from_bytes(b"hello world".to_vec());
let mut footer = [0u8; 5];
io.pread(&mut footer, 6, Whence::Start).unwrap();
assert_eq!(&footer, b"world");
```

## Features

Heavy dependencies are optional. The compression codec backends ship on by
default; everything else is off:

| feature | effect |
| --- | --- |
| `gzip` / `zstd` / `snappy` (default) | the matching streamed compression backend |
| `mmap` | `LocalPath` memory-maps files (zero-copy) |
| `media` | `media_type()` discovery on `Io` / `Path` and codec inference from stats |
| `json` | `Io::json()` — a zero-copy parse of a handle's bytes |
| `log` | structured `log` events on the hot paths |

The `media` module is always compiled (the URL types reference it
unconditionally); the `media` feature only gates the `media_type()` /
`from_mime` / `from_media` / `from_stats` accessors. The compression module is
always compiled too; only the codec *backends* are gated by `gzip` / `zstd` /
`snappy` (an unavailable codec still parses and names itself, but reports
`Unsupported` on encode/decode).
