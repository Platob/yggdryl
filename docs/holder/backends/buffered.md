# Buffered

A page cache over any [`IOBase`](../iobase/bytes.md) handle, with the value's first and last pages pinned.

## Contract

| | |
| --- | --- |
| Owns | Rust `Buffered<H>`; Python `buffered(page_size=, max_bytes=, ttl=)` and JavaScript `buffered({ pageSize, maxBytes, ttlMs })` return the same handle |
| Mirrors | `size`, `url`, `media_type`, `kind`, `parent`, `child_by_path`, `ls` |
| Lazy | Nothing cached at construction; a pinned page fills on the first read that wants it |
| Page size | Default 64 KiB; rounded up to a power of two, clamped to `64 ..= 1 GiB` |
| Max bytes | Default 8 MiB, pinned pages included; clamped up to two pages; least recently read page evicted first |
| TTL | Default 30 s from last access; Python `ttl` in seconds, JavaScript `ttlMs` in milliseconds; lazy expiry, no background thread |
| Pinned | First page and the page holding the current last byte; never evicted or expired; follows a moved end |
| Writes | `pwrite` writes through, then patches or drops overlapped pages; `truncate` invalidates at or past the new size; `flush` delegates |
| Drops the cache | `close`, `clear`, `remove`, `handle_mut`, `clear_cache`, `into_handle` |
| Re-wrapping | `buffered` again, in any binding or on a `Holder`, reconfigures the one cache; never stacks |

## Use

=== "Rust"

    ```rust
    use yggdryl::holder::buffered::BufferedOptions;
    use yggdryl::IOBase;
    use yggdryl::holder::Buffer;

    let handle = Buffer::from_bytes(b"symbol,price\nAAPL,1\n".to_vec())
        .buffered(BufferedOptions::default());

    assert_eq!(handle.read_range_bytes(0, 6)?, b"symbol");
    assert_eq!(handle.read_range_bytes(13, 4)?, b"AAPL");
    assert_eq!(handle.cached_pages(), 1);
    ```

=== "Python"

    ```python
    from yggdryl import IOBase

    handle = IOBase.from_bytes(b"symbol,price\nAAPL,1\n")
    assert handle.buffered(page_size=64, max_bytes=256, ttl=30.0) is handle
    assert handle.read_range_bytes(0, 6) == b"symbol"
    assert handle.read_range_bytes(13, 4) == b"AAPL"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { IOBase } = require('yggdryl')

    const handle = IOBase.fromBytes(Buffer.from('symbol,price\nAAPL,1\n'))
    assert.equal(handle.buffered({ pageSize: 64, maxBytes: 256, ttlMs: 30_000 }), handle)
    assert.deepEqual(handle.readRangeBytes(0, 6), Buffer.from('symbol'))
    assert.deepEqual(handle.readRangeBytes(13, 4), Buffer.from('AAPL'))
    ```

## Pages

A miss fetches one aligned page and copies the range out; a hit copies from the held page.

```rust
use yggdryl::holder::buffered::BufferedOptions;
use yggdryl::IOBase;
use yggdryl::holder::Buffer;

let handle = Buffer::from_bytes(vec![4_u8; 4_096]).buffered(BufferedOptions::default());

// The first read fetches the page holding the range; the second is memory.
assert_eq!(handle.read_range_bytes(0, 8)?, [4_u8; 8]);
assert_eq!(handle.read_range_bytes(2_000, 8)?, [4_u8; 8]);
assert_eq!(handle.cached_pages(), 1);
```

A read crossing pages copies each page straight into the caller's buffer.

```rust
use yggdryl::holder::buffered::{Buffered, BufferedOptions};
use yggdryl::IOBase;
use yggdryl::holder::Buffer;

// 256-byte pages, so a 1 KiB value is four of them.
let options = BufferedOptions::default().with_page_size(256);
let handle = Buffered::new(Buffer::from_bytes(vec![7_u8; 1_024]), options);

// A read that misses fetches the whole page holding it, aligned.
assert_eq!(handle.read_range_bytes(300, 4)?.len(), 4);
assert_eq!(handle.cached_pages(), 1);
assert_eq!(handle.cached_bytes(), 256);
assert!(handle.has_cached_page(1));

// A read spanning pages assembles from each of them, caching all it crossed.
assert_eq!(handle.read_range_bytes(100, 600)?.len(), 600);
assert_eq!(handle.cached_pages(), 3);

// The page a given offset lives in is arithmetic, not a lookup.
assert_eq!(handle.options().page_index(700), 2);
assert_eq!(handle.options().page_start(2), 512);
```

## The three knobs

```rust
use std::time::Duration;

use yggdryl::holder::buffered::BufferedOptions;

let options = BufferedOptions::default();
assert_eq!(options.page_size(), 64 * 1024);
assert_eq!(options.max_bytes(), 8 * 1024 * 1024);
assert_eq!(options.ttl(), Duration::from_secs(30));

// A page size is rounded up to a power of two and clamped to 64 ..= 1 GiB.
assert_eq!(BufferedOptions::default().with_page_size(1_000).page_size(), 1_024);
assert_eq!(BufferedOptions::default().with_page_size(0).page_size(), 64);

// A budget below two pages is clamped up to exactly two, never rejected,
// because the two pinned pages have to fit for the cache to work at all.
let tight = BufferedOptions::default().with_page_size(4_096).with_max_bytes(1);
assert_eq!(tight.max_bytes(), 8_192);

// Raising the page size re-applies that clamp to a budget set earlier.
let grown = BufferedOptions::default()
    .with_page_size(1_024)
    .with_max_bytes(4_096)
    .with_page_size(8_192);
assert_eq!(grown.max_bytes(), 16_384);
```

## Both ends are pinned

Both ends carry discovery: magic bytes at the head, a Parquet footer or Arrow IPC schema at the tail.

```rust
use yggdryl::holder::buffered::{Buffered, BufferedOptions};
use yggdryl::IOBase;
use yggdryl::holder::Buffer;

// Sixteen pages of value, four pages of budget.
let options = BufferedOptions::default()
    .with_page_size(64)
    .with_max_bytes(4 * 64);
let handle = Buffered::new(Buffer::from_bytes(vec![1_u8; 16 * 64]), options);

// The footer first, then the header: the shape a container is opened with.
handle.read_range_bytes(16 * 64 - 8, 8)?;
handle.read_range_bytes(0, 8)?;

// Then a scan of the middle, four times what the budget can hold.
for page in 1..15 {
    handle.read_range_bytes(page * 64, 8)?;
}

// The budget held throughout, the middle was evicted, and both ends stayed.
assert!(handle.cached_bytes() <= handle.options().max_bytes());
assert!(handle.has_cached_page(0));
assert!(handle.has_cached_page(15));
assert!(!handle.has_cached_page(7));
```

Pinned pages count toward the budget, hence the two-page clamp. A moved end releases the old last page; the new one is pinned when next cached.

```rust
use yggdryl::holder::buffered::{Buffered, BufferedOptions};
use yggdryl::IOBase;
use yggdryl::holder::Buffer;

let options = BufferedOptions::default()
    .with_page_size(64)
    .with_max_bytes(4 * 64);
let mut handle = Buffered::new(Buffer::from_bytes(vec![1_u8; 4 * 64]), options);

// Page 3 ends the value, so it holds a pin.
assert_eq!(handle.read_all_bytes()?.len(), 4 * 64);
assert!(handle.has_cached_page(3));

// A write doubling the value moves the end; page 3 is ordinary again, and a
// scan under budget pressure now evicts it while page 0 stays.
handle.pwrite(8 * 64 - 1, b"z")?;
for page in 4..8 {
    handle.read_range_bytes(page * 64, 8)?;
}
handle.read_range_bytes(5 * 64, 8)?;
handle.read_range_bytes(6 * 64, 8)?;
assert!(handle.has_cached_page(0));
assert!(handle.has_cached_page(7));
assert!(!handle.has_cached_page(3));
```

## Writes are never stale

```rust
use yggdryl::holder::buffered::BufferedOptions;
use yggdryl::IOBase;
use yggdryl::holder::Buffer;

let mut handle = Buffer::from_bytes(b"symbol,price\nAAPL,1\n".to_vec())
    .buffered(BufferedOptions::default());
assert_eq!(handle.read_all_bytes()?.len(), 20);

// A write goes straight to the wrapped handle and folds into the pages it
// overlapped, so the read after it can never see the bytes it replaced.
handle.pwrite(13, b"MSFT")?;
assert_eq!(handle.read_range_bytes(13, 4)?, b"MSFT");
assert_eq!(handle.handle().as_slice()[13..17], *b"MSFT");

// Truncating drops every page a resize could have changed, both ways.
handle.truncate(13)?;
assert_eq!(handle.read_all_bytes()?, b"symbol,price\n");
handle.truncate(15)?;
assert_eq!(handle.read_all_bytes()?, b"symbol,price\n\0\0");
```

`clear` and `remove` drop the cache before delegating, so no page outlives a failed removal. `close` flushes, drops every page, and leaves a working handle.

```rust
use yggdryl::holder::buffered::BufferedOptions;
use yggdryl::IOBase;
use yggdryl::holder::Buffer;

let mut handle = Buffer::from_bytes(vec![3_u8; 4_096]).buffered(BufferedOptions::default());
assert_eq!(handle.read_all_bytes()?.len(), 4_096);
assert_eq!(handle.cached_pages(), 1);

handle.close()?;
assert_eq!(handle.cached_pages(), 0);
assert_eq!(handle.read_range_bytes(0, 4)?, [3, 3, 3, 3]);
```

## Over a compressed handle

A closed [`Coded`](../../coding/index.md) read decodes through the range and retains nothing; wrapping it retains decoded pages instead.

```rust
use yggdryl::holder::buffered::BufferedOptions;
use yggdryl::coding::gzip::Gzip;
use yggdryl::IOBase;
use yggdryl::holder::Buffer;

let payload = "symbol,price\nAAPL,1\n".repeat(512).into_bytes();
let mut source = Gzip::new(Buffer::new());
source.write_all_bytes(&payload)?;
source.flush()?;
let encoded = source.into_handle()?;

// The cache wraps the coding, so the pages it holds are decoded bytes.
let handle = Gzip::new(encoded).buffered(BufferedOptions::default());
assert_eq!(handle.read_range_bytes(0, 6)?, b"symbol");
assert_eq!(handle.read_range_bytes(13, 4)?, b"AAPL");

// Two reads, one page, one decode.
assert_eq!(handle.cached_pages(), 1);
assert_eq!(handle.size(), payload.len() as u64);
```

- `Buffered<Coded<_>>` caches decoded bytes; `Coded<Buffered<_>>` caches encoded transport.
- [`pstream_bytes`](../iobase/bytes.md) keeps one decoder and zero pages for a scan; `open` materializes once for repeated random access.
- A page miss still restarts at the frame beginning, so later misses cost more.

## Wrapping twice wraps once

```rust
use yggdryl::holder::buffered::BufferedOptions;
use yggdryl::holder::Holder;
use yggdryl::IOBase;
use yggdryl::holder::Buffer;

let once = Buffer::from_bytes(vec![5_u8; 128]).buffered(BufferedOptions::default());

// `Buffered` has an inherent `buffered`, which wins method resolution, so
// this re-wraps the handle it holds instead of stacking a second cache.
let twice = once.buffered(BufferedOptions::default().with_page_size(512));
assert_eq!(twice.options().page_size(), 512);
assert_eq!(twice.read_range_bytes(0, 4)?, [5, 5, 5, 5]);

// A holder does the same, so a listing entry can be buffered without care.
let held = Holder::buffer(Buffer::from_bytes(vec![5_u8; 128]))
    .buffered(BufferedOptions::default())
    .buffered(BufferedOptions::default());
assert!(matches!(&held, Holder::Buffered(inner) if matches!(inner.handle(), Holder::Buffer(_))));

// `into_handle` gives the wrapped handle back, cache dropped.
let inner: Buffer = twice.into_handle();
assert_eq!(inner.size(), 128);
```

## Cursors ride the cache

`cursor` and `cursor_at` come from [`IOBase`](../iobase/bytes.md), so `Read`, `Write`, and `Seek` go through the pages.

```rust
use std::io::{Read, Seek, SeekFrom};

use yggdryl::holder::buffered::BufferedOptions;
use yggdryl::{IOBase, IOCursor};
use yggdryl::holder::Buffer;

let payload: Vec<u8> = (0..1_024_u32).map(|index| index as u8).collect();
let mut cursor = Buffer::from_bytes(payload)
    .buffered(BufferedOptions::default().with_page_size(256))
    .cursor();

// Sequential reads stream across page boundaries through the cache.
let mut chunk = [0_u8; 300];
cursor.read_exact(&mut chunk)?;
assert_eq!(cursor.tell(), 300);
assert_eq!(chunk[299], 299_u32 as u8);

// A seek to the end lands on the pinned footer page. `IOCursor` and
// `std::io::Seek` both spell `seek`, so this one names the trait it means.
IOCursor::seek(&mut cursor, SeekFrom::End(-4))?;
cursor.read_exact(&mut chunk[..4])?;
assert_eq!(chunk[3], 1_023_u32 as u8);
assert_eq!(cursor.handle().cached_pages(), 3);
```

## A file, and what the cache is for

Over a memory-mapped [local file](local.md) a `pread` is already a `memcpy`, so the wrapper costs a lock and a copy. It pays where a fetch is real: an [`fs`](filesystems.md) handle calls the vtable per read, and a [coded](../../coding/index.md) handle decodes.

```rust
use yggdryl::holder::buffered::BufferedOptions;
use yggdryl::IOBase;
use yggdryl::holder::local::{File, Folder};

let path = Folder::temporary()?.path()?.join(format!("yggdryl-doc-buffered-{}.bin", std::process::id()));
std::fs::write(&path, vec![9_u8; 4_096])?;

let handle = File::new(&path)?.buffered(BufferedOptions::default().with_page_size(1_024));
assert_eq!(handle.size(), 4_096);
assert_eq!(handle.read_range_bytes(2_000, 8)?, [9_u8; 8]);
assert_eq!(handle.cached_pages(), 1);

// The wrapper is the file for every purpose but the reading.
let bare = File::new(&path)?;
assert_eq!(handle.url(), bare.url());

drop(handle);
let _ = std::fs::remove_file(&path);
```

## Edges

- `with_page_size(1_000)` -> 1024; `with_page_size(0)` -> 64.
- `with_max_bytes(1)` over 4 KiB pages -> 8192; clamped, never rejected, and re-applied when the page size grows.
- A hit -> no call on the inner handle; the size is remembered and re-asked only when a read runs past it.
- Bytes written behind the cache -> invisible; `handle_mut` and `clear_cache` drop every page first.
- `Coded<Buffered<_>>` -> caches compressed bytes and still decodes on every read.
- `cached_pages`, `cached_bytes`, `has_cached_page`, `options` -> Rust only; the bindings expose no page inspection.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib holder::buffered::
    cargo test --features "parquet iceberg" -p yggdryl --lib iobase::tests::buffered_handle
    cargo bench --bench holder --features parquet -- io_buffered
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/holder/test_io.py -k buffered
    python/.venv/bin/python python/benchmarks/holder/io.py --iterations 10000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern buffered "node/tests/holder/io.test.js"
    npm run --prefix node bench:holder:io
    ```

## Performance

`io_buffered` runs three workloads over one 16 MiB fixture and every shipped handle: one containerized x86_64 Linux run, group alone, Criterion, 100 samples, medians. Run-to-run spread reaches 15%, so read the multiples, never the percentages.

| workload | shape |
| --- | --- |
| `random` | 512-byte reads in a 4 MiB hot region under the 8 MiB budget |
| `sequential` | one 16 MiB pass in 8 KiB steps, twice the budget, nothing re-read |
| `footer` | both ends, a 12 MiB middle sweep, both ends again |

`buffer`, `file`, and `fs_memory` are already memory; `fs_local` pays an `open`, `seek`, and `read` per `pread` and a `stat` per `size`.

```text
                                     random        sequential        footer
buffer                             10.041 µs        606.58 µs      439.58 µs
file                               28.037 µs        517.80 µs      367.66 µs
buffered  (over file)              75.362 µs      1.9495 ms        507.49 µs
fs_memory                     46.739 µs        734.30 µs      392.83 µs
fs_memory_buffered            77.633 µs      2.0068 ms        503.69 µs
fs_local                     1.0832 ms       2.7574 ms       2.0924 ms
fs_local_buffered             73.668 µs      2.3841 ms        532.18 µs
```

Over a backend that is already memory the cache costs 2.7x (`random`, against `file`); over one that fetches, 4x to 15x.

| Workload | `fs_local` | with the cache | |
| --- | --- | --- | --- |
| `random` (a hot region, re-read) | 1.0832 ms | 73.668 µs | **14.7x faster** |
| `footer` (both ends, big middle) | 2.0924 ms | 532.18 µs | **3.9x faster** |
| `sequential` (one pass, nothing re-read) | 2.7574 ms | 2.3841 ms | **1.2x faster** |

Pinning is asserted as a count: after a 12 MiB middle scan, re-reading both ends costs zero inner fetches.

### Coded cases

The `coded` cases read a 256 KiB gzip value in 64 reads of 4 KiB.

```text
io_buffered/coded/closed      12.182 ms    20.522 MiB/s
io_buffered/coded/open         5.8867 us   41.474 GiB/s
io_buffered/coded/buffered    10.2080 us   23.916 GiB/s
```

- `closed` restarts the decoder: 64 calls create 64 decoders and discard growing prefixes.
- `open` serves the one decoded snapshot it owns; fastest when holding the whole value is acceptable.
- `buffered` retains only fetched decoded pages and stays within 2x of `open` here.

```bash
cargo bench --bench holder --features parquet -- io_buffered
```
