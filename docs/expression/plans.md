# Plans

`Plan` is the sections of one read or write: what it creates, writes, selects, reads from, keeps, orders, and how many rows. `Expression::Sequence` runs plans one after another.

## Contract

| Key | Value |
| --- | --- |
| Owns | `Plan`, `Write`, `Verb`, `Ordering`, `Source`, `Target`, `Location`, `IntoPlan`, `Expression::Sequence`, `Holder::from_url` |
| Sections | `create [target] (schema) [with (...)]`, a write verb with an optional target and `by (keys)`, `select`, `from target \| (plan)`, `where`, `order by`, `limit`, `offset` |
| Verbs | `insert into` (append), `insert overwrite` (replace), `upsert into ... by (keys)` (merge), `delete from ... where`; every common alias reads and prints canonically |
| Location | a quoted URL, or a catalog path `catalog.schema.table` whose parts may be quoted with `"`, backticks, or `[...]`; parts resolve against a base URL, a URL stands alone |
| Target | a location and `with (name = 'value', ...)` properties: `media_type`, `codec`, `safe`, `batch_row_size`, `batch_byte_size`, `commit_row_size`, `max_row_size`, `max_byte_size`, and whatever a holder reads |
| Execute | `execute()` reads the source through its holder with the read sections pushed down, runs a nested plan first, and writes where the plan says; a plan with no source starts from the empty stream, which is what `create` alone needs |
| Apply | `apply_arrow_reader(reader)` shapes a stream it is given, source or not; `where` and `select` stream, `order by` collects, `offset` and `limit` slice views |
| Field | `Plan::from_field(field)` is `create name (columns)`; `field()` reads a `create` section back; `field_from(root)` types the read sections against a root |
| Bindings | Python and JavaScript `Plan` and `Expression`; `execute` and every application in all three |

## Use

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::expression::Plan;
    use yggdryl::local::Folder;
    use yggdryl::{DataType, Expression, Url};

    let root = Folder::temporary()?.path()?.join("yggdryl-docs-plans");
    std::fs::create_dir_all(&root)?;
    let url = Url::from_path(root.join("trades.arrow"))?;

    // `create` alone writes the declared schema and no rows.
    let created: Plan = format!("create '{url}' (id int64 not null, name utf8)").parse()?;
    assert_eq!(created.field()?.map(|field| field.field_len()), Some(2));
    created.execute()?.count();

    // A write shapes the stream it is given and sends it to its target.
    let schema = DataType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::utf8().nullable_field("name"),
    ])?
    .required_field("trades");
    let rows = RecordBatch::try_new(
        schema.into_arrow_schema()?,
        vec![
            Arc::new(Int64Array::from(vec![1_i64, 2, 3])),
            Arc::new(StringArray::from(vec!["a", "b", "c"])),
        ],
    )?;
    let insert: Plan = format!("insert into '{url}'").parse()?;
    insert.apply_arrow_reader(yggdryl::arrow::batch_reader(rows.schema(), [rows]))?.count();

    // A read pushes its sections into the holder and orders what comes back.
    let read: Plan = format!("select name from '{url}' where id > 1 order by id desc").parse()?;
    let names: Vec<String> = read
        .execute()?
        .map(|batch| {
            let batch = batch?;
            let column = batch.column(0).as_any().downcast_ref::<StringArray>().unwrap();
            Ok::<_, yggdryl::Error>(column.iter().map(|name| name.unwrap().to_owned()).collect::<Vec<_>>())
        })
        .collect::<yggdryl::Result<Vec<_>>>()?
        .concat();
    assert_eq!(names, ["c", "b"]);

    // Every alias prints its canonical verb, and a sequence is plans with `;`.
    let plan: Plan = "merge into t on (id) select id from s".parse()?;
    assert_eq!(plan.to_string(), "upsert into t by (id) select id from s");
    let steps: Expression = "delete from t where id = 1; select * from t".parse()?;
    assert_eq!(steps.steps().len(), 2);

    std::fs::remove_dir_all(&root)?;
    ```

=== "Python"

    ```python
    import pathlib
    import tempfile

    import pyarrow as pa

    from yggdryl import Expression, Plan

    url = (pathlib.Path(tempfile.mkdtemp()) / "trades.arrow").as_uri()

    # `create` alone writes the declared schema and no rows.
    created = Plan(f"create '{url}' (id int64 not null, name utf8)")
    assert created.field().name == "row"
    created.execute().read_all()

    # A write shapes the stream it is given and sends it to its target.
    rows = pa.record_batch({"id": pa.array([1, 2, 3], pa.int64()), "name": ["a", "b", "c"]})
    Plan(f"insert into '{url}'").apply_arrow_batch(rows)

    # A read pushes its sections into the holder and orders what comes back.
    read = Plan(f"select name from '{url}' where id > 1 order by id desc")
    assert read.execute().read_all().column("name").to_pylist() == ["c", "b"]

    # Every alias prints its canonical verb, and a sequence is plans with `;`.
    assert str(Plan("merge into t on (id) select id from s")) == "upsert into t by (id) select id from s"
    assert len(Expression("delete from t where id = 1; select * from t").steps) == 2

    # A plan is also built section by section.
    built = Plan().with_select("id, name").with_source("raw").with_filter("id > 1").with_limit(10)
    assert str(built) == "select id, name from raw where id > 1 limit 10"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const fs = require('node:fs')
    const os = require('node:os')
    const path = require('node:path')
    const { pathToFileURL } = require('node:url')
    const arrow = require('apache-arrow')
    const { Expression, Plan } = require('yggdryl')

    const root = fs.mkdtempSync(path.join(os.tmpdir(), 'yggdryl-docs-plans-'))
    const url = pathToFileURL(path.join(root, 'trades.arrow')).href

    // `create` alone writes the declared schema and no rows.
    const created = new Plan(`create '${url}' (id int64 not null, name utf8)`)
    assert.equal(created.field().name, 'row')
    created.execute().intoTable()

    // A write shapes the stream it is given and sends it to its target.
    const rows = new arrow.Table({
      id: arrow.vectorFromArray([1n, 2n, 3n], new arrow.Int64()),
      name: arrow.vectorFromArray(['a', 'b', 'c'], new arrow.Utf8()),
    }).batches[0]
    new Plan(`insert into '${url}'`).applyArrowBatch(rows)

    // A read pushes its sections into the holder and orders what comes back.
    const read = new Plan(`select name from '${url}' where id > 1 order by id desc`)
    assert.deepEqual([...read.execute().intoTable().getChild('name')], ['c', 'b'])

    // Every alias prints its canonical verb, and a sequence is plans with `;`.
    assert.equal(new Plan('merge into t on (id) select id from s').toString(), 'upsert into t by (id) select id from s')
    assert.equal(new Expression('delete from t where id = 1; select * from t').steps.length, 2)

    // A plan is also built section by section.
    const built = new Plan().withSelect('id, name').withSource('raw').withFilter('id > 1').withLimit(10)
    assert.equal(built.toString(), 'select id, name from raw where id > 1 limit 10')

    fs.rmSync(root, { recursive: true, force: true })
    ```

## Sections, in the order they run

| Section | Spelled | Runs |
| --- | --- | --- |
| `create` | `create [target] (id int64 not null, ...) [with (k = 'v')]` | declares the schema the stream is cast to, and the target a `create` alone writes |
| write | `insert into t`, `insert overwrite t`, `upsert into t by (id)`, `delete from t` | where the shaped stream goes; a verb with no target writes to the handle the plan is given to |
| `select` | `select id, price * 2 as doubled` | the [selector](selectors.md); `select *` when absent |
| `from` | `from t`, `from 'file:///lake/t.parquet'`, `from (select ... )` | what `execute` reads, a nested plan running first |
| `where` | `where price > 0` | the [filter](filters.md), pushed into the read |
| `order by` | `order by id desc nulls first, ccy` | collects the stream and sorts it; a key may name a column the projection drops, or an alias it publishes |
| `limit`, `offset` | `limit 10 offset 5` | slice views over the stream; pushed into the read when nothing orders |

`read_sections()` is the plan without its `create`, write and `from`: what a media is handed to push down. A `Plan` with only a `select` or a `where` collapses into that clause through `into_expression`.

## Locations and targets

| Spelling | Location |
| --- | --- |
| `'file:///lake/trades.parquet'` | a URL, held as it is |
| `lake.raw.trades` | the parts `lake`, `raw`, `trades`, joined onto the base URL a media resolves them against |
| `catalog."my schema".[tbl.x].` `` `odd-one` `` | the same parts, each quoted a way an engine quotes it |
| `t with (media_type = 'application/vnd.apache.arrow.stream', batch_row_size = '1024')` | a target with properties: the holder is opened with them, and the read or write knobs are read from them |

`Holder::from_url(url, properties)` is the one builder every target goes through: a `file:` URL is a local path or, with a fragment, a ZIP member; an object-store scheme needs the `object` feature and reads its credentials from the properties; `media_type` and `codec` type an extensionless resource.

## Verbs and their aliases

| Canonical | Also read as | Does |
| --- | --- | --- |
| `insert into t` | `insert t`, `append to t`, `append into t`, `append t` | adds the rows after the stored ones |
| `insert overwrite t` | `insert overwrite into t`, `overwrite t`, `overwrite into t`, `replace t`, `replace into t` | replaces the stored rows |
| `upsert into t by (id)` | `upsert t by (id)`, `merge into t on (id)`, `merge t by (id)` | replaces the stored rows the keys match, adds the rest |
| `delete from t where ...` | `delete t where ...` | removes the stored rows the predicate keeps; the stream it is given is ignored |

## Edges

- A plan with no `from` -> `execute` starts from the empty stream, so `create` alone writes a schema and `delete` reads its target itself.
- A store that is not there yet -> reads as the empty stream, which is what lets the first `insert into` it be the same plan as the rest.
- `apply_arrow_reader` on a plan with a `from` -> shapes the stream it is given; the source is only what `execute` reads.
- `order by` -> the one section that collects; `limit` after it counts sorted rows and stays with it rather than being pushed down.
- `create (...)` with no target over a stream -> casts the stream to the declared schema under the selector's rule; a `not null` column the stream cannot fill is refused naming it.
- `from (plan)` nested past the shared depth limit -> refused at parse.
- A section word as a bare location -> refused; a URL stands alone only quoted.
- `Plan::from_field` -> spells every column with its nullability, so `create trades (id int64 not null, ccy utf8 null)`.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib expression::plan::tests
    cargo test --features "parquet iceberg" -p yggdryl --lib -- expression::plan::tests::streams::a_plan_creates_inserts_upserts_deletes_and_reads_a_store expression::plan::tests::streams::a_sliced_reader_walks_batch_boundaries_without_copying_whole_batches expression::plan::tests::streams::a_holder_is_built_from_a_url_and_properties
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/expression -k "plan or field_is_a_plan or whichever_clause"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="plan|whichever clause" node/tests/expression
    ```
