---
name: yggdryl-storage
description: Read and write bytes through yggdryl's one positional handle (IOBase / Holder) on memory, local files, Arrow filesystems, S3 / Google Cloud Storage / Azure Blob, page caches and ZIP archives, with gzip/zlib/zstd codings and charsets. Use when opening a path or URL, reading a range or streaming chunks (read_range_bytes / readRangeBytes, pstream_bytes / pstreamBytes), cursors, listing or globbing a folder (ls, glob, rglob), clear/remove, open/close scopes, compress_into / decompressInto, gzip.dumps / zstd.loads, Charset / charset.decode, digests or read_scalar on a handle, S3Options / S3File, AWS credentials (aws::Session, profiles, SSO, assume role) or a MinIO / S3-compatible endpoint, Buffered caches, Counted call budgets. Covers Rust, Python and Node.js.
---

# Storage: handles, bytes, codings, charsets

Every storage backend is one positional `IOBase` handle; `Holder` is the Rust
enum over all of them and the bindings expose it as one `IOBase` class. A
caller writes the same calls whatever it holds: `pread`/`pwrite` are the
primitives and every whole read, stream, digest, coding and record surface
derives from them. Hold one mental model: **a call is a round trip** - one
syscall, one object-store request, one lock per wrapper - so the fastest code
is the one that makes the fewest calls, and construction makes none.

Records on a handle (`read_arrow_reader`, overwrite/append/merge,
`RecordOptions`, partitions) are `yggdryl-records`; JSON/YAML/TOML/XML codecs
are `yggdryl-documents`; parsing a URL, a glob or a Hive path is `yggdryl-uri`.
Install and cross-language conventions are in `yggdryl`.

## Choose the door

| Task | Rust | Python | JavaScript |
| --- | --- | --- | --- |
| handle from a path, role decided later | `Holder::local(path)?` | `IOBase(path)` (composes coding + media, see below) | `new IOBase(path)`, `IOBase.from(value)` |
| handle from a URL (scheme picks backend) | `Holder::from_url(&url, [("media_type", "...")])?` | `IOBase("s3://b/k")`, `IOBase(Url(...))` | `new IOBase('s3://b/k')` |
| in-memory bytes | `Buffer::new()`, `Buffer::from_bytes(vec)`, `Holder::buffer(b)` | `IOBase.from_bytes(data)` | `IOBase.fromBytes(buf)` |
| declare what bytes are | `Buffer::with_media_type(mt)`, `set_media_type(mt)` | `h.media_type = "text/csv"` | `h.mediaType = 'text/csv'` |
| commit to a role, stored bytes | `LocalFile::new(p)?`, `LocalFolder::new(p)?`, `LocalPath::new(p)?` | `LocalFile(p)`, `LocalFolder(p)`, `LocalPath(p)` | n/a: one `IOBase` class |
| well-known roots | `LocalFolder::temporary()? / home()? / config()?` | `LocalFolder.temporary() / home() / config()` | n/a |
| whole read / write | `read_all_bytes()?`, `write_all_bytes(b)?` | `read_bytes()`, `read_text()`, `write_bytes(b)`, `write_text(s)` | `readBytes()`, `readText()`, `writeBytes(b)`, `writeText(s)` |
| ranged read | `read_range_bytes(offset, len)?` | `read_range_bytes(offset, len)`, `read_range(o, n, cls=str)` | `readRangeBytes(offset, len)`, `readRange(o, n, { text: true })` |
| positional write, append | `pwrite(o, b)?`, `append_bytes(b)?` -> offset | `pwrite(o, b)`, `append_bytes(b)`, `append(str_or_bytes)` | `pwrite(o, b)`, `appendBytes(b)`, `append(strOrBytes)` |
| bounded chunks | `pstream_bytes(position, batch_size)?` | `pstream_bytes(position=0, batch_size=65536)` | `pstreamBytes(position?, batchSize?)` |
| cursor | `cursor()`, `cursor_at(n)` + `IOCursor` (`tell`, `seek_to`, `read_next`, `write_next`, `stream_bytes`), `std::io::Read/Write/Seek` | `cursor(pos)`: `read`, `readinto`, `write`, `seek(o, whence)`, `tell`, `stream_bytes` | `cursor(pos)`: `read`, `write`, `seek`, `tell`, `position`, `streamBytes` |
| `std::io` adapters | `reader_at(o)`, `writer_at(o)` | the cursor is file-like | n/a |
| size, kind, existence | `size()`, `kind()`, `is_container()`, `kind().is_known()` | `size`, `kind`, `exists()`, `is_dir()`, `is_file()` | `size`, `kind`, `exists()`, `isDir()`, `isFile()` |
| child, parent | `child_by_path("a/b.bin")?`, `parent()` | `h / "a/b.bin"`, `joinpath(...)`, `parent` | `joinpath('a/b.bin')`, `parent` |
| list, glob | `ls(recursive, include_private)`, `glob(pattern, include_private)?` | `ls(recursive=False)`, `iterdir()`, `glob(p)`, `rglob(p)` | `ls(recursive?)`, `iterdir()`, `glob(p)`, `rglob(p)`, `[...h]` |
| make a folder | `LocalPath::as_directory()?.create()?`, `truncate(0)?` on a folder | `mkdir()` (returns the folder role) | `mkdir()` |
| empty / delete | `clear()?`, `remove(recursive)?` | `clear()`, `remove(recursive=False)` | `clear()`, `remove(recursive?)` |
| media type, stored coding | `media_type()`, `codec()` | `media_type`, `codec` (`None` = identity) | `mediaType`, `codec` (`null` = identity) |
| decoded view of coded bytes | `Coded::wrap(h, Codec::Gzip)`, `Coded::infer(h)`, `gzip::Gzip::new(h)`, `holder.into_coded()` | `IOBase("x.log.gz")`, `LocalPath(p).into_coded(codec=None, level=None)` | n/a: byte methods address stored bytes |
| move bytes adding/removing a coding | `compress_into(&mut t, codec)?`, `decompress_into(&mut t)?`, `copy_into(&mut t)?` | `compress_into(t, codec=None, level=None)`, `decompress_into(t)`, `copy_into(t)` | `compressInto(t, codec?, level?)`, `decompressInto(t)`, `copyInto(t)` |
| whole-buffer codec | `gzip::dump/load`, `zlib::dump_raw/load_raw`, `zstd::dump_with_level(b, Level::BEST)`, `Codec::Zstd.dump(b)` | `gzip.dumps(b, level=None)`, `gzip.loads(b)`, `zlib.dumps_raw`, `zstd.dumps` | `gzip.dumps(b, level?)`, `gzip.loads(b)`, `zlib.dumpsRaw`, `zstd.dumps` |
| streaming codec | `gzip::reader(r)`, `gzip::writer(w)` + `finish()?`, `Codec::reader/writer` | n/a (use a coded handle) | n/a |
| charset decode / encode | `Charset::Cp1252.decode(b)?`, `.encode(s)?`, `.decode_lossy(b)`, `.transcribe(b)` | `charset.decode(name, b)`, `encode`, `decode_lossy` | `charset.decode(name, b)`, `encode`, `decodeLossy` |
| byte-order mark, names | `Charset::from_bom(b)`, `Charset::from_str(s)?`, `as_str()` | `charset.from_bom(b)`, `bom(name)`, `canonical_name(name)` | `charset.fromBom(b)`, `bom(name)`, `canonicalName(name)` |
| whole resource in a charset | `Transcoded::new(h, Charset::Cp1252)`, `Transcoded::infer(h)` | media type `;charset=` (record reads); `charset.decode(name, h.read_bytes())` | same as Python |
| digest of a handle | `read_digest(DigestAlgorithm::Xxh3)?`, `read_range_digest(o, n, alg)?` | `read_digest("xxh3-64")`, `read_range_digest(o, n)` | `readDigest('xxh3-64')`, `readRangeDigest(o, n)` |
| structured value | `read_scalar(Some(&field))?`, `write_scalar(&v)?` | `read_scalar(field, cls=Scalar)`, `write_scalar(v)` | `readScalar(field)`, `readScalar({ field, scalar: true })`, `writeScalar(v)` |
| scope (cache metadata) | `open()?` ... `close()?` | `with IOBase(p) as h:` | `h.open()` ... `h.close()` |
| page cache | `h.buffered(BufferedOptions::default())` | `h.buffered(page_size=, max_bytes=, ttl=)` (spends `h`) | `h.buffered({ pageSize, maxBytes, ttlMs })` (returns `h`) |
| count calls | `Counted::new(h)`, `calls().get(Call::Pread)`, `counts()` | Rust only | Rust only |
| object store (`s3` feature) | `s3::file(url)?`, `s3::file_with(url, S3Options)?`, `s3::file_at(Provider::Aws, bucket, key)?` | `IOBase("gs://b/k")`, `S3File(url)`, `S3File(bucket, key, provider="s3", options={...})` | `new IOBase('az://c@acct.blob.core.windows.net/k')` |
| foreign Arrow filesystem | `FsFile::from_path(Arc<dyn FileSystem>, path, uri)?` | `IOBase.from_fs(pyarrow_fs, path, uri=None)`, `IOBase.from_uri(uri, options=)` | `IOBase.fromFs(handler, path, uri?)` |
| ZIP archive | `zip::mount(holder)`, `zip::from_url(&url)?`, `ZipArchive::new(h).mount()` | Rust only | Rust only |
| from an open file | n/a | `IOBase(open_file)` (path), `IOBase(io.BytesIO(b))` (content) | n/a |

## Rules for fast, correct use

1. **Count calls, not bytes.** `read_all_bytes` is one call whatever the size;
   a footer is `read_range_bytes(size - 8, 8)` - one ranged `GET`, not the
   object; `size` itself is one `HEAD` on a closed store handle and free on a
   listed handle, inside `open()`, or after `S3File::with_known_size(n)`.
   Slice later needs out of a read you already hold.
2. **Construction touches nothing.** Building a handle, a child
   (`child_by_path`, `/`, `joinpath`), a media type or a partition costs zero
   calls on every backend, S3/GCS/Azure included.
3. **EAFP - never guard.** A read of something absent is empty (`b""`, size
   `0`), a write creates the resource and every missing parent, `remove` of
   something absent succeeds. `exists()`/`is_dir()`/`mkdir()` before an
   operation is a wasted round trip and a race.
4. **Stream bounded chunks.** `pstream_bytes` hands out 64 KiB owned chunks
   and never asks for `size`; digests (`read_digest`) stream too. Reach for
   `read_all_bytes` only when the whole value is the answer.
5. **One scan, one stream on a coded handle.** A `pread` at a compressed
   offset rebuilds a decoder from the start: sixteen ranged reads of a gzip
   value cost sixteen decodes. Drain one `pstream_bytes`, or `open()` it
   (caches the decoded value), or wrap it in `buffered`.
6. **Open a scope for metadata-heavy work.** `open` caches the descriptor and
   mapping (local), the object's metadata (stores: `size` is a `HEAD` closed,
   free open), a Parquet footer, a decoded value; `close` publishes and
   releases. Outside a scope each call fetches fresh - right for a resource
   another writer changes.
7. **The name declares the coding.** `trades.json.gz` is base `application/json`
   plus coding `gzip`: `media_type` names the decoded representation, `codec`
   the stored coding. In-memory bytes are sniffed; declare a type the bytes
   cannot prove (CSV) with `with_media_type` / the `media_type` setter.
8. **Know which bytes a handle presents.** Python `IOBase(name)` composes what
   the name declares (`Text(Gzip(LocalPath))`) and reads decoded bytes; the
   role classes (`LocalPath`, `LocalFile`, `S3File`, `FsPath`) address stored
   bytes. JavaScript byte methods always address stored bytes; the coding is
   applied by `readScalar`/`writeScalar`, the record surface and
   `compressInto`/`decompressInto`. Rust `Holder::local`, `Holder::from_url`
   and the `Local*` roles address stored bytes; `holder.into_coded()` (or
   `into_declared_media()`, which also puts the record encoding on top)
   composes what the name declares - the Rust equivalent of Python `IOBase(name)`.
9. **Wrappers compose over a handle, never inside it.** `Buffered<Coded<_>>`
   caches decoded pages, `Coded<Buffered<_>>` the encoded transport; wrapping
   twice reconfigures the one layer. Python conversions (`buffered`,
   `into_coded`) consume the handle they took.
10. **Text crosses the charset boundary once.** Decode at intake - a
    `Transcoded` handle, a media type's `;charset=`, or `Charset::decode` - and
    work in UTF-8 after. No reader takes a charset argument. `iso-8859-1` is
    never `windows-1252`; bare `utf-16` is refused (say `utf-16le`/`utf-16be`).
11. **Listings are lazy, sorted, and free to stop.** Building a listing costs
    nothing and stopping early stops the walk: taking the first entries reads
    one directory level (all its names, sorted - never the tree) locally, or
    one 1000-entry page on a store. A recursive walk skips `.git`, `.venv`,
    `.DS_Store` unless `include_private`. A glob descends its fixed prefix
    (`year=2024/**/*.parquet`) rather than listing and filtering; an object
    listing states every size, so a listed object is never re-asked.
12. **Join nested children in one string.** `child_by_path("sub/inner.bin")`
    creates the parents on write; joining segment by segment needs each
    intermediate to already be a container.
13. **A local file is memory-mapped.** Another process truncating it raises
    SIGBUS; `copy_into` a `Buffer` when the file may change underneath.
    `flush`/`close` publish the logical length.
14. **Object stores state their request count.** Ranged read: one `GET`;
    whole read/drain/digest: one `GET`; whole write: one `PUT`; listing: one
    request per 1000 entries; remove: one `DELETE`, no probe. `with_known_size`
    skips the `HEAD`; `open()` before `buffered` on a remote handle.
15. **Credentials resolve lazily, explicit wins.** Unset knobs come from the
    URL, the environment (`AWS_`, `GOOGLE_`, `AZURE_`, `YGGDRYL_`), the store's
    files, then defaults; the AWS identity is botocore's chain through
    `aws::Session`. `with_environment(false)` seals everything but explicit
    values - see `references/backends.md`.

## Pitfalls

| Wrong | Right |
| --- | --- |
| `if not h.exists(): h.mkdir()` then write | write; parents are created on the first write |
| reading the whole value to slice off a footer (`read_all_bytes` / `read_bytes()[-8:]`) | Rust `h.read_range_bytes(h.size() - 8, 8)?`, Python `h.read_range_bytes(h.size - 8, 8)`, JS `h.readRangeBytes(h.size - 8, 8)` |
| a loop of `read_range_bytes` over a `.gz` handle | one `pstream_bytes` drain, or `open()`/`buffered` first |
| Python `IOBase("x.gz").read_bytes()` expecting gzip bytes | it is decoded; `LocalPath("x.gz").read_bytes()` is stored |
| JS `new IOBase('x.gz').writeText(s)` expecting a gzip file | writes plain bytes under a `.gz` name; `plain.compressInto(gz)` or `gz.writeBytes(gzip.dumps(b))` |
| Python `cached = h.buffered(); h.read_bytes()` | `h` is spent ("consumed by a conversion"); use `cached` |
| `compress_into(IOBase("x.gz"))` in Python | the target must present stored bytes: `LocalPath("x.gz")` |
| `compress_into(memory_target)` with no codec | a nameless target declares nothing: pass `codec="zstd"` |
| `read_text()` on windows-1252 bytes | it is strict UTF-8: `charset.decode("windows-1252", h.read_bytes())`, or declare `text/plain;charset=windows-1252` for record reads |
| `charset.decode("latin1", ...)` for Windows `0x80` = `€` | `windows-1252` (`cp1252`); ISO 8859-1 maps `0x80` to `U+0080` |
| `folder.remove(); assert not folder.exists()` | a folder handle keeps answering what it was asked for; check the parent's listing |
| JS `root.joinpath('a', 'b.bin')` when `a` does not exist | `root.joinpath('a/b.bin')` |
| treating `read_range_bytes(past_end, n)` as an error | it answers what exists (possibly empty); check the length |
| Python `IOBase.from_uri("s3://...")` expecting the native S3 client | `from_uri` binds `pyarrow.fs.S3FileSystem`; `IOBase("s3://...")`/`S3File` is the native store |
| a `str` of document content passed to `IOBase(...)` | a `str` is a path; content is `IOBase.from_bytes(b)` or `IOBase(io.BytesIO(b))` |
| `s3://my.bucket.com/key` | a first part ending `.com`/`.io` is a host; use `s3::file_at(Provider::Aws, bucket, key)` / `S3File(bucket, key, provider="s3")` |
| logging `bound_uri` | it may carry credentials; log `masked_uri` |

## Language references

- `references/rust.md` - read when writing Rust (`Holder`, `Buffer`, `Coded`, `Transcoded`, `Counted`, `s3`, `zip`).
- `references/python.md` - read when writing Python (`IOBase`, role classes, `yggdryl.gzip`, `yggdryl.charset`).
- `references/javascript.md` - read when writing Node.js (`IOBase`, `gzip`/`zlib`/`zstd`/`charset` namespaces, handler protocol).
- `references/backends.md` - configuration and cost tables for Local, Filesystems, S3 / GCS / Azure, Buffered and ZIP.

## Deeper

- Handles, bytes, values, call counts, every backend: https://platob.github.io/yggdryl/holder/
- Laziness and kinds: https://platob.github.io/yggdryl/holder/#laziness-and-kinds
- Media type and codings: https://platob.github.io/yggdryl/holder/#media-type-and-codings
- Object stores: https://platob.github.io/yggdryl/holder/#object-stores
- ZIP: https://platob.github.io/yggdryl/holder/#zip
- Compression (gzip, zlib, zstd): https://platob.github.io/yggdryl/media/#compression
- Charsets: https://platob.github.io/yggdryl/media/#charsets
- Sibling skills: `yggdryl-records` (rows on a handle, partitions), `yggdryl-uri`
  (URLs, globs, Hive paths), `yggdryl-documents` (JSON/YAML/TOML/XML),
  `yggdryl-hashing` (digest values).
