# Byte IO

`yggdryl-core` splits byte IO `std::io::Cursor`-style into **storage** and
**cursor**, both backed by an Apache Arrow `Buffer` (the core is Arrow-backed, so
`from_arrow_byte_buffer` / `to_arrow_byte_buffer` and the bit-buffer equivalents are
zero-copy):

- [`ByteBuffer`] holds bytes — pure storage, no position.
- [`ByteCursor`] (from `ByteBuffer.byte_cursor()`) holds a share of the buffer plus
  a position, and does the reading and writing. Reads/writes **advance** the cursor
  and resolve their start via [`Whence`] (`Start` / `Current` / `End`); a write
  copies the shared bytes out first (copy-on-write), so the buffer stays intact.

!!! note "A cursor's size is what **remains**"
    `byte_size` / `bit_size` / `size` on a **cursor** report the bytes / bits /
    elements remaining from the current position to the end (what a read still yields);
    they drop as the cursor advances and are `0` at the end. Storage `ByteBuffer`
    keeps returning the **total**, and `byte_capacity` / `capacity` stay total-capacity.
    The `End` seek origin always resolves against the total extent.

!!! note "The traits are Rust-only; the concrete resources are replicated"
    `IOBase` / `TypedIOBase` / `IOCursor` / `TypedIOCursor` are generic Rust
    contracts that can't cross the FFI boundary. The **concrete** resources are
    replicated in the bindings: the byte `ByteBuffer` / `ByteCursor` / `ByteSlice`, the
    element-typed `TypedCursor<T>` / `TypedSlice<T>` as one class per primitive
    (`I8Cursor` … `F64Cursor`, `I8Slice` … `F64Slice`), the wide-integer cursors and
    slices `I96Cursor` / `I256Slice` / … (values as Python `int` / Node `BigInt`), and
    the `Whence` seek origin. Node omits the `u64` classes (no native `u64` scalar) and
    marshals the `f32` classes over an `f64` boundary.

## Read and write through a cursor

=== "Python"

    ```python
    from yggdryl.io import ByteBuffer, Whence

    buffer = ByteBuffer(b"hello world")
    cursor = buffer.byte_cursor()
    assert cursor.pread_byte_array(5) == b"hello"   # reads at 0, advances to 5
    assert cursor.pread_byte_array(6, Whence.Current) == b" world"
    assert cursor.tell() == 11
    assert buffer.byte_size() == 11                 # buffer untouched
    ```

=== "Node"

    ```js
    const { ByteBuffer, Whence } = require('yggdryl').io

    const buffer = new ByteBuffer(Buffer.from('hello world'))
    const cursor = buffer.byteCursor()
    console.assert(cursor.preadByteArray(5).equals(Buffer.from('hello')))
    console.assert(cursor.preadByteArray(6, Whence.Current).equals(Buffer.from(' world')))
    console.assert(Number(cursor.tell()) === 11)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{ByteBuffer, IOBase, Whence};

    let buffer = ByteBuffer::from_bytes(b"hello world");
    let mut cursor = buffer.byte_cursor();
    assert_eq!(cursor.pread_byte_array(5, Whence::Start).unwrap(), b"hello");
    assert_eq!(cursor.pread_byte_array(6, Whence::Current).unwrap(), b" world");
    assert_eq!(cursor.byte_tell().unwrap(), 11);
    ```

## Copy-on-write keeps the buffer intact

A cursor's write never mutates the source `ByteBuffer` — it copies first:

=== "Python"

    ```python
    from yggdryl.io import ByteBuffer

    buffer = ByteBuffer(b"abcdef")
    cursor = buffer.byte_cursor()
    cursor.pwrite_byte_array(b"XYZ")
    assert buffer.as_bytes() == b"abcdef"   # source intact
    assert cursor.as_bytes() == b"XYZdef"
    ```

=== "Node"

    ```js
    const { ByteBuffer } = require('yggdryl').io

    const buffer = new ByteBuffer(Buffer.from('abcdef'))
    const cursor = buffer.byteCursor()
    cursor.pwriteByteArray(Buffer.from('XYZ'))
    console.assert(buffer.asBytes().equals(Buffer.from('abcdef')))
    console.assert(cursor.asBytes().equals(Buffer.from('XYZdef')))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{ByteBuffer, IOBase, Whence};

    let buffer = ByteBuffer::from_bytes(b"abcdef");
    let mut cursor = buffer.byte_cursor();
    cursor.pwrite_byte_array(b"XYZ", Whence::Start).unwrap();
    assert_eq!(buffer.as_bytes(), b"abcdef");
    assert_eq!(cursor.as_bytes(), b"XYZdef");
    ```

## Fast reads: fill a reused buffer

`pread_byte_array` allocates a fresh buffer each call. For hot read loops, fill a
**reusable** buffer with `pread_into` — no per-call allocation, and nothing extra
crosses the FFI boundary (~3.5× faster than the allocating read in Python):

=== "Python"

    ```python
    from yggdryl.io import ByteBuffer, Whence

    cursor = ByteBuffer(b"abcdefgh").byte_cursor()
    scratch = bytearray(4)                       # reuse this
    n = cursor.pread_into(scratch, Whence.Current)
    assert n == 4 and bytes(scratch) == b"abcd"
    ```

=== "Node"

    ```js
    const { ByteBuffer, Whence } = require('yggdryl').io

    const cursor = new ByteBuffer(Buffer.from('abcdefgh')).byteCursor()
    const scratch = Buffer.alloc(4)              // filled in place
    const n = cursor.preadInto(scratch, Whence.Current)
    console.assert(Number(n) === 4 && scratch.toString() === 'abcd')
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{ByteBuffer, IOBase, Whence};

    let mut cursor = ByteBuffer::from_bytes(b"abcdefgh").byte_cursor();
    let mut scratch = [0u8; 4];
    let n = cursor.pread_into(&mut scratch, Whence::Current).unwrap();
    assert_eq!(n, 4);
    assert_eq!(&scratch, b"abcd");
    ```

## Typed & capacity

Cursors read/write every fixed-width primitive (little-endian, `pread_i64` /
`pwrite_i64_array` / …). Buffers and cursors report `byte_size` / `bit_size`,
`byte_capacity` / `bit_capacity`, and are built with `with_byte_capacity` /
`with_bit_capacity`.

=== "Python"

    ```python
    from yggdryl.io import ByteBuffer, Whence

    cursor = ByteBuffer.with_byte_capacity(64).byte_cursor()
    cursor.pwrite_i64_array([1, 2, 3, -4])
    cursor.seek(0)
    assert cursor.pread_i64_array(4, Whence.Current) == [1, 2, 3, -4]
    assert cursor.size() == 32  # u8 count == byte_size
    ```

=== "Node"

    ```js
    const { ByteBuffer, Whence } = require('yggdryl').io

    const cursor = ByteBuffer.withByteCapacity(64).byteCursor()
    cursor.pwriteI64Array([1n, 2n, 3n, -4n]) // i64 marshals as BigInt
    cursor.seek(0)
    console.assert(Number(cursor.byteSize()) === 32)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{ByteBuffer, IOBase, TypedIOBase, Whence};

    let mut cursor = ByteBuffer::with_byte_capacity(64).byte_cursor();
    cursor.pwrite_i64_array(&[1, 2, 3, -4], Whence::Start).unwrap();
    cursor.byte_seek(0, Whence::Start).unwrap();
    assert_eq!(cursor.pread_i64_array(4, Whence::Current).unwrap(), [1, 2, 3, -4]);
    assert_eq!(TypedIOBase::<u8>::size(&cursor).unwrap(), 32);
    ```

!!! note "Node type limits"
    On the cursor's typed surface Node omits `u64` (napi has no native `u64`
    scalar — use `preadI64` or raw bytes), and `f32` marshals over an `f64`
    boundary. The `large*Size` accessors are JS `BigInt`.

## Element-typed cursors

`ByteCursor`'s `tell` / `seek` count in **bytes** (its native unit). For a cursor whose
native unit is a wider primitive, open a **typed cursor** with a buffer's
`cursor()` (or `<Type>Cursor.with_capacity(n)`): `tell` / `seek` then count in
`T` values, while `byte_tell` / `byte_seek` (and `bit_tell` / `bit_seek`) still reach
the byte and bit positions. A write past the end fills the gap with the type's
`default_value` (zero for every native primitive).

=== "Python"

    ```python
    from yggdryl.buffer import I32Buffer
    from yggdryl.io import Whence

    cursor = I32Buffer([10, 20, 30, 40]).cursor()
    assert cursor.pread_one(Whence.Start) == 10
    assert cursor.tell() == 1          # one i32 in
    assert cursor.byte_tell() == 4     # four bytes in
    cursor.seek(-1, Whence.End)        # last i32
    assert cursor.pread_one(Whence.Current) == 40
    ```

=== "Node"

    ```js
    const { I32Buffer } = require('yggdryl').buffer
    const { Whence } = require('yggdryl').io

    const cursor = new I32Buffer([10, 20, 30, 40]).cursor()
    console.assert(cursor.preadOne(Whence.Start) === 10)
    console.assert(Number(cursor.tell()) === 1)      // one i32 in
    console.assert(Number(cursor.byteTell()) === 4)  // four bytes in
    cursor.seek(-1, Whence.End)                       // last i32
    console.assert(cursor.preadOne(Whence.Current) === 40)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{I32Buffer, IOBase, TypedIOBase, Whence};

    let mut cursor = I32Buffer::from_slice(&[10, 20, 30, 40]).cursor();
    assert_eq!(cursor.pread_one(Whence::Start).unwrap(), 10);
    assert_eq!(cursor.tell().unwrap(), 1);        // one i32 in
    assert_eq!(cursor.byte_tell().unwrap(), 4);   // four bytes in
    cursor.seek(-1, Whence::End).unwrap();        // last i32
    assert_eq!(cursor.pread_one(Whence::Current).unwrap(), 40);
    ```

!!! note "Bit-addressed seeks are byte-aligned"
    `bit_tell` returns `byte_tell * 8`; `bit_seek` moves in bit units but the offset
    must be a multiple of 8 (every origin is byte-aligned), else a guiding error. A
    negative `offset` seeks backward on every `*_seek`.

## Bounded windows (slices)

A **slice** is the fixed-length, non-growing sibling of a cursor: a window
`[offset, offset + len)` over a buffer's bytes. Its positions `0..len` are relative to
the window start, reads and writes are **clamped** to the window (a slice never grows),
and a write is copy-on-write, leaving the source intact. Open a byte window with
`byte_slice(offset, len)` or an element-typed window with a typed buffer's
`slice(offset, len)` — the concrete classes are `ByteSlice` and `I8Slice` …
`I256Slice`, mirroring the cursors.

=== "Python"

    ```python
    from yggdryl.buffer import I32Buffer
    from yggdryl.io import ByteBuffer, Whence

    window = ByteBuffer(b"hello world").byte_slice(6, 5)  # the "world" window
    assert window.slice_offset() == 6 and window.slice_len() == 5
    assert window.pread_byte_array(100) == b"world"       # clamped

    typed = I32Buffer([10, 20, 30, 40, 50]).slice(1, 3)   # [20, 30, 40]
    assert typed.size() == 3
    assert typed.pread_array(100, Whence.Start) == [20, 30, 40]
    ```

=== "Node"

    ```js
    const { ByteBuffer, Whence } = require('yggdryl').io
    const { I32Buffer } = require('yggdryl').buffer

    const window = new ByteBuffer(Buffer.from('hello world')).byteSlice(6, 5)
    console.assert(Number(window.sliceLen()) === 5)
    console.assert(window.preadByteArray(100).equals(Buffer.from('world'))) // clamped

    const typed = new I32Buffer([10, 20, 30, 40, 50]).slice(1, 3) // [20, 30, 40]
    console.assert(Number(typed.size()) === 3)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{ByteBuffer, I32Buffer, IOBase, IOSlice, Whence};

    let mut window = ByteBuffer::from_bytes(b"hello world").byte_slice(6, 5);
    assert_eq!(window.slice_offset(), 6);
    assert_eq!(window.pread_byte_array(100, Whence::Start).unwrap(), b"world"); // clamped

    let mut typed = I32Buffer::from_slice(&[10, 20, 30, 40, 50]).slice(1, 3); // [20, 30, 40]
    assert_eq!(typed.size().unwrap(), 3);
    ```

!!! note "`IOSlice` / `TypedIOSlice` are Rust-only traits"
    As with the cursor markers, the `IOSlice` / `TypedIOSlice<T>` traits are generic
    Rust contracts; only the concrete `ByteSlice` / `<Type>Slice` classes cross the FFI
    boundary.

## Wide integers (`i96` / `i128` / `i256`)

Beyond the native primitives, three **wide signed integers** — 96-, 128-, and
256-bit — round-trip through cursors (`I96Cursor` / `I128Cursor` / `I256Cursor`). They
have no fixed-width scalar in Python or JS, so values marshal as an arbitrary-precision
`int` / `BigInt`; an out-of-range value raises. See the [wide integers](wide_ints.md)
page for the Rust types and their arithmetic.

=== "Python"

    ```python
    from yggdryl.io import I256Cursor, Whence

    cursor = I256Cursor.with_capacity(2)
    big = 2**200 + 12345           # far beyond i128
    cursor.pwrite_array([big, -big], Whence.Start)
    cursor.seek(0)
    assert cursor.pread_array(2, Whence.Start) == [big, -big]
    ```

=== "Node"

    ```js
    const { I256Cursor, Whence } = require('yggdryl').io

    const cursor = I256Cursor.withCapacity(2)
    const big = 2n ** 200n + 12345n            // far beyond i128
    cursor.pwriteArray([big, -big], Whence.Start)
    cursor.seek(0)
    console.assert(cursor.preadArray(2, Whence.Start).every((v, i) => v === [big, -big][i]))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{i256, IOBase, TypedCursor, TypedIOBase, Whence};

    let big = i256::from_i128(i128::MAX) + i256::from_i128(1); // beyond i128
    let mut cursor = <TypedCursor<i256> as TypedIOBase<i256>>::with_capacity(2);
    cursor.pwrite_array(&[big, -big], Whence::Start).unwrap();
    cursor.seek(0, Whence::Start).unwrap();
    assert_eq!(cursor.pread_array(2, Whence::Current).unwrap(), vec![big, -big]);
    ```

## Streaming compression

A [`Gzip`](compression.md) codec streams between two cursors — rewind the
compressed cursor before decompressing.

=== "Python"

    ```python
    from yggdryl import compression
    from yggdryl.io import ByteBuffer

    gzip = compression.Gzip(6)
    source = ByteBuffer(b"stream me " * 500).byte_cursor()
    packed = ByteBuffer().byte_cursor()
    gzip.compress_stream(source, packed)

    packed.seek(0)
    restored = ByteBuffer().byte_cursor()
    gzip.decompress_stream(packed, restored)
    assert restored.as_bytes() == b"stream me " * 500
    ```

=== "Node"

    ```js
    const { compression, io } = require('yggdryl')
    const { ByteBuffer } = io

    const gzip = new compression.Gzip(6)
    const source = new ByteBuffer(Buffer.from('stream me '.repeat(500))).byteCursor()
    const packed = new ByteBuffer().byteCursor()
    gzip.compressStream(source, packed)

    packed.seek(0)
    const restored = new ByteBuffer().byteCursor()
    gzip.decompressStream(packed, restored)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{ByteBuffer, CompressionDecoder, CompressionEncoder, Gzip, IOBase, Whence};

    let gzip = Gzip::new(6).unwrap();
    let mut source = ByteBuffer::from_bytes(&b"stream me ".repeat(500)).byte_cursor();
    let mut packed = ByteBuffer::new().byte_cursor();
    gzip.compress_stream(&mut source, &mut packed).unwrap();

    packed.byte_seek(0, Whence::Start).unwrap();
    let mut restored = ByteBuffer::new().byte_cursor();
    gzip.decompress_stream(&mut packed, &mut restored).unwrap();
    ```

## Zero-copy Arrow (Rust)

`ByteBuffer` **is** backed by an Apache Arrow `Buffer` (the core is Arrow-backed, not
gated behind a feature), so it wraps one **zero-copy** — sharing the allocation rather
than copying it — and emits one back:

```rust
use yggdryl_core::ByteBuffer;
use yggdryl_core::arrow_buffer::Buffer; // re-exported at the matching version

let arrow = Buffer::from_vec(b"payload".to_vec());
let buffer = ByteBuffer::from_arrow_byte_buffer(arrow); // shares the allocation
assert_eq!(buffer.as_bytes(), b"payload");

let out = buffer.to_arrow_byte_buffer();               // zero-copy back
assert_eq!(out.as_slice(), b"payload");
```

A cursor that then writes copies-on-write, leaving the Arrow allocation intact.
`from_arrow_bit_buffer` wraps an Arrow bitmap (packed bits) the same way. This is
Rust-only — an `arrow_buffer::Buffer` does not cross the FFI boundary.

## Benchmarks

Cursor IO, the element-typed cursor, the bounded slices, and streaming have throughput
benchmarks in all three surfaces (`cargo bench -p yggdryl-core`; `bindings/*/…/io.*`).
See the
[byte report](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/io/byte_buffer.md),
the [`TypedCursor` report](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/io/typed_cursor.md),
and the [`ByteSlice` report](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/io/byte_slice.md).

[`ByteBuffer`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/struct.ByteBuffer.html
[`ByteCursor`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/struct.ByteCursor.html
[`Whence`]: https://docs.rs/yggdryl-core/latest/yggdryl_core/enum.Whence.html
