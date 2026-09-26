# Holder

Every storage implementation is one positional `IOBase` handle: a caller writes the same calls whatever it holds, and every backend below answers them.

| Section | Owns | Build |
| --- | --- | --- |
| [Handles](#handles) | the `Holder` enum, what a name composes to, roles, delegation | default |
| [Bytes](#bytes) | `pread`/`pwrite`, addresses, laziness, kinds, streams, cursors, media type, codings, open/close, clear/remove | default |
| [Values](#values) | whole bytes, digests, structured JSON/YAML/TOML/XML scalars, `std::io` adapters | default |
| [Records](#records) | Arrow batch reads, the three write intents, pushdown, limits, native rows | `arrow` (default), `parquet` |
| [Partitions](#partitions) | listings, globs, Hive pruning, partition columns, derived columns | default |
| [Call counts](#call-counts) | `Counted`, the `IOBase` call budget every derived operation is held to | default |
| [Buffer](#buffer) | in-memory bytes | default |
| [Local](#local) | `LocalPath`, `LocalFolder`, mapped `LocalFile` | default |
| [Filesystems](#filesystems) | Arrow-style `FileSystem`, `FsPath`, `FsFolder`, `FsFile` | default |
| [Object stores](#object-stores) | `S3Path`, `S3Folder`, `S3File` over Amazon S3, Google Cloud Storage and Azure Blob Storage | `s3` feature |
| [Buffered](#buffered) | the page cache over any handle | default |
| [ZIP](#zip) | `ZipPath`, `ZipNode`, `ZipLeaf` inside one archive, nested archives included | default, Rust only |

## Handles

`Holder` is one enum over every `IOBase` implementation. Construction records a location without probing it; a role resolves only when an operation needs it.

```text
Holder::local(path) -> Result<Holder>          // LocalPath: the role is decided later
Holder::folder(path) / Holder::file(path)      // commit to a role up front
Holder::buffer(Buffer) -> Holder               // in memory
Holder::from_url(&Url, properties)             // the scheme picks the backend
holder.into_declared_media() -> Holder         // compose what the name declares, reading nothing
holder.open() -> Result<()>                    // into_media, then open; keeps schema and footer caches
holder.as_io() -> &dyn IOBase                  // the variant as the trait object
```

=== "Rust"

    ```rust
    use yggdryl::local::LocalFolder;
    use yggdryl::holder::{Buffer, Holder};
    use yggdryl::{IOBase, IOKind};

    // Generic construction records the location without probing its role,
    // whether something is there or not.
    let root = LocalFolder::temporary()?.path()?;
    assert!(matches!(Holder::local(&root)?, Holder::LocalPath(_)));
    assert!(matches!(Holder::local(root.join("yggdryl-generic-doc.bin"))?, Holder::LocalPath(_)));

    // A value that could have been any handle. The calls do not change.
    let mut handle = Holder::buffer(Buffer::new());
    handle.write_all_bytes(b"AAPL,1\n")?;

    assert_eq!(handle.read_all_bytes()?, b"AAPL,1\n");
    assert_eq!(handle.kind(), IOKind::Memory);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl.holder import Buffer, IOBase, LocalPath
    from yggdryl.media import Text

    root = pathlib.Path(tempfile.mkdtemp())

    # The class is read off the name, not the store: neither location exists.
    assert type(IOBase(root / "trades.bin")) is LocalPath
    assert type(IOBase(root / "trades.txt.gz")) is Text

    # The class names the implementation; the contract is the same one.
    handle = IOBase.from_bytes()
    handle.write_bytes(b"AAPL,1\n")
    assert type(handle) is Buffer
    assert handle.read_bytes() == b"AAPL,1\n"
    assert handle.kind == "memory"
    ```

### Variants

| variant | holds | Python class |
| --- | --- | --- |
| `Buffer` | an in-memory byte array | `holder.Buffer` |
| `LocalFolder`, `LocalPath`, `LocalFile` | a local directory, an undecided local location, a mapped local leaf | `holder.LocalFolder`, `holder.LocalPath`, `holder.LocalFile` |
| `FsFolder`, `FsPath`, `FsFile` | the same three on an Arrow `FileSystem` | `holder.FsFolder`, `holder.FsPath`, `holder.FsFile` |
| `S3Folder`, `S3Path`, `S3File` | a prefix or container, an undecided location, one object on an [object store](#object-stores) | `holder.S3Folder`, `holder.S3Path`, `holder.S3File` |
| `ZipNode`, `ZipPath`, `ZipLeaf` | the archive root or a member prefix, an undecided member location, one member of a [ZIP archive](#zip) | Rust only |
| `Buffered` | any of the others behind the [page cache](#buffered) | `holder.Buffered` |
| `Coded` | any of the others, presenting the decoded bytes of a content coding | `coding.Identity`, `Gzip`, `Zlib`, `Zstd` |
| `Text` | any handle retained as plain-text records | `media.Text` |
| `Media` | any handle retained behind its record encoding | `media.Ipc`, `media.Parquet`, `media.Avro` |

The last four own the `Holder` they wrap; `repr` renders that stack outermost first and `into_handle` descends one layer. `into_text`, `into_coded`, `buffered`, `into_media` and `into_declared_media` never stack. JavaScript has one `IOBase` class over the whole enum.

### What the name declares

`into_declared_media` puts the name's content coding underneath and its record encoding on top, without resolving the store.

| name | composed handle |
| --- | --- |
| `trades.txt.gz` | `Text(Gzip(LocalPath))` |
| `archive.bin.gz` | `Gzip(LocalPath)` |
| `trades.parquet` | `Parquet(LocalPath)` |
| `trades.arrows` | `Ipc(LocalPath)` |
| `trades.avro` | `Avro(LocalPath)` |
| `trades.log` | `Text(LocalPath)` |
| `trades.json`, `trades` | `LocalPath` |
| `trades.parquet.gz` | `LocalPath`: Parquet compresses internally, so the writer refuses the name |

=== "Rust"

    ```rust
    use yggdryl::holder::{Buffer, Holder};
    use yggdryl::{Codec, IOBase, MimeType, Url};

    let named = Url::from_str("file:///trades.txt.gz")?;
    let stored = Codec::Gzip.dump(b"AAPL,1\n")?;
    let handle = Holder::buffer(Buffer::from_bytes(stored).with_media_type(named.media_type()));

    let composed = handle.into_declared_media();

    // Text records over a gzip view over the bytes, and no read paid for it.
    assert!(matches!(composed, Holder::Text(_)));
    assert_eq!(composed.read_all_bytes()?, b"AAPL,1\n");
    assert_eq!(composed.media_type().base(), &MimeType::PLAIN_TEXT);
    ```

=== "Python"

    ```python
    import gzip
    import pathlib
    import tempfile

    from yggdryl.holder import IOBase, LocalPath
    from yggdryl.media import Text

    location = pathlib.Path(tempfile.mkdtemp()) / "trades.txt.gz"

    handle = IOBase(location)
    assert type(handle) is Text
    assert repr(handle) == f'Text(Gzip(LocalPath("{handle.url}")))'

    # The composed handle reads and writes the decoded value.
    handle.write_bytes(b"AAPL,1\n")
    assert handle.read_bytes() == b"AAPL,1\n"
    assert gzip.decompress(location.read_bytes()) == b"AAPL,1\n"

    # `media_type` is what the handle presents; `codec` is what the store holds.
    assert str(handle.media_type) == "text/plain"
    assert handle.codec == "gzip"

    # `LocalPath` commits to a role and skips the composition: the stored bytes.
    assert LocalPath(location).read_bytes()[:2] == b"\x1f\x8b"
    ```

### Hierarchy

`parent`, `child_by_path` and `ls` return `Holder`, so one enum walks any tree. Python composes wherever a handle is described, children included.

=== "Rust"

    ```rust
    use yggdryl::holder::Holder;
    use yggdryl::IOBase;
    use yggdryl::local::LocalFolder;

    let root = Holder::folder(LocalFolder::temporary()?.path()?)?;
    assert!(root.is_container());

    // A child need not exist. Naming one yields a leaf handle, and nothing is created.
    let leaf = root.child_by_path("yggdryl-generic-child.bin")?;
    assert!(matches!(leaf, Holder::LocalFile(_)));
    assert!(!leaf.is_container());
    assert_eq!(leaf.size(), 0);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl.holder import LocalFile, LocalFolder
    from yggdryl.media import Ipc

    root = LocalFolder(pathlib.Path(tempfile.mkdtemp()))

    # A child need not exist. Naming one yields a leaf handle, and nothing is created.
    plain = root / "yggdryl-generic-child.bin"
    assert type(plain) is LocalFile
    assert plain.size == 0

    # Composition is applied wherever a handle is described, children included.
    records = root.joinpath("yggdryl-generic-child.arrows")
    assert type(records) is Ipc
    assert repr(records) == f'Ipc(LocalFile("{records.url}"))'
    assert type(records.parent) is LocalFolder
    ```

### Roles

A role is what a location turns out to be. A backend implements one trait per role and gets the rest of `IOBase` from its defaults; Python names the three constructors that commit to a role and skip the composition.

```text
trait IOPath   { fn path_url(&self) -> &Url;   fn is_folder(&self) -> bool;   fn is_file(&self) -> bool; }
trait IOFolder { fn folder_url(&self) -> &Url; fn folder_exists(&self) -> bool;
                 fn create_folder(&self) -> Result<()>;
                 fn list_folder(&self, recursive: bool, include_private: bool) -> Listing;
                 fn delete_folder(&mut self) -> Result<()>; }
trait IOFile   { fn file_url(&self) -> &Url;   fn file_exists(&self) -> bool;
                 fn clear_file(&mut self) -> Result<()>;  fn delete_file(&mut self) -> Result<()>; }
```

Everything else is pre-implemented: a folder reads nothing, refuses byte writes, is created by `truncate(0)` and answers `inode/directory`; a file lists nothing and refuses a child; a path answers `Directory`, `File` or `Unknown` by looking.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::{IOKind, MimeType};
    use yggdryl::local;

    let temporary = local::LocalFolder::temporary()?.path()?;
    let path = temporary.join("yggdryl-docs-io-folder");
    let _ = std::fs::remove_dir_all(&path);
    let mut folder = local::LocalFolder::new(&path)?;

    // A container holds no bytes: reads are empty, byte writes are refused.
    let mut probe = [0_u8; 4];
    assert_eq!(folder.pread(0, &mut probe)?, 0);
    assert_eq!(folder.size(), 0);
    let refused = folder.pwrite(0, b"x").unwrap_err().to_string();
    assert!(refused.contains("got the directory"), "{refused}");

    // Truncating to zero is the write that brings a container into being.
    folder.truncate(0)?;
    assert!(folder.exists());
    assert_eq!(folder.kind(), IOKind::Directory);
    assert_eq!(folder.media_type().base(), &MimeType::DIRECTORY);
    assert_eq!(folder.ls(false, false).count(), 0);

    // A location that arrived from outside answers by looking at what is there.
    assert_eq!(local::LocalPath::new(&temporary)?.kind(), IOKind::Directory);
    let undecided = local::LocalPath::new(temporary.join("yggdryl-docs-io-undecided"))?;
    assert_eq!(undecided.kind(), IOKind::Unknown);
    assert!(undecided.read_all_bytes()?.is_empty());

    // A leaf is not a container: it lists nothing and resolves no child.
    let leaf = local::LocalFile::new(temporary.join("yggdryl-docs-io-leaf.arrows"))?;
    assert_eq!(leaf.ls(true, false).count(), 0);
    assert!(leaf.child_by_path("nested").is_err());

    std::fs::remove_dir_all(&path)?;
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl.holder import LocalFile, LocalFolder, LocalPath

    root = pathlib.Path(tempfile.mkdtemp())
    path = root / "yggdryl-docs-io-folder"
    folder = LocalFolder(path)

    # A container holds no bytes: reads are empty, byte writes are refused.
    assert folder.read_bytes() == b""
    assert folder.size == 0
    try:
        folder.pwrite(0, b"x")
        raise AssertionError("a directory accepted bytes")
    except IsADirectoryError as refused:
        assert "got the directory" in str(refused)

    # Truncating to zero is the write that brings a container into being.
    folder.truncate(0)
    assert path.is_dir()
    assert folder.kind == "directory"
    assert str(folder.media_type) == "inode/directory"
    assert list(folder.ls()) == []

    # A location that arrived from outside answers by looking at what is there.
    assert LocalPath(root).kind == "directory"
    undecided = LocalPath(root / "yggdryl-docs-io-undecided")
    assert undecided.kind == "unknown"
    assert undecided.read_bytes() == b""

    # A leaf is not a container: it lists nothing and resolves no child.
    leaf = LocalFile(root / "yggdryl-docs-io-leaf.arrows")
    assert list(leaf.ls()) == []
    try:
        leaf.joinpath("nested")
        raise AssertionError("a leaf resolved a child")
    except ValueError as refused:
        assert "expected a container" in str(refused)
    ```

### Delegating to a wrapped handle

A wrapper forwards the contract to the handle it holds with one macro per trait. Rust only: neither binding can add a backend.

```text
delegate_iobase!(handle)                      // storage contract, open, opened, close; no records
delegate_iomedia!(handle)                     // dimensions, options, Field and reader reads, typed writes
delegate_iobase!(handle, except_lifecycle)    // omits clear, remove, is_atomic, is_tabular, is_io
delegate_iobase!(handle: pread, size, ...)    // only the named methods; the rest keep the trait default
```

```rust
use yggdryl::{IOBase, IOMedia};
use yggdryl::holder::Buffer;

/// A wrapper mirrors the handle's bytes rather than owning bytes of its own.
struct Wrapped {
    handle: Buffer,
}

impl IOMedia for Wrapped {
    yggdryl::delegate_iomedia!(handle);
}

impl IOBase for Wrapped {
    yggdryl::delegate_iobase!(handle);
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut wrapper = Wrapped {
        handle: Buffer::new(),
    };
    wrapper.open()?;
    wrapper.write_all_bytes(b"AAPL")?;

    assert_eq!(wrapper.opened(), wrapper.handle.opened());
    assert_eq!(wrapper.read_all_bytes()?, b"AAPL");
    assert_eq!(wrapper.handle.as_slice(), b"AAPL");
    Ok(())
}
```

## Bytes

A backend implements the methods below; every other byte operation derives from `pread` and `pwrite`. Offsets are explicit, so two readers never interfere and a footer-first container reads its index without seeking.

```text
fn pread(&self, offset: u64, buffer: &mut [u8]) -> Result<usize>   // short only at end of value; 0 past it
fn pwrite(&mut self, offset: u64, bytes: &[u8]) -> Result<usize>   // grows, zero-fills a gap
fn size(&self) -> u64                                              // never above capacity()
fn reserve(&mut self, capacity: u64) -> Result<()>                 // moves capacity only
fn capacity(&self) -> u64
fn truncate(&mut self, size: u64) -> Result<()>
fn uri(&self) -> Option<&Uri>                                      // the one identity every backend owes
fn media_type(&self) -> &MediaType                                  // decoded representation; set_media_type declares one
```

The bindings spell the read `read_range_bytes` / `readRangeBytes` and keep `pwrite`.

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

### Addresses

`uri` is the identifier the bytes are reached through; `url` narrows it when it names a place. A buffer is stored nowhere and still answers a `mem:` identity. `mtime()` (Rust only) answers UTC nanoseconds for a store that records one, and `None` otherwise.

=== "Rust"

    ```rust
    use yggdryl::holder::Buffer;
    use yggdryl::local::LocalFolder;
    use yggdryl::{IOBase, Uri};

    let root = LocalFolder::temporary()?.path()?;
    let folder = LocalFolder::new(&root)?;

    // A located handle answers one identifier through both doors.
    assert_eq!(IOBase::uri(&folder), IOBase::url(&folder).map(AsRef::<Uri>::as_ref));
    assert_eq!(IOBase::uri(&folder).unwrap().scheme().as_str(), "file");

    // A buffer is not stored anywhere, so its address is an identity.
    let buffer = Buffer::from_bytes(b"symbol\n".to_vec());
    assert_eq!(buffer.uri().unwrap().scheme().as_str(), "mem");
    ```

=== "Python"

    ```python
    import tempfile, pathlib
    from yggdryl import IOBase, Uri, Url

    root = pathlib.Path(tempfile.mkdtemp())
    (root / "ticks.csv").write_text("symbol\n", encoding="utf-8")
    handle = IOBase(root / "ticks.csv")

    assert handle.uri == handle.url
    assert isinstance(handle.uri, Url)
    assert handle.uri.scheme == "file"

    # A buffer is addressed by its identity rather than by a place.
    assert IOBase.from_bytes(b"symbol\n").uri.scheme == "mem"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-uri-'))
    fs.writeFileSync(path.join(root, 'ticks.csv'), 'symbol\n')
    const handle = new IOBase(path.join(root, 'ticks.csv'))

    assert.equal(handle.uri.toString(), handle.url.toString())
    assert.equal(handle.uri.scheme, 'file')
    assert.equal(IOBase.fromBytes(Buffer.from('symbol\n')).uri.scheme, 'mem')
    ```

### Laziness and kinds

Constructing touches nothing, a read of something absent is empty, and a write creates the resource and any missing parent, so probing a location needs no existence check.

=== "Rust"

    ```rust
    use yggdryl::{holder::Buffer, local};
    use yggdryl::{IOBase, IOKind};

    // Kinds that need no probe: bytes with no location, and a directory.
    assert_eq!(Buffer::new().kind(), IOKind::Memory);
    assert!(IOKind::Memory.is_leaf());
    let folder = local::LocalFolder::temporary()?;
    assert_eq!(folder.kind(), IOKind::Directory);
    assert!(folder.is_container());

    let path = folder.path()?.join("yggdryl-docs-io-lazy.csv");
    let _ = std::fs::remove_file(&path);

    // Constructing touches nothing: no file is created, opened, or mapped.
    let mut handle = local::LocalFile::new(&path)?;
    assert!(!handle.exists());

    // Reading something absent yields nothing rather than failing, and nothing
    // there has decided a kind.
    assert_eq!(handle.size(), 0);
    let mut probe = [0_u8; 8];
    assert_eq!(handle.pread(0, &mut probe)?, 0);
    assert_eq!(handle.kind(), IOKind::Unknown);
    assert!(!handle.kind().is_known());

    // Writing creates the resource, and any parent it needs; the write settles the kind.
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
    folder = IOBase(root)
    assert folder.is_dir()
    assert not folder.is_file()

    # Constructing touches nothing: no file is created, opened, or mapped.
    handle = IOBase(root / "nested" / "lazy.csv")
    assert not handle.exists()

    # Reading something absent yields nothing rather than raising.
    assert handle.size == 0
    assert handle.read_bytes() == b""

    # Writing creates the resource, and any parent it needs; the write settles the kind.
    handle.write_text("symbol,price\n")
    assert handle.is_file()
    assert not handle.is_dir()
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
    const folder = new IOBase(root)
    assert.ok(folder.isDir())
    assert.ok(!folder.isFile())

    // Constructing touches nothing: no file is created, opened, or mapped.
    const handle = new IOBase(path.join(root, 'nested', 'lazy.csv'))
    assert.ok(!handle.exists())

    // Reading something absent yields nothing rather than throwing.
    assert.equal(handle.size, 0)
    assert.equal(handle.readBytes().length, 0)

    // Writing creates the resource, and any parent it needs; the write settles the kind.
    handle.writeText('symbol,price\n')
    assert.ok(handle.isFile())
    assert.ok(!handle.isDir())
    assert.equal(handle.readText(), 'symbol,price\n')

    fs.rmSync(root, { recursive: true, force: true })
    ```

| `IOKind` | Meaning |
| --- | --- |
| `Memory` | bytes with no location |
| `File` | leaf holding bytes |
| `Directory` | container of resources |
| `Unknown` | location that does not exist yet |
| `Table`, `Namespace`, `Catalog` | containers a table format adds |

Rust asks `is_container`, `is_leaf`, `is_known`; the bindings `exists`, `is_dir`, `is_file`.

### Bytes or rows

*Atomic* is the byte surface, *tabular* the [record surface](#records). They exclude each other everywhere except plain text, which is both; a container holding neither answers `false` to both.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::{MimeType};
    use yggdryl::local;

    // A leaf answers from its representation, and the two are complements.
    let mut notes = Buffer::new();
    notes.set_media_type(MimeType::PLAIN_TEXT.into());
    assert!(notes.is_atomic());
    assert!(!notes.is_tabular());

    // The name is enough: nothing has been written to this location yet.
    let trades = local::LocalFile::new(local::LocalFolder::temporary()?.path()?.join("yggdryl-docs-shape.parquet"))?;
    assert!(trades.is_tabular());
    assert!(!trades.is_atomic());

    // A container is neither one whole byte value nor - with nothing under
    // it - a table.
    let folder = local::LocalFolder::temporary()?;
    assert!(!folder.is_atomic());
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    root = IOBase(pathlib.Path(tempfile.mkdtemp()))

    # Nothing this build reads as rows, so the bytes are the whole value.
    notes = root / "notes.json"
    assert notes.is_atomic()
    assert not notes.is_tabular()

    # The name is enough: nothing has been written to this location yet.
    trades = root / "trades.parquet"
    assert trades.is_tabular()
    assert not trades.is_atomic()

    # Plain text is both: one byte value, and the rows that value splits into.
    log = root / "app.log"
    assert log.is_atomic()
    assert log.is_tabular()

    assert not root.is_atomic()
    assert not root.is_tabular()
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

### Streams and cursors

`pstream_bytes(position, batch_size)` yields owned arrays lazily and never asks for `size`; the bindings default the batch to 65,536 bytes. A cursor makes the position explicit, and two cursors advance independently.

=== "Rust"

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

`tell` and `seek` move a cursor; reads and writes through it advance it, and `std::io::Read` rides the same position.

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
| Rust | `IOCursor`: `tell`, `seek_to`, `seek`, `read_next`, `write_next`; `Cursor<H>` from `cursor`/`cursor_at` stays a full handle and implements `Read`, `Write`, `Seek` |
| Python | shares the handle; `seek(offset, whence)`, `read(size=-1)` |
| JavaScript | shares the handle; `seek`, `tell`, `position` |

`ByteStream` implements `std::io::Read` and decodes a coded handle straight from its source, retaining no decoded pages. `DEFAULT_STREAM_BATCH_SIZE` (64 KiB) shapes what a reader hands out and `DEFAULT_FETCH_BYTE_SIZE` (1 MiB) what it asks the store for, so a compressed scan costs one request per window rather than one per decoder pull: a 1 GiB object is about a thousand asks instead of thirty-two thousand.

### Media type and codings

`media_type` is computed on ask, re-derived when the bytes change, and a declared type wins. It names the decoded representation; `codec` names the coding the stored bytes carry.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::{Codec, MimeType, Url};

    // Nothing names an in-memory buffer, so its type comes from its bytes.
    let mut handle = Buffer::from_bytes(br#"{"symbol":"AAPL"}"#.to_vec());
    assert_eq!(handle.media_type().base(), &MimeType::JSON);

    // It is re-derived after the content changes.
    handle.write_all_bytes(b"PAR1payload")?;
    assert_eq!(handle.media_type().base(), &MimeType::PARQUET);

    // A declared type wins, and the codings it carries are what `codec` reports.
    let named = Buffer::new().with_media_type(Url::from_str("file:///trades.json.gz")?.media_type());
    assert_eq!(named.media_type().base(), &MimeType::JSON);
    assert_eq!(named.codec(), Codec::Gzip);
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

`compress_into` and `decompress_into` move every byte into another handle and add or remove a coding. Both address stored bytes, so a handle presenting a decoded view is refused by name rather than double-coded.

```text
fn compress_into(&self, target: &mut dyn IOBase, codec: Codec) -> Result<u64>   // bindings: the target's declared coding
fn decompress_into(&self, target: &mut dyn IOBase) -> Result<u64>                // the source's declared coding
fn copy_into(&self, target: &mut dyn IOBase) -> Result<u64>                      // chunked; writes through the target's coding
```

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
    from yggdryl.holder import LocalPath

    root = pathlib.Path(tempfile.mkdtemp())
    plain = IOBase(root / "rows.json")
    plain.write_bytes(b'{"symbol":"AAPL"}')

    # Nothing wraps these bytes, so there is nothing to undo.
    assert plain.codec is None

    # `LocalPath` addresses the stored bytes. `IOBase` would compose the gzip the
    # name declares and present the decoded value, which is not what moves here.
    stored = LocalPath(root / "rows.json.gz")
    assert stored.codec == "gzip"

    # The target's name already said gzip, so nothing here repeats it.
    assert plain.compress_into(stored) == stored.size
    assert stored.read_bytes()[:2] == b"\x1f\x8b"

    decoded = LocalPath(root / "roundtrip.json")
    assert stored.decompress_into(decoded) == 17
    assert decoded.read_bytes() == plain.read_bytes()
    assert decoded.codec is None

    # Writing through a coded handle is the other way to store the coded form.
    coded = IOBase(root / "copy.json.gz")
    assert plain.copy_into(coded) == 17
    assert coded.read_bytes() == plain.read_bytes()
    assert LocalPath(root / "copy.json.gz").read_bytes()[:2] == b"\x1f\x8b"

    # Which is why that handle is refused here: it codes what passes through it.
    try:
        plain.compress_into(coded)
    except ValueError as error:
        reason = str(error)
    assert "expected a target presenting its stored bytes" in reason

    # A target declaring no coding is refused rather than copied unchanged.
    try:
        plain.compress_into(LocalPath(root / "copy.json"))
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

A name declaring a coding is composed at construction, so `IOBase("app.log.gz")` already presents the decoded value; `LocalPath` addresses the stored bytes and `into_coded` puts the coding back on.

=== "Python"

    ```python
    import gzip
    import pathlib
    import tempfile

    from yggdryl import IOBase
    from yggdryl.coding import Gzip
    from yggdryl.holder import LocalPath

    root = pathlib.Path(tempfile.mkdtemp())
    path = root / "app.log.gz"
    path.write_bytes(gzip.compress(b"symbol,price\n"))

    # The name declared gzip, so the handle already reads the log it holds.
    handle = IOBase(path)
    assert handle.read_text() == "symbol,price\n"
    assert str(handle.media_type) == "text/plain"
    assert handle.codec == "gzip"

    # `LocalPath` addresses the stored bytes instead, and `into_coded` puts the
    # coding back on.
    stored = LocalPath(path)
    assert stored.read_bytes()[:2] == b"\x1f\x8b"
    coded = stored.into_coded()
    assert isinstance(coded, Gzip)
    assert coded.read_text() == "symbol,price\n"

    # The wrapper owns what it wraps, so the handle it took is spent.
    reason = None
    try:
        stored.read_bytes()
    except ValueError as error:
        reason = str(error)
    assert "consumed by a conversion" in reason

    # A name declaring no coding passes its bytes through, so this is safe to
    # call on any leaf; a repeat never decodes twice.
    plain = IOBase(root / "plain.csv")
    plain.write_text("symbol,price\n")
    assert plain.into_coded().into_coded().read_text() == "symbol,price\n"
    ```

### Open and close

A handle works without `open`; opening moves materialization to a known point and keeps cached state until `close`. Python binds the pair to `with`, JavaScript adds `Symbol.dispose`.

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

    import pyarrow as pa

    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp())
    path = root / "trades.csv"

    # `with` is the scoped pair: `__enter__` opens and `__exit__` closes.
    with IOBase(path) as handle:
        handle.write_text("symbol,price\n")
        assert handle.opened
        assert not handle.closed

    # Closing published the bytes at their exact length, which is what another
    # reader needs; the handle stays usable and simply re-materializes.
    assert path.stat().st_size == 13
    assert IOBase(path).read_text() == "symbol,price\n"

    # Metadata-heavy work belongs inside the scope: the schema probe, the
    # per-batch reads, and the size checks all reuse what `open` cached, and
    # `close` releases it at a known point.
    target = root / "lake" / "trades.parquet"
    IOBase(target).overwrite_arrow_table(pa.table({"id": [1, 2]}))
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

| Implementation | `open` caches |
| --- | --- |
| [`Buffer`](#buffer) | nothing; `opened` stays `false` |
| [`LocalFile`](#local) | descriptor and memory mapping |
| [`Coded`](../media/index.md#compression) | the decoded value |
| [IPC](../media/index.md#arrow-ipc) | schema and dimensions |
| [Parquet](../media/index.md#parquet) | the footer |
| [Avro](../media/index.md#avro) | header and block metadata |
| [Text](../media/index.md#plain-text) | resolved field, coding plan, dimensions |

### Clear and remove

`clear` empties and keeps the resource; `remove` deletes it without a probe, treats absence as success, and refuses a container with children unless `recursive`. A wrapping handle removes what it wraps, cache included.

| Call | Leaf | Container | [Iceberg](../media/index.md#iceberg) `Table` |
| --- | --- | --- | --- |
| `clear` | size `0` | loses every child recursively | one snapshot with no data files; schema, properties, history stay |
| `remove` | deleted | deleted; refused while children remain, unless `recursive` | the whole location, metadata and data files |

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::local::LocalFolder;

    let root = LocalFolder::temporary()?.path()?.join(format!("yggdryl-docs-lifecycle-{}", std::process::id()));
    let mut folder = LocalFolder::new(&root)?;
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
    let mut nested = LocalFolder::new(&leaf)?;
    nested.truncate(0)?;
    assert_eq!(folder.ls(false, false).count(), 1);
    nested.remove(false)?;
    nested.remove(false)?;
    assert_eq!(folder.ls(false, false).count(), 0);
    folder.remove(false)?;

    // A wrapping handle removes what it wraps, cache included.
    let mut coded = yggdryl::gzip::Gzip::new(Buffer::new());
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

### From an open file

Python only: `IOBase(...)` takes an open file or stream; a named file captures its location, a nameless stream its content.

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

### Bytes performance

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

## Values

Whole-value conveniences derive from `pread`/`pwrite`. The bindings spell them `read_bytes`/`read_text` and `write_bytes`/`write_text`; `read_range_bytes` and `append_bytes` keep the core name.

```text
fn read_all_bytes(&self) -> Result<Vec<u8>>
fn read_range_bytes(&self, offset: u64, length: usize) -> Result<Vec<u8>>   // clamped: past the end is empty
fn write_all_bytes(&mut self, bytes: &[u8]) -> Result<()>
fn append_bytes(&mut self, bytes: &[u8]) -> Result<u64>                    // the offset the bytes landed at
fn read_digest(&self, algorithm: DigestAlgorithm) -> Result<Digest>
fn read_scalar(&self, field: Option<&Field>) -> Result<Scalar>              // JSON, YAML, TOML or XML by media type
fn write_scalar(&mut self, value: &Scalar) -> Result<()>
```

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;

    let mut handle = Buffer::new();
    handle.write_all_bytes(b"symbol,price\n")?;

    // `append_bytes` reports the offset the bytes landed at.
    assert_eq!(handle.append_bytes(b"AAPL,1\n")?, 13);
    assert_eq!(handle.read_range_bytes(0, 6)?, b"symbol");
    // A range past the end yields what exists rather than failing.
    assert!(handle.read_range_bytes(100, 4)?.is_empty());
    assert_eq!(handle.read_all_bytes()?.len(), 20);
    ```

=== "Python"

    ```python
    from yggdryl import IOBase

    handle = IOBase.from_bytes()
    handle.write_bytes(b"symbol,price\n")

    # `append_bytes` reports the offset the bytes landed at.
    assert handle.append_bytes(b"AAPL,1\n") == 13
    assert handle.read_range_bytes(0, 6) == b"symbol"
    # A range past the end yields what exists rather than raising.
    assert handle.read_range_bytes(100, 4) == b""
    assert len(handle.read_bytes()) == 20
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase } = require('yggdryl')

    const handle = IOBase.fromBytes()
    handle.writeBytes(Buffer.from('symbol,price\n'))

    // `appendBytes` reports the offset the bytes landed at.
    assert.equal(handle.appendBytes(Buffer.from('AAPL,1\n')), 13)
    assert.equal(handle.readRangeBytes(0, 6).toString(), 'symbol')
    // A range past the end yields what exists rather than throwing.
    assert.equal(handle.readRangeBytes(100, 4).length, 0)
    assert.equal(handle.readBytes().length, 20)
    ```

### Digests

Both stream [`pstream_bytes`](#streams-and-cursors) and retain one bounded chunk; absence digests as no bytes. [Hashing](../hashing.md) owns the digest values.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::DigestAlgorithm;

    let mut handle = Buffer::new();
    handle.write_all_bytes(b"symbol,price\nAAPL,1\n")?;

    assert_eq!(
        handle.read_digest(DigestAlgorithm::Xxh3)?,
        DigestAlgorithm::Xxh3.digest(&handle.read_all_bytes()?),
    );
    assert_eq!(
        handle.read_range_digest(0, 6, DigestAlgorithm::Xxh3)?,
        DigestAlgorithm::Xxh3.digest(b"symbol"),
    );
    // Absence is emptiness here too: nothing written digests as no bytes.
    assert_eq!(
        Buffer::new().read_digest(DigestAlgorithm::Xxh32)?,
        DigestAlgorithm::Xxh32.digest(b""),
    );
    ```

=== "Python"

    ```python
    from yggdryl import IOBase
    from yggdryl import xxhash

    handle = IOBase.from_bytes()
    handle.write_bytes(b"symbol,price\nAAPL,1\n")

    assert handle.read_digest("xxh3-64") == xxhash.digest(handle.read_bytes(), "xxh3-64")
    assert handle.read_range_digest(0, 6) == xxhash.digest(b"symbol", "xxh3-64")
    assert IOBase.from_bytes().read_digest("xxh32") == xxhash.digest(b"", "xxh32")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase, xxhash } = require('yggdryl')

        const handle = IOBase.fromBytes()
    const payload = Buffer.from('symbol,price\nAAPL,1\n')
    handle.writeBytes(payload)

    assert.ok(handle.readDigest('xxh3-64').equals(xxhash.digest(payload, 'xxh3-64')))
    assert.ok(handle.readRangeDigest(0, 6).equals(xxhash.digest(Buffer.from('symbol'), 'xxh3-64')))
    ```

### Structured values

The media type selects JSON, YAML, TOML or XML and any outer gzip, zlib or zstd; a `field` directs parsing, and without one the natural value is inferred. Rust reads a struct row as `Scalar::Serie`; Python and JavaScript restore field names, and `cls=Scalar` / `{ scalar: true }` return the core value. The codecs are on the [Media](../media/index.md#json) page.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::{Field, Url, Scalar};

    let media = Url::from_str("file:///trade.json.gz")?.media_type();
    let mut handle = Buffer::new().with_media_type(media);
    let value = Scalar::from_struct([
        ("quantity", Scalar::from(2_i64)),
        ("symbol", Scalar::from("AAPL")),
    ])?;
    handle.write_scalar(&value)?;

    let field = Field::from_str(
        "trade: struct<quantity: int32 not null, symbol: utf8 not null> not null",
    )?;
    assert_eq!(handle.read_scalar(Some(&field))?.get(0).as_deref(), Some(&Scalar::from(2_i64)));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase, Scalar

    path = pathlib.Path(tempfile.mkdtemp()) / "trade.json.gz"
    handle = IOBase(path)
    handle.write_scalar({"quantity": 2, "symbol": "AAPL"})
    field = "trade: struct<quantity: int32 not null, symbol: utf8 not null> not null"
    assert handle.read_scalar(field) == {"quantity": 2, "symbol": "AAPL"}
    value = handle.read_scalar(field, cls=Scalar)
    assert value.kind == "serie"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase, Scalar } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-value-'))
    const handle = new IOBase(path.join(root, 'trade.json.gz'))
    handle.writeScalar({ quantity: 2, symbol: 'AAPL' })
    const field = 'trade: struct<quantity: int32 not null, symbol: utf8 not null> not null'
    assert.deepEqual(handle.readScalar(field), { quantity: 2, symbol: 'AAPL' })
    const value = handle.readScalar({ field, scalar: true })
    assert.ok(value instanceof Scalar)
    assert.equal(value.kind, 'serie')
    ```

### Streaming adapters

Rust only: `reader_at` and `writer_at` borrow the handle as `std::io::Read` / `std::io::Write`, each advancing its own offset.

```rust
use std::io::{Read, Write};

use yggdryl::IOBase;
use yggdryl::holder::Buffer;

let mut handle = Buffer::new();
handle.writer_at(0).write_all(b"symbol,price\n")?;
handle.append_bytes(b"AAPL,1\n")?;

let mut text = String::new();
handle.reader_at(13).read_to_string(&mut text)?;
assert_eq!(text, "AAPL,1\n");
```

### Values performance

Criterion measured one 16,384-record JSON value through `IOBase`; each compressed case includes coding and parsing or rendering. The host is Windows 11 x86_64, an AMD Ryzen 5 150 (6 cores/12 threads), rustc 1.96.1, on 2026-08-23.

| representation | `read_scalar` | read throughput | `write_scalar` | write throughput |
| --- | ---: | ---: | ---: | ---: |
| JSON | 71.445 ms | 11.880 MiB/s | 19.137 ms | 44.352 MiB/s |
| JSON + gzip | 81.427 ms | 10.424 MiB/s | 394.83 ms | 2.150 MiB/s |
| JSON + zlib | 79.588 ms | 10.665 MiB/s | 385.78 ms | 2.200 MiB/s |
| JSON + zstd | 78.580 ms | 10.801 MiB/s | 195.60 ms | 4.339 MiB/s |

```bash
cargo bench --bench media --features parquet -- io_scalar
```

## Records

One Arrow batch read and three explicit write intents on every handle. The handle's media type picks the encoding through `record_options()`; one [`RecordOptions`](../media/index.md#options) is the only settings argument - Rust requires it, Python takes keyword-only `options=`, JavaScript a trailing `options?`. A write completes its rows onto the field the resource already stores by the [declared-column rule](../types/cast.md): a required stored column refuses a value it cannot hold, a null or a missing column by name, and the resource is left as it was.

=== "Rust"

    ```text
    read_arrow_reader(&self, options: &RecordOptions) -> Result<BatchReader>
    read_arrow_field(&self, options: &RecordOptions) -> Result<Field>
    row_size(&self) -> Result<u64>          // whole media; projection and limits never change it
    column_size(&self) -> Result<usize>

    overwrite_arrow_reader(&mut self, reader: BatchReader, options: &RecordOptions) -> Result<()>   // the one required hook
    append_arrow_reader(&mut self, reader: BatchReader, options: &RecordOptions) -> Result<()>
    merge_arrow_reader(&mut self, reader: BatchReader, options: &RecordOptions) -> Result<()>       // needs merge_by

    overwrite|append|merge_arrow_batch(&mut self, batch: RecordBatch, options: &RecordOptions) -> Result<()>
    overwrite|append|merge_records(&mut self, records, options: &RecordOptions) -> Result<()>

    write_arrow_reader(&mut self, reader: BatchReader, mode: IOMode, options: &RecordOptions) -> Result<()>
    write_arrow_batch(&mut self, batch: RecordBatch, mode: IOMode, options: &RecordOptions) -> Result<()>
    write_records(&mut self, records, mode: IOMode, options: &RecordOptions) -> Result<()>
    ```

=== "Python"

    ```text
    read_arrow_reader(*, options=None) -> pyarrow.RecordBatchReader
    read_records(cls=None, *, options=None) -> Iterator[dict | dataclass]
    overwrite|append|merge_arrow_reader(reader, *, options=None) -> None
    overwrite|append|merge_arrow_table(table, *, options=None) -> None
    overwrite|append|merge_arrow_batch(batch, *, options=None) -> None
    overwrite|append|merge_records(records, *, options=None) -> None
    write_arrow_reader|table|batch(value, mode, *, options=None) -> None
    write_records(records, mode, *, options=None) -> None
    ```

=== "JavaScript"

    ```text
    readArrowReader(options?) -> BatchReader
    readRecords(cls?, options?) -> Iterable<object>
    overwrite|append|mergeArrowReader(reader, options?) -> void
    overwrite|append|mergeArrowTable(table, options?) -> void
    overwrite|append|mergeArrowBatch(batch, options?) -> void
    overwrite|append|mergeRecords(records, options?) -> void | Promise<void>
    writeArrowReader|Table|Batch(value, mode, options?) -> void
    writeRecords(records, mode, options?) -> void | Promise<void>
    ```

Default append and merge shape once and delegate to `overwrite_arrow_reader`. `read_arrow` / `write_arrow` answer and take a [`SerieReader`](../types/serie.md#a-handle-reads-and-writes-it-whatever-it-holds) whatever the handle holds.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::arrow;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, StructType, Url};

    // A non-null struct Field is the schema.
    let schema = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])?)
    .required_field("row");

    let arrow_schema = schema.clone().into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("AAPL"), None])),
        ],
    )?;

    // The handle's own media type picks the encoding; no format argument is passed.
    let mut handle = Buffer::new().with_media_type(Url::from_str("file:///trades.arrows")?.media_type());
    let options = handle.record_options()?;

    // Overwrite takes a batch reader; the method name fixes the write intent.
    handle.overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &options)?;
    assert_eq!(handle.read_arrow_field(&options)?, schema);
    assert_eq!((handle.row_size()?, handle.column_size()?), (2, 2));

    // The read path returns one. Batches arrive one at a time, never as a vector.
    let mut rows = 0;
    for batch in handle.read_arrow_reader(&options)? {
        rows += batch?.num_rows();
    }
    assert_eq!(rows, 2);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    # A PyArrow schema is the schema; the binding imports it once at the boundary.
    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string()),
    ])
    batch = pa.record_batch({"id": [1, 2], "symbol": ["AAPL", None]}, schema=schema)

    # The handle's own media type picks the encoding; no format argument is passed.
    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
    options = handle.record_options()

    # The write path takes a batch reader and nothing else.
    handle.overwrite_arrow_batch(batch, options=options)
    assert handle.read_arrow_field(options=options).name == "row"

    # The read path returns one. Batches arrive one at a time, never as a vector.
    rows = sum(part.num_rows for part in handle.read_arrow_reader(options=options))
    assert rows == 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, Field, IOBase, MimeType, fields } = require('yggdryl')

    // A non-null struct Field is the schema.
    const schema = fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('symbol: utf8')],
      { nullable: false },
    )

    const table = new arrow.Table({
      id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
      symbol: arrow.vectorFromArray(['AAPL', null], new arrow.Utf8()),
    })

    // The handle's own media type picks the encoding; no format argument is passed.
    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM
    const options = handle.recordOptions()

    // The write path takes a batch reader and nothing else.
    handle.overwriteArrowReader(BatchReader.from(table), options)
    assert.ok(handle.readArrowField(options).equals(schema))

    // The read path returns one. Batches arrive one at a time, never as a vector.
    let rows = 0
    for (const batch of handle.readArrowReader(options)) {
      rows += batch.numRows
    }
    assert.equal(rows, 2)
    ```

### Native rows

A row is anything `TryInto<Scalar>`: an ordered `Scalar::Serie` under `options.field`, or a `Scalar::Struct` resolved to that order. The bindings take plain objects and dataclasses, and JavaScript `readRecords(Class)` builds instances.

```rust
use yggdryl::media::IORecordOptions;
use yggdryl::{IOBase, IOMedia, StructType};
use yggdryl::holder::Buffer;
use yggdryl::{DataType, MimeType, Scalar};

struct Quote(i32, &'static str);

impl From<Quote> for Scalar {
    fn from(row: Quote) -> Self {
        Scalar::from_sequence([Scalar::from(row.0), Scalar::from(row.1)])
    }
}

let field = DataType::from(StructType::from_fields([
    DataType::Int32.required_field("id"),
    DataType::utf8().required_field("symbol"),
])?)
.required_field("quote");
let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
let options = handle.record_options()?.with_field(field);

handle.overwrite_records([Quote(1, "AAPL"), Quote(2, "MSFT")], &options)?;
handle.append_records([Quote(3, "AMD")], &options)?;
handle.merge_records(
    [Quote(2, "MSFT.O")],
    &options.clone().with_merge_by("id")?,
)?;
assert_eq!(
    handle
        .read_arrow_reader(&options)?
        .map(|batch| batch.unwrap().num_rows())
        .sum::<usize>(),
    3,
);
```

```javascript
const assert = require('node:assert/strict')
const fs = require('node:fs')
const os = require('node:os')
const path = require('node:path')
const { IOBase } = require('yggdryl')

const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
const handle = new IOBase(path.join(root, 'trades.arrows'))
// Plain objects are rows; `overwriteRecords` widens them into the Arrow stream.
handle.overwriteRecords([
  { id: 1n, venue: 'XNAS' },
  { id: 2n, venue: null },
])

// Plain objects out, streamed batch by batch ...
assert.deepEqual([...handle.readRecords()].map((row) => row.id), [1n, 2n])

// ... or instances of any class whose constructor takes the plain row.
class Trade {
  constructor(row) {
    Object.assign(this, row)
  }
}
const trades = [...handle.readRecords(Trade)]
assert.ok(trades.every((t) => t instanceof Trade))

// An absent resource yields no records rather than raising.
assert.deepEqual([...new IOBase(path.join(root, 'absent.arrows')).readRecords()], [])

fs.rmSync(root, { recursive: true, force: true })
```

### Column pushdown

The options' field selects and casts in one pass; `select` narrows by name. [Parquet](../media/index.md#parquet) skips the column chunks, [Arrow IPC](../media/index.md#arrow-ipc) skips decode and allocation.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, RecordBatchReader, StringArray};
    use yggdryl::arrow;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, MimeType, StructType};

    let stored = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().required_field("symbol"),
        DataType::utf8().required_field("venue"),
    ])?)
    .required_field("row");
    let arrow_schema = stored.into_arrow_schema()?;

    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec!["AAPL", "MSFT"])),
            Arc::new(StringArray::from(vec!["XNAS", "XNAS"])),
        ],
    )?;

    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let plain = handle.record_options()?;
    handle.overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &plain)?;

    // One of the three columns, declared as this read's schema.
    let wanted = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?).required_field("row");

    let projected = handle.read_arrow_reader(&plain.clone().with_field(wanted))?;
    assert_eq!(projected.schema().fields().len(), 1);
    assert_eq!(projected.map(|batch| batch.unwrap().num_columns()).sum::<usize>(), 1);

    // The resource is unchanged: it still holds all three.
    assert_eq!(handle.read_arrow_field(&plain)?.field_len(), 3);

    // `select` narrows by name instead, in the order the names are given.
    let selecting = plain.clone().with_select("symbol")?;
    let first = handle.read_arrow_reader(&selecting)?.next().unwrap()?;
    assert_eq!(first.num_columns(), 1);
    assert_eq!(first.schema().field(0).name(), "symbol");

    // A column it does not hold cannot be projected out of it, so the encoding
    // reads everything and the cast supplies that column as nulls.
    let invented = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("nowhere"),
    ])?)
    .required_field("row");
    let widened = handle.read_arrow_reader(&plain.with_field(invented))?;
    assert_eq!(widened.schema().fields().len(), 2);
    assert_eq!(widened.schema().field(1).name(), "nowhere");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    stored = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string(), nullable=False),
        pa.field("venue", pa.string(), nullable=False),
    ])
    batch = pa.record_batch(
        {"id": [1, 2], "symbol": ["AAPL", "MSFT"], "venue": ["XNAS", "XNAS"]},
        schema=stored,
    )

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
    handle.overwrite_arrow_batch(batch)

    # One of the three columns, declared as this read's schema.
    options = handle.record_options()
    options.field = pa.schema([pa.field("id", pa.int64(), nullable=False)])

    projected = handle.read_arrow_reader(options=options)
    assert projected.schema.names == ["id"]
    assert projected.read_all().num_columns == 1

    # The resource is unchanged: it still holds all three.
    assert len(handle.read_arrow_field().dtype) == 3

    # A column it does not hold cannot be projected out of it, so the encoding
    # reads everything and the cast supplies that column as nulls.
    options.field = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("nowhere", pa.string()),
    ])
    widened = handle.read_arrow_reader(options=options)
    assert widened.schema.names == ["id", "nowhere"]

    # `select` narrows by name instead, in the order the names are given.
    selecting = handle.record_options()
    selecting.select = ["symbol"]
    assert handle.read_arrow_reader(options=selecting).read_all().column_names == ["symbol"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, Field, IOBase, MimeType, fields } = require('yggdryl')

    const table = new arrow.Table({
      id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
      symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
      venue: arrow.vectorFromArray(['XNAS', 'XNAS'], new arrow.Utf8()),
    })

    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM
    handle.overwriteArrowReader(BatchReader.from(table))

    // One of the three columns, declared as this read's schema.
    const wanted = fields.struct('row', [Field.from('id: int64')], { nullable: false })
    const options = handle.recordOptions()

    const projected = handle.readArrowReader(options.withField(wanted))
    assert.equal(projected.field.dtype.length, 1)
    assert.equal(projected.intoTable().numCols, 1)

    // The resource is unchanged: it still holds all three.
    assert.equal(handle.readArrowField().dtype.length, 3)

    // A column it does not hold cannot be projected out of it, so the encoding
    // reads everything and the cast supplies that column as nulls.
    const invented = fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('nowhere: utf8?')],
      { nullable: false },
    )
    const widened = handle.readArrowReader(options.withField(invented))
    assert.equal(widened.field.dtype.length, 2)

    // `select` narrows by name instead, in the order the names are given.
    const selected = handle.readArrowReader(options.withSelect(['symbol'])).intoTable()
    assert.deepEqual(selected.schema.fields.map((field) => field.name), ['symbol'])
    ```

### Limits

`max_row_size` counts result rows, `max_byte_size` their uncompressed Arrow bytes; both apply last, and a satisfied limit stops pulling.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, RecordBatchReader};
    use yggdryl::arrow;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, MimeType, StructType};

    let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
        .required_field("row");
    let arrow_schema = schema.into_arrow_schema()?;
    let batch = RecordBatch::try_new(
        Arc::clone(&arrow_schema),
        vec![Arc::new(Int64Array::from_iter_values(0..20))],
    )?;

    let mut handle = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    let plain = handle.record_options()?;
    handle.overwrite_arrow_reader(arrow::batch_reader(arrow_schema, [batch]), &plain)?;

    // Ten result rows, exactly: the batch the bound lands inside is sliced.
    let first = handle.read_arrow_reader(&plain.clone().with_max_row_size(10))?;
    assert_eq!(first.map(|batch| batch.unwrap().num_rows()).sum::<usize>(), 10);

    // Zero is a valid ask: the shaped schema answers, and no batch flows.
    let mut none = handle.read_arrow_reader(&plain.clone().with_max_row_size(0))?;
    assert_eq!(none.schema().fields().len(), 1);
    assert!(none.next().is_none());

    // A non-zero byte bound always yields at least one row.
    let narrow = handle.read_arrow_reader(&plain.clone().with_max_byte_size(1))?;
    assert_eq!(narrow.map(|batch| batch.unwrap().num_rows()).sum::<usize>(), 1);

    // A limited write truncates the data the caller offered: three rows land,
    // and what the bound cut off is never pulled from the reader.
    let mut copy = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    copy.overwrite_arrow_reader(
        handle.read_arrow_reader(&plain)?,
        &plain.clone().with_max_row_size(3),
    )?;
    let kept = copy.read_arrow_reader(&plain)?;
    assert_eq!(kept.map(|batch| batch.unwrap().num_rows()).sum::<usize>(), 3);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import IOBase

    handle = IOBase.from_bytes()
    handle.media_type = "application/vnd.apache.arrow.stream"
    handle.overwrite_arrow_table(pa.table({"id": list(range(20))}))

    # Ten result rows, exactly: the batch the bound lands inside is sliced.
    ten = handle.record_options()
    ten.max_row_size = 10
    assert handle.read_arrow_reader(options=ten).read_all().num_rows == 10

    # Zero is a valid ask: the shaped schema answers, and no batch flows.
    zero = handle.record_options()
    zero.max_row_size = 0
    empty = handle.read_arrow_reader(options=zero)
    assert empty.schema.names == ["id"]
    assert empty.read_all().num_rows == 0

    # A non-zero byte bound always yields at least one row.
    one_byte = handle.record_options()
    one_byte.max_byte_size = 1
    assert handle.read_arrow_reader(options=one_byte).read_all().num_rows == 1

    # A limited write truncates the data the caller offered: three rows land,
    # and what the bound cut off is never pulled from the reader.
    copy = IOBase.from_bytes()
    copy.media_type = "application/vnd.apache.arrow.stream"
    first_three = copy.record_options()
    first_three.max_row_size = 3
    copy.overwrite_arrow_reader(handle.read_arrow_reader(), options=first_three)
    assert copy.read_arrow_reader().read_all().num_rows == 3
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, IOBase, MimeType } = require('yggdryl')

    const table = new arrow.Table({
      id: arrow.vectorFromArray(
        Array.from({ length: 20 }, (_, index) => BigInt(index)),
        new arrow.Int64(),
      ),
    })

    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM
    handle.overwriteArrowReader(BatchReader.from(table))
    const options = handle.recordOptions()

    // Ten result rows, exactly: the batch the bound lands inside is sliced.
    assert.equal(handle.readArrowReader(options.withMaxRowSize(10)).intoTable().numRows, 10)

    // Zero is a valid ask: the shaped schema answers, and no batch flows.
    const empty = handle.readArrowReader(options.withMaxRowSize(0))
    assert.equal(empty.field.dtype.length, 1)
    assert.equal(empty.intoTable().numRows, 0)

    // A non-zero byte bound always yields at least one row.
    assert.equal(handle.readArrowReader(options.withMaxByteSize(1)).intoTable().numRows, 1)

    // A limited write truncates the data the caller offered: three rows land,
    // and what the bound cut off is never pulled from the reader.
    const copy = IOBase.fromBytes()
    copy.mediaType = MimeType.ARROW_STREAM
    copy.overwriteArrowReader(handle.readArrowReader(), options.withMaxRowSize(3))
    assert.equal(copy.readArrowReader().intoTable().numRows, 3)
    ```

### Append and merge

Overwrite replaces, append keeps the stored rows, merge updates matching `merge_by` keys and adds the rest. Keys use Arrow's row format: null matches null and the last arrival wins. Merge holds only the stored side in memory.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::arrow;
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::{DataType, StructType, Url};

    let schema = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("symbol"),
    ])?)
    .required_field("row");
    let arrow_schema = schema.clone().into_arrow_schema()?;
    let rows = |ids: Vec<i64>, symbols: Vec<&'static str>| {
        let batch = RecordBatch::try_new(
            Arc::clone(&arrow_schema),
            vec![
                Arc::new(Int64Array::from(ids)),
                Arc::new(StringArray::from(symbols)),
            ],
        )
        .expect("a batch matching the root");
        arrow::batch_reader(batch.schema(), [batch])
    };

    let mut handle =
        Buffer::new().with_media_type(Url::from_str("file:///trades.arrows")?.media_type());
    let options = handle.record_options()?.with_field(schema.clone());

    // Overwrite replaces the resource.
    handle.overwrite_arrow_reader(rows(vec![1, 2], vec!["AAPL", "MSFT"]), &options)?;

    // Appending reads what is there, chains the new batches after it, and rewrites.
    handle.append_arrow_reader(rows(vec![3], vec!["NVDA"]), &options)?;
    let total: usize = handle
        .read_arrow_reader(&options)?
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(total, 3);

    // Merge requires a match key: `2` updates and `9` appends.
    let merging = options.clone().with_merge_by("id")?;
    handle.merge_arrow_reader(rows(vec![2, 9], vec!["MSFT.O", "AMD"]), &merging)?;
    let total: usize = handle
        .read_arrow_reader(&options)?
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(total, 4);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    schema = pa.schema([
        pa.field("id", pa.int64(), nullable=False),
        pa.field("symbol", pa.string()),
    ])
    rows = lambda ids, symbols: pa.record_batch(
        {"id": ids, "symbol": symbols}, schema=schema
    )

    handle = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trades.arrows")
    options = handle.record_options()
    options.field = schema

    # No match key: the resource is replaced.
    handle.overwrite_arrow_batch(rows([1, 2], ["AAPL", "MSFT"]), options=options)

    # Appending reads what is there, chains the new batches after it, and rewrites.
    handle.append_arrow_batch(rows([3], ["NVDA"]), options=options)
    assert handle.read_arrow_reader(options=options).read_all().num_rows == 3

    # A match key merges: `2` is already stored and updates, `9` is new and appends.
    merging = handle.record_options()
    merging.field = schema
    merging.merge_by = ["id"]
    handle.merge_arrow_batch(rows([2, 9], ["MSFT.O", "AMD"]), options=merging)
    assert handle.read_arrow_reader(options=options).read_all().num_rows == 4
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, Field, IOBase, MimeType, fields } = require('yggdryl')

    const schema = fields.struct(
      'row',
      [Field.from('id: int64'), Field.from('symbol: utf8?')],
      { nullable: false },
    )
    const rows = (ids, symbols) =>
      BatchReader.from(
        new arrow.Table({
          id: arrow.vectorFromArray(ids, new arrow.Int64()),
          symbol: arrow.vectorFromArray(symbols, new arrow.Utf8()),
        }),
      )

    const handle = IOBase.fromBytes()
    handle.mediaType = MimeType.ARROW_STREAM
    const options = handle.recordOptions().withField(schema)

    // No match key: the resource is replaced.
    handle.overwriteArrowReader(rows([1n, 2n], ['AAPL', 'MSFT']), options)

    // Appending reads what is there, chains the new batches after it, and rewrites.
    handle.appendArrowReader(rows([3n], ['NVDA']), options)
    assert.equal(handle.readArrowReader(options).intoTable().numRows, 3)

    // A match key merges: `2` is already stored and updates, `9` is new and appends.
    const merging = options.withMergeBy(['id'])
    handle.mergeArrowReader(rows([2n, 9n], ['MSFT.O', 'AMD']), merging)
    assert.equal(handle.readArrowReader(options).intoTable().numRows, 4)
    ```

### Commit cadence

`commit_row_size` is the one publication boundary of a streamed write, applied after shaping.

| `commit_row_size` | publication |
| --- | --- |
| unset | once, when the source ends |
| `N > 0` | every complete group of `N` rows, then the remainder; a committed prefix survives a later failure |
| `0` | rejected before any input is pulled |

A plain folder publishes each leaf on its own; an Iceberg folder uses its [snapshot commit](../media/index.md#iceberg).

### Absent and unknown

An absent resource reads as no batches; an encoding this build does not implement is named, never guessed.

=== "Rust"

    ```rust
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::holder::Buffer;
    use yggdryl::MimeType;

    // An absent resource holds no batches rather than failing to parse.
    let empty = Buffer::new().with_media_type(MimeType::ARROW_STREAM.into());
    assert_eq!(
        empty.read_arrow_reader(&empty.record_options()?)?.count(),
        0
    );

    // An encoding this build does not implement is named rather than guessed.
    let csv = Buffer::new().with_media_type(MimeType::CSV.into());
    let message = csv.record_options().unwrap_err().to_string();
    assert!(message.contains("text/csv"), "{message}");
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pytest

    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp())

    # An absent resource holds no batches rather than failing to parse.
    empty = IOBase(root / "absent.arrows")
    assert empty.read_arrow_reader().read_all().num_rows == 0

    # An encoding this build does not implement is named rather than guessed.
    csv = IOBase(root / "trades.csv")
    with pytest.raises(ValueError, match="text/csv"):
        csv.record_options()
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase, MimeType } = require('yggdryl')

    // An absent resource holds no batches rather than failing to parse.
    const empty = IOBase.fromBytes()
    empty.mediaType = MimeType.ARROW_STREAM
    assert.equal([...empty.readArrowReader()].length, 0)

    // An encoding this build does not implement is named rather than guessed.
    const csv = IOBase.fromBytes()
    csv.mediaType = MimeType.CSV
    assert.throws(() => csv.recordOptions(), /text\/csv/)
    ```

### Lazy scans

Python only: `scan_polars` returns a `polars.LazyFrame` and `scan_arrow` a `pyarrow.dataset.Scanner`; a local Parquet leaf scans natively, everything else streams through the native reader.

```python
import pathlib
import tempfile

import pyarrow as pa

from yggdryl import IOBase

target = pathlib.Path(tempfile.mkdtemp()) / "trades.parquet"
IOBase(target).overwrite_arrow_table(pa.table({"symbol": ["AAPL", "MSFT"], "price": [187.23, 402.11]}))

# A local Parquet leaf becomes the real lazy scan - projection and
# predicate pushdown belong to the engine, and the handle publishes its
# bytes at their exact length first so the foreign reader sees a whole file.
lazy = IOBase(target).scan_polars()
assert lazy.select("symbol").head(10).collect().height == 2

# The pyarrow spelling of the same idea, as a dataset Scanner.
scanner = IOBase(target).scan_arrow()
assert scanner.to_table().num_rows == 2

# Anything a foreign scanner cannot mmap - an in-memory buffer, a
# compressed name, an Arrow stream - streams through the native reader
# instead, so both calls answer for every holder.
memory = IOBase.from_bytes()
memory.media_type = "application/vnd.apache.arrow.stream"
memory.overwrite_arrow_table(pa.table({"symbol": ["AAPL"]}))
assert memory.scan_arrow().to_table().num_rows == 1
```

Plain-text records use the same methods; [Plain text](../media/index.md#plain-text) owns their schema.

### Records performance

Write-mode dispatch, 4,096 rows, one local Windows x86_64 release run (Criterion point estimates; regenerate on the deployment host).

| generic dispatcher | overwrite | append | merge |
| --- | ---: | ---: | ---: |
| `write_arrow_reader` | 255 us (16.1M rows/s) | 703 us (5.83M rows/s) | 3.27 ms (1.25M rows/s) |
| `write_arrow_batch` | 92.0 us (44.5M rows/s) | 637 us (6.43M rows/s) | 4.14 ms (989k rows/s) |
| `write_records` | 3.00 ms (1.36M rows/s) | 4.44 ms (922k rows/s) | 8.96 ms (457k rows/s) |

```bash
cargo bench --bench media --features parquet -- io_write_mode_dispatch
```

## Partitions

Listings are lazy iterators: building one costs nothing, entries are sorted, a failure is yielded once at the entry that failed, and a recursive walk holds one cursor per open depth. A glob descends its fixed prefix rather than listing and filtering; the syntax is in [Patterns](../uri/patterns.md).

```text
fn ls(&self, recursive: bool, include_private: bool) -> Listing
fn glob(&self, pattern: &str, include_private: bool) -> Result<Listing>
fn children_where(&self, filters: &[(&str, &str)], include_private: bool) -> Result<Listing>   // leaves whose Hive path spells every pair
fn partitions(&self) -> Vec<(String, String)>                                                  // the pairs the path spells
```

Python listings are `pathlib`-style (`iterdir`, `glob`, `rglob`); JavaScript's are iterables.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::local::LocalFolder;

    let root = LocalFolder::temporary()?.path()?.join("yggdryl-doc-lake");
    let _ = std::fs::remove_dir_all(&root);
    for year in ["2024", "2025"] {
        let leaf = root.join(format!("year={year}")).join("month=01");
        std::fs::create_dir_all(&leaf)?;
        std::fs::write(leaf.join("part-0.parquet"), b"parquet")?;
    }

    let lake = LocalFolder::new(&root)?;

    // A fixed prefix is descended, not listed and filtered.
    assert_eq!(lake.glob("year=2024/**/*.parquet", false)?.count(), 1);
    assert_eq!(lake.glob("**/*.parquet", false)?.count(), 2);

    // Partition filters select the leaves to overwrite or upsert.
    let selected: Vec<_> = lake
        .children_where(&[("year", "2024")], false)?
        .collect::<yggdryl::Result<_>>()?;
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].partitions(), vec![
        ("year".to_owned(), "2024".to_owned()),
        ("month".to_owned(), "01".to_owned()),
    ]);

    let _ = std::fs::remove_dir_all(&root);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp()) / "lake"
    for year in ("2024", "2025"):
        leaf = root / f"year={year}" / "month=01"
        leaf.mkdir(parents=True)
        (leaf / "part-0.parquet").write_bytes(b"parquet")

    lake = IOBase(root)

    assert len(list(lake.glob("year=2024/**/*.parquet"))) == 1
    assert len(list(lake.rglob("*.parquet"))) == 2

    selected = list(lake.children_where({"year": "2024"}))
    assert len(selected) == 1
    assert selected[0].partitions == (("year", "2024"), ("month", "01"))
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-')), 'lake')
    for (const year of ['2024', '2025']) {
      const leaf = path.join(root, `year=${year}`, 'month=01')
      fs.mkdirSync(leaf, { recursive: true })
      fs.writeFileSync(path.join(leaf, 'part-0.parquet'), 'parquet')
    }

    const lake = new IOBase(root)

    // A fixed prefix is descended, not listed and filtered.
    assert.equal([...lake.glob('year=2024/**/*.parquet')].length, 1)
    assert.equal([...lake.rglob('*.parquet')].length, 2)

    // Partition filters select the leaves to overwrite or upsert.
    const selected = [...lake.childrenWhere({ year: '2024' })]
    assert.equal(selected.length, 1)
    assert.deepEqual(selected[0].partitions, [
      { column: 'year', value: '2024' },
      { column: 'month', value: '01' },
    ])

    fs.rmSync(root, { recursive: true, force: true })
    ```

### Pruning and filtering

The equalities a `filter` pins (`partition_pairs`) prune leaves by path: a leaf whose path names another value for a filtered column is never decoded, and one that does not name the column stays for the rows to answer. The whole [filter](../expression/filters.md#pushdown) then runs over the rows that survive; a range or `in` list pins no pair, so it prunes no leaf by path.

=== "Rust"

    ```rust
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::MimeType;

    let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?
        .with_filter("year = '2024' and month = '01' and id > 5")?;

    // The equalities the filter pins are read as paths spell them; a folder read
    // through these options lists only the January 2024 leaves, and keeps only
    // their rows the rest of the predicate answers true for.
    assert_eq!(
        options.partition_pairs(),
        [
            ("year".to_owned(), "2024".to_owned()),
            ("month".to_owned(), "01".to_owned()),
        ]
    );
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp()) / "lake"
    IOBase(root / "year=2024" / "month=01" / "trades.arrows").overwrite_arrow_table(
        pa.table({"id": [1, 2]})
    )
    IOBase(root / "year=2024" / "month=02" / "trades.arrows").overwrite_arrow_table(
        pa.table({"id": [3]})
    )

    lake = IOBase(root)
    options = lake.record_options()
    options.filter = "year = '2024' and month = '01'"
    reader = lake.read_arrow_reader(options=options)
    assert reader.read_all().num_rows == 2
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    const lake = path.join(root, 'lake')
    new IOBase(path.join(lake, 'year=2024', 'month=01', 'trades.arrows'))
      .overwriteRecords([{ id: 1n }, { id: 2n }])
    new IOBase(path.join(lake, 'year=2024', 'month=02', 'trades.arrows'))
      .overwriteRecords([{ id: 3n }])

    const handle = new IOBase(lake)
    const options = handle.recordOptions()
    options.filter = "year = '2024' and month = '01'"
    assert.equal(handle.readArrowReader(options).intoTable().numRows, 2)

    fs.rmSync(root, { recursive: true, force: true })
    ```

### Partition columns in the data

Addressing the folder restores the columns its directories spell, with the declared types, and routes each written row to its leaf. Batch by batch: the first batch to reach a leaf performs the operation, later ones append.

=== "Rust"

    ```rust
    use yggdryl::holder::Holder;
    use yggdryl::media::{IORecordOptions, RecordOptions};
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::local::LocalFolder;
    use yggdryl::{DataType, MimeType, StructType};

    let root = LocalFolder::temporary()?.path()?.join("yggdryl-doc-partitioned");
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("year=2024").join("month=01"))?;

    let schema = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("price"),
        DataType::Int32.required_field("year"),
        DataType::utf8().required_field("month"),
    ])?)
    .required_field("row");
    let arrow_schema = schema.clone().into_arrow_schema()?;
    let batch = arrow_array::RecordBatch::try_new(
        std::sync::Arc::clone(&arrow_schema),
        vec![
            std::sync::Arc::new(arrow_array::Int64Array::from(vec![10, 20])),
            std::sync::Arc::new(arrow_array::Int32Array::from(vec![2024, 2024])),
            std::sync::Arc::new(arrow_array::StringArray::from(vec!["01", "01"])),
        ],
    )?;

    // The rows carry every column; the write drops the two the path spells out.
    let mut lake = Holder::folder(&root)?;
    let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?.with_field(schema.clone());
    lake.overwrite_arrow_reader(
        yggdryl::arrow::batch_reader(arrow_schema, [batch]),
        &options,
    )?;

    // Only `price` reached the leaf; the other two are the directory names.
    let leaf = lake.child_by_path("year=2024/month=01/part-0.arrows")?;
    assert_eq!(
        leaf.read_arrow_field(&RecordOptions::for_media_type(leaf.media_type())?)?.field_len(),
        1
    );

    // Reading the folder restores them with their declared types.
    let restored = lake
        .read_arrow_reader(&options)?
        .next()
        .expect("one batch")?;
    assert_eq!(restored.num_columns(), 3);
    assert_eq!(restored.schema().field(1).data_type(), &arrow_schema::DataType::Int32);

    let _ = std::fs::remove_dir_all(&root);
    ```

=== "Python"

    ```python
    import pathlib
    import shutil
    import tempfile

    import pyarrow as pa

    from yggdryl import IOBase, RecordOptions

    root = pathlib.Path(tempfile.mkdtemp())
    (root / "year=2024" / "month=01").mkdir(parents=True)

    schema = pa.schema([
        pa.field("price", pa.int64(), nullable=False),
        pa.field("year", pa.int32(), nullable=False),
        pa.field("month", pa.string(), nullable=False),
    ])
    batch = pa.record_batch(
        {"price": [10, 20], "year": [2024, 2024], "month": ["01", "01"]},
        schema=schema,
    )

    # The rows carry every column; the write drops the two the path spells out.
    lake = IOBase(root)
    options = RecordOptions("part.arrows")
    options.field = schema
    lake.overwrite_arrow_batch(batch, options=options)

    # Only `price` reached the leaf; the other two are the directory names.
    leaf = lake / "year=2024" / "month=01" / "part-0.arrows"
    assert len(leaf.read_arrow_field().dtype) == 1

    # Reading the folder restores them with their declared yggdryl.
    restored = lake.read_arrow_reader(options=options).read_all()
    assert restored.column_names == ["price", "year", "month"]
    assert restored.schema.field("year").type == pa.int32()

    shutil.rmtree(root)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const arrow = require('apache-arrow')
    const { BatchReader, Field, IOBase, MimeType, RecordOptions, fields } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    fs.mkdirSync(path.join(root, 'year=2024', 'month=01'), { recursive: true })

    const schema = fields.struct(
      'row',
      [Field.from('price: int64'), Field.from('year: int32'), Field.from('month: utf8')],
      { nullable: false },
    )
    const table = new arrow.Table({
      price: arrow.vectorFromArray([10n, 20n], new arrow.Int64()),
      year: arrow.vectorFromArray([2024, 2024], new arrow.Int32()),
      month: arrow.vectorFromArray(['01', '01'], new arrow.Utf8()),
    })

    // The rows carry every column; the write drops the two the path spells out.
    const lake = new IOBase(root)
    const options = RecordOptions.forMimeType(MimeType.ARROW_STREAM).withField(schema)
    lake.overwriteArrowReader(BatchReader.from(table), options)

    // Only `price` reached the leaf; the other two are the directory names.
    const leaf = lake.joinpath('year=2024').joinpath('month=01').joinpath('part-0.arrows')
    assert.equal(leaf.readArrowField().dtype.length, 1)

    // Reading the folder restores them with their declared types.
    const restored = lake.readArrowReader(options).intoTable()
    assert.equal(restored.numCols, 3)
    assert.equal(restored.schema.fields[1].type.toString(), 'Int32')
    assert.deepEqual(restored.getChild('month').toArray(), ['01', '01'])

    fs.rmSync(root, { recursive: true, force: true })
    ```

A folder that spells nothing takes its layout from the schema's [partition-marked fields](../types/protocol.md). Rust only.

```rust
use yggdryl::holder::Holder;
use yggdryl::media::{IORecordOptions, RecordOptions};
use yggdryl::{IOBase, IOMedia};
use yggdryl::local::LocalFolder;
use yggdryl::{DataType, MimeType, StructType};

let root = LocalFolder::temporary()?.path()?.join("yggdryl-doc-declared-layout");
let _ = std::fs::remove_dir_all(&root);
std::fs::create_dir_all(&root)?;

// Nothing is on disk, so nothing spells a layout. The schema does.
let schema = DataType::from(StructType::from_fields([
    DataType::Int64.required_field("price"),
    DataType::Int32.required_field("year"),
])?)
.required_field("row")
.with_partition_fields(&["year"])?;
assert_eq!(schema.partition_field_names().collect::<Vec<_>>(), ["year"]);

let arrow_schema = schema.clone().into_arrow_schema()?;
let batch = arrow_array::RecordBatch::try_new(
    std::sync::Arc::clone(&arrow_schema),
    vec![
        std::sync::Arc::new(arrow_array::Int64Array::from(vec![10, 20])),
        std::sync::Arc::new(arrow_array::Int32Array::from(vec![2024, 2024])),
    ],
)?;

let mut lake = Holder::folder(&root)?;
let options = RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?.with_field(schema);
lake.overwrite_arrow_reader(
    yggdryl::arrow::batch_reader(arrow_schema, [batch]),
    &options,
)?;

// The directory came from the declaration, and the leaf stores what the
// path does not carry.
assert!(root.join("year=2024").is_dir());

// Reading it back reports the layout without being told it.
let derived = lake.read_arrow_field(
    &RecordOptions::for_mime_type(&MimeType::ARROW_STREAM)?,
)?;
assert_eq!(derived.partition_field_names().collect::<Vec<_>>(), ["year"]);

let _ = std::fs::remove_dir_all(&root);
```

### Derived partition columns

A column can also be computed from another column of the same rows. The [`PARTITION:`](../types/protocol.md) view declares it: `PARTITION:sources` names the field it reads, `PARTITION:transform` the [expression](../expression/grammar.md) function, identity when absent. `apply_arrow_batch` on the Struct root fills a declared column that is absent or all null and leaves one carrying values alone.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{ArrayRef, Date32Array, Int32Array, RecordBatch};
    use yggdryl::expression::Function;
    use yggdryl::{DataType, StructType};

    let mut year = DataType::Int32.nullable_field("year");
    year.as_partition_mut().set_sources(["event"])?;
    year.as_partition_mut().set_transform(Function::Year)?;
    let root = DataType::from(StructType::from_fields([DataType::date32().required_field("event"), year])?)
        .required_field("row");

    let batch = RecordBatch::try_from_iter([(
        "event",
        Arc::new(Date32Array::from(vec![19_723, 20_089])) as ArrayRef,
    )])?;

    let filled = root.as_transform().apply_arrow_batch(&batch)?;

    assert_eq!(filled.num_columns(), 2);
    assert_eq!(
        filled.column(1).as_ref(),
        &Int32Array::from(vec![2024, 2025]) as &dyn arrow_array::Array,
    );

    // The declaration is one term, which is also what a predicate over the
    // same value binds against.
    assert_eq!(
        root.field_at(1)?.as_partition().term()?.map(|read| read.to_string()),
        Some("year(event)".to_owned()),
    );
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType, Field

    year = Field("year", "int32", nullable=True)
    year.partition.sources = ["event"]
    year.partition.transform = "dayofmonth"

    # A dialect alias resolves on the way in, so one name is stored.
    assert year.partition.transform == "day"

    root = Field(
        "row",
        DataType.from_fields([Field("event", "date32", nullable=False), year]),
        nullable=False,
    )
    batch = pa.record_batch({"event": pa.array([19_723, 20_089], pa.date32())})

    filled = root.partition.apply_arrow_batch(batch)

    assert filled.column_names == ["event", "year"]
    assert filled.column("year").to_pylist() == [1, 1]
    ```

## Call counts

Every derived operation makes the fewest `IOBase` calls it needs: a whole read is one call, and a question already answered is none. On a store each call is a round trip, so `holder::counted::Counted` wraps a handle, forwards every call unchanged and tallies it by name; `rust/tests/iobase_calls.rs` asserts the counts exactly.

```text
Counted::new(handle: H) -> Counted<H>
counted.calls() -> &Arc<Calls>          // shared tally: get(Call), group(Group), total(), snapshot(), reset()
counted.counts() -> CallCounts          // a snapshot; Display renders "read_all_bytes=1"
```

```rust
use std::sync::Arc;

use yggdryl::IOBase;
use yggdryl::holder::Buffer;
use yggdryl::holder::counted::{Call, Counted, Group};

let mut source = Buffer::new();
source.write_all_bytes(&vec![7_u8; 4096])?;

let counted = Counted::new(source);
let calls = Arc::clone(counted.calls());

// A whole read is one call, whatever the value's length.
assert_eq!(counted.read_all_bytes()?.len(), 4096);
assert_eq!(calls.get(Call::ReadAllBytes), 1);
assert_eq!(calls.total(), 1);

// The tally names the call, so a read that became two is legible.
assert_eq!(counted.counts().to_string(), "read_all_bytes=1");

// And groups them, so a scan is read as four numbers rather than thirty-one.
calls.reset();
assert_eq!(counted.size(), 4096);
assert_eq!(calls.group(Group::Metadata), 1);
assert_eq!(calls.group(Group::Read), 0);
```

Put the instrument under the layer being measured, never over it: a cache or media reader built on a `Counted` handle has every storage call counted. `Counted` forwards what a backend implements and deliberately not the derived defaults (`glob`, `partitions`, `copy_into`, `read_scalar`, ...), so the tally shows what those decompose into.

```rust
use std::sync::Arc;

use yggdryl::holder::Buffer;
use yggdryl::holder::buffered::BufferedOptions;
use yggdryl::holder::counted::Counted;
use yggdryl::IOBase;

let mut source = Buffer::new();
source.write_all_bytes(&vec![7_u8; 64 * 1024])?;
let counted = Counted::new(source);
let calls = Arc::clone(counted.calls());
let cached = counted.buffered(BufferedOptions::default());

// The first read fetches the page holding the window, and learns the length
// while it is there.
assert_eq!(cached.read_range_bytes(0, 16)?.len(), 16);
assert_eq!(calls.snapshot().to_string(), "pread=1 size=1");

// Every later read inside that page reaches the handle for nothing at all -
// including the length bound, which the inherited default would re-ask for.
calls.reset();
assert_eq!(cached.read_range_bytes(0, 16)?.len(), 16);
assert!(calls.snapshot().is_empty());
```

A child from `parent` or `child_by_path` is the backend's own, not another `Counted`. [ZIP](#zip) counts itself instead, through `ZipArchive::handle_reads` / `handle_writes`. Requests a backend makes to the network per call are that backend's own counter - see [Object stores](#object-stores).

### Call counts performance

One run of each operation, wall clock beside the calls it makes, over a 4 MiB
value on an in-memory handle. The point of the pairing is that the timings are
the *floor*: memory answers a call in nanoseconds, so a row that costs several
still looks cheap here. Read the count as what the same row would cost in round
trips against a store, where each one is a request.

| Operation | Calls | Time |
| --- | --- | --- |
| whole read | `read_all_bytes=1` | 58 µs |
| footer read | `read_range_bytes=1` | 116 ns |
| stream drain | `pstream_bytes=1` | 39 µs |
| whole digest | `read_digest=1` | 65 µs |
| ranged digest | `read_range_digest=1` | 2.9 µs |
| length | `size=1` | 9.7 ns |
| reader drained to the end | `read_all_bytes=1` | 133 µs |
| coding: whole read | `pstream_bytes=1` | 221 µs |
| coding: decoded length | `pstream_bytes=1` | 116 µs |
| cache: warm ranged read | none | 184 ns |
| listing: recursive over 100 | `ls=1` | 156 µs |
| listing: glob over 100 | `bound_location=2 ls=1` | 221 µs |
| IPC: schema | `pstream_bytes=1 url=1 media_type=1 is_container=1 parent=1` | 214 µs |
| IPC: row count | `pread=8 size=1 media_type=2 is_container=2` | 800 ns |
| Avro: schema | `read_all_bytes=1 media_type=1 is_container=1` | 8.8 µs |
| Avro: row count | `pread=2 size=2 media_type=2 is_container=2` | 5.7 µs |
| Parquet: schema | `read_all_bytes=1 size=1 media_type=1 is_container=1` | 13.8 µs |
| Parquet: row count | `read_range_bytes=2 size=2 media_type=2 is_container=2` | 1.2 µs |
| Parquet: full read | `read_all_bytes=1 size=1 media_type=1 is_container=1` | 1.6 ms |

The IPC row count is the one row whose count grows with the value: it is one
`pread` per message, because the walk exists to skip the bodies a read-ahead
window would transfer. Everything else is flat in the size of the value.

## Buffer

`yggdryl::holder::Buffer` is the in-memory handle every example reaches for. Its allocation doubles, so small appends stay amortized constant; a media type is declared with `with_media_type`, never guessed from a name. Python `IOBase.from_bytes` answers a `holder.Buffer`; JavaScript reaches it through `IOBase.fromBytes`.

```text
Buffer::new() / Buffer::with_capacity(usize) / Buffer::from_bytes(Vec<u8>)
buffer.with_media_type(MediaType) -> Buffer
buffer.as_slice() -> &[u8]         // Rust only
buffer.as_mut_slice() -> &mut [u8] // drops any inferred media type
buffer.into_bytes() -> Vec<u8>
```

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::MimeType;

    let mut handle = Buffer::with_capacity(1_024);
    handle.reserve(4_096)?;
    assert!(handle.capacity() >= 4_096);
    // Reserving changes the allocation, never the length.
    assert_eq!(handle.size(), 0);

    handle.pwrite(0, b"symbol,price\n")?;
    assert_eq!(handle.as_slice(), b"symbol,price\n");

    // A format the bytes cannot identify is declared rather than guessed.
    let csv = Buffer::from_bytes(handle.into_bytes()).with_media_type(MimeType::CSV.into());
    assert_eq!(csv.media_type().base(), &MimeType::CSV);
    ```

=== "Python"

    ```python
    from yggdryl import IOBase
    from yggdryl.holder import Buffer

    handle = IOBase.from_bytes(b"symbol,price\n")
    assert type(handle) is Buffer
    assert handle.read_bytes() == b"symbol,price\n"

    # Raw bytes declare no format, so the media type stays generic.
    assert str(handle.media_type) == "application/octet-stream"
    assert handle.url.scheme == "mem"
    ```

## Local

The local file system as three roles: `LocalPath` a location, `LocalFolder` a directory, `LocalFile` a memory-mapped leaf. The only validated state is the canonical `file:` [`Url`](../uri/index.md). Python has the same three classes; JavaScript reaches them through `IOBase`.

```text
LocalFile::new(path) / LocalFile::create(path)     // create truncates
LocalFolder::new(path) / LocalFolder::from_url(Url)
LocalFolder::temporary() / home() / config()       // none creates; home reads HOME, then USERPROFILE
LocalPath::new(path).as_directory() / as_file()    // state the role before anything exists
```

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::local::{LocalFile, LocalFolder};

    let path = LocalFolder::temporary()?.path()?.join(format!("yggdryl-doc-lead-{}.bin", std::process::id()));

    let mut file = LocalFile::create(&path)?;
    file.write_all_bytes(b"AAPL")?;
    file.flush()?;

    assert_eq!(file.read_all_bytes()?, b"AAPL");

    drop(file);
    let _ = std::fs::remove_file(&path);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl.holder import LocalFile

    path = pathlib.Path(tempfile.mkdtemp()) / "trades.bin"

    leaf = LocalFile(path)
    leaf.write_bytes(b"AAPL")
    leaf.flush()

    assert leaf.read_bytes() == b"AAPL"
    ```

### The three roles

`LocalPath` resolves once and routes every call through that role. The Python constructors commit to a role, so they skip the composition a name declares and address the stored bytes.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::local::{LocalFile, LocalFolder, LocalPath};
    use yggdryl::IOKind;

    let root = LocalFolder::temporary()?.path()?.join(format!("yggdryl-doc-roles-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("nested"))?;
    std::fs::write(root.join("a.bin"), b"a")?;

    // A container: it holds no bytes of its own, only children.
    let folder = LocalFolder::new(&root)?;
    assert_eq!(folder.size(), 0);
    assert_eq!(folder.ls(false, false).count(), 2);

    // A leaf: bytes addressed by offset.
    let leaf = LocalFile::new(root.join("a.bin"))?;
    assert_eq!(leaf.read_all_bytes()?, b"a");

    // A location: it answers by looking at what is actually there.
    assert_eq!(LocalPath::new(&root)?.kind(), IOKind::Directory);
    assert_eq!(LocalPath::new(root.join("a.bin"))?.kind(), IOKind::File);

    let _ = std::fs::remove_dir_all(&root);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl.holder import LocalFile, LocalFolder, LocalPath

    root = pathlib.Path(tempfile.mkdtemp())
    (root / "nested").mkdir()
    (root / "a.bin").write_bytes(b"a")

    # A container: it holds no bytes of its own, only children.
    folder = LocalFolder(root)
    assert folder.size == 0
    assert len(list(folder.ls())) == 2

    # A leaf: bytes addressed by offset.
    leaf = LocalFile(root / "a.bin")
    assert leaf.read_bytes() == b"a"

    # A location: it answers by looking at what is actually there.
    assert LocalPath(root).kind == "directory"
    assert LocalPath(root / "a.bin").kind == "file"
    ```

### Well-known roots

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::local::LocalFolder;

    let temporary = LocalFolder::temporary()?;
    assert!(temporary.is_container());
    assert!(temporary.url().to_string().starts_with("file:"));

    // When a home resolves, the configuration directory is that home joined with `.config`.
    match LocalFolder::home() {
        Ok(home) => assert_eq!(LocalFolder::config()?.path()?, home.path()?.join(".config")),
        Err(error) => assert!(error.is_absent()),
    }
    ```

=== "Python"

    ```python
    from yggdryl.holder import LocalFolder

    temporary = LocalFolder.temporary()
    assert isinstance(temporary, LocalFolder)
    assert str(temporary.url).startswith("file:")

    try:
        home = LocalFolder.home()
    except ValueError as error:
        # An absence names both variables it looked at.
        assert "HOME or USERPROFILE" in str(error)
    else:
        # When a home resolves, the configuration directory is that home joined with `.config`.
        assert LocalFolder.config().url == home.url / ".config"
    ```

### A write decides an undecided location

A byte write settles an `Unknown` location as a file; `as_directory` (Python `mkdir()`) settles it the other way.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::local::{LocalFolder, LocalPath};
    use yggdryl::IOKind;

    let root = LocalFolder::temporary()?.path()?.join(format!("yggdryl-doc-decide-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root)?;

    // Nothing is there, so nothing has decided what it is.
    let mut location = LocalPath::new(root.join("trades.bin"))?;
    assert_eq!(location.kind(), IOKind::Unknown);

    // A byte write settles it: an undecided location becomes a file.
    location.write_all_bytes(b"AAPL")?;
    location.flush()?;
    assert_eq!(location.kind(), IOKind::File);
    assert_eq!(location.read_all_bytes()?, b"AAPL");

    // To settle it the other way, say so before writing.
    let container = LocalPath::new(root.join("day=2026-08-16"))?;
    assert_eq!(container.kind(), IOKind::Unknown);
    container.as_directory()?.create()?;
    assert_eq!(container.kind(), IOKind::Directory);

    drop(location);
    let _ = std::fs::remove_dir_all(&root);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl.holder import LocalFolder, LocalPath

    root = pathlib.Path(tempfile.mkdtemp())

    # Nothing is there, so nothing has decided what it is.
    location = LocalPath(root / "trades.bin")
    assert location.kind == "unknown"

    # A byte write settles it: an undecided location becomes a file.
    location.write_bytes(b"AAPL")
    location.flush()
    assert location.kind == "file"
    assert location.read_bytes() == b"AAPL"

    # To settle it the other way, say so before writing. The container is a
    # different role, so it is a different handle, not the one that made it.
    container = LocalPath(root / "day=2026-08-16").mkdir()
    assert isinstance(container, LocalFolder)
    assert container.kind == "directory"
    ```

### Walking the tree

Listings are sorted, and a recursive one stays out of `.git`, `.venv` and `.DS_Store` entirely; `.` and `..` collapse. A nested child creates its parents on write.

=== "Rust"

    ```rust
    use yggdryl::holder::Holder;
    use yggdryl::IOBase;
    use yggdryl::local::{LocalFile, LocalFolder};

    let root = LocalFolder::temporary()?.path()?.join(format!("yggdryl-doc-walk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    // Constructing, listing, and reading an absent location create nothing.
    let folder = LocalFolder::new(&root)?;
    let absent = LocalFile::new(root.join("sub").join("inner.bin"))?;
    assert!(!folder.exists());
    assert_eq!(folder.ls(true, false).count(), 0);
    assert!(absent.read_all_bytes()?.is_empty());
    assert!(!root.exists());
    drop(absent);
    folder.create()?;

    // A child is a handle; writing through it creates the leaf.
    let mut leaf = folder.child_by_path("trades.arrows")?;
    leaf.write_all_bytes(b"payload")?;
    leaf.flush()?;
    assert!(matches!(leaf, Holder::LocalFile(_)));

    // A nested child creates its parent directory on write.
    let mut nested = folder.child_by_path("sub/inner.bin")?;
    nested.write_all_bytes(b"deep")?;
    nested.flush()?;

    // Listings are sorted, so two runs agree; recursion reaches the nested leaf.
    let names: Vec<String> = folder
        .ls(false, false)
        .map(|entry| {
            let entry = entry?;
            Ok(entry
                .url()
                .and_then(|url| url.file_name())
                .unwrap_or_default()
                .to_owned())
        })
        .collect::<yggdryl::Result<_>>()?;
    assert_eq!(names, ["sub", "trades.arrows"]);
    assert_eq!(folder.ls(true, false).count(), 3);

    // A leaf's parent is the directory holding it.
    let parent = leaf.parent().expect("a file has a parent");
    assert!(parent.is_container());
    assert_eq!(parent.url().unwrap(), folder.url());

    drop(leaf);
    drop(nested);
    let _ = std::fs::remove_dir_all(&root);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl.holder import LocalFile, LocalFolder

    root = pathlib.Path(tempfile.mkdtemp()) / "lake"

    # Constructing, listing, and reading an absent location create nothing.
    folder = LocalFolder(root)
    assert len(list(folder.ls(True))) == 0
    assert LocalFile(root / "sub" / "inner.bin").read_bytes() == b""
    assert not root.exists()
    folder.mkdir()

    # A child is a handle; writing through it creates the leaf.
    leaf = folder / "trades.bin"
    assert isinstance(leaf, LocalFile)
    leaf.write_bytes(b"payload")
    leaf.flush()

    # A nested child creates its parent directory on write. One string descends
    # the whole path; joining segment by segment needs each one to exist first.
    nested = folder / "sub/inner.bin"
    nested.write_bytes(b"deep")
    nested.flush()

    # Listings are sorted, so two runs agree; recursion reaches the nested leaf.
    assert [entry.name for entry in folder.ls()] == ["sub", "trades.bin"]
    assert len(list(folder.ls(True))) == 3

    # A leaf's parent is the directory holding it.
    parent = leaf.parent
    assert isinstance(parent, LocalFolder)
    assert parent.url == folder.url
    ```

`include_private` lists the dot-prefixed entries a listing otherwise skips, and the rule is one accessor on the location, `Url::is_private`.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::local::LocalFolder;
    use yggdryl::Url;

    let root = LocalFolder::temporary()?.path()?.join("yggdryl-doc-private");
    std::fs::create_dir_all(root.join(".git"))?;
    std::fs::write(root.join("trades.arrows"), b"x")?;

    let folder = LocalFolder::new(&root)?;
    assert_eq!(folder.ls(false, false).count(), 1);
    assert_eq!(folder.ls(false, true).count(), 2);

    // The rule is one accessor on the location itself, because every child has one.
    assert!(Url::from_str("file:///project/.git")?.is_private());
    assert!(!Url::from_str("file:///project/trades.arrows")?.is_private());

    std::fs::remove_dir_all(&root)?;
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import Url
    from yggdryl.holder import LocalFolder

    root = pathlib.Path(tempfile.mkdtemp())
    (root / ".git").mkdir()
    (root / "trades.arrows").write_bytes(b"x")

    folder = LocalFolder(root)
    assert len(list(folder.ls())) == 1
    assert len(list(folder.ls(False, True))) == 2

    # The rule is one accessor on the location itself, because every child has one.
    assert Url("file:///project/.git").is_private()
    assert not Url("file:///project/trades.arrows").is_private()
    ```

### The mapping

`size` is logical and `capacity` mapped; appends remap a logarithmic number of times, and `flush` / `close` unmap and publish the logical length. The mapping aliases the file, so another process truncating it raises SIGBUS.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::local::{LocalFile, LocalFolder};

    let path = LocalFolder::temporary()?.path()?.join(format!("yggdryl-doc-growth-{}.bin", std::process::id()));

    let mut file = LocalFile::create(&path)?;
    file.pwrite(0, b"ab")?;
    file.pwrite(5, b"z")?;

    // The gap the offset created is zero-filled.
    assert_eq!(file.read_all_bytes()?, b"ab\0\0\0z");

    // Writing past the mapping remaps at a larger capacity instead of failing.
    let bulk = vec![7_u8; 256 * 1024];
    file.append_bytes(&bulk)?;
    assert_eq!(file.size(), 6 + bulk.len() as u64);
    assert!(file.capacity() >= file.size());

    // Flushing publishes the logical length, so the file is the bytes, not the mapping.
    file.flush()?;
    assert_eq!(std::fs::metadata(&path)?.len(), file.size());

    drop(file);
    let _ = std::fs::remove_file(&path);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl.holder import LocalFile

    path = pathlib.Path(tempfile.mkdtemp()) / "growth.bin"

    leaf = LocalFile(path)
    leaf.pwrite(0, b"ab")
    leaf.pwrite(5, b"z")

    # The gap the offset created is zero-filled.
    assert leaf.read_bytes() == b"ab\0\0\0z"

    # Writing past the mapping remaps at a larger capacity instead of failing.
    bulk = bytes(256 * 1024)
    leaf.append_bytes(bulk)
    assert leaf.size == 6 + len(bulk)

    # Flushing publishes the logical length, so the file is the bytes, not the mapping.
    leaf.flush()
    assert path.stat().st_size == leaf.size
    ```

Copy the bytes into a [`Buffer`](#buffer) when the file may change underneath you; `copy_into` streams in chunks and carries the media type.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::local::{LocalFile, LocalFolder};

    let path = LocalFolder::temporary()?.path()?.join(format!("yggdryl-doc-snapshot-{}.bin", std::process::id()));
    std::fs::write(&path, b"trade")?;

    // The handle - and its mapping - is gone by the time the copy returns.
    let mut snapshot = Buffer::new();
    LocalFile::new(&path)?.copy_into(&mut snapshot)?;

    assert_eq!(snapshot.into_bytes(), b"trade");
    let _ = std::fs::remove_file(&path);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase
    from yggdryl.holder import Buffer, LocalFile

    path = pathlib.Path(tempfile.mkdtemp()) / "snapshot.bin"
    path.write_bytes(b"trade")

    # The handle - and its mapping - is unreferenced by the time the copy returns.
    snapshot = IOBase.from_bytes()
    assert isinstance(snapshot, Buffer)
    LocalFile(path).copy_into(snapshot)

    assert snapshot.read_bytes() == b"trade"
    ```

A reader takes a handle, not a path, so one function runs over a file, a `Buffer` or a coded handle.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::local::{LocalFile, LocalFolder};

    fn head(handle: &dyn IOBase) -> yggdryl::Result<Vec<u8>> {
        handle.read_range_bytes(0, 4)
    }

    let path = LocalFolder::temporary()?.path()?.join(format!("yggdryl-doc-agnostic-{}.bin", std::process::id()));

    let mut file = LocalFile::create(&path)?;
    file.write_all_bytes(b"AAPL,100")?;

    let memory = Buffer::from_bytes(b"AAPL,100".to_vec());
    assert_eq!(head(&file)?, b"AAPL");
    assert_eq!(head(&file)?, head(&memory)?);

    drop(file);
    let _ = std::fs::remove_file(&path);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase
    from yggdryl.holder import LocalFile

    def head(handle: IOBase) -> bytes:
        return handle.read_range_bytes(0, 4)

    leaf = LocalFile(pathlib.Path(tempfile.mkdtemp()) / "trades.bin")
    leaf.write_bytes(b"AAPL,100")

    memory = IOBase.from_bytes(b"AAPL,100")
    assert head(leaf) == b"AAPL"
    assert head(leaf) == head(memory)
    ```

## Filesystems

`yggdryl::fs::FileSystem` is the one Arrow-compatible storage seam; `from_fs` binds a filesystem and an opaque path, which is never parsed, decoded or normalized - `bucket/v=a%2Fb.bin` reaches the store literally. `MemoryFileSystem` and `LocalFileSystem` ship as references; Python binds `pyarrow.fs`, JavaScript a synchronous handler protocol.

```text
trait FileSystem: Send + Sync {
    fn type_name(&self) -> &str;
    fn equals(&self, other: &dyn FileSystem) -> bool;
    fn file_info(&self, path: &str) -> Result<FileInfo>;          // absence is FileInfo, not an error
    fn list(&self, selector: &FileSelector) -> FileInfos;
    fn create_dir / delete_dir / delete_dir_contents / delete_root_dir_contents / delete_file
    fn copy_file(&self, source: &str, target: &str) -> Result<()>;  // one native call, no client stream
    fn move_file(&self, source: &str, target: &str) -> Result<()>;
    fn open_input_file / open_input_stream / open_output_stream / open_append_stream
}
FsFile::from_path(Arc<dyn FileSystem>, path, uri: Option<String>) -> Result<FsFile>   // FsFolder, FsPath alike
```

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::IOBase;
    use yggdryl::fs::{
        FileSystem, FsFile, FsFolder, MemoryFileSystem, OutputMetadata,
    };

    let filesystem: Arc<dyn FileSystem> = Arc::new(MemoryFileSystem::new());
    // The bucket is a directory to the filesystem, and an object write needs it.
    FsFolder::from_path(Arc::clone(&filesystem), "bucket", None)?.create(false)?;
    let file = FsFile::from_path(
        filesystem,
        "bucket/v=a%2Fb.bin",
        Some("s3://bucket/v=a%2Fb.bin".to_owned()),
    )?;
    let metadata = OutputMetadata::from_entries([
        ("content-type", "application/octet-stream"),
    ]);
    let mut output = file.open_output_stream(Some(&metadata))?;
    output.write(b"literal")?;
    output.close()?;

    assert_eq!(file.read_all_bytes()?, b"literal");
    ```

=== "Python"

    ```python
    import pyarrow.fs as pafs
    from yggdryl import IOBase

    filesystem = pafs._MockFileSystem()
    # The bucket is a directory to the filesystem, and an object write needs it.
    IOBase.from_fs(filesystem, "bucket").create_dir()

    handle = IOBase.from_fs(
        filesystem,
        "bucket/v=a%2Fb.bin",
        uri="s3://bucket/v=a%2Fb.bin",
    )

    with handle.open_output_stream(
        compression=None,
        metadata={"content-type": "application/octet-stream"},
    ) as output:
        output.write(b"literal")

    with handle.open_input_file() as source:
        source.seek(2)
        assert source.read(3) == b"ter"

    assert handle.filesystem is filesystem
    assert handle.path == "bucket/v=a%2Fb.bin"
    assert handle.bound_uri == "s3://bucket/v=a%2Fb.bin"
    assert handle.masked_uri == "s3://bucket/v=a%2Fb.bin"
    ```

### The three foreign roles

`from_fs` answers the role and composition the name declares, and every composition keeps the bound location. `FsPath`, `FsFile` and `FsFolder` commit to a role and address the stored bytes.

=== "Python"

    ```python
    import pyarrow.fs as pafs
    from yggdryl import IOBase
    from yggdryl.holder import FsFolder, FsPath
    from yggdryl.media import Text

    filesystem = pafs._MockFileSystem()
    folder = IOBase.from_fs(filesystem, "bucket").create_dir()
    assert isinstance(folder, FsFolder)

    handle = IOBase.from_fs(
        filesystem,
        "bucket/trades.txt.gz",
        uri="s3://bucket/trades.txt.gz",
    )
    assert isinstance(handle, Text)
    assert repr(handle) == 'Text(Gzip(FsPath("s3://bucket/trades.txt.gz")))'

    handle.write_bytes(b"AAPL,10\n")
    assert handle.read_bytes() == b"AAPL,10\n"

    assert handle.filesystem is filesystem
    assert handle.path == "bucket/trades.txt.gz"
    assert handle.bound_uri == "s3://bucket/trades.txt.gz"

    stored = FsPath(filesystem, "bucket/trades.txt.gz")
    assert stored.read_bytes()[:2] == b"\x1f\x8b"
    assert handle.info().size == stored.size
    ```

### Resolve a URI once

`from_uri` is the only boundary where a URI chooses and configures a filesystem. An `options` mapping overrides the query and is forwarded to `pyarrow.fs.S3FileSystem`.

=== "Python"

    ```python
    from yggdryl import IOBase

    local = IOBase.from_uri("file:///tmp/events.bin")

    s3 = IOBase.from_uri(
        "s3://bucket/v=a%2Fb"
        "?endpoint_override=minio%3A9000"
        "&scheme=http"
        "&region=eu-west-1",
        options={
            "anonymous": True,
            "force_path_style": True,
        },
    )

    assert s3.path == "bucket/v=a%2Fb"
    ```

| Input | Filesystem configuration | Bound path |
| --- | --- | --- |
| `s3://bucket/key`, `s3a://`, `s3n://` | default S3 | `bucket/key` |
| `s3://key:secret@bucket/key` | credentials from user information | `bucket/key` |
| `s3://key:secret@minio:9000/bucket/key` | endpoint `minio:9000` | `bucket/key` |
| `s3://bucket/key?endpoint_override=minio%3A9000&scheme=http&region=eu-west-1` | explicit endpoint, transport, region | `bucket/key` |
| `s3://bucket.s3.eu-west-1.amazonaws.com/key` | virtual addressing, inferred region | `bucket/key` |

`bound_uri` may carry secrets; errors, logs and `repr` use `masked_uri`. `same_location` needs filesystem equality plus byte-for-byte path equality.

### Streams, copy and move

| Open | Capability |
| --- | --- |
| `open_input_file` | random read, read-at, seek, tell, close |
| `open_input_stream` | sequential read, tell, close |
| `open_output_stream` | truncating streamed write, flush, tell, close |
| `open_append_stream` | streamed append, flush, tell, close |

Each open retains one backend stream. On one filesystem a copy or move is exactly one native call; across two, a copy streams in bounded chunks and publishes only after success.

=== "Python"

    ```python
    import pyarrow.fs as pafs
    from yggdryl import IOBase

    filesystem = pafs._MockFileSystem()
    IOBase.from_fs(filesystem, "bucket").create_dir()
    IOBase.from_fs(filesystem, "archive").create_dir()

    source = IOBase.from_fs(filesystem, "bucket/events.bin")
    source.write_bytes(b"literal")
    target = IOBase.from_fs(filesystem, "bucket/events.copy.bin")
    archive = IOBase.from_fs(filesystem, "archive/events.bin")

    copied = source.copy_into(target)
    moved = target.move_into(archive)

    assert copied == source.info().size
    assert moved.same_location(archive)
    assert not target.exists()
    ```

### JavaScript handler protocol

A handler is any object answering sixteen synchronous calls - `typeName`, `equals`, `normalizePath`, `fileInfo`, `list`, the five deletes and `createDir`, `copyFile`, `move`, and the four opens. Sizes, offsets and nanosecond mtimes are `bigint`.

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase } = require('yggdryl')

    // The whole protocol over one Map.
    const files = new Map()

    const handler = {
      typeName: 'memory',
      equals: (other) => other === handler,
      normalizePath: (path) => path,
      fileInfo: (path) =>
        files.has(path)
          ? { path, kind: 'file', size: BigInt(files.get(path).length) }
          : { path, kind: 'not-found' },
      *list(selector) {
        for (const [path, bytes] of files) {
          if (path.startsWith(selector.baseDir)) {
            yield { path, kind: 'file', size: BigInt(bytes.length) }
          }
        }
      },
      createDir() {},
      deleteDir: (path) => void files.delete(path),
      deleteDirContents: () => files.clear(),
      deleteRootDirContents: () => files.clear(),
      deleteFile: (path) => void files.delete(path),
      copyFile: (source, target) => void files.set(target, files.get(source)),
      move(source, target) {
        files.set(target, files.get(source))
        files.delete(source)
      },
      openInputFile(path) {
        const bytes = files.get(path) ?? new Uint8Array()
        let at = 0n
        return {
          closed: false,
          readAt: (offset, length) =>
            bytes.slice(Number(offset), Number(offset) + Number(length)),
          seek(offset) {
            at = offset
            return at
          },
          read(length) {
            const out = bytes.slice(Number(at), Number(at) + Number(length))
            at += BigInt(out.length)
            return out
          },
          tell: () => at,
          close() {},
        }
      },
      openInputStream: (path) => handler.openInputFile(path),
      openOutputStream(path) {
        const chunks = []
        return {
          closed: false,
          write(bytes) {
            chunks.push(bytes)
            return BigInt(bytes.length)
          },
          tell: () => BigInt(chunks.reduce((sum, one) => sum + one.length, 0)),
          flush() {},
          close: () => void files.set(path, Buffer.concat(chunks)),
        }
      },
      openAppendStream: (path) => handler.openOutputStream(path),
    }

    const source = IOBase.fromFs(handler, 'bucket/v=a%2Fb.bin')
    source.writeBytes(Buffer.from('trades'))

    // The filesystem receives that literal name; `%2F` never becomes a slash.
    assert.deepEqual([...files.keys()], ['bucket/v=a%2Fb.bin'])

    const target = IOBase.fromFs(handler, 'archive/v=a%2Fb.bin')
    source.copyInto(target)
    assert.equal(Buffer.from(target.readBytes()).toString(), 'trades')
    ```

### Filesystems performance

The benchmark times the wrapper against direct PyArrow, local, or native local operations. Conformance tests, not timing, are the evidence for streaming correctness.

The native local parity harness runs custom medians only in an optimized,
explicit benchmark invocation. All-target tests perform one untimed read,
write and copy per path over the same 64 MiB fixture, retaining the size and
content checks without warm-up, samples or throughput thresholds.

| gate | value |
| --- | ---: |
| benchmark payload | at least 64 MiB |
| chunk sizes | identical on both legs |
| warm-up | before every median |
| native parity failure | wrapper throughput below 75% of direct throughput |
| retained streams | one per transfer |
| retained payload | bounded |
| same-filesystem copy or move | zero stream operations |
| native recursive listing | one directory at a time |

```bash
cargo bench --bench holder --features parquet -- fs_bytes
cargo bench --bench holder --features parquet -- fs_record
cargo bench --bench holder --features parquet -- fs_listing
python/.venv/bin/python python/benchmarks/holder.py --min-time 0.2 --repeat 7
npm run --prefix node bench:holder
```

## Object stores

`S3Path`, `S3Folder` and `S3File` reach Amazon S3 (and every store answering its API), Google Cloud Storage and Azure Blob Storage through each store's REST API over synchronous HTTP/1.1 - no SDK, no async runtime. Behind the non-default `s3` feature. The scheme picks the store: `s3`/`s3a`/`s3n`, `gs`/`gcs`, `az`/`abfs`/`abfss`/`wasb`/`wasbs`, and a handle reports the spelling it was handed.

```text
s3::file(url) -> Result<S3File>                 // s3::folder, s3::located (a Holder) alike
s3::file_with(url, S3Options) -> Result<S3File>
s3::file_at(Provider, container, key)           // a raw key, not a URL; folder_at, path_at alike
S3Options::from_properties(pairs)               // PyIceberg s3.*/gcs.*/adls.*, PyArrow, env names
S3File::stats() -> StatsSnapshot                // the requests that actually went out
```

=== "Rust"

    ```{ .rust .no_run }
    use yggdryl::IOBase;
    use yggdryl::s3;

    // Credentials, tokens, region, and endpoint resolve the way each store's
    // own tools resolve them, and nothing is read until the first request.
    let mut part = s3::file("s3://trades/lake/year=2026/part.parquet")?;

    // A footer read transfers the footer, not the file.
    let footer = part.read_range_bytes(part.size().saturating_sub(8), 8)?;
    assert_eq!(footer.len(), 8);

    // The same call against the other two stores.
    let google = s3::file("gs://trades/lake/year=2026/part.parquet")?;
    let azure = s3::file("abfss://lake@trades.dfs.core.windows.net/part.parquet")?;
    assert_eq!(google.key(), "lake/year=2026/part.parquet");
    assert_eq!(azure.key(), "part.parquet");
    ```

=== "Python"

    ```{ .python .ignore }
    from yggdryl import IOBase
    from yggdryl.holder import S3File

    # The same class, the same methods: the scheme picks the backend.
    part = IOBase("s3://trades/lake/year=2026/part.parquet")
    footer = part.read_range_bytes(part.size - 8, 8)
    blob = IOBase("gs://trades/lake/year=2026/part.parquet")

    # Or the role by name - one class for all three stores, with `provider`
    # saying which - and the catalog's properties handed over whole.
    part = S3File(
        "trades",
        "lake/year=2026/part.parquet",
        provider="gs",
        options=catalog.properties,
    )
    ```

=== "JavaScript"

    ```{ .javascript .ignore }
    const { IOBase } = require('yggdryl')

    const part = new IOBase('s3://trades/lake/year=2026/part.parquet')
    const footer = part.readRangeBytes(part.size - 8, 8)
    const blob = new IOBase('abfss://lake@trades.dfs.core.windows.net/part.parquet')
    ```

### What each operation costs

The request count is the contract, asserted by tests.

| operation | Amazon S3 | Google Cloud Storage | Azure Blob Storage |
| --- | --- | --- | --- |
| building a handle, resolving a child, a media type or a partition | none | none | none |
| resolving a `lake/` location | none | none | none |
| resolving any other location | one single-key listing, or two | the same | the same |
| a ranged read | one ranged `GET` | one ranged `GET` with `alt=media` | one `GET` with `x-ms-range` |
| a whole read, stream drain or digest | one `GET` | one `GET` | one `GET` |
| the size of a listed object | none | none | none |
| `size` on a closed handle | one `HEAD`; none while open | one `objects.get`; none while open | one `HEAD`; none while open |
| a whole write | one `PUT` | one `multipart/related` `POST` | one `PUT` |
| a large write | `parts + 2` | `chunks + 1` | `blocks + 1` |
| an append | one `GET` and one write; no `GET` while open | the same | the same |
| a removal | one `DELETE`, no probe | one `objects.delete` | one `DELETE` |
| a listing, one level or a subtree | one request per 1000 entries | the same | the same |
| emptying or removing a prefix | one listing and one bulk delete per 1000 keys | per 100 | per 256 |

A recursive listing is one flat listing, because keys in byte order already are depth-first pre-order. A ranged read learns the length from `Content-Range`, and `S3File::with_known_size` takes one a manifest already stated, which is how an [Iceberg](../media/index.md#iceberg) scan reads each data file with one `GET`.

| Iceberg operation | requests |
| --- | ---: |
| create | 5 |
| append, one partition | 9 |
| append, three partitions | 12 |
| upsert into one partition of three | 14 |
| open | 2 |
| full scan, four files in two manifests | 7 |
| pruned scan, one file | 3 |
| projected scan | 7 |

### Naming an object

`file` and `folder` take a URL; `file_at` and `folder_at` take the store's raw key and the `Provider`. A prefix always ends in the delimiter.

```rust
use yggdryl::s3::{self, Provider};

let handle = s3::file_at(Provider::Aws, "trades", "lake/a b/part.parquet")?;
assert_eq!(handle.key(), "lake/a b/part.parquet");
assert_eq!(handle.url().to_string(), "s3://trades/lake/a%20b/part.parquet");

// A prefix always ends in the delimiter, whether or not the caller wrote one.
let lake = s3::folder_at(Provider::Google, "trades", "lake")?;
assert_eq!(lake.prefix(), "lake/");
assert_eq!(lake.url().to_string(), "gs://trades/lake");
```

```rust
use yggdryl::s3;
use yggdryl::IOBase;

let hadoop = s3::file("s3a://trades/lake/part.parquet")?;
assert_eq!(hadoop.bucket(), "trades");
assert_eq!(hadoop.key(), "lake/part.parquet");
assert_eq!(hadoop.url().to_string(), "s3a://trades/lake/part.parquet");

// A child keeps the spelling its parent was named with.
let child = s3::folder("s3n://trades/lake/")?.child_by_path("part.parquet")?;
assert_eq!(
    child.url().map(ToString::to_string),
    Some("s3n://trades/lake/part.parquet".to_owned())
);

// Google's two names, and Azure's container attached to its account host.
let google = s3::file("gcs://trades/lake/part.parquet")?;
assert_eq!(google.bucket(), "trades");

let azure = s3::file("abfss://lake@trades.dfs.core.windows.net/part.parquet")?;
assert_eq!(azure.bucket(), "lake");
assert_eq!(azure.key(), "part.parquet");
```

### Configuration

Each unset knob is found in the URL, then the environment, then the store's own files, then a default; `with_environment(false)` leaves only explicit values and the URL. The environment is swept under `AWS_`, `GOOGLE_`, `AZURE_` and `YGGDRYL_`, or any prefix added, and an explicit value always wins. The variables the AWS tools read for themselves - `AWS_PROFILE`, `AWS_ACCESS_KEY_ID`, `AWS_REGION`, `AWS_ENDPOINT_URL_S3` and the rest - are not swept: they are the [session's](#aws-identity), read with the precedence those tools give them.

```rust
use yggdryl::s3::{Credentials, S3Options};

let options = S3Options::default()
    .with_endpoint("http://localhost:9000")
    .with_credentials(Credentials::new("minioadmin", "minioadmin"))
    .with_region("us-east-1");
assert_eq!(options.endpoint(), Some("http://localhost:9000"));

// The options record what the caller asked for; the store that answers is what
// clamps it, because the three floors differ - 5 MiB on S3, a multiple of
// 256 KiB on Google, a block on Azure.
assert_eq!(S3Options::default().with_part_size(1).part_size(), 1);
```

```rust
use yggdryl::s3::S3Options;

// The defaults: one prefix per store, plus this crate's own.
assert_eq!(
    S3Options::default().environment_prefixes(),
    [
        "AWS_".to_owned(),
        "GOOGLE_".to_owned(),
        "AZURE_".to_owned(),
        "YGGDRYL_".to_owned()
    ]
);

// A deployment that spells its configuration its own way gets every knob -
// `TRADING_ENDPOINT`, `TRADING_SSE_TYPE`, `TRADING_ROLE_ARN` - rather than the
// handful someone remembered to wire up.
let options = S3Options::default().with_environment_prefix("TRADING_");
assert_eq!(options.environment_prefixes().len(), 5);

// Explicit wins, which is what makes the order above a fact rather than a
// special case per knob: a knob left at its default takes the ambient answer,
// one the caller set keeps theirs.
let ambient = S3Options::from_properties([("region", "us-east-1"), ("max_attempts", "9")])?;
let explicit = S3Options::default().with_region("eu-west-1").under(&ambient);
assert_eq!(explicit.region(), Some("eu-west-1"));
assert_eq!(explicit.max_attempts(), 9);
```

```rust
use yggdryl::s3::S3Options;

// Hand it the catalog's properties whole; what is not about a store is
// ignored, because most of a catalog's properties are not.
let options = S3Options::from_properties([
    ("warehouse", "s3://trades/lake"),
    ("token", "a catalog bearer token, which is not a session token"),
    ("s3.endpoint", "http://localhost:9000"),
    ("s3.access-key-id", "minioadmin"),
    ("s3.secret-access-key", "minioadmin"),
    ("s3.force-virtual-addressing", "false"),
    ("s3.request-timeout", "30"),
])?;
assert_eq!(options.endpoint(), Some("http://localhost:9000"));
assert_eq!(options.path_style(), Some(true));

// A value that will not parse is heard here rather than at the store.
assert!(S3Options::from_properties([("s3.request-timeout", "soon")]).is_err());
// And a knob this client cannot honor is refused rather than dropped, so
// nothing a caller asked for silently does not happen.
assert!(S3Options::from_properties([("s3.signer.uri", "https://signer")]).is_err());

// One catalog can hold all three stores' properties at once, because a
// warehouse on one cloud and a backup on another is an ordinary thing to
// configure. Each name reaches its own store's options and no other's.
let options = S3Options::from_properties([
    ("gcs.project-id", "trading-analytics"),
    ("gcs.oauth2.token", "ya29.a0AfH6"),
    ("adls.account-name", "trades"),
    ("adls.account-key", "a2V5"),
])?;
assert_eq!(options.google().project(), Some("trading-analytics"));
assert_eq!(options.azure().account(), Some("trades"));
```

Names match loosely - case, `-`, `_` and `.` are one, and a store or tool prefix is dropped - so `s3.access-key-id`, `AWS_ACCESS_KEY_ID` and `access_key` are one knob. A value that will not parse, or a knob the client cannot honor, is refused rather than dropped.

| knob | this crate | PyIceberg | PyArrow |
| --- | --- | --- | --- |
| endpoint | `endpoint` | `s3.endpoint`, `gcs.service.host`, `adls.endpoint` | `endpoint_override`, `scheme` |
| region | `region` | `s3.region` | `region` |
| credentials | `access_key_id`, `secret_access_key`, `session_token` | `s3.access-key-id`, `s3.secret-access-key` | `access_key`, `secret_key` |
| anonymous | `anonymous` | | `anonymous` |
| addressing | `path_style` | `s3.force-virtual-addressing` | `force_virtual_addressing` |
| timeouts | `timeout`, `connect_timeout` | `s3.request-timeout`, `s3.connect-timeout` | `request_timeout`, `connect_timeout` |
| proxy | `proxy` | `s3.proxy-uri` | `proxy_options` |
| encryption | `sse_type`, `sse_key`, `sse_md5`, `kms_key_id`, `encryption_scope` | `s3.sse.type`, `s3.sse.key`, `s3.sse.md5` | |
| containers | `allow_container_creation`, `allow_bucket_creation` | | `allow_bucket_creation` |
| uploads | `part_size`, `multipart_threshold` | `s3.multipart.part-size-bytes` | |
| retries | `max_attempts`, `num_retries` | `s3.retry.num-retries` | |
| listing | `list_page_size`, `page_size`, `max_keys`, `max_results` | | |

| store | knob | names |
| --- | --- | --- |
| Amazon S3 | profile, role | `profile`, `role_arn`, `role_session_name`, `external_id`, `mfa_serial`, `source_profile`, `credential_source`, `web_identity_token_file`, `sts_endpoint`, `sts_regional_endpoints` |
| | sign-in, files | `sso_start_url`, `sso_region`, `sso_account_id`, `sso_role_name`, `sso_session`, `credential_process`, `config_file`, `shared_credentials_file`, `ca_bundle` |
| | endpoints, metadata | `use_fips_endpoint`, `use_dualstack_endpoint`, `ec2_metadata_disabled`, `ec2_metadata_service_endpoint`, `metadata_service_timeout`, `metadata_service_num_attempts` |
| | signing, class, payer, checksum | `payload_signing`, `storage_class`, `requester_pays`, `checksum_algorithm` (`CRC32` or `SHA256`) |
| Google | project, billing | `project_id`, `gcs.project-id`, `GOOGLE_CLOUD_PROJECT`, `user_project`, `quota_project_id` |
| | identity | `credentials_file`, `GOOGLE_APPLICATION_CREDENTIALS`, `credentials_json`, `access_token`, `impersonate_service_account` |
| | storage class, ACL | `storage_class`, `predefined_acl` |
| Azure | account, signature | `account_name`, `account_key`, `AZURE_STORAGE_ACCOUNT_NAME`, `connection_string`, `sas_token` |
| | identity | `tenant_id`, `client_id`, `client_secret`, `federated_token_file`, `managed_identity` |
| | blob, endpoint | `blob_type` (`block`, `append`, `page`), `access_tier`, `encryption_scope`, `api_version`, `data_lake`, `authority_host` |

Sizes may carry a unit (`8MiB`, `32 MB`); durations are seconds.

### AWS identity

Who the process is to AWS - the profile, the region, the endpoint a service is reached at, and the credential set every request signs with - is one `Session` in `yggdryl::aws`, resolved the way the AWS tools resolve it and shared by every handle built on it. `S3Options::with_session` hands one over; an explicit credential pair or `with_anonymous` on the options still wins, and `with_environment(false)` seals it. Behind the `aws` feature, which `s3` implies.

```text
Session::new()                                   // states nothing; resolves lazily, once, and caches
  .with_profile(name).with_region(region)        // else AWS_DEFAULT_PROFILE, AWS_PROFILE, AWS_REGION, the profile's own
  .with_credentials(keys).with_anonymous(true)   // an explicit set, or none
  .with_assumed_role(role).with_sso(sso)         // a role or a sign-in, as a profile would state one
  .with_sso_login(prompt).with_mfa_prompt(ask)   // how a person is asked, when one is needed
  .with_variables(pairs).with_environment(false) // another environment, or none at all
session.credentials(now) -> Result<Option<Credentials>>   // the chain, walked once, refreshed in time
session.profile() / region() / endpoint_url("s3") / sts_endpoint(region) / login()
```

The chain is botocore's, in botocore's order: an explicit set; an explicit role, signed by its `source_profile` or `credential_source`, else by whatever the rest of the chain answers; the environment (`AWS_ACCESS_KEY_ID`, with `AWS_CREDENTIAL_EXPIRATION` and `AWS_ACCOUNT_ID`); a profile that assumes a role through `source_profile`, `credential_source` or a web identity token; IAM Identity Center through the sign-in `aws sso login` cached; the credentials file; a `credential_process`; the configuration file; the legacy boto files; the container endpoint; the instance metadata service. Where botocore fails on the first source that is configured and broken, the session records why and walks on, and refuses only when every source has been asked - naming each. A temporary set is replaced fifteen minutes before it lapses, a refresh that fails keeps the set in hand until it has actually lapsed, and a store answering `ExpiredToken` makes the client walk the chain once more before it gives up. Assumed-role and SSO sessions are read from and written to `~/.aws/cli/cache` and `~/.aws/sso/cache` in the AWS CLI's own shape, so a sign-in or an MFA code the CLI already obtained serves this crate, and the other way round.

```rust
use yggdryl::aws::{AssumedRole, Credentials, Session};
use yggdryl::s3::S3Options;

// A sealed session states everything and consults nothing.
let role = AssumedRole::new("arn:aws:iam::123456789012:role/lake-reader")
    .with_session_name("power-desk")
    .with_external_id("desk-42");
let session = Session::new()
    .with_environment(false)
    .with_region("eu-west-3")
    .with_credentials(Credentials::new("AKIAIOSFODNN7EXAMPLE", "wJalrXUtnFEMI"))
    .with_assumed_role(role);
assert_eq!(
    session.assumed_role().and_then(AssumedRole::session_name),
    Some("power-desk")
);
assert_eq!(session.sts_endpoint("eu-west-3"), "https://sts.eu-west-3.amazonaws.com");

// The options carry it, and every handle built on them shares its answers.
let options = S3Options::default().with_session(session.clone());
assert_eq!(options.session().region().as_deref(), Some("eu-west-3"));
```

```rust
use yggdryl::aws::Session;

// The files, read the way the AWS tools read them: `[profile x]` in the
// configuration file, indented tables, `[sso-session]` and `[services]`.
let session = Session::new()
    .with_environment(false)
    .with_config_text(
        "[profile trading]\nregion = eu-west-3\nsso_session = desk\n\
         sso_account_id = 123456789012\nsso_role_name = LakeReader\n\
         s3 =\n  addressing_style = path\n\n\
         [sso-session desk]\nsso_start_url = https://trading.awsapps.com/start\n\
         sso_region = eu-west-1\n",
    )
    .with_profile("trading");
let profile = session.profile().expect("the profile the text spells");
assert_eq!(profile.region(), Some("eu-west-3"));
assert_eq!(profile.s3("addressing_style"), Some("path"));
let sso = profile.sso()?.expect("a sign-in");
assert_eq!(sso.start_url(), "https://trading.awsapps.com/start");
assert_eq!(sso.session_name(), Some("desk"));
assert_eq!(session.region().as_deref(), Some("eu-west-3"));
```

```rust
use yggdryl::aws::Session;

// Another environment in place of the process's own, and no file read where
// none is.
let session = Session::new()
    .with_variables([
        ("AWS_REGION", "ap-southeast-1"),
        ("AWS_ENDPOINT_URL_S3", "http://localhost:9000/"),
        ("AWS_USE_FIPS_ENDPOINT", "true"),
    ])
    .with_directory("/nonexistent/.aws");
assert_eq!(session.region().as_deref(), Some("ap-southeast-1"));
assert_eq!(session.endpoint_url("s3").as_deref(), Some("http://localhost:9000"));
assert_eq!(session.endpoint_url("sts"), None);
assert_eq!(
    session.sts_endpoint("ap-southeast-1"),
    "https://sts-fips.ap-southeast-1.amazonaws.com"
);
```

```{ .rust .no_run }
use std::sync::Arc;

use yggdryl::IOBase;
use yggdryl::aws::{Session, SsoLogin};
use yggdryl::s3::{self, S3Options};

// A sign-in nobody made yet: the session is told how to show it, performs
// the device flow, files the token where `aws sso login` would, and trades
// it for the profile's role.
let session = Session::new()
    .with_profile("trading")
    .with_sso_login(SsoLogin::Handler(Arc::new(|authorization| {
        eprintln!("{authorization}");
    })));
let part = s3::file_with(
    "s3://trades/lake/part.parquet",
    S3Options::default().with_session(session),
)?;
let _ = part.read_range_bytes(0, 8)?;
```

Google's shape is the same idea on `GoogleOptions`: whatever the credential chain answers signs one call to `iamcredentials`, and the token that call returns is what reaches the store; Azure's is an Entra ID application on `AzureOptions`. Container creation and deletion can be forbidden, refused without a request.

```rust
use yggdryl::s3::{GoogleOptions, S3Options};

let options = S3Options::default().with_google(
    GoogleOptions::default().with_impersonation("lake-reader@trading.iam.gserviceaccount.com"),
);
assert_eq!(
    options.google().impersonation(),
    Some("lake-reader@trading.iam.gserviceaccount.com")
);
```

```rust
use yggdryl::IOBase;
use yggdryl::s3::S3Options;

let options = S3Options::default()
    .with_container_creation(false)
    .with_container_deletion(false);
assert!(!options.container_creation());

// And every write can carry metadata: a name the store defines goes over as
// that header, anything else as user metadata under the store's own prefix -
// `x-amz-meta-`, `x-goog-meta-`, `x-ms-meta-` - and a name the write sets for
// itself is never displaced.
let options = options.with_default_metadata([("desk", "power"), ("cache-control", "no-store")]);
assert_eq!(
    options.default_metadata()[0],
    ("desk".to_owned(), "power".to_owned())
);
```

### Encryption at rest

One `Encryption` value across the three stores, refused once at build time where a store lacks that shape. A customer key is presented again on every read.

| | Amazon S3 | Google Cloud Storage | Azure Blob Storage |
| --- | --- | --- | --- |
| `Default` | the bucket's rule | the bucket's rule | the container's rule |
| `Managed` | `SSE-S3` | already the default | already the default |
| `Kms` | `SSE-KMS`, `aws:kms:dsse` | a CMEK `kmsKeyName` | refused; use a scope |
| `Customer` | `SSE-C`, with the key's MD5 | `x-goog-encryption-key`, with its SHA-256 | `x-ms-encryption-key`, with its SHA-256 |
| `Scope` | refused | refused | `x-ms-encryption-scope` |

```rust
use yggdryl::s3::{CustomerKey, Encryption, KmsKey, Provider, S3Options};

// The bucket's own default, which is what an unset value means.
assert!(S3Options::default().encryption().is_default());

// A named KMS key, an encryption context KMS records and requires again, and
// an S3 Bucket Key - one KMS call per bucket and window rather than per
// object, which is what makes writing a lake of small parts affordable.
let key = KmsKey::new("arn:aws:kms:eu-west-1:123456789012:key/abcd")
    .with_context(r#"{"desk":"power"}"#)
    .with_bucket_key(true);
let options = S3Options::default().with_encryption(Encryption::Kms(key));
assert_eq!(
    options.encryption().write_headers(Provider::Aws)[0],
    ("x-amz-server-side-encryption", "aws:kms".to_owned())
);

// A key the caller holds. It is checked here rather than by the store: 32
// bytes, and the MD5 the AWS tools carry beside it has to be the key's.
let held = CustomerKey::from_base64("AwoRGB8mLTQ7QklQV15lbHN6gYiPlp2kq7K5wMfO1dw=")?;
assert_eq!(held.checksum(), "N+iD0sgzzGlyEXJzCNck5w==");
// The same key, three spellings, and two different digests of it: S3 asks for
// the MD5 and the other two for the SHA-256.
let encryption = Encryption::Customer(held);
assert_eq!(encryption.read_headers(Provider::Aws).len(), 3);
assert_eq!(
    encryption.read_headers(Provider::Google)[0],
    ("x-goog-encryption-algorithm", "AES256".to_owned())
);
assert_eq!(
    encryption.read_headers(Provider::Azure)[0],
    ("x-ms-encryption-algorithm", "AES256".to_owned())
);

// Nothing renders it.
let key = CustomerKey::new(&[7; 32])?;
assert!(format!("{key:?}").contains("<redacted>"));
```

### Retries and failures

A throttle, a 5xx or a failed connection is retried with a full-jitter wait from a token budget of 500 (`StatsSnapshot::retry_tokens`), honoring `Retry-After` up to thirty seconds; a refusal is not retried. A stream cut part way resumes from the byte it stopped at. Absence and conflict stay typed; everything else is `Error::Remote` with the store's own code.

```rust
use yggdryl::Error;

let refused = Error::remote(
    "s3", "GetObject", 403, "AccessDenied", "Access Denied", "s3://trades/lake/part.parquet",
);
assert!(!refused.is_absent());
assert!(!refused.is_conflict());
```

Everything above the three roles - globs, partitions, the page cache, codings, IPC, Parquet, Avro, Iceberg - is inherited from `IOBase`.

```{ .rust .no_run }
use yggdryl::IOBase;
use yggdryl::holder::buffered::BufferedOptions;
use yggdryl::s3;

let mut part = s3::file("s3://trades/lake/part.parquet")?;
// Opening caches the object's metadata - never its bytes - so the questions a
// reader asks along the way stop being round trips. It is what to do before
// wrapping a remote handle in a page cache.
part.open()?;
let cached = part.buffered(BufferedOptions::default());
let _ = cached.read_range_bytes(0, 16)?;

// One partition of a lake, selected by path alone, with no call per file.
for leaf in s3::folder("s3://trades/lake/")?.children_where(&[("year", "2026")], false)? {
    let _ = leaf?;
}
```

### Object stores performance

Both clients run against one in-process store over a real socket, on the same
payloads and keys, so the difference is the two implementations rather than two
networks. Criterion medians on a containerized x86_64 Linux host (Intel Xeon
@ 2.10 GHz, 4 cores, 16 GiB; rustc 1.94.1, thin LTO), `object_store` 0.13.2:

| operation | yggdryl | `object_store` |
| --- | ---: | ---: |
| read 4 MiB whole | 2.16 ms | 2.37 ms |
| read an 8 KiB footer | 62 us | 74 us |
| drain 4 MiB streamed | 1.93 ms | 2.35 ms |
| write 4 MiB, unsigned payload | 9.13 ms | 9.96 ms |
| write 4 MiB, signed payload | 12.31 ms | - |
| first three entries of 2000 | 2.36 ms | 2.80 ms |
| one level of 2000 | 161 us | 180 us |
| whole subtree of 2000 | 7.15 ms | 5.79 ms |

The subtree row is not like for like: this listing yields the container each key
implies as well, which `object_store` does not report at all, so it hands back
half again as many entries for the same pages. The write rows are the same
request under two payload policies, which is why both are given.

A loopback fixture exaggerates fixed cost - a real round trip is 1-20 ms,
against which 60 us is noise - so what these establish is that no per-request
cost is hiding, not that reads are faster in production. The request *count* is
what decides that, and it is held by the accounting tests above.

Regenerate with:

```console
cargo bench --bench holder --features s3 -- object_ --noplot
```

`scripts/check_object_interop.py` (MinIO with `boto3`), `scripts/check_azure_interop.py` (Azurite, which recomputes the Shared Key signature) and `scripts/check_gcs_interop.py` (`fake-gcs-server`, request shape only) check each dialect against the vendor's own client.

```console
python scripts/check_object_interop.py
python scripts/check_azure_interop.py
python scripts/check_gcs_interop.py
```

## Buffered

A page cache over any handle, with the value's first and last pages pinned because both ends carry discovery - magic bytes at the head, a Parquet footer or IPC schema at the tail. Rust `buffered(options)`; Python `buffered(page_size=, max_bytes=, ttl=)` answers a `holder.Buffered` and spends the handle it took; JavaScript `buffered({ pageSize, maxBytes, ttlMs })` returns the same handle.

```text
Buffered::new(handle: H, options: BufferedOptions) -> Buffered<H>
BufferedOptions::default()          // 64 KiB pages, 8 MiB budget, 30 s TTL from last access
    .with_page_size(usize)          // rounded up to a power of two, clamped to 64 ..= 1 GiB
    .with_max_bytes(u64)            // pinned pages included; clamped up to two pages
    .with_ttl(Duration)
buffered.cached_pages() / cached_bytes() / has_cached_page(index)
buffered.clear_cache() / into_handle() -> H
```

=== "Rust"

    ```rust
    use yggdryl::holder::buffered::BufferedOptions;
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;

    let handle = Buffer::from_bytes(b"symbol,price\nAAPL,1\n".to_vec())
        .buffered(BufferedOptions::default());

    assert_eq!(handle.read_range_bytes(0, 6)?, b"symbol");
    assert_eq!(handle.read_range_bytes(13, 4)?, b"AAPL");
    assert_eq!(handle.cached_pages(), 1);
    ```

=== "Python"

    ```python
    from yggdryl import IOBase
    from yggdryl.holder import Buffered

    handle = IOBase.from_bytes(b"symbol,price\nAAPL,1\n")

    # A cache owns the handle it caches, so `handle` is spent from here and
    # the one this answers with is the only usable one.
    cached = handle.buffered(page_size=64, max_bytes=256, ttl=30.0)
    assert type(cached) is Buffered
    assert cached.read_range_bytes(0, 6) == b"symbol"
    assert cached.read_range_bytes(13, 4) == b"AAPL"
    assert cached.cached_pages == 1
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase } = require('yggdryl')

    const handle = IOBase.fromBytes(Buffer.from('symbol,price\nAAPL,1\n'))
    assert.equal(handle.buffered({ pageSize: 64, maxBytes: 256, ttlMs: 30_000 }), handle)
    assert.deepEqual(handle.readRangeBytes(0, 6), Buffer.from('symbol'))
    assert.deepEqual(handle.readRangeBytes(13, 4), Buffer.from('AAPL'))
    ```

### Pages

A miss fetches one aligned page; a hit copies from the held page; a read crossing pages copies each straight into the caller's buffer.

```rust
use yggdryl::holder::buffered::{Buffered, BufferedOptions};
use yggdryl::IOBase;
use yggdryl::holder::Buffer;

// 256-byte pages, so a 1 KiB value is four of them.
let options = BufferedOptions::default().with_page_size(256);
let handle = Buffered::new(Buffer::from_bytes(vec![7_u8; 1_024]), options);

// A read that misses fetches the whole page holding it, aligned.
assert_eq!(handle.read_range_bytes(300, 4)?.len(), 4);
assert_eq!(handle.cached_pages(), 1);
assert_eq!(handle.cached_bytes(), 256);
assert!(handle.has_cached_page(1));

// A read inside that page is a hit: memory, and no second page.
assert_eq!(handle.read_range_bytes(500, 8)?, [7_u8; 8]);
assert_eq!(handle.cached_pages(), 1);

// A read spanning pages assembles from each of them, caching all it crossed.
assert_eq!(handle.read_range_bytes(100, 600)?.len(), 600);
assert_eq!(handle.cached_pages(), 3);

// The page a given offset lives in is arithmetic, not a lookup.
assert_eq!(handle.options().page_index(700), 2);
assert_eq!(handle.options().page_start(2), 512);
```

```rust
use std::time::Duration;

use yggdryl::holder::buffered::BufferedOptions;

let options = BufferedOptions::default();
assert_eq!(options.page_size(), 64 * 1024);
assert_eq!(options.max_bytes(), 8 * 1024 * 1024);
assert_eq!(options.ttl(), Duration::from_secs(30));

// A page size is rounded up to a power of two and clamped to 64 ..= 1 GiB.
assert_eq!(BufferedOptions::default().with_page_size(1_000).page_size(), 1_024);
assert_eq!(BufferedOptions::default().with_page_size(0).page_size(), 64);

// A budget below two pages is clamped up to exactly two, never rejected,
// because the two pinned pages have to fit for the cache to work at all.
let tight = BufferedOptions::default().with_page_size(4_096).with_max_bytes(1);
assert_eq!(tight.max_bytes(), 8_192);

// Raising the page size re-applies that clamp to a budget set earlier.
let grown = BufferedOptions::default()
    .with_page_size(1_024)
    .with_max_bytes(4_096)
    .with_page_size(8_192);
assert_eq!(grown.max_bytes(), 16_384);
```

### Both ends are pinned

Pinned pages are never evicted or expired and count toward the budget, hence the two-page clamp. A moved end releases the old last page.

=== "Rust"

    ```rust
    use yggdryl::holder::buffered::{Buffered, BufferedOptions};
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;

    // Sixteen pages of value, four pages of budget.
    let options = BufferedOptions::default()
        .with_page_size(64)
        .with_max_bytes(4 * 64);
    let handle = Buffered::new(Buffer::from_bytes(vec![1_u8; 16 * 64]), options);

    // The footer first, then the header: the shape a container is opened with.
    handle.read_range_bytes(16 * 64 - 8, 8)?;
    handle.read_range_bytes(0, 8)?;

    // Then a scan of the middle, four times what the budget can hold.
    for page in 1..15 {
        handle.read_range_bytes(page * 64, 8)?;
    }

    // The budget held throughout, the middle was evicted, and both ends stayed.
    assert!(handle.cached_bytes() <= handle.options().max_bytes());
    assert!(handle.has_cached_page(0));
    assert!(handle.has_cached_page(15));
    assert!(!handle.has_cached_page(7));

    // Four pages of value under the same budget: page 3 ends it, so it holds a pin.
    let mut handle = Buffered::new(Buffer::from_bytes(vec![1_u8; 4 * 64]), options);
    assert_eq!(handle.read_all_bytes()?.len(), 4 * 64);
    assert!(handle.has_cached_page(3));

    // A write doubling the value moves the end; page 3 is ordinary again, and a
    // scan under budget pressure now evicts it while page 0 stays.
    handle.pwrite(8 * 64 - 1, b"z")?;
    for page in 4..8 {
        handle.read_range_bytes(page * 64, 8)?;
    }
    handle.read_range_bytes(5 * 64, 8)?;
    handle.read_range_bytes(6 * 64, 8)?;
    assert!(handle.has_cached_page(0));
    assert!(handle.has_cached_page(7));
    assert!(!handle.has_cached_page(3));
    ```

=== "Python"

    ```python
    from yggdryl import IOBase

    # Sixteen pages of value, four pages of budget.
    handle = IOBase.from_bytes(bytes(16 * 64)).buffered(page_size=64, max_bytes=4 * 64)

    # The footer first, then the header: the shape a container is opened with.
    handle.read_range_bytes(16 * 64 - 8, 8)
    handle.read_range_bytes(0, 8)

    # Then a scan of the middle, four times what the budget can hold.
    for page in range(1, 15):
        handle.read_range_bytes(page * 64, 8)

    # The budget held throughout, the middle was evicted, and both ends stayed.
    assert handle.cached_bytes <= 4 * 64
    assert handle.has_cached_page(0)
    assert handle.has_cached_page(15)
    assert not handle.has_cached_page(7)
    ```

### Writes are never stale

`pwrite` writes through and patches or drops the pages it overlapped; `truncate` drops every page a resize could change; `close`, `clear`, `remove`, `handle_mut` and `clear_cache` drop the cache.

=== "Rust"

    ```rust
    use yggdryl::holder::buffered::BufferedOptions;
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;

    let mut handle = Buffer::from_bytes(b"symbol,price\nAAPL,1\n".to_vec())
        .buffered(BufferedOptions::default());
    assert_eq!(handle.read_all_bytes()?.len(), 20);
    assert_eq!(handle.cached_pages(), 1);

    // The read after a write can never see the bytes it replaced.
    handle.pwrite(13, b"MSFT")?;
    assert_eq!(handle.read_range_bytes(13, 4)?, b"MSFT");
    assert_eq!(handle.handle().as_slice()[13..17], *b"MSFT");

    // Truncating drops every page a resize could have changed, both ways.
    handle.truncate(13)?;
    assert_eq!(handle.read_all_bytes()?, b"symbol,price\n");
    handle.truncate(15)?;
    assert_eq!(handle.read_all_bytes()?, b"symbol,price\n\0\0");

    // Closing drops every page and leaves a working handle.
    handle.close()?;
    assert_eq!(handle.cached_pages(), 0);
    assert_eq!(handle.read_range_bytes(0, 4)?, b"symb");
    ```

=== "Python"

    ```python
    from yggdryl import IOBase

    handle = IOBase.from_bytes(bytes(4_096)).buffered()
    assert len(handle.read_bytes()) == 4_096
    assert handle.cached_pages == 1

    # `clear_cache` drops every page and keeps the cache and its options.
    handle.clear_cache()
    assert handle.cached_pages == 0
    assert handle.read_range_bytes(0, 4) == bytes(4)
    ```

### Composition

`Buffered<Coded<_>>` caches decoded pages; `Coded<Buffered<_>>` caches the encoded transport. Wrapping twice reconfigures the one cache, and cursors read through the pages. Over a mapped local file a `pread` is already a `memcpy`, so the cache pays only where a fetch is real.

```rust
use yggdryl::holder::buffered::BufferedOptions;
use yggdryl::gzip::Gzip;
use yggdryl::IOBase;
use yggdryl::holder::Buffer;

let payload = "symbol,price\nAAPL,1\n".repeat(512).into_bytes();
let mut source = Gzip::new(Buffer::new());
source.write_all_bytes(&payload)?;
source.flush()?;
let encoded = source.into_handle()?;

// The cache wraps the coding, so the pages it holds are decoded bytes.
let handle = Gzip::new(encoded).buffered(BufferedOptions::default());
assert_eq!(handle.read_range_bytes(0, 6)?, b"symbol");
assert_eq!(handle.read_range_bytes(13, 4)?, b"AAPL");

// Two reads, one page, one decode.
assert_eq!(handle.cached_pages(), 1);
assert_eq!(handle.size(), payload.len() as u64);
```

=== "Rust"

    ```rust
    use yggdryl::holder::buffered::BufferedOptions;
    use yggdryl::holder::Holder;
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;

    let once = Buffer::from_bytes(vec![5_u8; 128]).buffered(BufferedOptions::default());

    // `Buffered` has an inherent `buffered`, which wins method resolution, so
    // this re-wraps the handle it holds instead of stacking a second cache.
    let twice = once.buffered(BufferedOptions::default().with_page_size(512));
    assert_eq!(twice.options().page_size(), 512);
    assert_eq!(twice.read_range_bytes(0, 4)?, [5, 5, 5, 5]);

    // A holder does the same, so a listing entry can be buffered without care.
    let held = Holder::buffer(Buffer::from_bytes(vec![5_u8; 128]))
        .buffered(BufferedOptions::default())
        .buffered(BufferedOptions::default());
    assert!(matches!(&held, Holder::Buffered(inner) if matches!(inner.handle(), Holder::Buffer(_))));

    // `into_handle` gives the wrapped handle back, cache dropped.
    let inner: Buffer = twice.into_handle();
    assert_eq!(inner.size(), 128);
    ```

=== "Python"

    ```python
    from yggdryl import IOBase
    from yggdryl.holder import Buffer, Buffered

    once = IOBase.from_bytes(bytes(128)).buffered(page_size=64)

    # Re-wrapping reconfigures the one cache and spends `once`, as every
    # conversion does.
    twice = once.buffered(page_size=512)
    assert type(twice) is Buffered
    assert twice.read_range_bytes(0, 4) == bytes(4)

    # `into_handle` descends one layer, cache dropped: the buffer is directly
    # under the cache, never a second cache.
    inner = twice.into_handle()
    assert type(inner) is Buffer
    assert inner.size == 128
    ```

```rust
use std::io::{Read, Seek, SeekFrom};

use yggdryl::holder::buffered::BufferedOptions;
use yggdryl::{IOBase, IOCursor};
use yggdryl::holder::Buffer;

let payload: Vec<u8> = (0..1_024_u32).map(|index| index as u8).collect();
let mut cursor = Buffer::from_bytes(payload)
    .buffered(BufferedOptions::default().with_page_size(256))
    .cursor();

// Sequential reads stream across page boundaries through the cache.
let mut chunk = [0_u8; 300];
cursor.read_exact(&mut chunk)?;
assert_eq!(cursor.tell(), 300);
assert_eq!(chunk[299], 299_u32 as u8);

// A seek to the end lands on the pinned footer page. `IOCursor` and
// `std::io::Seek` both spell `seek`, so this one names the trait it means.
IOCursor::seek(&mut cursor, SeekFrom::End(-4))?;
cursor.read_exact(&mut chunk[..4])?;
assert_eq!(chunk[3], 1_023_u32 as u8);
assert_eq!(cursor.handle().cached_pages(), 3);
```

```rust
use yggdryl::holder::buffered::BufferedOptions;
use yggdryl::IOBase;
use yggdryl::local::{LocalFile, LocalFolder};

let path = LocalFolder::temporary()?.path()?.join(format!("yggdryl-doc-buffered-{}.bin", std::process::id()));
std::fs::write(&path, vec![9_u8; 4_096])?;

let handle = LocalFile::new(&path)?.buffered(BufferedOptions::default().with_page_size(1_024));
assert_eq!(handle.size(), 4_096);
assert_eq!(handle.read_range_bytes(2_000, 8)?, [9_u8; 8]);
assert_eq!(handle.cached_pages(), 1);

// The wrapper is the file for every purpose but the reading.
let bare = LocalFile::new(&path)?;
assert_eq!(handle.url(), bare.url());

drop(handle);
let _ = std::fs::remove_file(&path);
```

### Buffered performance

`io_buffered` runs three workloads over one 16 MiB fixture and every shipped handle: one containerized x86_64 Linux run, group alone, Criterion, 100 samples, medians. Run-to-run spread reaches 15%, so read the multiples, never the percentages.

| workload | shape |
| --- | --- |
| `random` | 512-byte reads in a 4 MiB hot region under the 8 MiB budget |
| `sequential` | one 16 MiB pass in 8 KiB steps, twice the budget, nothing re-read |
| `footer` | both ends, a 12 MiB middle sweep, both ends again |

`buffer`, `file`, and `fs_memory` are already memory; `fs_local` pays an `open`, `seek`, and `read` per `pread` and a `stat` per `size`.

```text
                                     random        sequential        footer
buffer                             10.041 µs        606.58 µs      439.58 µs
file                               28.037 µs        517.80 µs      367.66 µs
buffered  (over file)              75.362 µs      1.9495 ms        507.49 µs
fs_memory                     46.739 µs        734.30 µs      392.83 µs
fs_memory_buffered            77.633 µs      2.0068 ms        503.69 µs
fs_local                     1.0832 ms       2.7574 ms       2.0924 ms
fs_local_buffered             73.668 µs      2.3841 ms        532.18 µs
```

Over a backend that is already memory the cache costs 2.7x (`random`, against `file`); over one that fetches, 4x to 15x.

| Workload | `fs_local` | with the cache | |
| --- | --- | --- | --- |
| `random` (a hot region, re-read) | 1.0832 ms | 73.668 µs | **14.7x faster** |
| `footer` (both ends, big middle) | 2.0924 ms | 532.18 µs | **3.9x faster** |
| `sequential` (one pass, nothing re-read) | 2.7574 ms | 2.3841 ms | **1.2x faster** |

Pinning is asserted as a count: after a 12 MiB middle scan, re-reading both ends costs zero inner fetches.

#### Coded cases

The `coded` cases read a 256 KiB gzip value in 64 reads of 4 KiB.

```text
io_buffered/coded/closed      12.182 ms    20.522 MiB/s
io_buffered/coded/open         5.8867 us   41.474 GiB/s
io_buffered/coded/buffered    10.2080 us   23.916 GiB/s
```

- `closed` restarts the decoder: 64 calls create 64 decoders and discard growing prefixes.
- `open` serves the one decoded snapshot it owns; fastest when holding the whole value is acceptable.
- `buffered` retains only fetched decoded pages and stays within 2x of `open` here.

```bash
cargo bench --bench holder --features parquet -- io_buffered
```

## ZIP

A file system inside one file: the archive is the container, its members are the leaves, and both are ordinary `IOBase` handles. Rust only. `ZipNode` is the root or a member prefix, `ZipLeaf` one member, `ZipPath` whichever is there. Stored, raw DEFLATE and Zstandard members; ZIP64 read and written; encrypted members and paths climbing above the root refused.

```text
zip::mount(handle: Holder) -> Holder             // Holder::zip alike; a .zip leaf stays a leaf until mounted
zip::from_url(&Url) -> Result<Holder>            // the fragment names the member: day.zip#trades/eu.csv
ZipArchive::new(Holder).with_restart_stride(n).try_with_codec(Codec)?.mount() -> ZipNode
archive.write_member(path, &[u8]) / write_member_from(path, impl Read, Codec) -> Result<ZipEntry>
archive.read_member(path) / get_entry(path) / remove_member(path) / create_directory(path)
archive.flush()                                  // writes the central directory once
archive.handle_reads() / handle_writes() -> u64  // every call made into the handle beneath
```

=== "Rust"

    ```rust
    use yggdryl::{holder::{Buffer, Holder}, zip};
    use yggdryl::IOBase;

    let root = zip::mount(Holder::buffer(Buffer::new()));

    let mut trades = root.child_by_path("2024/06/trades.csv")?;
    trades.write_all_bytes(b"symbol,price\nAAPL,187.23\n")?;

    // Members are children, so the ordinary walk reaches them.
    assert_eq!(root.glob("**/*.csv", false)?.count(), 1);
    assert_eq!(
        root.child_by_path("2024/06/trades.csv")?.read_range_bytes(7, 5)?,
        b"price",
    );
    ```

=== "Python"

=== "JavaScript"

### The tree comes from the names

A ZIP has one flat index. A prefix is a directory when a record names it or a member continues it; listing reads no member byte.

```rust
use yggdryl::{holder::{Buffer, Holder}, zip::ZipArchive};
use yggdryl::{IOBase, IOKind};

let root = ZipArchive::new(Holder::buffer(Buffer::new())).mount();
root.child_by_path("2024/06/trades.csv")?.write_all_bytes(b"symbol")?;

// Nothing recorded `2024` or `2024/06`, and both list as directories.
assert_eq!(root.child_by_path("2024")?.kind(), IOKind::Directory);
assert_eq!(root.child_by_path("2024/06")?.kind(), IOKind::Directory);
assert_eq!(root.ls(false, false).count(), 1);
assert_eq!(root.ls(true, false).count(), 3);

// A directory that holds nothing yet is the one case a name cannot imply,
// so it is the one that needs a record of its own.
root.archive().create_directory("2025")?;
assert_eq!(root.child_by_path("2025")?.kind(), IOKind::Directory);
```

### A member is addressed by a fragment

The archive's URL with the member path as the fragment carries both facts, reopens the member through `zip::from_url`, and partitions from both halves.

```rust
use yggdryl::{holder::Holder, zip};
use yggdryl::{IOBase, Url};

let root = zip::mount(Holder::file("/lake/day.zip")?);
let member = root.child_by_path("trades/eu ndx.csv")?;

assert_eq!(
    member.url().expect("a member url"),
    &Url::from_str("file:///lake/day.zip#trades/eu%20ndx.csv")?,
);

// The representation comes from the member's own name, not from `day.zip`.
assert_eq!(*root.child_by_path("logs/app.log.gz")?.media_type().base(), yggdryl::MimeType::PLAIN_TEXT);

// Partitions spelled by the archive's location, then by the member's path.
let partitioned = zip::mount(Holder::file("/lake/region=eu/day.zip")?);
assert_eq!(
    partitioned.child_by_path("year=2024/month=01/part-0.parquet")?.partitions(),
    vec![
        ("region".to_owned(), "eu".to_owned()),
        ("year".to_owned(), "2024".to_owned()),
        ("month".to_owned(), "01".to_owned()),
    ],
);
```

```rust
use yggdryl::{holder::Holder, zip};
use yggdryl::IOBase;

let path = std::env::temp_dir().join(format!("yggdryl-doc-zip-{}.zip", std::process::id()));
let root = zip::mount(Holder::file(&path)?);
root.child_by_path("trades/eu.csv")?.write_all_bytes(b"symbol,price")?;

let member = root.child_by_path("trades/eu.csv")?;
let reopened = zip::from_url(member.url().expect("a member url"))?;
assert_eq!(reopened.read_all_bytes()?, b"symbol,price");

// The same location without a fragment is the archive itself.
let archive = zip::from_url(&yggdryl::Url::from_path(&path)?)?;
assert!(archive.is_container());

let _ = std::fs::remove_file(&path);
```

### Reading a member

A stored member is one positional read of the archive. A compressed member this crate wrote carries a restart map (the `0x5967` extra field, at most 2048 points), so a read decodes from the point at or before the offset: bounded by the stride, not the offset. A member another writer compressed has no map and decodes from its first byte; `open` is the answer for many reads of one. A whole read verifies the CRC-32.

```rust
use yggdryl::{holder::{Buffer, Holder}, zip::ZipArchive};
use yggdryl::{Codec, IOBase};

let payload: Vec<u8> = (0..=255_u8).cycle().take(4_096).collect();
let root = ZipArchive::new(Holder::buffer(Buffer::new())).mount();
root.archive().write_member_with("blob.bin", &payload, Codec::Identity)?;
root.archive().flush()?;

let member = root.as_leaf("blob.bin")?;
assert_eq!(member.read_range_bytes(1_000, 16)?, payload[1_000..1_016]);

// A positional read materialized nothing, and a read past the end is empty.
assert!(!member.opened());
assert!(member.read_range_bytes(9_999, 16)?.is_empty());
```

```rust
use yggdryl::{holder::{Buffer, Holder}, zip::ZipArchive};
use yggdryl::{Codec, IOBase};

let payload: Vec<u8> = b"symbol,price\nAAPL,187.23\n".repeat(2_048);
let root = ZipArchive::new(Holder::buffer(Buffer::new()))
    .with_restart_stride(4_096)
    .mount();
root.archive().write_member("blob.bin", &payload)?;
root.archive().flush()?;

// The map is metadata beside the member, so reading it costs no member byte.
let entry = root.archive().get_entry("blob.bin")?.expect("the member");
assert_eq!(entry.restarts().stride(), 4_096);
assert_eq!(entry.restarts().before(10_000).0, 8_192);

let mut member = root.as_leaf("blob.bin")?;
assert_eq!(member.read_range_bytes(40_000, 32)?, payload[40_000..40_032]);
assert!(!member.opened(), "a positional read retains nothing");

// Opened, every later read answers from the decoded member it now holds.
member.open()?;
assert!(member.opened());
assert_eq!(member.read_range_bytes(0, 4)?, payload[0..4]);
member.close()?;

// A solid member is what a writer that states no points produces: an empty map,
// read from the member's first byte.
let solid = ZipArchive::new(Holder::buffer(Buffer::new()))
    .with_restart_stride(0)
    .mount();
solid.archive().write_member_with("blob.bin", &payload, Codec::Deflate)?;
solid.archive().flush()?;

let entry = solid.archive().get_entry("blob.bin")?.expect("the member");
assert!(entry.restarts().is_empty());
assert_eq!(entry.restarts().before(10_000), (0, 0));
assert_eq!(solid.as_leaf("blob.bin")?.read_range_bytes(10_000, 8)?, payload[10_000..10_008]);
```

### Writing a member

One streaming writer: a member costs one window, not its size, and a header is settled once the sizes are known. A positional write republishes the member whole on `flush`. The coding is the member's own, then `Identity` for an already coded name, then the archive's default.

```rust
use yggdryl::{holder::{Buffer, Holder}, zip::ZipArchive};
use yggdryl::{Codec, IOBase};

let root = ZipArchive::new(Holder::buffer(Buffer::new())).mount();
let source = std::io::Cursor::new(b"symbol,price\nAAPL,187.23\n".repeat(4_096));
let entry = root.archive().write_member_from("trades.csv", source, Codec::Deflate)?;
root.archive().flush()?;

assert_eq!(entry.size(), 102_400);
assert!(entry.compressed_size() < entry.size());
assert_eq!(root.as_leaf("trades.csv")?.read_range_bytes(0, 6)?, b"symbol");

// A positional write republishes the member whole on its flush.
let mut member = root.as_leaf("notes.txt")?;

member.write_all_bytes(b"symbol,price")?;
member.pwrite(0, b"ticker")?;
member.truncate(6)?;
member.flush()?;

assert_eq!(root.archive().read_member("notes.txt")?, b"ticker");

// Writing past the end grows the member and zero-fills the gap.
let mut sparse = root.as_leaf("sparse.bin")?;
sparse.pwrite(4, b"tail")?;
sparse.flush()?;
assert_eq!(root.archive().read_member("sparse.bin")?, b"\0\0\0\0tail");
```

```rust
use yggdryl::{holder::{Buffer, Holder}, zip::ZipArchive};
use yggdryl::{Codec, IOBase, Level};

let root = ZipArchive::new(Holder::buffer(Buffer::new()))
    .try_with_codec(Codec::Zstd)?
    .with_level(Level::new(9))
    .mount();

root.as_leaf("blob.bin")?.write_all_bytes(&vec![1_u8; 512])?;
assert_eq!(root.archive().get_entry("blob.bin")?.expect("the member").codec()?, Codec::Zstd);

// A `.gz` member is already compressed, so it is stored rather than recoded.
root.as_leaf("app.log.gz")?.write_all_bytes(&yggdryl::gzip::dump(b"symbol")?)?;
assert_eq!(root.archive().get_entry("app.log.gz")?.expect("the member").codec()?, Codec::Identity);

// And an explicit coding on the member handle wins over both.
let member = root.as_leaf("forced.bin")?.try_with_codec(Codec::Identity)?;
assert_eq!(member.name(), "forced.bin");
```

### Nesting, publishing, the index

An archive mounted over a member is a resource of its own, `day.zip#inner.zip//trades/eu.csv`. Writes append records and `flush` writes the directory once; removal compacts on the next flush. `ZipEntry` is what the directory says about one member and costs no member byte.

```rust
use yggdryl::{holder::{Buffer, Holder}, zip};
use yggdryl::IOBase;

// Stage an archive, then hold it as one member of another.
let inner = {
    let staged = zip::mount(Holder::buffer(Buffer::new()));
    staged.child_by_path("trades/eu.csv")?.write_all_bytes(b"symbol,price")?;
    staged.read_all_bytes()?
};
let outer = zip::mount(Holder::file("/lake/day.zip")?);
let mut member = outer.child_by_path("inner.zip")?;

// A `.zip` member is stored rather than deflated, so its own members stay
// one positional read of the outer archive away.
let mounted = Holder::zip(outer.child_by_path("inner.zip")?);
assert_eq!(
    mounted.url().expect("the inner archive").to_string(),
    "file:///lake/day.zip#inner.zip//",
);
assert_eq!(
    mounted.child_by_path("trades/eu.csv")?.url().expect("a member url").to_string(),
    "file:///lake/day.zip#inner.zip//trades/eu.csv",
);
let _ = (inner, &mut member);
```

```rust
use yggdryl::{holder::{Buffer, Holder}, zip::ZipArchive};
use yggdryl::{Codec, IOBase};

let root = ZipArchive::new(Holder::buffer(Buffer::new())).mount();
let archive = root.archive();

archive.write_member_with("big.bin", &vec![7_u8; 4_096], Codec::Identity)?;
archive.write_member_with("small.bin", b"kept", Codec::Identity)?;
archive.flush()?;
let before = archive.size();

// Removing compacts, so the removed member's bytes go with the record.
assert!(archive.remove_member("big.bin")?);
archive.flush()?;
assert!(archive.size() < before - 4_000);
assert_eq!(archive.read_member("small.bin")?, b"kept");
```

```rust
use yggdryl::{holder::{Buffer, Holder}, zip::ZipArchive};
use yggdryl::IOBase;

let root = ZipArchive::new(Holder::buffer(Buffer::new())).mount();
root.archive().write_member("trades/eu.csv", b"symbol,price\nAAPL,187.23\n")?;
root.archive().set_comment(b"day one")?;
root.archive().flush()?;

let entry = root.archive().get_entry("trades/eu.csv")?.expect("the member");
assert_eq!(entry.name(), "trades/eu.csv");
assert_eq!(entry.size(), 25);
assert!(entry.compressed_size() > 0);
assert!(!entry.is_directory());
assert!(!entry.is_encrypted());
assert_eq!(root.archive().comment()?, b"day one");
```

### What an operation costs the handle

```rust
use yggdryl::{holder::{Buffer, Holder}, zip::ZipArchive};
use yggdryl::{Codec, IOBase};

let root = ZipArchive::new(Holder::buffer(Buffer::new())).mount();
root.archive().write_member_with("blob.bin", &vec![4_u8; 4_096], Codec::Identity)?;
root.archive().write_member_with("notes.txt", b"symbol", Codec::Identity)?;

// One write per record, then the trailer and the flush behind it. Nothing
// shortens the archive, because it only grew.
assert_eq!(root.archive().handle_writes(), 2);
root.archive().flush()?;
assert_eq!(root.archive().handle_writes(), 4);

// A listing walks the index the archive already holds.
let quiet = root.archive().handle_reads();
assert_eq!(root.ls(true, true).count(), 2);
assert_eq!(root.archive().handle_reads(), quiet);

// The write knew where it put the bytes, so reading them back is one read.
root.as_leaf("blob.bin")?.read_range_bytes(0, 16)?;
assert_eq!(root.archive().handle_reads() - quiet, 1);
```

| operation | reads | writes |
| --- | --- | --- |
| Mount and parse the directory | 2 (3 past the 64 KiB tail) | 0 |
| Listing, glob, `size`, `partitions`, member metadata, restart map | 0 | 0 |
| Positional or whole read of a stored member, warm | 1 | 0 |
| Positional read of a compressed member, warm | 1 per encoded window it decodes | 0 |
| First seek into a mapped member, proving its map | +1 | 0 |
| First read of a member this archive did not write | +1 | 0 |
| Write one member within one window | 0 | 1 |
| Write one member longer than that | 0 | 1 per window + 1 settle |
| Publish | 0 | 2, or 3 when the archive shrank |

A member is a leaf like any other, so the record surface reaches it and the directory above reads as a table.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch};

use yggdryl::{holder::{Buffer, Holder}, zip};
use yggdryl::{DataType, IOBase, IOMedia, StructType};

let schema = DataType::from(StructType::from_fields([DataType::Int64.required_field("id")])?)
    .required_field("row");
let arrow_schema = schema.clone().into_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
    vec![Arc::new(Int64Array::from(vec![7_i64, 9]))],
)?;

let root = zip::mount(Holder::buffer(Buffer::new()));
let mut member = root.child_by_path("trades.arrows")?;
let options = member.record_options()?;
member.overwrite_arrow_reader(yggdryl::arrow::batch_reader(arrow_schema, [batch]), &options)?;

assert_eq!(member.row_size()?, 2);
assert!(root.is_tabular());
```

`python3 scripts/check_zip_interop.py` exchanges archives with Python's `zipfile` in both directions.

### ZIP performance

`io_zip` runs one containerized x86_64 Linux release build, group alone, Criterion, 100 samples, medians. The numbers below are a fresh run on this machine and are not comparable with any published before them: run-to-run spread reaches 10%, so read the multiples, never the percentages.

One 1 MiB member, read positionally 512 bytes at a time, written with the default 64 KiB restart stride:

| leg | | |
| --- | --- | --- |
| `read/stored` | 136.27 ns | the archive's own `pread`, no decode |
| `read/deflate_opened` | 75.48 ns | a copy out of the decoded member `open` holds |
| `read/deflate_onpoint` | 4.8353 µs | the offset is a restart point, so only the answer is decoded |
| `read/deflate_closed` | 7.7590 µs | the offset is mid-unit, so half a stride is decoded and discarded |
| `read/deflate_solid` | 55.270 µs | the same read of a member written with no points |

A mapped member is **7.1x** cheaper to read at an arbitrary offset than a solid one, and **11.4x** on a point. That multiple is the whole feature: what a read decodes is one stride rather than everything before the offset. A stored member is cheaper again by two orders of magnitude, because there is nothing between the caller's buffer and the archive's bytes.

The multiple grows with the distance a scan covers. Sixteen 512-byte reads at *decreasing* offsets across the same member:

| leg | | |
| --- | --- | --- |
| `read/backward/mapped` | 80.024 µs | 5.00 µs a read |
| `read/backward/solid` | 779.86 µs | 48.7 µs a read |

**9.7x**, and it is a quadratic cost becoming a linear one: a solid member pays for the whole prefix on every read, so the gap widens with the member's size, while a mapped one pays for one unit whatever the offset.

Restart points cost size, because a unit begins with no history of the one before it. Measured with the same encoder over 3.7 MB of realistic trade CSV that deflates 3.8:1:

| stride | encoded size | |
| --- | --- | --- |
| solid | 1,012,829 | the baseline |
| 1 MiB | +0.28% | 3 points |
| 256 KiB | +1.31% | 14 points |
| 64 KiB | +5.21% | 59 points, the default |

The default is 64 KiB because it is the page a [`Buffered`](#buffered) handle fetches and the batch a stream hands out, so a page fill begins exactly on a point and decodes nothing it does not return. A caller that wants the bytes back writes with `with_restart_stride(0)`.

Whole-member reads over the same 1 MiB, digest check included:

| leg | | |
| --- | --- | --- |
| `read_all/stored` | 137.23 µs | 7.11 GiB/s |
| `read_all/deflate` | 182.88 µs | 5.16 GiB/s |

Both are dominated by the CRC-32 pass over the decoded bytes, which every whole read performs; the deflated leg also pays for the window each restart drops.

Writing the same 1 MiB member from a reader:

| leg | | |
| --- | --- | --- |
| `write/stream/solid` | 552.50 µs | one window, no points |
| `write/stream/mapped` | 619.17 µs | the same, restarting every 64 KiB |

Restarting costs **12%** of the write, which buys the read multiples above.

Resolving a location and reading through it:

| leg | | |
| --- | --- | --- |
| `read/held_path` | 206.36 ns | a location that already resolved its role |
| `read/through_path` | 918.96 ns | resolving the name again on every read |

A location owns the role it resolved, so the difference is the resolution - a name to canonicalize and a member URL to build - and not the read.

An archive of 2,000 members across ten directories:

| leg | | |
| --- | --- | --- |
| `mount/index` | 1.3356 ms | two handle reads, then parsing 2,000 records |
| `listing/first_entry` | 840.48 µs | the index snapshot a recursive listing walks |
| `listing/drain` | 1.6701 ms | every entry |
| `listing/glob` | 2.7910 ms | `part=03/**/*.csv` over the same tree |
| `write/members` | 41.635 ms | 2,000 members and one directory, ~21 µs each |

Listing reads no member byte at all: the cost is building the name snapshot out of the index. Unlike a directory backend, then, time to first entry is not cheaper than the drain - the index is one map, and a recursive listing walks all of it before yielding.

```bash
cargo bench --bench holder -- io_zip
```
