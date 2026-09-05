# URL and URN

This page owns `Url` and `Urn`, the two narrowed forms of the canonical [`Uri`](index.md), and the accessors a URL's scheme alone decides.

## Contract

| Aspect | Rule |
| --- | --- |
| `Url` requires | hierarchical authority syntax and a non-empty host, except under `file:` |
| `Urn` requires | the `urn` scheme, no authority, a namespace plus a non-empty namespace-specific string |
| URN case | namespace lowercased; namespace-specific string kept exactly as written |
| Conversion | `Uri` to `Url` or `Urn` and back: no re-parsing; `Url` and `Urn` refuse each other |
| URN filenames | [accessors](path.md) read the namespace-specific string, not a slash path; setters leave the namespace alone |
| `default_port` | the port a client dials when the authority omits one: `http` 80, `https` 443, `postgres` 5432, `mysql` 3306, else `None` |
| `is_local`, `join_path` | scheme is `file:`; `Path::join` for URLs, one segment per component |
| `exists`, `is_dir`, `is_file` | local URL only; `false` for every other scheme, no network call |
| `local_mime_type` | existing directory: [`MimeType::DIRECTORY`](../types/scalar.md); local file: from its name, else `FILE`; remote: `mime_type` |
| Rust only | `default_port`, `is_local`, `join_path`, `local_mime_type`; the three predicates exist in both bindings |
| Errors | Rust `Err`, Python `ValueError`, JavaScript throw |

## Use

Both narrowed forms are the same canonical value, so conversion either way is free.

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
    assert!(urn.into_uri().into_url().is_err());
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
        lambda: urn.into_uri().into_url(),
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
    const { Uri, Url, Urn } = require('yggdryl')

    const uri = Uri.from('https://example.test/a/data.json?raw=true')
    const url = Url.from(uri)
    assert.equal(url.authority, 'example.test')
    assert.ok(Uri.from(url).equals(uri))

    const urn = Urn.from('URN:ISBN:9780131103627')
    assert.equal(urn.toString(), 'urn:isbn:9780131103627')
    assert.equal(urn.namespace, 'isbn')
    assert.equal(urn.namespaceSpecific, '9780131103627')
    assert.equal(urn.authority, '')

    assert.throws(() => urn.intoUri().intoUrl())
    assert.throws(() => Urn.fromUri(uri))
    assert.throws(() => Url.fromString('mailto:user@example.test'))
    ```

## What the scheme decides

Rust only, except `exists`, `is_dir`, and `is_file`, which both bindings expose under those names.

=== "Rust"

    ```rust
    use yggdryl::holder::local::Folder;
    use yggdryl::{MimeType, Uri, Url};

    // The port belongs to the scheme, not to the authority text.
    assert_eq!(Url::from_str("https://example.test")?.default_port(), Some(443));
    assert_eq!(Uri::from_str("postgres://host/db")?.default_port(), Some(5432));
    assert_eq!(Uri::from_str("s3://bucket/key")?.default_port(), None);

    let root = Folder::temporary()?.path()?.join(format!("yggdryl-doc-uri-{}", std::process::id()));
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

## Edges

- `urn:x:value`, `urn:-bad:value`, `urn:isbn:`, or `urn:example:value?plain-query` -> `Urn` refuses.
- `urn:a$:value` -> parse error with target `urn` and the offending byte offset, 5.
- `urn:example:reports/data.csv` -> file name `data.csv`; `set_file_name("bad/name")` refuses, URN unchanged.
- `https://example.test:8443` -> `default_port` is still `Some(443)`; a written port is never read.
- `join_path` with an absolute path -> replaces the path; `..` escaping the root or a non-UTF-8 component -> refused.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test uri -- url_conversion urn_ default_port
    cargo bench -p yggdryl --bench uri -- "resource_parse/(url|urn)_canonical"
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/uri -k "url_converts or urn_components"
    python/.venv/bin/python -m pytest python/tests/holder/test_io.py::TestUrlPathlibParity -k file_system_predicates
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="URL conversion|URN values|file system predicates" node/tests/uri/uri.test.js
    ```
