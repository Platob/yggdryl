# Local

The local file system as three [`IOBase`](../iobase/bytes.md) handles: `Path` a location, `Folder` a directory, `File` a memory-mapped leaf.

## Contract

| | |
| --- | --- |
| Owns | `holder::local::{Path, Folder, File}`, the `IOPath`, `IOFolder`, `IOFile` roles of [Holder](../index.md) |
| Bindings | Rust only |
| Validates | Only the canonical `file:` [`Url`](../../uri/index.md), which is a `Folder`'s whole state |
| Lazy | Constructing touches nothing; a write creates the file and every missing parent |
| Roots | `temporary()`, `home()` (`HOME`, then `USERPROFILE`), `config()` (home joined with `.config`); none creates |
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

`Path` resolves once and routes every call through that implementation; each role trait pre-implements the rest.

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

## A write decides an undecided location

`as_directory` and `as_file` state the intent before anything exists.

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

## Walking the tree

`ls`, `child_by_path`, and `parent` return [`Holder`](../index.md), so one enum walks a tree; `.` and `..` collapse.

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

## The mapping

Appends remap a logarithmic number of times, so the mapping outruns the bytes written.

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

[Arrow IPC](../../media/ipc.md) and [Parquet](../../media/parquet.md) take a handle, not a path, so one reader runs over a file, a `Buffer`, or a [coded](../../coding/index.md) handle.

## Private entries

A recursive listing stays out of `.git`, `.venv`, and `.DS_Store` entirely.

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

## Edges

- `home()` or `config()` with neither variable set -> an absence naming both; `Error::is_absent()` is true.
- `from_url` with a non-local URL -> error; `new` fails only when the path has no `file:` URL form.
- Read of an absent path -> empty bytes, `size` 0, no entries; nothing created.
- Byte write through a `Path` of kind `Unknown` -> a file; `as_directory()?.create()?` decides otherwise.
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
