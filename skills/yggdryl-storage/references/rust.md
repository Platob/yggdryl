# yggdryl-storage in Rust

`use yggdryl::IOBase;` brings every byte method into scope (the trait is
object-safe: `&dyn IOBase`). Object stores need the `s3` feature, the AWS
session the `aws` feature (implied by `s3`); everything else is default.

## Open a handle for a path or URL

`Holder::local` records the location and decides the role only when an
operation needs it; `Holder::from_url` lets the scheme pick the backend and
reads `media_type`/`codec` properties for an extensionless resource.

```rust
use yggdryl::holder::Holder;
use yggdryl::local::LocalFolder;
use yggdryl::{IOBase, IOKind, MimeType, Url};

let root = LocalFolder::temporary()?.path()?.join(format!("ygg-skill-open-{}", std::process::id()));

// Nothing is probed or created: the location simply is not decided yet.
let mut handle = Holder::local(root.join("ticks.csv"))?;
assert!(matches!(handle, Holder::LocalPath(_)));
assert_eq!(handle.kind(), IOKind::Unknown);

// A write creates the file and every missing parent, and settles the kind.
handle.write_all_bytes(b"symbol,price\nAAPL,1\n")?;
assert_eq!(handle.kind(), IOKind::File);
assert_eq!(handle.media_type().base(), &MimeType::CSV);

// The URL door: the scheme picks the backend, a property types the bytes.
let url = Url::from_path(root.join("blob"))?;
let typed = Holder::from_url(&url, [("media_type", "application/vnd.apache.parquet")])?;
assert_eq!(typed.media_type().base(), &MimeType::PARQUET);

handle.close()?;
std::fs::remove_dir_all(&root)?;
```

## Hold bytes in memory

`Buffer` is the in-memory handle; bytes are sniffed for a media type, so
declare one the bytes cannot prove.

```rust
use yggdryl::holder::{Buffer, Holder};
use yggdryl::{IOBase, IOKind, MimeType};

let sniffed = Buffer::from_bytes(br#"{"symbol":"AAPL"}"#.to_vec());
assert_eq!(sniffed.media_type().base(), &MimeType::JSON);
assert_eq!(sniffed.kind(), IOKind::Memory);
assert_eq!(sniffed.uri().unwrap().scheme().as_str(), "mem");

// CSV is not provable from bytes: declare it.
let csv = Buffer::from_bytes(b"symbol,price\n".to_vec()).with_media_type(MimeType::CSV.into());
assert_eq!(csv.media_type().base(), &MimeType::CSV);

// Any backend fits in a `Holder`; the calls do not change.
let held = Holder::buffer(csv);
assert_eq!(held.read_all_bytes()?, b"symbol,price\n");
```

## Read a range, write at an offset, append

`read_range_bytes` is one call that transfers only the window, clamped at the
end; `append_bytes` answers the offset the bytes landed at.

```rust
use yggdryl::holder::Buffer;
use yggdryl::IOBase;

let mut handle = Buffer::new();
handle.write_all_bytes(b"symbol,price\n")?;
assert_eq!(handle.append_bytes(b"AAPL,1\n")?, 13);

// Offsets are explicit: two readers never share a position.
assert_eq!(handle.read_range_bytes(13, 4)?, b"AAPL");
assert_eq!(handle.read_range_bytes(0, 6)?, b"symbol");
// Past the end is empty, not an error.
assert!(handle.read_range_bytes(100, 4)?.is_empty());

// A write past the end zero-fills the gap.
handle.pwrite(22, b"!")?;
assert_eq!(handle.size(), 23);
assert_eq!(handle.read_range_bytes(20, 3)?, b"\0\0!");

// The primitive: fill a caller buffer, short only at the end.
let mut head = [0_u8; 6];
assert_eq!(handle.pread(0, &mut head)?, 6);
assert_eq!(&head, b"symbol");
```

## Stream bounded chunks and use a cursor

`pstream_bytes` yields owned chunks lazily and never asks for `size`; a
`Cursor` owns one position and implements `std::io::Read`/`Write`/`Seek`.
`reader_at`/`writer_at` borrow the handle as `std::io` adapters.

```rust
use std::io::{Read, Write};

use yggdryl::holder::Buffer;
use yggdryl::{IOBase, IOCursor, DEFAULT_STREAM_BATCH_SIZE};

let handle = Buffer::from_bytes(b"0123456789".to_vec());
let chunks = handle.pstream_bytes(2, 3)?.collect::<yggdryl::Result<Vec<_>>>()?;
assert_eq!(chunks, [b"234".to_vec(), b"567".to_vec(), b"89".to_vec()]);
assert_eq!(DEFAULT_STREAM_BATCH_SIZE, 64 * 1024);

// A cursor advances only as bytes are yielded.
let mut cursor = handle.cursor_at(1);
let first = cursor.stream_bytes(2)?.next().transpose()?.unwrap();
assert_eq!(first, b"12");
assert_eq!(cursor.tell(), 3);
cursor.seek_to(7);
let mut rest = String::new();
cursor.read_to_string(&mut rest)?;
assert_eq!(rest, "789");

// `std::io` adapters over a handle, each with its own offset.
let mut target = Buffer::new();
target.writer_at(0).write_all(b"symbol,price\n")?;
let mut text = String::new();
target.reader_at(7).read_to_string(&mut text)?;
assert_eq!(text, "price\n");
```

## Decide a role: file, folder, or not yet

A backend has three roles: `LocalPath` (undecided), `LocalFolder`,
`LocalFile`. A byte write settles an undecided location as a file;
`as_directory()?.create()?` settles it as a folder. Nothing needs an
existence check first.

```rust
use yggdryl::local::{LocalFile, LocalFolder, LocalPath};
use yggdryl::{IOBase, IOKind};

let root = std::env::temp_dir().join(format!("ygg-skill-roles-{}", std::process::id()));

// Construction touches nothing; absence reads as empty.
let absent = LocalFile::new(root.join("sub").join("inner.bin"))?;
assert!(!absent.exists());
assert_eq!(absent.size(), 0);
assert!(absent.read_all_bytes()?.is_empty());
assert!(!root.exists());

// A folder is brought into being by saying so.
let day = LocalPath::new(root.join("day=2026-08-16"))?;
assert_eq!(day.kind(), IOKind::Unknown);
day.as_directory()?.create()?;
assert_eq!(day.kind(), IOKind::Directory);

// A byte write makes a file, parents included.
let mut leaf = LocalPath::new(root.join("a").join("b.bin"))?;
leaf.write_all_bytes(b"AAPL")?;
leaf.flush()?;
assert_eq!(leaf.kind(), IOKind::File);

// A container holds no bytes: reads are empty, byte writes are refused.
let mut folder = LocalFolder::new(&root)?;
assert!(folder.pwrite(0, b"x").is_err());

drop(leaf);
std::fs::remove_dir_all(&root)?;
```

## Walk, glob, clear and remove a tree

`child_by_path` takes one slash path and returns a `Holder`; listings are lazy
`Result` iterators, sorted, skipping dot-names unless `include_private`.
`clear` empties and keeps; `remove` deletes, treats absence as success, and
refuses a non-empty container unless `recursive`.

```rust
use yggdryl::local::LocalFolder;
use yggdryl::{IOBase, IOKind};

let root = std::env::temp_dir().join(format!("ygg-skill-walk-{}", std::process::id()));
let mut lake = LocalFolder::new(&root)?;

for part in ["year=2024/month=01/part-0.parquet", "year=2025/month=01/part-0.parquet"] {
    let mut leaf = lake.child_by_path(part)?;
    leaf.write_all_bytes(b"PAR1")?;
    leaf.flush()?;
}
std::fs::create_dir_all(root.join(".git"))?;

// Sorted and lazy; the private `.git` is skipped unless asked for.
let names = lake
    .ls(false, false)
    .map(|entry| Ok(entry?.url().and_then(|url| url.file_name()).unwrap_or_default().to_owned()))
    .collect::<yggdryl::Result<Vec<_>>>()?;
assert_eq!(names, ["year=2024", "year=2025"]);
assert_eq!(lake.ls(false, true).count(), 3);

// A glob descends its fixed prefix instead of listing everything.
assert_eq!(lake.glob("year=2024/**/*.parquet", false)?.count(), 1);
assert_eq!(lake.glob("**/*.parquet", false)?.count(), 2);

// Clear keeps the container; remove deletes it, and twice is fine.
lake.clear()?;
assert_eq!(lake.ls(true, true).count(), 0);
assert_eq!(lake.kind(), IOKind::Directory);
lake.remove(false)?;
lake.remove(false)?;
assert!(!root.exists());
```

## Media type and coding from the name

`media_type` is the decoded representation, `codec` the stored coding; both
come from the suffix chain. `into_declared_media` composes the coding
underneath and the record encoding on top, reading nothing.

```rust
use yggdryl::holder::{Buffer, Holder};
use yggdryl::{Codec, IOBase, MimeType, Url};

let named = Url::from_str("file:///trades.json.gz")?;
let declared = Buffer::new().with_media_type(named.media_type());
assert_eq!(declared.media_type().base(), &MimeType::JSON);
assert_eq!(declared.codec(), Codec::Gzip);

// Compose what `trades.txt.gz` declares: text records over a gzip view.
let stored = Codec::Gzip.dump(b"AAPL,1\n")?;
let log = Url::from_str("file:///trades.txt.gz")?;
let composed = Holder::buffer(Buffer::from_bytes(stored).with_media_type(log.media_type()))
    .into_declared_media();
assert!(matches!(composed, Holder::Text(_)));
assert_eq!(composed.read_all_bytes()?, b"AAPL,1\n");
```

## Read and write through a coding

A `Coded` handle presents decoded bytes over stored ones and codes on write.
`compress_into`/`decompress_into` move every byte between two handles that
address stored bytes, adding or removing a coding.

```rust
use yggdryl::coding::Coded;
use yggdryl::gzip::Gzip;
use yggdryl::holder::Buffer;
use yggdryl::{Codec, IOBase, Level, Url};

// Decoded view over a zstd store.
let mut coded = Coded::wrap(Buffer::new(), Codec::Zstd).with_level(Level::BEST);
coded.write_all_bytes(b"symbol,price\nAAPL,1\n")?;
coded.flush()?;
assert_eq!(coded.read_all_bytes()?, b"symbol,price\nAAPL,1\n");
let stored = coded.into_handle()?;
assert_eq!(&stored.read_all_bytes()?[..4], &[0x28, 0xB5, 0x2F, 0xFD]);

// The per-codec wrapper is the same idea with the codec fixed.
let mut gz = Gzip::new(Buffer::new());
gz.write_all_bytes(b"AAPL,1\n")?;
gz.flush()?;
assert_eq!(&gz.handle().read_all_bytes()?[..2], b"\x1f\x8b");

// Re-code between handles: the target's name is where the codec is read.
let plain = Buffer::from_bytes(br#"{"symbol":"AAPL"}"#.to_vec());
let mut encoded = Buffer::new().with_media_type(Url::from_str("file:///rows.json.gz")?.media_type());
let codec = encoded.codec();
plain.compress_into(&mut encoded, codec)?;
assert_eq!(&encoded.read_all_bytes()?[..2], b"\x1f\x8b");
let mut decoded = Buffer::new();
encoded.decompress_into(&mut decoded)?;
assert_eq!(decoded.read_all_bytes()?, plain.read_all_bytes()?);
```

## Compress bytes without a handle

Each codec is a root module with `load`/`dump` over whole buffers and
`reader`/`writer` over `std::io` streams; `Codec` dispatches to them by value.
A streaming `Encoder` must be `finish`ed to write the trailer.

```rust
use std::io::{Read, Write};

use yggdryl::{gzip, zlib, zstd, Codec, Level};

let payload = "symbol,price\n".to_owned() + &"AAPL,1\n".repeat(64);

assert_eq!(gzip::load(&gzip::dump(payload.as_bytes())?)?, payload.as_bytes());
let best = zstd::dump_with_level(payload.as_bytes(), Level::BEST)?;
assert!(best.len() < payload.len());

// zlib has a second framing: raw DEFLATE, what a ZIP member holds.
let framed = zlib::dump(payload.as_bytes())?;
let raw = zlib::dump_raw(payload.as_bytes())?;
assert_eq!(framed.len(), raw.len() + 6);
assert!(zlib::load(&raw).is_err());
assert_eq!(zlib::load_raw(&raw)?, payload.as_bytes());

// Streaming: an encoder over any `Write`, a decoder over any `Read`.
let mut sink = Vec::new();
let mut encoder = Codec::Gzip.writer(&mut sink);
encoder.write_all(payload.as_bytes())?;
encoder.finish()?;
let mut text = String::new();
gzip::reader(sink.as_slice()).read_to_string(&mut text)?;
assert_eq!(text, payload);
assert_eq!(Codec::from_str("zstd")?, Codec::Zstd);
```

## Decode a charset once

`Charset` is the one vocabulary: `decode` refuses bad bytes by position,
`decode_lossy` marks them `U+FFFD`, `transcribe` reads every byte. A
`Transcoded` handle decodes a whole resource so everything above sees UTF-8.

```rust
use yggdryl::charset::Transcoded;
use yggdryl::holder::Buffer;
use yggdryl::{Charset, IOBase};

let wire = b"prix: 12\x80";
assert_eq!(Charset::Cp1252.decode(wire)?, "prix: 12€");
assert_eq!(Charset::Cp1252.encode("prix: 12€")?.as_ref(), wire);
// ISO 8859-1 is a different document for the same bytes.
assert_eq!(Charset::Latin1.decode(wire)?, "prix: 12\u{0080}");
assert_eq!(Charset::from_str("cp1252")?, Charset::Cp1252);
assert!(Charset::from_str("utf-16").is_err());

// Refuse, mark, or recover.
assert!(Charset::Ascii.decode(b"caf\xe9").is_err());
assert_eq!(Charset::Ascii.decode_lossy(b"caf\xe9"), "caf\u{FFFD}");
assert_eq!(Charset::Utf8.transcribe(b"caf\xe9"), "café");

// A byte-order mark is framing: its charset and length.
assert_eq!(Charset::from_bom(b"\xef\xbb\xbfid"), Some((Charset::Utf8, 3)));

// A whole resource: decode on read, encode on write.
let mut handle = Transcoded::new(Buffer::new(), Charset::Cp1252);
handle.write_all_bytes("symbol,désk\n".as_bytes())?;
handle.flush()?;
assert_eq!(handle.read_all_bytes()?, "symbol,désk\n".as_bytes());
assert_eq!(handle.handle().read_all_bytes()?, b"symbol,d\xe9sk\n");
```

## Digest or parse the value a handle holds

Digests stream `pstream_bytes` and hold one chunk; `read_scalar` picks
JSON/YAML/TOML/XML and any outer coding from the media type.

```rust
use yggdryl::holder::Buffer;
use yggdryl::{DigestAlgorithm, Field, IOBase, Scalar, Url};

let mut handle = Buffer::new();
handle.write_all_bytes(b"symbol,price\nAAPL,1\n")?;
assert_eq!(
    handle.read_digest(DigestAlgorithm::Xxh3)?,
    DigestAlgorithm::Xxh3.digest(&handle.read_all_bytes()?),
);
assert_eq!(
    handle.read_range_digest(0, 6, DigestAlgorithm::Xxh3)?,
    DigestAlgorithm::Xxh3.digest(b"symbol"),
);

let mut document = Buffer::new().with_media_type(Url::from_str("file:///trade.json.gz")?.media_type());
document.write_scalar(&Scalar::from_struct([
    ("quantity", Scalar::from(2_i64)),
    ("symbol", Scalar::from("AAPL")),
])?)?;
let field = Field::from_str("trade: struct<quantity: int32 not null, symbol: utf8 not null> not null")?;
assert_eq!(document.read_scalar(Some(&field))?.get(0).as_deref(), Some(&Scalar::from(2_i64)));
```

## Repeat reads: open a scope or add a page cache

`open` moves materialization to a known point and keeps caches until `close`;
`buffered` holds aligned pages (both ends pinned) within a byte budget. Wrap
the coding inside the cache to cache decoded pages.

```rust
use yggdryl::gzip::Gzip;
use yggdryl::holder::buffered::BufferedOptions;
use yggdryl::holder::Buffer;
use yggdryl::IOBase;

let payload = "symbol,price\nAAPL,1\n".repeat(512).into_bytes();
let mut source = Gzip::new(Buffer::new());
source.write_all_bytes(&payload)?;
source.flush()?;
let encoded = source.into_handle()?;

// A scope: one decode serves every read until close.
let mut scoped = Gzip::new(encoded.clone());
scoped.open()?;
assert!(scoped.opened());
assert_eq!(scoped.read_range_bytes(13, 4)?, b"AAPL");
scoped.close()?;
assert!(!scoped.opened());

// A cache over the coding: pages hold decoded bytes.
let cached = Gzip::new(encoded).buffered(BufferedOptions::default().with_page_size(4_096));
assert_eq!(cached.read_range_bytes(0, 6)?, b"symbol");
assert_eq!(cached.read_range_bytes(13, 4)?, b"AAPL");
assert_eq!(cached.cached_pages(), 1);
assert_eq!(cached.size(), payload.len() as u64);
```

## Count the calls an operation makes

`Counted` forwards every call and tallies it by name; put it under the layer
you measure. On a store each call is a round trip, so this is the cost.

```rust
use std::sync::Arc;

use yggdryl::holder::buffered::BufferedOptions;
use yggdryl::holder::counted::{Call, Counted, Group};
use yggdryl::holder::Buffer;
use yggdryl::IOBase;

let counted = Counted::new(Buffer::from_bytes(vec![7_u8; 64 * 1024]));
let calls = Arc::clone(counted.calls());

assert_eq!(counted.read_all_bytes()?.len(), 64 * 1024);
assert_eq!(calls.get(Call::ReadAllBytes), 1);
assert_eq!(counted.counts().to_string(), "read_all_bytes=1");

calls.reset();
assert_eq!(counted.size(), 64 * 1024);
assert_eq!(calls.group(Group::Metadata), 1);

// A cache built on the counted handle: a warm read reaches the store for nothing.
let cached = counted.buffered(BufferedOptions::default());
calls.reset();
cached.read_range_bytes(0, 16)?;
assert_eq!(calls.snapshot().to_string(), "pread=1 size=1");
calls.reset();
cached.read_range_bytes(0, 16)?;
assert!(calls.snapshot().is_empty());
```

## Name an object on S3, Google Cloud Storage or Azure

`s3::file`/`folder` take a URL, `file_at`/`folder_at` a `Provider` and the
store's raw key; construction makes zero requests. `S3Options` holds what all
three stores share, `aws::Session` who the process is to AWS. Knobs and
request counts are in `backends.md`.

```rust
use yggdryl::aws::{Credentials, Session};
use yggdryl::s3::{self, Provider, S3Options};
use yggdryl::IOBase;

// A raw key: the handle does the escaping.
let part = s3::file_at(Provider::Aws, "trades", "lake/a b/part.parquet")?;
assert_eq!(part.key(), "lake/a b/part.parquet");
assert_eq!(part.url().to_string(), "s3://trades/lake/a%20b/part.parquet");

// A prefix always ends in the delimiter.
let lake = s3::folder_at(Provider::Google, "trades", "lake")?;
assert_eq!(lake.prefix(), "lake/");

// Azure writes the container ahead of the account host.
let blob = s3::file("abfss://lake@trades.dfs.core.windows.net/part.parquet")?;
assert_eq!((blob.bucket(), blob.key()), ("lake", "part.parquet"));

// Explicit configuration; `with_environment(false)` consults nothing else.
let session = Session::new()
    .with_environment(false)
    .with_region("eu-west-3")
    .with_credentials(Credentials::new("AKIAIOSFODNN7EXAMPLE", "secret"));
let options = S3Options::default()
    .with_endpoint("http://localhost:9000")
    .with_path_style(true)
    .with_session(session);
let local = s3::file_with("s3://trades/lake/part.parquet", options)?;
assert_eq!(local.bucket(), "trades");

// Catalog properties handed over whole; unknown names are ignored, bad values refused.
let catalog = S3Options::from_properties([("s3.endpoint", "http://localhost:9000"), ("warehouse", "s3://trades")])?;
assert_eq!(catalog.endpoint(), Some("http://localhost:9000"));
assert!(S3Options::from_properties([("s3.request-timeout", "soon")]).is_err());
```

## Bind an Arrow-style filesystem

`fs::FileSystem` is the seam for foreign storage; the path is opaque and
reaches the filesystem literally (`%2F` stays `%2F`).

```rust
use std::sync::Arc;

use yggdryl::fs::{FileSystem, FsFile, FsFolder, MemoryFileSystem};
use yggdryl::IOBase;

let filesystem: Arc<dyn FileSystem> = Arc::new(MemoryFileSystem::new());
FsFolder::from_path(Arc::clone(&filesystem), "bucket", None)?.create(false)?;

let mut file = FsFile::from_path(
    Arc::clone(&filesystem),
    "bucket/v=a%2Fb.bin",
    Some("s3://bucket/v=a%2Fb.bin".to_owned()),
)?;
file.write_all_bytes(b"literal")?;
assert_eq!(file.read_range_bytes(2, 3)?, b"ter");
assert_eq!(filesystem.file_info("bucket/v=a%2Fb.bin")?.size, Some(7));
```

## Read and write members of a ZIP archive

`zip::mount` turns a handle into the archive's root; members are ordinary
leaves addressed by the archive URL plus a fragment. `flush` writes the
central directory once. Rust only.

```rust
use yggdryl::holder::{Buffer, Holder};
use yggdryl::zip::{self, ZipArchive};
use yggdryl::{Codec, IOBase, IOKind};

let root = zip::mount(Holder::buffer(Buffer::new()));
root.child_by_path("2024/06/trades.csv")?.write_all_bytes(b"symbol,price\nAAPL,187.23\n")?;

// Members are children: the ordinary walk and glob reach them, reading no member byte.
assert_eq!(root.glob("**/*.csv", false)?.count(), 1);
assert_eq!(root.child_by_path("2024")?.kind(), IOKind::Directory);
assert_eq!(root.child_by_path("2024/06/trades.csv")?.read_range_bytes(7, 5)?, b"price");

// The archive API: explicit codings, restart points, the index.
let archive = ZipArchive::new(Holder::buffer(Buffer::new())).with_restart_stride(4_096).mount();
let payload = b"symbol,price\nAAPL,187.23\n".repeat(2_048);
archive.archive().write_member_with("blob.bin", &payload, Codec::Deflate)?;
archive.archive().flush()?;
let entry = archive.archive().get_entry("blob.bin")?.expect("the member");
assert!(entry.compressed_size() < entry.size());
// A positional read decodes one stride, not the prefix.
assert_eq!(archive.as_leaf("blob.bin")?.read_range_bytes(40_000, 8)?, payload[40_000..40_008]);
```

## Gotchas in Rust

- `IOBase` must be in scope (`use yggdryl::IOBase;`) for any byte method,
  `IOCursor` for `tell`/`seek_to`; `IOCursor::seek` and `std::io::Seek::seek`
  collide - name the trait.
- `pwrite`/`write_all_bytes` on a mapped `LocalFile` stage; `flush()` or
  `close()` publishes the logical length. A mapped file truncated by another
  process raises SIGBUS - snapshot with `copy_into(&mut Buffer::new())`.
- `Buffered::buffered` re-wraps the one cache (inherent method wins);
  `into_handle()` gives the inner handle back, cache dropped.
- `compress_into` and `decompress_into` refuse a handle presenting a decoded
  view (a `Coded` target) by name rather than double-coding.
- `Holder::from_url` with an `s3:`/`gs:`/`az:` scheme needs the `s3` feature;
  without it the scheme is refused.
- `Counted` tallies what a backend implements, not the derived defaults
  (`glob`, `copy_into`, `read_scalar`), so the tally shows what those decompose
  into; a child from `child_by_path` is the backend's own, not another `Counted`.
- ZIP counts itself: `ZipArchive::handle_reads()`/`handle_writes()`.
