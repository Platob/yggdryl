# yggdryl-compression

Streamed byte **compression** — gzip, Zstandard or Snappy — layered on top of the
[**yggdryl-io**](../yggdryl-io) handle abstraction, part of the
[**yggdryl**](https://github.com/Platob/yggdryl) project. A `Compression` codec
wraps any `Io` handle (or raw `ReadBytes` / `WriteBytes`) to compress and
decompress **a chunk at a time**, never buffering the whole payload.

## What it offers

- `Compression` — `None` (identity / store), `Gzip`, `Zstd`, `Snappy`. Parse with
  `from_str` (`gzip`/`gz`, `zstd`/`zst`, `snappy`/`snap`/`sz`, `none`) or
  `from_extension`; name with `as_str` / `extension`. `is_available` reports
  whether a codec's backend is compiled in.
- `encoder(sink)` → an `Encoder` implementing `WriteBytes` (compress-on-write;
  `finish()` flushes the trailer and recovers the sink); `decoder(source)` → a
  `Decoder` implementing `ReadBytes` (decompress-on-read). One-shot `compress` /
  `decompress` (`&[u8] -> Vec<u8>`) build on top.
- `CompressIo` — an extension trait, blanket-implemented for every `Io`, adding
  `compress(codec)` and `decompress(codec)` straight onto a handle, returning a
  fresh in-memory `BytesIO`. On `decompress` the codec may be left to inference
  (`None`): the handle's URL extension first, then (under `media`) its
  `stats()` media / content type.
- Inference helpers (under `media`): `from_mime`, `from_media`, `from_stats`.

```rust
use yggdryl_compression::{Compression, CompressIo};
use yggdryl_io::BytesIO;

let codec = Compression::from_str("zstd").unwrap();

// One-shot over a slice.
# #[cfg(feature = "zstd")]
# {
let packed = codec.compress(b"hello hello hello").unwrap();
assert_eq!(codec.decompress(&packed).unwrap(), b"hello hello hello");

// Or stream a whole Io handle into a compressed BytesIO and back.
let mut src = BytesIO::from_bytes(b"payload bytes".to_vec());
let mut gz = src.compress(codec).unwrap();
assert_eq!(gz.decompress(Some(codec)).unwrap().getvalue(), b"payload bytes");
# }
```

## Features (off by default)

- `gzip` / `zstd` / `snappy` — the matching backend (`flate2` / `zstd` / `snap`).
  A codec whose feature is off still parses and names itself but reports
  `Unsupported` on encode/decode.
- `media` — infer a codec from an `Io`'s MIME / media type via `yggdryl-media`.
- `log` — structured `log` events on the hot paths.

Run the benchmarks with `cargo bench -p yggdryl-compression --all-features`
(reports compression ratio and one-shot vs streamed MiB/s per codec).
