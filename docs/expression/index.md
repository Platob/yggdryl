# Expression

`yggdryl::Expression` is what one piece of expression text is: a `select` clause, a `where` clause, a plan with sections, or a `;`-separated sequence of plans - all over one term tree, typed against a schema and compiled once.

## Pages

| Page | Purpose |
| --- | --- |
| [Expression](index.md) | This page: the layers, the pipeline, the best-effort rule |
| [Grammar](grammar.md) | The term and plan grammars, the closed function set, the decisions |
| [Terms](terms.md) | `Term` and `Bound`: literals, binding, parameters, simplification, `explain` |
| [Selectors](selectors.md) | `Selector`: projections, declared columns, transforms, a field as a plan |
| [Filters](filters.md) | `Filter`: predicates, pushdown normalization, three-valued logic |
| [Functions](functions.md) | The closed function set's one door: registered user functions, their signature as a field, the Python decorators |
| [Plans](plans.md) | `Plan`: sections, write verbs, locations, nested sources, sequences, `execute` |
| [Evaluate](evaluate.md) | The streamed Arrow tier, native records, statistics pruning, Iceberg scan planning |

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `Term`, `Bound`, `Filter`, `Selector`, `BoundSelector`, `Plan`, `Expression`, `Records`; `(column, value)` pairs are sugar, not a second implementation |
| Layers | a `Term` is a tree, a `Filter` is one predicate, a `Selector` is a projection list, a `Plan` is the sections of one read or write, an `Expression` is whichever of those the text is |
| Not a value | [`Scalar`](../types/scalar.md) is plain data; a term needs a schema to mean anything |
| Pipeline | parse -> type -> simplify -> bind(schema) -> apply; bind runs once per stream, never per batch or row |
| Reader first | `apply_arrow_reader` is the primary application and never collects; `apply_arrow_batch` is the one-batch spelling of it, and a batch that is returned unchanged is the caller's own |
| Best effort | a constant coerces into the operand it meets, operands with no common type compare as text, a declared column casts safely unless it is `not null`; an error names what could not be done |
| Canonical text | `Display` inverts `FromStr` for every layer, asserted by a corpus test |
| Structural form | Tagged JSON document, as `DataType` and `Field` |
| Null | Three-valued; a filter keeps a row only when the answer is exactly `true` |
| Errors | Byte-positioned parse errors; nesting and node limits shared with the schema grammar |
| Bindings | every layer in Python and JavaScript; `Bounds` statistics are Rust and Python |

## Use

Parse, bind once, ask a row.

=== "Rust"

    ```rust
    use std::str::FromStr;

    use yggdryl::expression::Term;
    use yggdryl::{Field, Filter, Scalar};

    let schema = Field::from_str("trades:struct<ccy:utf8,price:decimal(9,2),size:bigint>")?;
    let filter: Filter = "ccy = 'EUR' and price > 100".parse()?;

    // Text is the canonical form, and it re-parses to the same tree.
    assert_eq!(filter.to_string(), "ccy = 'EUR' and price > 100");
    assert_eq!(filter.columns(), vec!["ccy".to_owned(), "price".to_owned()]);

    let bound = filter.bind(&schema)?;
    // The literal was converted once, into the column's own exact type.
    assert_eq!(
        bound.term().to_string(),
        "ccy = 'EUR' and price > decimal32(9,2) '100.00'",
    );

    let row = Scalar::from_sequence([
        Scalar::from("EUR"),
        Scalar::d128(15_000, 2),
        Scalar::from(5_i64),
    ]);
    assert!(bound.matches(&row)?);

    // A null price makes the answer unknown, and unknown does not keep the row.
    let missing = Scalar::from_sequence([Scalar::from("EUR"), Scalar::Null, Scalar::from(5_i64)]);
    assert_eq!(bound.eval(&missing)?, Scalar::Null);
    assert!(!bound.matches(&missing)?);

    // A term is the tree a filter is one predicate over.
    let term: Term = "price > 100".parse()?;
    assert_eq!(Filter::new(term).to_string(), "price > 100");
    ```

=== "Python"

    ```python
    from decimal import Decimal

    from yggdryl import Field, Filter, Term

    schema = Field("trades", "struct<ccy:utf8,price:decimal(9,2),size:bigint>", False)
    filter = Filter("ccy = 'EUR' and price > 100")

    assert str(filter) == "ccy = 'EUR' and price > 100"
    assert filter.columns() == ["ccy", "price"]

    bound = filter.bind(schema)
    assert str(bound.term) == "ccy = 'EUR' and price > decimal32(9,2) '100.00'"

    # A row is a sequence in schema order, or a mapping of column to value.
    # The price is a `Decimal`, because the column is exact and so is the
    # comparison: a float here would be a different number.
    assert bound.matches(["EUR", Decimal("150.00"), 5])
    assert bound.matches({"ccy": "EUR", "price": Decimal("150.00"), "size": 5})
    assert not bound.matches({"ccy": "USD", "price": Decimal("150.00"), "size": 5})

    # A null price makes the answer unknown, and unknown does not keep the row.
    assert not bound.matches({"ccy": "EUR", "price": None, "size": 5})

    # A term is the tree a filter is one predicate over.
    assert str(Filter(Term("price > 100"))) == "price > 100"
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, Filter, Scalar, Term } = require('yggdryl')

    const schema = new Field('trades', 'struct<ccy:utf8,price:decimal(9,2),size:bigint>', false)
    const filter = new Filter("ccy = 'EUR' and price > 100")

    assert.equal(filter.toString(), "ccy = 'EUR' and price > 100")
    assert.deepEqual(filter.columns, ['ccy', 'price'])

    const bound = filter.bind(schema)
    assert.equal(
      bound.term.toString(),
      "ccy = 'EUR' and price > decimal32(9,2) '100.00'",
    )

    // The price is an exact decimal, because the column is exact and so is
    // the comparison: a JavaScript number here would be a different one.
    const price = Scalar.decimal(15000n, 2)
    assert.equal(bound.matches(Scalar.from(['EUR', price, 5])), true)
    assert.equal(bound.matches(Scalar.from(['USD', price, 5])), false)

    // A null price makes the answer unknown, and unknown does not keep the row.
    assert.equal(bound.matches(Scalar.from(['EUR', null, 5])), false)

    // A term is the tree a filter is one predicate over.
    assert.equal(new Filter(new Term('price > 100')).toString(), 'price > 100')
    ```

## The layers

```text
text ──parse──▶ Expression ─┬─ Selector ──bind──▶ BoundSelector ──▶ batch | row | Records
                            ├─ Filter   ──bind──▶ Bound         ──▶ mask  | bool | Records
                            ├─ Plan     ──execute / apply──▶ BatchReader
                            └─ Sequence ──apply, step by step──▶ BatchReader
Term ──bind(schema)──▶ Bound ──▶ Scalar | ArrayRef | certainty
```

| Layer | Is | Spelled |
| --- | --- | --- |
| `Term` | one tree: a column, a constant, a comparison, a function | `price > 100` |
| `Filter` | one predicate over rows | `where price > 100` (the keyword optional on its own) |
| `Selector` | the columns published, computed, declared, or excluded | `select id, price * 2 as doubled int64` |
| `Plan` | the sections of one read or write | `upsert into t by (id) select id from s where price > 0 limit 10` |
| `Expression` | whichever of those the text is, or a `;`-separated sequence | `where id > 1; select name` |
| `Records` | native rows streaming out of any of them | `selector.apply_records(schema, rows)` |

A lone `select` or `where` parses as its clause, not as a one-section plan, so the same text means the same thing whichever type it is read into; `Plan::into_expression` collapses the other way.

## Four stages

| Stage | Does |
| --- | --- |
| Parse | One recursive grammar; byte-positioned errors |
| Type | Output `Field` resolved against the schema; decided here only |
| Simplify | The same answer in fewer nodes: `a = 1 or a = 2` is `a in (1, 2)`, a self alias is dropped, a same-type cast is skipped |
| Bind | Names to indices, parameters, literals converted once, constants folded, `and` cheapest-first |
| Apply | One bound tree answers rows, batches, readers, statistics, and prints its own plan with `explain` |

## Best effort, then a named refusal

The rule every layer follows: do what the text asks whenever one reading does it, and refuse only what no reading can, naming it.

| Situation | Answer |
| --- | --- |
| `i = '1'` on an `int64` column | the constant is read as the column's type: `i = 1` |
| `t > '2023-01-01'` on a datetime column | the text parses as the column's temporal; it stays a datetime comparison |
| `s > 1` on a `utf8` column | no common type, so both sides compare as text: `s > '1'` |
| `n as small int8` over a value past `int8` | null, the safe reading of a cast |
| `n as small int8 not null` over the same value | refused, naming `small` and the value |
| `select *` over a stream, or a cast to the type a column already has | the batch is handed back unchanged |

The grammar itself accepts every common spelling - `insert into`, `append to`, `merge into ... on (...)`, `upsert into ... by (...)`, `* exclude (...)` and `* except (...)`, `limit` before or after `offset` - and prints one canonical text.

## Text round-trips

Parentheses come from precedence, and a literal prints its type when the bare spelling would not recover it.

| Bare spelling | Datatype |
| --- | --- |
| integer | `int64` |
| float | `float64` |
| quoted | `utf8` |
| anything else | `<datatype> '<text>'` |

```rust
use yggdryl::expression::Term;
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
    "trade.legs[1:3]",
    "try_cast(x as decimal128(9,2)) > decimal128(9,2) '1.50'",
] {
    let parsed: Term = text.parse()?;
    assert_eq!(parsed.to_string().parse::<Term>()?, parsed);
}

for text in [
    "select a, b as c",
    "select * exclude (secret)",
    "where a > 1",
    "upsert into t by (id) select id from s where a > 1 order by id desc limit 10 offset 5",
    "create t (id int64 not null); insert into t from s; select id from t",
] {
    let parsed: Expression = text.parse()?;
    assert_eq!(parsed.to_string(), text);
    assert_eq!(parsed.to_string().parse::<Expression>()?, parsed);
}
```

## Null is unknown

`and` is false when any operand is false, and `or` is true when any operand is true, even when another operand is unknown.
`not unknown` is unknown. A null operand in a comparison makes the answer unknown: `eval` gives `Scalar::Null` and `matches` gives false. The last rows of [Use](#use) assert both in Rust and `matches` in Python and JavaScript.

## Edges

- Null operand in a comparison -> unknown; `eval` gives `Scalar::Null`, `matches` gives false.
- `is distinct from`, `is not distinct from` -> two-valued, never unknown.
- `(a and b) and c`, `a and (b and c)` -> one value, one printing.
- Python `float` or JavaScript number against `decimal(9,2)` -> a different number; pass `decimal.Decimal` or `Scalar.decimal`.
- `"a > 1"` read as an `Expression` -> refused naming `select` and `where`: the text has to say which clause it is; a `Filter` or `Term` takes it as is.
- Nesting past the shared limit -> parse error at the failing byte.

## Commands

=== "Rust"

    ```bash
    cargo test --features "iceberg internals parquet" -p yggdryl --test expression
    cargo test --features "parquet iceberg" -p yggdryl --test expression -- plan
    cargo bench -p yggdryl --bench expression -- expression_parse
    cargo bench -p yggdryl --bench expression -- expression_bind
    cargo bench -p yggdryl --bench expression -- expression_display
    cargo bench -p yggdryl --bench expression -- expression_identity
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_expression.py
    python/.venv/bin/python -m pytest python/tests/test_expression.py -k "round_trips or document or binding_resolves or rows_answer or parameters"
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/expression.test.js
    node --test --test-name-pattern="round-trips|binding resolves|a row answers" node/tests/expression.test.js
    ```
