# The memory layer

`memory` is yggdryl's **abstract byte / memory-access layer** — the `IOBase` contract that defines
positioned access to a byte region, independent of *where the bytes live*, plus the concrete pieces
built over it. A **source** implements `IOBase`, so everything above reads and writes through one
contract. `IOBase` is also the **central access path**: it carries the addressing `uri`, the
[graph surface](local.md#the-graph-surface) (`ls` streaming children of the same type, `rm`
CRUD), and the [memory-tree](local.md#a-directory-is-a-memory-tree) container reads — the
in-memory sources here are leaves. The in-heap source is [`Heap`](#heap); the
local-filesystem family (`LocalIO`, the single access point, over the raw `Mmap`) lives on
the [local page](local.md) and implements the same contract.

## The contract

| Type | What it is |
|---|---|
| `IOBase` | the **source contract** — the `pread_byte_array` / `pwrite_byte_array` primitives; the typed `byte` / `bit` / `i32` / `i64` accessors (`pread_i32`, `pwrite_byte`, …); [bulk vectorized arrays, repeated-value fills, and UTF-8 text](#bulk-repeated-and-text-io); the buffer-reusing `pread_into` transfer; `byte_size` / `bit_size`; the full `Vec`-like capacity family — `capacity` / `spare_capacity`, `reserve` / `reserve_exact` and the **checked** `try_reserve` / `try_reserve_exact` (a guided error instead of an abort), the absolute-target `ensure_capacity` / `try_ensure_capacity`, `shrink_to_fit` / `shrink_to`, and a pre-allocating `with_capacity(capacity)` builder — with amortized (auto-scaling) growth on appends; an addressing [`uri`](#addressing) plus [`headers` metadata, an access `mode`, and a `kind`](#metadata-mode-and-kind); the [`cursor()` / `window()`](#cursors-and-windows) builders; and the **graph surface** — `ls` / `ls_recursive` streaming children of the same source type (`children` collected), `name` / `parent`, `rm` / `rmfile` / `rmdir`, and the `tree_*` [memory-tree](local.md#a-directory-is-a-memory-tree) container methods |
| `IOCursor<T>` | a concrete **cursor** wrapping any source: `read` / `write` advance a position, `seek` moves it relative to a [`Whence`] anchor, typed `read_byte` / `read_i32` / `read_i64` / `read_utf8`, and the bounded bulk readers (`read_to_end`, `read_exact_vec`) |
| `IOSlice<T>` | a concrete bounded **window** wrapping any source, addressed from its own `0` |
| `Whence` | the seek anchor: `Start` / `Current` / `End` |
| `IoError` | the guided failures the byte-access methods return (`UnexpectedEof` / `InvalidSeek` / `SliceOutOfBounds` / `InvalidUtf8` / `UnknownName` / `CapacityOverflow` / `FileIo`) |

Bit addressing is **LSB-first** (bit `i` is bit `i % 8` of byte `i / 8`, least-significant first),
and integers are **little-endian**, matching Arrow. The two byte-array primitives are infallible
(a read past the end returns fewer bytes; a write past the end grows the source, zero-filling any
gap); the typed and *exact* helpers built on them return a guided error at the end of the data.
When the size is known up front, build with `with_capacity(capacity)` (or `ensure_capacity` on
a live source) so the first writes never reallocate; when it is not, appends auto-scale with
amortized doubling — 64 chunked appends cost only ~7 reallocations (asserted). For sizes that
may be hostile or miscomputed, use the **checked** `try_reserve` family: a guided error instead
of a process abort.

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
    use yggdryl_core::io::memory::{Heap, IOBase, Whence};

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

Every source carries an addressing [`Uri`](../uri.md) — `uri()` on any `IOBase`. An in-memory
source stores **no address**: every `Heap` reports the **`mem` scheme**'s stable synthetic
address `mem://heap` (deterministic — an anonymous buffer has no other identity, and the real
allocation address is deliberately not leaked). The address is **lazy-built**: parsed once into
a process-wide static and cloned per call, never re-parsed. A source with a real address (a
future file/network source) reports its own; the `cursor()` / `window()` wrappers delegate to
their inner source's.

=== "Python"

    ```python
    from yggdryl.memory import Heap

    h = Heap(b"data")
    assert str(h.uri) == "mem://heap"     # every heap: the synthetic address
    assert h.uri.scheme == "mem" and h.uri.host == "heap"
    ```

=== "Node"

    ```js
    const { Heap } = require('yggdryl').memory

    const h = new Heap(Buffer.from('data'))
    console.assert(h.uri.toString() === 'mem://heap')  // every heap: the synthetic address
    console.assert(h.uri.scheme === 'mem' && h.uri.host === 'heap')
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::memory::{Heap, IOBase};

    let h = Heap::from_slice(b"data");
    assert_eq!(h.uri().to_string(), "mem://heap"); // every heap: the synthetic address
    assert_eq!(h.uri().scheme(), Some("mem"));
    ```

## Metadata, mode, and kind

Beyond its address, every source reports three more facets — all delegated by the wrappers:

- **`headers()` / `headers_mut()`** — the source's metadata, as the project-wide
  [`Headers`](../headers.md) map (there is exactly one metadata type).
  In the bindings `heap.headers` returns a **copy**; write back with `set_headers` /
  `with_headers`.
- **`mode()`** — how the source may be accessed, an [`IOMode`](index.md#iomode-and-iokind-int-enums-with-parsers)
  (`ReadWrite` by default for in-memory sources; settable on `Heap`).
- **`kind()`** — what the source *is*, an [`IOKind`](index.md#iomode-and-iokind-int-enums-with-parsers)
  (`Heap` reports `IOKind::Heap`; a file source reports `File` / `Directory` / `Missing`).

Like the address, all three are metadata — excluded from a heap's value equality.

=== "Python"

    ```python
    from yggdryl.headers import Headers
    from yggdryl.io import IOKind, IOMode
    from yggdryl.memory import Heap

    h = Heap(b"x")
    meta = h.headers                 # a copy — mutate then write back
    meta.insert("Content-Type", "text/plain")
    h.set_headers(meta)
    assert h.headers.content_type() == "text/plain"

    assert h.mode == IOMode.ReadWrite and h.kind == IOKind.Heap
    ro = h.with_mode(IOMode.Read)
    assert ro.mode == IOMode.Read
    ```

=== "Node"

    ```js
    const { Headers } = require('yggdryl').headers
    const { IOMode, IOKind } = require('yggdryl').io
    const { Heap } = require('yggdryl').memory

    const h = new Heap(Buffer.from('x'))
    const meta = h.headers                    // a copy — mutate then write back
    meta.insert('Content-Type', 'text/plain')
    h.setHeaders(meta)
    console.assert(h.headers.contentType() === 'text/plain')

    console.assert(h.mode === IOMode.ReadWrite && h.kind === IOKind.Heap)
    console.assert(h.withMode(IOMode.Read).mode === IOMode.Read)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::memory::{Heap, IOBase};
    use yggdryl_core::io::{IOKind, IOMode};

    let mut h = Heap::from_slice(b"x");
    h.headers_mut().insert("Content-Type", "text/plain"); // direct mutable access
    assert_eq!(h.headers().content_type(), Some("text/plain"));

    assert_eq!(h.mode(), IOMode::ReadWrite);
    assert_eq!(h.kind(), IOKind::Heap);
    assert_eq!(h.with_mode(IOMode::Read).mode(), IOMode::Read);
    ```

## Bulk, repeated, and text IO

The typed accessors scale up to **vectorized bulk** forms that stage through fixed stack chunks
(zero heap allocation; the dense conversion loops auto-vectorize on stable Rust), a
**repeated-value fill** that never materializes the full array, and **UTF-8 text** built on the
byte layer (invalid bytes are a guided error).

=== "Python"

    ```python
    from yggdryl.memory import Heap

    h = Heap()
    h.pwrite_i32_array(0, [1, -2, 3])            # bulk write
    assert h.pread_i32_array(0, 3) == [1, -2, 3] # bulk read

    h.pwrite_i32_repeat(12, -1, 1000)            # fill: no 1000-element list is built
    assert h.pread_i32(12 + 999 * 4) == -1

    text = Heap()
    text.pwrite_utf8(0, "héllo")
    assert text.pread_utf8(0, 6) == "héllo"      # é is 2 bytes
    ```

=== "Node"

    ```js
    const { Heap } = require('yggdryl').memory

    const h = new Heap()
    h.pwriteI32Array(0, [1, -2, 3])                        // bulk write
    console.assert(h.preadI32Array(0, 3).join() === '1,-2,3')

    h.pwriteI32Repeat(12, -1, 1000)                        // fill: no array is built
    console.assert(h.preadI32(12 + 999 * 4) === -1)

    const text = new Heap()
    text.pwriteUtf8(0, 'héllo')
    console.assert(text.preadUtf8(0, 6) === 'héllo')       // é is 2 bytes
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::memory::{Heap, IOBase};

    let mut h = Heap::new();
    h.pwrite_i32_array(0, &[1, -2, 3]).unwrap();           // bulk write
    let mut back = [0i32; 3];
    h.pread_i32_array(0, &mut back).unwrap();              // bulk read
    assert_eq!(back, [1, -2, 3]);

    h.pwrite_i32_repeat(12, -1, 1000).unwrap();            // fill: no array is built
    assert_eq!(h.pread_i32(12 + 999 * 4).unwrap(), -1);

    let mut text = Heap::new();
    text.pwrite_utf8(0, "héllo");
    assert_eq!(text.pread_utf8(0, 6).unwrap(), "héllo");   // é is 2 bytes
    ```

The [`io_memory_heap` benchmark](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/io/memory/heap.md)
pins the claims: bulk arrays run allocation-free at multi-Gelem/s, and `pwrite_i32_repeat` is
~3.5× the build-a-full-array path.

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

    win = Heap(b"hello world").window(6, 5)  # a bounded window over its own copy of the source
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
    use yggdryl_core::io::memory::{Heap, IOBase};

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
`pread_vec`'s fresh `Vec` per call. The [`heap` benchmark](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/io/memory/heap.md)
measures the difference and the `io_memory_heap_alloc` test pins the counts. The bindings return owned
byte objects (`bytes` / `Buffer`) from `pread_byte_array`, so this reuse is a Rust-core capability.
