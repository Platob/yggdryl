# Path

This page owns the path as a sequence of names: segments, filenames, media type, `std::path`, and navigation.

## Contract

| Owns | Rule |
| --- | --- |
| Segments | non-empty names; a trailing or doubled slash adds none |
| Filename | one segment of valid URI bytes; the caller encodes |
| `extension`, `extensions` | last suffix; all suffixes, left to right |
| `stem` | filename minus last extension; dotfiles keep the dot |
| Mutators | atomic; a rejected name changes nothing |
| `mime_type`, `media_type` | last suffix; whole chain as base plus encodings |
| `from_path`, `into_path` | drive: first segment, empty authority; UNC server: authority |
| Navigation | `UriPath`, lifted onto `Uri` and `Url`; `Urn` has none |

## Use

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
    const { Uri } = require('yggdryl')

    const uri = Uri.from('https://example.test/archive/2026/report.tar.gz')

    assert.deepEqual(uri.pathSegments, ['archive', '2026', 'report.tar.gz'])
    assert.equal(uri.length, 3)
    assert.equal(uri.at(0), 'archive')
    assert.equal(uri.at(-1), 'report.tar.gz')
    assert.deepEqual([...uri], uri.pathSegments)
    ```

## Compound filenames

`.env.local` has stem `.env` and extension `local`; `.env` has none.

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
    const { Uri } = require('yggdryl')

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

## Media type in the name

[`MediaType`](../types/scalar.md) splits the chain into base plus encodings, as `Content-Type` plus `Content-Encoding` do.

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
    const { MediaType, MimeType, Uri } = require('yggdryl')

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

## Platform paths

[`local`](../holder/backends/local.md) stores this canonical `file:` URL as a handle's whole state.

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
    assert_eq!(unc.into_path()?, PathBuf::from("//server/share/prices/ticks.csv"));

    // Only a `file:` identifier has a path at all.
    assert!(Url::from_str("https://example.test/data.csv")?.into_path().is_err());
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

    # `__fspath__` is `into_path`, so a URI goes straight into `open` or `pathlib`.
    assert uri.into_path() == "C:/Users/Ada Lovelace/report.parquet"
    assert os.fspath(uri) == uri.into_path()
    assert Uri.from_path(uri.into_path()) == uri

    unc = Uri.from_path(r"\\server\share\prices\ticks.csv")
    assert str(unc) == "file://server/share/prices/ticks.csv"
    assert unc.authority == "server"
    assert unc.into_path() == "//server/share/prices/ticks.csv"

    try:
        Url("https://example.test/data.csv").into_path()
        raise AssertionError("a network URL has no path")
    except ValueError:
        pass
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Uri, Url } = require('yggdryl')

    const uri = Uri.fromPath('C:\\Users\\Ada Lovelace\\report.parquet')
    assert.equal(uri.toString(), 'file:///C:/Users/Ada%20Lovelace/report.parquet')
    assert.equal(uri.authority, '')
    assert.equal(uri.fileName, 'report.parquet')

    assert.equal(uri.intoPath(), 'C:/Users/Ada Lovelace/report.parquet')
    assert.ok(Uri.fromPath(uri.intoPath()).equals(uri))

    const unc = Uri.fromPath('\\\\server\\share\\prices\\ticks.csv')
    assert.equal(unc.toString(), 'file://server/share/prices/ticks.csv')
    assert.equal(unc.authority, 'server')
    assert.equal(unc.intoPath(), '//server/share/prices/ticks.csv')

    assert.throws(() => Url.fromString('https://example.test/data.csv').intoPath())
    ```

## Walking the path

`segments` is the literal text; `parts` resolves it, dropping `.` and applying `..`.

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
    from yggdryl import Uri, Url

    url = Url("https://example.test/a/b/c?q=1#frag")
    uri = Uri(url)

    # Both the generic URI and the narrower URL compose like a `PurePath`.
    assert str(uri.joinpath("../d")) == "https://example.test/a/b/d?q=1#frag"
    assert str(uri / "/root") == "https://example.test/root?q=1#frag"
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
    const { Url } = require('yggdryl')

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

## Edges

- `bad/name`, `bad?name`, or a bare `%` -> filename refused, URI unchanged.
- Unknown suffix -> `application/octet-stream`, not an error.
- `application/vnd.example` -> `set_mime_type` refuses; a `+json` suffix writes `json`.
- Query, fragment, `%2F`, `%5C`, an escape forming `C:`, or a non-`file:` value -> `into_path` refuses.
- Absolute `joinpath` argument -> replaces the path.
- `parents` -> values of the source type; invalid steps skipped.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test uri -- path unc escaped_ascii identifiers_infer parts_resolves parents_walks navigation --skip receive_file_scheme
    cargo bench -p yggdryl --bench uri -- "resource_parse/windows_(drive|unc)_normalization"
    cargo bench -p yggdryl --bench uri -- "resource_value/(path_segment_iteration|extension_iteration|stem_access|media_type_inference|media_type_mutation|file_path_projection)"
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/uri -k "path_collection or joinpath or windows_and_unc or filename_mutators or mime_and_media"
    python/.venv/bin/python -m pytest python/tests/holder/test_io.py::TestUrlPathlibParity -k "naming or joining or parents or renaming"
    python/.venv/bin/python python/benchmarks/uri.py --iterations 2000
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="path collections|path joining|fromPath|filename mutations|MIME and media|naming questions|joining and climbing|renaming a URL" node/tests/uri/uri.test.js
    ```
