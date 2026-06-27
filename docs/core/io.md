# Byte IO — the `Io` handle

`Io` is the **single** byte-IO abstraction the whole stack is built on. It hides
*where* bytes live, so a reader works the same over an in-memory buffer, a
memory-mapped local file, or a remote HTTP body — mixing **random** access (read a
footer, a column chunk) with **streamed** access (scan record batches) on one
handle. A [`BytesIO`](#backends) holds bytes in memory, a [`LocalPath`](#backends)
maps a file lazily, and an [HTTP response is itself an `Io`](../http/request-response.md).

## The surface

Every handle knows its `url()` and `stats()`, carries a cursor moved with
`seek`, and does **both** random and streamed access:

| concern | methods |
| --- | --- |
| identity | `url()`, `stats()` |
| cursor | `seek(offset, whence)`, `stream_position()`, `stream_len()` |
| streamed | `read(buf)`, `write(bytes)`, `read_to_end(out)`, `read_exact(buf)`, `write_all(bytes)`, `flush()` |
| random | `pread(buf, offset, whence)`, `pwrite(bytes, offset, whence)` |
| zero-copy | `as_slice()`, `copy_to(dst)` |
| storage | `capacity()`, `reserve_capacity(n)`, `truncate(n)` |
| lifecycle | `open(mode, stream)`, `close()` |
| metadata | `media_type()` *(feature `media`)*, `json()` *(feature `json`)* |
| stats cache | `cached_stats()` *(get)*, `set_stats(stats)` *(set)* |

`Whence::Current` (`1`) uses and **advances** the cursor — a streamed read/write;
`Whence::Start` (`0`) and `Whence::End` (`2`) are purely positional and **leave
the cursor put** — exactly what you want to read a footer without disturbing a
sequential scan. The convenience `read` / `write` are the cursor-relative case;
`read_to_end` / `copy_to` drain from the cursor.

!!! tip "Zero-copy, for free"
    A memory-resident backend overrides `as_slice()` and gets zero-copy `pread` /
    `copy_to` / `read_to_end` / `json()` for free; a streamed backend (an HTTP
    body, a decoder) just overrides `read`. Streaming transfers move data **1 MiB
    at a time** (`STREAM_CHUNK`), tuned for the large columnar payloads (Parquet /
    CSV / JSON) this stack underpins.

## Backends

- **`BytesIO`** — an in-memory buffer with a cursor, modelled on Python's
  `io.BytesIO`: read / write / seek, and `getvalue()` borrows the whole buffer.
- **`LocalPath`** — a filesystem path: it stats up front (so `url` / `stats` /
  `exists` are ready immediately), memory-maps lazily on the first read for
  zero-copy access, and its `write` auto-creates missing parent directories.
- **`HttpStream`** — a seekable remote body (see [Streaming body](../http/stream.md)).
- *(downstream)* cloud object stores implement `RemotePath: Io`.

A read with a `size` of `None` (or omitted / negative) reads all remaining bytes.
The `stream` flag governs whether the Python-style `read` / `read_line` / `write`
advance the cursor — on by default; turn it off for repeated random reads from a
fixed position.

=== "Python"

    ```python
    import yggdryl

    io = yggdryl.BytesIO(b"hello world")
    assert io.read(5) == b"hello"     # streamed: advances the cursor
    assert io.tell() == 5
    io.seek(6)                        # whence defaults to start (0)
    assert io.read() == b"world"      # no size reads the rest
    assert io.url.scheme == "mem"     # every Io has a URL
    ```

=== "Node"

    ```javascript
    const { BytesIO } = require("yggdryl");

    const io = new BytesIO(Buffer.from("hello world"));
    io.read(5);            // Buffer "hello" — advances the cursor
    io.tell();             // 5
    io.seek(6);            // whence defaults to start (0)
    io.read();             // Buffer "world" — no size reads the rest
    io.url.scheme;         // "mem"
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{BytesIO, Io, Whence};

    let mut io = BytesIO::from_bytes(b"hello world".to_vec());
    let mut head = [0u8; 5];
    Io::read(&mut io, &mut head)?;              // streamed: from the cursor
    assert_eq!(&head, b"hello");
    io.seek(6, Whence::Start)?;
    assert_eq!(Io::stream_position(&io), 6);
    assert_eq!(io.url().scheme(), "mem");      // every Io has a URL
    ```

## Build a `BytesIO` from a string

A `BytesIO` can be built straight from a string: if it **names an existing file**
its bytes are read in, otherwise the string is taken **verbatim as UTF-8** content.
The constructors accept a string or raw bytes; `from_str` is the explicit named
form (Rust only constructs from a string this way — `from_bytes` is the bytes form).

=== "Python"

    ```python
    import yggdryl

    yggdryl.BytesIO("hello").getvalue()             # b"hello" (UTF-8 of the text)
    yggdryl.BytesIO.from_str("data.csv").getvalue() # the file's bytes, if it exists
    yggdryl.BytesIO(b"\x00\x01").getvalue()         # raw bytes, unchanged
    ```

=== "Node"

    ```javascript
    const { BytesIO } = require("yggdryl");

    new BytesIO("hello").getValue();             // Buffer "hello" (UTF-8 of the text)
    BytesIO.fromStr("data.csv").getValue();      // the file's bytes, if it exists
    new BytesIO(Buffer.from([0, 1])).getValue(); // raw bytes, unchanged
    ```

=== "Rust"

    ```rust
    use yggdryl_core::BytesIO;

    assert_eq!(BytesIO::from_str("hello").getvalue(), b"hello"); // UTF-8 of the text
    // BytesIO::from_str("data.csv") reads the file's bytes when it exists.
    ```

## Cached metadata — stats & media type

`stats()` discovers metadata, and the expensive parts (a media-type sniff, a
remote `HEAD`) are **memoized** so they happen at most once. Two accessors expose
that cache: `cached_stats()` peeks it (the *get* side — `null` when nothing is
cached) and `set_stats(stats)` installs it (the *set* side), so a caller who
already knows a handle's content type or media type can attach it and skip
rediscovery. For an in-memory `BytesIO` the **live byte count always wins** over a
cached size, so writes are still reflected.

A `BytesIO` also **caches its inferred media type**: the magic bytes are sniffed
once on the first `media_type` access and reused. When you already know the type,
pass it in at construction to skip the sniff entirely.

=== "Python"

    ```python
    import yggdryl

    io = yggdryl.BytesIO(bytes([0x1F, 0x8B, 0x08, 0x00]))  # gzip magic
    io.media_type.first.mime         # "application/gzip" — sniffed once, cached

    # Put the type in instead of inferring it.
    csv = yggdryl.MediaType.from_str("text/csv")
    typed = yggdryl.BytesIO(b"a,b,c\n1,2,3\n", media_type=csv)

    # Attach / peek cached metadata; the live size is never frozen.
    io.set_stats(yggdryl.IoStats(content_type="application/json"))
    io.cached_stats().content_type   # "application/json"
    io.stats().size                  # live byte count
    ```

=== "Node"

    ```javascript
    const { BytesIO, MediaType, IoStats } = require("yggdryl");

    const io = new BytesIO(Buffer.from([0x1f, 0x8b, 0x08, 0x00])); // gzip magic
    io.mediaType.first.mime;         // "application/gzip" — sniffed once, cached

    // Put the type in instead of inferring it (3rd constructor arg).
    const csv = MediaType.fromStr("text/csv");
    const typed = new BytesIO(Buffer.from("a,b,c\n1,2,3\n"), undefined, csv);

    // Attach / peek cached metadata; the live size is never frozen.
    io.setStats(new IoStats(0, "file", undefined, "application/json"));
    io.cachedStats().contentType;    // "application/json"
    io.stats().size;                 // live byte count
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{BytesIO, Io, IoStats, MediaType, MimeType};

    let io = BytesIO::from_bytes(vec![0x1f, 0x8b, 0x08, 0x00]); // gzip magic
    assert_eq!(io.media_type().unwrap().first(), Some(&MimeType::Gzip));

    // Put the type in instead of inferring it.
    let csv = MediaType::from_str("text/csv")?;
    let _typed = BytesIO::from_bytes(b"a,b,c\n".to_vec()).with_media_type(csv);

    // Attach / peek cached metadata (cached_stats = get, set_stats = set).
    let mut io = BytesIO::from_bytes(b"abc".to_vec());
    io.set_stats(IoStats::new(0).with_content_type("application/json"));
    assert_eq!(io.cached_stats().unwrap().content_type(), Some("application/json"));
    assert_eq!(io.stats().unwrap().size(), 3); // live byte count, never frozen
    ```

## Random access — read a footer with `pread`

`pread` reads at an offset without moving the cursor, so a footer or a column
chunk costs nothing to a sequential scan in progress. With `Whence::End` you read
the **last** N bytes directly — over an `HttpStream` that is a single `Range`
request, never a full download.

=== "Python"

    ```python
    import yggdryl

    io = yggdryl.BytesIO(b"0123456789")
    io.seek(4)
    # Positional (whence=0, the default): read at offset 6, cursor stays at 4.
    assert io.pread(2, 6) == b"67"
    assert io.tell() == 4
    # Cursor-relative (whence=1): reads from the cursor and advances it.
    assert io.pread(2, 0, 1) == b"45"
    assert io.tell() == 6
    ```

=== "Node"

    ```javascript
    const { BytesIO } = require("yggdryl");

    const io = new BytesIO(Buffer.from("0123456789"));
    io.seek(4);
    // Positional (whence omitted = start): read at offset 6, cursor stays at 4.
    io.pread(2, 6);   // Buffer "67"
    io.tell();        // 4
    // Cursor-relative (whence=1): reads from the cursor and advances it.
    io.pread(2, 0, 1); // Buffer "45"
    io.tell();        // 6
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{BytesIO, Io, Whence};

    let mut io = BytesIO::from_bytes(b"0123456789".to_vec());
    io.seek(2, Whence::Start)?;
    let mut footer = [0u8; 4];
    io.pread(&mut footer, -4, Whence::End)?;   // last 4 bytes, cursor untouched
    assert_eq!(&footer, b"6789");
    assert_eq!(Io::stream_position(&io), 2);   // sequential scan undisturbed
    ```

## Write, seek and storage

A non-empty write past the end zero-fills the gap; an empty write is a no-op (as
in Python). `truncate(size)` grows (zero-fill) or shrinks the buffer, and
`with_capacity` / `reserve_capacity` preallocate for write-heavy use.

=== "Python"

    ```python
    import yggdryl

    io = yggdryl.BytesIO.with_capacity(64)
    io.write(b"abc")
    assert io.truncate(5) == 5            # grow, zero-fill
    assert io.getvalue() == b"abc\x00\x00"
    io.seek(1)
    io.write(b"XY")                       # overwrite in place
    assert io.getvalue()[:3] == b"aXY"
    ```

=== "Node"

    ```javascript
    const { BytesIO } = require("yggdryl");

    const io = BytesIO.withCapacity(64);
    io.write(Buffer.from("abc"));
    io.truncate(5);                       // 5 — grow, zero-fill
    io.getValue();                        // <Buffer 61 62 63 00 00>
    io.seek(1);
    io.write(Buffer.from("XY"));          // overwrite in place
    io.getValue().subarray(0, 3);         // Buffer "aXY"
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{BytesIO, Io, Whence};

    let mut io = BytesIO::with_capacity(64);
    io.write_all(b"abc")?;
    Io::truncate(&mut io, 5)?;            // grow, zero-fill
    assert_eq!(io.getvalue(), b"abc\0\0");
    io.seek(1, Whence::Start)?;
    io.write_all(b"XY")?;                 // overwrite in place
    assert_eq!(&io.getvalue()[..3], b"aXY");
    ```

## The `open` factory — a handle for any location

The factory picks the backend by URL scheme: a bare path or `file://` opens a
`LocalPath`; `http` / `https` send a request and hand back the response (itself an
`Io`). The bindings surface the local branch as a module-level `open(location)`;
remote schemes go through the [HTTP session](../http/session.md).

=== "Python"

    ```python
    import yggdryl

    handle = yggdryl.open("/etc/hostname")   # a LocalPath, statted up front
    assert handle.exists()
    data = handle.read()                     # mmaps lazily on first read
    ```

=== "Node"

    ```javascript
    const { open } = require("yggdryl");

    const handle = open("/etc/hostname");    // a LocalPath, statted up front
    handle.exists();                         // true
    const data = handle.read();              // mmaps lazily on first read
    ```

=== "Rust"

    ```rust
    use yggdryl_core::{from_str, Io};

    // A bare path / file:// opens a LocalPath; http/https a sent HttpResponse.
    let mut handle = from_str("/etc/hostname")?;
    let mut data = Vec::new();
    handle.read_to_end(&mut data)?;          // mmaps lazily, zero-copy tail
    ```

!!! note "Derived handles"
    `open(mode, stream)` on a `BytesIO` or `LocalPath` returns a new in-memory
    handle and records the original as its `parent()`: `"w"` truncates to empty,
    `"a"` positions the cursor at the end, `"r"` / `"r+"` copy the bytes with the
    cursor at the start. `close()` is idempotent — memory and mmap backends free
    their storage on drop.

## Codecs — typed values over bytes

`Codec<T>` reads, writes and streams typed values over **any** `Io` handle, so the
same coder works against a `BytesIO`, a `LocalPath`, or a cloud path alike.
`Frames` is the reference codec: each value is a big-endian `u32` length prefix
followed by that many payload bytes, so frames pack back to back and `stream`
reads them out until the source drains. This is a Rust-core concern; byte-stream
**compression** is a separate layer — see [Compression](compression.md).

```rust
use yggdryl_core::{BytesIO, Codec, Frames, Io, Whence};

let mut io = BytesIO::new();
Frames.write(&mut io, &b"one".to_vec())?;
Frames.write(&mut io, &b"two".to_vec())?;
io.seek(0, Whence::Start)?;
let items: Vec<Vec<u8>> = Frames.stream(io).collect::<Result<_, _>>()?;
assert_eq!(items, vec![b"one".to_vec(), b"two".to_vec()]);
```

## Next

- [Compression](compression.md) — streamed codecs that compose over any `Io`.
- [Media types](media.md) — what `media_type()` / `mime_type()` return.
- [Streaming body](../http/stream.md) — the seekable remote `HttpStream`.
- [Request & Response](../http/request-response.md) — the response *is* an `Io`.
