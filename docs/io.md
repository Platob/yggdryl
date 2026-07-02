# Positioned I/O

The `core` I/O layer reads and writes a resource one or many **bytes** (`u8`) or
**bits** (`bool`) at a time. Two concrete in-memory resources are provided:

- **`ByteBuffer`** — byte-granular; its bit size is always eight times its byte size.
- **`BitBuffer`** — bit-granular; it tracks an *exact* bit length, so its bit size
  need not be a multiple of eight (its byte size rounds up).

Both are exposed in Python, Node, and Rust. In Rust they implement the
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

!!! info "Rust core only"
    A `ByteBuffer` / `BitBuffer` is pure random access: it keeps no cursor, so
    `Whence.Current` reads as `Whence.Start`. Wrap one in a **`RawIOCursor`** for a
    position that advances on every read and write, turning random access into a
    sequential stream. The cursor is a Rust-core convenience over the same positioned
    surface; a Python or Node caller tracks an offset itself and passes `Whence.Start`.

`RawIOCursor` owns the wrapped resource (reachable again via `get_ref` / `get_mut` /
`into_inner`). `Whence::Current` is measured from the cursor, `seek` moves it without
touching the data, and `tell` reports it in bytes; each access then advances it:

```rust
use yggdryl_core::{ByteBuffer, RawIOBase, RawIOCursor, Seekable, Whence};

fn main() {
    let mut cursor = RawIOCursor::new(ByteBuffer::from_bytes(vec![10, 20, 30, 40]));
    // Each read starts where the last one stopped.
    assert_eq!(cursor.pread_byte_array(0, Whence::Current, 2).unwrap(), vec![10, 20]);
    assert_eq!(cursor.tell(), 2);
    assert_eq!(cursor.pread_byte_array(0, Whence::Current, 2).unwrap(), vec![30, 40]);
    assert_eq!(cursor.tell(), 4);

    // seek moves the cursor without touching the data.
    cursor.seek(0, Whence::Start).unwrap();
    cursor.pwrite_byte_array(0, Whence::Current, &[1, 2]).unwrap();
    assert_eq!(cursor.get_ref().as_bytes(), &[1, 2, 30, 40]);
}
```

`IOCursor<I>` is the same adapter for a typed [`IOBase<T>`](#iobaset) resource: its
typed `pwrite_one` / `pwrite_array` stream `T` values out one after another, each
advancing the cursor through the byte layer.

Any `RawIOBase` builds one with `.cursor()` (and any `IOBase<T>` the typed one with
`IOBase::<T>::cursor(resource)`), consuming the resource — recover it later with
`into_inner`.

## Slices

!!! info "Rust core only"
    A **`RawIOSlice`** bounds a resource to a byte window `[start, end)`: code handed
    the slice sees a smaller resource and cannot read or write outside it. Like the
    cursor it is a Rust-core convenience; a Python or Node caller passes explicit
    offsets instead.

`RawIOSlice` owns the wrapped resource and offsets every access by `start`, bounded by
`end`. `byte_size` reports the backed length within the window, writes may grow the
inner up to `end` (never past it), and `resize_bytes` moves the `end` bound:

```rust
use yggdryl_core::{ByteBuffer, RawIOBase, RawIOSlice, Whence};

fn main() {
    let mut slice = RawIOSlice::new(ByteBuffer::from_bytes(vec![10, 20, 30, 40, 50]), 1, 4);
    // The window is bytes [1, 4): positions are relative to it.
    assert_eq!(slice.byte_size(), 3);
    assert_eq!(slice.pread_byte_array(0, Whence::Start, 3).unwrap(), vec![20, 30, 40]);
    assert!(slice.pread_byte_one(3, Whence::Start).is_err()); // outside the window

    // Writes stay within the window and reach the underlying buffer.
    slice.pwrite_byte_one(0, Whence::Start, 99).unwrap();
    assert_eq!(slice.get_ref().as_bytes(), &[10, 99, 30, 40, 50]);
}
```

`IOSlice<I>` is the typed counterpart over an [`IOBase<T>`](#iobaset) resource: `size`
counts the whole `T` items in the window. Build them with `resource.slice(start, end)`
(or `IOBase::<T>::slice(resource, start, end)` for the typed one).

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
    `pread_io` / `pwrite_io` copy between any two `RawIOBase` resources in 64 KiB
    chunks, so a large transfer never materializes in full. They borrow two resources
    at once and stay in the Rust core; a Python or Node caller composes the same
    effect from `pread_byte_array` + `pwrite_byte_array`.

```rust
use yggdryl_core::{ByteBuffer, RawIOBase, Whence};

fn main() {
    let source = ByteBuffer::from_bytes(vec![1, 2, 3, 4, 5, 6, 7, 8]);
    let mut sink = ByteBuffer::new();
    // Copy four bytes from source@2 into sink@0, chunked.
    source.pread_io(2, Whence::Start, 4, &mut sink, 0, Whence::Start).unwrap();
    assert_eq!(sink.as_bytes(), &[3, 4, 5, 6]);
}
```

The mutable side's start is resolved once against its current size, so `Whence::End`
stays anchored even while the sink grows during the copy.

## The traits

The buffers implement a small trait stack (Rust). Any resource that reads or writes
byte/bit sequences implements it, and generic transfer code targets the traits rather
than a concrete buffer.

### `RawIOBase`

The positioned byte/bit surface. Implementors provide the four array primitives
(`pread_byte_array`, `pwrite_byte_array`, `pread_bit_array`, `pwrite_bit_array`) plus
`byte_size` and `resize_bytes`; the `*_one` accessors, `bit_size`, the capacities, the
capacity/`resize_bits` hints, and the `pread_io` / `pwrite_io` streams come free from
defaults. It keeps no cursor, so `Whence::Current` reads as `Whence::Start`.

### `IOBase<T>`

`IOBase<T>: RawIOBase` layers typed values on top. Given a type that already
implements `RawIOBase`, provide `value_to_bytes` (how a `T` becomes bytes), `size`
and `resize` (counted in items); the typed writes `pwrite_one` / `pwrite_array` then
come free, serializing through it into the raw byte methods.

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
