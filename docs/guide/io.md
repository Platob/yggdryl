# Byte I/O

yggdryl's core `io` module defines a small **byte-I/O trait family** and one in-memory
implementor, [`Bytes`](#bytes). The traits (Rust core) fix the signatures; `Bytes` is the
concrete value the Python and Node bindings hold.

- **`IOBase`** — random-access byte storage addressed by absolute offset: `pread` /
  `pwrite` (positioned, may be short / grow) and the *full* `pread_exact`.
- **`IOCursor`** — adds a moving **cursor**: `read` / `write` advance it, and `seek(whence,
  offset)` moves it from a [`Whence`](#whence) origin.
- **`IOSlice`** — hands out a bounded **window** of the storage as an independent value.

`Bytes` is **Arrow-backed** (`arrow-buffer`): reads and slices are **zero-copy** (a slice
shares the parent's allocation), and writes are **copy-on-write** — an in-place write reuses
the allocation, a write to a shared slice copies once, so the two never alias. Like a
`bytearray` it is mutable, and so compares by content but is not hashable.

## Whence

The seek origin, matching POSIX `lseek` — the same integer meaning as Python's `SEEK_SET` /
`SEEK_CUR` / `SEEK_END`. A signed offset is added to it; a position past the end is allowed
(a later write fills the gap), seeking before the start is an error.

| Origin | POSIX | Meaning |
| --- | --- | --- |
| `Whence.Start` | `SEEK_SET` (0) | from the start (absolute) |
| `Whence.Current` | `SEEK_CUR` (1) | from the current cursor |
| `Whence.End` | `SEEK_END` (2) | from the end |

## Positioned vs cursor read/write

`pread` / `pwrite` are **positioned** — they take an absolute offset and never touch the
cursor. `read` / `write` are their **cursor** counterparts — they act at the current position
and advance it. Reads return the bytes (short near the end); the `*_exact` variants insist on
the full count and error otherwise. A write grows the buffer, zero-filling any gap.

=== "Python"

    ```python
    from yggdryl.io import Bytes, Whence

    buf = Bytes()
    buf.write(b"hello")           # cursor write, grows the buffer
    buf.write(b" world")
    assert bytes(buf) == b"hello world"
    assert buf.position == 11

    buf.seek(Whence.Start, 6)     # move the cursor
    assert buf.read(5) == b"world"

    # Positioned access ignores the cursor.
    assert buf.pread(0, 5) == b"hello"
    buf.pwrite(0, b"HELLO")
    assert bytes(buf) == b"HELLO world"

    # A full read that can't be satisfied raises a guided ValueError.
    try:
        buf.pread_exact(6, 100)
    except ValueError as error:
        assert "end of data" in str(error)
    ```

=== "Node"

    ```js
    const { Bytes, Whence } = require('yggdryl').io

    const buf = new Bytes()
    buf.write(Buffer.from('hello'))       // cursor write, grows the buffer
    buf.write(Buffer.from(' world'))
    console.assert(buf.toBytes().toString() === 'hello world')
    console.assert(buf.position === 11)

    buf.seek(Whence.Start, 6)             // move the cursor
    console.assert(buf.read(5).toString() === 'world')

    // Positioned access ignores the cursor.
    console.assert(buf.pread(0, 5).toString() === 'hello')
    buf.pwrite(0, Buffer.from('HELLO'))
    console.assert(buf.toBytes().toString() === 'HELLO world')

    // A full read that can't be satisfied throws a guided Error.
    try {
      buf.preadExact(6, 100)
    } catch (error) {
      console.assert(/end of data/.test(error.message))
    }
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::{Bytes, IOBase, IOCursor, Whence};

    let mut buf = Bytes::new();
    buf.write(b"hello");              // cursor write, grows the buffer
    buf.write(b" world");
    assert_eq!(buf.as_slice(), b"hello world");
    assert_eq!(buf.position(), 11);

    buf.seek(Whence::Start, 6).unwrap();
    assert_eq!(buf.read_vec(5), b"world");

    // Positioned access ignores the cursor.
    assert_eq!(buf.pread_vec(0, 5), b"hello");
    buf.pwrite(0, b"HELLO");
    assert_eq!(buf.as_slice(), b"HELLO world");

    // A full read that can't be satisfied is a guided error.
    assert!(buf.pread_exact(6, &mut [0u8; 100]).is_err());
    ```

## Seeking

A seek is `whence + offset`, returning the new absolute position. Seeking **past the end** is
allowed — a read there is empty, and a write zero-fills the gap; seeking **before the start**
is a guided error.

=== "Python"

    ```python
    from yggdryl.io import Bytes, Whence

    buf = Bytes(b"hello")
    assert buf.seek(Whence.End, -2) == 3       # 2 before the end
    assert buf.read(2) == b"lo"

    assert buf.seek(Whence.End, 3) == 8        # past the end is allowed
    assert buf.read(4) == b""                   # ...a read there is empty
    buf.write(b"Z")                             # ...a write fills the gap
    assert bytes(buf) == b"hello\x00\x00\x00Z"

    try:
        buf.seek(Whence.Start, -1)             # before the start
    except ValueError as error:
        assert "before the start" in str(error)
    ```

=== "Node"

    ```js
    const { Bytes, Whence } = require('yggdryl').io

    const buf = new Bytes(Buffer.from('hello'))
    console.assert(buf.seek(Whence.End, -2) === 3)   // 2 before the end
    console.assert(buf.read(2).toString() === 'lo')

    console.assert(buf.seek(Whence.End, 3) === 8)    // past the end is allowed
    console.assert(buf.read(4).length === 0)          // ...a read there is empty
    buf.write(Buffer.from('Z'))                       // ...a write fills the gap
    console.assert(buf.toBytes().length === 9)

    try {
      buf.seek(Whence.Start, -1)                      // before the start
    } catch (error) {
      console.assert(/before the start/.test(error.message))
    }
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::{Bytes, IOCursor, Whence};

    let mut buf = Bytes::from_slice(b"hello");
    assert_eq!(buf.seek(Whence::End, -2).unwrap(), 3);   // 2 before the end
    assert_eq!(buf.read_vec(2), b"lo");

    assert_eq!(buf.seek(Whence::End, 3).unwrap(), 8);    // past the end is allowed
    assert!(buf.read_vec(4).is_empty());                  // ...a read there is empty
    buf.write(b"Z");                                      // ...a write fills the gap
    assert_eq!(buf.as_slice(), b"hello\0\0\0Z");

    assert!(buf.seek(Whence::Start, -1).is_err());        // before the start
    ```

## Zero-copy slices and copy-on-write

`slice(offset, length)` returns a bounded window addressed from its own `0`. It is
**zero-copy** — the window shares the parent's Arrow allocation (an atomic refcount bump). A
later write to *either* side copies-on-write, so a slice never sees a parent's write and a
parent never sees a slice's. An out-of-bounds window is a guided error.

=== "Python"

    ```python
    from yggdryl.io import Bytes

    parent = Bytes(b"hello world")
    window = parent.slice(6, 5)                 # zero-copy view of "world"
    assert window.to_bytes() == b"world"

    window.pwrite(0, b"WORLD")                  # copy-on-write, window only
    assert window.to_bytes() == b"WORLD"
    assert parent.to_bytes() == b"hello world"  # parent untouched

    try:
        parent.slice(6, 6)                      # 6 + 6 > 11
    except ValueError as error:
        assert "past the end" in str(error)
    ```

=== "Node"

    ```js
    const { Bytes } = require('yggdryl').io

    const parent = new Bytes(Buffer.from('hello world'))
    const window = parent.slice(6, 5)                       // zero-copy view of "world"
    console.assert(window.toBytes().toString() === 'world')

    window.pwrite(0, Buffer.from('WORLD'))                  // copy-on-write, window only
    console.assert(window.toBytes().toString() === 'WORLD')
    console.assert(parent.toBytes().toString() === 'hello world') // parent untouched

    try {
      parent.slice(6, 6)                                    // 6 + 6 > 11
    } catch (error) {
      console.assert(/past the end/.test(error.message))
    }
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::{Bytes, IOBase, IOSlice};

    let parent = Bytes::from_slice(b"hello world");
    let mut window = parent.slice(6, 5).unwrap();     // zero-copy view of "world"
    assert_eq!(window.as_slice(), b"world");

    window.pwrite(0, b"WORLD");                       // copy-on-write, window only
    assert_eq!(window.as_slice(), b"WORLD");
    assert_eq!(parent.as_slice(), b"hello world");    // parent untouched

    assert!(parent.slice(6, 6).is_err());             // 6 + 6 > 11
    ```
