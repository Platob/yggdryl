# URI, URL, and URN

`yggdryl::uri` parses every way of writing a resource identifier - URI, URL, URN, or platform path - into one canonical value.

=== "Rust"

    ```rust
    use yggdryl::Uri;

    let uri = Uri::from_str("HTTPS://example.test/archive/report.tar.gz?q=1#summary")?;

    assert_eq!(uri.to_string(), "https://example.test/archive/report.tar.gz?q=1#summary");
    assert_eq!(uri.scheme().as_str(), "https");
    assert_eq!(uri.authority().as_str(), "example.test");
    assert_eq!(uri.path().as_str(), "/archive/report.tar.gz");
    assert_eq!(uri.query(), Some("q=1"));
    assert_eq!(uri.fragment(), Some("summary"));
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
    assert uri.query == "q=1"
    assert uri.fragment == "summary"
    assert uri.file_name == "report.tar.gz"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Uri } = require('@yggdryl/node')

    const uri = Uri.from('HTTPS://example.test/archive/report.tar.gz?q=1#summary')

    assert.equal(uri.toString(), 'https://example.test/archive/report.tar.gz?q=1#summary')
    assert.equal(uri.scheme, 'https')
    assert.equal(uri.authority, 'example.test')
    assert.equal(uri.path, '/archive/report.tar.gz')
    assert.equal(uri.query, 'q=1')
    assert.equal(uri.fragment, 'summary')
    assert.equal(uri.fileName, 'report.tar.gz')
    ```

Scheme, authority, and path are concrete values, not optionals: a URI with no authority has the
empty authority, and a URI with no path has the empty path. Query and fragment are the two
components that are genuinely absent or present, so they are the only ones that arrive as an
option. The scheme is a [`Scheme`](enums.md) and the path is a `UriPath`, both of which validate
on construction, so an ill-formed component cannot reach a `Uri` at all.

## Canonical on arrival

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
    const { Uri } = require('@yggdryl/node')

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

Parsing accepts a wide input and emits one output, so two spellings of the same resource compare
equal and hash equal. What it will not do is guess: a token before the colon that is not a valid
scheme is an error rather than a silent fall back to `file:`, and a malformed percent escape,
a space, or a bracket in the wrong place is a parse error carrying the byte offset that failed.
The `file:` fallback applies only when there is no scheme token at all.

## Path segments

=== "Rust"

    ```rust
    use yggdryl::Uri;

    let uri = Uri::from_str("https://example.test/archive/2026/report.tar.gz")?;
    let segments: Vec<&str> = uri.path_segments().collect();

    assert_eq!(segments, ["archive", "2026", "report.tar.gz"]);
    assert_eq!(uri.path().segment_len(), 3);
    assert_eq!(uri.path().get_segment(0), Some("archive"));
    assert!(uri.path().contains_segment("2026"));

    // `&Uri` iterates its own segments, borrowing them from the URI.
    let mut visited: Vec<&str> = Vec::new();
    for segment in &uri {
        visited.push(segment);
    }
    assert_eq!(visited, segments);
    ```

=== "Python"

    ```python
    from yggdryl import Uri

    uri = Uri("https://example.test/archive/2026/report.tar.gz")

    assert uri.path_segments == ("archive", "2026", "report.tar.gz")
    assert len(uri) == 3
    assert uri[0] == "archive"
    assert uri[-1] == "report.tar.gz"
    assert "2026" in uri
    assert list(uri) == list(uri.path_segments)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Uri } = require('@yggdryl/node')

    const uri = Uri.from('https://example.test/archive/2026/report.tar.gz')

    assert.deepEqual(uri.pathSegments, ['archive', '2026', 'report.tar.gz'])
    assert.equal(uri.length, 3)
    assert.equal(uri.at(0), 'archive')
    assert.equal(uri.at(-1), 'report.tar.gz')
    assert.deepEqual([...uri], uri.pathSegments)
    ```

A path is a sequence of names, so nothing here asks the caller to split a string. Segments are
the non-empty ones, which means a trailing slash adds no segment and a doubled slash adds no
empty one. In Rust the iterator borrows from the URI and allocates nothing; the bindings project
it onto the protocol each language already has - `len`, indexing with negative indices, `in`, and
iteration in Python, and `length`, `at`, and the spread form in JavaScript.

## Compound filenames

=== "Rust"

    ```rust
    use yggdryl::Uri;

    let mut uri = Uri::from_str("https://example.test/archive/report.tar.gz?q=1#part")?;

    assert_eq!(uri.file_name(), Some("report.tar.gz"));
    assert_eq!(uri.stem(), Some("report.tar"));
    assert_eq!(uri.extension(), Some("gz"));
    assert_eq!(uri.extensions().collect::<Vec<_>>(), ["tar", "gz"]);

    // Renaming touches the filename and nothing else.
    uri.set_stem("renamed")?;
    assert_eq!(uri.to_string(), "https://example.test/archive/renamed.gz?q=1#part");
    uri.set_extensions(["csv", "gz"])?;
    assert_eq!(uri.to_string(), "https://example.test/archive/renamed.csv.gz?q=1#part");
    assert!(uri.remove_extension());
    assert!(uri.clear_extensions());
    assert_eq!(uri.to_string(), "https://example.test/archive/renamed?q=1#part");

    // A rejected name changes nothing.
    let unchanged = uri.to_string();
    assert!(uri.set_file_name("bad/name").is_err());
    assert_eq!(uri.to_string(), unchanged);
    ```

=== "Python"

    ```python
    from yggdryl import Uri

    uri = Uri("https://example.test/archive/report.tar.gz?q=1#part")

    assert uri.file_name == "report.tar.gz"
    assert uri.stem == "report.tar"
    assert uri.extension == "gz"
    assert uri.extensions == ("tar", "gz")

    uri.set_stem("renamed")
    assert str(uri) == "https://example.test/archive/renamed.gz?q=1#part"
    uri.set_extensions(["csv", "gz"])
    assert str(uri) == "https://example.test/archive/renamed.csv.gz?q=1#part"
    assert uri.remove_extension() is True
    assert uri.clear_extensions() is True
    assert str(uri) == "https://example.test/archive/renamed?q=1#part"

    unchanged = str(uri)
    try:
        uri.set_file_name("bad/name")
        raise AssertionError("a separator is not a filename")
    except ValueError:
        pass
    assert str(uri) == unchanged
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Uri } = require('@yggdryl/node')

    const uri = Uri.from('https://example.test/archive/report.tar.gz?q=1#part')

    assert.equal(uri.fileName, 'report.tar.gz')
    assert.equal(uri.stem, 'report.tar')
    assert.equal(uri.extension, 'gz')
    assert.deepEqual(uri.extensions, ['tar', 'gz'])

    uri.setStem('renamed')
    assert.equal(uri.toString(), 'https://example.test/archive/renamed.gz?q=1#part')
    uri.setExtensions(['csv', 'gz'])
    assert.equal(uri.toString(), 'https://example.test/archive/renamed.csv.gz?q=1#part')
    assert.equal(uri.removeExtension(), true)
    assert.equal(uri.clearExtensions(), true)
    assert.equal(uri.toString(), 'https://example.test/archive/renamed?q=1#part')

    const unchanged = uri.toString()
    assert.throws(() => uri.setFileName('bad/name'))
    assert.equal(uri.toString(), unchanged)
    ```

A filename carries a chain of extensions, not one: `extension` is the last of them and
`extensions` is all of them left to right, which is what makes `report.tar.gz` describable. The
stem is the filename minus its final extension, so a dotfile keeps its leading dot - `.env.local`
has stem `.env` and extension `local`, and `.env` has no extension at all.

Every mutator is atomic. It builds the candidate, validates it, and only then replaces the value,
so a rejected name leaves the URI exactly as it was rather than half-edited. A filename must be a
single segment of valid URI bytes, which is why `bad/name`, `bad?name`, and a bare `%` are all
refused: encoding is the caller's decision, and the setters will not silently make it.

## The media type is in the name

=== "Rust"

    ```rust
    use yggdryl::{MediaType, MimeType, Uri};

    let mut uri = Uri::from_str("https://example.test/report.csv.gz.zst?q=1#part")?;

    // The final suffix is the MIME type; the whole chain is the media type.
    assert_eq!(uri.mime_type(), MimeType::ZSTD);
    let media = uri.media_type();
    assert_eq!(media.base(), &MimeType::CSV);
    assert_eq!(media.encodings(), &[MimeType::GZIP, MimeType::ZSTD]);

    // Setting a MIME type rewrites the final suffix.
    uri.set_mime_type(MimeType::JSON)?;
    assert_eq!(uri.to_string(), "https://example.test/report.csv.gz.json?q=1#part");

    // Setting a media type rewrites the whole chain.
    let encoded = MediaType::from_parts(MimeType::CSV, [MimeType::GZIP, MimeType::ZSTD])?;
    uri.set_media_type(encoded)?;
    assert_eq!(uri.to_string(), "https://example.test/report.csv.gz.zst?q=1#part");

    // A MIME type with no preferred extension cannot name a file.
    let unchanged = uri.to_string();
    let custom = MimeType::from_str("application/vnd.example")?;
    assert!(uri.set_mime_type(custom).is_err());
    assert_eq!(uri.to_string(), unchanged);
    ```

=== "Python"

    ```python
    from yggdryl import MediaType, MimeType, Uri

    uri = Uri("https://example.test/report.csv.gz.zst?q=1#part")

    assert uri.mime_type == MimeType("application/zstd")
    assert uri.media_type.base == MimeType("text/csv")
    assert uri.media_type.encodings == (
        MimeType("application/gzip"),
        MimeType("application/zstd"),
    )

    uri.set_mime_type("application/json")
    assert str(uri) == "https://example.test/report.csv.gz.json?q=1#part"

    uri.set_media_type(
        MediaType.from_parts(
            MimeType("text/csv"),
            [MimeType("application/gzip"), MimeType("application/zstd")],
        )
    )
    assert str(uri) == "https://example.test/report.csv.gz.zst?q=1#part"

    unchanged = str(uri)
    try:
        uri.set_mime_type("application/vnd.example")
        raise AssertionError("no preferred filename extension")
    except ValueError:
        pass
    assert str(uri) == unchanged
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { MediaType, MimeType, Uri } = require('@yggdryl/node')

    const uri = Uri.from('https://example.test/report.csv.gz.zst?q=1#part')

    assert.ok(uri.mimeType.equals(MimeType.from('application/zstd')))
    assert.ok(uri.mediaType.base.equals(MimeType.from('text/csv')))
    assert.deepEqual(
      uri.mediaType.encodings.map((value) => value.toString()),
      ['application/gzip', 'application/zstd'],
    )

    uri.setMimeType('application/json')
    assert.equal(uri.toString(), 'https://example.test/report.csv.gz.json?q=1#part')

    uri.setMediaType(MediaType.fromParts('text/csv', ['application/gzip', 'application/zstd']))
    assert.equal(uri.toString(), 'https://example.test/report.csv.gz.zst?q=1#part')

    const unchanged = uri.toString()
    assert.throws(() => uri.setMimeType('application/vnd.example'), /preferred filename extension/)
    assert.equal(uri.toString(), unchanged)
    ```

`report.csv.gz.zst` is CSV that was gzipped and then zstd-compressed, and the extension chain says
so in order. [`MediaType`](enums.md) splits that into a base and its transparent encodings, which
is exactly what an HTTP `Content-Type` plus `Content-Encoding` pair carries, so a name and a set
of headers describe the same thing. A name with no recognised suffix reads as
`application/octet-stream` rather than failing.

The setters run the inference backwards, and that direction can fail: `application/vnd.example`
has no registered filename extension, so there is nothing to write into the name. Structured
suffixes still work - `application/vnd.example+json` writes `json` - because the `+json` names a
concrete syntax.

## URL and URN

=== "Rust"

    ```rust
    use yggdryl::{Uri, Url, Urn};

    let uri = Uri::from_str("https://example.test/a/data.json?raw=true")?;
    let url = Url::from_uri(uri.clone())?;
    assert_eq!(url.authority().as_str(), "example.test");
    assert_eq!(Uri::from(&url), uri);

    let urn = Urn::from_str("URN:ISBN:9780131103627")?;
    assert_eq!(urn.to_string(), "urn:isbn:9780131103627");
    assert_eq!(urn.namespace(), "isbn");
    assert_eq!(urn.namespace_specific(), "9780131103627");
    assert_eq!(urn.authority().as_str(), "");

    // Each refuses what it is not.
    assert!(urn.to_uri().to_url().is_err());
    assert!(Urn::from_uri(uri).is_err());
    assert!(Url::from_str("mailto:user@example.test").is_err());
    assert!(Url::from_str("https:///missing-authority").is_err());
    ```

=== "Python"

    ```python
    from yggdryl import Uri, Url, Urn

    uri = Uri("https://example.test/a/data.json?raw=true")
    url = Url(uri)
    assert url.authority == "example.test"
    assert Uri(url) == uri

    urn = Urn("URN:ISBN:9780131103627")
    assert str(urn) == "urn:isbn:9780131103627"
    assert urn.namespace == "isbn"
    assert urn.namespace_specific == "9780131103627"
    assert urn.authority == ""

    for rejected in (
        lambda: urn.to_uri().to_url(),
        lambda: Urn(uri),
        lambda: Url("mailto:user@example.test"),
    ):
        try:
            rejected()
            raise AssertionError("expected a rejection")
        except ValueError:
            pass
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Uri, Url, Urn } = require('@yggdryl/node')

    const uri = Uri.from('https://example.test/a/data.json?raw=true')
    const url = Url.from(uri)
    assert.equal(url.authority, 'example.test')
    assert.ok(Uri.from(url).equals(uri))

    const urn = Urn.from('URN:ISBN:9780131103627')
    assert.equal(urn.toString(), 'urn:isbn:9780131103627')
    assert.equal(urn.namespace, 'isbn')
    assert.equal(urn.namespaceSpecific, '9780131103627')
    assert.equal(urn.authority, '')

    assert.throws(() => urn.toUri().toUrl())
    assert.throws(() => Urn.fromUri(uri))
    assert.throws(() => Url.fromString('mailto:user@example.test'))
    ```

`Url` and `Urn` are the same canonical `Uri` under a narrower constraint, which is why conversion
either way is free of re-parsing. A URL requires hierarchical authority syntax and, unless it is
`file:`, a non-empty host - so `mailto:` and a URN are not URLs. A URN requires the `urn` scheme,
no authority, and a namespace plus a non-empty namespace-specific string; the namespace
lowercases while the namespace-specific string is left exactly as written, because case is
significant there.

On a URN the filename accessors read the namespace-specific string rather than a slash path, so
`urn:example:reports/data.csv` has file name `data.csv` and editing it leaves the namespace
alone.

## Platform paths

=== "Rust"

    ```rust
    use std::path::PathBuf;
    use yggdryl::{Uri, Url};

    // Drive and UNC detection is textual, so it behaves the same on every host.
    let uri = Uri::try_from(PathBuf::from(r"C:\Users\Ada Lovelace\report.parquet"))?;
    assert_eq!(uri.to_string(), "file:///C:/Users/Ada%20Lovelace/report.parquet");
    assert_eq!(uri.authority().as_str(), "");
    assert_eq!(uri.file_name(), Some("report.parquet"));

    // And back, with the escapes decoded.
    let path = PathBuf::try_from(&uri)?;
    assert_eq!(path, PathBuf::from("C:/Users/Ada Lovelace/report.parquet"));
    assert_eq!(Uri::try_from(path)?, uri);

    // A UNC share puts the server in the authority.
    let unc = Uri::from_path(r"\\server\share\prices\ticks.csv")?;
    assert_eq!(unc.to_string(), "file://server/share/prices/ticks.csv");
    assert_eq!(unc.authority().as_str(), "server");
    assert_eq!(unc.to_path()?, PathBuf::from("//server/share/prices/ticks.csv"));

    // Only a `file:` identifier has a path at all.
    assert!(Url::from_str("https://example.test/data.csv")?.to_path().is_err());
    ```

=== "Python"

    ```python
    import os
    from pathlib import PureWindowsPath

    from yggdryl import Uri, Url

    uri = Uri.from_path(PureWindowsPath(r"C:\Users\Ada Lovelace\report.parquet"))
    assert str(uri) == "file:///C:/Users/Ada%20Lovelace/report.parquet"
    assert uri.authority == ""
    assert uri.file_name == "report.parquet"

    # `__fspath__` is `to_path`, so a URI goes straight into `open` or `pathlib`.
    assert uri.to_path() == "C:/Users/Ada Lovelace/report.parquet"
    assert os.fspath(uri) == uri.to_path()
    assert Uri.from_path(uri.to_path()) == uri

    unc = Uri.from_path(r"\\server\share\prices\ticks.csv")
    assert str(unc) == "file://server/share/prices/ticks.csv"
    assert unc.authority == "server"
    assert unc.to_path() == "//server/share/prices/ticks.csv"

    try:
        Url("https://example.test/data.csv").to_path()
        raise AssertionError("a network URL has no path")
    except ValueError:
        pass
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Uri, Url } = require('@yggdryl/node')

    const uri = Uri.fromPath('C:\\Users\\Ada Lovelace\\report.parquet')
    assert.equal(uri.toString(), 'file:///C:/Users/Ada%20Lovelace/report.parquet')
    assert.equal(uri.authority, '')
    assert.equal(uri.fileName, 'report.parquet')

    assert.equal(uri.toPath(), 'C:/Users/Ada Lovelace/report.parquet')
    assert.ok(Uri.fromPath(uri.toPath()).equals(uri))

    const unc = Uri.fromPath('\\\\server\\share\\prices\\ticks.csv')
    assert.equal(unc.toString(), 'file://server/share/prices/ticks.csv')
    assert.equal(unc.authority, 'server')
    assert.equal(unc.toPath(), '//server/share/prices/ticks.csv')

    assert.throws(() => Url.fromString('https://example.test/data.csv').toPath())
    ```

`from_path` and `to_path` are the whole bridge to `std::path`, and `TryFrom` is spelled in both
directions - `Uri::try_from(path)` and `PathBuf::try_from(&uri)` - for the places where a
conversion is inferred rather than named. A Windows drive becomes the first path segment under an
empty authority, a UNC server becomes the authority, and the encoding round-trips: a space is
`%20` in the URI and a space again in the path.

The projection back to a path refuses anything it cannot represent faithfully. A query or a
fragment has no place in a path, `%2F` and `%5C` would smuggle a separator into a segment, and a
percent escape that would manufacture a `C:` drive designator is rejected outright, so a decoded
path can never address something other than what the URI named. Python exposes the same
conversion through `__fspath__`, which is why a `Uri` can be handed directly to `open`. That
canonical `file:` URL is also what [`local`](local.md) stores as the entire state of a handle.

## Walking the path

=== "Rust"

    ```rust
    use yggdryl::{UriPath, Url};

    let url = Url::from_str("https://example.test/a/b/c?q=1#frag")?;

    // `joinpath` composes the way a shell `cd` does; everything else survives.
    let joined = url.joinpath("../d")?;
    assert_eq!(joined.to_string(), "https://example.test/a/b/d?q=1#frag");

    // `parts` is the sequence of names the path actually addresses.
    assert_eq!(url.parts(), ["a", "b", "c"]);

    // `parents` climbs to the root and never yields the value itself.
    let parents: Vec<String> = url
        .parents()
        .map(|value| value.path().as_str().to_owned())
        .collect();
    assert_eq!(parents, ["/a/b", "/a", "/"]);
    assert_eq!(url.parent().unwrap().path().as_str(), "/a/b");

    // `..` past an absolute root is clamped, matching filesystem semantics.
    assert_eq!(UriPath::from_str("/../../a")?.parts(), ["a"]);
    // A relative path has no root to clamp against, so `..` is kept.
    assert_eq!(UriPath::from_str("../../a")?.parts(), ["..", "..", "a"]);
    ```

=== "Python"

    ```python
    from yggdryl import Url

    url = Url("https://example.test/a/b/c?q=1#frag")

    # `joinpath` and `/` compose the way they do on a `PurePath`.
    assert str(url / "d") == "https://example.test/a/b/c/d?q=1#frag"
    assert str(url.joinpath("d", "e")) == "https://example.test/a/b/c/d/e?q=1#frag"

    # `parts` is the sequence of names the path actually addresses.
    assert url.parts == ("a", "b", "c")

    # `parents` climbs to the root and never yields the value itself.
    assert [parent.path for parent in url.parents] == ["/a/b", "/a", "/"]
    assert url.parent.path == "/a/b"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Url } = require('@yggdryl/node')

    const url = Url.from('https://example.test/a/b/c?q=1#frag')

    // `joinpath` is variadic the way `path.join` is; everything else survives.
    assert.equal(url.joinpath('d').toString(), 'https://example.test/a/b/c/d?q=1#frag')
    assert.equal(url.joinpath('d', 'e').toString(), 'https://example.test/a/b/c/d/e?q=1#frag')

    // `parts` is the sequence of names the path actually addresses.
    assert.deepEqual(url.parts, ['a', 'b', 'c'])

    // `parents` climbs to the root and never yields the value itself.
    assert.deepEqual(url.parents.map((parent) => parent.path), ['/a/b', '/a', '/'])
    assert.equal(url.parent.path, '/a/b')
    ```

Navigation is defined on `UriPath` and lifted onto `Uri` and `Url` unchanged, so joining or
climbing preserves the scheme, authority, query, and fragment and only moves the path. An
absolute argument to `joinpath` replaces the path rather than extending it, again like `cd`.

`segments` and `parts` are not the same list: `segments` is the literal text and `parts` is what
the path resolves to, with `.` dropped and `..` applied. `parents` yields whole values of the
source type - `Url::parents` yields `Url`, not a path - and skips any step that would not be
valid for that type. `Urn` has no navigation, because a namespace-specific string is not a
hierarchy to walk.

## What the scheme decides

!!! note "Rust only"
    `default_port`, `is_local`, `join_path`, and `local_mime_type` are Rust-only.
    The `exists`, `is_dir`, and `is_file` predicates below are in both bindings,
    under those names.

=== "Rust"

    ```rust
    use yggdryl::{MimeType, Uri, Url};

    // The port belongs to the scheme, not to the authority text.
    assert_eq!(Url::from_str("https://example.test")?.default_port(), Some(443));
    assert_eq!(Uri::from_str("postgres://host/db")?.default_port(), Some(5432));
    assert_eq!(Uri::from_str("s3://bucket/key")?.default_port(), None);

    let root = std::env::temp_dir().join(format!("yggdryl-doc-uri-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root)?;
    std::fs::write(root.join("ticks.csv"), b"symbol\n")?;

    // `join_path` is `Path::join` for URLs: one segment per component.
    let folder = Url::try_from(root.as_path())?;
    assert!(folder.is_local());
    assert!(folder.is_dir());
    assert_eq!(folder.local_mime_type(), MimeType::DIRECTORY);

    let file = folder.join_path("ticks.csv")?;
    assert!(file.exists());
    assert!(file.is_file());
    assert_eq!(file.local_mime_type(), MimeType::CSV);

    // An existing file the name cannot identify is still a file.
    std::fs::write(root.join("MANIFEST"), b"")?;
    assert_eq!(folder.join_path("MANIFEST")?.local_mime_type(), MimeType::FILE);

    // A remote URL answers without a round trip: it is simply not local.
    let remote = Url::from_str("https://example.test/ticks.csv")?;
    assert!(!remote.is_local());
    assert!(!remote.exists());
    assert_eq!(remote.local_mime_type(), MimeType::CSV);

    let _ = std::fs::remove_dir_all(&root);
    ```

`default_port` reports the port a client would dial when the authority omits one; it does not
read a port that was written into the authority. Object stores and metadata namespaces have none,
which is a fact about the scheme rather than a missing value.

`exists`, `is_dir`, and `is_file` answer only for a local URL and answer `false` for every other
scheme, because reporting anything else would require a network call an accessor has no business
making. `local_mime_type` follows the same rule from the other side: an existing directory is
`MimeType::DIRECTORY`, an existing local file is identified from its name and falls back to
`MimeType::FILE`, and a remote URL falls back to the [`MimeType`](enums.md) its name already
implies.

## Patterns and partitions

Two conventions turn a path into a query, and both are read off the URL itself: a glob says *which*
locations a caller means, and a Hive partition says *what a location holds*.

=== "Rust"

    ```rust
    use yggdryl::Url;

    let pattern = Url::from_str("file:///lake/trades/year=2024/**/*.parquet")?;
    assert!(pattern.is_glob());
    assert!(pattern.is_recursive_glob());

    // A glob decomposes into the deepest fixed location and the rest.
    let (root, rest) = pattern.glob_parts()?;
    assert_eq!(root.to_string(), "file:///lake/trades/year=2024");
    assert_eq!(rest.as_deref(), Some("**/*.parquet"));

    // Matching follows the `.gitignore` rule.
    let part = Url::from_str("file:///lake/trades/year=2024/month=01/part-0.parquet")?;
    assert!(part.matches_glob("*.parquet"));
    assert!(part.matches_glob("lake/**/part-?.parquet"));
    assert!(!part.matches_glob("lake/*.parquet"));
    assert!(part.matches_glob_under(&root, "**/*.parquet"));

    // The directory names are the partition columns.
    assert_eq!(part.hive_partition("month").as_deref(), Some("01"));
    assert_eq!(
        part.hive_partitions(),
        vec![("year".to_owned(), "2024".to_owned()), ("month".to_owned(), "01".to_owned())]
    );
    ```

=== "Python"

    ```python
    from yggdryl import Url

    pattern = Url("file:///lake/trades/year=2024/**/*.parquet")
    assert pattern.is_glob()

    part = Url("file:///lake/trades/year=2024/month=01/part-0.parquet")
    assert part.match("*.parquet")
    assert part.match("lake/**/part-?.parquet")
    assert not part.match("lake/*.parquet")

    assert part.partition("month") == "01"
    assert part.partitions == (("year", "2024"), ("month", "01"))
    assert part.relative_to(Url("file:///lake/trades")) == "year=2024/month=01/part-0.parquet"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Url } = require('@yggdryl/node')

    const pattern = Url.from('file:///lake/trades/year=2024/**/*.parquet')
    assert.ok(pattern.isGlob())

    // Matching follows the `.gitignore` rule.
    const part = Url.from('file:///lake/trades/year=2024/month=01/part-0.parquet')
    assert.ok(part.match('*.parquet'))
    assert.ok(part.match('lake/**/part-?.parquet'))
    assert.ok(!part.match('lake/*.parquet'))

    // The directory names are the partition columns.
    assert.equal(part.partition('month'), '01')
    assert.deepEqual(part.partitions, [
      { column: 'year', value: '2024' },
      { column: 'month', value: '01' },
    ])
    assert.equal(
      part.relativeTo('file:///lake/trades'),
      'year=2024/month=01/part-0.parquet',
    )
    ```

`*` and `?` stay inside one name, `[a-z]` and `[!a-z]` pick one character, and `**` spans any
number of levels. A pattern with no separator matches the *name* at any depth and one with a
separator is anchored at the path root, which is the rule `.gitignore` uses. Only `*` survives URL
parsing - `?` opens the query and `[` is reserved for an IPv6 host - so a location that *is* a glob
spells it with stars, while the full syntax is available to the pattern text passed to
`matches_glob`.

A Hive layout names one directory per partition column, `column=value`. `hive_partitions` reads
them back in path order, which is what lets a partitioned read restore the columns the directory
names replaced; [io.md](io.md) does exactly that.

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](../notebooks/core_uri-rust.ipynb){ download },
[Python](../notebooks/core_uri-python.ipynb){ download },
[JavaScript](../notebooks/core_uri-javascript.ipynb){ download }.

<!-- /notebooks -->
