# Expression

`yggdryl::Expression` is one recursive tree that says which rows and which values, typed against a schema and compiled once.

## Contract

| Aspect | Rule |
| --- | --- |
| Owns | `Expression`, `Bound`; `(column, value)` pairs are sugar, not a second implementation |
| Not a value | [`Scalar`](../types/scalar.md) is plain data; an expression needs a schema to mean anything |
| Pipeline | parse -> type -> bind(schema) -> evaluate; bind runs once per stream, never per batch or row |
| Answers | `Scalar` row, Arrow batch, container statistics |
| Canonical text | `Display` inverts `FromStr`, asserted by a property test |
| Structural form | Tagged JSON document, as `DataType` and `Field` |
| Null | Three-valued; a row is kept only when `true` |
| Errors | Byte-positioned parse errors; nesting limit shared with the schema grammar |

## Use

Parse, bind once, ask a row.

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
        "ccy = 'EUR' and price > decimal32(9,2) '100.00'",
    );

    let row = Scalar::from_sequence([
        Scalar::from("EUR"),
        Scalar::d128(15_000, 2),
        Scalar::from(5_i64),
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
    assert str(bound.expression) == "ccy = 'EUR' and price > decimal32(9,2) '100.00'"

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
      "ccy = 'EUR' and price > decimal32(9,2) '100.00'",
    )

    // The price is an exact decimal, because the column is exact and so is
    // the comparison: a JavaScript number here would be a different one.
    const price = Scalar.decimal(15000n, 2)
    assert.equal(bound.matches(Scalar.fromJs(['EUR', price, 5])), true)
    assert.equal(bound.matches(Scalar.fromJs(['USD', price, 5])), false)
    ```

## Pages

| Page | Purpose |
| --- | --- |
| [Expression](index.md) | This page: stages, canonical text, nulls |
| [Grammar](grammar.md) | Grammar, closed function set, nesting and node-count limits |
| [Holder attributes](holder.md) | `&holder.*` attributes, cost classes, pruning without reading |
| [Evaluate](evaluate.md) | Arrow tier, `Statement::bind`, `ApplyExpression`, Iceberg scan planning |

## Four stages

```text
text ──parse──▶ Expression ──bind(schema)──▶ Bound ──▶ Scalar | ArrayRef | mask
```

| Stage | Does |
| --- | --- |
| Parse | One recursive grammar; byte-positioned errors |
| Type | Output `Field` resolved against the schema; decided here only |
| Bind | Names to indices, parameters, literals converted once, constants folded, `and` cheapest-first |
| Evaluate | One bound tree answers rows, batches, statistics |

## Text round-trips

Parentheses come from precedence, and a literal prints its type when the bare spelling would not recover it.

| Bare spelling | Datatype |
| --- | --- |
| integer | `int64` |
| float | `float64` |
| quoted | `utf8` |
| anything else | `<datatype> '<text>'` |

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

## Null is unknown

`and` is false when any operand is false, `or` is true when any operand is true, `not unknown` is unknown.

```rust
use yggdryl::{Expression, Field, Scalar};

let schema: Field = "rows:struct<a:bigint>".parse()?;
let bound = "a > 1".parse::<Expression>()?.bind(&schema)?;

let missing = Scalar::from_sequence([Scalar::Null]);
assert_eq!(bound.eval(&missing)?, Scalar::Null); // the answer is unknown
assert!(!bound.matches(&missing)?); // and unknown does not keep the row
```

## Edges

- Null operand in a comparison -> unknown; `eval` gives `Scalar::Null`, `matches` gives false.
- `is distinct from`, `is not distinct from` -> two-valued, never unknown.
- `(a and b) and c`, `a and (b and c)` -> one value, one printing.
- Python `float` or JavaScript number against `decimal(9,2)` -> a different number; pass `Decimal` or `Scalar.decimal`.
- Nesting past the shared limit -> parse error at the failing byte.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib expression::tests
    cargo test --features "parquet iceberg" -p yggdryl --lib -- expression::tests::text_round_trips expression::tests::statements_round_trip expression::tests::documents_round_trip expression::tests::expressions_and_statements_have_core_total_order_and_stable_hash expression::tests::binds_and_evaluates_rows expression::tests::unknown_is_not_true expression::tests::a_literal_is_converted_once_into_the_column_it_meets expression::tests::a_constant_subtree_is_folded_by_evaluating_it expression::tests::parameters_are_supplied_at_bind_and_never_again expression::tests::an_unknown_column_names_the_ones_there_are expression::tests::cheapest_first_is_stable_when_costs_tie
    cargo bench -p yggdryl --bench expression -- expression_parse
    cargo bench -p yggdryl --bench expression -- expression_bind
    cargo bench -p yggdryl --bench expression -- expression_display
    cargo bench -p yggdryl --bench expression -- expression_identity
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/expression
    python/.venv/bin/python -m pytest python/tests/expression -k "round_trips or document or binding_resolves or rows_answer or parameters"
    ```

=== "JavaScript"

    ```bash
    node --test node/tests/expression
    node --test --test-name-pattern="round-trips|binding resolves|a row answers" node/tests/expression
    ```
