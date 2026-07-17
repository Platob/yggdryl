# The local filesystem

`io::local` is the **local-filesystem family**: every type implements the byte contract
([`IOBase`](memory.md)) *and* the filesystem-graph contract (`Path`), addressed by
[`Uri`](../uri.md)s.

## The `Path` trait — one graph contract

`io::Path` is the uniform cross-filesystem abstraction: navigation (`name` / `parent` /
`join_str`), **streamed** discovery (`ls` one level, `ls_recursive` the subtree — iterators,
never a pre-collected tree; `children` is the collected convenience), and CRUD (`rm` removes
whatever exists, `rmfile` / `rmdir` are shape-checked with guided errors). Existence is a
probe: `kind` / `is_file` / `is_dir` / `exists` ask the backing each call. A future object
store or archive family implements the same trait and every caller works unchanged.

## `LocalIO` — one access point, lazy and self-optimizing

Per the one-access-point rule, the local family has a **single handle**: `LocalIO`, a lazy
node over any path (file, folder, or nothing yet) that decides per call how to serve I/O:

- **Constructing / probing / navigating touches nothing** — `kind` / `exists` / `is_file` /
  `is_dir` ask the disk per call.
- **Reads pick their own path** — before any write, one ad-hoc positioned OS read (missing or
  directory nodes read as empty); after the handle has written, reads come from its kept
  **memory-mapped backing** at memory speed.
- **Writes auto-create and self-optimize** — the first write creates the missing parent
  folders and the file, maps it, and keeps the mapping (zero-allocation I/O, `Heap`-style
  amortized growth). No `mkdir`, no `touch`, no separate file object; `mkdir()` exists for
  when a folder itself is the goal, and `close()` releases the mapping (truncating to the
  logical length) while leaving the handle usable.

=== "Rust"

    ```rust
    use yggdryl_core::io::local::LocalIO;
    use yggdryl_core::io::memory::IOBase;
    use yggdryl_core::io::Path;

    let root = LocalIO::from_path(std::env::temp_dir().join("example"));
    let mut note = root.join_str("deep/nested/note.txt"); // lazy — nothing exists yet
    assert!(!note.exists());

    note.pwrite_utf8(0, "hello");        // auto-creates deep/, nested/, the file — and maps it
    assert!(note.is_file() && note.is_mapped());
    assert_eq!(note.pread_utf8(0, 5).unwrap(), "hello"); // memory-speed from here on

    for entry in root.ls_recursive().unwrap() {          // streamed discovery
        println!("{}", entry.unwrap().name());
    }
    note.close();                                        // release the mapping
    root.rmdir().unwrap();                               // recursive cleanup
    ```

=== "Python"

    ```python
    import tempfile

    from yggdryl.local import LocalIO

    root = LocalIO(tempfile.mkdtemp())
    note = root / "deep/nested/note.txt"      # lazy — nothing exists yet
    assert not note.exists()

    note.pwrite_utf8(0, "hello")              # auto-creates deep/, nested/, the file — and maps it
    assert note.is_file() and note.is_mapped
    assert note.pread_utf8(0, 5) == "hello"   # memory-speed from here on

    names = [entry.name for entry in root.ls(recursive=True)]
    assert "note.txt" in names
    note.close()                              # release the mapping — the handle stays usable
    root.rmdir()                              # recursive cleanup
    ```

=== "Node"

    ```javascript
    const fs = require('node:fs');
    const os = require('node:os');
    const { local } = require('yggdryl');

    const root = new local.LocalIO(fs.mkdtempSync(`${os.tmpdir()}/example-`));
    const note = root.join('deep/nested/note.txt');    // lazy — nothing exists yet
    console.assert(!note.exists());

    note.pwriteUtf8(0, 'hello');                       // auto-creates deep/, nested/, the file — and maps it
    console.assert(note.isFile() && note.isMapped);
    console.assert(note.preadUtf8(0, 5) === 'hello');  // memory-speed from here on

    const names = [...root.ls(true)].map((entry) => entry.name);
    console.assert(names.includes('note.txt'));
    note.close();                                      // release the mapping — the handle stays usable
    root.rmdir();                                      // recursive cleanup
    ```

Both bindings mirror the whole surface — the generic `LocalIO(path_or_uri)` constructor, the
byte contract of the [memory page](memory.md) (minus the `Heap`-only `cursor()` / `window()`
builders and `with_capacity` — a handle's size comes from the file), the probing predicates
(`is_file` / `is_dir` / `exists`), navigation (`name`, `parent()`, `join` — plus Python's
`node / "a/b.txt"` operator), the one generic `ls(recursive=…)` entry **streaming** the core
iterators (`children()` is the collected convenience), the shape-checked `rm()` / `rmfile()` /
`rmdir()`, and
`mkdir()` / `flush()` / `close()` / `is_mapped`. A `copy()` is a fresh lazy handle to the same
path; handles compare equal by path.

## `Mmap` — the memory-mapped file

The on-disk source: a file exposed through the **same contract** as `Heap` — every typed,
bulk, utf8, cursor, and capacity method works identically over the mapping. It is opened from
a [`Uri`](../uri.md) (`file://…` or a plain path), reports `IOKind.File` and its own address
back, and **auto-resizes**: a write past the end grows the file with the same amortized
doubling as `Heap` (`O(log n)` remaps), and the capacity padding is truncated back to the
logical length on close. Read-only mappings (`open_readonly`) physically cannot be written —
the full writes report a guided error naming the fix. Mapped I/O allocates nothing — the OS
pages back the mapping; asserted deterministically by the `io_local_mmap_alloc` test and
measured in the
[`io_local_mmap` benchmark](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/io/local/mmap.md).

=== "Rust"

    ```rust
    use yggdryl_core::io::local::Mmap;
    use yggdryl_core::io::memory::IOBase;
    use yggdryl_core::uri::Uri;

    let path = std::env::temp_dir().join("example.bin");
    let uri = Uri::from_path(&path.to_string_lossy());

    let mut map = Mmap::create_uri(&uri).unwrap();   // create-or-open, read-write
    map.write_utf8("hello mapped world");            // the same cursor stream as Heap
    map.pwrite_i32_array(32, &[1, -2, 3]).unwrap();  // the same bulk ops
    assert_eq!(map.pread_utf8(6, 6).unwrap(), "mapped");
    map.flush().unwrap();                            // msync + fsync
    drop(map);                                       // truncates capacity padding

    let mut ro = Mmap::open_uri_readonly(&uri).unwrap(); // physically unwritable
    assert!(ro.pwrite_all(0, b"x").is_err());            // guided error names the fix
    std::fs::remove_file(&path).ok();
    ```

The byte surface is identical to `Heap`'s ([memory page](memory.md)). Both bindings expose
`Mmap` in the same `local` namespace as `LocalIO`, with generic type-inferring factories
(`Mmap.open(path_or_uri)` / `open_readonly` / `create`, dispatching `str`/`string` → the path
constructors and a `Uri` → the uri ones), plus a deterministic **`close()`** (idempotent; a
Python context manager — `with Mmap.create(p) as m:` — and a `closed` getter on both), since
a live mapping should not wait for the garbage collector to unmap and truncate.
