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
    assert_eq!(handle.read_all()?, b"");

    handle.write_all_bytes(b"AAPL")?;
    handle.close()?;
    assert_eq!(handle.read_all()?, b"AAPL");

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

The first path segment is the authority, which is exactly what a bucket is, so `"bucket/key"` on an
S3 filesystem spells `s3://bucket/key`. In Python the filesystem may also be inferred from the
first argument: `IOBase(fs, "bucket/key")` means the same as `IOBase.from_arrow_fs(fs, "bucket/key")`.

The real thing looks like this, and needs credentials and a network, so it is shown rather than run:

=== "Python"

    ```python,ignore
    from pyarrow.fs import S3FileSystem
    from yggdryl import IOBase, iceberg

    handle = IOBase.from_arrow_fs(S3FileSystem(region="eu-west-1"), "bucket/table")
    table = iceberg.Table.open(handle)
    rows = table.scan().read_all()
    ```

## A write publishes when the handle closes

An Arrow filesystem replaces whole files. It has no random write - an object store cannot patch
five bytes in the middle of an object - while `IOBase::pwrite` is positional. So a leaf stages its
mutations in memory and publishes them as exactly one whole-value replacement on `flush` or
`close`. Until then the stored value is untouched:

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::arrowfs::{ArrowFileSystem, File, MemoryFileSystem};
    use yggdryl::io::IOBase;

    let filesystem = Arc::new(MemoryFileSystem::new());
    filesystem.write_full("bucket/trades.bin", b"stored")?;
    let mut handle = File::from_location(filesystem.clone(), "bucket/trades.bin")?;

    handle.write_all_bytes(b"pending")?;

    // The handle presents the pending value; the filesystem still has the old one.
    assert_eq!(handle.read_all()?, b"pending");
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

    handle.write_bytes(b"pending")
    assert not (root / "staged.bin").exists()

    handle.close()
    assert (root / "staged.bin").read_bytes() == b"pending"
    ```

That is why a file another reader will open is written inside a scope - `with` in Python - which
binds to exactly `open` and `close`. Reads need none of it: a read maps straight onto one ranged
fetch, so a footer-first reader such as Parquet never downloads an object to read its footer.

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

A folder handle reads as the partitioned table beneath it, and a folder holding an Iceberg metadata
document reads through its snapshots - both are the container behavior every backend inherits.

## Composing with the wrappers

Nothing about a foreign filesystem is special to the wrappers, because they only ever see an
`IOBase`. A content coding round trips over a bucket exactly as it does over a file:

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
    assert_eq!(coded.read_all()?, br#"{"symbol":"AAPL"}"#);

    // ...while what the filesystem holds is gzip.
    let stored = File::from_location(filesystem, "bucket/trades.json.gz")?;
    assert_eq!(&stored.read_all()?[..2], &[0x1f, 0x8b]);
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
    assert_eq!(handle.read_all()?, b"AAPL!");
    assert_eq!(handle.read_range(1, 3)?, b"APL");
    ```

In Python, write a `pyarrow.fs.FileSystemHandler` and wrap it in `pyarrow.fs.PyFileSystem`. That is
also how an `fsspec` filesystem arrives, so this one shape covers both:

=== "Python"

    ```python,ignore
    import pyarrow.fs as pafs
    from yggdryl import IOBase

    class MyHandler(pafs.FileSystemHandler):
        ...  # get_file_info, get_file_info_selector, open_input_file, open_output_stream, ...

    handle = IOBase.from_arrow_fs(pafs.PyFileSystem(MyHandler()), "bucket/key.parquet")
    ```

A working handler is longer than a documentation page wants, so the complete one lives in
`python/tests/test_arrowfs.py`, where it is exercised end to end - including an Iceberg table whose
every byte goes through it.

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/arrowfs-rust.ipynb){ download },
[Python](notebooks/arrowfs-python.ipynb){ download }.

<!-- /notebooks -->
