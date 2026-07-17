# The memory layer

`memory` is yggdryl's **abstract byte / memory-access layer** — the `IOBase` contract that defines
positioned access to a byte region, independent of *where the bytes live*, plus the concrete pieces
built over it. A **source** implements `IOBase`, so everything above reads and writes through one
contract. `IOBase` is also the **central access path**: it carries the addressing `uri`, the
[graph surface](local.md#the-graph-surface) (`join` / `parent` / `parents` / `name` navigation,
`ls` streaming children of the same type, `rm` CRUD), and the
[memory-tree](local.md#a-directory-is-a-memory-tree) container reads — the in-memory sources here
are leaves. The in-heap source is [`Heap`](#heap); the local-filesystem family (`LocalIO`, the
single access point, over the raw `Mmap`) lives on the [local page](local.md) and implements the
same contract.

This page is a cookbook that grows in complexity: start with a `Heap` and positioned typed
access, then bits and text, vectorized bulk operations, capacity control, cursors and windows,
addressing and the IO graph, and finally value identity.

## The contract

| Type | What it is |
|---|---|
| `IOBase` | the **source contract** — the `pread_byte_array` / `pwrite_byte_array` primitives; the typed `byte` / `bit` / `i32` / `i64` accessors (`pread_i32`, `pwrite_byte`, …); [bulk vectorized arrays and repeated-value fills](#bulk-arrays-and-repeated-fills) plus [bit addressing and UTF-8 text](#bits-and-utf-8-text); the buffer-reusing `pread_into` transfer; `byte_size` / `bit_size`; the full `Vec`-like [capacity family](#capacity-discipline) — `capacity` / `spare_capacity`, `reserve` / `reserve_exact` and the **checked** `try_reserve` / `try_reserve_exact` (a guided error instead of an abort), the absolute-target `ensure_capacity` / `try_ensure_capacity`, `shrink_to_fit` / `shrink_to`, and a pre-allocating `with_capacity(capacity)` builder — with amortized (auto-scaling) growth on appends; an addressing [`uri`](#addressing) plus [`headers` metadata, an access `mode`, and a `kind`](#metadata-mode-and-kind); the [`cursor()` / `window()`](#cursors-and-windows) builders; and the [**graph surface**](#the-graph-surface-every-source-is-a-node) — `join` / `parent` / `parents` / `name` navigation, `ls` / `ls_recursive` streaming children of the same source type (`children` collected), `rm` / `rmfile` / `rmdir`, and the `tree_*` [memory-tree](local.md#a-directory-is-a-memory-tree) container methods |
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
constructor accepts a bytes value (or nothing) and infers what to build. Start with the essentials:
construct a buffer, write and read typed values at absolute offsets, stream through the built-in
cursor, and take a bounded window.

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

## Bits and UTF-8 text

Two finer-grained accessors sit on the same byte primitives. **Bit addressing** is LSB-first — bit
`i` is bit `i % 8` of byte `i / 8`, least-significant first, matching Arrow validity bitmaps —
through `pread_bit` / `pwrite_bit`; setting a bit past the end grows the buffer, zero-filling the
gap and read-modify-writing its byte. **UTF-8 text** (`pwrite_utf8` / `pread_utf8`) writes and reads
text over the same bytes: a write returns the number of **bytes** (not characters), and a read
decodes a byte range, raising a guided error that names the offending byte on invalid UTF-8 —
including a multi-byte character the range cuts in half.

=== "Python"

    ```python
    from yggdryl.memory import Heap

    h = Heap()
    h.pwrite_bit(0, True)                 # bit 0 (LSB of byte 0)
    h.pwrite_bit(2, True)                 # bit 2 of byte 0
    h.pwrite_bit(10, True)                # grows to 2 bytes; bit 2 of byte 1
    assert h.pread_bit(0) and h.pread_bit(2) and h.pread_bit(10)
    assert not h.pread_bit(1)
    assert bytes(h) == bytes([0b0000_0101, 0b0000_0100])

    text = Heap()
    assert text.pwrite_utf8(0, "héllo") == 6   # bytes written — é is 2 bytes
    assert text.pread_utf8(0, 6) == "héllo"
    try:
        text.pread_utf8(0, 2)                  # cuts é in half
    except ValueError as e:
        assert "invalid UTF-8" in str(e)
    ```

=== "Node"

    ```js
    const { Heap } = require('yggdryl').memory

    const h = new Heap()
    h.pwriteBit(0, true)                   // bit 0 (LSB of byte 0)
    h.pwriteBit(2, true)                   // bit 2 of byte 0
    h.pwriteBit(10, true)                  // grows to 2 bytes; bit 2 of byte 1
    console.assert(h.preadBit(0) && h.preadBit(2) && h.preadBit(10))
    console.assert(!h.preadBit(1))
    console.assert(h.toBytes().equals(Buffer.from([0b00000101, 0b00000100])))

    const text = new Heap()
    console.assert(text.pwriteUtf8(0, 'héllo') === 6)   // bytes written — é is 2 bytes
    console.assert(text.preadUtf8(0, 6) === 'héllo')
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::memory::{Heap, IOBase};

    let mut h = Heap::new();
    h.pwrite_bit(0, true).unwrap();        // bit 0 (LSB of byte 0)
    h.pwrite_bit(2, true).unwrap();        // bit 2 of byte 0
    h.pwrite_bit(10, true).unwrap();       // grows to 2 bytes; bit 2 of byte 1
    assert!(h.pread_bit(0).unwrap() && h.pread_bit(2).unwrap() && h.pread_bit(10).unwrap());
    assert!(!h.pread_bit(1).unwrap());
    assert_eq!(h.as_slice(), &[0b0000_0101, 0b0000_0100]);

    let mut text = Heap::new();
    assert_eq!(text.pwrite_utf8(0, "héllo"), 6);         // bytes written — é is 2 bytes
    assert_eq!(text.pread_utf8(0, 6).unwrap(), "héllo");
    assert!(text.pread_utf8(0, 2).is_err());             // cuts é in half
    ```

## Bulk arrays and repeated fills

The typed accessors scale up to **vectorized bulk** array reads and writes and a **repeated-value
fill**. Bulk arrays stage through fixed stack chunks — zero heap allocation — and convert in dense,
branch-free loops LLVM auto-vectorizes on stable Rust (no SIMD dependency); a `Heap` over its
contiguous `Vec` converts the whole range in a single pass. A repeated-value fill
(`pwrite_byte_repeat` / `pwrite_i32_repeat` / `pwrite_i64_repeat`) never materializes the full
array — one stack chunk is filled once and written repeatedly, the `memset` of the family. The
bindings return a freshly allocated list/array (with a fail-fast bounds check *before* allocating,
so a hostile `count` never triggers a runaway allocation); the Rust core fills a caller-provided
slice.

=== "Python"

    ```python
    from yggdryl.memory import Heap

    h = Heap()
    h.pwrite_i32_array(0, [1, -2, 3])            # bulk write
    assert h.pread_i32_array(0, 3) == [1, -2, 3] # bulk read

    h.pwrite_i64_array(64, [10, 20])
    assert h.pread_i64_array(64, 2) == [10, 20]

    h.pwrite_i32_repeat(128, -1, 1000)          # fill: no 1000-element list is built
    assert h.pread_i32(128 + 999 * 4) == -1

    h.pwrite_byte_repeat(256, 0xAB, 4)          # the byte-level memset
    assert h.pread_byte_array(256, 4) == b"\xab\xab\xab\xab"
    ```

=== "Node"

    ```js
    const { Heap } = require('yggdryl').memory

    const h = new Heap()
    h.pwriteI32Array(0, [1, -2, 3])                        // bulk write
    console.assert(h.preadI32Array(0, 3).join() === '1,-2,3')

    h.pwriteI64Array(64, [10, 20])
    console.assert(h.preadI64Array(64, 2).join() === '10,20')

    h.pwriteI32Repeat(128, -1, 1000)                      // fill: no array is built
    console.assert(h.preadI32(128 + 999 * 4) === -1)

    h.pwriteByteRepeat(256, 0xAB, 4)                      // the byte-level memset
    console.assert(h.preadByteArray(256, 4).equals(Buffer.from([0xAB, 0xAB, 0xAB, 0xAB])))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::memory::{Heap, IOBase};

    let mut h = Heap::new();
    h.pwrite_i32_array(0, &[1, -2, 3]).unwrap();           // bulk write
    let mut back = [0i32; 3];
    h.pread_i32_array(0, &mut back).unwrap();              // bulk read into a caller slice
    assert_eq!(back, [1, -2, 3]);

    h.pwrite_i64_array(64, &[10, 20]).unwrap();
    let mut wide = [0i64; 2];
    h.pread_i64_array(64, &mut wide).unwrap();
    assert_eq!(wide, [10, 20]);

    h.pwrite_i32_repeat(128, -1, 1000).unwrap();           // fill: no array is built
    assert_eq!(h.pread_i32(128 + 999 * 4).unwrap(), -1);

    h.pwrite_byte_repeat(256, 0xAB, 4).unwrap();           // the byte-level memset
    assert_eq!(h.pread_vec(256, 4), b"\xab\xab\xab\xab");
    ```

The [`io_memory_heap` benchmark](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/io/memory/heap.md)
pins the claims: bulk arrays run allocation-free at multi-Gelem/s, and `pwrite_i32_repeat` is
~3.5× the build-a-full-array path.

## Capacity discipline

When the final size is known, pre-allocate so the first writes never reallocate; when it is not,
appends auto-scale with amortized doubling. `Heap` mirrors `Vec`'s full capacity surface:
`with_capacity` builds pre-sized; `capacity` / `spare_capacity` report the allocation and its unused
tail; `reserve` (amortized) and `reserve_exact` (no over-allocation) grow headroom past the current
size; `ensure_capacity` grows to an absolute total; `shrink_to_fit` / `shrink_to` release it. For
sizes that may be hostile or miscomputed, the **checked** `try_reserve` / `try_reserve_exact` /
`try_ensure_capacity` family returns the guided `CapacityOverflow` error (`"cannot reserve … the
size overflows or the allocator refused"`) instead of the **process abort** the unchecked `reserve`
would trigger.

=== "Python"

    ```python
    from yggdryl.memory import Heap

    h = Heap.with_capacity(1024)             # pre-allocated: the first writes never reallocate
    assert h.is_empty() and h.capacity() >= 1024

    h.pwrite_byte_array(0, b"\x00" * 16)
    assert h.spare_capacity() >= 1008        # room already allocated, minus the 16 written

    h.reserve(4096)                          # amortized headroom past the current size
    h.ensure_capacity(8192)                  # absolute target; never shrinks
    assert h.capacity() >= 8192

    # Checked growth: a hostile size is a guided error, never a process abort.
    h.try_reserve(64)                        # fine
    try:
        h.try_reserve(2**63)                 # would overflow
    except ValueError as e:
        assert "cannot reserve" in str(e)

    h.shrink_to_fit()                        # release the spare back to the allocator
    ```

=== "Node"

    ```js
    const { Heap } = require('yggdryl').memory

    const h = Heap.withCapacity(1024)        // pre-allocated: the first writes never reallocate
    console.assert(h.isEmpty() && h.capacity() >= 1024)

    h.pwriteByteArray(0, Buffer.alloc(16))
    console.assert(h.spareCapacity() >= 1008)   // room already allocated, minus the 16 written

    h.reserve(4096)                          // amortized headroom past the current size
    h.ensureCapacity(8192)                   // absolute target; never shrinks
    console.assert(h.capacity() >= 8192)

    // Checked growth: a hostile size is a guided error, never a process abort.
    h.tryReserve(64)                         // fine
    try {
      h.tryReserve(2 ** 53)                  // the allocator refuses
    } catch (e) {
      console.assert(e.message.includes('cannot reserve'))
    }

    h.shrinkToFit()                          // release the spare back to the allocator
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::memory::{Heap, IOBase, IoError};

    let mut h = Heap::with_capacity(1024);   // pre-allocated: the first writes never reallocate
    assert!(h.is_empty() && h.capacity() >= 1024);

    h.pwrite_byte_array(0, &[0u8; 16]);
    assert!(h.spare_capacity() >= 1008);     // room already allocated, minus the 16 written

    h.reserve(4096);                         // amortized headroom past the current size
    h.ensure_capacity(8192);                 // absolute target; never shrinks
    assert!(h.capacity() >= 8192);

    // Checked growth: a hostile size is a guided error, never a process abort.
    h.try_reserve(64).unwrap();              // fine
    let err = h.try_reserve(u64::MAX).unwrap_err();
    assert!(matches!(err, IoError::CapacityOverflow { .. }));

    h.shrink_to_fit();                       // release the spare back to the allocator
    ```

## Cursors and windows

`Heap` has a built-in cursor and a materialized `slice` (a copy), but the cursor and window are also
**standalone wrappers over any source**: `cursor()` returns an [`IOCursor<T>`](#the-contract) (a
moving position seeked relative to a `Whence` anchor — `Start` / `Current` / `End`, the POSIX
`SEEK_SET` / `SEEK_CUR` / `SEEK_END`), and `window(offset, len)` returns an
[`IOSlice<T>`](#the-contract) (a bounded view addressed from its own `0`, fixed-length, so a write
past its end is clamped away). Both are themselves `IOBase`, so they compose — a window of a window,
a cursor over a window. In the bindings these are the `Cursor` and `Slice` classes.

=== "Python"

    ```python
    from yggdryl.memory import Heap, Whence

    cur = Heap(b"").cursor()          # a cursor over a fresh source
    cur.write_i32(-7)
    cur.write_i32(99)
    cur.seek(Whence.Start, 0)         # SEEK_SET — back to the front
    assert cur.read_i32() == -7
    cur.seek(Whence.Current, 0)       # SEEK_CUR — stay put
    assert cur.read_i32() == 99
    cur.seek(Whence.End, -4)          # SEEK_END — the last i32
    assert cur.read_i32() == 99

    win = Heap(b"hello world").window(6, 5)  # a bounded window over its own copy of the source
    assert bytes(win) == b"world"
    assert len(win) == 5
    ```

=== "Node"

    ```js
    const { Heap, Whence } = require('yggdryl').memory

    const cur = new Heap(Buffer.alloc(0)).cursor()
    cur.writeI32(-7)
    cur.writeI32(99)
    cur.seek(Whence.Start, 0)         // SEEK_SET — back to the front
    console.assert(cur.readI32() === -7)
    cur.seek(Whence.Current, 0)       // SEEK_CUR — stay put
    console.assert(cur.readI32() === 99)
    cur.seek(Whence.End, -4)          // SEEK_END — the last i32
    console.assert(cur.readI32() === 99)

    const win = new Heap(Buffer.from('hello world')).window(6, 5)
    console.assert(win.toBytes().toString() === 'world')
    console.assert(win.byteSize() === 5)
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::memory::{Heap, IOBase, Whence};

    let mut cur = Heap::new().cursor();            // IOCursor<Heap>
    cur.write_i32(-7).unwrap();
    cur.write_i32(99).unwrap();
    cur.seek(Whence::Start, 0).unwrap();           // SEEK_SET — back to the front
    assert_eq!(cur.read_i32().unwrap(), -7);
    cur.seek(Whence::Current, 0).unwrap();         // SEEK_CUR — stay put
    assert_eq!(cur.read_i32().unwrap(), 99);
    cur.seek(Whence::End, -4).unwrap();            // SEEK_END — the last i32
    assert_eq!(cur.read_i32().unwrap(), 99);

    let win = Heap::from_slice(b"hello world").window(6, 5).unwrap(); // IOSlice<Heap>
    assert_eq!(win.pread_vec(0, 5), b"world");
    assert_eq!(win.byte_size(), 5);
    ```

## Addressing

Every source carries an addressing [`Uri`](../uri.md) — `uri()` on any `IOBase`. An **anonymous**
in-memory source stores **no address**: every fresh `Heap` reports the **`mem` scheme**'s stable
synthetic address `mem://heap` (deterministic — an anonymous buffer has no other identity, and the
real allocation address is deliberately not leaked). A heap re-addressed by
[`join`](#the-graph-surface-every-source-is-a-node) carries and reports its composed place in the
URI graph (`mem://heap/logs/app.bin`) instead. The address is **lazy-built**: the default is parsed
once into a process-wide static and cloned per call, never re-parsed. A source with a real address
(a file/network source) reports its own; the `cursor()` / `window()` wrappers delegate to their
inner source's.

=== "Python"

    ```python
    from yggdryl.memory import Heap

    h = Heap(b"data")
    assert str(h.uri) == "mem://heap"     # every anonymous heap: the synthetic address
    assert h.uri.scheme == "mem" and h.uri.host == "heap"
    ```

=== "Node"

    ```js
    const { Heap } = require('yggdryl').memory

    const h = new Heap(Buffer.from('data'))
    console.assert(h.uri.toString() === 'mem://heap')  // every anonymous heap: the synthetic address
    console.assert(h.uri.scheme === 'mem' && h.uri.host === 'heap')
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::memory::{Heap, IOBase};

    let h = Heap::from_slice(b"data");
    assert_eq!(h.uri().to_string(), "mem://heap"); // every anonymous heap: the synthetic address
    assert_eq!(h.uri().scheme(), Some("mem"));
    ```

## The graph surface — every source is a node

`IOBase` is not only the byte contract; it is the **central access path**. Every source is a node
of one IO graph, addressed by its [`uri`](#addressing) and navigable by the same handful of methods
— whether the bytes live in the heap, in a memory-mapped file, or (later) in an object store. On an
in-memory buffer that surface is **pure address algebra**: bytes do not nest, but addresses do.

- **`join(segment)`** composes a child *address* — `Heap().join("logs/app.bin")` returns a **new,
  independent buffer** addressed `mem://heap/logs/app.bin`. Constructing it touches nothing and
  copies no bytes; the child owns its own (initially empty) buffer that you write and read on its
  own, and writing it never touches the parent. `segment` may be multi-segment (`"a/b/c"`); an
  **absolute** segment (leading `/`) re-roots. In Python it also spells as the `/` operator
  (`heap / "logs/app.bin"`, via `__truediv__`); Node has no operator overloading, so `join` is the
  method name there.
- **`parent()`** navigates back up one segment — the inverse of `join` — and **`parents()`**
  iterates the whole ancestor chain, nearest first, up to the `mem://heap` root. **`name`** is the
  last path segment, percent-decoded, so `mem://heap/logs/app.bin` names `app.bin`; the bare
  `mem://heap` root names nothing (`""`) and has no parent (`None` / `null`).

Because navigation composes through the URI (`Uri::joinpath` / `Uri::parent`), the child's
`parent()` addresses the original node again — the graph is consistent by construction. This is the
**same** `join` / `parent` / `parents` / `name` surface a filesystem node exposes
([`LocalIO`](local.md#the-graph-surface)); the only difference is what a read or write *does* — an
in-heap child grows its own bytes, a `LocalIO` child auto-creates a file on first write — so code
written against the graph runs over memory or disk unchanged.

A heap is a **leaf** for *discovery*: `ls()` (streamed, same source type) and `children()`
(collected) are always empty, and `rm()` / `rmfile()` / `rmdir()` **refuse** with a guided error —
an in-memory buffer has no removable backing, and the message points you at a filesystem node.
(A real [`Mmap`](local.md) file, being an actual file, instead lets `rm` unlink it.)

=== "Python"

    ```python
    from yggdryl.memory import Heap

    root = Heap()
    child = root / "logs/app.bin"                 # `/` == join: a child ADDRESS
    assert str(child.uri) == "mem://heap/logs/app.bin"
    assert child.name == "app.bin"

    # The child is an independent buffer — writing it never touches `root`.
    child.pwrite_utf8(0, "ok")
    assert child.pread_utf8(0, 2) == "ok"
    assert bytes(root) == b""

    # Navigate back up the address.
    assert str(child.parent().uri) == "mem://heap/logs"
    assert [str(p.uri) for p in child.parents()] == ["mem://heap/logs", "mem://heap"]

    # A heap is a discovery leaf, and has no removable backing.
    assert list(child.ls()) == [] and child.children() == []
    try:
        child.rm()
    except ValueError as e:
        assert "removable backing" in str(e)
    ```

=== "Node"

    ```js
    const { Heap } = require('yggdryl').memory

    const root = new Heap()
    const child = root.join('logs/app.bin')       // JS has no `/` operator — join is the method
    console.assert(child.uri.toString() === 'mem://heap/logs/app.bin')
    console.assert(child.name === 'app.bin')

    // The child is an independent buffer — writing it never touches `root`.
    child.pwriteUtf8(0, 'ok')
    console.assert(child.preadUtf8(0, 2) === 'ok')
    console.assert(root.toBytes().length === 0)

    // Navigate back up the address.
    console.assert(child.parent().uri.toString() === 'mem://heap/logs')
    console.assert(child.parents().map(p => p.uri.toString()).join() === 'mem://heap/logs,mem://heap')

    // A heap is a discovery leaf, and has no removable backing.
    console.assert([...child.ls()].length === 0 && child.children().length === 0)
    try {
      child.rm()
    } catch (e) {
      console.assert(e.message.includes('removable backing'))
    }
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::memory::{Heap, IOBase};

    let mut child = Heap::new().join("logs/app.bin").unwrap(); // a child ADDRESS
    assert_eq!(child.uri().to_string(), "mem://heap/logs/app.bin");
    assert_eq!(child.name(), "app.bin");

    // The child is an independent buffer with its own bytes.
    child.pwrite_utf8(0, "ok");
    assert_eq!(child.pread_utf8(0, 2).unwrap(), "ok");

    // Navigate back up the address.
    assert_eq!(child.parent().unwrap().uri().to_string(), "mem://heap/logs");
    let ancestors: Vec<String> = child.parents().map(|p| p.uri().to_string()).collect();
    assert_eq!(ancestors, ["mem://heap/logs", "mem://heap"]);

    // A heap is a discovery leaf, and has no removable backing.
    assert_eq!(child.ls().unwrap().count(), 0);
    assert!(child.rm().is_err());
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

Like the address, all three are metadata — excluded from a heap's value equality (see
[One identity](#one-identity-equality-and-the-byte-codec)).

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

## One identity — equality and the byte codec

A heap *is* its stored bytes. Equality compares **only** those bytes — the cursor position, address,
headers, and mode are all transient metadata and never enter the comparison, so two heaps holding
the same bytes are equal whatever their cursors or annotations. Being a mutable buffer (like
`bytearray`), a heap is deliberately **not** hashable.

That same identity is the wire form: `serialize_bytes()` returns the stored bytes and
`deserialize_bytes(...)` is its exact inverse (cursor at `0`, default address/metadata) — so a heap
round-trips across a wire, a file, or (in Python) `pickle`. One rule holds everywhere: equal iff the
canonical bytes are equal, and the canonical bytes are simply the content.

=== "Python"

    ```python
    import copy, pickle
    from yggdryl.memory import Heap

    a, b = Heap(b"abc"), Heap(b"abc")
    assert a == b                           # equal by content
    b.rewind(); b.read_byte()               # moving the cursor changes nothing
    assert a == b

    raw = a.serialize_bytes()               # the value form: the stored bytes
    assert raw == b"abc"
    assert Heap.deserialize_bytes(raw) == a
    assert pickle.loads(pickle.dumps(a)) == a   # pickles through the same codec
    assert copy.deepcopy(a) == a
    ```

=== "Node"

    ```js
    const { Heap } = require('yggdryl').memory

    const a = new Heap(Buffer.from('abc'))
    const b = new Heap(Buffer.from('abc'))
    console.assert(a.equals(b))                 // equal by content
    b.rewind(); b.readByte()                    // moving the cursor changes nothing
    console.assert(a.equals(b))

    const raw = a.serializeBytes()              // the value form: the stored bytes
    console.assert(raw.equals(Buffer.from('abc')))
    console.assert(Heap.deserializeBytes(raw).equals(a))
    console.assert(a.copy().equals(a))
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::memory::{Heap, IOBase};
    use yggdryl_core::io::Serializable;

    let a = Heap::from_slice(b"abc");
    let mut b = Heap::from_slice(b"abc");
    assert_eq!(a, b);                       // equal by content
    b.rewind();
    let _ = b.read_byte();                  // moving the cursor changes nothing
    assert_eq!(a, b);

    let raw = a.serialize_bytes();          // the value form: the stored bytes
    assert_eq!(raw, b"abc");
    assert_eq!(Heap::deserialize_bytes(&raw).unwrap(), a);
    ```

## Zero-copy transfers

In the Rust core, `pread_into(offset, len, &mut buf)` reads into a caller-owned `Vec`, **reusing
its allocation** across a whole transfer loop — one warm buffer, zero allocations per chunk, versus
`pread_vec`'s fresh `Vec` per call. The [`heap` benchmark](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/io/memory/heap.md)
measures the difference and the `io_memory_heap_alloc` test pins the counts. The bindings return owned
byte objects (`bytes` / `Buffer`) from `pread_byte_array`, so this reuse is a Rust-core capability.
