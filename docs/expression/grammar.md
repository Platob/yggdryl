# Grammar

The language the [expression layer](index.md) parses: one statement grammar, one closed function set, two parse limits.

## Contract

| Key | Value |
| --- | --- |
| Owns | the statement and expression grammar, the function set, the parse limits |
| Types | `Expression` is name-based and serializable, `Bound` is schema-resolved and is not |
| Bindings | one expression binds against a data schema, a partition schema, and a listing |
| Logic | Kleene three-valued, and a filter keeps a row only when the answer is exactly true |
| Functions | 18, closed, no registry |
| Nesting | the schema grammar's hard limit |
| Nodes | at most 100,000, checked once before any walk |
| Comment | `--` to end of line |
| Evaluation | [Evaluate](evaluate.md) |
| `&holder.*` | [Holder attributes](holder.md) |

## Use

Every statement parses to this shape.

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

## Spellings

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

## Functions

The set is closed, because an open registry cannot promise that the three evaluators agree about a function none of them knows.

`lower`, `upper`, `length`, `substring`, `trim`, `starts_with`, `ends_with`, `contains`, `concat`,
`year`, `month`, `day`, `hour`, `truncate`, `coalesce`, `if_null`, `size`, `get`.

## Decisions

Settled against Iceberg's bound/unbound split, Substrait's reference model, Arrow and DataFusion coercion, and the SQL that DuckDB, Spark, Calcite, and Polars ship.

### Accepted

| Rule | Behaviour |
| --- | --- |
| `is distinct from` | two-valued, the operator that answers about a null |
| indices | 0-based, and negative from the end |
| `substring` | 1-based, window `[start, start + length)` intersected with the characters that exist, a negative start counting back from the end |
| text order | code point, no collation, so every statistics bound stays valid |
| names | ASCII case-insensitive, and a genuine collision is an error |
| floats | IEEE 754 totalOrder, so `nan` equals `nan` and sorts above everything and `-0.0` sorts below `+0.0` |
| decimals | never implicitly a float, an explicit cast is required |
| division | at least six fractional places, so `1.00 / 3.00` stays a division |
| null against failure | a null operand produces null, while overflow, division by zero, an inexact quotient, and an undefined operand pair stay distinct core errors |

### Refused

| Refused | Because |
| --- | --- |
| subqueries, joins, aggregates, windows | each needs a second relation |
| regular expressions (`~`, `rlike`, `similar to`) | a regex engine is a dependency this workspace does not add |
| `element_at` | engines disagree about 0-based or 1-based, so the operation is spelled `get` |
| implicit string-to-number coercion | a direction that depends on syntactic position generates bugs |
| a per-row `like` pattern | a different operation, and it makes the vectorized tier slower than the scalar one |
| `[ident]` bracket-quoted identifiers | the brackets are already list construction and list indexing |
| `\|\|` as `or`, `&&` as `and` | one operator, one meaning |
| a session timezone | meaning would depend on who evaluates it |
| calendar units in `truncate` | a month is not a fixed length, read one with `year()` or `month()` |

### Reserved

Parsed as an error today, with the syntax kept free for a non-breaking addition.

- slices `[a:b]`, integer division `//`
- `date_diff`, `concat_ws`, `strip_prefix`, `strip_suffix`
- the JSON operators `->` and `->>`
- hexadecimal and digit-separator literals, grapheme-aware length
- Parquet row-group pruning through the same `Bounds` the other three containers use

## Edges

- Nesting past the limit -> refused at parse, never a crash.
- More than 100,000 nodes -> refused, because depth alone does not bound work.
- `'2' > 1` -> a bind error naming the cast that would fix it.
- An index past the end, or a missing map key -> null.
- A struct child reached by a missing name -> a bind error.
- One column named twice under case-insensitive resolution -> an ambiguity error.
- A failed `cast` -> an error, where `try_cast` -> null.
- A quotient still inexact at the declared scale -> the core inexact-arithmetic error.
- A `like` pattern that changes per row -> refused at bind.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib -- expression::tests::a_parse_failure_names_where_it_stopped expression::tests::nesting_past_the_limit_is_refused_not_crashed expression::tests::quoted_names_survive_every_encapsulator expression::tests::a_pattern_that_changes_per_row_is_refused_at_bind expression::tests::a_pattern_with_no_wildcard_becomes_an_equality expression::tests::substring_takes_the_window_the_standard_names expression::tests::a_column_named_twice_in_two_cases_is_ambiguous expression::tests::an_exact_quotient_keeps_room_to_be_a_quotient expression::tests::scalar_casts_return_the_exact_target_leaf expression::tests::scalar_arithmetic_propagates_checked_failures expression::tests::two_operands_with_no_common_type_are_refused expression::tests::a_struct_expression_produces_and_reprints_a_row_sequence
    cargo bench -p yggdryl --bench expression -- expression_parse
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/expression -k "never_taken or operators_build or arithmetic_builders"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="never taken|either spelling|arithmetic builders" node/tests/expression
    ```
