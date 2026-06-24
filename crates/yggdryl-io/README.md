# yggdryl-io

The **byte-IO foundation** for the
[**yggdryl**](https://github.com/Platob/yggdryl) project: one set of methods to
read, write, seek and stat bytes **wherever they live** — in memory, on a local
path, or (via downstream crates) in cloud object storage. It is the base buffer
layer for columnar formats such as Arrow / Parquet, mixing *random* access (read
a footer, a column chunk) with *streamed* access (scan record batches) over the
same handle.

## Layers

- `ReadBytes` / `WriteBytes` — byte source/sink primitives (`&[u8]`, `Vec<u8>`).
- `Seek` — the cursor: `seek` / `stream_position` / `stream_len`.
- `Io: ReadBytes + Seek` — the base handle: `read_at` (positioned read that does
  not move the cursor), `as_slice` (zero-copy hook), `stats`, and `copy_to`
  (transfer with a memory fast path). `copy` is the free-function form.
- `IoStats` — `size` / `mtime` / `content_type` / `etag` eager; `media_type`
  discovered lazily (and cached) under the `media` feature.
- `Path: Io` — a named resource; `LocalPath` is the filesystem backend,
  memory-mapping the file (zero-copy) under the `mmap` feature. Cloud paths (S3,
  Azure) are downstream crates implementing the same `Path` trait.
- `Codec<T>` — typed read/write/stream of values over any byte handle; `Frames`
  is the reference length-delimited codec.

```rust
use yggdryl_io::{BytesIO, Io, Seek, Whence};

let mut io = BytesIO::from_bytes(b"hello world".to_vec());

// Random access: read a slice at an offset without moving the cursor.
let mut footer = [0u8; 5];
io.read_at(6, &mut footer).unwrap();
assert_eq!(&footer, b"world");

// Streamed access from the cursor, plus lazy metadata.
assert_eq!(io.read(Some(5)), b"hello");
assert_eq!(io.stats().unwrap().size(), 11);
```

`LocalPath` is the filesystem `Path`, memory-mapped under `mmap`:

```rust,ignore
use yggdryl_io::{copy, Io, LocalPath};

let mut src = LocalPath::open("data.parquet").unwrap();
let mut buf: Vec<u8> = Vec::new();
copy(&mut src, &mut buf).unwrap(); // zero-copy hand-off of the mapping
```

## Features (off by default — the default build is dependency-free)

- `log` — structured `log` events on the hot paths.
- `mmap` — `LocalPath` memory-maps files (zero-copy) instead of reading them.
- `media` — lazy `media_type()` discovery via `yggdryl-media`.

Run the benchmarks with `cargo bench -p yggdryl-io` (add `--features mmap`).
