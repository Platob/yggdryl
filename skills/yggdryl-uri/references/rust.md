# yggdryl-uri in Rust

`use yggdryl::{Uri, Url, Urn, Arn, Scheme};` - all default features. The
values are `Clone + Eq + Ord + Hash + Display + serde`; every parse returns
`yggdryl::Result` with the failing byte offset.

## Parse any identifier and read its components

`Uri::from_str` accepts every spelling; components are borrowed `&str`, and
`query`/`fragment`/`path_text` take `decode` and answer the stored text when
it is `false`.

```rust
use yggdryl::uri::{Authority, UriPath};
use yggdryl::{Scheme, Uri};

let uri = Uri::from_str("HTTPS://example.test/archive/caf%c3%a9.tar.gz?as%20of=1#part")?;

assert_eq!(uri.to_string(), "https://example.test/archive/caf%C3%A9.tar.gz?as%20of=1#part");
assert_eq!(uri.scheme().as_str(), "https");
assert_eq!(uri.authority().as_str(), "example.test");
assert_eq!(uri.path().as_str(), "/archive/caf%C3%A9.tar.gz");
assert_eq!(uri.query(false)?.as_deref(), Some("as%20of=1"));
assert_eq!(uri.query(true)?.as_deref(), Some("as of=1"));
assert_eq!(uri.path_text(true)?, "/archive/café.tar.gz");
assert_eq!(uri.fragment(false)?.as_deref(), Some("part"));

// Built from parts, validated the same way.
let built = Uri::from_parts(
    Scheme::HTTPS,
    Authority::from_str("example.test")?,
    UriPath::from_str("/a")?,
    Some("q=1".into()),
    None,
)?;
assert_eq!(built.to_string(), "https://example.test/a?q=1");
```

## Compare two spellings of one resource

Parsing canonicalizes, so equality, hashing and ordering compare resources,
not text. A string with no scheme token is a file path.

```rust
use std::collections::HashSet;

use yggdryl::Uri;

let windows = Uri::from_str(r"file:///C:\Users\Ada\report.parquet")?;
assert_eq!(windows.to_string(), "file:///C:/Users/Ada/report.parquet");

assert_eq!(Uri::from_str("/var/lib/data.arrow")?.to_string(), "file:///var/lib/data.arrow");
assert_eq!(Uri::from_str("data/ticks.csv")?.to_string(), "file:data/ticks.csv");
assert_eq!(Uri::from_str("file:/data")?.to_string(), "file:///data");

// A colon after the first separator is data, not a scheme.
assert_eq!(Uri::from_str("/data/2026-08-16T00:00:00/p.parquet")?.scheme().as_str(), "file");
// An invalid scheme token is an error, never a silent path.
assert!(Uri::from_str("bad scheme://x").is_err());

let seen: HashSet<Uri> = ["HTTPS://x.test/caf%c3%a9", "https://x.test/caf%C3%A9"]
    .into_iter()
    .map(Uri::from_str)
    .collect::<yggdryl::Result<_>>()?;
assert_eq!(seen.len(), 1);
let one = seen.into_iter().next().unwrap();
assert_eq!(Uri::from_json(&one.clone().into_json()?)?, one);
assert_eq!(one.stable_hash(), Uri::from_str("https://x.test/caf%C3%A9")?.stable_hash());
```

## Narrow to a location, a name or an ARN

`into_url`/`into_urn`/`into_arn` and `Url::from_uri` re-use the parsed value
and refuse what it is not. `Url::from_str` is strict; `Url::from_location` is
the door for user input: it roots a relative path at the working directory and
resolves a name to where it points.

```rust
use yggdryl::{Arn, Uri, Url};

let uri = Uri::from_str("https://example.test/a/data.json?raw=true")?;
let url = Url::from_uri(uri.clone())?;
assert_eq!(Uri::from(&url), uri);

let urn = Uri::from_str("URN:ISBN:9780131103627")?.into_urn()?;
assert_eq!(urn.to_string(), "urn:isbn:9780131103627");
assert!(urn.clone().into_uri().into_url().is_err());
assert!(Url::from_str("mailto:user@example.test").is_err());

// Strict: relative text names no location.
assert!(Url::from_str("data/ticks.csv").is_err());
// The location door: rooted at the working directory.
let rooted = Url::from_location("data/ticks.csv")?;
assert!(rooted.is_local());
assert!(rooted.to_string().ends_with("/data/ticks.csv"));

// An S3 ARN is a name for an s3: location.
let arn: Arn = Uri::from_str("arn:aws:s3:::market-data/part.parquet")?.into_arn()?;
assert_eq!(Url::from_location(&arn.to_string())?.to_string(), "s3://market-data/part.parquet");
```

## Convert platform paths

`from_path` detects drives and UNC shares textually (same answer on every
host) and encodes names; `into_path` decodes them back and refuses anything
that is not a `file:` path.

```rust
use std::path::PathBuf;

use yggdryl::{Uri, Url};

let uri = Uri::try_from(PathBuf::from(r"C:\Users\Ada Lovelace\report.parquet"))?;
assert_eq!(uri.to_string(), "file:///C:/Users/Ada%20Lovelace/report.parquet");
assert_eq!(uri.authority().as_str(), "");
assert_eq!(PathBuf::try_from(&uri)?, PathBuf::from("C:/Users/Ada Lovelace/report.parquet"));

let unc = Uri::from_path(r"\\server\share\prices\ticks.csv")?;
assert_eq!(unc.to_string(), "file://server/share/prices/ticks.csv");
assert_eq!(unc.into_path()?, PathBuf::from("//server/share/prices/ticks.csv"));

// A `Url` is absolute, so a relative path is rooted at the working directory.
assert!(Url::from_path("relative/x.csv")?.to_string().ends_with("/relative/x.csv"));

// Only a `file:` identifier has a path, and an escape never becomes structure.
assert!(Url::from_str("https://example.test/data.csv")?.into_path().is_err());
assert!(Uri::from_str("file:///lake/a%2Fb")?.into_path().is_err());
```

## Read and rename the filename; read the media type

Filename accessors borrow; setters are atomic and change only the filename.
The last suffix is the MIME type, the chain the media type.

```rust
use yggdryl::{MediaType, MimeType, Uri};

let mut uri = Uri::from_str("https://example.test/archive/report.csv.gz?q=1#part")?;
assert_eq!(uri.file_name(), Some("report.csv.gz"));
assert_eq!(uri.stem(), Some("report.csv"));
assert_eq!(uri.extension(), Some("gz"));
assert_eq!(uri.extensions().collect::<Vec<_>>(), ["csv", "gz"]);

assert_eq!(uri.mime_type(), MimeType::GZIP);
assert_eq!(uri.media_type().base(), &MimeType::CSV);
assert_eq!(uri.media_type().encodings(), &[MimeType::GZIP]);

uri.set_stem("renamed")?;
assert_eq!(uri.to_string(), "https://example.test/archive/renamed.gz?q=1#part");
uri.set_media_type(MediaType::from_parts(MimeType::JSON, [MimeType::ZSTD])?)?;
assert_eq!(uri.file_name(), Some("renamed.json.zst"));
assert!(uri.clear_extensions());

// A refused name changes nothing.
let unchanged = uri.to_string();
assert!(uri.set_file_name("bad/name").is_err());
assert!(uri.set_mime_type(MimeType::from_str("application/vnd.example")?).is_err());
assert_eq!(uri.to_string(), unchanged);
```

## Walk and join paths

`joinpath` takes URI text and resolves `.`/`..` like `cd`; `parts` is the
resolved path, `parents` climbs to the root. `Url::join_path` takes an OS path
and encodes each component as a name.

```rust
use yggdryl::{UriPath, Url};

let url = Url::from_str("https://example.test/a/b/c?q=1#frag")?;
assert_eq!(url.joinpath("../d")?.to_string(), "https://example.test/a/b/d?q=1#frag");
assert_eq!(url.joinpath("/root")?.path().as_str(), "/root");
assert_eq!(url.parts(), ["a", "b", "c"]);

let parents: Vec<String> = url.parents().map(|p| p.path().as_str().to_owned()).collect();
assert_eq!(parents, ["/a/b", "/a", "/"]);
assert_eq!(url.parent().unwrap().path().as_str(), "/a/b");

let segments: Vec<&str> = url.path_segments().collect();
assert_eq!(segments, ["a", "b", "c"]);

// `..` past an absolute root is clamped; a relative path keeps it.
assert_eq!(UriPath::from_str("/../../a")?.parts(), ["a"]);
assert_eq!(UriPath::from_str("../../a")?.parts(), ["..", "..", "a"]);

// URI text vs an OS path.
let lake = Url::from_str("file:///lake")?;
assert!(lake.joinpath("100%.csv").is_err());
assert_eq!(lake.join_path("100%.csv")?.to_string(), "file:///lake/100%25.csv");
```

## Read and edit query parameters

`parameters(decode)` borrows the query as ordered `key=value` pairs (a key may
repeat); `into_owned` frees the view so `set_parameters` can write it back.

```rust
use yggdryl::Url;

let mut url = Url::from_str("https://example.com/t?symbol=AAPL&venue=XNAS&symbol=MSFT&note=a%26b")?;

let raw = url.parameters(false)?;
assert_eq!(raw.get("symbol"), Some("AAPL"));
assert_eq!(raw.get_all("symbol").collect::<Vec<_>>(), ["AAPL", "MSFT"]);
assert_eq!(raw.get("note"), Some("a%26b"));
assert_eq!(url.parameters(true)?.get("note"), Some("a&b"));

let mut edited = url.parameters(true)?.into_owned();
edited.insert("symbol", "TSLA")?; // replaces the first, drops the rest
edited.remove("venue");
edited.remove("note");
edited.insert("side", "buy & sell")?;
url.set_parameters(&edited)?;
assert_eq!(url.to_string(), "https://example.com/t?symbol=TSLA&side=buy%20%26%20sell");

url.set_query(None)?;
assert_eq!(url.query(false)?, None);
```

## Read credentials and object-store locations

Everything is read off the authority, with no request. The bucket/host rule:
a first part ending `.com`/`.io`, with a port, an IP literal, or `localhost`
is the host.

```rust
use yggdryl::Uri;

let secured = Uri::from_str("https://user:pass:word@example.com/data")?;
assert_eq!(secured.user(), Some("user"));
assert_eq!(secured.password(), Some("pass:word"));
assert_eq!(secured.hostname(), Some("example.com"));

let s3 = Uri::from_str("s3://trades.s3.eu-west-3.amazonaws.com/lake/part.parquet")?;
assert_eq!((s3.bucket(), s3.region(), s3.key()), (Some("trades"), Some("eu-west-3"), Some("lake/part.parquet")));
assert!(s3.is_virtual_hosted());

let minio = Uri::from_str("s3://localhost:9000/trades/lake/")?;
assert_eq!((minio.hostname(), minio.bucket(), minio.key()), (Some("localhost"), Some("trades"), Some("lake/")));

let azure = Uri::from_str("abfss://lake@trades.dfs.core.windows.net/part.parquet")?;
assert_eq!((azure.bucket(), azure.account(), azure.key()), (Some("lake"), Some("trades"), Some("part.parquet")));

let google = Uri::from_str("gs://trades.storage.googleapis.com/lake/x")?;
assert_eq!(google.store_endpoint(), Some("storage.googleapis.com"));
```

## Detect, split and match globs

A URL glob is spelled with `*` in its path; `glob_parts` answers the deepest
fixed location (where a listing should start) and the pattern below it.
Matching follows the `.gitignore` rule.

```rust
use yggdryl::Url;

let pattern = Url::from_str("file:///lake/trades/year=2024/**/*.parquet")?;
assert!(pattern.is_glob());
assert!(pattern.is_recursive_glob());
let (root, rest) = pattern.glob_parts()?;
assert_eq!(root.to_string(), "file:///lake/trades/year=2024");
assert_eq!(rest.as_deref(), Some("**/*.parquet"));

// A plain location is its own root.
let plain = Url::from_str("file:///lake/trades.arrows")?;
assert_eq!(plain.glob_parts()?, (plain.clone(), None));
assert!(Url::is_pattern("part-?.parquet"));

let part = Url::from_str("file:///lake/trades/year=2024/month=01/part-0.parquet")?;
assert!(part.matches_glob("*.parquet")); // no `/`: the name, at any depth
assert!(part.matches_glob("lake/**/part-?.parquet")); // a `/`: anchored at the root
assert!(!part.matches_glob("lake/*.parquet"));
assert!(part.matches_glob_under(&root, "**/*.parquet"));
assert_eq!(part.segments_under(&root), Some(vec!["month=01", "part-0.parquet"]));
```

## Read Hive partitions and select leaves by them

`column=value` directories are the partition columns, in path order; the
`_under` form answers only those below a table root. A handle's
`children_where` keeps the leaves whose path spells every pair: it walks the
whole tree (no call per file beyond the listing, but no pruning); to skip other
partitions, `glob` from the fixed prefix.

```rust
use yggdryl::local::LocalFolder;
use yggdryl::{IOBase, Url};

let part = Url::from_str("file:///lake/year=2024/month=01/part-0.parquet")?;
assert_eq!(
    part.hive_partitions(),
    vec![("year".to_owned(), "2024".to_owned()), ("month".to_owned(), "01".to_owned())]
);
assert_eq!(part.hive_partition("month").as_deref(), Some("01"));
assert!(part.is_hive_partitioned());
assert_eq!(
    part.hive_partitions_under(&Url::from_str("file:///lake/year=2024")?),
    vec![("month".to_owned(), "01".to_owned())]
);
assert_eq!(
    Url::from_str("file:///lake")?.with_hive_partition("year", "2025")?.to_string(),
    "file:///lake/year=2025"
);

let root = std::env::temp_dir().join(format!("ygg-skill-hive-{}", std::process::id()));
for year in ["2024", "2025"] {
    let leaf = root.join(format!("year={year}")).join("month=01");
    std::fs::create_dir_all(&leaf)?;
    std::fs::write(leaf.join("part-0.parquet"), b"PAR1")?;
}
let lake = LocalFolder::new(&root)?;
let selected = lake.children_where(&[("year", "2024")], false)?.collect::<yggdryl::Result<Vec<_>>>()?;
assert_eq!(selected.len(), 1);
assert_eq!(selected[0].partitions()[0], ("year".to_owned(), "2024".to_owned()));
std::fs::remove_dir_all(&root)?;
```

## Resolve a URN to a location

A URN's namespace and its `:`-separated parts are a relative path;
`resolve(&base)` places it under a store, `locator()` under the working
directory.

```rust
use yggdryl::{Uri, Url, Urn};

let urn = Urn::from_str("urn:lake:trades:2026:part.parquet")?;
assert_eq!(urn.namespace(), "lake");
assert_eq!(urn.namespace_specific(), "trades:2026:part.parquet");
assert_eq!(urn.locator_path()?.as_str(), "lake/trades/2026/part.parquet");

let base = Url::from_str("s3://market-data/warehouse/")?;
assert_eq!(urn.resolve(&base)?.to_string(), "s3://market-data/warehouse/lake/trades/2026/part.parquet");

let located = urn.locator()?;
assert!(located.is_local());
assert_eq!(Uri::from_str("urn:lake:trades:2026:part.parquet")?.locator()?, located);

// An empty part would let two names spell one path.
assert!(Urn::from_str("urn:example:a::b")?.locator_path().is_err());
assert!(Urn::from_str("urn:isbn:").is_err());
```

## Read an ARN and locate an S3 object

An ARN is five AWS fields plus a service-owned resource. Only an Amazon S3
bucket ARN (and an S3 Tables ARN) locates a URL; every other service is
refused by name.

```rust
use yggdryl::{Arn, Uri, Url};

let object = Arn::from_str("arn:aws:s3:::market-data/2026/part.parquet")?;
assert_eq!((object.partition(), object.service()), ("aws", "s3"));
assert_eq!((object.region(), object.account()), (None, None));
assert_eq!((object.bucket(), object.key()), (Some("market-data"), Some("2026/part.parquet")));
assert_eq!(object.file_name(), Some("part.parquet"));
assert_eq!(object.locator()?, Url::from_str("s3://market-data/2026/part.parquet")?);

let function = Arn::from_str("arn:aws:lambda:us-east-1:123456789012:function:trades:1")?;
assert_eq!(function.resource_type(), Some("function"));
assert_eq!(function.resource_id(), "trades:1");
assert_eq!(function.resource_separator(), Some(':'));
assert!(function.locator().is_err());

let table = Arn::from_str("arn:aws:s3tables:us-east-1:123456789012:bucket/lake/table/t-a1")?;
assert_eq!((table.bucket(), table.table(), table.key()), (Some("lake"), Some("t-a1"), None));
assert_eq!(table.locator()?.to_string(), "s3tables://lake/t-a1");
assert!(!Uri::from_str("s3tables://lake/t-a1")?.scheme().is_object_store());

assert_eq!(Arn::from_parts("aws", "s3", "", "", "b/k")?.to_string(), "arn:aws:s3:::b/k");
```

## Ask the scheme and the local filesystem

`Scheme` owns scheme facts (ports, which schemes name a container or an object
store). `Url`'s local predicates look at the disk for `file:` and answer
`false` for anything else, with no network call.

```rust
use yggdryl::local::LocalFolder;
use yggdryl::{MimeType, Scheme, Uri, Url};

assert_eq!(Url::from_str("https://example.test:8443")?.default_port(), Some(443));
assert_eq!(Uri::from_str("postgres://host/db")?.default_port(), Some(5432));

let store = Uri::from_str("gcs://trades/lake/x")?;
assert!(store.scheme().is_object_store());
assert!(store.scheme().has_container());
assert_eq!(Uri::from_str("S3://b/k")?.scheme(), &Scheme::S3);

let root = LocalFolder::temporary()?.path()?.join(format!("ygg-skill-local-{}", std::process::id()));
std::fs::create_dir_all(&root)?;
std::fs::write(root.join("ticks.csv"), b"symbol\n")?;
let folder = Url::try_from(root.as_path())?;
assert!(folder.is_local() && folder.is_dir());
assert_eq!(folder.local_mime_type(), MimeType::DIRECTORY);
assert!(folder.join_path("ticks.csv")?.is_file());
assert!(Url::from_str("file:///project/.git")?.is_private());

let remote = Url::from_str("https://example.test/ticks.csv")?;
assert!(!remote.is_local() && !remote.exists());
assert_eq!(remote.local_mime_type(), MimeType::CSV);
std::fs::remove_dir_all(&root)?;
```

## Declare a column of URLs or URNs

`url` and `urn` are the two leaves of the `uri` family: text entering either
is parsed and canonicalized, a value of the other leaf or relative text is
refused, and each stores as `Utf8` under `yggdryl.url` / `yggdryl.urn`.

```rust
use yggdryl::{DataType, Field, Scalar, Uri, UriType, Url, Urn};

let location = Field::new("location", DataType::url(), false);
let name = Field::new("name", DataType::urn(), false);
assert_eq!(DataType::urn().uri_type(), Some(UriType::Urn));

assert_eq!(location.scalar("HTTPS://example.com/a%2fb")?, Scalar::from(Url::from_str("https://example.com/a%2Fb")?));
assert_eq!(location.scalar("/lake/part.txt")?, Scalar::from(Url::from_str("file:///lake/part.txt")?));
assert!(location.scalar("./relative").is_err());
assert!(location.scalar("urn:isbn:0451450523").is_err());

let held = name.scalar("URN:ISBN:0451450523")?;
assert_eq!(held.as_uri(), Some(&Uri::from_str("urn:isbn:0451450523")?));
assert_eq!(held, Scalar::from(Urn::from_str("urn:isbn:0451450523")?));
```

## Gotchas in Rust

- `Uri::query`/`fragment`/`path_text` take `decode: bool` and return
  `Result<Option<Cow<str>>>` (`path_text`: `Result<Cow<str>>`) - a decoding
  view can fail on `%FF`.
- A `Parameters` view borrows its URL: `url.set_parameters(&url.parameters(true)?)`
  does not compile; `into_owned()` first.
- Globs, Hive partitions, `join_path`, `is_local`, `is_private` and the local
  predicates are on `Url`, not `Uri`: narrow with `into_url()?` first.
- `Url::from_str` is strict (no relative text); `Url::from_path` and
  `Url::from_location` root a relative path at the process working directory -
  the answer differs per machine, so never store it.
- `Scheme` compares by value (`&Scheme::S3`); `uri.scheme().as_str()` for text.
- `hive_partitions` answer owned `Vec<(String, String)>`; everything else
  borrows from the value.
