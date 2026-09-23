# Selectors

`Selector` is a `select` clause: the columns published, computed, declared, or excluded, and the one owner of a schema declaration.

## Contract

| Key | Value |
| --- | --- |
| Owns | `Selector`, `Projection`, `BoundSelector`, `IntoSelector` |
| Projection | a term, an optional alias, an optional declared datatype with `null` / `not null`, optional `with (...)` metadata |
| `*` | every column; `* exclude (a, b)` every column but those |
| Declared column | a `create table` column: `id int64 not null` publishes `id` cast to `int64`, refusing a null; a declared cast is safe (a value that does not fit becomes null) unless the column is `not null` |
| Field | `from_field(field)` is lossless: each child becomes a declared column carrying its metadata and its `TRANSFORM:` derivation - a function over columns as `TRANSFORM:function` and `TRANSFORM:sources`, any other term as `TRANSFORM:expression`; `into_field(root)` writes the selector back as that declaration, so a `Field` is a plan holder |
| Identity | binding `select *`, a self alias, or a cast to the type a column has is skipped; the batch or reader is handed back as is |
| Apply | `apply_field(root)`, `apply_scalar(root, row)`, `apply_arrow_reader` (primary, streamed), `apply_arrow_batch`, `apply_arrow_array`, `apply_records` |
| Bindings | Python and JavaScript `Selector` and `BoundSelector`, the same names; a projection list can be text, terms, or `(term, alias)` pairs |

## Use

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::{DataType, Field, Selector, StructType};

    let root = DataType::from(StructType::from_fields([
        DataType::utf8().nullable_field("ccy"),
        DataType::Int64.nullable_field("size"),
    ])?)
    .required_field("rows");
    let selector: Selector = "ccy, size as quantity, size * 2 as doubled int32".parse()?;
    assert_eq!(selector.names(), ["ccy", "quantity", "doubled"]);

    // The published root, typed against the input root.
    let published = selector.apply_field(&root)?;
    assert_eq!(published.fields()[2].dtype(), &DataType::Int32);

    // One bind per stream; every batch runs the settled plan.
    let bound = selector.bind(&root)?;
    let batch = RecordBatch::try_new(
        root.clone().into_arrow_schema()?,
        vec![
            Arc::new(StringArray::from(vec!["EUR", "USD"])),
            Arc::new(Int64Array::from(vec![1_i64, 2])),
        ],
    )?;
    let projected = bound.apply_arrow_batch(&batch)?;
    assert_eq!(projected.schema().field(1).name(), "quantity");
    assert_eq!(projected.column(2).len(), 2);

    // `select *` changes nothing, so the batch is the caller's own.
    assert!(Selector::all().bind(&root)?.is_identity());

    // A field is a selector, and the selector is a field again: the one
    // that declares every column the field stores.
    let stored = selector.into_field(&root)?;
    assert_eq!(stored.fields()[2].get_metadata("TRANSFORM:expression"), Some("size * 2"));
    // A call over plain columns is stored as the function and its sources.
    let year: Selector = "year(event) as year".parse()?;
    let dated = DataType::from(StructType::from_fields([DataType::date32().required_field("event")])?).required_field("rows");
    let stored_year = year.into_field(&dated)?;
    assert_eq!(stored_year.fields()[0].get_metadata("TRANSFORM:function"), Some("year"));
    assert_eq!(stored_year.fields()[0].get_metadata("TRANSFORM:sources"), Some(r#"["event"]"#));
    let declared: Selector = "ccy utf8 null, size as quantity int64 null, size * 2 as doubled int32 null".parse()?;
    assert_eq!(Selector::from_field(&stored), declared);
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import DataType, Field, Selector, Term

    root = Field("rows", "struct<ccy:utf8,size:int64>", False)
    selector = Selector("ccy, size as quantity, size * 2 as doubled int32")
    assert selector.names == ["ccy", "quantity", "doubled"]
    # A projection list is also terms, text, or (term, alias) pairs.
    assert Selector(["ccy", (Term.column("size"), "quantity")]) == Selector("ccy, size as quantity")

    published = selector.apply_field(root)
    assert published.dtype["doubled"].dtype == DataType("int32")

    batch = pa.record_batch({"ccy": ["EUR", "USD"], "size": pa.array([1, 2], pa.int64())})
    projected = selector.apply_arrow_batch(batch)
    assert projected.schema.names == ["ccy", "quantity", "doubled"]
    assert projected.column("doubled").to_pylist() == [2, 4]
    assert Selector.all().bind(root).is_identity

    # Native rows go through the same bound plan, one bind for all of them.
    rows = selector.apply_records([{"ccy": "EUR", "size": 1}], root)
    assert list(rows) == [{"ccy": "EUR", "quantity": 1, "doubled": 2}]

    # A field is a selector, and the selector is a field again: the one
    # that declares every column the field stores.
    stored = selector.into_field(root)
    assert stored.dtype["doubled"].transform["expression"] == "size * 2"
    assert Selector.from_field(stored) == Selector(
        "ccy utf8 null, size as quantity int64 null, size * 2 as doubled int32 null"
    )
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { Field, Scalar, Selector } = require('yggdryl')

    const root = new Field('rows', 'struct<ccy:utf8,size:int64>', false)
    const selector = new Selector('ccy, size as quantity, size * 2 as doubled int32')
    assert.deepEqual(selector.names, ['ccy', 'quantity', 'doubled'])
    // A projection list is also projection texts.
    assert.ok(new Selector(['ccy', 'size as quantity']).equals('ccy, size as quantity'))

    const published = selector.applyField(root)
    assert.equal(String(published.dtype.getFieldAt(2).dtype), 'int32')

    const batch = new arrow.Table({
      ccy: arrow.vectorFromArray(['EUR', 'USD'], new arrow.Utf8()),
      size: arrow.vectorFromArray([1n, 2n], new arrow.Int64()),
    }).batches[0]
    const projected = selector.applyArrowBatch(batch)
    assert.deepEqual(projected.schema.fields.map((field) => field.name), ['ccy', 'quantity', 'doubled'])
    assert.deepEqual([...projected.getChild('doubled')], [2, 4])
    assert.equal(Selector.all().bind(root).isIdentity, true)

    // Native rows go through the same bound plan, one bind for all of them.
    const rows = selector.applyRecords([{ ccy: 'EUR', size: 1n }], root)
    assert.ok([...rows][0].equals(Scalar.from(['EUR', 1n, 2])))

    // A field is a selector, and the selector is a field again: the one
    // that declares every column the field stores.
    const stored = selector.intoField(root)
    assert.equal(stored.dtype.getFieldAt(2).transform.get('expression'), 'size * 2')
    assert.ok(
      Selector.fromField(stored).equals(
        'ccy utf8 null, size as quantity int64 null, size * 2 as doubled int32 null',
      ),
    )
    ```

## A selector declares a schema

A projection with a datatype is a `create table` column, and a `Selector` is what a plan's `create` section holds.

| Projection | Publishes |
| --- | --- |
| `price` | the column, unchanged |
| `price as amount` | the column under another name |
| `price * 2 as doubled` | the computed value, typed by the term |
| `price decimal(9,2)` | the column cast to `decimal(9,2)`, a value that does not fit becoming null |
| `price decimal(9,2) not null` | the same cast, a null or a value that does not fit refused naming `price` |
| `id int64 with (comment = 'key')` | the column with that metadata on its field |

`Selector::from_field` spells every child as `name dtype null|not null`, its metadata as `with (...)`, and its derivation as the term: a `TRANSFORM:function` over its `TRANSFORM:sources` - `year(event)`, `py.double(size)`, the shape a [user function's signature](functions.md) and a partition spec share - or a `TRANSFORM:expression` for any other term. `declared_field` and a plan's `create` section read the declaration back into a `Field`. The [transform protocol](../types/protocol.md) is where a stored field carries the derivation, beside the [partition](../holder/index.md#derived-partition-columns) declaration that is a transform of one source.

## Edges

- `select *` -> `is_all`; `* exclude (a)` -> not all, the column dropped when the schema has it and ignored when it does not.
- A self alias (`a as a`), a same-type cast -> dropped by `simplify` and skipped at bind.
- A declared column a value cannot fit -> null, unless `not null`, where the error names the column and the value.
- `apply_arrow_reader` -> the output schema is known before the first batch; `apply_arrow_batch` is the one-batch spelling and never collects a stream.
- `apply_records` with no schema and no record -> refused; with records, the first one's own datatype is the schema.
- `into_field` of a selector with a bare column -> no `TRANSFORM:` property, so `from_field` gives the bare column back.
- `into_field` of `lower(s) as name` -> `TRANSFORM:function = lower` and `TRANSFORM:sources = ["s"]`, never an expression; of `s || 'x'` or `size * 2` -> `TRANSFORM:expression`, because only a call over plain columns is a function over sources.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --test expression -- arrow::grammar selector::grammar transform::grammar
    cargo bench -p yggdryl --bench expression -- expression_identity
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_expression.py -k "selector or records_stream"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="selector|records stream" node/tests/expression.test.js
    ```
