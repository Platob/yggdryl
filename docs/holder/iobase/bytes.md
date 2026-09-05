# Bytes

This page owns positional bytes over any handle: `pread`/`pwrite`, streams, laziness, kinds, cursors, media type, coding, open/close, clear/remove.

## Contract

| Key | Rule |
| --- | --- |
| Required | `pread` and `pwrite`; every other byte method derives from them |
| Invariants | `pread` short only at end of value; `pwrite` grows, zero-fills gaps; `size <= capacity`; `reserve` moves `capacity` only |
| Lazy | Constructing touches nothing; reads of an absent resource yield nothing; writes, `truncate`, `reserve` create |
| Cached | Open caches, closed fetches; no ordinary read fills the cache |
| Media type | Computed on ask, re-derived when bytes change; a declared type wins; `codec` is its last coding |
| Errors | Bindings refuse `compress_into` to a target with no coding; `remove` refuses a container with children unless `recursive` |
| Bindings | `IOBase`; read is `read_range_bytes`/`readRangeBytes`, write is `pwrite`; `IOCursor`, `ByteStream`, wrappers are Rust only |

## Use

Explicit offsets mean two readers never interfere and a footer-first container reads its index without seeking.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;

    let mut handle = Buffer::new();
    handle.pwrite(0, b"symbol,price\n")?;
    handle.pwrite(13, b"AAPL,1\n")?;
    assert_eq!(handle.size(), 20);

    // Two reads at different offsets, in any order: there is no shared cursor.
    let mut tail = [0_u8; 4];
    handle.pread(13, &mut tail)?;
    let mut head = [0_u8; 6];
    handle.pread(0, &mut head)?;
    assert_eq!(&head, b"symbol");
    assert_eq!(&tail, b"AAPL");
    ```

=== "Python"

    ```python
    from yggdryl import IOBase

    handle = IOBase.from_bytes()
    handle.pwrite(0, b"symbol,price\n")
    handle.pwrite(13, b"AAPL,1\n")
    assert handle.size == 20

    # Two reads at different offsets, in any order: there is no shared cursor.
    assert handle.read_range_bytes(13, 4) == b"AAPL"
    assert handle.read_range_bytes(0, 6) == b"symbol"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase } = require('yggdryl')

    const handle = IOBase.fromBytes()
    handle.pwrite(0, Buffer.from('symbol,price\n'))
    handle.pwrite(13, Buffer.from('AAPL,1\n'))
    assert.equal(handle.size, 20)

    // Two reads at different offsets, in any order: there is no shared cursor.
    assert.equal(handle.readRangeBytes(13, 4).toString(), 'AAPL')
    assert.equal(handle.readRangeBytes(0, 6).toString(), 'symbol')
    ```

## Streamed bytes

`pstream_bytes(position, batch_size)` yields owned arrays of at most `batch_size` bytes from a decoded position; construction is lazy and never asks for `size`.

```rust
use yggdryl::{IOBase, IOCursor};
use yggdryl::holder::Buffer;

let handle = Buffer::from_bytes(b"0123456789".to_vec());
let chunks = handle
    .pstream_bytes(2, 3)?
    .collect::<yggdryl::Result<Vec<_>>>()?;
assert_eq!(chunks, [b"234".to_vec(), b"567".to_vec(), b"89".to_vec()]);

// The cursor form starts at `tell` and advances only when bytes are yielded.
let mut cursor = handle.cursor_at(1);
let first = cursor.stream_bytes(2)?.next().transpose()?.unwrap();
assert_eq!(first, b"12");
assert_eq!(cursor.tell(), 3);
```

The bindings expose the same lazy iterator with a 65,536-byte default batch.

=== "Python"

    ```python
    from yggdryl import IOBase

    handle = IOBase.from_bytes(b"0123456789")
    assert list(handle.pstream_bytes(2, 3)) == [b"234", b"567", b"89"]

    cursor = handle.cursor(1)
    stream = cursor.stream_bytes(2)
    assert next(stream) == b"12"
    assert cursor.tell() == 3
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase } = require('yggdryl')

    const handle = IOBase.fromBytes(Buffer.from('0123456789'))
    assert.deepEqual(
      [...handle.pstreamBytes(2, 3)].map((part) => part.toString()),
      ['234', '567', '89'],
    )

    const cursor = handle.cursor(1)
    const first = cursor.streamBytes(2).next()
    assert.equal(first.value.toString(), '12')
    assert.equal(cursor.tell(), 3)
    ```

`ByteStream` implements `std::io::Read`; a coded handle decodes straight from the encoded source and retains no decoded pages.

## Built from what you already hold

Python only.

`IOBase(...)` accepts an open file or stream: a named file captures its location, a nameless stream its content.

```python
import io
import pathlib
import tempfile

from yggdryl import IOBase

# An open file names its own location, so the handle addresses the path.
target = pathlib.Path(tempfile.mkdtemp()) / "quotes.json"
target.write_bytes(b"{}")
with open(target, "rb") as stream:
    handle = IOBase(stream)
assert handle.name == "quotes.json"

# A nameless stream holds only content, so the content is what is taken.
buffered = IOBase(io.BytesIO(b'{"symbol": "AAPL"}'))
buffered.media_type = "application/json"
assert buffered.read_text() == '{"symbol": "AAPL"}'
```

## Laziness

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::{IOKind};
    use yggdryl::holder::local;

    let path = local::Folder::temporary()?.path()?.join("yggdryl-docs-io-lazy.csv");
    let _ = std::fs::remove_file(&path);

    // Constructing touches nothing: no file is created, opened, or mapped.
    let mut handle = local::File::new(&path)?;
    assert!(!handle.exists());

    // Reading something absent yields nothing rather than failing.
    assert_eq!(handle.size(), 0);
    let mut probe = [0_u8; 8];
    assert_eq!(handle.pread(0, &mut probe)?, 0);
    assert_eq!(handle.kind(), IOKind::Unknown);

    // Writing creates the resource, and any parent it needs.
    handle.write_all_bytes(b"symbol,price\n")?;
    assert_eq!(handle.kind(), IOKind::File);
    assert_eq!(handle.read_all_bytes()?, b"symbol,price\n");

    handle.close()?;
    // Teardown through the abstraction: absence is a no-op success.
    handle.remove(false)?;
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp())

    # Constructing touches nothing: no file is created, opened, or mapped.
    handle = IOBase(root / "nested" / "lazy.csv")
    assert not handle.exists()

    # Reading something absent yields nothing rather than raising.
    assert handle.size == 0
    assert handle.read_bytes() == b""

    # Writing creates the resource, and any parent it needs.
    handle.write_text("symbol,price\n")
    assert handle.is_file()
    assert handle.read_text() == "symbol,price\n"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))

    // Constructing touches nothing: no file is created, opened, or mapped.
    const handle = new IOBase(path.join(root, 'nested', 'lazy.csv'))
    assert.ok(!handle.exists())

    // Reading something absent yields nothing rather than throwing.
    assert.equal(handle.size, 0)
    assert.equal(handle.readBytes().length, 0)

    // Writing creates the resource, and any parent it needs.
    handle.writeText('symbol,price\n')
    assert.ok(handle.isFile())
    assert.equal(handle.readText(), 'symbol,price\n')

    fs.rmSync(root, { recursive: true, force: true })
    ```

Non-existence is resolved at the operation, so probing a location needs no separate existence check.

## Kinds

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::{IOKind};
    use yggdryl::holder::local;

    assert_eq!(Buffer::new().kind(), IOKind::Memory);
    assert!(IOKind::Memory.is_leaf());

    let folder = local::Folder::temporary()?;
    assert_eq!(folder.kind(), IOKind::Directory);
    assert!(folder.is_container());

    // Nothing is there, so nothing has decided; a write settles it.
    let absent = local::File::new(local::Folder::temporary()?.path()?.join("yggdryl-docs-io-absent.bin"))?;
    assert_eq!(absent.kind(), IOKind::Unknown);
    assert!(!absent.kind().is_known());
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    folder = IOBase(pathlib.Path(tempfile.mkdtemp()))
    assert folder.is_dir()
    assert not folder.is_file()

    # Nothing is there, so nothing has decided; a write settles it.
    leaf = folder / "ticks.csv"
    assert not leaf.exists()
    leaf.write_text("symbol\n")
    assert leaf.is_file()
    assert not leaf.is_dir()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const folder = new IOBase(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')))
    assert.ok(folder.isDir())
    assert.ok(!folder.isFile())

    // Nothing is there, so nothing has decided; a write settles it.
    const leaf = folder.joinpath('ticks.csv')
    assert.ok(!leaf.exists())
    leaf.writeText('symbol\n')
    assert.ok(leaf.isFile())
    assert.ok(!leaf.isDir())

    fs.rmSync(folder.intoPath(), { recursive: true, force: true })
    ```

| `IOKind` | Meaning |
| --- | --- |
| `Memory` | bytes with no location |
| `File` | leaf holding bytes |
| `Directory` | container of resources |
| `Unknown` | location that does not exist yet |
| `Table`, `Namespace`, `Catalog` | containers a table format adds |

Callers ask `is_container`, `is_leaf`, `is_known`; the bindings expose `exists`, `is_dir`, `is_file` instead.

## Bytes or rows

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::{MimeType};
    use yggdryl::holder::local;

    // A leaf answers from its representation, and the two are complements.
    let mut notes = Buffer::new();
    notes.set_media_type(MimeType::PLAIN_TEXT.into());
    assert!(notes.is_atomic());
    assert!(!notes.is_tabular());

    // The name is enough: nothing has been written to this location yet.
    let trades = local::File::new(local::Folder::temporary()?.path()?.join("yggdryl-docs-shape.parquet"))?;
    assert!(trades.is_tabular());
    assert!(!trades.is_atomic());

    // A container is neither one whole byte value nor - with nothing under
    // it - a table.
    let folder = local::Folder::temporary()?;
    assert!(!folder.is_atomic());
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    root = IOBase(pathlib.Path(tempfile.mkdtemp()))

    notes = root / "notes.txt"
    assert notes.is_atomic()
    assert not notes.is_tabular()

    # The name is enough: nothing has been written to this location yet.
    trades = root / "trades.parquet"
    assert trades.is_tabular()
    assert not trades.is_atomic()

    assert not root.is_atomic()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = new IOBase(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')))

    const notes = root.joinpath('notes.txt')
    assert.ok(notes.isAtomic())
    assert.ok(!notes.isTabular())

    // The name is enough: nothing has been written to this location yet.
    const trades = root.joinpath('trades.parquet')
    assert.ok(trades.isTabular())
    assert.ok(!trades.isAtomic())

    assert.ok(!root.isAtomic())

    fs.rmSync(root.intoPath(), { recursive: true, force: true })
    ```

*Atomic* is the byte surface, *tabular* the record surface on [Records](records.md); wherever bytes are held the two are complements. A container holding neither answers `false` to both; only a plain `Directory` is probed, stopping at the first settling leaf.

## Cursors

A cursor makes a position explicit; two cursors over one resource advance independently.

=== "Rust"

    ```rust
    use std::io::Read;

    use yggdryl::{IOBase, IOCursor};
    use yggdryl::holder::Buffer;

    let mut cursor = Buffer::new().cursor();
    cursor.write_next(b"symbol,price\n")?;
    assert_eq!(cursor.tell(), 13);

    cursor.seek_to(7);
    let mut word = [0_u8; 5];
    cursor.read_exact(&mut word)?; // std::io::Read rides the same position
    assert_eq!(&word, b"price");
    ```

=== "Python"

    ```python
    from yggdryl import IOBase

    handle = IOBase.from_bytes()
    cursor = handle.cursor()
    cursor.write(b"symbol,price\n")

    # The write landed on the handle itself; the position is the cursor's.
    assert handle.read_bytes() == b"symbol,price\n"
    assert cursor.seek(-6, 2) == 7
    assert cursor.read(5) == b"price"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase } = require('yggdryl')

    const handle = IOBase.fromBytes()
    const cursor = handle.cursor()
    cursor.write(Buffer.from('symbol,price\n'))

    assert.equal(handle.readBytes().toString(), 'symbol,price\n')
    cursor.seek(7)
    assert.equal(cursor.read(5).toString(), 'price')
    ```

| Language | Surface |
| --- | --- |
| Rust | `IOCursor`: `tell`, `seek_to`, `seek`, `read_next`, `write_next`; `Cursor<H>` implements `Read`, `Write`, `Seek` |
| Python | shares the handle; `seek(offset, whence)`, `read(size=-1)` |
| JavaScript | shares the handle; `seek`, `tell`, `position` |

## Media type

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::MimeType;

    // Nothing names an in-memory buffer, so its type comes from its bytes.
    let mut handle = Buffer::from_bytes(br#"{"symbol":"AAPL"}"#.to_vec());
    assert_eq!(handle.media_type().base(), &MimeType::JSON);

    // It is re-derived after the content changes.
    handle.write_all_bytes(b"PAR1payload")?;
    assert_eq!(handle.media_type().base(), &MimeType::PARQUET);
    ```

=== "Python"

    ```python
    from yggdryl import IOBase

    # Nothing names an in-memory buffer, so its type comes from its bytes.
    handle = IOBase.from_bytes(b'{"symbol":"AAPL"}')
    assert str(handle.media_type.base) == "application/json"

    # It is re-derived after the content changes.
    handle.write_bytes(b"PAR1payload")
    assert str(handle.media_type.base) == "application/vnd.apache.parquet"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase, MimeType } = require('yggdryl')

    // Nothing names an in-memory buffer, so its type comes from its bytes.
    const handle = IOBase.fromBytes(Buffer.from('{"symbol":"AAPL"}'))
    assert.ok(handle.mediaType.base.equals(MimeType.JSON))

    // It is re-derived after the content changes.
    handle.writeBytes(Buffer.from('PAR1payload'))
    assert.ok(handle.mediaType.base.equals(MimeType.PARQUET))
    ```

`media_type` answers what the bytes are and which content codings sit on top.

```rust
use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::{Codec, MimeType, Url};

// A declared type wins, and the codings it carries are what `codec` reports.
let named = Buffer::new().with_media_type(Url::from_str("file:///trades.json.gz")?.media_type());
assert_eq!(named.media_type().base(), &MimeType::JSON);
assert_eq!(named.codec(), Codec::Gzip);
```

`codec` is the last coding in the media type, so compression is never a separate argument; `set_media_type` declares what content cannot identify.

## Adding and removing a coding

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::{Codec, Url};

    let mut plain = Buffer::new().with_media_type(Url::from_str("file:///rows.json")?.media_type());
    plain.write_all_bytes(br#"{"symbol":"AAPL"}"#)?;
    // Nothing wraps these bytes, so there is nothing to undo.
    assert_eq!(plain.codec(), Codec::Identity);

    let mut encoded =
        Buffer::new().with_media_type(Url::from_str("file:///rows.json.gz")?.media_type());
    assert_eq!(encoded.codec(), Codec::Gzip);

    // The coding is an argument here, and the target's name is one place to read it from.
    let codec = encoded.codec();
    plain.compress_into(&mut encoded, codec)?;
    assert_eq!(&encoded.read_all_bytes()?[..2], b"\x1f\x8b");

    let mut decoded = Buffer::new();
    encoded.decompress_into(&mut decoded)?;
    assert_eq!(decoded.read_all_bytes()?, plain.read_all_bytes()?);
    assert_eq!(decoded.codec(), Codec::Identity);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp())
    plain = IOBase(root / "rows.json")
    plain.write_bytes(b'{"symbol":"AAPL"}')

    # Nothing wraps these bytes, so there is nothing to undo.
    assert plain.codec is None

    encoded = IOBase(root / "rows.json.gz")
    assert encoded.codec == "gzip"

    # The target's name already said gzip, so nothing here repeats it.
    assert plain.compress_into(encoded) == encoded.size
    assert encoded.read_bytes()[:2] == b"\x1f\x8b"

    decoded = IOBase(root / "roundtrip.json")
    assert encoded.decompress_into(decoded) == 17
    assert decoded.read_bytes() == plain.read_bytes()
    assert decoded.codec is None

    # A target declaring no coding is refused rather than copied unchanged.
    reason = None
    try:
        plain.compress_into(IOBase(root / "copy.json"))
    except ValueError as error:
        reason = str(error)
    assert "expected a target declaring a content coding" in reason
    assert not (root / "copy.json").exists()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const plain = new IOBase(path.join(root, 'rows.json'))
    plain.writeBytes(Buffer.from('{"symbol":"AAPL"}'))

    // Nothing wraps these bytes, so there is nothing to undo.
    assert.equal(plain.codec, null)

    const encoded = new IOBase(path.join(root, 'rows.json.gz'))
    assert.equal(encoded.codec, 'gzip')

    // The target's name already said gzip, so nothing here repeats it.
    assert.equal(plain.compressInto(encoded), encoded.size)
    assert.deepEqual([...encoded.readBytes().subarray(0, 2)], [0x1f, 0x8b])

    const decoded = new IOBase(path.join(root, 'roundtrip.json'))
    assert.equal(encoded.decompressInto(decoded), 17)
    assert.equal(decoded.readText(), '{"symbol":"AAPL"}')
    assert.equal(decoded.codec, null)

    // An in-memory target has no name to declare a coding, so this one is named.
    const memory = IOBase.fromBytes()
    assert.ok(plain.compressInto(memory, 'zstd') > 0)
    assert.equal(memory.codec, 'zstd')

    // A target declaring no coding is refused rather than copied unchanged.
    assert.throws(
      () => plain.compressInto(new IOBase(path.join(root, 'copy.json'))),
      /expected a target declaring a content coding/,
    )
    assert.equal(fs.existsSync(path.join(root, 'copy.json')), false)

    fs.rmSync(root, { recursive: true, force: true })
    ```

Both calls move every byte into another handle and add or remove a coding, recorded in the target's media type.

| Call | Coding used |
| --- | --- |
| `compress_into(target)` | the target's declared coding; an explicit codec overrides |
| `decompress_into(target)` | the source's declared coding |
| Rust `compress_into(target, codec)` | always the argument |
| `level` | the shared 0-9 scale |

Readers already decode through a name's codings; see [gzip](../../coding/gzip.md), [zlib](../../coding/zlib.md), and [zstd](../../coding/zstd.md).

## Open and close

A handle works without `open`; calling it moves materialization to a known point and keeps cached state across many small operations. Python binds the pair to `with`; JavaScript adds `Symbol.dispose`.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::coding::Coded;
    use yggdryl::Codec;

    let mut handle = Coded::wrap(Buffer::new(), Codec::Zstd);
    assert!(!handle.opened());

    handle.open()?;
    assert!(handle.opened());
    handle.write_all_bytes(b"symbol,price\n")?;

    // Closing publishes the pending write and releases the cache.
    handle.close()?;
    assert!(!handle.opened());

    // The handle stays usable; the next read re-materializes.
    assert_eq!(handle.read_all_bytes()?, b"symbol,price\n");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    path = pathlib.Path(tempfile.mkdtemp()) / "trades.csv"

    # `with` is the scoped pair: `__enter__` opens and `__exit__` closes.
    with IOBase(path) as handle:
        handle.write_text("symbol,price\n")
        assert handle.opened
        assert not handle.closed

    # Closing published the bytes at their exact length, which is what another
    # reader needs; the handle stays usable and simply re-materializes.
    assert path.stat().st_size == 13
    assert IOBase(path).read_text() == "symbol,price\n"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const target = path.join(root, 'value.bin')
    fs.writeFileSync(target, 'symbol,price\n')

    const handle = new IOBase(target)
    handle.open()
    assert.equal(handle.opened(), true)
    handle.close()
    assert.equal(handle.closed(), true)

    fs.rmSync(root, { recursive: true, force: true })
    ```

A closed handle re-derives metadata on every ask; an open one holds what `open` cached until `close`.

| Implementation | `open` caches |
| --- | --- |
| [`Buffer`](../backends/buffer.md) | nothing; `opened` stays `false` |
| [`local::File`](../backends/local.md) | descriptor and memory mapping |
| [`Coded`](../../coding/index.md) | the decoded value |
| [IPC](../../media/ipc.md) | schema and dimensions |
| [Parquet](../../media/parquet.md) | the footer |
| [Avro](../../media/avro.md) | header and block metadata |
| [Text](../../media/text.md) | resolved field, coding plan, dimensions |

Python only.

```python
import pathlib
import tempfile

import pyarrow as pa

from yggdryl import IOBase

target = pathlib.Path(tempfile.mkdtemp()) / "lake" / "trades.parquet"
IOBase(target).overwrite_arrow_table(pa.table({"id": [1, 2], "venue": ["XNAS", "XNYS"]}))

# Metadata-heavy work belongs inside the scope: the schema probe, the
# per-batch reads, and the size checks all reuse what `open` cached, and
# `close` releases it at a known point.
rows = 0
with IOBase(target) as handle:
    field = handle.read_arrow_field()
    for batch in handle.read_arrow_reader():
        rows += batch.num_rows
assert rows == 2

# Outside a scope the same calls still work - each one just fetches fresh,
# which is exactly right for a resource another writer may be changing.
assert IOBase(target).read_arrow_field() == field
```

## Clearing and removing

`clear` empties and keeps the resource; `remove` deletes it, issuing the delete without a probe and treating not-found as success.

| Call | Leaf | Container | [Iceberg](../../media/iceberg/index.md) `Table` |
| --- | --- | --- | --- |
| `clear` | size `0` | loses every child recursively | one snapshot with no data files; schema, properties, history stay |
| `remove` | deleted | deleted; refused by name while children remain, unless `recursive` | the whole location, metadata and data files |

A wrapping handle removes what it wraps, cached schema or footer included.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::holder::local::Folder;

    let root = Folder::temporary()?.path()?.join(format!("yggdryl-docs-lifecycle-{}", std::process::id()));
    let mut folder = Folder::new(&root)?;
    folder.truncate(0)?;
    folder.child_by_path("a.log")?.write_all_bytes(b"line\n")?;

    // Clearing empties the container and keeps it.
    folder.clear()?;
    assert_eq!(folder.ls(true, false).count(), 0);
    assert_eq!(folder.kind(), yggdryl::IOKind::Directory);

    // Removing deletes it; a second call succeeds, having done nothing. A
    // handle asked for as a container keeps answering `Directory`, because that
    // is what it was asked for - the parent's listing is what shows it gone.
    let leaf = root.join("nested");
    let mut nested = Folder::new(&leaf)?;
    nested.truncate(0)?;
    assert_eq!(folder.ls(false, false).count(), 1);
    nested.remove(false)?;
    nested.remove(false)?;
    assert_eq!(folder.ls(false, false).count(), 0);
    folder.remove(false)?;

    // A wrapping handle removes what it wraps, cache included.
    let mut coded = yggdryl::coding::gzip::Gzip::new(Buffer::new());
    coded.write_all_bytes(b"symbol,price\n")?;
    coded.remove(false)?;
    assert_eq!(coded.size(), 0);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp())
    handle = IOBase(root / "logs")
    handle.mkdir()
    (handle / "a.log").write_text("line\n")

    # Clearing empties the container and keeps it.
    handle.clear()
    assert list(handle.iterdir()) == []
    assert handle.is_dir()

    # Removing deletes it; a second call succeeds, having done nothing. A
    # handle asked for as a container keeps answering `is_dir`, because that
    # is what it was asked for - the parent's listing is what shows it gone.
    handle.remove()
    handle.remove()
    assert list(IOBase(root).iterdir()) == []

    # A container that still has children is refused rather than recursed into.
    handle.mkdir()
    (handle / "a.log").write_text("line\n")
    try:
        handle.remove()
    except Exception as error:
        assert "children" in str(error)
    handle.remove(recursive=True)
    assert list(IOBase(root).iterdir()) == []

    # The handle stays usable and lazy - a write recreates the resource.
    leaf = IOBase(root / "trades.csv")
    leaf.write_text("symbol,price\n")
    leaf.remove()
    assert not leaf.exists()
    leaf.write_text("symbol,price\n")
    assert leaf.read_text() == "symbol,price\n"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = path.join(os.tmpdir(), `yggdryl-docs-lifecycle-${process.pid}`)
    const handle = new IOBase(path.join(root, 'logs'))
    handle.mkdir()
    handle.joinpath(['a.log']).writeText('line\n')

    // Clearing empties the container and keeps it.
    handle.clear()
    assert.equal([...handle.ls(true, false)].length, 0)

    // Removing deletes it; a second call succeeds, having done nothing.
    handle.remove()
    handle.remove()
    assert.equal([...new IOBase(root).ls(false, false)].length, 0)

    // The handle stays usable and lazy - a write recreates the resource.
    const leaf = new IOBase(path.join(root, 'trades.csv'))
    leaf.writeText('symbol,price\n')
    leaf.remove()
    assert.equal(leaf.exists(), false)
    leaf.writeText('symbol,price\n')
    assert.equal(leaf.readText(), 'symbol,price\n')
    new IOBase(root).remove(true)
    ```

`remove` returns nothing: absence and removal are indistinguishable.

## Edges

- `pstream_bytes(position, 0)` -> refused; `batch_size` must be non-zero.
- Stream error -> yielded once after every successful prefix, then the iterator stays fused.
- `pstream_bytes` at a non-zero position on a coded handle -> decodes and discards the prefix; frames are not seekable.
- `pstream_bytes` through [Buffered](../backends/buffered.md) -> bypasses the page cache, `cached_pages() == 0`.
- `is_tabular` on a `.parquet` leaf without the `parquet` feature -> `true`; `record_options` on [Records](records.md) names the undecodable encoding.
- `codec` with nothing to undo -> Python `None`, JavaScript `null`, never `"identity"`.
- `compress_into` to a target declaring no coding (bindings) -> `expected a target declaring a content coding`; nothing is written.
- `compress_into` to an in-memory target -> name the codec; the target has no name.
- `open` on an absent resource -> succeeds without creating it.
- `clear`/`remove` on an absent resource -> success, nothing created; permission, network, busy failures stay typed errors.
- `remove(false)` on a container with children -> refused by name; `recursive` is ignored on a leaf.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib iobase::tests::conformance
    cargo test --features "parquet iceberg" -p yggdryl --lib iobase::tests::laziness
    cargo test --features "parquet iceberg" -p yggdryl --lib iobase::tests::lifecycle
    cargo test --features "parquet iceberg" -p yggdryl --lib iobase::tests::shape
    cargo test --features "parquet iceberg" -p yggdryl --lib iocursor::
    cargo bench --bench coding -- io_pstream
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/holder/test_io.py
    python/.venv/bin/python python/benchmarks/holder/io.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test "node/tests/holder/io.test.js"
    npm run --prefix node bench:holder:io
    ```

## Performance

Criterion measured medians on one 8 MiB decoded fixture: Windows 11 x86_64, AMD Ryzen 5 150 (6 cores/12 threads), rustc 1.96.1, 2026-08-23.

| operation | decoded bytes | plain | gzip | zlib | zstd |
| --- | ---: | ---: | ---: | ---: | ---: |
| first `pstream_bytes` item | 64 KiB | 3.42 us | 80.96 us | 71.94 us | 281.24 us |
| one `pread` | 64 KiB | 1.84 us | 73.09 us | 65.32 us | 260.15 us |
| `pstream_bytes` drain | 8 MiB | 0.501 ms | 8.405 ms | 8.225 ms | 14.995 ms |
| `read_all_bytes` | 8 MiB | 1.981 ms | 15.265 ms | 14.761 ms | 21.049 ms |
| sixteen sequential `pread` calls | 1 MiB | 0.037 ms | 10.268 ms | 8.581 ms | 14.412 ms |

The last row rebuilds a decoder at every compressed offset; one `ByteStream` per scan is faster and bounded-memory.

```bash
cargo bench --bench coding -- io_pstream
```
