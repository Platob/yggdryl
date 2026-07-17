# The memory layer

`memory` is yggdryl's **abstract byte / memory-access layer** — the `IOBase` contract that defines
positioned access to a byte region, independent of *where the bytes live*, plus the concrete pieces
built over it. A **source** implements `IOBase`, so everything above reads and writes through one
contract. The in-heap source is [`Heap`](#heap); a memory-mapped source (and others) plug in
against the same contract.

## The contract

| Type | What it is |
|---|---|
| `IOBase` | the **source contract** — the `pread_byte_array` / `pwrite_byte_array` primitives, the typed `byte` / `bit` / `i32` / `i64` accessors (`pread_i32`, `pwrite_byte`, …), the buffer-reusing `pread_into` transfer, `byte_size` / `bit_size`, `Vec`-like `capacity` / `reserve`, an addressing [`uri`](#addressing), and the [`cursor()` / `window()`](#cursors-and-windows) builders |
| `IOCursor<T>` | a concrete **cursor** wrapping any source: `read` / `write` advance a position, `seek` moves it relative to a [`Whence`] anchor, typed `read_byte` / `read_i32` / `read_i64`, and the bounded bulk readers (`read_to_end`, `read_exact_vec`) |
| `IOSlice<T>` | a concrete bounded **window** wrapping any source, addressed from its own `0` |
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
    use yggdryl_core::memory::{Heap, IOBase, Whence};

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

## Addressing

Every source carries an addressing [`Uri`](uri.md) — `uri()` on any `IOBase`. A raw scratch buffer
reports the empty URI; a `Heap` can be given one (`with_uri` / `set_uri`), and the `cursor()` /
`window()` wrappers delegate to their inner source's. The address is metadata: it is **not** part of
a heap's value equality (two heaps with the same bytes are equal regardless of address).

=== "Python"

    ```python
    from yggdryl.memory import Heap
    from yggdryl.uri import Uri

    h = Heap(b"data").with_uri(Uri.parse("mem://scratch/a"))
    assert h.uri.host == "scratch"
    assert h == Heap(b"data")           # address is not part of equality
    ```

=== "Node"

    ```js
    const { Heap } = require('yggdryl').memory
    const { Uri } = require('yggdryl').uri

    const h = new Heap(Buffer.from('data')).withUri(Uri.parse('mem://scratch/a'))
    console.assert(h.uri.host === 'scratch')
    console.assert(h.equals(new Heap(Buffer.from('data'))))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::memory::{Heap, IOBase};
    use yggdryl_core::uri::Uri;

    let h = Heap::from_slice(b"data").with_uri(Uri::parse_str("mem://scratch/a").unwrap());
    assert_eq!(h.uri().host(), Some("scratch"));
    assert_eq!(h, Heap::from_slice(b"data")); // address is not part of equality
    ```

## Cursors and windows

`Heap` has a built-in cursor and a materialized `slice` (a copy), but the cursor and window are also
**standalone wrappers over any source**: `cursor()` returns an [`IOCursor<T>`](#the-contract) (a
moving position), and `window(offset, len)` returns an [`IOSlice<T>`](#the-contract) (a bounded view
addressed from its own `0`). Both are themselves `IOBase`, so they compose — a window of a window, a
cursor over a window. In the bindings these are the `Cursor` and `Slice` classes.

=== "Python"

    ```python
    from yggdryl.memory import Heap

    cur = Heap(b"").cursor()      # a cursor over a fresh source
    cur.write_i32(-7)
    cur.rewind()
    assert cur.read_i32() == -7

    win = Heap(b"hello world").window(6, 5)  # a bounded view, no copy of the tail
    assert bytes(win) == b"world"
    assert len(win) == 5
    ```

=== "Node"

    ```js
    const { Heap } = require('yggdryl').memory

    const cur = new Heap(Buffer.alloc(0)).cursor()
    cur.writeI32(-7)
    cur.rewind()
    console.assert(cur.readI32() === -7)

    const win = new Heap(Buffer.from('hello world')).window(6, 5)
    console.assert(win.toBytes().toString() === 'world')
    console.assert(win.byteSize() === 5)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::memory::{Heap, IOBase};

    let mut cur = Heap::new().cursor();            // IOCursor<Heap>
    cur.write_i32(-7).unwrap();
    cur.rewind();
    assert_eq!(cur.read_i32().unwrap(), -7);

    let win = Heap::from_slice(b"hello world").window(6, 5).unwrap(); // IOSlice<Heap>
    assert_eq!(win.pread_vec(0, 5), b"world");
    assert_eq!(win.byte_size(), 5);
    ```

## Zero-copy transfers

In the Rust core, `pread_into(offset, len, &mut buf)` reads into a caller-owned `Vec`, **reusing
its allocation** across a whole transfer loop — one warm buffer, zero allocations per chunk, versus
`pread_vec`'s fresh `Vec` per call. The [`heap` benchmark](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/heap.md)
measures the difference and the `memory_heap_alloc` test pins the counts. The bindings return owned
byte objects (`bytes` / `Buffer`) from `pread_byte_array`, so this reuse is a Rust-core capability.
