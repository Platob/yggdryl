# Positioned I/O

The `core` I/O layer reads and writes a resource one or many **bytes** (`u8`) or
**bits** (`bool`) at a time. Three concrete in-memory resources are provided:

- **`ByteBuffer`** — byte-granular; its bit size is always eight times its byte size.
- **`BitBuffer`** — bit-granular; it tracks an *exact* bit length, so its bit size
  need not be a multiple of eight (its byte size rounds up).
- **`StringBuffer`** — a `ByteBuffer` over UTF-8 bytes with a typed `char` view
  (`IOBase<char>`): writing a `char` appends its UTF-8 encoding, and its typed
  `size` counts Unicode scalar values. It backs the `utf8` string scalar the way
  `ByteBuffer` backs `binary`. (Rust-only for now; the string scalar crosses as
  native text.)

`ByteBuffer` and `BitBuffer` are exposed in Python, Node, and Rust. In Rust they
implement the
[`RawIOBase`](#rawiobase) trait (described at the end of this page); a buffer keeps
no cursor of its own, so for a position that advances on each access you wrap it in a
[`RawIOCursor`](#cursors), and for a bounded byte window in a
[`RawIOSlice`](#slices) (or, for typed values, an `IOCursor` / `IOSlice`).

## Whence

Every access names a `position` and the `Whence` it is measured from:

| `Whence` | Measured from |
| --- | --- |
| `Start` | the beginning (an absolute offset) |
| `Current` | the cursor of a [`RawIOCursor`](#cursors) — a bare buffer has none, so it reads as `Start` |
| `End` | the end — `End, 0` is the append point |

Positions are counted in **bytes** for the `*_byte_*` methods and in **bits**
(MSB-first, so bit `0` of a byte is its most significant bit) for the `*_bit_*`
methods.

## Bytes

Create a buffer, write some bytes, and read them back:

=== "Python"

    ```python
    from yggdryl import core

    buf = core.ByteBuffer()
    buf.pwrite_byte_array(0, core.Whence.Start, b"\x01\x02\x03")
    buf.pwrite_byte_array(0, core.Whence.End, b"\x04\x05")  # append

    assert buf.byte_size() == 5
    assert buf.pread_byte_one(1, core.Whence.Start) == 2
    buf.pwrite_i64(3, core.Whence.Start, -2)  # every numeric primitive, little-endian
    assert buf.pread_i64(3, core.Whence.Start) == -2
    assert buf.pread_byte_array(0, core.Whence.Start, 5) == b"\x01\x02\x03\x04\x05"
    ```

=== "Node"

    ```js
    const { ByteBuffer, Whence } = require('yggdryl').core

    const buf = new ByteBuffer()
    buf.pwriteByteArray(0, Whence.Start, Buffer.from([1, 2, 3]))
    buf.pwriteByteArray(0, Whence.End, Buffer.from([4, 5])) // append

    console.assert(buf.byteSize() === 5)
    console.assert(buf.preadByteOne(1, Whence.Start) === 2)
    buf.pwriteI64(3, Whence.Start, -2n) // every numeric primitive, little-endian
    console.assert(buf.preadI64(3, Whence.Start) === -2n)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{ByteBuffer, RawIOBase, Whence};

    fn main() {
        let mut buf = ByteBuffer::new();
        buf.pwrite_byte_array(0, Whence::Start, &[1, 2, 3]).unwrap();
        buf.pwrite_byte_array(0, Whence::End, &[4, 5]).unwrap(); // append

        assert_eq!(buf.byte_size(), 5);
        assert_eq!(buf.pread_byte_one(1, Whence::Start).unwrap(), 2);
        buf.pwrite_i64(3, Whence::Start, -2).unwrap(); // every numeric primitive
        assert_eq!(buf.pread_i64(3, Whence::Start).unwrap(), -2);
        assert_eq!(buf.pread_byte_array(0, Whence::Start, 5).unwrap(), vec![1, 2, 3, 4, 5]);
    }
    ```

A read past the end fails (Python raises `ValueError`; Node throws; Rust returns
`IOError::OutOfBounds`). An empty write is a no-op and never grows the buffer.

## Bits

Bits are addressed MSB-first. `ByteBuffer` exposes the same bytes as bits:

=== "Python"

    ```python
    buf = core.ByteBuffer.from_bytes(bytes([0b1010_0000]))
    assert buf.pread_bit_one(0, core.Whence.Start) is True   # the MSB
    assert buf.pread_bit_one(1, core.Whence.Start) is False
    buf.pwrite_bit_one(1, core.Whence.Start, True)
    assert buf.pread_byte_one(0, core.Whence.Start) == 0b1110_0000
    ```

=== "Node"

    ```js
    const buf = ByteBuffer.fromBytes(Buffer.from([0b1010_0000]))
    console.assert(buf.preadBitOne(0, Whence.Start) === true) // the MSB
    console.assert(buf.preadBitOne(1, Whence.Start) === false)
    buf.pwriteBitOne(1, Whence.Start, true)
    console.assert(buf.preadByteOne(0, Whence.Start) === 0b1110_0000)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{ByteBuffer, RawIOBase, Whence};

    fn main() {
        let mut buf = ByteBuffer::from_bytes(vec![0b1010_0000]);
        assert!(buf.pread_bit_one(0, Whence::Start).unwrap()); // the MSB
        assert!(!buf.pread_bit_one(1, Whence::Start).unwrap());
        buf.pwrite_bit_one(1, Whence::Start, true).unwrap();
        assert_eq!(buf.pread_byte_one(0, Whence::Start).unwrap(), 0b1110_0000);
    }
    ```

## Cursors

A `ByteBuffer` / `BitBuffer` is pure random access: it keeps no cursor, so
`Whence.Current` reads as `Whence.Start`. Wrap one with `.cursor()` for a position
that advances on every read and write, turning random access into a sequential stream.
`Whence.Current` is measured from the cursor, `seek` moves it without touching the
data, and `tell` reports it in bytes.

In Rust the cursor owns the buffer (recover it with `get_ref` / `into_inner`); in
Python and Node — which have no move semantics — `cursor()` operates on a **copy** of
the buffer's bytes, and `to_bytes()` reads the result back:

=== "Python"

    ```python
    buf = core.ByteBuffer.from_bytes(bytes([10, 20, 30, 40]))
    cursor = buf.cursor()
    assert cursor.pread_byte_array(0, core.Whence.Current, 2) == bytes([10, 20])
    assert cursor.tell() == 2
    assert cursor.pread_byte_array(0, core.Whence.Current, 2) == bytes([30, 40])

    cursor.seek(0, core.Whence.Start)
    cursor.pwrite_byte_array(0, core.Whence.Current, bytes([1, 2]))
    assert cursor.to_bytes() == bytes([1, 2, 30, 40])
    ```

=== "Node"

    ```js
    const buf = ByteBuffer.fromBytes(Buffer.from([10, 20, 30, 40]))
    const cursor = buf.cursor()
    console.assert(Buffer.compare(cursor.preadByteArray(0, Whence.Current, 2), Buffer.from([10, 20])) === 0)
    console.assert(cursor.tell() === 2)

    cursor.seek(0, Whence.Start)
    cursor.pwriteByteArray(0, Whence.Current, Buffer.from([1, 2]))
    console.assert(Buffer.compare(cursor.toBytes(), Buffer.from([1, 2, 30, 40])) === 0)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{ByteBuffer, RawIOBase, Seekable, Whence};

    fn main() {
        let mut cursor = ByteBuffer::from_bytes(vec![10, 20, 30, 40]).cursor();
        assert_eq!(cursor.pread_byte_array(0, Whence::Current, 2).unwrap(), vec![10, 20]);
        assert_eq!(cursor.tell(), 2);
        assert_eq!(cursor.pread_byte_array(0, Whence::Current, 2).unwrap(), vec![30, 40]);

        cursor.seek(0, Whence::Start).unwrap();
        cursor.pwrite_byte_array(0, Whence::Current, &[1, 2]).unwrap();
        assert_eq!(cursor.get_ref().as_bytes(), &[1, 2, 30, 40]);
    }
    ```

!!! note "Typed cursor (Rust core only)"
    `IOCursor<I>` is the same adapter over a typed [`IOBase<T>`](#iobaset) resource:
    its typed `pwrite_one` / `pwrite_array` stream `T` values out one after another.
    Build it with `IOBase::<T>::cursor(resource)`. It stays Rust-only until a binding
    exposes an `IOBase<T>` resource.

## Slices

A slice bounds a resource to a byte window `[start, end)`: code handed the slice sees a
smaller resource and cannot read or write outside it. Build one with
`.slice(start, end)`. Positions are relative to the window, `byte_size` reports the
backed length within it, and writes may grow the inner up to `end` but never past it.
As with the cursor, Python and Node operate on a copy of the buffer's bytes:

=== "Python"

    ```python
    buf = core.ByteBuffer.from_bytes(bytes([10, 20, 30, 40, 50]))
    sliced = buf.slice(1, 4)  # bytes [1, 4)
    assert sliced.byte_size() == 3
    assert (sliced.start(), sliced.end()) == (1, 4)
    assert sliced.pread_byte_array(0, core.Whence.Start, 3) == bytes([20, 30, 40])

    sliced.pwrite_byte_one(0, core.Whence.Start, 99)
    assert sliced.to_bytes() == bytes([10, 99, 30, 40, 50])
    ```

=== "Node"

    ```js
    const buf = ByteBuffer.fromBytes(Buffer.from([10, 20, 30, 40, 50]))
    const slice = buf.slice(1, 4) // bytes [1, 4)
    console.assert(slice.byteSize() === 3)
    console.assert(slice.start() === 1 && slice.end() === 4)
    console.assert(Buffer.compare(slice.preadByteArray(0, Whence.Start, 3), Buffer.from([20, 30, 40])) === 0)

    slice.pwriteByteOne(0, Whence.Start, 99)
    console.assert(Buffer.compare(slice.toBytes(), Buffer.from([10, 99, 30, 40, 50])) === 0)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{ByteBuffer, RawIOBase, Whence};

    fn main() {
        let mut slice = ByteBuffer::from_bytes(vec![10, 20, 30, 40, 50]).slice(1, 4);
        assert_eq!(slice.byte_size(), 3);
        assert_eq!((slice.start(), slice.end()), (1, 4));
        assert_eq!(slice.pread_byte_array(0, Whence::Start, 3).unwrap(), vec![20, 30, 40]);
        assert!(slice.pread_byte_one(3, Whence::Start).is_err()); // outside the window

        slice.pwrite_byte_one(0, Whence::Start, 99).unwrap();
        assert_eq!(slice.get_ref().as_bytes(), &[10, 99, 30, 40, 50]);
    }
    ```

!!! note "Typed slice (Rust core only)"
    `IOSlice<I>` is the typed counterpart over an [`IOBase<T>`](#iobaset) resource:
    `size` and `resize` count whole `T` items in the window (via the inner's
    `element_width`; a `resize` whose width can't be inferred returns
    `IOError::IndeterminateElementWidth`). Build it with `IOBase::<T>::slice(resource,
    start, end)`. Like the typed cursor it stays Rust-only for now.

## Sizes, capacities and resizing

Both buffers report their size (`byte_size` / `bit_size`) and the allocation they can
hold without reallocating (`byte_capacity` / `bit_capacity`). Capacity requests are
hints that never change the size; `resize_bytes` / `resize_bits` change the size,
truncating or zero-filling. On `ByteBuffer` a bit resize rounds up to whole bytes:

=== "Python"

    ```python
    buf = core.ByteBuffer.from_bytes(b"\x01\x02\x03")
    assert buf.resize_byte_capacity(64) >= 64
    assert buf.byte_size() == 3          # capacity never changes the size

    buf.resize_bytes(5)
    assert buf.to_bytes() == b"\x01\x02\x03\x00\x00"
    buf.resize_bits(9)                    # rounds up to whole bytes
    assert (buf.byte_size(), buf.bit_size()) == (2, 16)
    ```

=== "Node"

    ```js
    const buf = ByteBuffer.fromBytes(Buffer.from([1, 2, 3]))
    console.assert(buf.resizeByteCapacity(64) >= 64)
    console.assert(buf.byteSize() === 3) // capacity never changes the size

    buf.resizeBytes(5)
    console.assert(Buffer.compare(buf.toBytes(), Buffer.from([1, 2, 3, 0, 0])) === 0)
    buf.resizeBits(9) // rounds up to whole bytes
    console.assert(buf.byteSize() === 2 && buf.bitSize() === 16)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{ByteBuffer, RawIOBase};

    fn main() {
        let mut buf = ByteBuffer::from_bytes(vec![1, 2, 3]);
        assert!(buf.resize_byte_capacity(64).unwrap() >= 64);
        assert_eq!(buf.byte_size(), 3); // capacity never changes the size

        buf.resize_bytes(5).unwrap();
        assert_eq!(buf.as_bytes(), &[1, 2, 3, 0, 0]);
        buf.resize_bits(9).unwrap(); // rounds up to whole bytes
        assert_eq!((buf.byte_size(), buf.bit_size()), (2, 16));
    }
    ```

## BitBuffer: exact bit lengths

`BitBuffer` tracks an exact bit length, so `resize_bits` is exact and `bit_size` need
not be a multiple of eight. Truncation zeroes the dropped padding bits:

=== "Python"

    ```python
    buf = core.BitBuffer()
    buf.pwrite_bit_array(0, core.Whence.Start, [True, False, True])
    assert buf.bit_size() == 3
    assert buf.byte_size() == 1          # three bits round up to one byte

    buf.resize_bits(2)                    # exact truncation
    assert buf.bit_size() == 2
    assert buf.pread_bit_array(0, core.Whence.Start, 2) == [True, False]
    ```

=== "Node"

    ```js
    const { BitBuffer, Whence } = require('yggdryl').core

    const buf = new BitBuffer()
    buf.pwriteBitArray(0, Whence.Start, [true, false, true])
    console.assert(buf.bitSize() === 3)
    console.assert(buf.byteSize() === 1) // three bits round up to one byte

    buf.resizeBits(2) // exact truncation
    console.assert(buf.bitSize() === 2)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{BitBuffer, RawIOBase, Whence};

    fn main() {
        let mut buf = BitBuffer::new();
        buf.pwrite_bit_array(0, Whence::Start, &[true, false, true]).unwrap();
        assert_eq!(buf.bit_size(), 3);
        assert_eq!(buf.byte_size(), 1); // three bits round up to one byte

        buf.resize_bits(2).unwrap(); // exact truncation
        assert_eq!(buf.bit_size(), 2);
        assert_eq!(buf.pread_bit_array(0, Whence::Start, 2).unwrap(), vec![true, false]);
    }
    ```

## Streaming between resources

!!! info "Rust core only"
    Both stream pairs stay in the Rust core: `pread_raw_io` / `pwrite_raw_io` (by
    **bytes**) and `pread_typed_io` / `pwrite_typed_io` (by **items**) copy between two
    resources in 64 KiB chunks, so a large transfer never materializes in full. They
    borrow two resources at once; a Python or Node caller composes the same effect from
    `pread_byte_array` + `pwrite_byte_array`.

```rust
use yggdryl_core::{ByteBuffer, RawIOBase, Whence};

fn main() {
    let source = ByteBuffer::from_bytes(vec![1, 2, 3, 4, 5, 6, 7, 8]);
    let mut sink = ByteBuffer::new();
    // Copy four bytes from source@2 into sink@0, chunked.
    source.pread_raw_io(2, Whence::Start, 4, &mut sink, 0, Whence::Start).unwrap();
    assert_eq!(sink.as_bytes(), &[3, 4, 5, 6]);
}
```

The mutable side's start is resolved once against its current size, so `Whence::End`
stays anchored even while the sink grows during the copy.

`IOBase<T>` adds `pread_typed_io` / `pwrite_typed_io`, the same streams counted in
**items** rather than bytes: item offsets are scaled by the resource's `element_width`
and the element-aligned bytes are copied through `pread_raw_io` — no item is
serialized or deserialized, so it stays an optimized bulk copy (both sides must share
the element width and byte layout). A non-zero transfer over a resource whose width is
indeterminate returns `IOError::IndeterminateElementWidth`.

## The traits

The buffers implement a small trait stack (Rust). Any resource that reads or writes
byte/bit sequences implements it, and generic transfer code targets the traits rather
than a concrete buffer.

### `RawIOBase`

The positioned byte/bit surface. Implementors provide the four array primitives
(`pread_byte_array`, `pwrite_byte_array`, `pread_bit_array`, `pwrite_bit_array`) plus
`byte_size` and `resize_bytes`; the `*_one` accessors, `bit_size`, the capacities, the
capacity/`resize_bits` hints, and the `pread_raw_io` / `pwrite_raw_io` streams come free from
defaults. It keeps no cursor, so `Whence::Current` reads as `Whence::Start`.

### `IOBase<T>`

`IOBase<T>: RawIOBase` layers typed values on top. Given a type that already
implements `RawIOBase`, provide `value_to_bytes` (how a `T` becomes bytes), `size`
and `resize` (counted in items); the typed writes `pwrite_one` / `pwrite_array` then
come free, serializing through it into the raw byte methods. A fixed-width
implementor should also override `element_width` (the default infers it as
`byte_size / size`) so a derived [`IOSlice`](#slices) can convert item counts to bytes
even over an empty resource.

```rust
use yggdryl_core::{IOBase, IOError, RawIOBase};

// For some `Store` that implements `RawIOBase` (e.g. holding bytes four to a u32):
impl IOBase<u32> for Store {
    fn value_to_bytes(&self, value: &u32) -> Vec<u8> {
        value.to_le_bytes().to_vec()
    }
    fn size(&self) -> usize {
        self.byte_size() / 4
    }
    fn resize(&mut self, size: usize) -> Result<(), IOError> {
        self.resize_bytes(size * 4)
    }
}
```

### `Seekable`, cursors and slices

`Seekable` is a cursor: `tell(&self) -> usize` and `seek(&mut self, position,
whence) -> Result<usize, IOError>`. Bare buffers do **not** implement it — the
[`RawIOCursor`](#cursors) / `IOCursor<I>` adapters do. Each wraps a resource, adds the
cursor, measures `Whence::Current` from it, and advances it on every read and write.
The [`RawIOSlice`](#slices) / `IOSlice<I>` adapters instead bound a resource to a byte
window `[start, end)`. All four re-implement `RawIOBase` (and the typed pair
`IOBase<T>`), so an adapter is itself a resource that generic transfer code can
target, and they compose.

`RawIOBase` and `IOBase<T>` each expose `cursor()` and `slice(start, end)` factory
methods that consume the resource and return the matching adapter. A type that
implements both traits carries both pairs, so name the typed one explicitly —
`IOBase::<T>::cursor(resource)` — to disambiguate.
