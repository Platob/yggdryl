# URL and URN

This page owns `Url` and `Urn`, two of the three narrowed forms of the canonical [`Uri`](index.md) — the third is the [`Arn`](arn.md) — and the accessors a URL's scheme alone decides. It also owns how a name becomes a location, which is what lets a URN be opened.

## Contract

| Aspect | Rule |
| --- | --- |
| `Url` requires | hierarchical authority syntax and a non-empty host, except under `file:` |
| `Urn` requires | the `urn` scheme, no authority, a namespace plus a non-empty namespace-specific string |
| URN case | namespace lowercased; namespace-specific string kept exactly as written |
| Conversion | `Uri` to `Url` or `Urn` and back: no re-parsing. `Url::from_uri` / `Url.from_uri` is the strict door and refuses a name; the `Url` constructor is the *location* door and resolves one, the way it roots a bare path |
| `locator_path` | the relative path a name spells: its namespace first, then the namespace-specific string's `:` separators — `urn:lake:trades:2026:part.parquet` spells `lake/trades/2026/part.parquet`. An empty part between two `:` is refused, because two names would then spell one path |
| `resolve`, `locator` | `resolve(base)` joins that path onto a base location; `locator()` joins it onto the process working directory, which is the root every relative path is read against. `Uri::locator` answers the same for whichever narrowing the value is |
| URN filenames | [accessors](path.md) read the namespace-specific string, not a slash path; setters leave the namespace alone |
| `default_port` | the port a client dials when the authority omits one: `http` 80, `https` 443, `postgres` 5432, `mysql` 3306, else `None` |
| `is_local`, `join_path` | scheme is `file:`; `Path::join` for URLs, one segment per component, each component percent-encoded as the name it is |
| `exists`, `is_dir`, `is_file` | local URL only; `false` for every other scheme, no network call |
| `local_mime_type` | existing directory: [`MimeType::DIRECTORY`](../types/scalar.md); local file: from its name, else `FILE`; remote: `mime_type` |
| Bindings | Python answers `default_port`, `is_local`, `local_mime_type`, and reaches `join_path` by handing `joinpath` an `os.PathLike`; JavaScript is Rust-only here. The three predicates exist in both bindings |
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

## Where a name is

A URN names a resource without saying where it is. Reading the name as a path is what answers that, and it is the whole of the resolution: the namespace leads the path and the namespace-specific string's `:` separators are the ones after it, so one base turns a whole namespace of names into locations. That is what makes a name openable — everything that takes a location takes a name too.

=== "Rust"

    ```rust
    use yggdryl::{Uri, Url, Urn};

    let urn = Urn::from_str("urn:lake:trades:2026:part.parquet")?;
    assert_eq!(urn.locator_path()?.as_str(), "lake/trades/2026/part.parquet");

    // Under a base, the name is a location in that store.
    let base = Url::from_str("s3://market-data/warehouse/")?;
    assert_eq!(
        urn.resolve(&base)?.to_string(),
        "s3://market-data/warehouse/lake/trades/2026/part.parquet"
    );

    // With no base named, the working directory is the root.
    let located = urn.locator()?;
    assert!(located.is_local());
    assert!(located.to_string().ends_with("/lake/trades/2026/part.parquet"));
    assert_eq!(Uri::from_str("urn:lake:trades:2026:part.parquet")?.locator()?, located);

    // The location door reads a name written as text the same way.
    assert_eq!(Url::from_location("urn:lake:trades:2026:part.parquet")?, located);

    // Escapes cross as the name holds them, and an empty part spells no path.
    assert_eq!(Urn::from_str("urn:example:a%2Fb")?.locator_path()?.as_str(), "example/a%2Fb");
    assert!(Urn::from_str("urn:example:a::b")?.locator_path().is_err());
    ```

=== "Python"

    ```python
    from yggdryl import Uri, Url, Urn

    urn = Urn("urn:lake:trades:2026:part.parquet")
    assert urn.locator_path() == "lake/trades/2026/part.parquet"

    assert urn.resolve("s3://market-data/warehouse/") == Url(
        "s3://market-data/warehouse/lake/trades/2026/part.parquet"
    )

    located = urn.locator()
    assert located.is_local()
    assert str(located).endswith("/lake/trades/2026/part.parquet")
    assert Uri("urn:lake:trades:2026:part.parquet").locator() == located

    # The location door takes a name, written as a value or as text; the
    # strict `from_uri` door refuses it.
    assert Url(urn) == located
    assert Url("urn:lake:trades:2026:part.parquet") == located
    try:
        Url.from_uri(urn)
        raise AssertionError("expected a rejection")
    except ValueError:
        pass
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Uri, Url, Urn } = require('yggdryl')

    const urn = Urn.from('urn:lake:trades:2026:part.parquet')
    assert.equal(urn.locatorPath(), 'lake/trades/2026/part.parquet')

    assert.equal(
      urn.resolve('s3://market-data/warehouse/').toString(),
      's3://market-data/warehouse/lake/trades/2026/part.parquet',
    )

    const located = urn.locator()
    assert.equal(located.scheme, 'file')
    assert.ok(located.toString().endsWith('/lake/trades/2026/part.parquet'))
    assert.equal(Uri.from('urn:lake:trades:2026:part.parquet').locator().toString(), located.toString())

    assert.equal(Url.from(urn).toString(), located.toString())
    assert.throws(() => Url.fromUri(urn))
    ```

## What the scheme decides

Shown in Rust; Python answers `default_port`, `is_local` and `local_mime_type` under those names and reaches `join_path` through `joinpath` with an `os.PathLike`, and JavaScript reaches none of them. The `exists`, `is_dir`, and `is_file` predicates are in all three, under those names.

=== "Rust"

    ```rust
    use yggdryl::local::LocalFolder;
    use yggdryl::{MimeType, Uri, Url};

    // The port belongs to the scheme, not to the authority text.
    assert_eq!(Url::from_str("https://example.test")?.default_port(), Some(443));
    assert_eq!(Uri::from_str("postgres://host/db")?.default_port(), Some(5432));
    assert_eq!(Uri::from_str("s3://bucket/key")?.default_port(), None);

    let root = LocalFolder::temporary()?.path()?.join(format!("yggdryl-doc-uri-{}", std::process::id()));
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
- `urn:example:a::b` -> `locator_path` refuses: an empty part would let two names spell one path.
- `https://example.test:8443` -> `default_port` is still `Some(443)`; a written port is never read.
- `join_path` with an absolute path -> replaces the path outright, components and all; a non-UTF-8 component -> refused.
- `join_path("100%.csv")` -> `.../100%25.csv`; `joinpath("100%.csv")` -> refused, because that door takes URI text.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test uri -- url_conversion urn_ default_port
    cargo bench -p yggdryl --bench uri -- "resource_parse/(url|urn)_canonical"
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_uri.py -k "url_converts or urn_components or name_resolves"
    python/.venv/bin/python -m pytest python/tests/test_iobase.py::TestUrlPathlibParity -k file_system_predicates
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="URL conversion|URN values|file system predicates" node/tests/uri.test.js
    ```
