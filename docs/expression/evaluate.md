# Evaluate

Evaluation over every target: the vectorized Arrow tier, `Statement::bind` with streamed projection, the `ApplyExpression` traits, and Iceberg scan planning.

## Contract

| Key | Value |
| --- | --- |
| Owns | Arrow tier, `Statement::bind`, `project_reader`, `ApplyExpression` / `ApplyExpressionStream`, `Table::plan_matching` / `scan_matching` |
| Arrow tier | An optimization of the row tier, never a second definition; a property test asserts equality on every operator, nulls and `nan` included |
| Kernels | Comparisons run `arrow-ord`, null tests read the validity buffer, `and` / `or` / `not` are three-valued buffer arithmetic; all else runs the row evaluator and gathers |
| Zero-copy | A mask keeping every row hands back the input batch; a projection reorders `ArrayRef`s and touches no buffer |
| Bind | Resolves projections, predicate, ordering, parameters, and output field against one struct `Field`, once |
| Parameters | Rust `bind_with(&field, &[(name, Scalar)])`; Python `bind(field, parameters=None)`; JavaScript `bind(fieldLike, parameters?)` |
| Streamed | `project_reader` / `project_arrow` / `projectArrow` apply predicate and projection per batch with one limit across the stream |
| Sort | `sort` / `sort_arrow_batch` / `sortArrowBatch` order one materialized batch |
| Bindings | Statement bind and Iceberg planning in all three; `ApplyExpression` is Rust only |

## Use

A `BoundStatement` exposes the resolved plan without reparsing it.

=== "Rust"

    ```rust
    use yggdryl::expression::Statement;
    use yggdryl::Field;

    let field: Field = "rows:struct<ccy:utf8,size:bigint>".parse()?;
    let statement: Statement = "select ccy, size as quantity where size >= 2 limit 10".parse()?;
    let bound = statement.bind(&field)?;
    assert_eq!(bound.output().fields()[1].name(), "quantity");
    ```

=== "Python"

    ```python
    from yggdryl import Field, Statement

    field = Field("rows", "struct<ccy:utf8,size:bigint>", False)
    bound = Statement(
        "select ccy, size as quantity where size >= :floor limit 10"
    ).bind(field, {"floor": 2})
    assert bound.output.name == "rows"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, Statement } = require('yggdryl')

    const field = new Field('rows', 'struct<ccy:utf8,size:bigint>', false)
    const bound = new Statement(
      'select ccy, size as quantity where size >= :floor limit 10',
    ).bind(field, { floor: 2 })
    assert.equal(bound.output.name, 'rows')
    ```

## Vectorized, and zero-copy where the shape allows

A text to temporal cast is the one kernel with a reading in front of it: the column spells through the row code, and Arrow answers only spellings a row refuses.

| Shape | Buffers |
| --- | --- |
| mask keeps every row | input batch handed straight back, columns pointer-identical |
| mask keeps some rows | copied, because a `RecordBatch` is dense |
| projection | `ArrayRef`s reordered, no buffer touched |

## Bind a whole statement once

Global ordering needs every row, so the `sort` family sorts one materialized batch; the spelled-out projection methods make the return type explicit.

| Accessor | Reports |
| --- | --- |
| `output()` | the resolved output field, aliases applied |
| `ordering` | each expression with its direction and optional null placement |
| `is_all` | true only for an unfiltered, unordered, unlimited `select *` |

## Targets own their application

`ApplyExpression` lets the target say what applying an expression produces, as an associated type; `Bound` keeps only `matches` (apply then require boolean) and `filter` (apply then select).

| Target | Applying an expression produces |
| --- | --- |
| one row (`Scalar`) | the `Scalar` the expression computes |
| one holder (`dyn Attributes`) | what the holder alone settles, three-valued |
| one Arrow `RecordBatch` | one column of answers, an `ArrayRef` |
| one container's statistics (`Bounds`) | the `Option<bool>` certainty pruning runs on |

Rust only.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use yggdryl::expression::{ApplyExpression, ApplyExpressionStream, Attributes, Bounds};
use yggdryl::{Expression, Field, Url, Scalar};

// Non-nullable at the root, because the batch below projects it to Arrow.
let schema = "trades:struct<ccy:utf8,size:bigint>".parse::<Field>()?.with_nullable(false);
let bound = "ccy = 'EUR' and size > 10".parse::<Expression>()?.bind(&schema)?;

// One row applies to the value the expression computes.
let row = Scalar::from_sequence([Scalar::from("EUR"), Scalar::from(25_i64)]);
assert_eq!(row.apply_expression(&bound)?, Scalar::from(true));

// One batch applies to one column of answers, one per row.
let arrow_schema = schema.into_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
    vec![
        Arc::new(StringArray::from(vec!["EUR", "USD"])),
        Arc::new(Int64Array::from(vec![25_i64, 25])),
    ],
)?;
assert_eq!(batch.apply_expression(&bound)?.len(), 2);

// Statistics apply to the certainty pruning runs on: every size is below 10,
// so no row can match and the container is skipped unread.
let bounds = Bounds::new(Some(1_000))
    .with_column(
        "size",
        Some(Scalar::from(1_i64)),
        Some(Scalar::from(5_i64)),
        Some(0),
    );
assert_eq!(bounds.apply_expression(&bound)?, Some(false));

// A holder settles only the conjuncts that need no row - here none - and an
// unknown answer excludes nothing.
let url = Url::from_str("file:///lake/year=2024/part-0.parquet")?;
let holder: &dyn Attributes = &url;
assert_eq!(holder.apply_expression(&bound)?, Scalar::Null);

// The stream sibling consumes its reader, and its application is the
// filtering reader: only the EUR row above survives.
let filtered = yggdryl::arrow::batch_reader(arrow_schema, [batch])
    .apply_expression_stream(&bound)?;
let mut kept = 0;
for batch in filtered {
    kept += batch?.num_rows();
}
assert_eq!(kept, 1);
```

A new target implements the trait beside its own type and `expression/` never learns it exists; `rust/src/iobase/tests/applying.rs` proves it with a `Listing` target reached through the public surface alone.

## Iceberg: one predicate, every level of the metadata

The scan is planned by the expression that filters the rows: a manifest-list summary answers first, then a manifest entry's partition tuple and column bounds. Pushdown and time travel are on [Reading](../media/iceberg/read.md).

=== "Rust"

    ```{ .rust .ignore }
    use yggdryl::media::iceberg::Table;
    use yggdryl::holder::local::Folder;

    let table = Table::open(Folder::new("/lake/trades")?)?;

    let plan = table.plan_matching("&holder.partition['year'] = '2024'")?;
    println!("{} manifests never opened", plan.manifests_skipped());

    let reader = table.scan_matching(
        "ccy = 'EUR' and price > 100 and &holder.partition['year'] = '2024'",
        None,
    )?;
    ```

=== "Python"

    ```{ .python .ignore }
    from yggdryl.media.iceberg import Table

    table = Table("/lake/trades")

    plan = table.plan_matching("&holder.partition['year'] = '2024'")
    print(plan["manifests_skipped"], "manifests never opened")

    reader = table.scan_matching(
        "ccy = 'EUR' and price > 100 and &holder.partition['year'] = '2024'"
    )
    ```

=== "JavaScript"

    ```{ .javascript .ignore }
    const { iceberg } = require('yggdryl')

    const table = iceberg.Table.open('/lake/trades')

    const plan = table.planMatching("&holder.partition['year'] = '2024'")
    console.log(plan.manifestsSkipped, 'manifests never opened')

    const reader = table.scanMatching(
      "ccy = 'EUR' and price > 100 and &holder.partition['year'] = '2024'",
    )
    ```

## Edges

- Mask keeps some rows -> the batch is copied; only a mask keeping every row is zero-copy.
- `BatchReader` -> no `&self` application; `apply_expression_stream` consumes it and yields the filtering reader, one batch at a time.
- Holder settles no conjunct -> `Scalar::Null`; an unknown answer excludes nothing.
- `Bounds` prove no row can match -> `Some(false)`; the container is skipped unread.
- Conjunct proven by the partition tuple -> dropped rather than re-tested; what no metadata level settles is left for the rows.
- Text to temporal cast on a column -> the row reader spells first; the kernel sees only spellings a row refuses.
- `sort` / `sort_arrow_batch` / `sortArrowBatch` -> one batch, never a stream.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib -- expression::tests::scalar_and_vectorized_agree expression::tests::projections_agree_between_the_tiers expression::tests::a_mask_that_keeps_everything_keeps_the_batch_itself expression::tests::a_projection_reorders_without_touching_a_buffer expression::tests::a_reader_filters_and_projects_in_one_pass expression::tests::binds_and_evaluates_rows
    cargo test --features "parquet iceberg" -p yggdryl --lib iobase::tests::applying
    cargo test --features "parquet iceberg" -p yggdryl --lib media::iceberg::tests::planning
    cargo bench -p yggdryl --bench expression -- expression_mask
    cargo bench -p yggdryl --bench expression -- kernel_mask
    cargo bench -p yggdryl --bench expression -- expression_filter
    cargo bench -p yggdryl --bench expression -- kernel_filter
    cargo bench -p yggdryl --bench expression -- expression_rows
    cargo bench -p yggdryl --bench expression -- expression_parse/statement
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/expression -k "statement or partitioned_table or one_predicate or one_plan"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="statement|prunes manifests" node/tests/expression
    ```

## Performance

`benchmarks/expression.rs` writes each predicate by hand against `arrow-ord` / `arrow-select` as the baseline; `expression_mask` and `kernel_mask` share case IDs, so the gap is the price of the grammar. One containerized x86_64 Linux run on the published host ([Benchmarks](../benchmarks.md); rustc 1.94.1, thin LTO), 65,536 rows, `--measurement-time 1`:

```text
                       expression   kernel
utf8_equality             181.5 us  177.2 us
int64_range                41.1 us   36.9 us
decimal_range              48.6 us   51.0 us
set_membership            347.3 us  349.1 us
conjunction               277.6 us  271.4 us
```

`expression_parse` runs 0.5-2.4 us per predicate and `expression_bind` 0.5-4.5 us, once per stream rather than once per batch. `null_test`, the filter groups, and `expression_rows` carry no published numbers; ROWS is `bench_profile::corpus(65_536, 16_384)`, the full corpus only under `cargo bench`.

```bash
cargo bench -p yggdryl --bench expression -- --measurement-time 1 "expression_mask|kernel_mask"
```
