# Compression

Four codecs — **Gzip**, **Zlib**, **Zstd**, **Lzma** (xz) — over their native cores, plus the
[`IOBase`](io/memory.md) integration that runs them with a zero-copy read and infers the codec
from a source's [media type](mediatype.md). The core is dependency-free; the codecs are behind
the **`compression`** cargo feature (the extensions enable it by default).

Gzip/zlib run on flate2's **`zlib-rs`** backend — a pure-Rust port of the SIMD-tuned zlib-ng that
needs **no C toolchain** yet **out-compresses** the C `zlib` that Python's `gzip` and Node's
`node:zlib` link. The binding takes a byte input as a **borrowed buffer** (Python `bytes`/
`bytearray`, Node `Buffer`) — never copied element by element — so that codec speed reaches the
caller. Measured throughput vs. the language-native codecs is in
[`benchmarks/yggdryl-core/compression.md`](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/compression.md).

## Compress and decompress bytes

Each codec takes an optional level and round-trips a byte buffer losslessly.

=== "Python"

    ```python
    from yggdryl.compression import Zstd

    codec = Zstd(level=3)                     # or Gzip(), Zlib(), Lzma()
    packed = codec.compress(b"the quick brown fox " * 100)
    assert len(packed) < 2000
    assert codec.decompress(packed) == b"the quick brown fox " * 100
    assert codec.essence == "application/zstd"
    ```

=== "Node"

    ```javascript
    const { Zstd } = require('yggdryl').compression

    const codec = new Zstd(3)                 // or new Gzip(), Zlib(), Lzma()
    const data = Buffer.from('the quick brown fox '.repeat(100))
    const packed = codec.compress(data)
    console.assert(packed.length < 2000)
    console.assert(codec.decompress(packed).equals(data))
    console.assert(codec.essence === 'application/zstd')
    ```

=== "Rust"

    ```rust
    use yggdryl_core::compression::{Compression, Zstd};

    let codec = Zstd::with_level(3); // or Gzip::new(), Zlib::new(), Lzma::new()
    let data = b"the quick brown fox ".repeat(100);
    let packed = codec.compress(&data).unwrap();
    assert!(packed.len() < 2000);
    assert_eq!(codec.decompress(&packed).unwrap(), data);
    assert_eq!(codec.essence(), "application/zstd");
    ```

## Resolve a codec from a media type

`is_compression` tells you a type is compressible; `codec_for` hands you the codec.

=== "Python"

    ```python
    from yggdryl.mimetype import MimeType
    from yggdryl.compression import codec_for

    gz = MimeType.from_extension("gz")
    assert gz.is_compression()
    assert codec_for(gz).name == "gzip"
    assert codec_for("application/json") is None   # not a compression
    ```

=== "Node"

    ```javascript
    const { MimeType } = require('yggdryl').mimetype
    const { codecFor } = require('yggdryl').compression

    const gz = MimeType.fromExtension('gz')
    console.assert(gz.isCompression())
    console.assert(codecFor(gz).name === 'gzip')
    console.assert(codecFor('application/json') === null) // not a compression
    ```

=== "Rust"

    ```rust
    use yggdryl_core::mimetype::MimeType;
    use yggdryl_core::compression::codec_for_mime;

    let gz = MimeType::from_extension("gz").unwrap();
    assert!(gz.is_compression());
    assert_eq!(codec_for_mime(&gz).unwrap().name(), "gzip");
    ```

## Shared codec instances

Resolving a **default-level** codec never rebuilds it. In Python, `codec_for` and every
source's `compression()` hand back a **cached singleton** per format — one shared object,
returned by reference — so neither the resolver nor a source's accessor constructs a codec per
call. The Rust core exposes the same as `codec_ref_for` / `codec_ref_for_mime`, which return a
`&'static` default-level codec (the zero-allocation counterpart of `codec_for` /
`codec_for_mime`). Build a codec with an explicit level (`Zstd(level=9)`, `new Zstd(9)`,
`Zstd::with_level(9)`) only when you need a non-default one; the balanced defaults cost nothing
to reuse.

## Decompress a source (codec inferred from its media type)

A source addressed as a compression — a `.gz` file, or a heap whose `Content-Type` says so —
decompresses itself. The read is **zero-copy**: a mapped file or a heap hands its bytes straight
to the codec.

=== "Python"

    ```python
    from yggdryl.memory import Heap
    from yggdryl.compression import Gzip

    packed = Gzip().compress(b"payload " * 500)
    src = Heap(packed)
    hdrs = src.headers                          # a copy; write it back to store
    hdrs.set_content_type("application/gzip")
    src.set_headers(hdrs)
    assert src.decompress() == b"payload " * 500          # codec inferred
    # Or an explicit codec:
    assert src.decompress_with(Gzip()) == b"payload " * 500
    ```

=== "Node"

    ```javascript
    const { Heap } = require('yggdryl').memory
    const { Gzip } = require('yggdryl').compression
    const { Headers } = require('yggdryl').headers

    const packed = new Gzip().compress(Buffer.from('payload '.repeat(500)))
    const src = new Heap(packed)
    src.setHeaders(new Headers().with('Content-Type', 'application/gzip'))
    console.assert(src.decompress().equals(Buffer.from('payload '.repeat(500)))) // inferred
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::memory::{Heap, IOBase};
    use yggdryl_core::compression::{Compression, Gzip};

    let packed = Gzip::new().compress(&b"payload ".repeat(500)).unwrap();
    let mut src = Heap::from_slice(&packed);
    src.headers_mut().set_content_type("application/gzip");
    assert_eq!(src.decompress().unwrap(), b"payload ".repeat(500)); // codec inferred
    ```

## In-place (self-overwriting) codecs

A source can rewrite **its own** bytes: `compress_in_place` swaps its content for the
compressed form and `decompress_in_place` swaps it back — each keeping `Content-Type`,
`Content-Length`, and `mtime` in sync in the **same pass**. They complement the byte-returning
`compress_with` / `decompress_with` above, which leave the source untouched. With no codec,
`compress_in_place` uses the one inferred from the source's media type (address the heap as
`.gz` / set its `Content-Type`); pass an explicit codec to override.

=== "Python"

    ```python
    from yggdryl.memory import Heap

    src = Heap(b"payload " * 500)
    hdrs = src.headers                          # a copy; write it back to store
    hdrs.set_content_type("application/gzip")   # address the heap as gzip
    src.set_headers(hdrs)

    src.compress_in_place()                     # codec=None → inferred from the media type
    assert len(src) < 500                       # the heap now holds its own packed bytes
    src.decompress_in_place()                   # peel it back — codec inferred again
    assert bytes(src) == b"payload " * 500      # round-trips to the original
    ```

=== "Node"

    ```javascript
    const { Heap } = require('yggdryl').memory
    const { Headers } = require('yggdryl').headers

    const src = new Heap(Buffer.from('payload '.repeat(500)))
    src.setHeaders(new Headers().with('Content-Type', 'application/gzip')) // address as gzip

    src.compressInPlace()                                      // codec inferred from the type
    console.assert(src.toBytes().length < 500)                 // the heap now holds packed bytes
    src.decompressInPlace()                                    // peel it back — codec inferred
    console.assert(src.toBytes().equals(Buffer.from('payload '.repeat(500))))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::memory::{Heap, IOBase};

    let mut src = Heap::from_slice(&b"payload ".repeat(500));
    src.headers_mut().set_content_type("application/gzip"); // address the heap as gzip

    src.compress_in_place(None).unwrap();                   // codec inferred from the media type
    assert!(src.byte_size() < 500);                         // the heap now holds packed bytes
    src.decompress_in_place().unwrap();                     // peel it back — codec inferred again
    assert_eq!(src.pread_vec(0, src.byte_size() as usize), b"payload ".repeat(500));
    ```

## Sniff a type by magic, then peel it

`infer_mime_type` reads the head's magic bytes with a **positioned** read (it never moves the
cursor). `infer_media_type` goes further: it **peels compression layers**, so a gzipped PDF
reads as `[application/gzip, application/pdf]` — decoding only the head it needs.

=== "Python"

    ```python
    from yggdryl.memory import Heap
    from yggdryl.compression import Gzip

    gzipped_pdf = Gzip().compress(b"%PDF-1.7\n" + b"body " * 200)
    src = Heap(gzipped_pdf)
    assert src.infer_mime_type().essence == "application/gzip"      # magic, cursor unmoved
    assert src.infer_media_type().essences() == ["application/gzip", "application/pdf"]
    ```

=== "Node"

    ```javascript
    const { Heap } = require('yggdryl').memory
    const { Gzip } = require('yggdryl').compression

    const gzippedPdf = new Gzip().compress(Buffer.concat(
      [Buffer.from('%PDF-1.7\n'), Buffer.from('body '.repeat(200))]))
    const src = new Heap(gzippedPdf)
    console.assert(src.inferMimeType().essence === 'application/gzip')
    console.assert(JSON.stringify(src.inferMediaType().essences()) ===
      '["application/gzip","application/pdf"]')
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::memory::{Heap, IOBase};
    use yggdryl_core::compression::{Compression, Gzip};

    let mut body = b"%PDF-1.7\n".to_vec();
    body.extend_from_slice(&b"body ".repeat(200));
    let src = Heap::from_slice(&Gzip::new().compress(&body).unwrap());
    assert_eq!(src.infer_mime_type().essence(), "application/gzip"); // magic, cursor unmoved
    assert_eq!(src.infer_media_type().essences(),
               vec!["application/gzip", "application/pdf"]);
    ```

The codec core is native; the pipeline around it is leaner — see the
[compression benchmark](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/compression.md)
for the zero-copy-vs-naive numbers.
