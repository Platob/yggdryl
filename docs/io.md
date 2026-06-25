# Byte IO — the `Io` trait

`Io` is the **single** byte-IO abstraction the whole stack is built on. It hides
*where* bytes live so a reader works the same over an in-memory buffer, a
memory-mapped local file, or a remote object — mixing **random** access (a footer,
a column chunk) with **streamed** access (scan record batches) on one handle.

## The surface

| concern | methods |
| --- | --- |
| identity | `url()`, `stats()` |
| cursor | `seek(offset, whence)`, `stream_position()`, `stream_len()` |
| streamed | `read(buf)`, `write(bytes)`, `read_to_end(out)`, `read_exact(buf)`, `write_all(bytes)`, `flush()` |
| random | `pread(buf, offset, whence)`, `pwrite(bytes, offset, whence)` |
| zero-copy | `as_slice()`, `copy_to(dst)` |
| storage | `capacity()`, `reserve_capacity(n)`, `truncate(n)` |
| metadata | `media_type()` *(feature `media`)*, `json()` *(feature `json`)* |

`Whence::Current` uses and advances the cursor (a *streamed* read/write);
`Whence::Start` / `Whence::End` are purely positional and leave the cursor put —
ideal for reading a footer without disturbing a sequential scan. A memory-resident
backend overrides `as_slice()` and gets zero-copy `pread` / `copy_to` /
`read_to_end` for free; a streamed backend just overrides `read`.

Streaming transfers move data **1 MiB at a time** (`STREAM_CHUNK`), tuned for the
large columnar payloads (Parquet / CSV / JSON) this stack underpins.

## Backends

- **`BytesIO`** — an in-memory buffer with a cursor (Python's `io.BytesIO`).
- **`LocalPath`** — a filesystem path; stats up front, memory-maps lazily on first
  read for zero-copy access; `write` auto-creates parent directories.
- **`HttpStream`** — a seekable remote body (see [HTTP](http.md)).
- *(downstream)* cloud object stores implement `RemotePath: Io`.

```rust
use yggdryl_io::{BytesIO, Io, Whence};

let mut io = BytesIO::from_bytes(b"0123456789".to_vec());
let mut tail = [0u8; 4];
io.pread(&mut tail, -4, Whence::End)?;   // positional: cursor untouched
assert_eq!(&tail, b"6789");
let mut head = [0u8; 4];
io.read(&mut head)?;                      // streamed: from the cursor
assert_eq!(&head, b"0123");
```

## Codecs

`Codec<T>` reads / writes / streams typed values over any `Io` handle; `Frames` is
the reference length-delimited codec. Byte-stream **compression** is a separate
layer — see [Compression](compression.md).
