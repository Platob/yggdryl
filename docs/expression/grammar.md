# Grammar

The language the [expression layer](index.md) parses: one plan grammar over one term grammar, one closed function set, two parse limits.

## Contract

| Key | Value |
| --- | --- |
| Owns | the plan, clause and term grammars, the function set, the parse limits |
| Types | `Term`, `Filter`, `Selector`, `Plan` and `Expression` are name-based and serializable; `Bound` and `BoundSelector` are schema-resolved and are not; partially bound is unrepresentable |
| Bindings | one term binds against a data schema and a partition schema |
| Logic | Kleene three-valued, and a filter keeps a row only when the answer is exactly true |
| Functions | 20, closed; a registered user function is spelled `namespace.name(...)` and is not in the grammar ([Functions](functions.md)) |
| Nesting | the schema grammar's hard limit, for terms and for plans in `from (...)` |
| Nodes | at most 100,000, checked once before any walk |
| Comment | `--` to end of line |
| Reference | the DuckDB SQL and Python expression API for spellings and aliases; where DuckDB and the best-effort rule disagree, this grammar follows the [rule](index.md#best-effort-then-a-named-refusal) |
| Evaluation | [Evaluate](evaluate.md) |

## Use

Every expression parses to this shape.

```text
expression  := plan (";" plan)*
plan        := [create] [write] ["select" selector] ["from" source] ["where" expr]
               ["order" "by" orders] ["limit" n] ["offset" n]      -- limit and offset in either order
create      := "create" ["table" | "view"] [target] "(" selector ")" ["with" properties]
write       := verb [target] ["by" | "on" "(" selector ")"]        -- keys only after upsert
verb        := "insert" ["into"] | "insert" "overwrite" ["into"] | "append" ["into" | "to"]
             | "overwrite" ["into"] | "replace" ["into"] | "upsert" ["into"] | "merge" ["into"]
             | "delete" ["from"]
target      := location ["with" properties]
location    := "'url'" | part ("." part)*
part        := identifier | "\"quoted\"" | "`quoted`" | "[bracketed]" | number
properties  := "(" name "=" "'value'" ("," ...)* ")"
source      := target | "(" plan ")"
selector    := "*" [("exclude" | "except") "(" identifier,* ")"] ("," projection)*
             | projection ("," projection)*                  -- a star leads or is absent
projection  := expr ["as" identifier] [datatype ["null" | "not null"]] ["with" properties]
                                                   -- unnest(expr) only as the whole expr, once
orders      := (expr ["asc" | "desc"] ["nulls" ("first" | "last")]) ("," ...)*

expr        := disjunction
disjunction := conjunction ("or" conjunction)*
conjunction := negation ("and" negation)*
negation    := "not" negation | predicate
predicate   := additive [ comparison | "is" .. | "in" .. | "between" .. | "like" .. | "glob" .. ]
additive    := product (("+" | "-") product)*
product     := unary (("*" | "/" | "%") unary)*
unary       := "-" unary | accessor
accessor    := atom ("." identifier | "[" key "]" | "[" [n] ":" [n] "]" | "[" expr "]")*
atom        := literal | "(" expr ")" | column | ":" parameter
             | "cast" "(" expr "as" datatype ")" | "try_cast" "(" .. ")"
             | "case" ("when" expr "then" expr)+ ["else" expr] "end"
             | function "(" expr,* ")" | identifier "." identifier "(" expr,* ")"
             | "[" expr,* "]" | "{" expr ":" expr,* "}"
             | "struct" "(" expr "as" identifier,* ")" | datatype ("'text'" | "null")
```

A lone `select ...` is a `Selector` and a lone `where ...` a `Filter`; either keyword is optional when the text is read as that clause alone. `Plan::from_str` reads one plan, `Expression::from_str` one plan or a `;` sequence.

Inside `[...]` a whole number is a position, a text constant a key, a `:` form a run, and anything else - a bare boolean column included - a predicate over the elements of a serie of structs, read against the element's own fields ([Terms](terms.md#predicate-segments)).

## Spellings

| Shape | Spelling |
| --- | --- |
| comparison | `=`, `<>` (or `!=`), `<`, `<=`, `>`, `>=` |
| distinctness | `is distinct from`, `is not distinct from` |
| null | `is null`, `is not null` |
| membership | `x in (a, b)`, `x not in (a, b)` |
| range | `x between low and high`, `x not between low and high` |
| pattern | `x like 'a%'`, `x ilike 'A%'`, `x like 'a!%' escape '!'`, `x glob '**/*.parquet'` |
| path | `a.b`, `a[0]`, `a[-1]`, `a['key']`, `a[1:3]`, `a[:-1]`, `a[ccy = 'EUR']`, `a[active]`, `a[ccy = 'EUR'][0].price` |
| identifier | `name`, `"odd name"`, `` `odd name` `` |
| literal | `1`, `1.5`, `'text'`, `true`, `null`, `decimal128(9,2) '1.50'`, `date32 '2024-01-01'`, `utf8 null` |
| constructor | `[1, 2]` a serie, `{'k': 1}` a map, `struct(1 as a)` a struct |
| conditional | `case when c then v else w end` |
| conversion | `cast(x as int32)`, `try_cast(x as int32)` |
| parameter | `:since` |
| projection | `price`, `price as amount`, `price as amount decimal(9,2) not null`, `id int64 with (comment = 'key')` |
| exclusion | `* exclude (secret)`, `* except (secret)`, and a star appending: `*, upper(name) as name`, `* exclude (secret), upper(name) as name` |
| row-multiplying projection | `unnest(legs) as leg`, `explode(legs)`, `unnest([bid, ask]) as side` |
| write verb | `insert into`, `insert overwrite`, `upsert into ... by (...)`, `delete from`; aliases `append to`, `overwrite`, `replace into`, `merge into ... on (...)` |
| location | `'file:///lake/trades.parquet'`, `catalog.schema.table`, `catalog."odd schema".[odd.table]` |
| target properties | `t with (media_type = 'text/csv', batch_row_size = '1024')` |
| comment | `-- to end of line` |

## Functions

The set is closed, because an open registry cannot promise that the three evaluators agree about a function none of them knows.

`lower`, `upper`, `length`, `substring`, `trim`, `starts_with`, `ends_with`, `contains`, `concat`,
`year`, `month`, `day`, `hour`, `truncate`, `coalesce`, `if_null`, `size`, `get`, `slice`, [`unnest`](#unnest) (alias `explode`).

A qualified name - `py.double(size)` - is a [user-defined function](functions.md): registered with a signature outside the grammar, typed and called by the two row evaluators through it, and unknown to the statistics evaluator, which is what keeps the promise above.

## Unnest { #unnest }

`unnest(<serie>) [as name]` - DuckDB's verb, `explode` its alias, printed `unnest` - is the one select-list form that multiplies rows: each parent row becomes one row per element of its serie, the other columns repeated beside it. It stands only as the whole term of a projection, at most once per select. The serie is a column or path of any serie layout, or a constructor, `unnest([bid, ask]) as side` stacking two same-typed columns. A struct item expands one level to a column per child named `<name>.<child>`, any other item to one column named `name`; the name is the alias, else the path's column name, else the term's canonical text, and a declared datatype and nullability apply to the item. A null or empty serie drops its row. A `where` or an `order by` naming a column the unnest publishes - `"leg.ccy"` - runs after it, and one naming only stored columns runs first, where it prunes.

=== "Rust"

    ```rust
    use yggdryl::expression::Plan;
    use yggdryl::{DataType, Scalar, Serie, StructType};

    let leg = DataType::from(StructType::from_fields([
        DataType::utf8().nullable_field("ccy"),
        DataType::Int64.nullable_field("size"),
    ])?)
    .nullable_field("leg");
    let root = DataType::from(StructType::from_fields([
        DataType::Int64.required_field("id"),
        DataType::serie(leg).nullable_field("legs"),
    ])?)
    .required_field("trades");
    let leg = |ccy: &str, size: i64| Scalar::from_sequence([Scalar::from(ccy), Scalar::from(size)]);
    let batch = Serie::from_scalars(
        root,
        [
            Scalar::from_sequence([Scalar::from(1_i64), Scalar::from_sequence([leg("EUR", 1), leg("USD", 2)])]),
            Scalar::from_sequence([Scalar::from(2_i64), Scalar::from_sequence([])]),
            Scalar::from_sequence([Scalar::from(3_i64), Scalar::Null]),
        ],
    )?
    .into_arrow_batch()?;

    // One row per element beside its parent; a struct item is a column per child.
    let plan: Plan = "select id, unnest(legs) as leg where \"leg.ccy\" <> 'GBP'".parse()?;
    let reader = plan.apply_arrow_reader(yggdryl::arrow::batch_reader(batch.schema(), [batch]))?;
    let names: Vec<String> = reader.schema().fields().iter().map(|field| field.name().clone()).collect();
    assert_eq!(names, ["id", "leg.ccy", "leg.size"]);
    let rows: usize = reader.map(|batch| batch.expect("a batch").num_rows()).sum();
    assert_eq!(rows, 2, "an empty serie and a null one publish no row");

    // `explode` is the same verb, printed as `unnest`.
    assert_eq!("select explode(legs) as leg".parse::<Plan>()?.to_string(), "select unnest(legs) as leg");
    ```

=== "Python"

    ```python
    import pyarrow as pa

    from yggdryl import Plan

    legs = pa.list_(pa.struct([("ccy", pa.utf8()), ("size", pa.int64())]))
    batch = pa.record_batch(
        {
            "id": pa.array([1, 2, 3], pa.int64()),
            "legs": pa.array([[{"ccy": "EUR", "size": 1}, {"ccy": "USD", "size": 2}], [], None], legs),
        }
    )

    # One row per element beside its parent; a struct item is a column per child.
    plan = Plan("select id, unnest(legs) as leg where \"leg.ccy\" <> 'GBP'")
    out = plan.apply_arrow_reader(pa.RecordBatchReader.from_batches(batch.schema, [batch])).read_all()
    assert out.schema.names == ["id", "leg.ccy", "leg.size"]
    assert out.num_rows == 2, "an empty serie and a null one publish no row"
    assert out.column("leg.ccy").to_pylist() == ["EUR", "USD"]
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const arrow = require('apache-arrow')
    const { BatchReader, Plan } = require('yggdryl')

    const leg = new arrow.Struct([
      new arrow.Field('ccy', new arrow.Utf8(), true),
      new arrow.Field('size', new arrow.Int64(), true),
    ])
    const table = new arrow.Table({
      id: arrow.vectorFromArray([1n, 2n, 3n], new arrow.Int64()),
      legs: arrow.vectorFromArray(
        [[{ ccy: 'EUR', size: 1n }, { ccy: 'USD', size: 2n }], [], null],
        new arrow.List(new arrow.Field('item', leg, true)),
      ),
    })

    // One row per element beside its parent; a struct item is a column per child.
    const plan = new Plan("select id, unnest(legs) as leg where \"leg.ccy\" <> 'GBP'")
    const out = plan.applyArrowReader(BatchReader.from(table)).intoTable()
    assert.deepEqual(out.schema.fields.map((field) => field.name), ['id', 'leg.ccy', 'leg.size'])
    assert.equal(out.numRows, 2, 'an empty serie and a null one publish no row')
    assert.deepEqual([...out.getChild('leg.ccy')], ['EUR', 'USD'])
    ```

## Decisions

Settled against Iceberg's bound/unbound split, Substrait's reference model, Arrow and DataFusion coercion, and the SQL that DuckDB, Spark, Calcite, and Polars ship. DuckDB is the reference for what a spelling should mean when engines differ, because its Arrow-based model is the closest to this one; its `ColumnExpression`, `StarExpression(exclude=...)`, `alias`, `cast`, `isin`, `between`, `isnull`, `when/otherwise`, `asc/desc` and `nulls_first` have the same names here.

### Accepted

| Rule | Behaviour |
| --- | --- |
| `is distinct from` | two-valued, the operator that answers about a null |
| indices | 0-based, and negative from the end; a slice is `[start:end)` with either bound optional |
| predicate segment | JSONPath's `[?(...)]` without the `?`: `serie[filter]` keeps the elements a boolean over the element's own fields answers exactly true for, a null element is dropped, a null serie stays null, and the answer is a serie of the same item type |
| `substring` | 1-based, window `[start, start + length)` intersected with the characters that exist, a negative start counting back from the end |
| text order | code point, no collation, so every statistics bound stays valid |
| names | ASCII case-insensitive, and a genuine collision is an error |
| floats | IEEE 754 totalOrder, so `nan` equals `nan` and sorts above everything and `-0.0` sorts below `+0.0` |
| decimals | never implicitly a float, an explicit cast is required |
| division | at least six fractional places, so `1.00 / 3.00` stays a division |
| constants | coerce into the other operand's type when the text reads as one; two columns with no common type compare as text |
| null against failure | a null operand produces null, while overflow, division by zero, an inexact quotient, and an undefined operand pair stay distinct core errors |
| verbs | every alias prints its canonical verb; `merge into ... on (...)` is `upsert into ... by (...)` |
| `limit` and `offset` | read in either order, printed `limit` first |
| a write with no target | writes to the handle the plan is given to |

### Refused

| Refused | Because |
| --- | --- |
| subqueries in `where`, joins, aggregates, windows | each needs a second relation; a plan's `from (plan)` is the one nesting there is |
| regular expressions (`~`, `rlike`, `similar to`) | a regex engine is a dependency this workspace does not add |
| `element_at` | engines disagree about 0-based or 1-based, so the operation is spelled `get` |
| a per-row `like` pattern | a different operation, and it makes the vectorized tier slower than the scalar one |
| `\|\|` as `or`, `&&` as `and` | one operator, one meaning |
| a session timezone | meaning would depend on who evaluates it |
| calendar units in `truncate` | a month is not a fixed length, read one with `year()` or `month()` |
| `select` keywords inside a projection list | a projection is an expression; `select` opens the clause once |
| `a, *`, a trailing `*,`, `* exclude ()` | the star leads the list or is absent, and an exclusion names what it drops |
| `unnest` inside a term, a `where`, an `order by`, a key or a column declaration; two in one select | a value read per row cannot be many rows, and two row-multiplying projections would multiply each other |

### Reserved

Parsed as an error today, with the syntax kept free for a non-breaking addition.

- integer division `//`
- `date_diff`, `concat_ws`, `strip_prefix`, `strip_suffix`
- the JSON operators `->` and `->>`
- hexadecimal and digit-separator literals, grapheme-aware length
- `group by`, `having`, `join`, `union`

## Edges

- Nesting past the limit -> refused at parse, never a crash; a plan nested in `from (...)` counts against the same limit.
- More than 100,000 nodes -> refused, because depth alone does not bound work.
- `'2' > 1` -> the text reads as the number it names, `2 > 1`; text that reads as no number compares as text.
- An index past the end, or a missing map key -> null.
- A predicate segment after a computed value (`lower(name)[x = 1]`) -> refused at parse naming the bracket; a bare constant no predicate can be (`a[1.5]`) -> refused at parse.
- A predicate segment on a column that is no serie of structs, or a predicate that answers no boolean -> a bind error naming the datatype; a name inside it the element lacks -> the unknown-column error listing the element's fields, never the row's.
- A struct child reached by a missing name -> a bind error.
- One column named twice under case-insensitive resolution -> an ambiguity error.
- A failed `cast` -> an error, where `try_cast` -> null.
- A quotient still inexact at the declared scale -> the core inexact-arithmetic error.
- A `like` pattern that changes per row -> refused at bind.
- A section word as a bare location (`from where`) -> refused naming the location it expected; quote it.
- `[unclosed` in a location -> refused naming the `]` it expected.
- `unnest(xs) + 1`, `coalesce(unnest(xs), 0)`, `where unnest(xs) > 1`, `order by unnest(xs)` -> ``unnest is a select-list form: expected `unnest(xs)` as the whole term of a projection, got it where a value is read``; in `create (...)` -> the same, `got it in a column declaration`.
- `unnest(xs) as x, unnest(legs) as leg` -> ``expected at most one unnest in a select, got `unnest(xs)` and `unnest(legs)` ``.
- `unnest(id)` over an `int64` -> ``expected a serie to unnest, got int64 in `unnest(id)` ``; `unnest(xs, legs)` -> refused at parse for its arity, and a tree built with two arguments by hand -> ``expected unnest to take one serie, got 2 arguments in `...` ``.
- `with (...)` on a projection unnesting a struct -> refused: the metadata would have no one column to sit on.
- A null or empty serie -> no row; a null element -> one row of nulls; a null struct element -> a row whose children are null.

## Commands

=== "Rust"

    ```bash
    cargo test --features "iceberg internals parquet" -p yggdryl --test expression -- bind::grammar eval::grammar eval::internal parser::grammar term::grammar typing::grammar
    cargo test --features "parquet iceberg" -p yggdryl --test expression -- plan
    cargo test --features "parquet iceberg" -p yggdryl --test expression -- predicate_segment inside_brackets
    cargo bench -p yggdryl --bench expression -- expression_parse
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/test_expression.py -k "never_taken or operators_build or arithmetic_builders or whichever_clause"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="never taken|either spelling|arithmetic builders|whichever clause" node/tests/expression.test.js
    ```
