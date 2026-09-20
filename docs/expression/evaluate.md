# Evaluate

Application over every target: the streamed Arrow tier, native records, the bound term over rows, batches, statistics, and Iceberg scan planning.

## Contract

| Key | Value |
| --- | --- |
| Owns | the Arrow tier, `apply_arrow_reader` / `apply_arrow_batch` / `apply_arrow_array` on every layer, `apply_records`, `Bound::evaluate` / `filter` / `filter_reader`, `Table::plan_matching` / `scan_matching` |
| Reader first | `apply_arrow_reader` binds once and wraps the stream; `apply_arrow_batch` is one batch through that same reader, so nothing is collected and one batch returned unchanged is the caller's own |
| Arrow tier | An optimization of the row tier, never a second definition; a property test asserts equality on every operator, nulls and `nan` included |
| Kernels | Comparisons run `arrow-ord`, null tests read the validity buffer, `and` / `or` / `not` are three-valued buffer arithmetic; all else runs the row evaluator and gathers, which is slower |
| Zero-copy | A mask keeping every row hands back the input batch; a projection of bare columns reorders `ArrayRef`s and touches no buffer; `select *` and a same-type cast are skipped at bind |
| Records | `apply_records(schema, rows)` binds once against `schema`, or the first record's own datatype, and streams canonical rows; `Records::into_arrow_reader` batches them, `from_arrow_reader` reads them back |
| Parameters | Rust `bind_with(&field, &[(name, Scalar)])`; Python `bind(field, parameters=None)`; JavaScript `bind(fieldLike, parameters?)` takes a `Scalar` record or a plain object; one core binder |
| Order | `order by` is the one section that collects; `offset` and `limit` slice views without copying |
| Bindings | every application in all three; `Bounds` statistics in Rust and Python |

## Use

One bind per stream, and the stream stays a stream.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::expression::Plan;
    use yggdryl::{DataType, Expression, Scalar, StructType};

    let root = DataType::from(StructType::from_fields([
        DataType::utf8().nullable_field("ccy"),
        DataType::Int64.nullable_field("size"),
    ])?)
    .required_field("rows");
    let batch = RecordBatch::try_new(
        root.clone().into_arrow_schema()?,
        vec![
            Arc::new(StringArray::from(vec!["A", "B", "C", "D"])),
            Arc::new(Int64Array::from(vec![Some(1_i64), Some(4), Some(3), None])),
        ],
    )?;

    // The sections run in order over the stream: where, order by, select, limit.
    let plan: Plan = "select ccy, size as quantity where size >= 2 order by size desc limit 1".parse()?;
    let reader = plan.apply_arrow_reader(yggdryl::arrow::batch_reader(batch.schema(), [batch.clone()]))?;
    assert_eq!(reader.schema().field(1).name(), "quantity");
    let kept: usize = reader.map(|batch| batch.unwrap().num_rows()).sum();
    assert_eq!(kept, 1);

    // A sequence applies its steps one after another.
    let steps: Expression = "where size is not null; select upper(ccy) as ccy".parse()?;
    let out = steps.apply_arrow_batch(&batch)?;
    assert_eq!(out.num_rows(), 3);
    assert_eq!(out.schema().field(0).name(), "ccy");

    // Native records go through the same bound plan, one bind for all of them.
    let rows = [
        Scalar::from_struct([("ccy", Scalar::from("a")), ("size", Scalar::from(1_i64))])?,
        Scalar::from_struct([("ccy", Scalar::from("b")), ("size", Scalar::from(2_i64))])?,
    ];
    let records = steps.apply_records(Some(&root), rows)?;
    assert_eq!(records.field().fields()[0].name(), "ccy");
    assert_eq!(
        records.collect_rows()?,
        vec![
            Scalar::from_sequence([Scalar::from("A")]),
            Scalar::from_sequence([Scalar::from("B")]),
        ]
    );
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Expression, Field, Plan

    root = Field("rows", "struct<ccy:utf8,size:int64>", False)
    batch = pa.record_batch(
        {"ccy": ["A", "B", "C", "D"], "size": pa.array([1, 4, 3, None], pa.int64())}
    )

    # The sections run in order over the stream: where, order by, select, limit.
    plan = Plan("select ccy, size as quantity where size >= 2 order by size desc limit 1")
    reader = plan.apply_arrow_reader(pa.RecordBatchReader.from_batches(batch.schema, [batch]))
    assert reader.schema.names == ["ccy", "quantity"]
    assert reader.read_all().column("quantity").to_pylist() == [4]
    # A batch, a table, or a reader comes back as what it was.
    assert isinstance(plan.apply_arrow(pa.Table.from_batches([batch])), pa.Table)

    # A sequence applies its steps one after another.
    steps = Expression("where size is not null; select upper(ccy) as ccy")
    assert steps.apply_arrow_batch(batch).column("ccy").to_pylist() == ["A", "B", "C"]

    # Native records go through the same bound plan, one bind for all of them.
    records = steps.apply_records([{"ccy": "a", "size": 1}, {"ccy": "b", "size": 2}], root)
    assert records.field.dtype["ccy"].dtype == root.dtype["ccy"].dtype
    assert records.collect() == [{"ccy": "A"}, {"ccy": "B"}]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, Expression, Field, Plan } = require('yggdryl')

    const root = new Field('rows', 'struct<ccy:utf8,size:int64>', false)
    const table = new arrow.Table({
      ccy: arrow.vectorFromArray(['A', 'B', 'C', 'D'], new arrow.Utf8()),
      size: arrow.vectorFromArray([1n, 4n, 3n, null], new arrow.Int64()),
    })
    const batch = table.batches[0]

    // The sections run in order over the stream: where, order by, select, limit.
    const plan = new Plan('select ccy, size as quantity where size >= 2 order by size desc limit 1')
    const reader = plan.applyArrowReader(BatchReader.from(table))
    assert.ok(reader instanceof BatchReader)
    assert.deepEqual([...reader.intoTable().getChild('quantity')], [4n])
    // A batch, a table, or a reader comes back as what it was.
    assert.ok(arrow.isArrowTable(plan.applyArrow(table)))

    // A sequence applies its steps one after another.
    const steps = new Expression('where size is not null; select upper(ccy) as ccy')
    assert.deepEqual([...steps.applyArrowBatch(batch).getChild('ccy')], ['A', 'B', 'C'])

    // Native records go through the same bound plan, one bind for all of them.
    const records = steps.applyRecords([{ ccy: 'a', size: 1n }, { ccy: 'b', size: 2n }], root)
    assert.equal(records.field.dtype.getFieldAt(0).name, 'ccy')
    assert.deepEqual([...records].map((row) => row.asJs()), [['A'], ['B']])
    ```

## Vectorized, and zero-copy where the shape allows

A text to temporal cast is the one kernel with a reading in front of it.
The column spells through the row code, and Arrow answers only spellings a row refuses.

| Shape | Buffers |
| --- | --- |
| mask keeps every row | input batch handed straight back, columns pointer-identical |
| mask keeps some rows | copied, because a `RecordBatch` is dense |
| projection of bare columns | `ArrayRef`s reordered, no buffer touched |
| `select *`, a self alias, a cast to the type a column has | skipped at bind; the reader is the one that came in |
| `offset`, `limit` | `RecordBatch::slice` views over the batches they cross |
| `order by` | the stream collected once, then `lexsort_to_indices` and one `take` |

## Targets

`Bound` answers each target with the method named for it.

| Target | Method | Produces |
| --- | --- | --- |
| one row (`Scalar`) | `eval`, `matches` | the `Scalar` the term computes, or whether it is exactly `true` |
| one holder (`dyn Attributes`) | `matches_holder`, `settle_holder` | what the holder alone settles, three-valued |
| one Arrow `RecordBatch` | `evaluate`, `filter_mask`, `filter` | one column of answers, a null-free mask, the kept rows |
| one Arrow stream | `filter_reader` | the filtering reader, one batch at a time |
| one container's statistics (`Bounds`) | `statistics_prune`, `statistics_certainty` | whether any row can match; the `Option<bool>` certainty |

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use yggdryl::expression::{Attributes, Bounds, Term};
use yggdryl::{Field, Scalar, Url};

// Non-nullable at the root, because the batch below projects it to Arrow.
let schema = "trades:struct<ccy:utf8,size:bigint>".parse::<Field>()?.with_nullable(false);
let bound = "ccy = 'EUR' and size > 10".parse::<Term>()?.bind(&schema)?;

// One row answers the value the term computes.
let row = Scalar::from_sequence([Scalar::from("EUR"), Scalar::from(25_i64)]);
assert_eq!(bound.eval(&row)?, Scalar::from(true));

// One batch answers one column of answers, one per row.
let arrow_schema = schema.into_arrow_schema()?;
let batch = RecordBatch::try_new(
    Arc::clone(&arrow_schema),
    vec![
        Arc::new(StringArray::from(vec!["EUR", "USD"])),
        Arc::new(Int64Array::from(vec![25_i64, 25])),
    ],
)?;
assert_eq!(bound.evaluate(&batch)?.len(), 2);
assert_eq!(bound.filter(&batch)?.num_rows(), 1);

// Statistics answer the certainty pruning runs on: every size is below 10,
// so no row can match and the container is skipped unread.
let bounds = Bounds::new(Some(1_000))
    .with_column("size", Some(Scalar::from(1_i64)), Some(Scalar::from(5_i64)), Some(0));
assert_eq!(bound.statistics_certainty(&bounds), Some(false));

// A holder settles only the conjuncts that need no row - here none - and an
// unknown answer excludes nothing.
let url = Url::from_str("file:///lake/year=2024/part-0.parquet")?;
let holder: &dyn Attributes = &url;
assert!(bound.matches_holder(holder)?);

// The stream application is the filtering reader: only the EUR row survives.
let filtered = bound.filter_reader(yggdryl::arrow::batch_reader(arrow_schema, [batch]));
let kept: usize = filtered.map(|batch| batch.unwrap().num_rows()).sum();
assert_eq!(kept, 1);
```

## Iceberg: one predicate, every level of the metadata

The scan is planned by the filter that keeps the rows: a manifest-list summary answers first, then a manifest entry's partition tuple and column bounds. A `where` on a record read of a table is that filter, pushed down whole - a range, an `in` list, a null test or a holder attribute prunes with the whole expression language, exactly as an equality does - and the `select` is the read's projection. Pushdown and time travel are on [Reading](../media/iceberg/read.md).

=== "Rust"

    ```{ .rust .ignore }
    use yggdryl::iceberg::Table;
    use yggdryl::local::Folder;

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
- `apply_arrow_batch` on a plan with `order by` -> the one batch is sorted; on a reader every batch is collected first, and nothing else collects.
- Holder settles no conjunct -> `matches_holder` is true; an unknown answer excludes nothing.
- `Bounds` prove no row can match -> `Some(false)`; the container is skipped unread.
- Conjunct proven by the partition tuple -> dropped rather than re-tested; what no metadata level settles is left for the rows.
- Text to temporal cast on a column -> the row reader spells first; the kernel sees only spellings a row refuses.
- `apply_arrow` / `applyArrow` -> the input kind is preserved: record batch, table, or reader.
- `apply_records` with a schema -> every row is canonicalized under it first; without one, the first record's own datatype is the schema and a stream with no record is refused.
- `Records::into_arrow_reader` -> the rows batched lazily under `field()`; consumed once.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib -- expression::tests::scalar_and_vectorized_agree expression::tests::projections_agree_between_the_tiers expression::tests::a_mask_that_keeps_everything_keeps_the_batch_itself expression::tests::a_projection_reorders_without_touching_a_buffer expression::tests::a_reader_filters_and_projects_in_one_pass expression::tests::binds_and_evaluates_rows
    cargo test --features "parquet iceberg" -p yggdryl --lib -- expression::plan::tests::streams::a_plan_shapes_a_stream_in_section_order expression::plan::tests::streams::records_run_through_every_expression
    cargo test --features "parquet iceberg" -p yggdryl --lib iceberg::tests::planning
    cargo bench -p yggdryl --bench expression -- expression_mask
    cargo bench -p yggdryl --bench expression -- kernel_mask
    cargo bench -p yggdryl --bench expression -- expression_filter
    cargo bench -p yggdryl --bench expression -- kernel_filter
    cargo bench -p yggdryl --bench expression -- expression_rows
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/expression -k "shapes_a_stream or records or batch_at_once or partitioned_table or one_predicate or one_plan"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="shapes a stream|records|prunes manifests" node/tests/expression
    ```

## Performance

`benchmarks/expression.rs` writes each predicate by hand against `arrow-ord` / `arrow-select`, and `expression_mask` and `kernel_mask` share case IDs.
Indicative numbers from one containerized x86_64 Linux run ([Benchmarks](../benchmarks.md)), 65,536 rows, `--measurement-time 1`:

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
