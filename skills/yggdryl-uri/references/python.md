# yggdryl-uri in Python

`from yggdryl import Uri, Url, Urn, Arn` (also `yggdryl.uri`). `Url`, `Urn`
and `Arn` subclass `Uri`, so `Uri(value)` answers the narrowing the scheme
names. `Url` answers the `pathlib.PurePath` questions under their `pathlib`
names. Errors are `ValueError` naming the byte offset.

## Parse any identifier and read its components

`Uri(value)` takes text, a `PathLike`, or another identifier. `query`,
`fragment` and `path_text` are methods with `decode=False`, answering the
stored text; everything else is a property.

```python
from yggdryl import Uri, Url

uri = Uri("HTTPS://example.test/archive/caf%c3%a9.tar.gz?as%20of=1#part")
assert type(uri) is Url  # the scheme names a location

assert str(uri) == "https://example.test/archive/caf%C3%A9.tar.gz?as%20of=1#part"
assert uri.scheme == "https"
assert uri.authority == "example.test"
assert uri.path == "/archive/caf%C3%A9.tar.gz"
assert uri.query() == "as%20of=1"
assert uri.query(decode=True) == "as of=1"
assert uri.path_text(decode=True) == "/archive/café.tar.gz"
assert uri.fragment() == "part"

built = Uri.from_parts("https", "example.test", "/a", "q=1")
assert str(built) == "https://example.test/a?q=1"
```

## Compare two spellings of one resource

Parsing canonicalizes, so equality and hashing compare resources, not text.
A string with no scheme token is a file path.

```python
import copy

from yggdryl import Uri

assert str(Uri(r"file:///C:\Users\Ada\report.parquet")) == "file:///C:/Users/Ada/report.parquet"
assert str(Uri("/var/lib/data.arrow")) == "file:///var/lib/data.arrow"
assert str(Uri("data/ticks.csv")) == "file:data/ticks.csv"
assert Uri("/data/2026-08-16T00:00:00/p.parquet").scheme == "file"

try:
    Uri("bad scheme://x")
    raise AssertionError("an invalid scheme token parsed")
except ValueError:
    pass

seen = {Uri("HTTPS://x.test/caf%c3%a9"), Uri("https://x.test/caf%C3%A9")}
assert len(seen) == 1

# The first hash locks a wrapper against setters; a copy is editable again.
locked = Uri("https://x.test/report.csv")
hash(locked)
try:
    locked.set_stem("other")
    raise AssertionError("a hashed wrapper changed")
except TypeError:
    pass
editable = copy.copy(locked)
editable.set_stem("other")
assert str(editable) == "https://x.test/other.csv"
assert locked.stable_hash() == Uri("https://x.test/report.csv").stable_hash()
assert Uri.from_json(locked.into_json()) == locked
```

## Narrow to a location, a name or an ARN

`into_url()`/`into_urn()`/`into_arn()` and `Url.from_uri` refuse what a value
is not. `Url.from_str` is strict; the `Url(...)` constructor is the location
door: it roots a relative path at the working directory and resolves a name
to where it points.

```python
from yggdryl import Arn, Uri, Url, Urn

uri = Uri("https://example.test/a/data.json?raw=true")
assert Uri(uri.into_url()) == uri

urn = Uri("URN:ISBN:9780131103627")
assert type(urn) is Urn
assert str(urn) == "urn:isbn:9780131103627"
assert isinstance(Uri("arn:aws:s3:::b/k"), Arn)

for rejected in (
    lambda: urn.into_uri().into_url(),
    lambda: Url("mailto:user@example.test"),
    lambda: Url.from_str("data/ticks.csv"),  # strict: relative text names no location
    lambda: Url.from_uri(urn),  # strict: a name is not a location
):
    try:
        rejected()
        raise AssertionError("expected a rejection")
    except ValueError:
        pass

rooted = Url("data/ticks.csv")  # the location door
assert rooted.is_local()
assert str(rooted).endswith("/data/ticks.csv")
assert Url("arn:aws:s3:::market-data/part.parquet") == Url("s3://market-data/part.parquet")
```

## Convert platform paths

`from_path` takes `str` or any `os.PathLike` (including `PureWindowsPath`)
and detects drives and UNC shares textually; `into_path` (also `__fspath__`)
decodes back and refuses anything that is not a `file:` path.

```python
import os
from pathlib import PureWindowsPath

from yggdryl import Uri, Url

uri = Uri.from_path(PureWindowsPath(r"C:\Users\Ada Lovelace\report.parquet"))
assert str(uri) == "file:///C:/Users/Ada%20Lovelace/report.parquet"
assert uri.into_path() == "C:/Users/Ada Lovelace/report.parquet"
assert os.fspath(uri) == uri.into_path()
assert Uri.from_path(uri.into_path()) == uri

unc = Uri.from_path(r"\\server\share\prices\ticks.csv")
assert str(unc) == "file://server/share/prices/ticks.csv"
assert unc.into_path() == "//server/share/prices/ticks.csv"

assert str(Url.from_path("relative/x.csv")).endswith("/relative/x.csv")
for refused in (Url("https://example.test/data.csv"), Uri("file:///lake/a%2Fb")):
    try:
        refused.into_path()
        raise AssertionError("expected a refusal")
    except ValueError:
        pass
```

## Read and rename the filename; read the media type

Filename accessors are properties; setters are atomic and change only the
filename. `Url` adds `pathlib`'s `name`, `suffix`, `suffixes` and the copying
`with_name`/`with_stem`/`with_suffix`.

```python
from yggdryl import MediaType, MimeType, Uri, Url

uri = Uri("https://example.test/archive/report.csv.gz?q=1#part")
assert (uri.file_name, uri.stem, uri.extension) == ("report.csv.gz", "report.csv", "gz")
assert uri.extensions == ("csv", "gz")
assert uri.mime_type == MimeType("application/gzip")
assert uri.media_type.base == MimeType("text/csv")
assert uri.media_type.encodings == (MimeType("application/gzip"),)

uri.set_stem("renamed")
assert str(uri) == "https://example.test/archive/renamed.gz?q=1#part"
uri.set_media_type(MediaType.from_parts(MimeType("application/json"), [MimeType("application/zstd")]))
assert uri.file_name == "renamed.json.zst"
assert uri.clear_extensions() is True

unchanged = str(uri)
for bad in (lambda: uri.set_file_name("bad/name"), lambda: uri.set_mime_type("application/vnd.example")):
    try:
        bad()
        raise AssertionError("a refused name was applied")
    except ValueError:
        pass
assert str(uri) == unchanged

url = Url("file:///lake/trades.csv.gz")
assert (url.name, url.suffix, url.suffixes) == ("trades.csv.gz", ".gz", (".csv", ".gz"))
assert str(url.with_suffix(".zst")) == "file:///lake/trades.csv.zst"
assert str(url.with_name("quotes.csv")) == "file:///lake/quotes.csv"
```

## Walk and join paths

`joinpath(*others)` and `/` take URI text and resolve `.`/`..` like `cd`; an
`os.PathLike` argument is encoded as names instead. `parts` is the resolved
path; `parents` climbs to the root.

```python
from pathlib import PurePosixPath

from yggdryl import Url

url = Url("https://example.test/a/b/c?q=1#frag")
assert str(url.joinpath("../d")) == "https://example.test/a/b/d?q=1#frag"
assert str(url / "d") == "https://example.test/a/b/c/d?q=1#frag"
assert str(url.joinpath("d", "e")) == "https://example.test/a/b/c/d/e?q=1#frag"
assert str(url / "/root") == "https://example.test/root?q=1#frag"

assert url.parts == ("a", "b", "c")
assert [parent.path for parent in url.parents] == ["/a/b", "/a", "/"]
assert url.parent.path == "/a/b"
assert url.path_segments == ("a", "b", "c")
assert (len(url), url[0], url[-1], "b" in url) == (3, "a", "c", True)

lake = Url("file:///lake")
try:
    lake.joinpath("100%.csv")  # URI text: a bare `%` is not an escape
    raise AssertionError("expected a refusal")
except ValueError:
    pass
assert str(lake.joinpath(PurePosixPath("100%.csv"))) == "file:///lake/100%25.csv"
assert Url("file:///lake/year=2024/p.bin").relative_to("file:///lake") == "year=2024/p.bin"
assert Url("file:///lake/p.bin").is_relative_to("file:///lake")
```

## Read and edit query parameters

`parameters(decode=False)` is a live, dict-like view of the query: a key may
repeat (`get_all`), `[k] = v` replaces the first and drops the rest, and an
edit writes back to the value it came from.

```python
from yggdryl import Url

url = Url("https://example.com/t?symbol=AAPL&venue=XNAS&symbol=MSFT&note=a%26b")
parameters = url.parameters()
assert len(parameters) == 4
assert parameters["symbol"] == "AAPL"
assert parameters.get_all("symbol") == ("AAPL", "MSFT")
assert parameters["note"] == "a%26b"
assert url.parameters(decode=True)["note"] == "a&b"

parameters["symbol"] = "TSLA"
del parameters["venue"]
del parameters["note"]
decoded = url.parameters(decode=True)
decoded["side"] = "buy & sell"
assert str(url) == "https://example.com/t?symbol=TSLA&side=buy%20%26%20sell"

try:
    url.parameters()["side"] = "buy & sell"  # a raw view refuses what it cannot write back
    raise AssertionError("expected a refusal")
except ValueError as error:
    assert "uri query" in str(error)

url.set_query(None)
assert url.query() is None
```

## Read credentials and object-store locations

Everything is read off the authority, with no request. A first part ending
`.com`/`.io`, with a port, an IP literal, or `localhost` is the host;
otherwise it is the bucket.

```python
from yggdryl import Uri

secured = Uri("https://user:pass:word@example.com:8443/data")
assert (secured.user, secured.password, secured.hostname) == ("user", "pass:word", "example.com")
assert (secured.port, secured.default_port) == (8443, 443)

s3 = Uri("s3://trades.s3.eu-west-3.amazonaws.com/lake/part.parquet")
assert (s3.bucket, s3.region, s3.key) == ("trades", "eu-west-3", "lake/part.parquet")
assert s3.is_virtual_hosted()

minio = Uri("s3://localhost:9000/trades/lake/")
assert (minio.hostname, minio.bucket, minio.key) == ("localhost", "trades", "lake/")

azure = Uri("abfss://lake@trades.dfs.core.windows.net/part.parquet")
assert (azure.bucket, azure.account, azure.key) == ("lake", "trades", "part.parquet")
assert Uri("gs://trades.storage.googleapis.com/x").store_endpoint == "storage.googleapis.com"
```

## Detect, split and match globs

A URL glob is spelled with `*` in its path; `glob_parts()` answers the deepest
fixed location and the pattern below it. `match` follows the `.gitignore`
rule; `full_match_under(root, pattern)` matches the path below a root.

```python
from yggdryl import Url

pattern = Url("file:///lake/trades/year=2024/**/*.parquet")
assert pattern.is_glob()
assert pattern.is_recursive_glob()
root, rest = pattern.glob_parts()
assert str(root) == "file:///lake/trades/year=2024"
assert rest == "**/*.parquet"
assert Url("file:///lake/trades.arrows").glob_parts()[1] is None
assert Url.is_pattern("part-?.parquet")

part = Url("file:///lake/trades/year=2024/month=01/part-0.parquet")
assert part.match("*.parquet")  # no `/`: the name, at any depth
assert part.match("lake/**/part-?.parquet")  # a `/`: anchored at the root
assert not part.match("lake/*.parquet")
assert part.full_match_under(root, "**/*.parquet")
```

## Read Hive partitions and select leaves by them

`partitions` are the `column=value` directories in path order;
`partitions_under(root)` answers only those below a table root. A handle's
`children_where` selects leaves by path with no call per file.

```python
import pathlib
import tempfile

from yggdryl import IOBase, Url

part = Url("file:///lake/year=2024/month=01/part-0.parquet")
assert part.partitions == (("year", "2024"), ("month", "01"))
assert part.partition("month") == "01"
assert part.partition("day") is None
assert part.is_partitioned()
assert part.partitions_under("file:///lake/year=2024") == (("month", "01"),)
assert str(Url("file:///lake").with_partition("year", "2025")) == "file:///lake/year=2025"

root = pathlib.Path(tempfile.mkdtemp()) / "lake"
for year in ("2024", "2025"):
    (IOBase(root) / f"year={year}/month=01/part-0.bin").write_bytes(b"x")
selected = list(IOBase(root).children_where({"year": "2024"}))
assert len(selected) == 1
assert selected[0].partitions == (("year", "2024"), ("month", "01"))
```

## Resolve a URN to a location

A URN's namespace and its `:`-separated parts are a relative path;
`resolve(base)` places it under a store, `locator()` under the working
directory.

```python
from yggdryl import Uri, Url, Urn

urn = Urn("urn:lake:trades:2026:part.parquet")
assert (urn.namespace, urn.namespace_specific) == ("lake", "trades:2026:part.parquet")
assert urn.locator_path() == "lake/trades/2026/part.parquet"
assert urn.resolve("s3://market-data/warehouse/") == Url(
    "s3://market-data/warehouse/lake/trades/2026/part.parquet"
)

located = urn.locator()
assert located.is_local()
assert Uri("urn:lake:trades:2026:part.parquet").locator() == located
assert Url(urn) == located  # the location door resolves a name

for refused in (lambda: Urn("urn:example:a::b").locator_path(), lambda: Urn("urn:isbn:")):
    try:
        refused()
        raise AssertionError("expected a refusal")
    except ValueError:
        pass
```

## Read an ARN and locate an S3 object

An ARN is five AWS fields plus a service-owned resource. Only an Amazon S3
bucket ARN (and an S3 Tables ARN) locates a URL.

```python
from yggdryl import Arn, Url

obj = Arn("arn:aws:s3:::market-data/2026/part.parquet")
assert (obj.partition, obj.service, obj.region, obj.account) == ("aws", "s3", None, None)
assert (obj.bucket, obj.key, obj.file_name) == ("market-data", "2026/part.parquet", "part.parquet")
assert obj.locator() == Url("s3://market-data/2026/part.parquet")

function = Arn("arn:aws:lambda:us-east-1:123456789012:function:trades:1")
assert (function.resource_type, function.resource_id, function.resource_separator) == (
    "function",
    "trades:1",
    ":",
)
try:
    function.locator()
    raise AssertionError("a Lambda ARN located a URL")
except ValueError:
    pass

table = Arn("arn:aws:s3tables:us-east-1:123456789012:bucket/lake/table/t-a1")
assert (table.bucket, table.table, table.key) == ("lake", "t-a1", None)
assert table.locator() == Url("s3tables://lake/t-a1")
assert str(Arn.from_parts("aws", "s3", "", "", "b/k")) == "arn:aws:s3:::b/k"
```

## Ask the local filesystem

`exists`, `is_dir`, `is_file`, `is_local` and `local_mime_type` look at the
disk for `file:` and answer `False` for any other scheme, with no network
call. `is_private` judges the last segment (a dot-name).

```python
import pathlib
import tempfile

from yggdryl import MimeType, Url

root = pathlib.Path(tempfile.mkdtemp())
(root / "ticks.csv").write_text("symbol\n")

folder = Url.from_path(root)
assert folder.is_local() and folder.is_dir()
assert folder.local_mime_type == MimeType("inode/directory")
assert (folder / "ticks.csv").is_file()
assert Url("file:///project/.git").is_private()

remote = Url("https://example.test/ticks.csv")
assert not remote.is_local() and not remote.exists()
assert remote.local_mime_type == MimeType("text/csv")
```

## Declare a column of URLs or URNs

`yggdryl.url(name)` and `yggdryl.urn(name)` build the two `uri`-family
fields; text entering either is parsed and canonicalized, and relative text
or the other leaf's identifier is refused.

```python
import yggdryl
from yggdryl import DataType, Scalar, json

location = yggdryl.url("location", nullable=False)
name = yggdryl.urn("name", nullable=False)
assert DataType("urn").kind == "text"

located = json.loads('"HTTPS://example.com/a"', field=location, cls=Scalar)
assert located.as_py() == "https://example.com/a"
named = json.loads('"URN:ISBN:0451450523"', field=name, cls=Scalar)
assert named.as_py() == "urn:isbn:0451450523"

try:
    json.loads('"./relative"', field=location, cls=Scalar)
    raise AssertionError("a relative location entered a url column")
except ValueError:
    pass
```

## Gotchas in Python

- `query()`, `fragment()`, `path_text()` are **methods** (`decode=False`);
  `scheme`, `authority`, `path`, `bucket`, `key`, `file_name`, `parts`,
  `parent`, `partitions` are properties.
- `Url(text)` is the location door (roots relative text at the working
  directory, resolves URNs and S3 ARNs); `Url.from_str(text)` and
  `Url.from_uri(value)` are strict.
- `hash(uri)` locks that wrapper: a later setter raises `TypeError`. Use
  `copy.copy` for an editable copy; `stable_hash()` never locks.
- A `Parameters` view is live: assigning through it rewrites the URL, and a
  hashed (locked) URL refuses the write.
- `Urn` and `Arn` filename accessors read the namespace-specific string /
  the resource, and their setters leave the other fields alone.
- Globs, `match`, partitions, `relative_to` and the local predicates are on
  `Url`; `Uri(...)` already answers a `Url` when the scheme names a location.
- `Scheme` is not bound: `scheme` is a `str`; ask `default_port` and
  `is_storage()` on the value.
