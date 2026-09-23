# Filters

`Filter` is a `where` clause: one predicate that keeps a row only when it answers exactly `true`.

## Contract

| Key | Value |
| --- | --- |
| Owns | `Filter`, `IntoFilter`, `Residual` |
| Build | from a predicate's text (`where` optional), a `Term`, `always_true`, `always_false`, `all`, `any`, `and`, `or`, `not` |
| Normalize | `simplify` turns `a = 1 or a = 2` into `a in (1, 2)`, drops `true` from `and` and `false` from `or`, folds double negation, and keeps every answer under three-valued logic |
| Pushdown | conjuncts are the unit: a holder answers what it can, statistics answer what they can, the rows answer the rest; `partition_pairs` reads the equalities a media prunes by |
| Apply | `apply_field(root)` (the schema itself), `apply_scalar`, `apply_arrow_reader` (primary), `apply_arrow_batch` (the batch itself when everything is kept), `apply_arrow_array`, `apply_records` |
| Bindings | Python and JavaScript `Filter`, with `&`, `\|`, `~` in Python and `and`, `or`, `not` in JavaScript |

## Use

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::{DataType, Filter, StructType};

    let root = DataType::from(StructType::from_fields([
        DataType::utf8().nullable_field("ccy"),
        DataType::Int64.nullable_field("size"),
    ])?)
    .required_field("rows");

    let filter: Filter = "where ccy = 'EUR' and size > 1".parse()?;
    assert_eq!(filter.to_string(), "ccy = 'EUR' and size > 1");
    assert_eq!(filter.conjuncts().len(), 2);
    assert_eq!(filter.clone().not().to_string(), "not (ccy = 'EUR' and size > 1)");

    let batch = RecordBatch::try_new(
        root.clone().into_arrow_schema()?,
        vec![
            Arc::new(StringArray::from(vec!["EUR", "USD", "EUR"])),
            Arc::new(Int64Array::from(vec![5_i64, 5, 0])),
        ],
    )?;
    let kept = filter.apply_arrow_batch(&batch)?;
    assert_eq!(kept.num_rows(), 1);

    // A filter that keeps everything hands the batch itself back.
    let everything = Filter::always_true().apply_arrow_batch(&batch)?;
    assert_eq!(everything.num_rows(), 3);

    // Normalization keeps the answer in fewer nodes.
    let spread: Filter = "size = 1 or size = 2 or size = 3".parse()?;
    assert_eq!(spread.simplify().to_string(), "size in (1, 2, 3)");
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Field, Filter

    root = Field("rows", "struct<ccy:utf8,size:int64>", False)
    filter = Filter("where ccy = 'EUR' and size > 1")
    assert str(filter) == "ccy = 'EUR' and size > 1"
    assert [str(part) for part in filter.conjuncts()] == ["ccy = 'EUR'", "size > 1"]
    assert str(~filter) == "not (ccy = 'EUR' and size > 1)"
    assert str(filter & "size < 9") == "ccy = 'EUR' and size > 1 and size < 9"

    batch = pa.record_batch({"ccy": ["EUR", "USD", "EUR"], "size": pa.array([5, 5, 0], pa.int64())})
    assert filter.apply_arrow_batch(batch).column("ccy").to_pylist() == ["EUR"]
    # A filter that keeps everything hands the batch itself back.
    assert Filter.always_true().apply_arrow_batch(batch) is batch

    kept = filter.apply_records([{"ccy": "EUR", "size": 5}, {"ccy": "USD", "size": 5}], root)
    assert list(kept) == [{"ccy": "EUR", "size": 5}]

    # Normalization keeps the answer in fewer nodes.
    assert str(Filter("size = 1 or size = 2 or size = 3").simplify()) == "size in (1, 2, 3)"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Field, Filter, Scalar } = require('yggdryl')

    const root = new Field('rows', 'struct<ccy:utf8,size:int64>', false)
    const filter = new Filter("where ccy = 'EUR' and size > 1")
    assert.equal(filter.toString(), "ccy = 'EUR' and size > 1")
    assert.deepEqual(filter.conjuncts().map(String), ["ccy = 'EUR'", 'size > 1'])
    assert.equal(filter.not().toString(), "not (ccy = 'EUR' and size > 1)")
    assert.equal(filter.and('size < 9').toString(), "ccy = 'EUR' and size > 1 and size < 9")

    const batch = new arrow.Table({
      ccy: arrow.vectorFromArray(['EUR', 'USD', 'EUR'], new arrow.Utf8()),
      size: arrow.vectorFromArray([5n, 5n, 0n], new arrow.Int64()),
    }).batches[0]
    assert.deepEqual([...filter.applyArrowBatch(batch).getChild('ccy')], ['EUR'])
    assert.equal(Filter.alwaysTrue().applyArrowBatch(batch).numRows, 3)

    const kept = filter.applyRecords([{ ccy: 'EUR', size: 5n }, { ccy: 'USD', size: 5n }], root)
    const rows = [...kept]
    assert.equal(rows.length, 1)
    assert.ok(rows[0].equals(Scalar.from(['EUR', 5n])))

    // Normalization keeps the answer in fewer nodes.
    assert.equal(new Filter('size = 1 or size = 2 or size = 3').simplify().toString(), 'size in (1, 2, 3)')
    ```

## Pushdown

A conjunct is the unit of pushdown, and each level answers only what it can prove.

| Level | Answers | Leaves to the next |
| --- | --- | --- |
| a listing | `&holder.*` attributes, free ones first, one stat at most | every row conjunct |
| a container's statistics | comparisons and null tests the minimum, maximum and null count settle | a straddling range, a function of a column |
| a media's partition columns | `partition_pairs`: the equalities `column = 'value'` and `column is null` pin, spelled as paths spell them | ranges and `in` lists, answered row by row |
| the rows | everything | nothing |

`Bound::partition_split` separates the conjuncts a partition layout answers from the residual; dropping a conjunct only widens what is kept, so a file is never wrongly discarded. [Holder attributes](holder.md) shows the listing and statistics levels; the [record options](../media/index.md#options) show the media level.

## Edges

- `Filter::new` of a term that is not a predicate -> binds with an error naming the boolean it wanted.
- `not (a = 1 or a = 2)` -> `not a in (1, 2)`; a null `a` stays unknown on both spellings.
- `apply_arrow_batch` keeping every row -> the input batch, columns pointer-identical; keeping some -> a copy, because a batch is dense.
- `apply_records` -> a kept row comes back canonical under the schema, in field order.
- `partition_pairs` -> `column = 'value'` in a conjunct at the top level only; `a = 1 or b = 2` pins nothing.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test expression -- arrow::grammar eval::grammar filter::grammar pushdown::grammar
    cargo test --features "iceberg internals parquet" -p yggdryl --test media -- options
    cargo bench -p yggdryl --bench expression -- expression_filter
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_expression.py -k "filter or unknown or statistics"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="filter|holder attribute" node/tests/expression.test.js
    ```
