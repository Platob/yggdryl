# Filesystems

`yggdryl::holder::fs` puts any filesystem, S3, GCS, Azure, a local tree, or your own, behind [`IOBase`](../index.md) without a transport.

## Contract

| | |
| --- | --- |
| Owns | `FileSystem` (seven synchronous methods, Arrow's API), `File`, `Folder`, `MemoryFileSystem`, `LocalFileSystem`; no transport |
| Identity | the first segment is the authority: `"bucket/key"` on S3 spells `s3://bucket/key` |
| Lazy | nothing exists until written; a missing file reads as empty bytes |
| Writes | `pwrite` and `truncate` stage until `flush` or `close`; `write_all_bytes` and record intents publish at once |
| Reads | `pread` is one ranged fetch; `parquet` still fetches the value whole |
| Directories | a prefix; exists when the filesystem reports entries or a marker, never invented |
| Bindings | Python `IOBase.from_fs(fs, path)` or `IOBase(fs, path)` over `pyarrow.fs`; JavaScript `IOBase.fromFs(handler, path)` or `new IOBase(handler, path)` |
| Rust only | `MemoryFileSystem`, `LocalFileSystem`, `Coded` composition |
| Feature flag | none; tests gate on `arrow` and `iceberg` |

## Use

The smallest handle is a filesystem and a path.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::holder::fs::{File, MemoryFileSystem};
    use yggdryl::IOBase;

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
    handle = IOBase.from_fs(pafs.LocalFileSystem(), (root / "trades.bin").as_posix())

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

    const handle = IOBase.fromFs(handler, 'bucket/trades.bin')

    // Per the laziness contract nothing exists until something is written.
    assert.equal(handle.exists(), false)
    assert.equal(handle.readText(), '')

    handle.writeText('AAPL')
    handle.close()

    assert.equal(handle.readText(), 'AAPL')
    assert.equal(String(handle.url), 'memory://bucket/trades.bin')
    ```

## Object stores

The real thing needs credentials and a network, so it is shown rather than run.

=== "Python"

    ```{ .python .ignore }
    from pyarrow.fs import S3FileSystem
    from yggdryl import IOBase
    from yggdryl.media import iceberg

    handle = IOBase.from_fs(S3FileSystem(region="eu-west-1"), "bucket/table")
    table = iceberg.Table.open(handle)
    rows = table.scan().read_all()
    ```

## Staged positional writes

An Arrow filesystem replaces whole files and has no random write, so positional mutations stage until `flush` or `close`.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::holder::fs::{FileSystem, File, MemoryFileSystem};
    use yggdryl::IOBase;

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
    handle = IOBase.from_fs(pafs.LocalFileSystem(), (root / "staged.bin").as_posix())

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

    const handle = IOBase.fromFs(handler, 'bucket/staged.bin')
    handle.pwrite(0, Buffer.from('pend'))
    handle.pwrite(4, Buffer.from('ing'))

    // The handle presents the pending value; the filesystem has not been
    // asked to store anything yet.
    assert.equal(handle.readText(), 'pending')
    assert.equal(files.has('bucket/staged.bin'), false)

    handle.close()
    assert.equal(files.get('bucket/staged.bin').toString(), 'pending')
    ```

A file another reader will open is written inside a scope, `with` or `using`, which binds to exactly `open` and `close`.

## Folders, globs, and partitions

A directory on an object store is a prefix; existence is what the filesystem reports, entries or a marker.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::holder::fs::{FileSystem, Folder, MemoryFileSystem};
    use yggdryl::IOBase;

    let filesystem = Arc::new(MemoryFileSystem::new());
    for year in ["2024", "2025"] {
        let leaf = format!("bucket/year={year}/part-0.parquet");
        filesystem.write_full(&leaf, b"PAR1")?;
    }
    let lake = Folder::from_location(filesystem, "bucket")?;

    assert!(lake.is_container());
    assert_eq!(lake.ls(false, false).count(), 2);
    assert_eq!(lake.glob("**/*.parquet", false)?.count(), 2);

    // A fixed prefix is descended rather than listed and filtered.
    assert_eq!(lake.glob("year=2024/**/*.parquet", false)?.count(), 1);

    // Hive pairs are read off the location, as they are for any backend.
    let selected: Vec<_> = lake
        .children_where(&[("year", "2024")], false)?
        .collect::<yggdryl::Result<_>>()?;
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

    lake = IOBase.from_fs(pafs.LocalFileSystem(), root.as_posix())

    assert lake.is_dir()
    assert len(list(lake.iterdir())) == 2
    assert len(list(lake.glob("**/*.parquet"))) == 2
    assert len(list(lake.children_where({"year": "2024"}))) == 1

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

    const lake = IOBase.fromFs(handler, 'bucket')

    assert.equal(lake.isDir(), true)
    assert.equal([...lake.ls()].length, 2)
    assert.equal([...lake.glob('**/*.parquet')].length, 2)
    assert.equal([...lake.glob('year=2024/**/*.parquet')].length, 1)
    assert.equal([...lake.childrenWhere({ year: '2024' })].length, 1)

    // A child still carries the filesystem it came from.
    assert.equal(lake.joinpath('year=2024', 'part-0.parquet').readText(), 'PAR1')
    ```

## Records

Every handle answers the same read plus the three write intents of [Records](../iobase/records.md); the encoding comes from the media type, never an argument.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::holder::fs::{File, MemoryFileSystem};
    use yggdryl::media::IORecordOptions;
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::DataType;

    let schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::Utf8.nullable_field("symbol"),
    ])?
    .required_field("row");

    let batch = RecordBatch::try_new(
        schema.clone().into_arrow_schema()?,
        vec![
            Arc::new(Int64Array::from(vec![1, 2])),
            Arc::new(StringArray::from(vec![Some("AAPL"), None])),
        ],
    )?;

    let filesystem = Arc::new(MemoryFileSystem::new());
    let mut handle = File::from_location(filesystem, "bucket/trades.parquet")?;
    let options = handle.record_options()?.with_field(schema.clone());

    handle.overwrite_arrow_reader(
        yggdryl::arrow::batch_reader(batch.schema(), [batch]),
        &options,
    )?;
    handle.close()?;

    let rows: usize = handle
        .read_arrow_reader(&options)?
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

    handle = IOBase.from_fs(pafs.LocalFileSystem(), (root / "trades.parquet").as_posix())
    with handle:
        handle.overwrite_arrow_table(table)

    assert handle.read_arrow_reader().read_all().num_rows == 2

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

    const handle = IOBase.fromFs(handler, 'bucket/trades.arrows')
    handle.overwriteArrowReader(BatchReader.from(table))
    handle.close()

    assert.equal(handle.readArrowReader().intoTable().numRows, 2)
    // The encoding came from the name, never from an argument.
    assert.equal(String(handle.mediaType), 'application/vnd.apache.arrow.stream')
    ```

## Composing with the wrappers

The wrappers only ever see an `IOBase`. Rust only: the bindings do not expose the compression wrappers.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::holder::fs::{File, MemoryFileSystem};
    use yggdryl::IOBase;
    use yggdryl::coding::Coded;
    use yggdryl::{Codec, MimeType};

    let filesystem = Arc::new(MemoryFileSystem::new());
    let leaf = File::from_location(filesystem.clone(), "bucket/trades.json.gz")?;

    let mut coded = Coded::wrap(leaf, Codec::Gzip);
    coded.write_all_bytes(br#"{"symbol":"AAPL"}"#)?;
    coded.close()?;

    // The view presents the decoded value...
    assert_eq!(coded.media_type().base(), &MimeType::JSON);
    assert_eq!(coded.read_all_bytes()?, br#"{"symbol":"AAPL"}"#);

    // ...while what the filesystem holds is gzip.
    let stored = File::from_location(filesystem, "bucket/trades.json.gz")?;
    assert_eq!(&stored.read_all_bytes()?[..2], &[0x1f, 0x8b]);
    ```

An [Iceberg](../../media/iceberg/index.md) table is a folder reached through `IOBase` only, so a foreign warehouse needs nothing from the table format.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch};
    use yggdryl::holder::fs::{Folder, MemoryFileSystem};
    use yggdryl::media::iceberg::{FormatVersion, PartitionSpec, Table};
    use yggdryl::{IOBase, IOMedia};
    use yggdryl::DataType;

    let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
        .required_field("row");
    let batch = RecordBatch::try_new(
        schema.clone().into_arrow_schema()?,
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
    table.commit_append(yggdryl::arrow::batch_reader(batch.schema(), [batch]))?;

    let options = table.record_options()?;
    let rows: usize = table
        .read_arrow_reader(&options)?
        .map(|batch| batch.unwrap().num_rows())
        .sum();
    assert_eq!(rows, 2);
    ```

=== "Python"

    ```python
    import tempfile, pathlib
    import pyarrow as pa
    import pyarrow.fs as pafs
    from yggdryl import IOBase
    from yggdryl.media import iceberg

    root = pathlib.Path(tempfile.mkdtemp())
    table_rows = pa.table({"id": [1, 2], "symbol": ["AAPL", "MSFT"]})

    warehouse = IOBase.from_fs(pafs.LocalFileSystem(), (root / "trades").as_posix())
    table = iceberg.Table.create(warehouse, table_rows.schema)
    table.append(table_rows)

    assert table.scan().read_all().num_rows == 2
    ```

## The two filesystems that ship here

`MemoryFileSystem` holds everything in one map and runs the tests and benchmarks; `LocalFileSystem` is a thin `std::fs` mapping. Rust only; neither replaces [Local](local.md), whose memory-mapped `File` remains the local backend.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::holder::fs::{File, LocalFileSystem};
    use yggdryl::IOBase;
    use yggdryl::holder::local::Folder;

    let root = Folder::temporary()?.path()?.join(format!("yggdryl-doc-fs-{}", std::process::id()));
    std::fs::create_dir_all(&root)?;
    let location = root.join("trades.bin").to_string_lossy().replace('\\', "/");

    let mut handle = File::from_location(Arc::new(LocalFileSystem::new()), &location)?;
    handle.write_all_bytes(b"AAPL")?;
    handle.close()?;

    assert_eq!(std::fs::read(root.join("trades.bin"))?, b"AAPL");
    let _ = std::fs::remove_dir_all(&root);
    ```

## Bringing your own filesystem

### Rust

Implement `FileSystem`, seven synchronous methods with the semantics Arrow already specifies.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::holder::fs::{FileSystem, FileInfo, File};
    use yggdryl::IOBase;
    use yggdryl::Result;

    /// A filesystem holding exactly one read-only object.
    struct OneObject;

    impl FileSystem for OneObject {
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

        fn list(&self, _path: &str, _recursive: bool) -> yggdryl::holder::fs::FileInfos {
            yggdryl::holder::fs::FileInfos::new(
                [Ok(FileInfo::file("bucket/only.bin", 5))].into_iter(),
            )
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
    assert_eq!(handle.read_range_bytes(1, 3)?, b"APL");
    ```

### Python

Write a `pyarrow.fs.FileSystemHandler` and wrap it in `pyarrow.fs.PyFileSystem`; `fsspec` arrives the same way. The complete handler lives in `python/tests/holder/test_fs.py`.

=== "Python"

    ```{ .python .ignore }
    import pyarrow.fs as pafs
    from yggdryl import IOBase

    class MyHandler(pafs.FileSystemHandler):
        ...  # get_file_info, get_file_info_selector, open_input_file, open_output_stream, ...

    handle = IOBase.from_fs(pafs.PyFileSystem(MyHandler()), "bucket/key.parquet")
    ```

### JavaScript

Arrow JS ships no filesystem, so the handler object is the filesystem and its six methods are the whole contract.

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-doc-fs-'))

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

    const handle = IOBase.fromFs(handler, path.posix.join(root, 'lake', 'trades.bin'))
    handle.writeText('AAPL')
    handle.close()

    assert.equal(fs.readFileSync(path.join(root, 'lake', 'trades.bin'), 'utf8'), 'AAPL')
    assert.equal(handle.readRangeBytes(1, 3).toString(), 'APL')

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Edges

- `pwrite` before `close` -> the handle shows the pending value; the store still holds the old one.
- `pread` of eight bytes -> eight bytes transferred; a [Parquet](../../media/parquet.md) read still costs the whole object today.
- Fixed glob prefix `year=2024/**/*.parquet` -> descended, not listed and filtered; see [Partitions](../iobase/partitions.md).
- Own `FileSystem`, missing path -> `FileInfo::not_found` (`IOKind::Unknown`), not an error.
- Own `FileSystem` -> a missing directory lists empty, a read past the end is short, a write replaces the value.
- JavaScript handle used from a `Worker` -> refuses with a message; the synchronous handler belongs to one thread, so build one per worker.
- `LocalFileSystem` write -> a temporary file plus a rename; a reader never sees a half-written value.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib holder::fs::
    cargo bench --bench holder --features parquet -- fs_bytes
    cargo bench --bench holder --features parquet -- fs_record
    cargo bench --bench holder --features parquet -- fs_listing
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/holder/test_fs.py
    python/.venv/bin/python python/benchmarks/holder.py --min-time 0.2 --repeat 7
    ```

=== "JavaScript"

    ```bash
    node --test "node/tests/holder/fs.test.js"
    npm run --prefix node bench:holder
    ```

## Performance

Every row lands the same payload twice, through an `fs` handle and natively; host and toolchain are the published ones in [benchmarks](../../benchmarks.md).

=== "Rust"

    `fs_bytes`, `fs_record`, and `fs_listing` on the `holder` target report Criterion medians over 512 KiB payloads and 65,536 rows.

    ```text
                                      fs      native handle
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

    The 4 KiB range costs 64 ns against 23 us for the whole value, so a range never materializes the value.

    | `glob` over a 16-leaf lake | estimate |
    | --- | ---: |
    | `**/*.parquet` | 57 us |
    | `year=2024/**/*.parquet` | 23 us |

=== "Python"

    The baseline is PyArrow's own calls against the same `LocalFileSystem`; `holder.py --min-time 0.2 --repeat 7`, release wheel, medians.

    ```text
                            wrapper      PyArrow
    bytes write            400.0 us     244.3 us
    bytes read             115.2 us      17.6 us
    range read (4 KiB)      14.5 us       2.9 us
    parquet write            8.16 ms      6.19 ms
    parquet read             2.00 ms      1.95 ms
    listing (16 entries)    85.5 us      34.4 us
    ```

    Each vtable call costs roughly 12 us fixed, per call not per byte.

=== "JavaScript"

    The handler crosses the boundary on every call; `bench:holder`, release build, throughput.

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

```bash
cargo bench --bench holder --features parquet -- fs_bytes
cargo bench --bench holder --features parquet -- fs_record
cargo bench --bench holder --features parquet -- fs_listing
python/.venv/bin/python python/benchmarks/holder.py --min-time 0.2 --repeat 7
npm run --prefix node bench:holder
```
