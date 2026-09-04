# Expressions

`yggdryl::Expression` is the one way this project says *which rows* and *which values*. It is a
recursive tree, it types itself against a schema, it compiles once, and it answers three ways: row at
a time over a [`Scalar`](text.md), vectorized over an Arrow batch, and three-valued over container
statistics so a file, a manifest, or a directory is skipped without being read.

Before it existed the same question was asked five times in the weakest language available - a
`(column, value)` text pair - and none of those spellings could express a range, a null test, a
nested path, or a question about the *file* rather than the rows. The pairs are still there, as
sugar; there is no second implementation behind them.

## An expression is not a value

A [`Scalar`](text.md) is the codec's lossless value tree: structural, serializable, meaningful on its
own. An `Expression` is a computation whose meaning depends on a schema. They meet at exactly two
points - a literal going in, and evaluation producing a value coming out - and keeping them apart is
what lets `Scalar` stay plain data while an expression carries schema-dependent meaning.

## The four stages

```text
text ──parse──▶ Expression ──bind(schema)──▶ Bound ──▶ Scalar | ArrayRef | mask
```

1. **Parse.** One recursive grammar, re-entered by every nested construct, with byte-positioned
   errors and a hard nesting limit shared with the schema grammar.
2. **Type.** The output `Field` resolved against a schema, recursively. This is the only place output
   types are decided; everything else asks.
3. **Bind.** Names become indices, parameters are substituted, literals are converted once into the
   type they are compared against, constant subtrees are folded by evaluating them, and the operands
   of every `and` are ordered cheapest-first. This happens **once per stream**, never per batch and
   never per row.
4. **Evaluate.** The one bound tree answers rows, batches, and statistics.

=== "Rust"

    ```rust
    use std::str::FromStr;

    use yggdryl::{Expression, Field, Scalar};

    let schema = Field::from_str("trades:struct<ccy:utf8,price:decimal(9,2),size:bigint>")?;
    let filter: Expression = "ccy = 'EUR' and price > 100".parse()?;

    // Text is the canonical form, and it re-parses to the same tree.
    assert_eq!(filter.to_string(), "ccy = 'EUR' and price > 100");
    assert_eq!(filter.columns(), vec!["ccy".to_owned(), "price".to_owned()]);

    let bound = filter.bind(&schema)?;
    // The literal was converted once, into the column's own exact type.
    assert_eq!(
        bound.expression().to_string(),
        "ccy = 'EUR' and price > decimal128(9,2) '100.00'",
    );

    let row = Scalar::from_sequence([
        Scalar::from("EUR"),
        Scalar::D128(15_000, 2),
        Scalar::I64(5),
    ]);
    assert!(bound.matches(&row)?);
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Expression, Field

    schema = Field("trades", "struct<ccy:utf8,price:decimal(9,2),size:bigint>", False)
    filter = Expression("ccy = 'EUR' and price > 100")

    assert str(filter) == "ccy = 'EUR' and price > 100"
    assert filter.columns() == ["ccy", "price"]

    bound = filter.bind(schema)
    assert str(bound.expression) == "ccy = 'EUR' and price > decimal128(9,2) '100.00'"

    # A row is a sequence in schema order, or a mapping of column to value.
    # The price is a `Decimal`, because the column is exact and so is the
    # comparison: a float here would be a different number.
    assert bound.matches(["EUR", Decimal("150.00"), 5])
    assert bound.matches({"ccy": "EUR", "price": Decimal("150.00"), "size": 5})
    assert not bound.matches({"ccy": "USD", "price": Decimal("150.00"), "size": 5})
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Expression, Field, Scalar } = require('yggdryl')

    const schema = new Field('trades', 'struct<ccy:utf8,price:decimal(9,2),size:bigint>', false)
    const filter = new Expression("ccy = 'EUR' and price > 100")

    assert.equal(filter.toString(), "ccy = 'EUR' and price > 100")
    assert.deepEqual(filter.columns, ['ccy', 'price'])

    const bound = filter.bind(schema)
    assert.equal(
      bound.expression.toString(),
      "ccy = 'EUR' and price > decimal128(9,2) '100.00'",
    )

    // The price is an exact decimal, because the column is exact and so is
    // the comparison: a JavaScript number here would be a different one.
    const price = Scalar.decimal(15000n, 2)
    assert.equal(bound.matches(Scalar.fromJs(['EUR', price, 5])), true)
    assert.equal(bound.matches(Scalar.fromJs(['USD', price, 5])), false)
    ```

## Text round-trips

`Display` is the inverse of `FromStr`, and a property test asserts it for every expression the module
can build. That is what lets a predicate cross a process boundary as text - into a log line, a
manifest property, an HTTP query - and come back the same expression.

Two rules make the inverse hold. Parentheses are emitted from precedence rather than from the input,
so `(a and b) and c` and `a and (b and c)` are one value and print one way. And a literal prints its
type whenever the bare spelling would not recover it: bare integer text is `int64`, bare float text is
`float64`, bare quoted text is `utf8`, and everything else prints as `<datatype> '<text>'`.

```rust
use yggdryl::Expression;

for text in [
    "x is not null",
    "x is distinct from y",
    "x in (1, 2, 3)",
    "x between 1 and 10",
    "name like 'a%' escape '\\'",
    "path glob '**/*.parquet'",
    "case when a then 1 else 2 end",
    "trade.legs[0]['ccy'] = 'EUR'",
    "try_cast(x as decimal128(9,2)) > decimal128(9,2) '1.50'",
] {
    let parsed: Expression = text.parse()?;
    assert_eq!(parsed.to_string().parse::<Expression>()?, parsed);
}
```

There is also a structural form - the tagged JSON document `DataType` and `Field` already cross a
wire as - for consumers that want to *walk* a predicate rather than parse one.

## Null is unknown

Predicates are three-valued. `and` is false when any operand is false even if another is unknown;
`or` is true when any operand is true even if another is unknown; `not unknown` is unknown. A
comparison with a null operand is unknown.

A row is kept when the predicate is **true**, so unknown filters the row out - and that is a separate
decision from what the predicate evaluated to.

```rust
use yggdryl::{Expression, Field, Scalar};

let schema: Field = "rows:struct<a:bigint>".parse()?;
let bound = "a > 1".parse::<Expression>()?.bind(&schema)?;

let missing = Scalar::from_sequence([Scalar::Null]);
assert_eq!(bound.eval(&missing)?, Scalar::Null); // the answer is unknown
assert!(!bound.matches(&missing)?); // and unknown does not keep the row
```

`is distinct from` and `is not distinct from` are the two comparisons that answer *about* a null
rather than through it, so they are two-valued and never produce unknown.

## `&holder.*`: asking about the file

A predicate over a lake asks two different kinds of question. `ccy = 'EUR'` is about the rows.
`&holder.size > 0` and `&holder.partition['year'] = '2024'` are about the *container*, and answering
those can skip the decode entirely. They live in one grammar because a reader that has to run two
filters in two languages ends up running the expensive one first.

Every attribute declares what it costs:

| Cost | Attributes | What it takes |
| --- | --- | --- |
| free | `url`, `path`, `name`, `stem`, `extension`, `scheme`, `parent`, `depth`, `mime_type`, `partition['column']` | the identifier alone |
| stat | `size`, `kind`, `is_container`, `is_empty` | one call into the backing store |

`bind` sorts a conjunction cheapest-first and evaluation stops at the first `false`, so a
free-attribute conjunct that answers `false` costs exactly zero backend calls. A cost class may be
over-stated without harming correctness - a selector ordered later is still answered - so an
attribute whose price depends on the backend is classified by its worst case.

=== "Rust"

    ```rust
    use yggdryl::io::IOBase;
    use yggdryl::local::Folder;
    use yggdryl::Expression;

    let lake = Folder::new(std::env::temp_dir().join("yggdryl-docs-lake"))?;
    std::fs::create_dir_all(lake.path()?.join("year=2024"))?;
    std::fs::write(lake.path()?.join("year=2024").join("part-0.parquet"), b"")?;
    std::fs::create_dir_all(lake.path()?.join("year=2025"))?;
    std::fs::write(lake.path()?.join("year=2025").join("part-0.parquet"), b"")?;

    let filter: Expression = "&holder.partition['year'] = '2024'".parse()?;
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

A conjunct that reads a row column cannot be answered by a listing, so it is dropped rather than
guessed at. `children_matching` may therefore keep a file the rows will later discard, and can never
discard a file the rows would have kept.

## Pruning: answering without reading

Two questions, both conservative, both answered from the same bound tree.

**Can any row in this container match?** `Bound::statistics_prune` answers it from per-column
minimums, maximums, and null counts - the statistics a Parquet footer, an Iceberg manifest, and a Hive
path all carry in some form. It only ever answers `false` when it can *prove* that no row matches;
everything it cannot prove is a `true` that costs one read.

```rust
use yggdryl::expression::Bounds;
use yggdryl::{Expression, Field, Scalar};

let schema: Field = "trades:struct<ccy:utf8,size:bigint>".parse()?;
let bounds = Bounds::new(Some(1_000))
    .with_column("ccy", Some(Scalar::from("EUR")), Some(Scalar::from("USD")), Some(0))
    .with_column("size", Some(Scalar::I64(1)), Some(Scalar::I64(99)), Some(4));

// Provably empty: no row can hold a size above the file's maximum.
assert!(!"size > 1000".parse::<Expression>()?.bind(&schema)?.statistics_prune(&bounds));
// Not provable either way: the range overlaps, so the file is read.
assert!("size > 50".parse::<Expression>()?.bind(&schema)?.statistics_prune(&bounds));
// A null test the count settles outright.
assert!("size is null".parse::<Expression>()?.bind(&schema)?.statistics_prune(&bounds));
```

A Hive path is expressed as the statistic it is - the minimum equal to the maximum, nothing null - so
one rule serves a footer, a manifest, and a directory name.

**Which part of this predicate can a layout answer?** `Bound::partition_split` splits the conjunction
into the part that reads only partition columns and holder attributes, and the residual that does
not. The first prunes the listing; the second runs over the rows that survive. Splitting a
conjunction is sound because dropping conjuncts only ever widens what is kept.

```rust
use yggdryl::{Expression, Field};

let mut schema: Field = "trades:struct<year:int32,price:decimal(9,2)>".parse()?;
let mut children = schema.fields().to_vec();
children[0].set_partition(true);
schema.set_dtype(yggdryl::DataType::from_fields(children)?)?;

let bound = "year = 2024 and price > 100".parse::<Expression>()?.bind(&schema)?;
let residual = bound.partition_split();
assert_eq!(residual.answerable().to_string(), "year = int32 '2024'");
assert_eq!(residual.remaining().to_string(), "price > decimal128(9,2) '100.00'");
assert!(!residual.is_complete());
```

## Vectorized, and zero-copy where the shape allows

The Arrow tier is an optimization of the row tier, never a second definition of it. Comparisons reach
`arrow-ord`'s kernels, null tests read the validity buffer directly, and `and`/`or`/`not` are
three-valued buffer arithmetic. Everything else runs the row evaluator and gathers the answers, which
is slower and *cannot disagree*; a property test asserts the equality on every operator, nulls and
`nan` included. A cast between text and a temporal is the one kernel with a reading in front of it:
the column reads and spells through the same code a row does, and Arrow's kernel answers only the
spellings this crate cannot read at all, which are the spellings a row refuses.

A mask that keeps every row hands the input batch straight back, so its columns stay
pointer-identical. A mask that keeps some rows must copy, because a `RecordBatch` is a dense
representation and there is no way to say "these rows" without moving them. A projection reorders
`ArrayRef`s and never touches a buffer.

Measured over 65,536 rows against the raw kernel call written by hand, the expression path is within
a few percent for every predicate family, and parse and bind together cost single-digit microseconds
once per stream. The numbers, and how to reproduce them, are [below](#against-the-raw-arrow-kernels).

## Bind a whole statement once

`Statement::bind` resolves projections, predicate, ordering, parameters, and output field against one
struct `Field`. The resulting `BoundStatement` exposes that resolved plan without reparsing it.
`ordering` reports each expression with its direction and optional null placement; `is_all` is true
only for an unfiltered, unordered, unlimited `select *`.

Rust uses `bind_with(&field, &[(name, Scalar)])`. Python accepts a mapping in
`statement.bind(field, parameters=None)`, and JavaScript accepts a native `Scalar` record or an ordinary
object in `statement.bind(fieldLike, parameters?)`; both redirect to the same core binder.

Arrow projection is streamed where the holder permits it. `project_reader` wraps a `BatchReader`,
applies the predicate and projection batch by batch, and enforces one limit across the stream. Python's
`project_arrow` and JavaScript's `projectArrow` preserve a record batch, table, or reader input; their
spelled-out methods make the return type explicit. Global ordering cannot be correct without seeing
all rows, so `sort` / `sort_arrow_batch` / `sortArrowBatch` intentionally sort one
materialized batch.

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

## Targets own their application

`Bound` does not grow one method per thing an expression can run over. The `ApplyExpression` trait
inverts that ownership: the *target* says what applying an expression to it produces and how, as an
associated type, because a target has exactly one natural application. `Bound` keeps only the verbs
that mean more than "apply" - `matches` is apply-then-require-boolean, `filter` is
apply-then-select - and each is a short composition over the trait. The four tiers above are its
four implementations:

| Target | Applying an expression produces |
| --- | --- |
| one row (`Scalar`) | the `Scalar` the expression computes |
| one holder (`dyn Attributes`) | what the holder alone settles, three-valued |
| one Arrow `RecordBatch` | one column of answers, an `ArrayRef` |
| one container's statistics (`Bounds`) | the `Option<bool>` certainty pruning runs on |

A `BatchReader` cannot be applied through `&self` - a stream is wrapped, not borrowed - so
`ApplyExpressionStream` is the consuming sibling. Its output is deliberately the *filtering* reader
rather than a stream of evaluated columns: a stream cannot hand back its answers without draining
itself, but it can yield only the rows the predicate keeps, one batch at a time. Selection is the
only application a stream can make lazily, so apply-then-select is what applying an expression to a
stream *means*.

```rust
use std::sync::Arc;

use arrow_array::{Int64Array, RecordBatch, StringArray};
use yggdryl::expression::{ApplyExpression, ApplyExpressionStream, Attributes, Bounds};
use yggdryl::{Expression, Field, Url, Scalar};

// Non-nullable at the root, because the batch below projects it to Arrow.
let schema = "trades:struct<ccy:utf8,size:bigint>".parse::<Field>()?.with_nullable(false);
let bound = "ccy = 'EUR' and size > 10".parse::<Expression>()?.bind(&schema)?;

// One row applies to the value the expression computes.
let row = Scalar::from_sequence([Scalar::from("EUR"), Scalar::I64(25)]);
assert_eq!(row.apply_expression(&bound)?, Scalar::Bool(true));

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
    .with_column("size", Some(Scalar::I64(1)), Some(Scalar::I64(5)), Some(0));
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

The point of the inversion is who can join. A new target implements the trait beside its own type -
a listing entry, a cache, a foreign table - and `expression/` never learns it exists; if an
implementation ever needed a line inside that module beyond a `use`, the trait would be shaped
wrong. The claim is proven by a test rather than asserted: `rust/src/io/tests.rs` defines a
`Listing` target outside the module, reaching it through the public surface alone.

## Iceberg: one predicate, every level of the metadata

A table's scan is planned by the same expression that filters its rows. Each level of the chain
answers it from the statistics it carries: a manifest-list summary, then a manifest entry's partition
tuple and column bounds. What none of them settles is left for the rows, and a conjunct that the
partition tuple *proves* is dropped rather than re-tested.

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
    from yggdryl.iceberg import Table

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

## The grammar

```text
statement   := "select" projections ["where" expr] ["order" "by" orders] ["limit" n]
projections := "*" | (expr ["as" identifier]) ("," ...)*
orders      := (expr ["asc" | "desc"] ["nulls" ("first" | "last")]) ("," ...)*

expr        := disjunction
disjunction := conjunction ("or" conjunction)*
conjunction := negation ("and" negation)*
negation    := "not" negation | predicate
predicate   := additive [ comparison | "is" .. | "in" .. | "between" .. | "like" .. | "glob" .. ]
additive    := product (("+" | "-") product)*
product     := unary (("*" | "/" | "%") unary)*
unary       := "-" unary | accessor
accessor    := atom ("." identifier | "[" key "]")*
atom        := literal | "(" expr ")" | column | "&holder." selector | ":" parameter
             | "cast" "(" expr "as" datatype ")" | "try_cast" "(" .. ")"
             | "case" ("when" expr "then" expr)+ ["else" expr] "end"
             | function "(" expr,* ")" | "[" expr,* "]" | "{" expr ":" expr,* "}"
             | "struct" "(" expr "as" identifier,* ")" | datatype ("'text'" | "null")
```

| Shape | Spelling |
| --- | --- |
| comparison | `=`, `<>` (or `!=`), `<`, `<=`, `>`, `>=` |
| distinctness | `is distinct from`, `is not distinct from` |
| null | `is null`, `is not null` |
| membership | `x in (a, b)`, `x not in (a, b)` |
| range | `x between low and high`, `x not between low and high` |
| pattern | `x like 'a%'`, `x ilike 'A%'`, `x like 'a!%' escape '!'`, `x glob '**/*.parquet'` |
| path | `a.b`, `a[0]`, `a[-1]`, `a['key']` |
| identifier | `name`, `"odd name"`, `` `odd name` `` |
| literal | `1`, `1.5`, `'text'`, `true`, `null`, `decimal128(9,2) '1.50'`, `date32 '2024-01-01'`, `utf8 null` |
| constructor | `[1, 2]` a list, `{'k': 1}` a map, `struct(1 as a)` a struct |
| conditional | `case when c then v else w end` |
| conversion | `cast(x as int32)`, `try_cast(x as int32)` |
| attribute | `&holder.size`, `&holder.partition['year']` |
| parameter | `:since` |
| comment | `-- to end of line` |

The functions are a closed set: `lower`, `upper`, `length`, `substring`, `trim`, `starts_with`,
`ends_with`, `contains`, `concat`, `year`, `month`, `day`, `hour`, `truncate`, `coalesce`, `if_null`,
`size`, `get`. Closed deliberately - an open registry is a plugin system, and a plugin system cannot
promise that the three evaluators agree about a function none of them knows.

## Decisions

The design of this layer was settled against Iceberg's bound/unbound split, Substrait's reference
model, Arrow and DataFusion's coercion rules, and the SQL semantics DuckDB, Spark, Calcite, and Polars
ship. What follows is what this grammar accepts, what it refuses, and what it keeps free.

### Accepted, and why it is spelled that way

- **Two types, not one flag.** `Expression` is name-based and serializable; `Bound` is
  schema-resolved and is not. "Partially bound" is unrepresentable rather than detected at runtime.
- **One unbound filter, many bindings.** The same expression binds against a data schema, a partition
  schema, and a listing, producing three bound trees. That is what makes one predicate answer a lake,
  a table, and a batch.
- **Kleene logic, evaluated.** Three values, with `false and unknown` = false and
  `true or unknown` = true. A filter keeps a row only when the answer is exactly true.
- **`is distinct from` is two-valued.** It is the operator that answers *about* a null.
- **0-based indices, negative from the end, out of range is null.** `a[0]` is first and `a[-1]` is
  last. Absence is not a failure anywhere else on this crate's read path, and a scan must not die on
  one ragged row.
- **`substring` is 1-based, with the standard's window.** `substring(s, 0, 5)` is four characters:
  the window `[start, start + length)` intersected with the characters that exist. A negative start
  counts back from the end before the window applies.
- **Text is compared by code point, with no collation.** Every statistics bound in every format is in
  byte order, and a collation would silently invalidate all pruning.
- **A struct child reached by a missing name is a bind error; a missing map key is null.** Only the
  nominal container has a plan-time contract, so a typo and a sparse row must not look alike.
- **Names resolve ASCII case-insensitively, and a genuine collision is an error.** First-match-wins is
  the one resolution rule nobody can debug.
- **Floats compare by IEEE 754 totalOrder.** `nan` equals `nan` and sorts above everything, and
  `-0.0` sorts below `+0.0`. This is Arrow's own predicate: the two tiers agreeing with *each other*
  matters more than either agreeing with another engine, because a disagreement between them is a
  correctness bug and a disagreement with DuckDB is a documented difference.
- **A decimal never silently becomes a float.** Exact decimals and approximate floats do not share
  an implicit arithmetic type; an explicit cast is required. `f64` holds about 16 decimal digits
  against `decimal(38, s)`'s 38, and an implicit loss would be invisible in the plan.
- **An exact quotient keeps room to be one.** Division uses at least six fractional places, because at
  the operands' own scale `1.00 / 3.00` would be `0.33` - a rounding, not a division. If the quotient
  is still inexact at the declared scale, evaluation returns the core inexact-arithmetic error.
- **Null propagates; failed arithmetic is an error.** A null operand produces null. Overflow,
  division by zero, an inexact quotient, and an undefined operand pair remain distinct core errors
  through `Bound::eval`; none is silently rewritten as unknown. `try_cast` is the explicit spelling
  for a caller who wants a failed conversion to become null, while `cast` refuses it.

### Deliberately refused

| Refused | Because |
| --- | --- |
| subqueries, joins, aggregates, windows | each needs a second relation; this is a filter and projection tree over one |
| regular expressions (`~`, `rlike`, `similar to`) | a regex engine is a dependency decision, not an expression decision, and this workspace adds no dependency |
| `element_at` | several engines ship one and disagree about whether its index is 0-based or 1-based; the operation is spelled `get` |
| implicit string-to-number coercion | `'2' > 1` is a bind error naming the cast that would fix it; a coercion whose direction depends on syntactic position is a bug generator |
| a per-row `like` pattern | a pattern that changes per row is a different operation, and pretending otherwise makes the vectorized tier silently slower than the scalar one |
| `[ident]` bracket-quoted identifiers | the brackets are already list construction and list indexing |
| `\|\|` as `or`, `&&` as `and` | one operator, one meaning |
| a session timezone | it would make an expression's meaning depend on who evaluates it |
| calendar units in `truncate` | a month is not a fixed length; read one with `year()` or `month()` |

### Reserved

Parsed as an error today, with the syntax kept free so it can be added without a breaking change:
slices `[a:b]`; integer division `//`; `date_diff`, `concat_ws`, `strip_prefix`, `strip_suffix`;
the JSON operators `->` and `->>`; hexadecimal and digit-separator literals; grapheme-aware length;
Parquet row-group pruning through the same `Bounds` the other three containers use.

## Limits

Nesting is bounded by the same hard limit the schema grammar uses, and an expression may hold at most
100,000 nodes. Depth alone does not bound work - a flat `in` list of a million literals is one level
deep - so both are checked once, before any walk, and a walk never has to check.

## Against the raw Arrow kernels

`benchmarks/expression.rs` carries the raw `arrow-ord` / `arrow-select` call as its baseline: the
same predicate written by hand against the kernels, with no expression involved. The
`expression_mask` group and the `kernel_mask` group share their case IDs, so the two are read side
by side and the gap between them is the price of the grammar.

Indicative numbers from one containerized x86_64 Linux run, 65,536 rows,
`--measurement-time 1`:

```text
                       expression   kernel
utf8_equality             181.5 us  177.2 us
int64_range                41.1 us   36.9 us
decimal_range              48.6 us   51.0 us
set_membership            347.3 us  349.1 us
conjunction               277.6 us  271.4 us
```

Parsing and binding are the other half, and they happen once per stream rather than once per batch:
`expression_parse` runs 0.5-2.4 us per predicate and `expression_bind` 0.5-4.5 us. A batch of 65,536
rows therefore pays the grammar back in the first few microseconds of the first batch.
