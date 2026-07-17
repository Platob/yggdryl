# The memory layer

`memory` is yggdryl's **abstract byte / memory-access layer** — the traits that define
positioned and cursor access to a byte region, independent of *where the bytes live*. A concrete
**source** implements them, so everything above reads and writes through one contract. The in-heap
source is [`Heap`](#heap); a memory-mapped source (and others) plug in against the same traits.

## The contract

| Trait / type | What it adds |
|---|---|
| `IOBase` | positioned access — the `pread_byte_array` / `pwrite_byte_array` primitives, the typed `byte` / `bit` / `i32` / `i64` accessors (`pread_i32`, `pwrite_byte`, …), the buffer-reusing `pread_into` transfer, `byte_size` / `bit_size`, and `Vec`-like `capacity` / `reserve` |
| `IOCursor` | a moving position over an `IOBase`: `read` / `write` advance it, `seek` moves it relative to a [`Whence`] anchor, typed `read_byte` / `read_i32` / `read_i64`, plus the bounded bulk readers (`read_to_end`, `read_exact_vec`) |
| `IOSlice` | a bounded sub-range window over an `IOBase`, addressed from its own `0` |
| `Whence` | the seek anchor: `Start` / `Current` / `End` |
| `IoError` | the guided failures the byte-access methods return (`UnexpectedEof` / `InvalidSeek` / `SliceOutOfBounds`) |

Bit addressing is **LSB-first** (bit `i` is bit `i % 8` of byte `i / 8`, least-significant first),
and integers are **little-endian**, matching Arrow. The two byte-array primitives are infallible
(a read past the end returns fewer bytes; a write past the end grows the source, zero-filling any
gap); the typed and *exact* helpers built on them return a guided error at the end of the data.

## `Heap`

The in-heap source — an owned byte buffer with a read/write cursor and `Vec`-like capacity. It is
the reference implementor of `IOBase` / `IOCursor` / `IOSlice`. Equality is over the stored bytes
(the cursor is transient), and — being a mutable buffer, like `bytearray` — it is intentionally
not hashable.

The **core** names each accessor by the exact type it moves (`pread_i32`, `write_byte`); the
**bindings** keep those explicit names and add generic, type-inferring entry points — the `Heap`
constructor accepts a bytes value (or nothing) and infers what to build.

=== "Python"

    ```python
    from yggdryl.memory import Heap, Whence

    h = Heap()                       # or Heap(b"..."), or Heap.with_capacity(32)
    h.write_byte(0x01)
    h.write_i32(-42)
    h.write_i64(2**40)

    # Positioned little-endian reads (bits are LSB-first).
    assert h.pread_byte(0) == 0x01
    assert h.pread_i32(1) == -42
    assert h.pread_i64(5) == 2**40

    # Cursor stream + seek from the end.
    h.rewind()
    assert h.read_byte() == 0x01
    h.seek(Whence.End, -8)
    assert h.read_i64() == 2**40

    # A bounded window, addressed from its own 0.
    window = h.slice(1, 4)
    assert len(window) == 4
    assert window.to_bytes() == bytes(h)[1:5]

    # Heap is a mutable buffer — equatable by content, but unhashable (like bytearray).
    assert Heap(b"abc") == Heap(b"abc")
    ```

=== "Node"

    ```js
    const { Heap, Whence } = require('yggdryl').memory

    const h = new Heap()             // or new Heap(Buffer.from('...')), or Heap.withCapacity(32)
    h.writeByte(0x01)
    h.writeI32(-42)
    h.writeI64(2 ** 40)

    // Positioned little-endian reads (bits are LSB-first).
    console.assert(h.preadByte(0) === 0x01)
    console.assert(h.preadI32(1) === -42)
    console.assert(h.preadI64(5) === 2 ** 40)

    // Cursor stream + seek from the end.
    h.rewind()
    console.assert(h.readByte() === 0x01)
    h.seek(Whence.End, -8)
    console.assert(h.readI64() === 2 ** 40)

    // A bounded window, addressed from its own 0.
    const window = h.slice(1, 4)
    console.assert(window.byteSize() === 4)

    // Equatable by content.
    console.assert(new Heap(Buffer.from('abc')).equals(new Heap(Buffer.from('abc'))))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::memory::{Heap, IOBase, IOCursor, IOSlice, Whence};

    let mut h = Heap::with_capacity(32);
    h.write_byte(0x01).unwrap();
    h.write_i32(-42).unwrap();
    h.write_i64(1 << 40).unwrap();

    // Positioned little-endian reads (bits are LSB-first).
    assert_eq!(h.pread_byte(0).unwrap(), 0x01);
    assert_eq!(h.pread_i32(1).unwrap(), -42);
    assert_eq!(h.pread_i64(5).unwrap(), 1 << 40);

    // Cursor stream + seek from the end.
    h.rewind();
    assert_eq!(h.read_byte().unwrap(), 0x01);
    h.seek(Whence::End, -8).unwrap();
    assert_eq!(h.read_i64().unwrap(), 1 << 40);

    // A bounded window, addressed from its own 0.
    let window = h.slice(1, 4).unwrap();
    assert_eq!(window.byte_size(), 4);
    ```

## Zero-copy transfers

In the Rust core, `pread_into(offset, len, &mut buf)` reads into a caller-owned `Vec`, **reusing
its allocation** across a whole transfer loop — one warm buffer, zero allocations per chunk, versus
`pread_vec`'s fresh `Vec` per call. The [`heap` benchmark](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/heap.md)
measures the difference and the `memory_heap_alloc` test pins the counts. The bindings return owned
byte objects (`bytes` / `Buffer`) from `pread_byte_array`, so this reuse is a Rust-core capability.
