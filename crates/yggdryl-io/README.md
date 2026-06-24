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
- `Io: ReadBytes + Seek` — the base handle. Every handle has a `url()` (in-memory
  ones use `mem://<address>`); it reads/writes at a position via `pread` /
  `pwrite` (a `Whence` selects positional — cursor untouched, the default — vs
  cursor-relative), manages storage with `capacity` / `reserve_capacity` /
  `truncate` (`Unsupported` on read-only backends), carries an access `mode`
  (`Mode`) and optional `parent`, can `open` derived handles and `close` them
  (idempotent no-op by default), exposes
  `as_slice` (zero-copy hook), reports `stats`, and `copy_to` (transfer with a
  memory fast path; `copy` is the free fn).
- `Mode` — `Read` / `Write` / `Append` / `ReadWrite`, parsed from Python mode
  strings (`Mode::from_str`: `r`, `rb`, `a`, `r+`, `w+`, `append`, …).
- `IoStats` — `kind` (missing / file / directory / other), `size`, `mtime`,
  `content_type`, `etag` eager; `media_type` discovered lazily (and cached) under
  the `media` feature.
- `Path: Io` — a local, hierarchical resource. `LocalPath` is a filesystem
  **instance**: `open` stats the path up front (holding `url` / `stats`, never
  failing — a missing path reports `kind == Missing`) and memory-maps the file
  *lazily* on first read (zero-copy under the `mmap` feature). Its instance
  `write` auto-creates missing parent dirs *lazily* (only after a `NotFound`
  failure, then retry — never a stat up front). It shares `BytesIO`'s `stream`
  flag, Python-style `read`/`read_line` and `open` semantics, so the two behave
  identically.
- `RemotePath: Io` — the URL-addressed cloud sibling (flat keys, no dir
  creation). Concrete S3 / Azure paths are downstream crates implementing it.
- `Codec<T>` — typed read/write/stream of values over any byte handle; `Frames`
  is the reference length-delimited codec. Byte-stream **compression** (gzip /
  Zstd / Snappy) lives in the sibling [`yggdryl-compression`](../yggdryl-compression)
  crate, which wraps any handle from here.

```rust
use yggdryl_io::{BytesIO, Io, Whence};

let mut io = BytesIO::from_bytes(b"hello world".to_vec());

// Positional read at an offset, leaving the cursor untouched.
let mut footer = [0u8; 5];
io.pread(&mut footer, 6, Whence::Start).unwrap();
assert_eq!(&footer, b"world");

// Streamed access from the cursor; every handle also has a URL and stats.
assert_eq!(io.read(Some(5)), b"hello");
assert_eq!(io.url().scheme(), "mem");
assert_eq!(io.stats().unwrap().size(), 11);
```

`LocalPath` is a filesystem `Path` instance — `open` never fails (it stats the
path), and the file is mapped lazily under `mmap`:

```rust,ignore
use yggdryl_io::{copy, Io, LocalPath};

let mut src = LocalPath::open("data.parquet"); // infallible: holds url + stats
let mut buf: Vec<u8> = Vec::new();
copy(&mut src, &mut buf).unwrap(); // zero-copy hand-off of the lazily-mapped file
```

## Features (off by default; the base build depends only on `yggdryl-url`)

- `log` — structured `log` events on the hot paths.
- `mmap` — `LocalPath` memory-maps files (zero-copy) instead of reading them.
- `media` — lazy `media_type()` discovery via `yggdryl-media`.

Run the benchmarks with `cargo bench -p yggdryl-io` (add `--features mmap`).
