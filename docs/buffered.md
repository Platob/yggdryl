# Buffered handles

A page cache that makes any [`IOBase`](io.md) handle buffered, with the value's first and last pages pinned.

!!! note "Rust only"
    The Python and JavaScript packages do not expose this module yet.

```rust
use yggdryl::buffered::BufferedOptions;
use yggdryl::io::{Buffer, IOBase};

let handle = Buffer::from_bytes(b"symbol,price\nAAPL,1\n".to_vec())
    .buffered(BufferedOptions::default());

// The first read fetches the page holding the range; the second is memory.
assert_eq!(handle.read_range(0, 6)?, b"symbol");
assert_eq!(handle.read_range(13, 4)?, b"AAPL");
assert_eq!(handle.cached_pages(), 1);
```

`IOBase::buffered` wraps any handle, and what comes back is a handle: `Buffered<H>` mirrors
everything it does not change, so `size`, `url`, `media_type`, `kind`, `parent`, `child_by`,
and `ls` answer exactly what the wrapped handle answers. It is invisible except for speed,
the same way [`Coded`](io.md) is invisible except for the coding.

Nothing is cached at construction. Per the [laziness contract](io.md), a handle is a
description of where bytes would live, and the cache only ever holds pages a read asked for.

## Pages

```rust
use yggdryl::buffered::{Buffered, BufferedOptions};
use yggdryl::io::{Buffer, IOBase};

// 256-byte pages, so a 1 KiB value is four of them.
let options = BufferedOptions::default().with_page_size(256);
let handle = Buffered::new(Buffer::from_bytes(vec![7_u8; 1_024]), options);

// A read that misses fetches the whole page holding it, aligned.
assert_eq!(handle.read_range(300, 4)?.len(), 4);
assert_eq!(handle.cached_pages(), 1);
assert_eq!(handle.cached_bytes(), 256);
assert!(handle.has_cached_page(1));

// A read spanning pages assembles from each of them, caching all it crossed.
assert_eq!(handle.read_range(100, 600)?.len(), 600);
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
handle.read_range(16 * 64 - 8, 8)?;
handle.read_range(0, 8)?;

// Then a scan of the middle, four times what the budget can hold.
for page in 1..15 {
    handle.read_range(page * 64, 8)?;
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
    handle.read_range(page * 64, 8)?;
}
handle.read_range(5 * 64, 8)?;
handle.read_range(6 * 64, 8)?;
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
assert_eq!(handle.read_range(13, 4)?, b"MSFT");
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
assert_eq!(handle.read_range(0, 4)?, [3, 3, 3, 3]);
```

The one thing the cache cannot see is a change made *behind* it - bytes written straight to
the inner handle, or to the same file by another process. `handle_mut` therefore drops every
page before it hands the inner handle over, and `clear_cache` is the same thing said
explicitly.

## Over a compressed handle

A content coding is not seekable, so [`Coded`](io.md) answers a positional read by decoding
the value - and *which* decode it pays depends on whether the handle is open. A handle nobody
opened decodes the whole payload **for every `pread`**, because nothing may be cached as a
side effect of an ordinary read. Wrapping it turns that into one decode per page miss.

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
assert_eq!(handle.read_range(0, 6)?, b"symbol");
assert_eq!(handle.read_range(13, 4)?, b"AAPL");

// Two reads, one page, one decode.
assert_eq!(handle.cached_pages(), 1);
assert_eq!(handle.size(), payload.len() as u64);
```

Three things follow, and the [benchmarks](benchmarks.md) measure all of them:

- **The order of wrapping matters.** `Buffered<Coded<_>>` caches the *decoded* bytes, which
  is what the reads want. `Coded<Buffered<_>>` would cache the compressed bytes and still
  decode on every read, which buys nothing.
- **`open` is still cheaper.** A caller who knows they hold a compressed value should open it:
  that materializes the decoded value once, every read is then a range copy out of it, and
  `close` releases it. The page cache is what makes an *unopened* coded handle behave - the
  case a caller who does not know what they were handed is in.
- **The cache does not make the decode disappear**, it divides it. A miss still decodes the
  whole payload, so a scan of a value much larger than the budget still pays one decode per
  page. For that shape, open the handle.

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
assert_eq!(twice.read_range(0, 4)?, [5, 5, 5, 5]);

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
use yggdryl::local::File;

let path = std::env::temp_dir().join(format!("yggdryl-doc-buffered-{}.bin", std::process::id()));
std::fs::write(&path, vec![9_u8; 4_096])?;

let handle = File::new(&path)?.buffered(BufferedOptions::default().with_page_size(1_024));
assert_eq!(handle.size(), 4_096);
assert_eq!(handle.read_range(2_000, 8)?, [9_u8; 8]);
assert_eq!(handle.cached_pages(), 1);

// The wrapper is the file for every purpose but the reading.
let bare = File::new(&path)?;
assert_eq!(handle.url(), bare.url());

drop(handle);
let _ = std::fs::remove_file(&path);
```

Over a memory-mapped [local file](local.md), a `pread` is already a `memcpy` out of the page
cache the kernel keeps, so wrapping one buys little and costs a lock and a second copy - the
[benchmarks](benchmarks.md) say by how much.

The cache earns its keep where a fetch is not a `memcpy`, and the core ships two such
handles. An [`arrowfs`](arrowfs.md) handle answers every read with one `read_range` call
through the foreign-filesystem vtable - over `LocalFileSystem` an `open`, a `seek` and a
`read` per call, and over an object store a round trip - and a [coded](io.md) handle decodes.
Both are the same code as the mapped case, which is the point of the wrapper: what changes is
only how much the fetch it removes was worth.

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/buffered-rust.ipynb){ download }.

<!-- /notebooks -->
