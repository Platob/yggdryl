# ZIP

A file system that lives inside one file: the archive is the container, its members are the leaves, and both are ordinary [`IOBase`](../iobase/bytes.md) handles.

## Contract

| | |
| --- | --- |
| Owns | Rust `holder::zip::{Archive, Entry, Folder, Path, File}`, `zip::mount`, `zip::from_url`; Rust-only |
| Roles | `Folder` is the archive root or any member prefix, `File` is one member, `Path` resolves to whichever is there |
| Mounting | `zip::mount(handle)` or `Holder::zip(handle)` over any handle; a `.zip` leaf stays a leaf until it is mounted |
| Member URL | The archive's URL with the member path as the **fragment**: `file:///lake/day.zip#trades/eu.csv` |
| Reopening | `zip::from_url(url)` mounts the archive the base names and resolves the member the fragment names |
| Codings | Stored, raw DEFLATE, Zstandard - `Codec::Identity`, `Codec::Deflate`, `Codec::Zstd`; any other method is reported by number |
| Refused | Encrypted members, and a member path that climbs above the archive root |
| Lazy | Construction touches nothing; the central directory is parsed on the first operation that needs it |
| Absent | An archive with no bytes indexes as empty; the first member write creates it |
| Stored reads | One positional read of the archive at the member's data offset - no decode, no copy, nothing retained |
| Compressed reads | Decoded through one bounded window; `open` decodes once and answers from that value until `close` |
| Writes | A positional write materializes the member and republishes it whole on `flush` |
| Publishing | Member records append; `flush` writes the directory once, so `n` members cost `n` appends and one directory |
| Removal | Compacts on the next flush, reclaiming the removed member and any dead space a replacement left |
| Streamed input | A member whose sizes follow its bytes reads from the directory; compaction settles them into the header it moves |
| ZIP64 | Read and written when an offset, a size, or the member count needs 64 bits |
| Digest | Every whole-member read verifies the record's CRC-32 |
| Handle calls | Mount 2 reads, warm stored `pread` 1, stored `read_all_bytes` 1, listing 0, publish `n` members `n` writes + trailer + flush |
| Counters | `Archive::handle_reads` / `handle_writes` report every call the archive made into the handle |
| Interop | `python3 scripts/check_zip_interop.py` exchanges archives with Python's `zipfile`, both directions |

## Use

=== "Rust"

    ```rust
    use yggdryl::holder::{Buffer, Holder, zip};
    use yggdryl::IOBase;

    let root = zip::mount(Holder::buffer(Buffer::new()));

    let mut trades = root.child_by_path("2024/06/trades.csv")?;
    trades.write_all_bytes(b"symbol,price\nAAPL,187.23\n")?;

    // Members are children, so the ordinary walk reaches them.
    assert_eq!(root.glob("**/*.csv", false)?.count(), 1);
    assert_eq!(
        root.child_by_path("2024/06/trades.csv")?.read_range_bytes(7, 5)?,
        b"price",
    );
    ```

=== "Python"

    Rust-only.

=== "JavaScript"

    Rust-only.

## The tree comes from the names

A ZIP has no directory tree. It has a flat list of members whose names contain separators, and a directory record is optional metadata beside them. The tree is derived from both: a prefix is a directory when a record names it or when some member continues it.

```rust
use yggdryl::holder::{Buffer, Holder, zip::Archive};
use yggdryl::{IOBase, IOKind};

let root = Archive::new(Holder::buffer(Buffer::new())).mount();
root.child_by_path("2024/06/trades.csv")?.write_all_bytes(b"symbol")?;

// Nothing recorded `2024` or `2024/06`, and both list as directories.
assert_eq!(root.child_by_path("2024")?.kind(), IOKind::Directory);
assert_eq!(root.child_by_path("2024/06")?.kind(), IOKind::Directory);
assert_eq!(root.ls(false, false).count(), 1);
assert_eq!(root.ls(true, false).count(), 3);

// A directory that holds nothing yet is the one case a name cannot imply,
// so it is the one that needs a record of its own.
root.archive().create_directory("2025")?;
assert_eq!(root.child_by_path("2025")?.kind(), IOKind::Directory);
```

Listing reads no member byte: the archive's directory is already the index, so one level is a range over it and the whole tree is a walk of it.

## A member is addressed by a fragment

The member path is the URL fragment, so one location carries both facts - which archive, and which member of it - and the archive's own name never becomes a directory that happens to end in `.zip`.

```rust
use yggdryl::holder::{Holder, zip};
use yggdryl::{IOBase, Url};

let root = zip::mount(Holder::file("/lake/day.zip")?);
let member = root.child_by_path("trades/eu ndx.csv")?;

assert_eq!(
    member.url().expect("a member url"),
    &Url::from_str("file:///lake/day.zip#trades/eu%20ndx.csv")?,
);

// The representation comes from the member's own name, not from `day.zip`.
assert_eq!(*root.child_by_path("logs/app.log.gz")?.media_type().base(), yggdryl::MimeType::PLAIN_TEXT);
```

That makes the location a round trip: what a member reports is what reopens it.

```rust
use yggdryl::holder::{Holder, zip};
use yggdryl::IOBase;

let path = std::env::temp_dir().join(format!("yggdryl-doc-zip-{}.zip", std::process::id()));
let root = zip::mount(Holder::file(&path)?);
root.child_by_path("trades/eu.csv")?.write_all_bytes(b"symbol,price")?;

let member = root.child_by_path("trades/eu.csv")?;
let reopened = zip::from_url(member.url().expect("a member url"))?;
assert_eq!(reopened.read_all_bytes()?, b"symbol,price");

// The same location without a fragment is the archive itself.
let archive = zip::from_url(&yggdryl::Url::from_path(&path)?)?;
assert!(archive.is_container());

let _ = std::fs::remove_file(&path);
```

Hive partitions read from both halves, so a lake can partition the archives and partition again inside one.

```rust
use yggdryl::holder::{Holder, zip};
use yggdryl::IOBase;

let root = zip::mount(Holder::file("/lake/region=eu/day.zip")?);
let member = root.child_by_path("year=2024/month=01/part-0.parquet")?;

assert_eq!(
    member.partitions(),
    vec![
        ("region".to_owned(), "eu".to_owned()),
        ("year".to_owned(), "2024".to_owned()),
        ("month".to_owned(), "01".to_owned()),
    ],
);
```

## Reading a member costs what its coding costs

A **stored** member is the archive's own bytes over a range, so a positional read is one positional read of the archive. Nothing is decompressed, nothing is copied beyond the caller's buffer, and nothing is retained between calls.

```rust
use yggdryl::holder::{Buffer, Holder, zip::Archive};
use yggdryl::{Codec, IOBase};

let payload: Vec<u8> = (0..=255_u8).cycle().take(4_096).collect();
let root = Archive::new(Holder::buffer(Buffer::new())).mount();
root.archive().write_member_with("blob.bin", &payload, Codec::Identity)?;
root.archive().flush()?;

let member = root.as_file("blob.bin")?;
assert_eq!(member.read_range_bytes(1_000, 16)?, payload[1_000..1_016]);

// A positional read materialized nothing, and a read past the end is empty.
assert!(!member.opened());
assert!(member.read_range_bytes(9_999, 16)?.is_empty());
```

A **compressed** member has no decoded seek, so a closed positional read decodes from the member's first byte and discards what precedes the offset through one bounded scratch buffer. `open` is how a caller doing many positional reads pays for the decode once.

```rust
use yggdryl::holder::{Buffer, Holder, zip::Archive};
use yggdryl::IOBase;

let payload: Vec<u8> = (0..=255_u8).cycle().take(8_192).collect();
let root = Archive::new(Holder::buffer(Buffer::new())).mount();
root.archive().write_member("blob.bin", &payload)?;
root.archive().flush()?;

let mut member = root.as_file("blob.bin")?;
assert_eq!(member.read_range_bytes(4_000, 32)?, payload[4_000..4_032]);
assert!(!member.opened());

// Opened, every later read answers from the decoded member it now holds.
member.open()?;
assert!(member.opened());
assert_eq!(member.read_range_bytes(0, 4)?, payload[0..4]);
member.close()?;
```

## Writing a member republishes it

A ZIP member is one compressed unit, so a positional write materializes the decoded member, applies the write, and republishes the whole member - the same shape a [content coding](../../coding/index.md) has.

```rust
use yggdryl::holder::{Buffer, Holder, zip::Archive};
use yggdryl::IOBase;

let root = Archive::new(Holder::buffer(Buffer::new())).mount();
let mut member = root.as_file("notes.txt")?;

member.write_all_bytes(b"symbol,price")?;
member.pwrite(0, b"ticker")?;
member.truncate(6)?;
member.flush()?;

assert_eq!(root.archive().read_member("notes.txt")?, b"ticker");

// Writing past the end grows the member and zero-fills the gap.
let mut sparse = root.as_file("sparse.bin")?;
sparse.pwrite(4, b"tail")?;
sparse.flush()?;
assert_eq!(root.archive().read_member("sparse.bin")?, b"\0\0\0\0tail");
```

The coding a write uses is the member's own if it has one, then `Identity` for a representation that already carries a content coding, then the archive's default.

```rust
use yggdryl::holder::{Buffer, Holder, zip::Archive};
use yggdryl::{Codec, IOBase, Level};

let root = Archive::new(Holder::buffer(Buffer::new()))
    .try_with_codec(Codec::Zstd)?
    .with_level(Level::new(9))
    .mount();

root.as_file("blob.bin")?.write_all_bytes(&vec![1_u8; 512])?;
assert_eq!(root.archive().get_entry("blob.bin")?.expect("the member").codec()?, Codec::Zstd);

// A `.gz` member is already compressed, so it is stored rather than recoded.
root.as_file("app.log.gz")?.write_all_bytes(&yggdryl::coding::gzip::dump(b"symbol")?)?;
assert_eq!(root.archive().get_entry("app.log.gz")?.expect("the member").codec()?, Codec::Identity);

// And an explicit coding on the member handle wins over both.
let member = root.as_file("forced.bin")?.try_with_codec(Codec::Identity)?;
assert_eq!(member.name(), "forced.bin");
```

## Publishing, replacing, removing

A member write appends its record after the last member and updates the in-memory index; `flush` writes the central directory after it. Until that flush the stored archive still reads as its previous state, because the directory an unfinished write has not reached is still the one that indexes it.

```rust
use yggdryl::holder::{Buffer, Holder, zip::Archive};
use yggdryl::{Codec, IOBase};

let root = Archive::new(Holder::buffer(Buffer::new())).mount();
let archive = root.archive();

archive.write_member_with("big.bin", &vec![7_u8; 4_096], Codec::Identity)?;
archive.write_member_with("small.bin", b"kept", Codec::Identity)?;
archive.flush()?;
let before = archive.size();

// Removing compacts, so the removed member's bytes go with the record.
assert!(archive.remove_member("big.bin")?);
archive.flush()?;
assert!(archive.size() < before - 4_000);
assert_eq!(archive.read_member("small.bin")?, b"kept");
```

Replacing a member leaves its previous bytes behind as dead space, which is what makes the replacement an append rather than a rewrite; the next removal reclaims it.

## The index is metadata

`Entry` is what the central directory says about one member. Reading one costs no member byte, which is what lets an archive answer a listing, a size, or a digest check without decompressing anything.

```rust
use yggdryl::holder::{Buffer, Holder, zip::Archive};
use yggdryl::IOBase;

let root = Archive::new(Holder::buffer(Buffer::new())).mount();
root.archive().write_member("trades/eu.csv", b"symbol,price\nAAPL,187.23\n")?;
root.archive().set_comment(b"day one")?;
root.archive().flush()?;

let entry = root.archive().get_entry("trades/eu.csv")?.expect("the member");
assert_eq!(entry.name(), "trades/eu.csv");
assert_eq!(entry.size(), 25);
assert!(entry.compressed_size() > 0);
assert!(!entry.is_directory());
assert!(!entry.is_encrypted());
assert_eq!(root.archive().comment()?, b"day one");
```

`open` parses the directory once and holds it for the scope; `close` publishes anything pending and releases it.

## What an operation costs the handle

A call into the handle beneath the archive is a round trip against an object
store, a syscall against a file, and a lock through every wrapper, so the
backend's cost is its call count rather than its byte count.
`Archive::handle_reads` and `handle_writes` report it, which is how the cost
model below is asserted rather than asserted-to.

```rust
use yggdryl::holder::{Buffer, Holder, zip::Archive};
use yggdryl::{Codec, IOBase};

let root = Archive::new(Holder::buffer(Buffer::new())).mount();
root.archive().write_member_with("blob.bin", &vec![4_u8; 4_096], Codec::Identity)?;
root.archive().write_member_with("notes.txt", b"symbol", Codec::Identity)?;

// One write per record, then the trailer and the flush behind it. Nothing
// shortens the archive, because it only grew.
assert_eq!(root.archive().handle_writes(), 2);
root.archive().flush()?;
assert_eq!(root.archive().handle_writes(), 4);

// A listing walks the index the archive already holds.
let quiet = root.archive().handle_reads();
assert_eq!(root.ls(true, true).count(), 2);
assert_eq!(root.archive().handle_reads(), quiet);

// The write knew where it put the bytes, so reading them back is one read.
root.as_file("blob.bin")?.read_range_bytes(0, 16)?;
assert_eq!(root.archive().handle_reads() - quiet, 1);
```

Where the calls went:

| operation | reads | writes |
| --- | --- | --- |
| Mount and parse the directory | 2 | 0 |
| The same, directory larger than the 64 KiB tail | 3 | 0 |
| Listing, glob, `size`, `partitions`, member metadata | 0 | 0 |
| Positional read of a stored member, warm | 1 | 0 |
| Whole read of a stored member | 1 | 0 |
| First read of a member this archive did not write | +1 | 0 |
| Write one member | 0 | 1 |
| Publish | 0 | 2, or 3 when the archive shrank |

The one extra read is the member's local file header, which is the only place
a member's data offset is stated. It is read once per member and held in the
index, so a second handle on that member, and every resolution of a location
naming it, answer from there - and a member this archive wrote never costs it
at all, because the write already knew the answer.

## Records inside an archive

A member is a leaf like any other, so the [record surface](../iobase/records.md) reaches it by name, and the directory above it reads as the table beneath it.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch};

use yggdryl::holder::{Buffer, Holder, zip};
use yggdryl::{DataType, IOBase, IOMedia};

let schema = DataType::from_fields([DataType::Int64.required_field("id")])?
    .required_field("row");
let arrow_schema = schema.clone().into_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
    vec![Arc::new(Int64Array::from(vec![7_i64, 9]))],
)?;

let root = zip::mount(Holder::buffer(Buffer::new()));
let mut member = root.child_by_path("trades.arrows")?;
let options = member.record_options()?;
member.overwrite_arrow_reader(yggdryl::arrow::batch_reader(arrow_schema, [batch]), &options)?;

assert_eq!(member.row_size()?, 2);
assert!(root.is_tabular());
```

## Edges

- A `.zip` leaf is a leaf until it is mounted: `copy_into` on one copies the archive's bytes, not its members.
- `zip::from_url` mounts local archives only; mount any other handle with `zip::mount`.
- An encrypted member -> `Error::Unsupported` naming the member; a compression method this build cannot decode -> `Error::Unsupported` naming the number.
- Gzip and zlib have no ZIP method: their framing wraps a whole resource, while a member carries the raw DEFLATE stream inside it. `try_with_codec` refuses both by name.
- A member whose decoded bytes do not match the record's CRC-32 -> `Error::Codec` naming both digests.
- `../` in a member path -> `Error::Parse`; `./` and `//` resolve, and the index holds one spelling of every name.
- A directory that still holds members -> `remove(false)` is refused; `remove(true)` empties it first.
- A member is republished whole, so a positional write costs the member's decoded size in memory.
- A self-extracting archive reads: the trailer says where the directory really ends, and every recorded offset is shifted by the difference.
- A member another writer streamed states its digest and sizes after its bytes. Compaction moves the record but not that trailer, so the header it writes carries the values from the directory instead and drops the bit that promised them.
- `flush` is not a write: an archive nothing has touched is not created by one.
- A member handle publishes the directory when it flushes, because a member is only durably in the archive once the directory indexes it. Writing many members through many handles therefore publishes many times: for a batch, write them with `Archive::write_member` and flush once.

## Commands

=== "Rust"

    ```bash
    cargo test -p yggdryl --lib holder::zip::
    cargo test -p yggdryl --test interop zip::
    python3 scripts/check_zip_interop.py
    cargo bench --bench holder -- io_zip
    ```

## Performance

`io_zip` runs one containerized x86_64 Linux release build, group alone, Criterion, 100 samples, medians. Run-to-run spread reaches 20%, so read the multiples, never the percentages.

One 1 MiB member, read positionally at its midpoint, 512 bytes at a time:

| leg | | |
| --- | --- | --- |
| `read/stored` | 110.51 ns | the archive's own `pread`, no decode |
| `read/deflate_opened` | 56.56 ns | a copy out of the decoded member `open` holds |
| `read/through_path` | 593.03 ns | the same read, resolving the member's name each time |
| `read/deflate_closed` | 40.43 µs | decoded from the member's first byte, nothing retained |

A stored member is **366x** cheaper to read positionally than a closed compressed one, because there is nothing between the caller's buffer and the archive's bytes. That is the whole reason the stored path exists; it is also why `open` is the answer for many positional reads over a compressed member, and why a closed one is still the right default for one read.

Whole-member reads over the same 1 MiB, digest check included:

| leg | | |
| --- | --- | --- |
| `read_all/stored` | 119.50 µs | 8.06 GiB/s |
| `read_all/deflate` | 127.31 µs | 7.67 GiB/s |

The two are close because both are dominated by the CRC-32 pass over the decoded bytes, which every whole-member read performs.

An archive of 2,000 members across ten directories:

| leg | | |
| --- | --- | --- |
| `mount/index` | 899.39 µs | two handle reads, then parsing 2,000 records |
| `listing/first_entry` | 662.45 µs | the index snapshot a recursive listing walks |
| `listing/drain` | 1.2993 ms | every entry |
| `listing/glob` | 2.1634 ms | `part=03/**/*.csv` over the same tree |
| `write/members` | 27.840 ms | 2,000 members and one directory, ~14 µs each |

Listing reads no member byte at all: the cost is building the name snapshot out of the index. Unlike a directory backend, then, time to first entry is not cheaper than the drain - the index is one map, and a recursive listing walks all of it before yielding.

### What the call budget bought

Holding the cost model to the counts above moved the timings with it, measured against the same group before the handle calls were cut:

| leg | before | after | |
| --- | --- | --- | --- |
| `read/stored` | 178.74 ns | 110.51 ns | **1.6x faster** |
| `read/deflate_opened` | 66.79 ns | 56.56 ns | 1.2x faster |
| `listing/first_entry` | 729.04 µs | 662.45 µs | 1.1x faster |
| `read_all/deflate` | 136.54 µs | 127.31 µs | 1.1x faster |
| `read_all/stored` | 114.34 µs | 119.50 µs | 1.04x slower |

The positional read is where it shows: it went from three lock acquisitions, a copied record, and an allocated lookup key down to one lock, one borrow, and one handle read. The one leg that lost is the whole stored read, which now fills a zeroed buffer in one ranged call where it used to stream into spare capacity - a constant-factor cost against one fewer round trip, which is the trade the call budget asks for and the one a real store rewards.

```bash
cargo bench --bench holder -- io_zip
```
