# Arrow filesystems

`yggdryl::arrowfs` puts any existing Arrow filesystem - S3, GCS, Azure, a local tree, or one you
wrote yourself - behind the crate's one storage abstraction, [`IOBase`](io.md).

Nothing here implements a transport. `ArrowFileSystem` is a seven-method contract modeled on
Arrow's own `FileSystem` API, so an implementation that already exists - `pyarrow.fs`, Arrow C++,
Arrow Java - maps onto it method for method, and the three roles above it inherit every wrapper the
crate already has. A handle over a bucket reads and writes folders and files, streams Arrow
records, and composes with [`Coded`](io.md), [`ipc`](ipc.md), [`parquet`](parquet.md), and
[`iceberg`](iceberg.md), with no transport code written in this repository.

## Construct from a filesystem and a path

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::arrowfs::{File, MemoryFileSystem};
    use yggdryl::io::IOBase;

    let filesystem = Arc::new(MemoryFileSystem::new());
    let mut handle = File::from_location(filesystem, "bucket/trades.bin")?;

    // Per the laziness contract nothing exists until something is written.
    assert!(!handle.exists());
    assert_eq!(handle.read_all_bytes()?, b"");

    handle.write_all_bytes(b"AAPL")?;
    handle.close()?;
    assert_eq!(handle.read_all_bytes()?, b"AAPL");

    // The handle's identity is a canonical URL naming the filesystem.
    assert_eq!(handle.url().to_string(), "memory://bucket/trades.bin");
    ```

=== "Python"

    ```python
    import tempfile, pathlib
    import pyarrow.fs as pafs
    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp())
    handle = IOBase.from_arrow_fs(pafs.LocalFileSystem(), (root / "trades.bin").as_posix())

    # Per the laziness contract nothing exists until something is written.
    assert not handle.exists()
    assert handle.read_bytes() == b""

    with handle:
        handle.write_bytes(b"AAPL")

    assert handle.read_bytes() == b"AAPL"
    assert (root / "trades.bin").read_bytes() == b"AAPL"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase } = require('yggdryl')

    // Arrow JS ships no filesystem, so the handler is the filesystem: the
    // same six calls, spelled in camelCase. This one is a Map; an S3 client
    // or a caching layer answers the same way.
    const files = new Map()
    const handler = {
      typeName: 'memory',
      fileInfo: (path) =>
        files.has(path)
          ? { path, kind: 'file', size: BigInt(files.get(path).length) }
          : { path, kind: 'not-found' },
      list: () => [],
      readRange: (path, offset, length) =>
        files.get(path)?.subarray(Number(offset), Number(offset) + length) ?? null,
      writeFull: (path, bytes) => { files.set(path, Buffer.from(bytes)) },
      createDir: () => {},
      deleteFile: (path) => { files.delete(path) },
    }

    const handle = IOBase.fromArrowFs(handler, 'bucket/trades.bin')

    // Per the laziness contract nothing exists until something is written.
    assert.equal(handle.exists(), false)
    assert.equal(handle.readText(), '')

    handle.writeText('AAPL')
    handle.close()

    assert.equal(handle.readText(), 'AAPL')
    assert.equal(String(handle.url), 'memory://bucket/trades.bin')
    ```

The first path segment is the authority, which is exactly what a bucket is, so `"bucket/key"` on an
S3 filesystem spells `s3://bucket/key`. In Python the filesystem may also be inferred from the
first argument: `IOBase(fs, "bucket/key")` means the same as `IOBase.from_arrow_fs(fs, "bucket/key")`,
and JavaScript infers the same way with `new IOBase(handler, 'bucket/key')`.

!!! note "A JavaScript handler belongs to one thread"
    The handler is called synchronously, in the middle of the native read or write, so it cannot be
    reached from another thread: a handle used from a `Worker` refuses with a message saying so
    rather than queueing work. A worker that needs its own view builds its own handler - only the
    location string has to travel. This is a named limitation rather than an emulation, because
    Node-API's only cross-thread call is asynchronous and every method here has to answer now.

The real thing looks like this, and needs credentials and a network, so it is shown rather than run:

=== "Python"

    ```{ .python .ignore }
    from pyarrow.fs import S3FileSystem
    from yggdryl import IOBase, iceberg

    handle = IOBase.from_arrow_fs(S3FileSystem(region="eu-west-1"), "bucket/table")
    table = iceberg.Table.open(handle)
    rows = table.scan().read_all()
    ```

## A positional write publishes when the handle closes

An Arrow filesystem replaces whole files. It has no random write - an object store cannot patch
five bytes in the middle of an object - while `IOBase::pwrite` is positional. So a leaf stages its
*positional* mutations in memory and publishes them as exactly one whole-value replacement on
`flush` or `close`. Until then the stored value is untouched:

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::arrowfs::{ArrowFileSystem, File, MemoryFileSystem};
    use yggdryl::io::IOBase;

    let filesystem = Arc::new(MemoryFileSystem::new());
    filesystem.write_full("bucket/trades.bin", b"stored")?;
    let mut handle = File::from_location(filesystem.clone(), "bucket/trades.bin")?;

    // Positional writes are pieces of a value, so they stage.
    handle.truncate(0)?;
    handle.pwrite(0, b"pend")?;
    handle.pwrite(4, b"ing")?;

    // The handle presents the pending value; the filesystem still has the old one.
    assert_eq!(handle.read_all_bytes()?, b"pending");
    assert_eq!(filesystem.file_info("bucket/trades.bin")?.size, 6);

    handle.close()?;
    assert_eq!(filesystem.file_info("bucket/trades.bin")?.size, 7);
    ```

=== "Python"

    ```python
    import tempfile, pathlib
    import pyarrow.fs as pafs
    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp())
    handle = IOBase.from_arrow_fs(pafs.LocalFileSystem(), (root / "staged.bin").as_posix())

    handle.pwrite(0, b"pend")
    handle.pwrite(4, b"ing")
    assert not (root / "staged.bin").exists()

    handle.close()
    assert (root / "staged.bin").read_bytes() == b"pending"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase } = require('yggdryl')

    const files = new Map()
    const handler = {
      typeName: 'memory',
      fileInfo: (path) =>
        files.has(path)
          ? { path, kind: 'file', size: BigInt(files.get(path).length) }
          : { path, kind: 'not-found' },
      list: () => [],
      readRange: (path, offset, length) =>
        files.get(path)?.subarray(Number(offset), Number(offset) + length) ?? null,
      writeFull: (path, bytes) => { files.set(path, Buffer.from(bytes)) },
      createDir: () => {},
      deleteFile: (path) => { files.delete(path) },
    }

    const handle = IOBase.fromArrowFs(handler, 'bucket/staged.bin')
    handle.pwrite(0, Buffer.from('pend'))
    handle.pwrite(4, Buffer.from('ing'))

    // The handle presents the pending value; the filesystem has not been
    // asked to store anything yet.
    assert.equal(handle.readText(), 'pending')
    assert.equal(files.has('bucket/staged.bin'), false)

    handle.close()
    assert.equal(files.get('bucket/staged.bin').toString(), 'pending')
    ```

That is why a file another reader will open is written inside a scope - `with` in Python, `using`
in JavaScript - which binds to exactly `open` and `close`.

A *whole-value* write needs none of that. `write_all_bytes`, `write_lines`, and `append_lines` each
describe one complete value, which is one store operation, so they publish when they finish. The
staging exists to fold many positional writes into one replacement; it is not a mode a caller has
to remember to leave.

Reads need none of it. A `pread` maps straight onto one ranged fetch, so asking for eight bytes of
a large object transfers eight bytes rather than the object. What a record encoding does with that
is its own business, and [`parquet`](parquet.md) currently fetches the value whole - its footer is
at the end, and a range-reading reader over `pread` is the optimization path that page names.
Reading one Parquet footer over a bucket therefore still costs a whole object today; the handle is
what stops being the reason.

## Folders, globs, and partitions

A directory on an object store is a prefix, so existence here is what the filesystem itself
reports: the prefix has entries, or a marker exists. Nothing invents marker objects a store would
not have written.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::arrowfs::{ArrowFileSystem, Folder, MemoryFileSystem};
    use yggdryl::io::IOBase;

    let filesystem = Arc::new(MemoryFileSystem::new());
    for year in ["2024", "2025"] {
        let leaf = format!("bucket/year={year}/part-0.parquet");
        filesystem.write_full(&leaf, b"PAR1")?;
    }
    let lake = Folder::from_location(filesystem, "bucket")?;

    assert!(lake.is_container());
    assert_eq!(lake.ls(false, false)?.len(), 2);
    assert_eq!(lake.glob("**/*.parquet", false)?.len(), 2);

    // A fixed prefix is descended rather than listed and filtered.
    assert_eq!(lake.glob("year=2024/**/*.parquet", false)?.len(), 1);

    // Hive pairs are read off the location, as they are for any backend.
    let selected: Vec<_> = lake.children_where(&[("year", "2024")], false)?.collect();
    assert_eq!(selected.len(), 1);
    ```

=== "Python"

    ```python
    import tempfile, pathlib
    import pyarrow.fs as pafs
    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp()) / "lake"
    for year in ("2024", "2025"):
        leaf = root / f"year={year}"
        leaf.mkdir(parents=True)
        (leaf / "part-0.parquet").write_bytes(b"PAR1")

    lake = IOBase.from_arrow_fs(pafs.LocalFileSystem(), root.as_posix())

    assert lake.is_dir()
    assert len(lake.iterdir()) == 2
    assert len(lake.glob("**/*.parquet")) == 2
    assert len(lake.children_where({"year": "2024"})) == 1

    # A child still carries the filesystem it came from.
    part = lake / "year=2024" / "part-0.parquet"
    assert part.read_bytes() == b"PAR1"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase } = require('yggdryl')

    // A directory is a prefix, so `list` derives one from the keys rather
    // than storing markers the caller's storage would not have written.
    const files = new Map([
      ['bucket/year=2024/part-0.parquet', Buffer.from('PAR1')],
      ['bucket/year=2025/part-0.parquet', Buffer.from('PAR1')],
    ])
    const under = (prefix) =>
      [...files.keys()].filter((name) => prefix === '' || name.startsWith(`${prefix}/`))
    const handler = {
      typeName: 'memory',
      fileInfo(path) {
        if (files.has(path)) return { path, kind: 'file', size: BigInt(files.get(path).length) }
        return under(path).length ? { path, kind: 'directory' } : { path, kind: 'not-found' }
      },
      list(path, recursive) {
        const prefix = path === '' ? '' : `${path}/`
        const directories = new Set()
        const found = []
        for (const name of under(path)) {
          const parts = name.slice(prefix.length).split('/')
          for (let depth = 1; depth < parts.length; depth += 1) {
            if (recursive || depth === 1) directories.add(prefix + parts.slice(0, depth).join('/'))
          }
          if (parts.length === 1 || recursive) {
            found.push({ path: name, kind: 'file', size: BigInt(files.get(name).length) })
          }
        }
        for (const name of directories) found.push({ path: name, kind: 'directory' })
        return found
      },
      readRange: (path, offset, length) =>
        files.get(path)?.subarray(Number(offset), Number(offset) + length) ?? null,
      writeFull: (path, bytes) => { files.set(path, Buffer.from(bytes)) },
      createDir: () => {},
      deleteFile: (path) => { files.delete(path) },
    }

    const lake = IOBase.fromArrowFs(handler, 'bucket')

    assert.equal(lake.isDir(), true)
    assert.equal(lake.ls().length, 2)
    assert.equal(lake.glob('**/*.parquet').length, 2)
    assert.equal(lake.glob('year=2024/**/*.parquet').length, 1)
    assert.equal(lake.childrenWhere({ year: '2024' }).length, 1)

    // A child still carries the filesystem it came from.
    assert.equal(lake.joinpath('year=2024', 'part-0.parquet').readText(), 'PAR1')
    ```

## Records

The record surface is the same three methods every handle answers, inherited rather than
reimplemented, so the encoding still comes from the media type and never from an argument.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::arrowfs::{File, MemoryFileSystem};
    use yggdryl::generic::IORecordOptions;
    use yggdryl::io::IOBase;
    use yggdryl::DataType;

    let schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("row");

    let batch = RecordBatch::try_new(
        yggdryl::arrow::schema_from_field(&schema)?,
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("AAPL"), None])),
        ],
    )?;

    let filesystem = Arc::new(MemoryFileSystem::new());
    let mut handle = File::from_location(filesystem, "bucket/trades.parquet")?;
    let options = handle.record_options()?.with_schema(schema.clone());

    handle.write_arrow_batch_reader(
        yggdryl::arrow::batch_reader(batch.schema(), [batch]),
        &options,
    )?;
    handle.close()?;

    let rows: usize = handle
        .read_arrow_batch_reader(&options)?
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(rows, 2);
    assert_eq!(handle.read_arrow_field(&options)?, schema);
    ```

=== "Python"

    ```python
    import tempfile, pathlib
    import pyarrow as pa
    import pyarrow.fs as pafs
    import pyarrow.parquet as pq
    from yggdryl import IOBase

    root = pathlib.Path(tempfile.mkdtemp())
    table = pa.table({"id": [1, 2], "symbol": ["AAPL", "MSFT"]})

    handle = IOBase.from_arrow_fs(pafs.LocalFileSystem(), (root / "trades.parquet").as_posix())
    with handle:
        handle.write_arrow_batch_reader(table)

    assert handle.read_arrow_batch_reader().read_all().num_rows == 2

    # What landed is an ordinary Parquet file, so PyArrow reads it back.
    assert pq.read_table(root / "trades.parquet").equals(table)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, IOBase } = require('yggdryl')

    const files = new Map()
    const handler = {
      typeName: 'memory',
      fileInfo: (path) =>
        files.has(path)
          ? { path, kind: 'file', size: BigInt(files.get(path).length) }
          : { path, kind: 'not-found' },
      list: () => [],
      readRange: (path, offset, length) =>
        files.get(path)?.subarray(Number(offset), Number(offset) + length) ?? null,
      writeFull: (path, bytes) => { files.set(path, Buffer.from(bytes)) },
      createDir: () => {},
      deleteFile: (path) => { files.delete(path) },
    }

    const table = new arrow.Table({
      id: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
      symbol: arrow.vectorFromArray(['AAPL', 'MSFT'], new arrow.Utf8()),
    })

    const handle = IOBase.fromArrowFs(handler, 'bucket/trades.arrows')
    handle.writeArrowBatchReader(BatchReader.from(table))
    handle.close()

    assert.equal(handle.readArrowBatchReader().toTable().numRows, 2)
    // The encoding came from the name, never from an argument.
    assert.equal(String(handle.mediaType), 'application/vnd.apache.arrow.stream')
    ```

A folder handle reads as the partitioned table beneath it, and a folder holding an Iceberg metadata
document reads through its snapshots - both are the container behavior every backend inherits.

## Composing with the wrappers

Nothing about a foreign filesystem is special to the wrappers, because they only ever see an
`IOBase`. A content coding round trips over a bucket exactly as it does over a file:

!!! note "Rust only"
    The Python and JavaScript packages do not expose the compression wrappers.
    The Iceberg composition below carries its own tabs.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::arrowfs::{File, MemoryFileSystem};
    use yggdryl::io::{Coded, IOBase};
    use yggdryl::{Codec, MimeType};

    let filesystem = Arc::new(MemoryFileSystem::new());
    let leaf = File::from_location(filesystem.clone(), "bucket/trades.json.gz")?;

    let mut coded = Coded::new(leaf, Codec::Gzip);
    coded.write_all_bytes(br#"{"symbol":"AAPL"}"#)?;
    coded.close()?;

    // The view presents the decoded value...
    assert_eq!(coded.media_type().base(), &MimeType::JSON);
    assert_eq!(coded.read_all_bytes()?, br#"{"symbol":"AAPL"}"#);

    // ...while what the filesystem holds is gzip.
    let stored = File::from_location(filesystem, "bucket/trades.json.gz")?;
    assert_eq!(&stored.read_all_bytes()?[..2], &[0x1f, 0x8b]);
    ```

An Iceberg table is a folder reached through `IOBase` only, so a warehouse on a foreign filesystem
needs nothing from the table format:

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::arrowfs::{Folder, MemoryFileSystem};
    use yggdryl::iceberg::{FormatVersion, PartitionSpec, Table};
    use yggdryl::io::IOBase;
    use yggdryl::DataType;

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");
    let batch = RecordBatch::try_new(
        yggdryl::arrow::schema_from_field(&schema)?,
        vec![Arc::new(Int64Array::from(vec![1, 2]))],
    )?;

    let filesystem = Arc::new(MemoryFileSystem::new());
    let root = Folder::from_location(filesystem, "warehouse/trades")?;

    let mut table = Table::create(
        root,
        FormatVersion::V2,
        schema,
        PartitionSpec::unpartitioned(),
    )?;
    table.append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))?;

    let options = table.record_options()?;
    let rows: usize = table
        .read_arrow_batch_reader(&options)?
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(rows, 2);
    ```

=== "Python"

    ```python
    import tempfile, pathlib
    import pyarrow as pa
    import pyarrow.fs as pafs
    from yggdryl import IOBase, iceberg

    root = pathlib.Path(tempfile.mkdtemp())
    table_rows = pa.table({"id": [1, 2], "symbol": ["AAPL", "MSFT"]})

    warehouse = IOBase.from_arrow_fs(pafs.LocalFileSystem(), (root / "trades").as_posix())
    table = iceberg.Table.create(warehouse, table_rows.schema)
    table.append(table_rows)

    assert table.scan().read_all().num_rows == 2
    ```

## The two filesystems that ship here

`MemoryFileSystem` holds everything in one map and is the substrate the tests and benchmarks run
on. `LocalFileSystem` is a thin `std::fs` mapping whose writes publish through a temporary file and
a rename, so a reader never observes a half-written value; it exists to prove the contract against
a real operating-system filesystem and to measure the wrapper against a native handle. **Neither
replaces [`local`](local.md)**, whose memory-mapped `File` remains the local backend.

!!! note "Rust only"
    Both are Rust types. A binding reaches a filesystem through the one its own
    ecosystem already has - `pyarrow.fs` in Python, a handler object in
    JavaScript.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::arrowfs::{File, LocalFileSystem};
    use yggdryl::io::IOBase;

    let root = std::env::temp_dir().join(format!("yggdryl-doc-arrowfs-{}", std::process::id()));
    std::fs::create_dir_all(&root)?;
    let location = root.join("trades.bin").to_string_lossy().replace('\\', "/");

    let mut handle = File::from_location(Arc::new(LocalFileSystem::new()), &location)?;
    handle.write_all_bytes(b"AAPL")?;
    handle.close()?;

    assert_eq!(std::fs::read(root.join("trades.bin"))?, b"AAPL");
    let _ = std::fs::remove_dir_all(&root);
    ```

## Bringing your own filesystem

In Rust, implement `ArrowFileSystem`. Seven methods, all synchronous, and the semantics are the
ones Arrow already specifies: a path that is not there is `Unknown` rather than an error, a missing
directory lists empty, a read past the end is short, and a write replaces the whole value.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::arrowfs::{ArrowFileSystem, FileInfo, File};
    use yggdryl::io::IOBase;
    use yggdryl::Result;

    /// A filesystem holding exactly one read-only object.
    struct OneObject;

    impl ArrowFileSystem for OneObject {
        fn type_name(&self) -> &str {
            "memory"
        }

        fn file_info(&self, path: &str) -> Result<FileInfo> {
            Ok(if path == "bucket/only.bin" {
                FileInfo::file(path, 5)
            } else {
                FileInfo::not_found(path)
            })
        }

        fn list(&self, _path: &str, _recursive: bool) -> Result<Vec<FileInfo>> {
            Ok(vec![FileInfo::file("bucket/only.bin", 5)])
        }

        fn read_range(&self, path: &str, offset: u64, buffer: &mut [u8]) -> Result<usize> {
            if path != "bucket/only.bin" {
                return Ok(0);
            }
            let value = b"AAPL!";
            let offset = offset as usize;
            if offset >= value.len() {
                return Ok(0);
            }
            let count = (value.len() - offset).min(buffer.len());
            buffer[..count].copy_from_slice(&value[offset..offset + count]);
            Ok(count)
        }

        fn write_full(&self, _path: &str, _bytes: &[u8]) -> Result<()> {
            Ok(())
        }

        fn create_dir(&self, _path: &str) -> Result<()> {
            Ok(())
        }

        fn delete_file(&self, _path: &str) -> Result<()> {
            Ok(())
        }
    }

    let handle = File::from_location(Arc::new(OneObject), "bucket/only.bin")?;
    assert_eq!(handle.read_all_bytes()?, b"AAPL!");
    assert_eq!(handle.read_range(1, 3)?, b"APL");
    ```

In Python, write a `pyarrow.fs.FileSystemHandler` and wrap it in `pyarrow.fs.PyFileSystem`. That is
also how an `fsspec` filesystem arrives, so this one shape covers both:

=== "Python"

    ```{ .python .ignore }
    import pyarrow.fs as pafs
    from yggdryl import IOBase

    class MyHandler(pafs.FileSystemHandler):
        ...  # get_file_info, get_file_info_selector, open_input_file, open_output_stream, ...

    handle = IOBase.from_arrow_fs(pafs.PyFileSystem(MyHandler()), "bucket/key.parquet")
    ```

A working handler is longer than a documentation page wants, so the complete one lives in
`python/tests/test_arrowfs.py`, where it is exercised end to end - including an Iceberg table whose
every byte goes through it.

In JavaScript there is nothing to wrap, because Arrow JS ships no filesystem - so the handler
object *is* the filesystem, and the six methods are the whole contract. Anything a Node program can
already reach answers them:

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-doc-arrowfs-'))

    // The same six calls, over node:fs. Absence may throw the way node:fs
    // does - the boundary asks what is there and turns ENOENT into the
    // contract's empty answer.
    const handler = {
      typeName: 'local',
      fileInfo(location) {
        try {
          const stat = fs.statSync(location)
          return {
            path: location,
            kind: stat.isDirectory() ? 'directory' : 'file',
            size: BigInt(stat.size),
          }
        } catch {
          return { path: location, kind: 'not-found' }
        }
      },
      list(location, recursive) {
        const found = []
        for (const entry of fs.readdirSync(location, { withFileTypes: true })) {
          const child = path.posix.join(location, entry.name)
          if (entry.isDirectory()) {
            found.push({ path: child, kind: 'directory' })
            if (recursive) found.push(...this.list(child, true))
          } else {
            found.push({ path: child, kind: 'file', size: BigInt(fs.statSync(child).size) })
          }
        }
        return found
      },
      readRange(location, offset, length) {
        const descriptor = fs.openSync(location, 'r')
        try {
          const buffer = Buffer.alloc(length)
          const read = fs.readSync(descriptor, buffer, 0, length, Number(offset))
          return buffer.subarray(0, read)
        } finally {
          fs.closeSync(descriptor)
        }
      },
      writeFull(location, bytes) {
        fs.mkdirSync(path.posix.dirname(location), { recursive: true })
        fs.writeFileSync(location, bytes)
      },
      createDir: (location) => fs.mkdirSync(location, { recursive: true }),
      deleteFile: (location) => fs.rmSync(location, { force: true }),
    }

    const handle = IOBase.fromArrowFs(handler, path.posix.join(root, 'lake', 'trades.bin'))
    handle.writeText('AAPL')
    handle.close()

    assert.equal(fs.readFileSync(path.join(root, 'lake', 'trades.bin'), 'utf8'), 'AAPL')
    assert.equal(handle.pread(1, 3).toString(), 'APL')

    fs.rmSync(root, { recursive: true, force: true })
    ```

## What the wrapper costs

Putting an existing Arrow filesystem behind `IOBase`, the only honest
question is what the wrapper adds to the transport underneath it. Every row below is the same
payload landing in the same place twice: once through an `arrowfs` handle, once through the native
handle (or the language's own filesystem calls) holding those same bytes.

=== "Rust"

    `cargo bench --bench arrowfs --features "parquet"`, Criterion medians,
    512 KiB payloads and 65,536 rows:

    ```text
                                      arrowfs      native handle
    bytes read_all   (memory)         22.99 us     23.68 us   Buffer
    bytes write_all  (memory)         67.00 us     24.85 us   Buffer
    bytes pread 4KiB (memory)         63.91 ns     35.05 ns   Buffer
    bytes read_all   (local)          25.82 us     23.51 us   local::File
    bytes write_all  (local)         212.43 us      1.11 ms   local::File
    ipc     write                      4.08 ms      4.44 ms   Buffer
    ipc     read                       1.49 ms      1.30 ms   Buffer
    parquet write                     18.16 ms     17.84 ms   Buffer
    parquet read                       5.17 ms      5.01 ms   Buffer
    ls recursive     (local)          61.49 us     92.84 us   local::Folder
    ```

    The ranged read is the row that matters most, and it is the one stated in
    nanoseconds. Serving 4 KiB out of a 512 KiB value costs 64 ns, not the
    23 us a whole-value read costs, so the handle serves a range without
    materializing the value. The vtable itself is the 29 ns difference against
    `Buffer`: one dynamic call plus a bounds check. This measures the handle,
    not any reader above it - [`parquet`](parquet.md) still fetches its value
    whole, as that page says.

    Whole-value writes are where staging shows. An Arrow filesystem replaces
    files rather than writing ranges, so a write is buffered and published
    once - 67 us against `Buffer`'s 25 us for 512 KiB, which is the copy the
    publication costs. Against the memory-mapped `local::File` the same write
    is **five times faster** (212 us against 1.11 ms), because publishing a
    whole file through a temporary and a rename beats remapping and resizing a
    mapping. Neither number makes one backend better than the other; they
    measure different write shapes, which is exactly why both exist.

    Records are within a few percent either way, because the encoding
    dominates and the wrapper only moves the finished bytes. Listing is faster
    than the local backend's because one `list` call answers a recursive walk
    that `std::fs::read_dir` has to make per directory.

    `glob` over the same tree shows the descent the contract promises:
    expanding `**/*.parquet` across a 16-leaf lake costs 57 us, while
    `year=2024/**/*.parquet` costs 23 us, because a fixed prefix is descended
    rather than listed and filtered.

=== "Python"

    The baseline is PyArrow's own calls against the same
    `pyarrow.fs.LocalFileSystem` - the implementation the wrapper delegates
    to - so the difference is the vtable crossing and nothing else.
    `arrowfs.py --min-time 0.2 --repeat 7`, release wheel, medians:

    ```text
                            wrapper      PyArrow
    bytes write            400.0 us     244.3 us
    bytes read             115.2 us      17.6 us
    range read (4 KiB)      14.5 us       2.9 us
    parquet write            8.16 ms      6.19 ms
    parquet read             2.00 ms      1.95 ms
    listing (16 entries)    85.5 us      34.4 us
    ```

    A Parquet read is at parity, because the decode dominates and the boundary
    moves only finished bytes. Everything smaller is dominated by the crossing
    itself: each vtable call acquires the GIL and makes a handful of PyArrow
    calls, which is roughly 12 us of fixed cost, so the 4 KiB range read costs
    14.5 us against PyArrow's 2.9 us. That cost is per *call*, not per byte -
    the ranged read still reads 4 KiB rather than the 512 KiB object, which is
    the property that matters on an object store, and it is why the read is
    14.5 us rather than the 115 us a whole-value read takes.

    A write costs more than PyArrow's because it is a different operation: the
    wrapper stages the value and publishes it once, which is what makes a
    positional `pwrite` API work over a filesystem that only replaces whole
    files.

=== "JavaScript"

    JavaScript pays the same shape of cost against `node:fs`, with the handler
    crossing the boundary on every call rather than only the handle.
    `bench:arrowfs`, release build:

    ```text
                                wrapper       node:fs
    handle from path         107,101/s     257,632/s
    write bytes                4,912/s      11,235/s
    read bytes                13,193/s      58,399/s
    read range (4 KiB)       124,733/s     227,893/s
    list children             58,181/s     264,682/s
    glob *.parquet             9,372/s      16,900/s
    read records            15.6M rows/s   11.6M rows/s (local handle)
    write records           10.2M rows/s   22.1M rows/s (local handle)
    ```

    The ranged read is again the row that carries the claim: it is the
    *fastest* byte operation of the three, not the slowest, because it fetches
    4 KiB rather than the whole payload. Records read faster than through the
    local handle because the staged value is already in memory once the first
    read has fetched it, and slower to write for the same reason a Python
    write is - the value is staged and published once.

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/rust/arrowfs.ipynb){ download },
[Python](notebooks/python/arrowfs.ipynb){ download },
[JavaScript](notebooks/javascript/arrowfs.ipynb){ download }.

<!-- /notebooks -->
