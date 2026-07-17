# The local filesystem

`io::local` is the **local-filesystem family**: every type implements the one
[`IOBase`](memory.md) contract — bytes, address, *and* the filesystem graph — addressed by
[`Uri`](../uri.md)s. Per the one-access-point rule the family has a **single handle**,
`LocalIO`, plus the raw memory-mapped [`Mmap`](#mmap-the-memory-mapped-file) it builds on.

## The graph surface

There is no separate path type or trait: **`IOBase` itself is the central access path**. One
`LocalIO` is a node of the local IO graph, so navigation (`name` / `parent` / `parents` /
`join`), **streamed** discovery (`ls`), the *byte* contract (`pread*` / `pwrite*`, including
over a directory), and CRUD (`rm` / `rmfile` / `rmdir`) all live on the same handle. A future
object store (s3, azure, …) implements the same surface and every example below works
unchanged.

## One access point — lazy and self-optimizing

`LocalIO` is a **lazy** node over any path (file, folder, or nothing yet) that decides, per
call, how to serve I/O:

- **Constructing / probing / navigating touches nothing** — `kind` / `exists` / `is_file` /
  `is_dir` ask the disk per call; `join` / `parent` are pure address algebra.
- **Reads pick their own path** — before any write, one ad-hoc positioned OS read (a missing
  or directory node reads as empty); after the handle has written, reads come from its kept
  **memory-mapped backing** at memory speed.
- **Writes auto-create and self-optimize** — the first write creates the missing parent
  folders and the file, memory-maps it, and keeps the mapping (`is_mapped` turns true), with
  `Heap`-style amortized growth. No `touch`, no separate file object; `mkdir()` exists for
  when a folder itself is the goal, and `close()` releases the mapping (truncating to the
  logical length) while **leaving the handle usable** — it simply returns to its lazy state.

The sections below build up from a single write to the whole surface.

## Open, write, read back

The simplest cookbook: address a path, write, read it straight back. The handle is lazy until
the first write auto-creates and maps the file; `close()` then releases the mapping so the file
can be removed (on Windows a mapped file cannot be deleted).

=== "Python"

    ```python
    import os
    import tempfile

    from yggdryl.local import LocalIO

    path = os.path.join(tempfile.mkdtemp(), "note.txt")
    note = LocalIO(path)                       # lazy — nothing on disk yet
    assert not note.exists()

    note.pwrite_utf8(0, "hello world")         # auto-creates the file — and maps it
    assert note.is_file() and note.is_mapped
    assert note.pread_utf8(0, 5) == "hello"    # memory-speed from here on

    note.close()                               # release the mapping (Windows can delete now)
    note.rmfile()
    ```

=== "Node"

    ```javascript
    const fs = require('node:fs');
    const os = require('node:os');
    const path = require('node:path');
    const { local } = require('yggdryl');

    const file = path.join(fs.mkdtempSync(path.join(os.tmpdir(), 'ex-')), 'note.txt');
    const note = new local.LocalIO(file);              // lazy — nothing on disk yet
    console.assert(!note.exists());

    note.pwriteUtf8(0, 'hello world');                 // auto-creates the file — and maps it
    console.assert(note.isFile() && note.isMapped);
    console.assert(note.preadUtf8(0, 5) === 'hello');  // memory-speed from here on

    note.close();                                      // release the mapping
    note.rmfile();
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::local::LocalIO;
    use yggdryl_core::io::memory::IOBase;

    let path = std::env::temp_dir().join("yggdryl_doc_easy").join("note.txt");
    let mut note = LocalIO::from_path(&path);           // lazy — nothing on disk yet
    assert!(!note.exists());

    note.pwrite_utf8(0, "hello world");                 // auto-creates the file — and maps it
    assert!(note.is_file() && note.is_mapped());
    assert_eq!(note.pread_utf8(0, 5).unwrap(), "hello"); // memory-speed from here on

    note.close();                                        // release the mapping
    note.rmfile().unwrap();
    ```

## Navigate the graph — join, parent, parents

`join(segment)` composes a child address through the URI ([`Uri::joinpath`](../uri.md)); the
segment may be **multi-segment** (`"logs/app.log"`). It is lazy — nothing on disk is touched
until the child is read or written, and writing the child auto-creates every missing parent.
`parent()` is the inverse (a child's parent addresses the node again), and `parents()`
iterates the ancestors **nearest-first** up to the filesystem root. Python also spells `join`
as the `/` operator.

=== "Python"

    ```python
    import tempfile

    from yggdryl.local import LocalIO

    root = LocalIO(tempfile.mkdtemp())
    child = root / "logs/app.log"              # `/` == join; multi-segment, still lazy
    child.pwrite_utf8(0, "boot ok")            # auto-creates logs/ and app.log

    assert child.parent().name == "logs"       # the inverse of join
    ancestors = [node.name for node in child.parents()]
    assert ancestors[0] == "logs"              # nearest first, walking up to the root

    child.close()
    root.rmdir()                               # recursive cleanup
    ```

=== "Node"

    ```javascript
    const fs = require('node:fs');
    const os = require('node:os');
    const path = require('node:path');
    const { local } = require('yggdryl');

    const root = new local.LocalIO(fs.mkdtempSync(path.join(os.tmpdir(), 'ex-')));
    const child = root.join('logs/app.log');           // multi-segment, still lazy
    child.pwriteUtf8(0, 'boot ok');                    // auto-creates logs/ and app.log

    console.assert(child.parent().name === 'logs');    // the inverse of join
    const ancestors = child.parents().map((node) => node.name);
    console.assert(ancestors[0] === 'logs');           // nearest first, up to the root

    child.close();
    root.rmdir();                                      // recursive cleanup
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::local::LocalIO;
    use yggdryl_core::io::memory::IOBase;

    let root = LocalIO::from_path(std::env::temp_dir().join("yggdryl_doc_nav"));
    let mut child = root.join_str("logs/app.log");      // multi-segment, still lazy
    child.pwrite_utf8(0, "boot ok");                    // auto-creates logs/ and app.log

    assert_eq!(child.parent().unwrap().name(), "logs"); // the inverse of join
    let ancestors: Vec<String> = child.parents().map(|node| node.name()).collect();
    assert_eq!(ancestors[0], "logs");                   // nearest first, up to the root

    child.close();
    root.rmdir().unwrap();                              // recursive cleanup
    ```

## Temporary scratch space — tmpfile and tmpfolder

`LocalIO.tmpfile(name?)` and `LocalIO.tmpfolder(name?)` are static builders for lazy handles
in the system temp directory; the default name is process-unique (the file name ends in
`.tmp`). Both stay lazy — a `tmpfile` is created on the first write, and a `tmpfolder` is
created by `mkdir()` or, as here, by writing a child that auto-creates it as a parent.

=== "Python"

    ```python
    from yggdryl.local import LocalIO

    scratch = LocalIO.tmpfile()                # unique <...>.tmp in the temp dir, lazy
    assert not scratch.exists()
    scratch.pwrite_utf8(0, "temp data")        # created + mapped on this write
    assert scratch.pread_utf8(0, 9) == "temp data"
    scratch.close()
    scratch.rmfile()

    work = LocalIO.tmpfolder()                 # unique temp folder, lazy
    out = work / "out.bin"
    out.pwrite_byte_array(0, b"x")             # auto-creates the work folder
    assert work.is_dir()
    out.close()
    work.rmdir()
    ```

=== "Node"

    ```javascript
    const { local } = require('yggdryl');

    const scratch = local.LocalIO.tmpfile();           // unique <...>.tmp, lazy
    console.assert(!scratch.exists());
    scratch.pwriteUtf8(0, 'temp data');                // created + mapped on this write
    console.assert(scratch.preadUtf8(0, 9) === 'temp data');
    scratch.close();
    scratch.rmfile();

    const work = local.LocalIO.tmpfolder();            // unique temp folder, lazy
    const out = work.join('out.bin');
    out.pwriteByteArray(0, Buffer.from('x'));          // auto-creates the work folder
    console.assert(work.isDir());
    out.close();
    work.rmdir();
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::local::LocalIO;
    use yggdryl_core::io::memory::IOBase;

    let mut scratch = LocalIO::tmpfile(None);           // unique <...>.tmp, lazy
    assert!(!scratch.exists());
    scratch.pwrite_utf8(0, "temp data");                // created + mapped on this write
    assert_eq!(scratch.pread_utf8(0, 9).unwrap(), "temp data");
    scratch.close();
    scratch.rmfile().unwrap();

    let work = LocalIO::tmpfolder(None);                // unique temp folder, lazy
    let mut out = work.join_str("out.bin");
    out.pwrite_byte_array(0, b"x");                     // auto-creates the work folder
    assert!(work.is_dir());
    out.close();
    work.rmdir().unwrap();
    ```

## A directory is a memory tree

A **container** node serves the *byte* contract too — generically, through `IOBase`'s `tree_*`
pattern, written once for every filesystem family:

- `byte_size` on a directory is the **lazy, streamed sum** of its subtree (recomputed live per
  call — nothing collected, nothing cached).
- `pread` / `pwrite` route across the directory's **name-sorted child blocks** as one
  contiguous byte region (listing order is OS-dependent; names are not). Reads recurse through
  child directories; a write inside a block is **capped at that block's end** (a middle block
  never grows — the layout would shift), and bytes past the end grow the **last** block. An
  empty directory refuses full writes with a guided error naming the fix (*join a file name
  onto this directory and write there*).

=== "Python"

    ```python
    import tempfile

    from yggdryl.local import LocalIO

    root = LocalIO(tempfile.mkdtemp())
    a = root / "a.txt"; a.pwrite_utf8(0, "AAA"); a.close()
    b = root / "b.txt"; b.pwrite_utf8(0, "BB"); b.close()

    assert root.byte_size() == 5               # lazy sum of the subtree
    assert root.pread_utf8(0, 5) == "AAABB"    # name-sorted blocks stitched together
    root.rmdir()
    ```

=== "Node"

    ```javascript
    const fs = require('node:fs');
    const os = require('node:os');
    const path = require('node:path');
    const { local } = require('yggdryl');

    const root = new local.LocalIO(fs.mkdtempSync(path.join(os.tmpdir(), 'ex-')));
    const a = root.join('a.txt'); a.pwriteUtf8(0, 'AAA'); a.close();
    const b = root.join('b.txt'); b.pwriteUtf8(0, 'BB'); b.close();

    console.assert(root.byteSize() === 5);             // lazy sum of the subtree
    console.assert(root.preadUtf8(0, 5) === 'AAABB');  // name-sorted blocks stitched
    root.rmdir();
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::local::LocalIO;
    use yggdryl_core::io::memory::IOBase;

    let root = LocalIO::from_path(std::env::temp_dir().join("yggdryl_doc_tree"));
    let mut a = root.join_str("a.txt"); a.pwrite_utf8(0, "AAA"); a.close();
    let mut b = root.join_str("b.txt"); b.pwrite_utf8(0, "BB"); b.close();

    assert_eq!(root.byte_size(), 5);                     // lazy sum of the subtree
    assert_eq!(root.pread_utf8(0, 5).unwrap(), "AAABB"); // name-sorted blocks stitched
    root.rmdir().unwrap();
    ```

## Streamed discovery — ls, children, and probes

Discovery is **streamed**: `ls()` yields the direct children one at a time as you pull them
(`ls(recursive=True)` walks the entire subtree depth-first) — never a pre-collected tree. Use
`children()` for the collected direct-children convenience. Existence is a probe, not a stored
state: `kind` / `is_file` / `is_dir` / `exists` ask the backing each call. A lazy handle to
nothing reports `IOKind.Missing`; a created node resolves to `IOKind.File` or
`IOKind.Directory`. (`IOKind` also carries `Unknown` — *something exists but its type is
undetermined* — and `Heap`; a local handle only ever resolves to `Missing`, `File`, or
`Directory`.)

=== "Python"

    ```python
    import tempfile

    from yggdryl.io import IOKind
    from yggdryl.local import LocalIO

    root = LocalIO(tempfile.mkdtemp())
    logs = root / "logs"
    app = logs / "app.log"
    app.pwrite_utf8(0, "boot ok"); app.close()

    for entry in root.ls():                    # one level, streamed lazily
        print(entry.name, entry.is_dir())
    names = [entry.name for entry in root.ls(recursive=True)]  # whole subtree
    assert "app.log" in names
    assert [child.name for child in root.children()] == ["logs"]  # collected convenience

    assert logs.kind == IOKind.Directory and logs.is_dir()   # probes, per call
    assert app.kind == IOKind.File and app.is_file()
    missing = root / "nope"
    assert missing.kind == IOKind.Missing and not missing.exists()
    root.rmdir()
    ```

=== "Node"

    ```javascript
    const fs = require('node:fs');
    const os = require('node:os');
    const path = require('node:path');
    const { local, io } = require('yggdryl');

    const root = new local.LocalIO(fs.mkdtempSync(path.join(os.tmpdir(), 'ex-')));
    const logs = root.join('logs');
    const app = logs.join('app.log');
    app.pwriteUtf8(0, 'boot ok'); app.close();

    for (const entry of root.ls()) {                   // one level, streamed lazily
      console.log(entry.name, entry.isDir());
    }
    const names = [...root.ls(true)].map((entry) => entry.name);  // whole subtree
    console.assert(names.includes('app.log'));
    console.assert(root.children().map((c) => c.name).join() === 'logs'); // collected

    console.assert(logs.kind === io.IOKind.Directory && logs.isDir());    // probes, per call
    console.assert(app.kind === io.IOKind.File && app.isFile());
    const missing = root.join('nope');
    console.assert(missing.kind === io.IOKind.Missing && !missing.exists());
    root.rmdir();
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::local::LocalIO;
    use yggdryl_core::io::memory::IOBase;
    use yggdryl_core::io::IOKind;

    let root = LocalIO::from_path(std::env::temp_dir().join("yggdryl_doc_ls"));
    let logs = root.join_str("logs");
    let mut app = logs.join_str("app.log");
    app.pwrite_utf8(0, "boot ok");
    app.close();

    for entry in root.ls().unwrap() {                    // one level, streamed lazily
        let entry = entry.unwrap();
        println!("{} {}", entry.name(), entry.is_dir());
    }
    let names: Vec<String> = root                        // whole subtree, depth-first
        .ls_recursive()
        .unwrap()
        .map(|entry| entry.unwrap().name())
        .collect();
    assert!(names.contains(&"app.log".to_string()));
    let kids: Vec<String> = root.children().unwrap().iter().map(IOBase::name).collect();
    assert_eq!(kids, vec!["logs"]);                      // collected convenience

    assert_eq!(logs.kind(), IOKind::Directory);          // probes, per call
    assert_eq!(app.kind(), IOKind::File);
    assert_eq!(root.join_str("nope").kind(), IOKind::Missing);
    root.rmdir().unwrap();
    ```

## Bulk SIMD access and concurrency

Once a handle is mapped, the typed **bulk** reads/writes (`pread_i32_array` /
`pwrite_i32_array`, `_i64_` too) and repeated-value fills (`pwrite_i32_repeat`, …) run as
dense, branch-free loops straight over the contiguous mapping, so LLVM auto-vectorizes them on
stable Rust (no SIMD dependency) and a fill never materializes the full array. In the bindings
each bulk read returns a fresh list; the Rust core fills a caller-provided buffer (the
least-reallocation *read-into* form).

=== "Python"

    ```python
    from yggdryl.local import LocalIO

    data = LocalIO.tmpfile()
    data.pwrite_i32_array(0, [1, -2, 3, 4])    # maps, then one contiguous vectorized write
    assert data.pread_i32_array(0, 4) == [1, -2, 3, 4]
    data.pwrite_i32_repeat(64, 7, 1000)        # memset-style fill, no array built
    data.close()
    data.rmfile()
    ```

=== "Node"

    ```javascript
    const { local } = require('yggdryl');

    const data = local.LocalIO.tmpfile();
    data.pwriteI32Array(0, [1, -2, 3, 4]);             // one contiguous vectorized write
    console.assert(data.preadI32Array(0, 4).join(',') === '1,-2,3,4');
    data.pwriteI32Repeat(64, 7, 1000);                 // memset-style fill, no array built
    data.close();
    data.rmfile();
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::local::LocalIO;
    use yggdryl_core::io::memory::IOBase;

    let mut data = LocalIO::tmpfile(None);
    data.pwrite_i32_array(0, &[1, -2, 3, 4]).unwrap(); // one contiguous vectorized write
    let mut out = [0i32; 4];
    data.pread_i32_array(0, &mut out).unwrap();        // read-into a caller buffer
    assert_eq!(out, [1, -2, 3, 4]);
    data.pwrite_i32_repeat(64, 7, 1000).unwrap();      // memset-style fill, no array built
    data.close();
    data.rmfile().unwrap();
    ```

!!! note "Concurrency — one writer per file, many readers per mapping"

    A mapping is `Send + Sync` and its `&self` reads never take a lock, so an `Arc<Mmap>` fans
    out to as many reader threads as there are cores with no contention: concurrent reads scale
    near-linearly (measured **144 → 777 Mops/s across 1 → 8 threads**). Concurrent writes to
    *disjoint* files scale too (33 → 192 Mops/s), while one file expects a single writer — a
    live mapping carries capacity padding until it closes. See the
    [`io_local_io` benchmark note](https://github.com/Platob/yggdryl/blob/main/benchmarks/yggdryl-core/io/local/io.md).

## Removing nodes — rm, rmfile, rmdir

`rm()` removes whatever exists (a file is unlinked; a directory is removed with its whole
subtree; a missing node is a no-op). `rmfile()` and `rmdir()` are **shape-checked** and raise
a guided error on a mismatch — `rmdir` on a file says *use rmfile instead of rmdir*, `rmfile`
on a directory says *use rmdir (recursive) instead of rmfile*. Call `close()` before removing a
file the handle has written: on Windows a mapped file cannot be deleted.

=== "Python"

    ```python
    import tempfile

    from yggdryl.local import LocalIO

    root = LocalIO(tempfile.mkdtemp())
    f = root / "data.bin"
    f.pwrite_utf8(0, "x")
    f.close()                                  # release the mapping before removal (Windows)

    try:
        f.rmdir()                              # f is a file — shape-checked
    except ValueError as e:
        assert "use rmfile instead of rmdir" in str(e)
    f.rmfile()                                 # the correct verb

    root.rm()                                  # removes whatever remains (here, the dir)
    ```

=== "Node"

    ```javascript
    const fs = require('node:fs');
    const os = require('node:os');
    const path = require('node:path');
    const { local } = require('yggdryl');

    const root = new local.LocalIO(fs.mkdtempSync(path.join(os.tmpdir(), 'ex-')));
    const f = root.join('data.bin');
    f.pwriteUtf8(0, 'x');
    f.close();                                         // release the mapping before removal

    try {
      f.rmdir();                                       // f is a file — shape-checked
    } catch (e) {
      console.assert(/use rmfile instead of rmdir/.test(e.message));
    }
    f.rmfile();                                        // the correct verb

    root.rm();                                         // removes whatever remains
    ```

=== "Rust"

    ```rust
    use yggdryl_core::io::local::LocalIO;
    use yggdryl_core::io::memory::IOBase;

    let root = LocalIO::from_path(std::env::temp_dir().join("yggdryl_doc_crud"));
    let mut f = root.join_str("data.bin");
    f.pwrite_utf8(0, "x");
    f.close();                                          // release the mapping before removal

    assert!(f.rmdir().is_err());                        // f is a file — guided "use rmfile…"
    f.rmfile().unwrap();                                // the correct verb

    root.rm().unwrap();                                 // removes whatever remains
    ```

## Surface parity

Both bindings mirror the whole surface: the generic `LocalIO(path_or_uri)` constructor and the
`tmpfile` / `tmpfolder` builders, the byte contract of the [memory page](memory.md) (minus the
`Heap`-only `cursor()` / `window()` builders and `with_capacity` — a handle's size comes from
the file), the probing predicates (`is_file` / `is_dir` / `exists` / `kind`), navigation
(`name`, `parent()`, `parents()`, `join` — plus Python's `node / "a/b.txt"` operator), the one
generic `ls(recursive=…)` entry **streaming** the core iterators (`children()` is the collected
convenience), the shape-checked `rm()` / `rmfile()` / `rmdir()`, and `mkdir()` / `flush()` /
`close()` / `is_mapped`. A `copy()` is a fresh lazy handle to the same path; handles compare
equal by path.

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
