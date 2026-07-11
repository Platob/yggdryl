# Compression

`yggdryl-core` centralises byte encoding behind a small **codec** hierarchy and
specialises it for lossless **compression**.

- [`Encoder`] / [`Decoder`] are the base byte-array contracts —
  `encode_byte_array` / `decode_byte_array`.
- [`TypedEncoder<T>`] / [`TypedDecoder<T>`] generalise them to arrays of an
  arbitrary element type `T` (`encode` / `decode`); the `T = u8` case is exactly
  the base traits.
- [`Compression`] (a [`CompressionEncoder`] + [`CompressionDecoder`], plus the
  element-generic [`TypedCompressionEncoder<T>`] / [`TypedCompressionDecoder<T>`])
  marks a codec that compresses losslessly and exposes its `name`.

!!! note "The traits are Rust-only"
    The `Encoder` / `Decoder` / `Compression` traits and their generic `Typed*`
    variants are generic Rust contracts that cannot cross the FFI boundary, so the
    **Python and Node bindings expose only the concrete codecs** — [`Gzip`] and
    [`Zstd`]. This is a deliberate, documented omission per the replication rule in
    `CLAUDE.md`.

Two codecs ship by default: **gzip** (`flate2`) and **zstd** (`zstd`). Both have the
same surface — `encode_byte_array` / `decode_byte_array`, the streaming
`compress_stream` / `decompress_stream`, `name`, `level`, and `serialize_bytes`.

## Gzip

The gzip (RFC 1952) codec is built on `flate2`, behind the `gzip` cargo feature
(on by default). Construct it with a compression level in `0..=9` (`0` = store,
`9` = best; default `6`), then compress with `encode_byte_array` and decompress
with `decode_byte_array`.

!!! tip "Fast backend"
    The Python and Node extensions build with the **`gzip-zlib-ng`** feature — a
    SIMD-optimised zlib-ng backend that is **~5.6× faster than stdlib gzip / zlib on
    level-6 encode** (the default). It needs CMake + Ninja at build time; run
    `uv run python scripts/setup-build-deps.py`. Pure-Rust builds default to the
    toolchain-free `miniz_oxide` backend. See the
    [benchmark report](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-compression/compression/gzip.md).

=== "Python"

    ```python
    from yggdryl import compression

    gzip = compression.Gzip()          # level 6 by default
    assert gzip.name == "gzip"

    data = b"the quick brown fox" * 8
    packed = gzip.encode_byte_array(data)
    assert gzip.decode_byte_array(packed) == data
    ```

=== "Node"

    ```js
    const { compression } = require('yggdryl')

    const gzip = new compression.Gzip()   // level 6 by default
    console.log(gzip.name)                 // -> gzip

    const data = Buffer.from('the quick brown fox'.repeat(8))
    const packed = gzip.encodeByteArray(data)
    console.assert(gzip.decodeByteArray(packed).equals(data))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{Decoder, Encoder, Gzip};

    let gzip = Gzip::default(); // level 6 by default
    assert_eq!(yggdryl_core::Compression::name(&gzip), "gzip");

    let data = b"the quick brown fox".repeat(8);
    let packed = gzip.encode_byte_array(&data).unwrap();
    assert_eq!(gzip.decode_byte_array(&packed).unwrap(), data);
    ```

### Choosing a level

An out-of-range level is rejected at construction (a Python `ValueError` / a
thrown `Error` / an `EncodeError::InvalidLevel`).

=== "Python"

    ```python
    from yggdryl import compression

    best = compression.Gzip(9)
    assert best.level == 9
    ```

=== "Node"

    ```js
    const { compression } = require('yggdryl')

    const best = new compression.Gzip(9)
    console.assert(best.level === 9)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::Gzip;

    let best = Gzip::new(9).unwrap();
    assert_eq!(best.level(), 9);
    ```

### Serializing the codec

A `Gzip` value round-trips through bytes (the single level byte) — Python via
`pickle` too.

=== "Python"

    ```python
    import pickle
    from yggdryl import compression

    gzip = compression.Gzip(3)
    restored = compression.Gzip.deserialize_bytes(gzip.serialize_bytes())
    assert restored.level == 3
    assert pickle.loads(pickle.dumps(gzip)).level == 3
    ```

=== "Node"

    ```js
    const { compression } = require('yggdryl')

    const gzip = new compression.Gzip(3)
    const restored = compression.Gzip.deserializeBytes(gzip.serializeBytes())
    console.assert(restored.level === 3)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::Gzip;

    let gzip = Gzip::new(3).unwrap();
    let restored = Gzip::deserialize_bytes(&gzip.serialize_bytes()).unwrap();
    assert_eq!(restored.level(), 3);
    ```

### Equality and hashing

Codecs are value types: two compare equal iff their `serialize_bytes` match, and
equal codecs hash equal, so they work as dictionary / set keys.

=== "Python"

    ```python
    from yggdryl import compression

    assert compression.Gzip(6) == compression.Gzip()      # default level is 6
    assert hash(compression.Gzip(6)) == hash(compression.Gzip())
    assert len({compression.Gzip(1), compression.Gzip(1)}) == 1
    ```

=== "Node"

    ```js
    const { compression } = require('yggdryl')

    console.assert(new compression.Gzip(6).equals(new compression.Gzip()))
    console.assert(new compression.Gzip(6).hashCode() === new compression.Gzip().hashCode())
    ```

=== "Rust"

    ```rust
    use std::collections::HashSet;
    use yggdryl_core::Gzip;

    assert_eq!(Gzip::new(6).unwrap(), Gzip::default()); // default level is 6
    let set: HashSet<Gzip> = [Gzip::new(1).unwrap(), Gzip::new(1).unwrap()].into_iter().collect();
    assert_eq!(set.len(), 1);
    ```

## Zstd

The Zstandard (RFC 8878) codec is built on `zstd` (bundling libzstd, so it needs a C
compiler at build time). Levels span `Zstd.level_range()` (default `3`); higher
levels compress more. On repetitive data it reaches far higher ratios than gzip.

=== "Python"

    ```python
    from yggdryl import compression

    zstd = compression.Zstd()   # level 3 by default
    assert zstd.name == "zstd"

    data = b"the quick brown fox " * 200
    packed = zstd.encode_byte_array(data)
    assert zstd.decode_byte_array(packed) == data
    ```

=== "Node"

    ```js
    const { compression } = require('yggdryl')

    const zstd = new compression.Zstd()   // level 3 by default
    const data = Buffer.from('the quick brown fox '.repeat(200))
    const packed = zstd.encodeByteArray(data)
    console.assert(zstd.decodeByteArray(packed).equals(data))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{Decoder, Encoder, Zstd};

    let zstd = Zstd::default(); // level 3
    let data = b"the quick brown fox ".repeat(200);
    let packed = zstd.encode_byte_array(&data).unwrap();
    assert_eq!(zstd.decode_byte_array(&packed).unwrap(), data);
    ```

## Streaming

Besides the one-shot `encode_byte_array` / `decode_byte_array`, a codec streams
between two cursors with `compress_stream` / `decompress_stream`, running the backend
in bounded memory. See [Byte IO → Streaming
compression](io.md#streaming-compression) for the example across all three
languages.

In Rust, the [`CompressIO`] extension trait adds the one-shot inverse on the IO side
— `cursor.compress(&codec)` / `cursor.decompress(&codec)` read the cursor's remaining
bytes and return a new cursor over the result (Rust-only: it takes a generic codec).

```rust
use yggdryl_core::{ByteBuffer, CompressIO, Zstd, IOBase, Whence};

let zstd = Zstd::default();
let mut data = ByteBuffer::from_bytes(b"compress me").byte_cursor();
let mut packed = data.compress(&zstd).unwrap();
packed.seek(0, Whence::Start).unwrap();
assert_eq!(packed.decompress(&zstd).unwrap().as_bytes(), b"compress me");
```

## Benchmarks

Each surface ships a throughput benchmark that reports encode/decode MB/s. The
Python and Node scripts compare `yggdryl` against the platform's native gzip
(stdlib `gzip` / `zlib`) so the pure-Rust `flate2` backend can be weighed against C
`zlib`. **Build the extensions in release first** — a debug build is roughly 20×
slower and the numbers are meaningless.

=== "Python"

    ```bash
    (cd bindings/python && uv run maturin develop --release)
    uv run python bindings/python/benchmarks/bench_compression.py
    ```

=== "Node"

    ```bash
    (cd bindings/node && npm run build)   # napi build --release
    node bindings/node/benchmark/compression.bench.js
    ```

=== "Rust"

    ```bash
    cargo bench -p yggdryl-compression   # the bench profile is optimised
    ```

[`Encoder`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/trait.Encoder.html
[`Decoder`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/trait.Decoder.html
[`TypedEncoder<T>`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/trait.TypedEncoder.html
[`TypedDecoder<T>`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/trait.TypedDecoder.html
[`Compression`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/trait.Compression.html
[`CompressionEncoder`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/trait.CompressionEncoder.html
[`CompressionDecoder`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/trait.CompressionDecoder.html
[`TypedCompressionEncoder<T>`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/trait.TypedCompressionEncoder.html
[`TypedCompressionDecoder<T>`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/trait.TypedCompressionDecoder.html
[`Gzip`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/struct.Gzip.html
[`Zstd`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/struct.Zstd.html
[`CompressIO`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/trait.CompressIO.html
