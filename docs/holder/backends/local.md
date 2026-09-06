# Local

The local file system as three [`IOBase`](../iobase/bytes.md) handles: `Path` a location, `Folder` a directory, `File` a memory-mapped leaf.

## Contract

| | |
| --- | --- |
| Owns | `holder::local::{Path, Folder, File}`, the `IOPath`, `IOFolder`, `IOFile` roles of [Holder](../index.md) |
| Bindings | Rust, and Python as `yggdryl.holder.{Path, File, Folder}`; JavaScript has one `IOBase` and no role classes |
| Validates | Only the canonical `file:` [`Url`](../../uri/index.md), which is a `Folder`'s whole state |
| Lazy | Constructing touches nothing; a write creates the file and every missing parent |
| Roots | `temporary()`, `home()` (`HOME`, then `USERPROFILE`, unset or empty skipped), `config()` (home joined with `.config`); none creates |
| Listings | Sorted; dot-prefixed entries skipped and never descended unless asked |
| Mapping | `size` logical, `capacity` mapped; geometric growth; `flush` and `close` unmap, then set the length |
| Unsafe | The mapping constructor: another process truncating a mapped file raises SIGBUS |

## Use

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::local::{File, Folder};

    let path = Folder::temporary()?.path()?.join(format!("yggdryl-doc-lead-{}.bin", std::process::id()));

    let mut file = File::create(&path)?;
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

    from yggdryl.holder import File

    path = pathlib.Path(tempfile.mkdtemp()) / "trades.bin"

    leaf = File(path)
    leaf.write_bytes(b"AAPL")
    leaf.flush()

    assert leaf.read_bytes() == b"AAPL"
    ```

## The three roles

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::local::{File, Folder, Path};
    use yggdryl::IOKind;

    let root = Folder::temporary()?.path()?.join(format!("yggdryl-doc-roles-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("nested"))?;
    std::fs::write(root.join("a.bin"), b"a")?;

    // A container: it holds no bytes of its own, only children.
    let folder = Folder::new(&root)?;
    assert_eq!(folder.size(), 0);
    assert_eq!(folder.ls(false, false).count(), 2);

    // A leaf: bytes addressed by offset.
    let leaf = File::new(root.join("a.bin"))?;
    assert_eq!(leaf.read_all_bytes()?, b"a");

    // A location: it answers by looking at what is actually there.
    assert_eq!(Path::new(&root)?.kind(), IOKind::Directory);
    assert_eq!(Path::new(root.join("a.bin"))?.kind(), IOKind::File);

    let _ = std::fs::remove_dir_all(&root);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl.holder import File, Folder, Path

    root = pathlib.Path(tempfile.mkdtemp())
    (root / "nested").mkdir()
    (root / "a.bin").write_bytes(b"a")

    # A container: it holds no bytes of its own, only children.
    folder = Folder(root)
    assert folder.size == 0
    assert len(list(folder.ls())) == 2

    # A leaf: bytes addressed by offset.
    leaf = File(root / "a.bin")
    assert leaf.read_bytes() == b"a"

    # A location: it answers by looking at what is actually there.
    assert Path(root).kind == "directory"
    assert Path(root / "a.bin").kind == "file"
    ```

`Path` resolves once and routes every call through that implementation; each role trait pre-implements the rest. The three Python constructors commit to a role, so they skip the composition a name declares and address the stored bytes.

## Well-known roots

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::local::Folder;

    let temporary = Folder::temporary()?;
    assert!(temporary.is_container());
    assert!(temporary.url().to_string().starts_with("file:"));

    // When a home resolves, the configuration directory is that home joined with `.config`.
    match Folder::home() {
        Ok(home) => assert_eq!(Folder::config()?.path()?, home.path()?.join(".config")),
        Err(error) => assert!(error.is_absent()),
    }
    ```

=== "Python"

    ```python
    from yggdryl.holder import Folder

    temporary = Folder.temporary()
    assert isinstance(temporary, Folder)
    assert str(temporary.url).startswith("file:")

    try:
        home = Folder.home()
    except ValueError as error:
        # An absence names both variables it looked at.
        assert "HOME or USERPROFILE" in str(error)
    else:
        # When a home resolves, the configuration directory is that home joined with `.config`.
        assert Folder.config().url == home.url / ".config"
    ```

## Laziness

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::local::{File, Folder};

    let root = Folder::temporary()?.path()?.join(format!("yggdryl-doc-lazy-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    // Constructing touches nothing.
    let folder = Folder::new(&root)?;
    let mut leaf = File::new(root.join("nested").join("trades.bin"))?;
    assert!(!folder.exists());
    assert!(!leaf.exists());

    // Reading something absent yields nothing - and still creates nothing.
    assert_eq!(folder.ls(true, false).count(), 0);
    assert!(leaf.read_all_bytes()?.is_empty());
    assert_eq!(leaf.size(), 0);
    assert!(!root.exists());

    // Writing creates the file and every missing parent.
    leaf.write_all_bytes(b"trade")?;
    leaf.flush()?;
    assert!(leaf.exists());
    assert_eq!(leaf.read_all_bytes()?, b"trade");

    drop(leaf);
    let _ = std::fs::remove_dir_all(&root);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl.holder import File, Folder

    root = pathlib.Path(tempfile.mkdtemp()) / "lake"

    # Constructing touches nothing.
    folder = Folder(root)
    leaf = File(root / "nested" / "trades.bin")
    assert not leaf.exists()
    assert not root.exists()

    # Reading something absent yields nothing - and still creates nothing.
    assert len(list(folder.ls(True))) == 0
    assert leaf.read_bytes() == b""
    assert leaf.size == 0
    assert not root.exists()

    # Writing creates the file and every missing parent.
    leaf.write_bytes(b"trade")
    leaf.flush()
    assert leaf.exists()
    assert leaf.read_bytes() == b"trade"
    ```

## A write decides an undecided location

`as_directory` and `as_file` state the intent before anything exists; Python names the role instead, and `mkdir()` answers the container it made.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::local::{Folder, Path};
    use yggdryl::IOKind;

    let root = Folder::temporary()?.path()?.join(format!("yggdryl-doc-decide-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root)?;

    // Nothing is there, so nothing has decided what it is.
    let mut location = Path::new(root.join("trades.bin"))?;
    assert_eq!(location.kind(), IOKind::Unknown);

    // A byte write settles it: an undecided location becomes a file.
    location.write_all_bytes(b"AAPL")?;
    location.flush()?;
    assert_eq!(location.kind(), IOKind::File);
    assert_eq!(location.read_all_bytes()?, b"AAPL");

    // To settle it the other way, say so before writing.
    let container = Path::new(root.join("day=2026-08-16"))?;
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

    from yggdryl.holder import Folder, Path

    root = pathlib.Path(tempfile.mkdtemp())

    # Nothing is there, so nothing has decided what it is.
    location = Path(root / "trades.bin")
    assert location.kind == "unknown"

    # A byte write settles it: an undecided location becomes a file.
    location.write_bytes(b"AAPL")
    location.flush()
    assert location.kind == "file"
    assert location.read_bytes() == b"AAPL"

    # To settle it the other way, say so before writing. The container is a
    # different role, so it is a different handle, not the one that made it.
    container = Path(root / "day=2026-08-16").mkdir()
    assert isinstance(container, Folder)
    assert container.kind == "directory"
    ```

## Walking the tree

`ls`, `child_by_path`, and `parent` return [`Holder`](../index.md), so one enum walks a tree; `.` and `..` collapse. Python answers the role class instead, from the same descent.

=== "Rust"

    ```rust
    use yggdryl::holder::Holder;
    use yggdryl::IOBase;
    use yggdryl::holder::local::Folder;

    let root = Folder::temporary()?.path()?.join(format!("yggdryl-doc-walk-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);

    let folder = Folder::new(&root)?;
    folder.create()?;

    // A child is a handle; writing through it creates the leaf.
    let mut leaf = folder.child_by_path("trades.arrows")?;
    leaf.write_all_bytes(b"payload")?;
    leaf.flush()?;
    assert!(matches!(leaf, Holder::File(_)));

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

    from yggdryl.holder import File, Folder

    root = pathlib.Path(tempfile.mkdtemp()) / "lake"

    folder = Folder(root)
    folder.mkdir()

    # A child is a handle; writing through it creates the leaf.
    leaf = folder / "trades.bin"
    assert isinstance(leaf, File)
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
    assert isinstance(parent, Folder)
    assert parent.url == folder.url
    ```

## The mapping

Appends remap a logarithmic number of times, so the mapping outruns the bytes written. `capacity` is Rust only; Python sees the logical size the flush publishes.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::local::{File, Folder};

    let path = Folder::temporary()?.path()?.join(format!("yggdryl-doc-growth-{}.bin", std::process::id()));

    let mut file = File::create(&path)?;
    file.pwrite(0, b"trade")?;

    // Writing past the mapping remaps at a larger capacity instead of failing.
    let bulk = vec![7_u8; 256 * 1024];
    file.append_bytes(&bulk)?;
    assert_eq!(file.size(), 5 + bulk.len() as u64);
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

    from yggdryl.holder import File

    path = pathlib.Path(tempfile.mkdtemp()) / "growth.bin"

    leaf = File(path)
    leaf.pwrite(0, b"trade")

    # Writing past the mapping remaps at a larger capacity instead of failing.
    bulk = bytes(256 * 1024)
    leaf.append_bytes(bulk)
    assert leaf.size == 5 + len(bulk)

    # Flushing publishes the logical length, so the file is the bytes, not the mapping.
    leaf.flush()
    assert path.stat().st_size == leaf.size
    ```

Offsets are absolute and a write may start past the end:

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::local::{File, Folder};

    let path = Folder::temporary()?.path()?.join(format!("yggdryl-doc-gap-{}.bin", std::process::id()));

    let mut file = File::create(&path)?;
    file.pwrite(0, b"ab")?;
    file.pwrite(5, b"z")?;

    // The gap the offset created is zero-filled.
    assert_eq!(file.read_all_bytes()?, b"ab\0\0\0z");

    drop(file);
    let _ = std::fs::remove_file(&path);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl.holder import File

    leaf = File(pathlib.Path(tempfile.mkdtemp()) / "gap.bin")
    leaf.pwrite(0, b"ab")
    leaf.pwrite(5, b"z")

    # The gap the offset created is zero-filled.
    assert leaf.read_bytes() == b"ab\0\0\0z"
    ```

## The SIGBUS hazard

The mapping aliases the file's bytes, so copy them into a [`Buffer`](buffer.md) when the file may change underneath you:

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::holder::local::{File, Folder};

    let path = Folder::temporary()?.path()?.join(format!("yggdryl-doc-snapshot-{}.bin", std::process::id()));
    std::fs::write(&path, b"trade")?;

    // The handle - and its mapping - is gone by the time the copy returns.
    let mut snapshot = Buffer::new();
    File::new(&path)?.copy_into(&mut snapshot)?;

    assert_eq!(snapshot.into_bytes(), b"trade");
    let _ = std::fs::remove_file(&path);
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    from yggdryl import IOBase
    from yggdryl.holder import Buffer, File

    path = pathlib.Path(tempfile.mkdtemp()) / "snapshot.bin"
    path.write_bytes(b"trade")

    # The handle - and its mapping - is unreferenced by the time the copy returns.
    snapshot = IOBase.from_bytes()
    assert isinstance(snapshot, Buffer)
    File(path).copy_into(snapshot)

    assert snapshot.read_bytes() == b"trade"
    ```

`copy_into` transfers in chunks and carries the media type onto the target.

## A remote backend is a sibling module

A new backend supplies the same three roles as a sibling module; see [Filesystems](filesystems.md).

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;
    use yggdryl::holder::local::{File, Folder};

    fn head(handle: &dyn IOBase) -> yggdryl::Result<Vec<u8>> {
        handle.read_range_bytes(0, 4)
    }

    let path = Folder::temporary()?.path()?.join(format!("yggdryl-doc-agnostic-{}.bin", std::process::id()));

    let mut file = File::create(&path)?;
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
    from yggdryl.holder import File

    def head(handle: IOBase) -> bytes:
        return handle.read_range_bytes(0, 4)

    leaf = File(pathlib.Path(tempfile.mkdtemp()) / "trades.bin")
    leaf.write_bytes(b"AAPL,100")

    memory = IOBase.from_bytes(b"AAPL,100")
    assert head(leaf) == b"AAPL"
    assert head(leaf) == head(memory)
    ```

[Arrow IPC](../../media/ipc.md) and [Parquet](../../media/parquet.md) take a handle, not a path, so one reader runs over a file, a `Buffer`, or a [coded](../../coding/index.md) handle.

## Private entries

A recursive listing stays out of `.git`, `.venv`, and `.DS_Store` entirely.

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::holder::local::Folder;
    use yggdryl::Url;

    let root = Folder::temporary()?.path()?.join("yggdryl-doc-private");
    std::fs::create_dir_all(root.join(".git"))?;
    std::fs::write(root.join("trades.arrows"), b"x")?;

    let folder = Folder::new(&root)?;
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
    from yggdryl.holder import Folder

    root = pathlib.Path(tempfile.mkdtemp())
    (root / ".git").mkdir()
    (root / "trades.arrows").write_bytes(b"x")

    folder = Folder(root)
    assert len(list(folder.ls())) == 1
    assert len(list(folder.ls(False, True))) == 2

    # The rule is one accessor on the location itself, because every child has one.
    assert Url("file:///project/.git").is_private()
    assert not Url("file:///project/trades.arrows").is_private()
    ```

## Edges

- `home()` or `config()` with neither variable set -> an absence naming both; `Error::is_absent()` is true, Python raises `ValueError`.
- `from_url` with a non-local URL -> error; `new` fails only when the path has no `file:` URL form.
- Read of an absent path -> empty bytes, `size` 0, no entries; nothing created.
- Byte write through a `Path` of kind `Unknown` -> a file; `as_directory()?.create()?` decides otherwise, and Python's `mkdir()` answers the `Folder` it created.
- Python: a named role is the answer to `kind`, so `Folder(absent).kind` is `directory`; `Path` and `File` answer `unknown` until something is there.
- `truncate(0)` on a container -> creates it and its parents; any other size -> error.
- `pwrite` past the end -> a zero-filled gap.
- Drop of a `File` -> publishes the length but cannot fail; `flush` when the write must be known to have landed.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib holder::local::
    cargo test --features "parquet iceberg" -p yggdryl --test holder
    cargo bench --bench holder --features parquet -- io_listing
    cargo bench --bench holder --features parquet -- 'fs_bytes/.*/local_file'
    cargo bench --bench holder --features parquet -- 'fs_listing/.*/local_folder'
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/holder/test_roles.py
    ```
