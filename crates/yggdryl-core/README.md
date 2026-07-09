# yggdryl-core

The Rust core foundations of **yggdryl**.

> **Project reset.** The implementation was removed and is being rebuilt around an
> Apache Arrow-centralized design. Only the hello-world skeleton and the contributor
> rules (`CLAUDE.md` at the repository root) remain.

Alongside the clean-slate `hello` / `version` entry points, the crate hosts the
`codec` foundations and the `compression` specialisation:

```rust
use yggdryl_core::{Decoder, Encoder, Gzip};

fn main() {
    let gzip = Gzip::default(); // gzip codec at level 6
    let packed = gzip.encode_byte_array(b"hello hello hello").unwrap();
    let restored = gzip.decode_byte_array(&packed).unwrap();
    assert_eq!(restored, b"hello hello hello");
}
```

- **`codec`** — the [`Encoder`] / [`Decoder`] byte-array contracts and their
  element-generic [`TypedEncoder<T>`] / [`TypedDecoder<T>`] extensions, plus the
  shared `EncodeError` / `DecodeError` types.
- **`compression`** — the [`Compression`] / [`CompressionEncoder`] /
  [`CompressionDecoder`] contracts (and their `Typed*` variants), the concrete
  [`Gzip`] and `Zstd` codecs (default-on `gzip` / `zstd` features), and the
  `CompressIO` extension that compresses/decompresses an IO with a codec.
- **`io`** — the positioned byte-IO surface: the `IOBase` / `TypedIOBase<T>` cursor
  contracts and the concrete `ByteBuffer` storage plus its advancing `ByteCursor`,
  with zero-copy Arrow interop under the default-on `arrow` feature.
- **`buffer`** — immutable, cheaply-shared typed buffers for the native primitives
  (`I8Buffer` … `F64Buffer`) plus the bit-packed `BooleanBuffer`, round-tripping
  through little-endian bytes and wrapping the matching Arrow buffer zero-copy.

The codec/compression traits are Rust-only (generics and marker traits do not
cross the FFI boundary); the Python and Node bindings expose the concrete codecs.

Add further foundational types here as the design lands — one module per concern,
each re-exported at the crate root — following the rules in `CLAUDE.md`.

[`Encoder`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/trait.Encoder.html
[`Decoder`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/trait.Decoder.html
[`TypedEncoder<T>`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/trait.TypedEncoder.html
[`TypedDecoder<T>`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/trait.TypedDecoder.html
[`Compression`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/trait.Compression.html
[`CompressionEncoder`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/trait.CompressionEncoder.html
[`CompressionDecoder`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/trait.CompressionDecoder.html
[`Gzip`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/struct.Gzip.html
