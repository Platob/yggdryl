# URI

`yggdryl::uri` parses every spelling of a resource identifier (URI, URL, URN, or platform path) into one canonical value.

## Pages

| Page | Purpose |
| --- | --- |
| [URI](index.md) | This page: canonical `Uri`, parsing, hash locking, credentials, object stores |
| [Path](path.md) | Segments, compound filenames, media type, `std::path` bridge, navigation |
| [URL and URN](url-urn.md) | The narrowed `Url` and `Urn` forms; what the scheme decides |
| [Query parameters](parameters.md) | The query read and written as its `key=value` pairs |
| [Patterns](patterns.md) | Globs, `.gitignore` matching, Hive partitions |

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `Uri`, the narrowed [`Url` / `Urn`](url-urn.md), `UriPath`, query [`Parameters`](parameters.md), path [patterns](patterns.md) |
| Components | Scheme, authority, path: concrete, empty when absent; query, fragment: optional |
| Validates | [`Scheme`](../types/index.md) and `UriPath` validate on construction |
| Canonical form | Lowercase scheme, uppercase percent escapes, `/` for `\` under `file:`; re-parses to the same value |
| `file:` fallback | Only with no scheme token at all |
| Errors | Bad scheme token, percent escape, space, or bracket: parse error with the failing byte offset |
| Escapes | Stored as written; `path_text`, `query`, and `fragment` take `decode` to answer with the text they stand for. Rust and Python; JavaScript reads the stored form only |
| Decoded text | Text, never structure: `%2F` decodes inside its segment, `%26` inside its pair |
| Hash lock | Python: the first `hash(...)` freezes that wrapper; a later setter raises `TypeError` |
| Stable hash | `stable_hash()` / `stableHash()` compute only; never lock |
| Credentials | Userinfo splits at its first colon; later colons stay in the password |
| Store authority | First component ending `.com` / `.io`, carrying a port, an IP literal, or `localhost` is a hostname, else the container; recognized AWS and Google hosts expose `region` |
| Store container | `bucket()` is the container on all three - a bucket on `s3:`/`gs:`, a container on `az:` - read from the hostname where the host names one, and from Azure's user position where the Hadoop spellings write it |
| Store account | `account()` is Azure's storage account, read from `container@account.host` or from the account's own host; `None` everywhere else |
| Store endpoint | `store_endpoint()` is the host and explicit port to address, with a virtual-hosted container removed; `is_virtual_hosted()` says whether the container was written into the hostname |
| Store key | `key()` is the path below the container as spelled - escapes and trailing slash kept, `""` at the root |

## Use

=== "Rust"

    ```rust
    use yggdryl::Uri;

    let uri = Uri::from_str("HTTPS://example.test/archive/report.tar.gz?q=1#summary")?;

    assert_eq!(uri.to_string(), "https://example.test/archive/report.tar.gz?q=1#summary");
    assert_eq!(uri.scheme().as_str(), "https");
    assert_eq!(uri.authority().as_str(), "example.test");
    assert_eq!(uri.path().as_str(), "/archive/report.tar.gz");
    assert_eq!(uri.query(false)?.as_deref(), Some("q=1"));
    assert_eq!(uri.fragment(false)?.as_deref(), Some("summary"));
    assert_eq!(uri.file_name(), Some("report.tar.gz"));
    ```

=== "Python"

    ```python
    from yggdryl import Uri

    uri = Uri("HTTPS://example.test/archive/report.tar.gz?q=1#summary")

    assert str(uri) == "https://example.test/archive/report.tar.gz?q=1#summary"
    assert uri.scheme == "https"
    assert uri.authority == "example.test"
    assert uri.path == "/archive/report.tar.gz"
    assert uri.query() == "q=1"
    assert uri.fragment() == "summary"
    assert uri.file_name == "report.tar.gz"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Uri } = require('yggdryl')

    const uri = Uri.from('HTTPS://example.test/archive/report.tar.gz?q=1#summary')

    assert.equal(uri.toString(), 'https://example.test/archive/report.tar.gz?q=1#summary')
    assert.equal(uri.scheme, 'https')
    assert.equal(uri.authority, 'example.test')
    assert.equal(uri.path, '/archive/report.tar.gz')
    assert.equal(uri.query, 'q=1')
    assert.equal(uri.fragment, 'summary')
    assert.equal(uri.fileName, 'report.tar.gz')
    ```

## Canonical on arrival

Two spellings of one resource compare equal and hash equal; the parser never guesses.

=== "Rust"

    ```rust
    use yggdryl::Uri;

    // The scheme lowercases and percent escapes uppercase.
    let uri = Uri::from_str("HTTPS://example.test/caf%c3%a9.csv")?;
    assert_eq!(uri.to_string(), "https://example.test/caf%C3%A9.csv");

    // Backslashes in a `file:` hierarchy are separators, not data.
    let windows = Uri::from_str(r"file:///C:\Users\Ada\report.parquet")?;
    assert_eq!(windows.to_string(), "file:///C:/Users/Ada/report.parquet");

    // A string with no usable scheme is a filesystem path.
    assert_eq!(
        Uri::from_str("/var/lib/data.arrow")?.to_string(),
        "file:///var/lib/data.arrow"
    );
    assert_eq!(
        Uri::from_str("data/ticks.csv")?.to_string(),
        "file:data/ticks.csv"
    );

    // A colon after the first separator is data, so this is not a scheme.
    let stamped = Uri::from_str("/data/2026-08-16T00:00:00/part.parquet")?;
    assert_eq!(stamped.scheme().as_str(), "file");

    // Canonical output re-parses to the same value.
    assert_eq!(Uri::from_str(&uri.to_string())?, uri);
    ```

=== "Python"

    ```python
    from yggdryl import Uri

    uri = Uri("HTTPS://example.test/caf%c3%a9.csv")
    assert str(uri) == "https://example.test/caf%C3%A9.csv"

    windows = Uri(r"file:///C:\Users\Ada\report.parquet")
    assert str(windows) == "file:///C:/Users/Ada/report.parquet"

    assert str(Uri("/var/lib/data.arrow")) == "file:///var/lib/data.arrow"
    assert str(Uri("data/ticks.csv")) == "file:data/ticks.csv"

    stamped = Uri("/data/2026-08-16T00:00:00/part.parquet")
    assert stamped.scheme == "file"

    assert Uri(str(uri)) == uri
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Uri } = require('yggdryl')

    const uri = Uri.from('HTTPS://example.test/caf%c3%a9.csv')
    assert.equal(uri.toString(), 'https://example.test/caf%C3%A9.csv')

    const windows = Uri.from('file:///C:\\Users\\Ada\\report.parquet')
    assert.equal(windows.toString(), 'file:///C:/Users/Ada/report.parquet')

    assert.equal(Uri.from('/var/lib/data.arrow').toString(), 'file:///var/lib/data.arrow')
    assert.equal(Uri.from('data/ticks.csv').toString(), 'file:data/ticks.csv')

    const stamped = Uri.from('/data/2026-08-16T00:00:00/part.parquet')
    assert.equal(stamped.scheme, 'file')

    assert.ok(Uri.from(uri.toString()).equals(uri))
    ```

[Python](../extensions/python.md) wrappers stay editable until their first `hash(...)`.

```python
import copy

from yggdryl import Url

url = Url("https://example.test/data.json")
lookup = {url: "cached"}  # locks this wrapper
assert lookup[url] == "cached"

editable = copy.copy(url)
editable.set_extension("parquet")
assert editable != url
```

## Credentials and store locations

Both are read off the authority without a network request. Ten schemes name the
three object stores - `s3`/`s3a`/`s3n`, `gs`/`gcs`, and
`az`/`abfs`/`abfss`/`wasb`/`wasbs` - and one set of accessors reads them all.

=== "Rust"

    ```rust
    use yggdryl::Uri;

    let secured = Uri::from_str("https://user:pass:word@example.com/data")?;
    assert_eq!(secured.user(), Some("user"));
    assert_eq!(secured.password(), Some("pass:word"));
    assert_eq!(secured.hostname(), Some("example.com"));

    let s3 = Uri::from_str("s3://trades.s3.eu-west-3.amazonaws.com/part.parquet")?;
    assert_eq!(s3.bucket(), Some("trades"));
    assert_eq!(s3.region(), Some("eu-west-3"));
    assert_eq!(s3.key(), Some("part.parquet"));

    // A port, an IP literal, or `localhost` is an endpoint - no bucket is
    // named any of those - so a local store reads the way AWS does.
    let local = Uri::from_str("s3://localhost:9000/trades/lake/")?;
    assert_eq!(local.hostname(), Some("localhost"));
    assert_eq!(local.bucket(), Some("trades"));
    assert_eq!(local.key(), Some("lake/"));

    // Google names its bucket the same two ways, and the endpoint answers
    // without the virtual-hosted half.
    let google = Uri::from_str("gs://trades.storage.googleapis.com/lake/part.parquet")?;
    assert_eq!(google.bucket(), Some("trades"));
    assert_eq!(google.store_endpoint(), Some("storage.googleapis.com"));
    assert!(google.is_virtual_hosted());

    // Azure writes the container in the user position, ahead of the account's
    // own host, so one location says both.
    let azure = Uri::from_str("abfss://lake@trades.dfs.core.windows.net/part.parquet")?;
    assert_eq!(azure.bucket(), Some("lake"));
    assert_eq!(azure.account(), Some("trades"));
    assert_eq!(azure.key(), Some("part.parquet"));
    ```

=== "Python"

    ```python
    from yggdryl import Uri

    secured = Uri("https://user:pass:word@example.com/data")
    assert (secured.user, secured.password, secured.hostname) == (
        "user", "pass:word", "example.com"
    )

    s3 = Uri("s3://trades.s3.eu-west-3.amazonaws.com/part.parquet")
    assert (s3.bucket, s3.region, s3.key) == ("trades", "eu-west-3", "part.parquet")

    local = Uri("s3://localhost:9000/trades/lake/")
    assert (local.hostname, local.bucket, local.key) == ("localhost", "trades", "lake/")

    google = Uri("gs://trades.storage.googleapis.com/lake/part.parquet")
    assert (google.bucket, google.store_endpoint) == ("trades", "storage.googleapis.com")
    assert google.is_virtual_hosted()

    azure = Uri("abfss://lake@trades.dfs.core.windows.net/part.parquet")
    assert (azure.bucket, azure.account, azure.key) == ("lake", "trades", "part.parquet")
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Uri } = require('yggdryl')

    const secured = Uri.from('https://user:pass:word@example.com/data')
    assert.deepEqual(
      [secured.user, secured.password, secured.hostname],
      ['user', 'pass:word', 'example.com'],
    )

    const s3 = Uri.from('s3://trades.s3.eu-west-3.amazonaws.com/part.parquet')
    assert.deepEqual([s3.bucket, s3.region, s3.key], ['trades', 'eu-west-3', 'part.parquet'])

    const local = Uri.from('s3://localhost:9000/trades/lake/')
    assert.deepEqual([local.hostname, local.bucket, local.key], ['localhost', 'trades', 'lake/'])

    const google = Uri.from('gs://trades.storage.googleapis.com/lake/part.parquet')
    assert.deepEqual([google.bucket, google.storeEndpoint], ['trades', 'storage.googleapis.com'])
    assert.ok(google.isVirtualHosted())

    const azure = Uri.from('abfss://lake@trades.dfs.core.windows.net/part.parquet')
    assert.deepEqual([azure.bucket, azure.account, azure.key], ['lake', 'trades', 'part.parquet'])
    ```

## Edges

- Invalid scheme token before the colon -> parse error, no `file:` fallback.
- `/data/2026-08-16T00:00:00/part.parquet` -> colon after the first separator is data; scheme `file`.
- Python setter after `hash(uri)` -> `TypeError`; `copy.copy` and pickle give unlocked wrappers.
- Rust -> ownership protects hashed keys; JavaScript -> call `stableHash()` explicitly.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test uri
    cargo test --features "parquet iceberg" -p yggdryl --lib uri::
    cargo test --features "parquet iceberg" -p yggdryl --test uri -- canonical_values core_components credentials s3_locations receive_file_scheme malformed structural_json scheme_less
    cargo bench -p yggdryl --bench uri -- "resource_parse/(uri_canonical|known_scheme|custom_scheme|display_parse_round_trip)"
    cargo bench -p yggdryl --bench uri -- "resource_value/(clone|stable_hash|component_access|credential_access|s3_location_access)"
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/uri
    python/.venv/bin/python -m pytest python/tests/uri -k "components_path_collection or credentials or hash_locks"
    python/.venv/bin/python python/benchmarks/uri.py --iterations 2000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/uri/uri.test.js
    node --test --test-name-pattern="canonical components|credentials and object store|scheme-less|rejects malformed" node/tests/uri/uri.test.js
    ```
