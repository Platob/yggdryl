# Holder

Every storage implementation is reached through the positional `IOBase` contract.

## Pages

| page | owns |
| --- | --- |
| [Bytes](iobase/bytes.md) | `pread`/`pwrite`, streams, cursors |
| [Records](iobase/records.md) | Arrow batches, pushdown, partitions |
| [Values](iobase/values.md) | bytes, digests, structured scalars |
| [Local](backends/local.md) | `Path`, `Folder`, mapped `File` |
| [Buffer](backends/buffer.md) | in-memory bytes |
| [Buffered](backends/buffered.md) | the page cache |
| [Filesystems](backends/filesystems.md) | Arrow-style `FileSystem` |

## Contract

| key | value |
| --- | --- |
| Owns | one enum over every `IOBase` implementation |
| Variants | one in memory, three local, three foreign, four wrapping another `Holder` |
| `Holder::local` | `Holder::Path`, the unresolved role |
| `buffer` / `folder` / `file` | commit to a role |
| Lazy | construction touches no filesystem; a role resolves only when an operation needs it |
| `into_declared_media` | composes what the name declares, the coding under the record implementation, reading nothing |
| `Holder::open` | promotes with `into_media`, then opens; keeps the schema, footer, and dimension caches |
| Idempotent | `into_text`, `into_coded`, `buffered`, `into_media`, `into_declared_media` never stack |
| Hierarchy | `parent`, `child_by_path`, `ls` return `Holder` |
| Python | one class per variant, composed wherever a handle is described |
| JavaScript | one `IOBase` class over the whole enum, promoted by `open` |

## Use

=== "Rust"

    ```rust
    use yggdryl::holder::Holder;
    use yggdryl::holder::local::Folder;

    // Generic construction records the location without probing its role.
    let directory = Holder::local(Folder::temporary()?.path()?)?;
    assert!(matches!(directory, Holder::Path(_)));

    let missing = Holder::local(Folder::temporary()?.path()?.join("yggdryl-generic-doc.bin"))?;
    assert!(matches!(missing, Holder::Path(_)));
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl.holder import IOBase, Path
    from yggdryl.media import Text

    root = pathlib.Path(tempfile.mkdtemp())

    # The class is read off the name, not the store: neither location exists.
    assert type(IOBase(root / "trades.bin")) is Path
    assert type(IOBase(root / "trades.txt.gz")) is Text
    ```

The enum answers the whole contract, so a caller writes the same calls whatever it holds.

=== "Rust"

    ```rust
    use yggdryl::holder::Holder;
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;

    // A value that could have been any handle. The calls do not change.
    let mut handle = Holder::buffer(Buffer::new());
    handle.write_all_bytes(b"AAPL,1\n")?;

    assert_eq!(handle.read_all_bytes()?, b"AAPL,1\n");
    assert_eq!(handle.kind(), yggdryl::IOKind::Memory);
    ```

=== "Python"

    ```python
    from yggdryl.holder import Buffer, IOBase

    handle = IOBase.from_bytes()
    handle.write_bytes(b"AAPL,1\n")

    # The class names the implementation; the contract is the same one.
    assert type(handle) is Buffer
    assert handle.read_bytes() == b"AAPL,1\n"
    assert handle.kind == "memory"
    ```

## Variants

| variant | holds | Python class |
| --- | --- | --- |
| `Buffer` | an in-memory byte array | `holder.Buffer` |
| `Folder`, `Path`, `File` | a local directory, an undecided local location, a mapped local leaf | `holder.Folder`, `holder.Path`, `holder.File` |
| `FsFolder`, `FsPath`, `FsFile` | the same three on an Arrow `FileSystem` | `holder.FsFolder`, `holder.FsPath`, `holder.FsFile` |
| `Buffered` | any of the others behind the page cache | `holder.Buffered` |
| `Coded` | any of the others, presenting the decoded bytes of a content coding | `coding.Identity`, `Gzip`, `Zlib`, `Zstd` |
| `Text` | any handle retained as plain-text records | `media.Text` |
| `Media` | any handle retained behind its record encoding | `media.Ipc`, `media.Parquet`, `media.Avro` |

The last four own the `Holder` they wrap. `repr` renders that stack outermost first and `into_handle` descends one layer of it.

## What the name declares

`into_declared_media` applies the content coding *and* the record implementation the handle's name declares, without resolving the store. The coding goes underneath, because the record implementation reads the decoded bytes.

| name | composed handle |
| --- | --- |
| `trades.txt.gz` | `Text(Gzip(Path))` |
| `archive.bin.gz` | `Gzip(Path)` |
| `trades.parquet` | `Parquet(Path)` |
| `trades.arrows` | `Ipc(Path)` |
| `trades.avro` | `Avro(Path)` |
| `trades.log` | `Text(Path)` |
| `trades.json` | `Path` |
| `trades` | `Path` |
| `trades.parquet.gz` | `Path` |

Nothing this build reads declares JSON, so that name composes to the location it already was. `trades.parquet.gz` is left alone on purpose: Parquet compresses internally, so the name is one no other Parquet reader could open, and the writer refuses it.

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

    from yggdryl.holder import IOBase, Path
    from yggdryl.media import Text

    location = pathlib.Path(tempfile.mkdtemp()) / "trades.txt.gz"

    handle = IOBase(location)
    assert type(handle) is Text
    assert repr(handle) == f'Text(Gzip(Path("{handle.url}")))'

    # The composed handle reads and writes the decoded value.
    handle.write_bytes(b"AAPL,1\n")
    assert handle.read_bytes() == b"AAPL,1\n"
    assert gzip.decompress(location.read_bytes()) == b"AAPL,1\n"

    # `media_type` is what the handle presents; `codec` is what the store holds.
    assert str(handle.media_type) == "text/plain"
    assert handle.codec == "gzip"

    # `Path` commits to a role and skips the composition: the stored bytes.
    assert Path(location).read_bytes()[:2] == b"\x1f\x8b"
    ```

## Hierarchy

=== "Rust"

    ```rust
    use yggdryl::holder::Holder;
    use yggdryl::IOBase;
    use yggdryl::holder::local::Folder;

    let root = Holder::folder(Folder::temporary()?.path()?)?;
    assert!(root.is_container());

    // A child need not exist. Naming one yields a leaf handle, and nothing is created.
    let leaf = root.child_by_path("yggdryl-generic-child.bin")?;
    assert!(matches!(leaf, Holder::File(_)));
    assert!(!leaf.is_container());
    assert_eq!(leaf.size(), 0);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl.holder import File, Folder
    from yggdryl.media import Ipc

    root = Folder(pathlib.Path(tempfile.mkdtemp()))

    # A child need not exist. Naming one yields a leaf handle, and nothing is created.
    plain = root / "yggdryl-generic-child.bin"
    assert type(plain) is File
    assert plain.size == 0

    # Composition is applied wherever a handle is described, children included.
    records = root.joinpath("yggdryl-generic-child.arrows")
    assert type(records) is Ipc
    assert repr(records) == f'Ipc(File("{records.url}"))'
    assert type(records.parent) is Folder
    ```

## Roles

A role is what a location turns out to be. Rust names three traits for it; Python names the three constructors that commit to one and skip the composition.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::{IOKind, MimeType};
    use yggdryl::holder::local;

    let path = local::Folder::temporary()?.path()?.join("yggdryl-docs-io-folder");
    let _ = std::fs::remove_dir_all(&path);
    let mut folder = local::Folder::new(&path)?;

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

    std::fs::remove_dir_all(&path)?;
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl.holder import Folder

    path = pathlib.Path(tempfile.mkdtemp()) / "yggdryl-docs-io-folder"
    folder = Folder(path)

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
    ```

| trait | declares | pre-implements |
| --- | --- | --- |
| `IOFolder` | `folder_url`, `folder_exists`, `create_folder`, `list_folder` | `folder_pread` (nothing), `folder_pwrite` (refuses), `folder_truncate` (creates on `0`, else errors), `folder_media_type` (`inode/directory`), `folder_kind` (`Directory`) |
| `IOFile` | `file_url`, `file_exists` | `file_ls` (nothing), `file_child_by_path` (refuses), `file_kind` (`File`, `Unknown` when absent) |
| `IOPath` | `path_url`, `is_folder`, `is_file` | `path_exists`, `path_kind` (`Directory`, `File`, or `Unknown`), `path_media_type` (container type, or the one the name implies) |

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::{IOKind};
    use yggdryl::holder::local;

    // A location that arrived from outside answers by looking at what is there.
    let existing = local::Path::new(local::Folder::temporary()?.path()?)?;
    assert_eq!(existing.kind(), IOKind::Directory);

    let undecided = local::Path::new(local::Folder::temporary()?.path()?.join("yggdryl-docs-io-undecided"))?;
    assert_eq!(undecided.kind(), IOKind::Unknown);
    assert!(undecided.read_all_bytes()?.is_empty());

    // A leaf is not a container: it lists nothing and resolves no child.
    let leaf = local::File::new(local::Folder::temporary()?.path()?.join("yggdryl-docs-io-leaf.arrows"))?;
    assert_eq!(leaf.ls(true, false).count(), 0);
    assert!(leaf.child_by_path("nested").is_err());
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl.holder import File, Path

    root = pathlib.Path(tempfile.mkdtemp())

    # A location that arrived from outside answers by looking at what is there.
    assert Path(root).kind == "directory"
    undecided = Path(root / "yggdryl-docs-io-undecided")
    assert undecided.kind == "unknown"
    assert undecided.read_bytes() == b""

    # A leaf is not a container: it lists nothing and resolves no child.
    leaf = File(root / "yggdryl-docs-io-leaf.arrows")
    assert list(leaf.ls()) == []
    try:
        leaf.joinpath("nested")
        raise AssertionError("a leaf resolved a child")
    except ValueError as refused:
        assert "expected a container" in str(refused)
    ```

## Delegating to a wrapped handle

Rust only. Neither binding can add a backend.

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

| spelling | forwards |
| --- | --- |
| `delegate_iobase!(handle)` | storage contract, `open`, `opened`, `close`; no records |
| `delegate_iomedia!(handle)` | dimensions, options, Field and reader reads, typed writes |
| `delegate_iobase!(handle, except_lifecycle)` | omits `clear`, `remove`, `is_atomic`, `is_tabular`, `is_io` |
| `delegate_iobase!(handle: pread, size, ...)` | only the named methods |

## Edges

- `pwrite` on a container -> refused, naming the directory; `truncate(0)` creates it.
- JSON, directories, unknown media -> nothing composes and `Holder::open` leaves them raw.
- A `.parquet.gz` name -> uncomposed, so the Parquet writer still refuses a file no Parquet reader could open.
- An in-memory buffer -> no name to read, so composition asks what it holds.
- Python `buffered`, `into_text`, `into_coded` -> answer the wrapper and spend the handle they took; the spent one raises `ValueError`.
- Python `compress_into` / `decompress_into` on a composed handle -> refused; address the stored bytes with `Path`, `File`, or `from_fs`, or use `copy_into`.
- A method omitted from the list form -> the trait default, so `clear` and `remove` truncate.
- A method the wrapper writes itself -> leave it out of the list; the macro cannot also expand it.
- `Ipc`, `Parquet`, the text handler -> `except_lifecycle`; [Buffered](backends/buffered.md) -> the list form.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib holder::
    cargo test --features "parquet iceberg" -p yggdryl --lib iobase::
    cargo bench --bench holder --features parquet
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/holder/test_roles.py
    python/.venv/bin/python -m pytest python/tests/holder/test_io.py
    ```
