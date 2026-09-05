# Filesystems

`yggdryl::holder::fs::FileSystem` is the one Arrow-compatible storage seam, and [`IOBase`](../index.md) is the one bound handle above it.

## Contract

| | |
| --- | --- |
| Owns | filesystem metadata, directory and file lifecycle, native copy and move, stateful byte streams |
| Not owned | Iceberg catalogs, reads, commits, caches, and their `FileIO` adapters stay downstream |
| Paths | `from_fs` never parses, decodes, or normalizes `path`, even when it holds `://` |
| URIs | `from_uri` is the only boundary where a URI chooses and configures a filesystem |
| Identity | `same_location` / `sameLocation` needs filesystem equality plus byte-for-byte path equality |
| Streams | four opens, one retained backend stream each; output streams rather than buffering the whole object |
| Secrets | `uri` may carry them; `masked_uri` / `maskedUri` is the credential-free spelling |
| Ships | `MemoryFileSystem` and `LocalFileSystem`, complete references for the public object-safe `FileSystem` trait |
| Bindings | Python over `pyarrow.fs` returning `NativeFile`; JavaScript over a synchronous handler protocol |
| Feature flag | none; tests gate on `arrow` and `iceberg` |

## Use

Pass the filesystem and its opaque path separately.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use yggdryl::holder::fs::{
        File, FileSystem, MemoryFileSystem, OutputMetadata,
    };

    # fn main() -> yggdryl::Result<()> {
    let filesystem: Arc<dyn FileSystem> = Arc::new(MemoryFileSystem::new());
    let file = File::from_path(
        filesystem,
        "bucket/v=a%2Fb.bin",
        Some("s3://bucket/v=a%2Fb.bin".to_owned()),
    )?;
    let metadata = OutputMetadata::from_entries([
        ("content-type", "application/octet-stream"),
    ]);
    let mut output = file.open_output_stream(Some(&metadata))?;
    output.write(b"literal")?;
    output.close()?;
    # Ok(())
    # }
    ```

=== "Python"

    ```python
    import pyarrow.fs as pafs
    from yggdryl import IOBase

    filesystem = pafs._MockFileSystem()
    handle = IOBase.from_fs(
        filesystem,
        "bucket/v=a%2Fb.bin",
        uri="s3://bucket/v=a%2Fb.bin",
    )

    with handle.open_output_stream(
        compression=None,
        metadata={"content-type": "application/octet-stream"},
    ) as output:
        output.write(b"literal")

    with handle.open_input_file() as source:
        source.seek(2)
        assert source.read(3) == b"ter"

    assert handle.filesystem is filesystem
    assert handle.path == "bucket/v=a%2Fb.bin"
    assert handle.uri == "s3://bucket/v=a%2Fb.bin"
    assert handle.masked_uri == "s3://bucket/v=a%2Fb.bin"
    ```

The filesystem receives the literal object name `bucket/v=a%2Fb.bin`, and the percent escape is not decoded into a slash. The same rule preserves `%25`, `+`, repeated slashes, non-ASCII text, and a literal `://`.

## Resolve a URI once

Use the dedicated URI boundary only when the URI must choose and configure a filesystem.

=== "Python"

    ```python
    from yggdryl import IOBase

    local = IOBase.from_uri("file:///tmp/events.bin")

    s3 = IOBase.from_uri(
        "s3://bucket/v=a%2Fb"
        "?endpoint_override=minio%3A9000"
        "&scheme=http"
        "&region=eu-west-1",
        options={
            "anonymous": True,
            "force_path_style": True,
        },
    )

    assert s3.path == "bucket/v=a%2Fb"
    ```

Yggdryl parses the URI with its own [`Uri` and `Url`](../../uri/index.md) implementation, and the authority supplies credentials, endpoint, bucket, region, and addressing. One raw path slice is retained, so object-key escape spelling is never reconstructed or decoded.

| Input | Filesystem configuration | Bound path |
| --- | --- | --- |
| `s3://bucket/key` | default S3 | `bucket/key` |
| `s3a://bucket/key` / `s3n://bucket/key` | default S3 | `bucket/key` |
| `s3://key:secret@bucket/key` | credentials from user information | `bucket/key` |
| `s3://key:secret@minio:9000/bucket/key` | endpoint `minio:9000` | `bucket/key` |
| `s3://bucket/key?endpoint_override=minio%3A9000&scheme=http&region=eu-west-1` | explicit endpoint, transport, region | `bucket/key` |
| `s3://bucket.s3.eu-west-1.amazonaws.com/key` | virtual addressing, inferred region | `bucket/key` |

The `options` mapping overrides URI query configuration, and Python forwards it to `pyarrow.fs.S3FileSystem`. Supported S3 settings are access key, secret key, session token, endpoint override, transport, region, anonymous mode, and addressing.

## Bound facts and identity

Every parent, child, listing result, and glob result retains four facts.

- the same filesystem equality domain;
- the exact raw filesystem path;
- the exact optional caller URI; and
- a credential-free diagnostic URI.

`uri` is explicit because it can carry secrets, so errors, logs, and snapshots use `masked_uri` or `maskedUri`. Repr and debug output never reveal user information, secret keys, or session tokens.

## Stream lifetime

The four opens have distinct contracts.

| Open | Capability |
| --- | --- |
| `open_input_file` / `openInputFile` | random read, read-at, seek, tell, close |
| `open_input_stream` / `openInputStream` | sequential read, tell, close |
| `open_output_stream` / `openOutputStream` | truncating streamed write, flush, tell, close |
| `open_append_stream` / `openAppendStream` | streamed append, flush, tell, close |

Each open retains one backend stream, so a read never repeats metadata lookup or reopens the file. Always close the stream, or use Python context management.

## Errors and metadata

Metadata keeps size and modification time optional in Rust.

| Binding | Metadata |
| --- | --- |
| Rust | `FileInfo`, size and modification time optional |
| Python | `pyarrow.fs.FileInfo` from `info()` |
| JavaScript | `ArrowFileInfo`, `bigint` size and UTC mtime in nanoseconds |

A zero-byte file, an unknown mtime, and an absent path therefore stay three different states.

## Copy and move

=== "Python"

    ```python
    copied = source.copy_into(target)
    moved = target.move_into(archive)

    assert copied == source.info().size
    assert moved.same_location(archive)
    ```

Equal filesystems receive exactly one native `copy_file` or `move` call and no client-side byte stream. A cross-filesystem move copies completely before deleting the source.

## Directory and file lifecycle

| Operation | Result |
| --- | --- |
| `create_dir(recursive)` | create with the backend's exact recursive policy |
| `delete_dir()` | remove an empty directory itself; refuse a non-empty directory |
| `delete_dir_contents(missing_dir_ok)` | remove descendants and retain the selected directory |
| `delete_root_dir_contents()` | explicitly clear the filesystem root only |
| `delete_file()` | remove a file; refuse a directory |
| `remove(recursive=False)` | remove the selected resource; recursion removes a directory root and descendants |
| `clear()` | preserve the selected file/directory and empty its contents |

Root-content deletion is unreachable through an empty or broad ordinary delete. The explicit root method accepts only a handle whose raw bound path is exactly empty.

## JavaScript handler protocol

Arrow JS supplies no filesystem backend, so the package exports synchronous protocols. They are `FileSystemHandler`, `FileSelector`, `ArrowFileInfo`, `RandomAccessReader`, `ByteReader`, `ByteWriter`, `OutputMetadata`, and typed filesystem errors.

=== "JavaScript"

    ```typescript
    const source = IOBase.fromFs(handler, "bucket/v=a%2Fb.bin")
    const target = IOBase.fromFs(otherHandler, "archive/v=a%2Fb.bin")

    using input = source.openInputFile()
    const header = input.readAt(0n, 8n)

    source.copyInto(target)
    ```

Handler calls stay synchronous and on the JavaScript isolate that supplied the handler. Sizes, offsets, and nanosecond mtimes use `bigint`.

## Edges

- `from_fs(fs, "bucket/v=a%2Fb.bin")` -> the filesystem receives that literal name; `%2F` never becomes a slash.
- `%25`, `+`, repeated slashes, non-ASCII text, a literal `://` -> preserved byte for byte in the bound path.
- custom Rust filesystem -> implement the same public object-safe `FileSystem` trait; nothing above the seam changes.
- `same_location` / `sameLocation` -> true only for filesystem equality plus byte-for-byte path equality.
- same type name and path -> not identity; subtree, endpoint, credential, and handler instances may expose different bytes.
- second `close()` -> idempotent, flushes at most once, and retains the first write, flush, or close failure.
- Python stream -> the original `pyarrow.NativeFile`, forwarding `compression`, `buffer_size`, and output `metadata` exactly.
- `IOCursor` -> satisfies Python binary-file behavior, so `pyarrow.PythonFile` wraps it without materializing the resource.
- metadata lookup on a missing path -> Arrow `NotFound`; only lookup represents absence that way.
- strict open or mutation -> preserves not-found, permission, already-exists, not-a-directory, is-a-directory, directory-not-empty, unsupported, and transport failures.
- high-level read of an absent resource -> empty for typed not-found only; permission and transport errors still raise.
- listing -> ascending by exact path; it yields its first error and then stays exhausted.
- native recursive walk -> loads at most one directory before yielding; a foreign eager result is sorted once.
- missing selector base -> raises unless `allow_not_found` / `allowNotFound` is true.
- glob -> uses the longest fixed prefix and preserves every bound-location fact; see [Partitions](../iobase/partitions.md).
- cross-filesystem copy -> one input and one output stream, bounded chunks, published only after complete success.
- failed or missing source -> the temporary output is removed, so an existing target is never truncated.
- backend without root deletion, append, or another optional capability -> typed `Unsupported`; yggdryl never emulates it.
- `from_uri` with `s3://` in JavaScript -> validated and resolved, then `Unsupported`; bind an S3 handler with `fromFs`.
- JavaScript stream -> explicit `close()` plus optional `Symbol.dispose`, so `using` releases it at scope end.

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

The benchmark times the wrapper against direct PyArrow, local, or native local operations. Conformance tests, not timing, are the evidence for streaming correctness.

| gate | value |
| --- | ---: |
| benchmark payload | at least 64 MiB |
| chunk sizes | identical on both legs |
| warm-up | before every median |
| benchmark failure | more than 25% slower |
| retained streams | one per transfer |
| retained payload | bounded |
| same-filesystem copy or move | zero stream operations |
| native recursive listing | one directory at a time |

```bash
cargo bench --bench holder --features parquet -- fs_bytes
cargo bench --bench holder --features parquet -- fs_record
cargo bench --bench holder --features parquet -- fs_listing
python/.venv/bin/python python/benchmarks/holder.py --min-time 0.2 --repeat 7
npm run --prefix node bench:holder
```
