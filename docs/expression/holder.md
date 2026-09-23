# Holder attributes

`&holder.*` asks about the container, not the rows, so a listing answers it before any decode.

## Contract

| Item | Rule |
| --- | --- |
| Owns | `Attribute`, `&holder.*`, `children_matching`, `statistics_prune`, `partition_split` |
| Ordering | `bind` sorts conjuncts cheapest-first, stopping at the first `false` |
| Over-stated cost | Harmless; a later attribute still answers, so a backend-dependent price is classed by its worst case |
| Pruning | `false` only when no row can match |
| Split | Sound because dropping conjuncts only widens what is kept |
| Grammar | [Grammar](grammar.md) |
| Partition columns | [Partitions](../holder/index.md#partitions) |

## Use

=== "Rust"

    ```rust
    use yggdryl::IOBase;
    use yggdryl::local::LocalFolder;
    use yggdryl::Filter;

    let lake = LocalFolder::new(LocalFolder::temporary()?.path()?.join("yggdryl-docs-lake"))?;
    std::fs::create_dir_all(lake.path()?.join("year=2024"))?;
    std::fs::write(lake.path()?.join("year=2024").join("part-0.parquet"), b"")?;
    std::fs::create_dir_all(lake.path()?.join("year=2025"))?;
    std::fs::write(lake.path()?.join("year=2025").join("part-0.parquet"), b"")?;

    let filter: Filter = "&holder.partition['year'] = '2024'".parse()?;
    let matched: Vec<_> = lake
        .children_matching(&filter, false)?
        .collect::<yggdryl::Result<_>>()?;
    assert!(!matched.is_empty());
    assert!(matched.iter().all(|entry| {
        entry.url().is_some_and(|url| url.to_string().contains("year=2024"))
    }));

    std::fs::remove_dir_all(lake.path()?)?;
    ```

=== "Python"

    ```python
    import tempfile
    from pathlib import Path

    from yggdryl import IOBase

    with tempfile.TemporaryDirectory() as root:
        for year in ("2024", "2025"):
            leaf = Path(root) / f"year={year}"
            leaf.mkdir()
            (leaf / "part-0.parquet").write_bytes(b"")

        lake = IOBase(root)
        matched = lake.children_matching("&holder.partition['year'] = '2024'")
        assert matched
        assert all("year=2024" in str(entry.url) for entry in matched)
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { IOBase } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-'))
    for (const year of ['2024', '2025']) {
      fs.mkdirSync(path.join(root, `year=${year}`))
      fs.writeFileSync(path.join(root, `year=${year}`, 'part-0.parquet'), '')
    }

    const lake = new IOBase(root)
    const matched = [...lake.childrenMatching("&holder.partition['year'] = '2024'")]
    assert.ok(matched.length > 0)
    for (const entry of matched) {
      assert.match(String(entry.url), /year=2024/)
    }

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Cost classes

| Cost | Attributes | What it takes |
| --- | --- | --- |
| free | `url`, `path`, `name`, `stem`, `extension`, `scheme`, `parent`, `depth`, `mime_type`, `partition['column']` | the identifier alone |
| stat | `size`, `kind`, `is_container`, `is_empty` | one call into the backing store |

## Statistics pruning

Per-column minimums, maximums, and null counts settle it, in a Parquet footer, an Iceberg manifest, or a Hive path.

Rust and Python; JavaScript has no `Bounds`.

=== "Rust"

    ```rust
    use yggdryl::expression::{Bounds, Term};
    use yggdryl::{Field, Scalar};

    let schema: Field = "trades:struct<ccy:utf8,size:bigint>".parse()?;
    let bounds = Bounds::new(Some(1_000))
        .with_column("ccy", Some(Scalar::from("EUR")), Some(Scalar::from("USD")), Some(0))
        .with_column(
            "size",
            Some(Scalar::from(1_i64)),
            Some(Scalar::from(99_i64)),
            Some(4),
        );

    // Provably empty: no row can hold a size above the file's maximum.
    assert!(!"size > 1000".parse::<Term>()?.bind(&schema)?.statistics_prune(&bounds));
    // Not provable either way: the range overlaps, so the file is read.
    assert!("size > 50".parse::<Term>()?.bind(&schema)?.statistics_prune(&bounds));
    // A null test the count settles outright.
    assert!("size is null".parse::<Term>()?.bind(&schema)?.statistics_prune(&bounds));
    ```

=== "Python"

    ```python
    from yggdryl import Bounds, Field, Term

    schema = Field("trades", "struct<ccy:utf8,size:bigint>", False)
    bounds = Bounds(rows=1_000).with_column("ccy", "EUR", "USD", 0).with_column("size", 1, 99, 4)

    # Provably empty: no row can hold a size above the file's maximum.
    assert not Term("size > 1000").bind(schema).statistics_prune(bounds)
    # Not provable either way: the range overlaps, so the file is read.
    assert Term("size > 50").bind(schema).statistics_prune(bounds)
    # A null test the count settles outright.
    assert Term("size is null").bind(schema).statistics_prune(bounds)
    ```

## Partition split

`partition_split` separates the conjuncts that read only partition columns and holder attributes from the residual over rows, each half a `Filter`.

Shown in Rust; Python's `Bound.partition_split()` answers the same two filters as a tuple, and JavaScript's `partitionSplit()` as `{ answerable, remaining }`.

```rust
use yggdryl::expression::Term;
use yggdryl::Field;

let mut schema: Field = "trades:struct<year:int32,price:decimal(9,2)>".parse()?;
let mut children = schema.fields().to_vec();
children[0].set_partition(true);
schema.set_dtype(yggdryl::DataType::from(yggdryl::StructType::from_fields(children)?))?;

let bound = "year = 2024 and price > 100".parse::<Term>()?.bind(&schema)?;
let residual = bound.partition_split();
assert_eq!(residual.answerable().to_string(), "year = int32 '2024'");
assert_eq!(residual.remaining().to_string(), "price > decimal32(9,2) '100.00'");
assert!(!residual.is_complete());
```

## Edges

- Row-column conjunct -> dropped; a file may be kept, never wrongly discarded.
- Free-attribute conjunct answering `false` -> exactly zero calls into the backing store.
- A Hive path -> the tightest statistic there is: minimum equal to maximum, nothing null.
- Unprovable predicate -> `statistics_prune` returns `true`, one read.
- Incomplete split -> `is_complete()` is `false`; `remaining()` runs over rows.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test expression -- attribute::grammar filter::grammar pushdown::grammar
    cargo bench -p yggdryl --bench expression -- expression_prune
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_expression.py -k "holder_attributes or lake"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="holder attribute|a lake is filtered" node/tests/expression.test.js
    ```
