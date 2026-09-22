# URI

`yggdryl::uri` parses every spelling of a resource identifier (URI, URL, URN, or platform path) into one canonical value.

## Pages

| Page | Purpose |
| --- | --- |
| [URI](index.md) | This page: canonical `Uri`, parsing, hash locking, credentials, object stores |
| [Path](path.md) | Segments, compound filenames, media type, `std::path` bridge, navigation |
| [URL and URN](url-urn.md) | The narrowed `Url` and `Urn` forms, and where a name resolves to; what the scheme decides |
| [ARN](arn.md) | The narrowed `Arn` form: AWS's five fields, and the location an Amazon S3 name addresses |
| [Query parameters](parameters.md) | The query read and written as its `key=value` pairs |
| [Patterns](patterns.md) | Globs, `.gitignore` matching, Hive partitions |

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `Uri`, the narrowed [`Url` / `Urn`](url-urn.md) and [`Arn`](arn.md), the [`uri` family](#as-a-column) - the `url` and `urn` datatypes a column declares over them, `UriPath`, query [`Parameters`](parameters.md), path [patterns](patterns.md) |
| Narrowings | Three, and the scheme decides which: a location, a name, and the name AWS writes for one of its resources. Python's `Url`, `Urn` and `Arn` are subclasses of `Uri`, so `Uri(value)` answers the one that value is; Rust and JavaScript answer a `Uri` and narrow through `into_url` / `into_urn` / `into_arn` |
| Locating | `locator()` answers the [`Url`](url-urn.md) an identifier names: a location locates itself, a name resolves to the path it spells, an Amazon S3 [ARN](arn.md) maps to its `s3:` URL. It is what lets any identifier be handed to a reader as the thing to open |
| Components | Scheme, authority, path: concrete, empty when absent; query, fragment: optional |
| Validates | [`Scheme`](../types/index.md) and `UriPath` validate on construction |
| Canonical form | Lowercase scheme, uppercase percent escapes, `/` for `\` under `file:`, the authority marker on every absolute `file:` path; re-parses to the same value |
| `file:` fallback | Only with no scheme token at all |
| Errors | Bad scheme token, percent escape, space, or bracket: parse error with the failing byte offset |
| Escapes | Stored as written; `path_text`, `query`, and `fragment` take `decode` to answer with the text they stand for. Rust and Python; JavaScript reads the stored form only |
| Decoded text | Text, never structure: `%2F` decodes inside its segment, `%26` inside its pair |
| Hash lock | Python: the first `hash(...)` freezes that wrapper; a later setter raises `TypeError` |
| Stable hash | `stable_hash()` / `stableHash()` compute only; never lock |
| Credentials | Userinfo splits at its first colon; later colons stay in the password |
| Store authority | First component ending `.com` / `.io`, carrying a port, an IP literal, or `localhost` is a hostname, else the container; recognized AWS and Google hosts expose `region` |
| Store container | `bucket()` is the container on all of them - a bucket on `s3:`/`gs:`, a container on `az:`, a table bucket on `s3tables:` - read from the hostname where the host names one, and from Azure's user position where the Hadoop spellings write it. `has_container()` is the scheme predicate behind it; `is_object_store()` stays the narrower question of which byte backend opens a location |
| Store account | `account()` is Azure's storage account, read from `container@account.host` or from the account's own host; `None` everywhere else |
| Store endpoint | `store_endpoint()` is the host and explicit port to address, with a virtual-hosted container removed; `is_virtual_hosted()` says whether the container was written into the hostname |
| Store key | `key()` is the path below the container as spelled - escapes and trailing slash kept, `""` at the root |
| Store tables | `s3tables:` names an [Amazon S3 Tables](arn.md) table bucket and a table below it. It is a container, so `bucket()` and `key()` read it; it is not an object store, so no byte backend opens it |

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

Python wrappers stay editable until their first `hash(...)`, which locks that wrapper against a later setter; `copy.copy` answers an unlocked, editable copy that no longer equals the original once edited.

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

## As a column

A column of locations declares `url`; a column of names declares `urn`. The two are the leaves of one `uri` family, and each holds the crate's own value - [`Url` or `Urn`](url-urn.md) - the same parsed value a handle addresses itself by, so a value read out of a table is a value a handle can be opened from, not prose that happens to look like one. Text entering either column is parsed and canonicalized on the way in, so a column holds one spelling per identifier and nothing that is not one. There is no wider leaf: an identifier a column holds is a location or a name, and a scalar of either leaf answers the `Uri` both narrow through `as_uri`.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Scalar, Uri, UriType, Url, Urn};

    let location = Field::new("location", DataType::url(), false);
    let name = Field::new("name", DataType::urn(), false);
    assert_eq!(DataType::urn(), DataType::Uri(UriType::Urn));
    assert_eq!(DataType::urn().uri_type(), Some(UriType::Urn));

    // One spelling per identifier, whichever spelling the text carries.
    assert_eq!(
        location.scalar("HTTPS://example.com/a%2fb")?,
        Scalar::from(Url::from_str("https://example.com/a%2Fb")?)
    );
    assert_eq!(
        name.scalar("URN:ISBN:0451450523")?,
        Scalar::from(Urn::from_str("urn:isbn:0451450523")?)
    );
    // A name is not a location and a location is not a name: each leaf
    // refuses the other's identifier.
    assert!(location.scalar("urn:isbn:0451450523").is_err());
    assert!(name.scalar("https://example.com/a").is_err());
    // A bare platform path is a `file:` URL, which is what a local handle is.
    assert_eq!(
        location.scalar("/lake/part.txt")?,
        Scalar::from(Url::from_str("file:///lake/part.txt")?)
    );
    // Either leaf's scalar answers the identifier both narrow.
    let held = name.scalar("urn:isbn:0451450523")?;
    assert_eq!(held.as_uri(), Some(&Uri::from_str("urn:isbn:0451450523")?));
    // Relative text names no location.
    assert!(location.scalar("./relative").is_err());
    ```

=== "Python"

    ```python
    import yggdryl
    from yggdryl import DataType, Scalar
    from yggdryl import json

    location = yggdryl.url("location", nullable=False)
    name = yggdryl.urn("name", nullable=False)
    assert DataType("urn").kind == "text"
    located = json.loads('"HTTPS://example.com/a"', field=location, cls=Scalar)
    assert located.as_py() == "https://example.com/a"
    named = json.loads('"URN:ISBN:0451450523"', field=name, cls=Scalar)
    assert named.as_py() == "urn:isbn:0451450523"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { DataType, fields, json } = require('yggdryl')

    const location = fields.url('location', { nullable: false })
    const name = fields.urn('name', { nullable: false })
    assert.equal(new DataType('urn').kind, 'text')
    const located = json.loads('"HTTPS://example.com/a"', { field: location, scalar: true })
    assert.equal(located.asJs(), 'https://example.com/a')
    const named = json.loads('"URN:ISBN:0451450523"', { field: name, scalar: true })
    assert.equal(named.asJs(), 'urn:isbn:0451450523')
    ```

| rule | `url` (`0x64`) | `urn` (`0x65`) |
| --- | --- | --- |
| Kind | `text`; `UriField` over `UriType::Url`, `yggdryl.url`, `fields.url` | `text`; `UriField` over `UriType::Urn`, `yggdryl.urn`, `fields.urn` |
| Value | `Url` behind one shared pointer, so a row clone moves a reference count rather than an identifier | `Urn`, the same way |
| Admits | a location: hierarchical, with a host unless `file:`; never a name | a name: `urn:<namespace>:<specific>`, its namespace folded to lower case; never a location |
| Storage | `Utf8` holding the canonical text, extension name `yggdryl.url` | `Utf8`, extension name `yggdryl.urn` |
| Ordering | the canonical text's, which is Arrow's own string order | the same |
| Default | `file:///`, the shortest location the validator accepts, because an identifier has no zero | `urn:nil:nil`, the shortest name it accepts |
| Merging | only with itself: merging into text, or into the other leaf, would drop the rule that makes it a location | only with itself |
| Refusal | a text cell that reads as no location is refused naming the field and the row; an empty cell is null | the same, naming `urn` |

## Edges

- Invalid scheme token before the colon -> parse error, no `file:` fallback.
- `/data/2026-08-16T00:00:00/part.parquet` -> colon after the first separator is data; scheme `file`.
- `file:/data` -> `file:///data`; one absolute local path has one spelling.
- `a://host/p` and `a:/b?q=1#f` -> the one-letter scheme `a`; a backslash, or a slash with no `//`, `?` or `#` after it, keeps the drive reading (`C:/x`, `C:\x`).
- Python setter after `hash(uri)` -> `TypeError`; `copy.copy` and pickle give unlocked wrappers.
- Rust -> ownership protects hashed keys; JavaScript -> call `stableHash()` explicitly.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test uri
    cargo test --features "parquet iceberg" -p yggdryl --test fs -- uri
    cargo test --features "parquet iceberg" -p yggdryl --test uri -- canonical_values core_components credentials s3_locations receive_file_scheme malformed structural_json scheme_less
    cargo bench -p yggdryl --bench uri -- "resource_parse/(uri_canonical|known_scheme|custom_scheme|display_parse_round_trip)"
    cargo bench -p yggdryl --bench uri -- "resource_value/(clone|stable_hash|component_access|credential_access|s3_location_access)"
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_uri.py
    python/.venv/bin/python -m pytest python/tests/test_uri.py -k "components_path_collection or credentials or hash_locks"
    python/.venv/bin/python python/benchmarks/uri.py --iterations 2000
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/uri.test.js
    node --test --test-name-pattern="canonical components|credentials and object store|scheme-less|rejects malformed" node/tests/uri.test.js
    ```
