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

The bindings mirror the family under `yggdryl.local` / `require('yggdryl').local` with the
generic entries (`LocalIO(path_or_uri)`, `ls(recursive=…)` over the streamed core iterators).

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

