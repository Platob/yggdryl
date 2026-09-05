# Patterns

Reading a query off a URL path: glob detection and decomposition, `.gitignore`-rule matching, and Hive `column=value` partitions.

## Contract

| Spelling | Meaning |
| --- | --- |
| `*`, `?` | Inside one name: any run, any one character |
| `[a-z]`, `[!a-z]` | One character in, or outside, the set |
| `**` | Any number of levels |
| No `/` | The name, at any depth |
| Any `/` | Anchored at the path root |
| Glob in a URL | Spelled with `*`; the full syntax is for `matches_glob` text |
| `glob_parts` | Deepest fixed root, then the rest |
| `hive_partitions` | `column=value` directories, in path order; read back by [Partitions](../holder/iobase/partitions.md) |
| Rust only | `glob_parts`, `is_recursive_glob`, `matches_glob_under` |

## Use

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
    const { Url } = require('yggdryl')

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

## Edges

- `?` or `[` in a URL -> not a glob: `?` opens the query, `[` is reserved for an IPv6 host.
- `glob_parts()` on a plain location -> `(self, None)`, no error.
- `matches_glob_under(root, ..)` from outside `root` -> `false`.
- `relative_to(root)` from outside `root` -> Python `ValueError`, JavaScript throws.
- `hive_partition("day")` with no such directory -> `None` / `null`.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib uri::pattern::
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/holder/test_io.py::TestUrlPathlibParity -k "matching or relative_to"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="gitignore rule|relative to|Hive partitions" node/tests/uri/uri.test.js
    ```
