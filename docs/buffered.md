# Buffered handles

A page cache that makes any [`IOBase`](io.md) handle buffered, with the value's first and last pages pinned.

=== "Rust"

    ```rust
    use yggdryl::buffered::BufferedOptions;
    use yggdryl::io::{Buffer, IOBase};

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

`IOBase::buffered` wraps any handle, and what comes back is a handle: `Buffered<H>` mirrors
everything it does not change, so `size`, `url`, `media_type`, `kind`, `parent`, `child_by_path`,
and `ls` answer exactly what the wrapped handle answers. It is invisible except for speed,
the same way [`Coded`](io.md) is invisible except for the coding.

Nothing is cached at construction. Per the [laziness contract](io.md), a handle is a
description of where bytes would live, and the cache only ever holds pages a read asked for.
Calling `buffered` again reconfigures the same cache layer; it never stacks a second one.
Rust additionally exposes page-inspection methods used by the detailed examples below.

## Pages

```rust
use yggdryl::buffered::{Buffered, BufferedOptions};
use yggdryl::io::{Buffer, IOBase};

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

A miss reads page-aligned from the inner handle and copies the requested range out; a hit
copies straight from the held page. A read crossing several pages copies each of them into
the caller's buffer directly, so nothing is assembled in between.

## The three knobs

```rust
use std::time::Duration;

use yggdryl::buffered::BufferedOptions;

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

**Page size** is the unit a miss fetches. **Max bytes** is the budget every cached page
shares, pinned ones included; over it, the least recently *read* page leaves first.
**Time to live** is counted from a page's last access, so a page that keeps being read
never expires. Expiry is lazy - a lapsed page is discarded when it is touched, and a miss
sweeps the table - so nothing here runs a background thread.

## Both ends are pinned

The first page, and the page holding the current last byte, are exempt from eviction and
from expiry. Both ends of a value are where discovery lives - magic bytes and media-type
sniffing at the head, a Parquet footer or an Arrow IPC schema and end-of-stream block at
the tail - and they are re-read constantly, so they must never be what a sweep takes.

```rust
use yggdryl::buffered::{Buffered, BufferedOptions};
use yggdryl::io::{Buffer, IOBase};

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

Two consequences worth stating, because both are observable:

- **Pinned pages count toward the budget.** That is why `max_bytes` is clamped to at least
  two pages: a budget that could not hold both ends would leave the cache thrashing.
- **The pin follows the current end.** A write or a `truncate` that moves the size releases
  the page that used to be last, and the new last page is pinned the next time it is
  cached. Pinning is a retention guarantee, never a prefetch: a pinned page is still filled
  lazily, by the first read that wants it.

```rust
use yggdryl::buffered::{Buffered, BufferedOptions};
use yggdryl::io::{Buffer, IOBase};

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
use yggdryl::buffered::BufferedOptions;
use yggdryl::io::{Buffer, IOBase};

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

`pwrite` is write-through: the bytes reach the inner handle first, and then every cached
page they overlapped is patched with them or dropped. `truncate` delegates and invalidates
everything at or past the new size. `flush` delegates. `close` flushes and drops the whole
cache - pinned pages included, because closing releases cached state - and leaves a working
handle behind that simply fetches again.

`clear` and `remove` drop the whole cache too, and they drop it **before** delegating: a page
that outlived either would answer a later read with bytes that are gone, and one that outlived a
*failed* removal would describe a resource whose state is no longer known. That ordering is why
they are written out rather than delegated by the macro - a body the macro provides cannot be
overridden, and a cache wrapper has to invalidate as part of the call.

```rust
use yggdryl::buffered::BufferedOptions;
use yggdryl::io::{Buffer, IOBase};

let mut handle = Buffer::from_bytes(vec![3_u8; 4_096]).buffered(BufferedOptions::default());
assert_eq!(handle.read_all_bytes()?.len(), 4_096);
assert_eq!(handle.cached_pages(), 1);

handle.close()?;
assert_eq!(handle.cached_pages(), 0);
assert_eq!(handle.read_range_bytes(0, 4)?, [3, 3, 3, 3]);
```

The one thing the cache cannot see is a change made *behind* it - bytes written straight to
the inner handle, or to the same file by another process. `handle_mut` therefore drops every
page before it hands the inner handle over, and `clear_cache` is the same thing said
explicitly.

## Over a compressed handle

A content coding is not seekable. A closed [`Coded`](io.md) positional read decodes through
the requested range and retains nothing. Wrapping it retains the decoded pages instead, so a
hit performs no second decode and a miss restarts only as far as that page.

```rust
use yggdryl::buffered::BufferedOptions;
use yggdryl::gzip::Gzip;
use yggdryl::io::{Buffer, IOBase};

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

Three things follow, and the [measurements below](#what-the-cache-buys-and-what-it-costs) cover all of them:

- **The order of wrapping matters.** `Buffered<Coded<_>>` caches decoded bytes;
  `Coded<Buffered<_>>` caches encoded transport.
- **Choose by access shape.** `pstream_bytes` keeps one decoder and zero pages for a scan;
  `Buffered<Coded<_>>` retains bounded decoded pages for locality; `open` materializes once
  for repeated random access and `close` releases it.
- **A page miss still starts at the frame beginning.** Compression has no decoded seek, so
  later misses cost more than early ones even though none retains the whole value.

## Wrapping twice wraps once

```rust
use yggdryl::buffered::BufferedOptions;
use yggdryl::generic::Holder;
use yggdryl::io::{Buffer, IOBase};

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

```rust
use std::io::{Read, Seek, SeekFrom};

use yggdryl::buffered::BufferedOptions;
use yggdryl::io::{Buffer, IOBase, IOCursor};

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

`cursor` and `cursor_at` come from [`IOBase`](io.md) itself, so a buffered handle gets the
same `Read`, `Write`, and `Seek` implementations every other handle gets - reads and writes
just go through the pages.

## A file, and what the cache is for

```rust
use yggdryl::buffered::BufferedOptions;
use yggdryl::io::IOBase;
use yggdryl::local::{File, Folder};

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

Over a memory-mapped [local file](local.md), a `pread` is already a `memcpy` out of the page
cache the kernel keeps, so wrapping one buys little and costs a lock and a second copy - the
[measurements below](#what-the-cache-buys-and-what-it-costs) say by how much.

The cache earns its keep where a fetch is not a `memcpy`, and the core ships two such
handles. An [`arrowfs`](arrowfs.md) handle answers every read with one `read_range` call
through the foreign-filesystem vtable - over `LocalFileSystem` an `open`, a `seek` and a
`read` per call, and over an object store a round trip - and a [coded](io.md) handle decodes.
Both are the same code as the mapped case, which is the point of the wrapper: what changes is
only how much the fetch it removes was worth.

## What the cache buys, and what it costs

`io_buffered` runs three read workloads over one 16 MiB fixture and every handle the core
ships, plus a fourth workload over a compressed one. The handles fall into two families, and
that split is the whole result:

- **Already memory.** An in-memory `Buffer`, a memory-mapped `local::File`, and an
  [`arrowfs`](arrowfs.md) handle over `MemoryFileSystem`. A read is a `memcpy` - out of a
  `Vec`, out of the mapping the kernel already caches, or out of the vtable's own map.
- **A fetch per read.** An `arrowfs` handle over `LocalFileSystem`, where every `pread` is an
  `open`, a `seek` and a `read`, and every `size` is a `stat`. That is the shape of every
  object store, and the only such backend the core ships.

`random` reads 512 bytes at a time inside a 4 MiB hot region that fits the 8 MiB budget;
`sequential` scans the whole 16 MiB in 8 KiB steps, twice the budget, so every page is
fetched once and evicted before it is wanted again; `footer` reads both ends, sweeps 12 MiB
of the middle, and reads both ends again.

From one containerized x86_64 Linux run with the group run alone
(`cargo bench --bench io --features "parquet" -- io_buffered`; Criterion, 100 samples,
medians). This box's run-to-run spread is wide - a case can move 15% between runs - so read
the multiples, never the percentages:

```text
                                     random        sequential        footer
buffer                             10.041 µs        606.58 µs      439.58 µs
file                               28.037 µs        517.80 µs      367.66 µs
buffered  (over file)              75.362 µs      1.9495 ms        507.49 µs
arrowfs_memory                     46.739 µs        734.30 µs      392.83 µs
arrowfs_memory_buffered            77.633 µs      2.0068 ms        503.69 µs
arrowfs_local                     1.0832 ms       2.7574 ms       2.0924 ms
arrowfs_local_buffered             73.668 µs      2.3841 ms        532.18 µs
```

**Over a backend that is already memory, the cache is a cost; over one that fetches, it is
worth 4x to 15x.** The same code, the same page table, the same pinning:

| Workload | `arrowfs_local` | with the cache | |
| --- | --- | --- | --- |
| `random` (a hot region, re-read) | 1.0832 ms | 73.668 µs | **14.7x faster** |
| `footer` (both ends, big middle) | 2.0924 ms | 532.18 µs | **3.9x faster** |
| `sequential` (one pass, nothing re-read) | 2.7574 ms | 2.3841 ms | **1.2x faster** |

The ordering of those three is the useful part. A cache pays where reads *repeat* - a hot
region, or the two ends of a footer-first container - and barely pays where every byte is
read exactly once, because a one-pass scan copies each byte twice and reuses none of it. The
`sequential` row is the honest floor: 8 KiB reads through 64 KiB pages, so the cache still
turns eight `open`/`seek`/`read` triples into one, and that is worth 16%.

Over the memory-like handles the same cache costs 2.7x (`random`, against `file`), because
there was no fetch to remove: a hit is a clock read, a lock, a map lookup and a copy against
a `memcpy` that was going to happen anyway. That is the price of not knowing what you were
handed, and it is why the wrapper is opt-in rather than automatic.

**Two changes during this work moved these numbers, and both are in the diff:**

- *A hit asks the handle for nothing.* `read_at` used to call `size()` on every read, for the
  end-of-value bound and the pin. On `arrowfs_local` that is a `stat` per read - the cache
  paying exactly the cost it exists to remove. The size is now remembered beside the pages and
  re-asked only when a read runs past what the cache knows. `random/arrowfs_local_buffered`
  went **597.88 µs to 73.668 µs** and `sequential` turned from a 1.2x *loss* into a win.
- *Dense page indexes hash with a multiply and a rotation* rather than SipHash, and the offset
  arithmetic shifts rather than divides - which is what the power-of-two page size is for.
  Together, −20% on the hit case.

**What pinning buys cannot be timed over a backend whose re-read is a `memcpy`**, so the
target asserts it as a count before any timer starts, over a counting handle: after a 12 MiB
middle scan four times wider than the whole budget, re-reading the head and the tail costs
**zero** inner fetches. On `arrowfs_local`, where a fetch is real, that count is what the
`footer` row's 3.9x is made of.

### Over a compressed handle

A content coding is not seekable, so [`Coded`](io.md) answers a positional read by decoding
the value, and *which* decode it pays depends on whether the handle is open. The `coded`
cases read a 256 KiB gzip value in 64 reads of 4 KiB:

```text
io_buffered/coded/closed      12.182 ms    20.522 MiB/s
io_buffered/coded/open         5.8867 us   41.474 GiB/s
io_buffered/coded/buffered    10.2080 us   23.916 GiB/s
```

- **`closed` restarts the decoder.** Each call now stops after its requested range rather than
  decoding the whole value, but 64 progressive calls still create 64 decoders and repeatedly
  discard growing prefixes.
- **`open` serves the one decoded snapshot it explicitly owns.** It is the fastest repeated
  random-access path when holding that complete value is acceptable.
- **`buffered` retains only fetched decoded pages.** It turns decoder restarts into page misses
  and stays within 2x of the opened path for this access pattern.

For a scan, [`pstream_bytes`](io.md#streamed-bytes) is the zero-cache choice: it keeps one
decoder and leaves `cached_pages() == 0`. The page cache is for reuse across genuinely
positional reads, not a prerequisite for sequential decoding.

The order of wrapping is the useful one: `Buffered<Coded<_>>` caches the *decoded* bytes.
`Coded<Buffered<_>>` would cache the compressed bytes and still decode on every read.
