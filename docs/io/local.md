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

## Lazy `LocalPath` → concrete `LocalFile` / `LocalFolder`

Per the auto-create rule, a `LocalPath` is a **lazy handle**: constructing one never touches
the disk, reads on a missing node are empty, and a **write auto-creates** the missing parent
folders and the file — no `mkdir`/`touch` pre-flighting, ever. It opens per operation; for
repeated access it **sub-instantiates** the optimized concrete types: `file()` auto-creates
and memory-maps a `LocalFile`, `folder()` auto-creates a `LocalFolder` (`mkdir -p`).

=== "Rust"

    ```rust
    use yggdryl_core::io::local::LocalPath;
    use yggdryl_core::io::memory::IOBase;
    use yggdryl_core::io::Path;

    let root = LocalPath::from_path(std::env::temp_dir().join("example"));
    let mut note = root.join_str("deep/nested/note.txt"); // lazy — nothing exists yet
    assert!(!note.exists());

    note.pwrite_utf8(0, "hello");                 // auto-creates deep/, nested/, the file
    assert!(note.is_file());
    assert_eq!(note.parent().unwrap().name(), "nested");

    // Streamed discovery + shape-checked CRUD.
    for entry in root.ls_recursive().unwrap() {
        let node = entry.unwrap();
        println!("{} ({})", node.name(), node.kind());
    }
    let fast = note.file().unwrap();              // sub-instantiate: memory-mapped access
    assert_eq!(fast.pread_utf8(0, 5).unwrap(), "hello");
    drop(fast);
    root.rmdir().unwrap();                        // recursive cleanup
    ```

The bindings mirror the family under `yggdryl.local` / `require('yggdryl').local` with the
generic entries (`LocalPath(path_or_uri)`, `ls(recursive=…)` dispatching to the streamed
core iterators).

## `Mmap` — the memory-mapped file

The on-disk source: a file exposed through the **same contract** as `Heap` — every typed,
bulk, utf8, cursor, and capacity method works identically over the mapping. It is opened from
a [`Uri`](../uri.md) (`file://…` or a plain path), reports `IOKind.File` and its own address
back, and **auto-resizes**: a write past the end grows the file with the same amortized
doubling as `Heap` (`O(log n)` remaps), and the capacity padding is truncated back to the
logical length on close. Read-only mappings (`open_readonly`) physically cannot be written —
the full writes report a guided error naming the fix. Mapped I/O allocates nothing (the OS
pages back the mapping — see the
[`io_memory_mmap` benchmark](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/io/memory/mmap.md)).

=== "Rust"

    ```rust
    use yggdryl_core::io::memory::{IOBase, Mmap};
    use yggdryl_core::uri::Uri;

    let path = std::env::temp_dir().join("example.bin");
    let uri = Uri::from_path(&path.to_string_lossy());

    let mut map = Mmap::create_uri(&uri).unwrap();   // create-or-open, read-write
    map.write_utf8("hello mapped world");            // the same cursor stream as Heap
    map.pwrite_i32_array(32, &[1, -2, 3]).unwrap();  // the same bulk ops
    assert_eq!(map.pread_utf8(6, 6).unwrap(), "mapped");
    map.flush().unwrap();                            // msync + fsync
    drop(map);                                       // truncates capacity padding

    let ro = Mmap::open_uri_readonly(&uri).unwrap(); // physically unwritable
    assert!(ro.pwrite_all(0, b"x").is_err());        // guided error names the fix
    std::fs::remove_file(&path).ok();
    ```

The byte surface is identical to `Heap`'s, so the Python/Node examples above apply verbatim —
both bindings expose `Mmap` with generic type-inferring factories (`Mmap.open(path_or_uri)` /
`open_readonly` / `create`, dispatching `str`/`string` → the path constructors and a `Uri` →
the uri ones), plus a deterministic **`close()`** (idempotent; a Python context manager —
`with Mmap.create(p) as m:` — and a `closed` getter on both), since a live mapping should not
wait for the garbage collector to unmap and truncate.

