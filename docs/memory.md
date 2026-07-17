# The memory layer

`memory` is yggdryl's **abstract byte / memory-access layer** — the traits that define
positioned and cursor access to a byte region, independent of where those bytes live. A concrete
backing implements them, so everything above reads and writes through one contract. The in-heap
backing is [`Bytes`](#bytes); a memory-mapped backing plugs in against the same traits.

## The traits

| Trait | What it adds |
|---|---|
| `IOBase` | positioned access — `pread` / `pwrite` at an explicit offset (no cursor), plus `len` |
| `IOCursor` | a moving position over an `IOBase`: `read` / `write` advance it, `seek` moves it by a signed offset relative to a [`Whence`] anchor, with bounded bulk readers (`read_exact_vec`, …) |
| `IOSlice` | a zero-copy sub-range view over an `IOBase` |
| `Whence` | the seek anchor: `Start` / `Current` / `End` |
| `IoError` | the guided failures the byte-access methods return |

## `Bytes`

The in-heap reference backing — an owned byte `Vec` with a read/write cursor, implementing all
three traits.

=== "Rust"

    ```rust
    use yggdryl_core::memory::{Bytes, IOBase, IOCursor, IOSlice};

    // Cursor writes/reads
    let mut b = Bytes::new();
    b.write_all(b"hello ").unwrap();
    b.write_all(b"world").unwrap();
    assert_eq!(b.as_slice(), b"hello world");

    b.rewind();
    let mut head = [0u8; 5];
    b.read_exact(&mut head).unwrap();
    assert_eq!(&head, b"hello");

    // Positioned access (no cursor)
    let mut buf = [0u8; 5];
    let n = b.pread(6, &mut buf);
    assert_eq!(&buf[..n], b"world");

    // A zero-copy sub-range, addressed from its own 0
    let world = b.slice(6, 5).unwrap();
    assert_eq!(world.as_slice(), b"world");
    assert!(b.slice(6, 6).is_err()); // 6 + 6 > 11
    ```

The byte-access surface is exposed only in the Rust core for now; the bindings currently expose
`version()` and the [URI family](uri.md).
