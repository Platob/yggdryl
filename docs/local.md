# Local file system

The file system as three [`IOBase`](io.md) handles: a generic location, a directory, and a memory-mapped file.

!!! note "Rust only"
    The Python and JavaScript packages do not expose this module yet.

=== "Rust"

    ```rust
    use yggdryl::io::IOBase;
    use yggdryl::local::File;

    let path = std::env::temp_dir().join(format!("yggdryl-doc-lead-{}.bin", std::process::id()));

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
    use yggdryl::io::IOBase;
    use yggdryl::local::{File, Folder, Path};
    use yggdryl::IOKind;

    let root = std::env::temp_dir().join(format!("yggdryl-doc-roles-{}", std::process::id()));
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

`Folder` and `File` are the two things a file system has; `Path` is for the case where the
caller does not yet know which it holds - a listing entry, a command-line argument, a
configuration value. `Path` resolves once, keeps the implementation it resolved to, and runs
every [`IOBase`](io.md) call through it.

Each role also implements the matching trait from [`yggdryl::io`](io.md) - `IOFolder`, `IOFile`,
`IOPath` - which pre-implements everything that follows from the role: a container refuses byte
writes, a leaf lists nothing and resolves no children, a location reports its
[`IOKind`](generic.md) by testing the path. A backend supplies each role's few required members -
four for a container, two for a leaf, three for a location - and inherits the rest.

## Laziness

=== "Rust"

    ```rust
    use yggdryl::io::IOBase;
    use yggdryl::local::{File, Folder};

    let root = std::env::temp_dir().join(format!("yggdryl-doc-lazy-{}", std::process::id()));
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

Nothing here validates a path up front, so a handle for a file that will exist later is a normal
value to hold. The one eager check is the conversion to a canonical `file:` [`Url`](uri.md):
`Folder::new`, `File::new`, and `Path::new` fail only when the path cannot be expressed as one,
and `Folder::from_url` and `Path::from_url` fail when the URL is not local.

That URL is the whole state of a `Folder`: the platform path is derived from it on demand, so a
stored path and a stored URL can never disagree.

## A write decides an undecided location

=== "Rust"

    ```rust
    use yggdryl::io::IOBase;
    use yggdryl::local::Path;
    use yggdryl::IOKind;

    let root = std::env::temp_dir().join(format!("yggdryl-doc-decide-{}", std::process::id()));
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

Bytes are what distinguish a leaf from a container, so a `Path` with no resource behind it
becomes a file the moment one writes to it. `as_directory` and `as_file` are the way to state
the intent instead, and both work before anything exists. `Folder::create` makes the directory
and every missing parent; on a container, `truncate(0)` does the same thing, and any other size
is an error, because a container has no bytes to resize.

## Walking the tree

=== "Rust"

    ```rust
    use yggdryl::generic::Holder;
    use yggdryl::io::IOBase;
    use yggdryl::local::Folder;

    let root = std::env::temp_dir().join(format!("yggdryl-doc-walk-{}", std::process::id()));
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

`ls`, `child_by_path`, and `parent` return [`Holder`](generic.md), so one enum walks a whole tree:
subdirectories come back as `Holder::Folder`, files as `Holder::File`. Children resolve through
the URL, so `.` and `..` segments collapse the way they do everywhere else in the crate, and a
name with separators in it is a nested child rather than an error.

`std::fs::read_dir` order is platform-defined; `ls` sorts, so a listing is reproducible.

## The mapping

=== "Rust"

    ```rust
    use yggdryl::io::IOBase;
    use yggdryl::local::File;

    let path = std::env::temp_dir().join(format!("yggdryl-doc-growth-{}.bin", std::process::id()));

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

`size` is the logical length and `capacity` is how much of the file is mapped. Growth is
geometric, so a run of appends remaps a logarithmic number of times rather than once per write,
which leaves the mapped file longer than the bytes written. `flush` and `close` reconcile the
two: they release the mapping and then set the file's length. Releasing first is not an
optimisation, it is required, because Windows refuses to resize a file while a mapped section is
open. Dropping the handle publishes too, but a drop cannot report failure, so call `flush` when
the write must be known to have landed.

Offsets are absolute and a write may start past the end:

=== "Rust"

    ```rust
    use yggdryl::io::IOBase;
    use yggdryl::local::File;

    let path = std::env::temp_dir().join(format!("yggdryl-doc-gap-{}.bin", std::process::id()));

    let mut file = File::create(&path)?;
    file.pwrite(0, b"ab")?;
    file.pwrite(5, b"z")?;

    // The gap the offset created is zero-filled.
    assert_eq!(file.read_all_bytes()?, b"ab\0\0\0z");

    drop(file);
    let _ = std::fs::remove_file(&path);
    ```

## The SIGBUS hazard

The mapping constructor is the only `unsafe` in the crate, and it is `unsafe` for a reason no
wrapper can remove: the mapping aliases the file's bytes. If another process truncates the file
while the mapping is live, touching the lost pages raises SIGBUS - a signal, not an error a
`Result` can carry. `File` documents that instead of pretending it away.

When the file may change underneath you, take the bytes into memory and work on the copy:

=== "Rust"

    ```rust
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::local::File;

    let path = std::env::temp_dir().join(format!("yggdryl-doc-snapshot-{}.bin", std::process::id()));
    std::fs::write(&path, b"trade")?;

    // The handle - and its mapping - is gone by the time the copy returns.
    let mut snapshot = Buffer::new();
    File::new(&path)?.copy_into(&mut snapshot)?;

    assert_eq!(snapshot.into_bytes(), b"trade");
    let _ = std::fs::remove_file(&path);
    ```

`copy_into` transfers in chunks, so neither side is buffered whole, and it carries the source's
media type onto the target.

## A remote backend is a sibling module

S3, GCS, and Azure are the same three ideas - a location, a container, a leaf - so a new backend
is a sibling module supplying the same three roles rather than a change to anything here. Code
written against [`IOBase`](io.md) does not learn which one it got:

=== "Rust"

    ```rust
    use yggdryl::io::{Buffer, IOBase};
    use yggdryl::local::File;

    fn head(handle: &dyn IOBase) -> yggdryl::Result<Vec<u8>> {
        handle.read_range_bytes(0, 4)
    }

    let path = std::env::temp_dir().join(format!("yggdryl-doc-agnostic-{}.bin", std::process::id()));

    let mut file = File::create(&path)?;
    file.write_all_bytes(b"AAPL,100")?;

    let memory = Buffer::from_bytes(b"AAPL,100".to_vec());
    assert_eq!(head(&file)?, b"AAPL");
    assert_eq!(head(&file)?, head(&memory)?);

    drop(file);
    let _ = std::fs::remove_file(&path);
    ```

That is why [`ipc`](ipc.md) and [`parquet`](parquet.md) take a handle rather than a path: the
same reader runs over a mapped file, an in-memory [`Buffer`](io.md), or a compressed handle from
[`gzip`](gzip.md), [`zlib`](zlib.md), or [`zstd`](zstd.md).

## Private entries

A listing excludes names beginning with a dot unless it is asked for them, so
walking a tree does not wander into `.git`, `.venv`, or `.DS_Store`.

```rust
use yggdryl::io::IOBase;
use yggdryl::local::Folder;
use yggdryl::Url;

let root = std::env::temp_dir().join("yggdryl-doc-private");
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

A private directory is not descended into either, so a recursive listing stays
out of them entirely.

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/rust/local.ipynb){ download }.

<!-- /notebooks -->
