# yggdryl-storage in Python

`from yggdryl import IOBase`; role classes are in `yggdryl.holder`, the coded
handles in `yggdryl.coding`, codecs in `yggdryl.gzip` / `zlib` / `zstd`,
charsets in `yggdryl.charset`. The wheel carries every backend, S3 included.

## Open a handle for a path or URL

`IOBase(value)` accepts a path, `pathlib.Path`, URL text, a `Url`/`Urn`/`Arn`,
or another handle, and answers the class doing the work - composed with what
the name declares (`Text(Gzip(LocalPath))` for `trades.txt.gz`). Nothing is
opened, created or read.

```python
import pathlib
import tempfile

from yggdryl import IOBase
from yggdryl.holder import LocalPath
from yggdryl.media import Parquet, Text

root = pathlib.Path(tempfile.mkdtemp())

# The class is read off the name; neither location exists.
assert type(IOBase(root / "trades.bin")) is LocalPath
assert type(IOBase(root / "trades.txt.gz")) is Text
assert type(IOBase(root / "trades.parquet")) is Parquet
assert repr(IOBase(root / "a.txt.gz")).startswith("Text(Gzip(LocalPath(")

# A write creates the file and every missing parent.
handle = IOBase(root / "nested" / "ticks.csv")
handle.write_text("symbol,price\nAAPL,1\n")
assert handle.is_file()
assert str(handle.media_type) == "text/csv"
assert handle.url.scheme == "file"
```

## Hold bytes in memory

`IOBase.from_bytes` answers a `holder.Buffer`; its media type is sniffed from
the bytes, so declare one the bytes cannot prove.

```python
from yggdryl import IOBase
from yggdryl.holder import Buffer

sniffed = IOBase.from_bytes(b'{"symbol":"AAPL"}')
assert type(sniffed) is Buffer
assert str(sniffed.media_type.base) == "application/json"
assert sniffed.kind == "memory"
assert sniffed.uri.scheme == "mem"

csv = IOBase.from_bytes(b"symbol,price\n", capacity=4096)
assert str(csv.media_type) == "application/octet-stream"
csv.media_type = "text/csv"
assert str(csv.media_type) == "text/csv"
assert csv.capacity >= 4096
```

## Read a range, write at an offset, append

`read_range_bytes` transfers only the window and is clamped at the end;
`append_bytes` answers the offset the bytes landed at. `read_range(cls=str)`
and `append(str)` are the inferring doors over them.

```python
from yggdryl import IOBase

handle = IOBase.from_bytes()
handle.write_bytes(b"symbol,price\n")
assert handle.append_bytes(b"AAPL,1\n") == 13

assert handle.read_range_bytes(13, 4) == b"AAPL"
assert handle.read_range_bytes(0, 6) == b"symbol"
assert handle.read_range_bytes(100, 4) == b""  # past the end is empty
assert handle.read_range(0, 6, cls=str) == "symbol"

# A write past the end zero-fills the gap.
handle.pwrite(22, b"!")
assert handle.size == 23
assert handle.read_range_bytes(20, 3) == b"\0\0!"

assert handle.append("MSFT") == 23
assert handle.read_text().endswith("!MSFT")
```

## Stream bounded chunks and use a cursor

`pstream_bytes(position, batch_size)` yields owned chunks lazily (64 KiB by
default) and never asks for `size`. A cursor shares the handle, owns one
position, and is file-like.

```python
from yggdryl import IOBase

handle = IOBase.from_bytes(b"0123456789")
assert list(handle.pstream_bytes(2, 3)) == [b"234", b"567", b"89"]

cursor = handle.cursor(1)
stream = cursor.stream_bytes(2)
assert next(stream) == b"12"
assert cursor.tell() == 3

buffer = bytearray(3)
assert cursor.readinto(buffer) == 3
assert bytes(buffer) == b"345"
assert cursor.seek(-2, 2) == 8
assert cursor.read() == b"89"

# A write through a cursor lands on the handle.
writer = IOBase.from_bytes().cursor()
writer.write(b"symbol,price\n")
assert writer.handle.read_bytes() == b"symbol,price\n"
```

## Decide a role: file, folder, or not yet

`LocalPath` is undecided until something is there; a byte write settles it as
a file, `mkdir()` as a folder (returning the folder role). `LocalFile` and
`LocalFolder` commit up front and address stored bytes.

```python
import pathlib
import tempfile

from yggdryl import IOBase
from yggdryl.holder import LocalFile, LocalFolder, LocalPath

root = pathlib.Path(tempfile.mkdtemp())

# Construction touches nothing; absence reads as empty.
absent = IOBase(root / "sub" / "inner.bin")
assert not absent.exists()
assert absent.size == 0
assert absent.read_bytes() == b""
assert not (root / "sub").exists()

location = LocalPath(root / "trades.bin")
assert location.kind == "unknown"
location.write_bytes(b"AAPL")
location.flush()
assert location.kind == "file"

folder = LocalPath(root / "day=2026-08-16").mkdir()
assert isinstance(folder, LocalFolder)
assert folder.kind == "directory"

# A folder holds no bytes: reads are empty, byte writes refused.
try:
    folder.pwrite(0, b"x")
    raise AssertionError("a directory accepted bytes")
except IsADirectoryError as refused:
    assert "got the directory" in str(refused)

assert type(LocalPath(root / "trades.bin").as_file()) is LocalFile
assert isinstance(LocalFolder.temporary(), LocalFolder)
```

## Walk, glob, clear and remove a tree

`/` and `joinpath` take one slash path; listings (`ls`, `iterdir`, `glob`,
`rglob`) are lazy, sorted, and skip dot-names unless `include_private=True`.
`clear` empties and keeps; `remove` deletes and treats absence as success.

```python
import pathlib
import tempfile

from yggdryl import IOBase

root = pathlib.Path(tempfile.mkdtemp()) / "lake"
lake = IOBase(root)
for year in ("2024", "2025"):
    (lake / f"year={year}/month=01/part-0.bin").write_bytes(b"x")
(root / ".git").mkdir()

assert [entry.name for entry in lake.ls()] == ["year=2024", "year=2025"]
assert len(list(lake.ls(False, True))) == 3
assert len(list(lake.ls(True))) == 6
assert len(list(lake.glob("year=2024/**/*.bin"))) == 1
assert len(list(lake.rglob("*.bin"))) == 2

# Clear keeps the container; remove refuses children unless recursive.
first = lake / "year=2024"
first.clear()
assert list(first.iterdir()) == []
try:
    lake.remove()
    raise AssertionError("a non-empty folder was removed")
except Exception as error:
    assert "children" in str(error)
lake.remove(recursive=True)
lake.remove(recursive=True)  # absent: a no-op success
assert not root.exists()
```

## Media type and coding from the name

`media_type` is what the handle presents (decoded), `codec` what the store
holds (`None` for no coding). `IOBase` presents decoded bytes;
`LocalPath` presents the stored ones.

```python
import gzip
import pathlib
import tempfile

from yggdryl import IOBase
from yggdryl.holder import LocalPath

path = pathlib.Path(tempfile.mkdtemp()) / "app.log.gz"
handle = IOBase(path)
handle.write_bytes(b"symbol,price\n")

assert handle.read_bytes() == b"symbol,price\n"
assert str(handle.media_type) == "text/plain"
assert handle.codec == "gzip"
assert gzip.decompress(path.read_bytes()) == b"symbol,price\n"

stored = LocalPath(path)
assert stored.read_bytes()[:2] == b"\x1f\x8b"
assert stored.codec == "gzip"
assert IOBase(path.with_name("plain.csv")).codec is None
```

## Read and write through a coding

`into_coded(codec=None, level=None)` puts a coding on a handle (the name's by
default) and consumes it; `compress_into`/`decompress_into` move every byte
into another handle presenting stored bytes, adding or removing a coding.

```python
import pathlib
import tempfile

from yggdryl import IOBase
from yggdryl.coding import Gzip, Zstd
from yggdryl.holder import LocalPath

root = pathlib.Path(tempfile.mkdtemp())

memory = IOBase.from_bytes().into_coded("zstd", level=9)
assert isinstance(memory, Zstd)
memory.write_bytes(b"symbol,price\n" * 64)
assert memory.read_bytes() == b"symbol,price\n" * 64
assert memory.into_handle().read_bytes()[:4] == bytes.fromhex("28b52ffd")

plain = IOBase(root / "rows.json")
plain.write_bytes(b'{"symbol":"AAPL"}')

stored = LocalPath(root / "rows.json.gz")  # stored bytes: the target a coding lands in
assert plain.compress_into(stored) == stored.size
assert stored.read_bytes()[:2] == b"\x1f\x8b"
coded = stored.into_coded()
assert isinstance(coded, Gzip)
assert coded.read_text() == '{"symbol":"AAPL"}'

# A nameless target declares no coding, so name one.
target = IOBase.from_bytes()
plain.compress_into(target, codec="zstd")
assert target.codec == "zstd"
restored = IOBase.from_bytes()
assert target.decompress_into(restored) == 17
assert restored.read_bytes() == b'{"symbol":"AAPL"}'
```

## Compress bytes without a handle

`yggdryl.gzip`, `zlib` and `zstd` are `dumps`/`loads` pairs over whole
buffers, wire-compatible with the standard library; `level` is the shared
0-9 scale. `zlib` adds the raw DEFLATE pair.

```python
import gzip as standard
import zlib as standard_zlib

from yggdryl import gzip, zlib, zstd

payload = b"symbol,price\n" + b"AAPL,1\n" * 64

encoded = gzip.dumps(payload, level=9)
assert gzip.loads(encoded) == payload
assert standard.decompress(encoded) == payload

raw = zlib.dumps_raw(payload)
assert zlib.loads_raw(raw) == payload
assert len(zlib.dumps(payload)) == len(raw) + 6
assert standard_zlib.decompress(raw, -standard_zlib.MAX_WBITS) == payload
try:
    zlib.loads(raw)
    raise AssertionError("raw DEFLATE read as framed zlib")
except ValueError:
    pass

frame = zstd.dumps(payload)
assert len(frame) < len(payload)
assert zstd.loads(frame) == payload
```

## Decode a charset once

`yggdryl.charset` answers what `bytes.decode(name)` answers, over the same
names, with the crate's refusal (charset, byte position, byte). A whole
resource is decoded by declaring the charset on its media type; record reads
then arrive as `str`. `read_text` is strict UTF-8.

```python
from yggdryl import IOBase, charset

wire = b"prix: 12\x80"
assert charset.decode("windows-1252", wire) == "prix: 12€"
assert charset.encode("windows-1252", "prix: 12€") == wire
assert charset.decode("iso-8859-1", wire) == wire.decode("iso-8859-1")
assert charset.canonical_name("cp1252") == "windows-1252"

try:
    charset.decode("us-ascii", b"caf\xe9")
    raise AssertionError("invalid ASCII decoded")
except ValueError as error:
    assert "us-ascii" in str(error)
assert charset.decode_lossy("us-ascii", b"caf\xe9") == "caf�"

assert charset.from_bom(b"\xef\xbb\xbfid") == ("utf-8", 3)
assert charset.bom("windows-1252") is None

# A declared charset is read once, below the line splitter.
handle = IOBase.from_bytes("Zürich\n".encode("cp1252"))
handle.media_type = "text/plain;charset=windows-1252"
assert [row["body"] for row in handle.read_records()] == ["Zürich"]
```

## Digest or parse the value a handle holds

`read_digest` streams and holds one chunk; `read_scalar` picks
JSON/YAML/TOML/XML and any outer coding from the media type, restores field
names, and answers the core value with `cls=Scalar`.

```python
import pathlib
import tempfile

from yggdryl import IOBase, Scalar, xxhash

handle = IOBase.from_bytes(b"symbol,price\nAAPL,1\n")
assert handle.read_digest() == xxhash.digest(handle.read_bytes(), "xxh3-64")
assert handle.read_range_digest(0, 6, "xxh32") == xxhash.digest(b"symbol", "xxh32")

document = IOBase(pathlib.Path(tempfile.mkdtemp()) / "trade.json.gz")
document.write_scalar({"quantity": 2, "symbol": "AAPL"})
field = "trade: struct<quantity: int32 not null, symbol: utf8 not null> not null"
assert document.read_scalar(field) == {"quantity": 2, "symbol": "AAPL"}
assert document.read_scalar(field, cls=Scalar).kind == "serie"
```

## Repeat reads: open a scope or add a page cache

`with` opens and closes: metadata work inside reuses what `open` cached, and
`close` publishes the bytes at their exact length. `buffered(...)` answers a
`holder.Buffered` page cache and spends the handle it took.

```python
import pathlib
import tempfile

from yggdryl import IOBase
from yggdryl.holder import Buffer, Buffered

path = pathlib.Path(tempfile.mkdtemp()) / "trades.csv"
with IOBase(path) as handle:
    handle.write_text("symbol,price\n")
    assert handle.opened
assert path.stat().st_size == 13
assert IOBase(path).read_text() == "symbol,price\n"

source = IOBase.from_bytes(bytes(16 * 64))
cached = source.buffered(page_size=64, max_bytes=4 * 64, ttl=30.0)
assert type(cached) is Buffered
cached.read_range_bytes(16 * 64 - 8, 8)  # footer first, as a container opens
cached.read_range_bytes(0, 8)
for page in range(1, 15):
    cached.read_range_bytes(page * 64, 8)
assert cached.cached_bytes <= 4 * 64
assert cached.has_cached_page(0) and cached.has_cached_page(15)

try:
    source.read_bytes()
    raise AssertionError("a spent handle answered")
except ValueError as error:
    assert "consumed by a conversion" in str(error)
assert type(cached.into_handle()) is Buffer
```

## Name an object on S3, Google Cloud Storage or Azure

The scheme picks the store (`s3`/`s3a`/`s3n`, `gs`/`gcs`,
`az`/`abfs`/`abfss`/`wasb`/`wasbs`); `S3File(bucket, key, provider=...)`
takes a raw key; `options=` takes PyIceberg / PyArrow / crate property names.
Construction makes no request - the first read or write does.

```python
from yggdryl import IOBase
from yggdryl.holder import S3File, S3Folder, S3Path

leaf = S3File("s3://trades/lake/year=2026/part.parquet")
assert str(leaf.media_type) == "application/vnd.apache.parquet"
assert (leaf.url.bucket, leaf.url.key) == ("trades", "lake/year=2026/part.parquet")
assert leaf.partitions == (("year", "2026"),)

# A raw key is escaped by the handle, not by the caller.
raw = S3File("trades", "lake/a b/part.parquet", provider="s3")
assert str(raw.url) == "s3://trades/lake/a%20b/part.parquet"

google = IOBase("gs://trades/lake/part.bin")
assert isinstance(google, S3Path)
assert isinstance(google.parent, S3Folder)

configured = S3File(
    "s3://trades/lake/part.parquet",
    options={"s3.endpoint": "http://localhost:9000", "region": "us-east-1", "anonymous": True},
)
assert configured.url.scheme == "s3"
```

## Bind a pyarrow filesystem

`IOBase.from_fs(filesystem, path, uri=None)` wraps any `pyarrow.fs`
filesystem; the path is opaque and reaches it literally. `IOBase.from_uri`
resolves a URI into a pyarrow filesystem once (options forwarded to
`pyarrow.fs.S3FileSystem`).

```python
import pyarrow.fs as pafs

from yggdryl import IOBase
from yggdryl.holder import FsFolder

filesystem = pafs._MockFileSystem()
folder = IOBase.from_fs(filesystem, "bucket").create_dir()
assert isinstance(folder, FsFolder)

handle = IOBase.from_fs(filesystem, "bucket/v=a%2Fb.bin", uri="s3://bucket/v=a%2Fb.bin")
with handle.open_output_stream(compression=None) as output:
    output.write(b"literal")
assert handle.read_range_bytes(2, 3) == b"ter"
assert handle.path == "bucket/v=a%2Fb.bin"
assert handle.masked_uri == "s3://bucket/v=a%2Fb.bin"

archive = IOBase.from_fs(filesystem, "bucket/v=a%2Fb.copy.bin")
assert handle.copy_into(archive) == 7
assert archive.read_bytes() == b"literal"
```

## Wrap an open file or stream

A named file captures its location; a nameless stream (`io.BytesIO`) is taken
as content.

```python
import io
import pathlib
import tempfile

from yggdryl import IOBase

target = pathlib.Path(tempfile.mkdtemp()) / "quotes.json"
target.write_bytes(b"{}")
with open(target, "rb") as stream:
    handle = IOBase(stream)
assert handle.name == "quotes.json"

content = IOBase(io.BytesIO(b'{"symbol": "AAPL"}'))
content.media_type = "application/json"
assert content.read_scalar() == {"symbol": "AAPL"}
```

## Rust only

Not bound in Python - reach for the Rust crate, never an invented name:
`Counted` call tallies, ZIP archives (`zip::mount`), `Transcoded` handles,
streaming codec `reader`/`writer`, `reader_at`/`writer_at`, the
`S3Options`/`aws::Session` builders (Python passes the same knobs as
`options=` properties), `mtime()`.

## Gotchas in Python

- A `str` argument to `IOBase(...)` is a path, never content; content is
  `IOBase.from_bytes(b)` or `IOBase(io.BytesIO(b))`.
- `IOBase("x.gz")` presents decoded bytes; `LocalPath`, `LocalFile`, `S3File`
  and `FsPath` address stored bytes - use them as `compress_into` targets.
- `buffered`, `into_coded`, `into_text`, `into_media` consume the handle:
  the old reference raises `ValueError("... consumed by a conversion")`.
- `write_bytes`, `pwrite`, `append_bytes` take `bytes` only (borrowed, no
  copy); `append` also takes `str`, `bytearray`, `memoryview`, at a copy.
- `size`, `kind`, `media_type`, `codec`, `parent`, `opened`, `closed` are
  properties; `exists()`, `is_dir()`, `is_file()` are methods.
- A folder handle keeps answering `is_dir()` after `remove()`; the parent's
  listing is what shows it gone.
- `IOBase.from_uri("s3://...")` goes through `pyarrow.fs.S3FileSystem`;
  `IOBase("s3://...")` and `S3File` are the native client.
- Errors map to `ValueError`, `IsADirectoryError`, `OSError` with the native
  message.
