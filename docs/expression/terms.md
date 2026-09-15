# Terms

`Term` is the tree every clause is built from; `Bound` is that tree compiled against one schema.

## Contract

| Key | Value |
| --- | --- |
| Owns | `Term`, `Bound`, `Literal`, `Comparison`, `Operator`, `Function`, `col`, `lit` |
| Build | from text, or method by method: `column`, `literal`, `attribute`, `parameter`, comparisons, arithmetic, paths, `call`, `case`, `cast` |
| Bind | `bind(schema)` and `bind_with(schema, parameters)` resolve names to indices, convert each literal once into the type it meets, fold constant subtrees, order `and` cheapest-first |
| Simplify | `simplify()` is exact under three-valued logic and reaches a fixed point: `a = 1 or a = 2` is `a in (1, 2)`, `not (a = 1 or a = 2)` is `not a in (1, 2)` |
| Explain | `explain()` draws the tree one node per line; a bound tree adds each node's datatype, nullability and cost |
| Identity | `Eq`, `Ord`, `Hash` and `stable_hash()` over the canonical text, in every language |
| Bindings | Python `Term` with operators and reflected operators; JavaScript `Term` with the same names as methods |

## Use

Compose without going back through text, then bind once.

=== "Rust"

    ```rust
    use yggdryl::expression::{Term, col, lit};
    use yggdryl::{Field, Scalar};

    let schema: Field = "trades:struct<ccy:utf8,price:decimal(9,2),size:bigint>".parse()?;

    let composed = col("price").gt(lit(100_i64)).and(col("ccy").eq(lit("EUR")));
    assert_eq!(composed, "price > 100 and ccy = 'EUR'".parse::<Term>()?);
    assert_eq!(col("size").add(lit(1_i64)).to_string(), "size + 1");
    assert_eq!(col("trade").child("legs").slice(Some(1), Some(3)).to_string(), "trade.legs[1:3]");

    // A parameter is supplied at bind, and never again.
    let late: Term = "size >= :floor".parse()?;
    assert_eq!(late.parameters(), vec!["floor".to_owned()]);
    assert!(late.bind(&schema).is_err());
    let bound = late.bind_with(&schema, &[("floor", Scalar::from(10_i64))])?;
    assert_eq!(bound.term().to_string(), "size >= 10");
    assert!(bound.matches(&Scalar::from_sequence([Scalar::Null, Scalar::Null, Scalar::from(11_i64)]))?);

    // Simplification keeps the answer and drops nodes.
    assert_eq!("a = 1 or a = 2".parse::<Term>()?.simplify().to_string(), "a in (1, 2)");

    // The plan a term runs, drawn.
    let plan = "price > 100".parse::<Term>()?.bind(&schema)?.explain();
    assert!(plan.starts_with('>'));
    assert!(plan.contains("column price"));
    ```

=== "Python"

    ```python
    from yggdryl import Field, Term

    schema = Field("trades", "struct<ccy:utf8,price:decimal(9,2),size:bigint>", False)
    price = Term.column("price")

    composed = price.gt(100) & Term.column("ccy").eq("'EUR'")
    assert composed == Term("price > 100 and ccy = 'EUR'")
    assert str(Term.column("size") + 1) == "size + 1"
    assert str(Term.column("trade").child("legs").slice(1, 3)) == "trade.legs[1:3]"

    # A parameter is supplied at bind, and never again.
    late = Term("size >= :floor")
    assert late.parameters() == ["floor"]
    bound = late.bind(schema, {"floor": 10})
    assert str(bound.term) == "size >= 10"
    assert bound.matches({"size": 11})

    # Simplification keeps the answer and drops nodes.
    assert str(Term("a = 1 or a = 2").simplify()) == "a in (1, 2)"

    # The plan a term runs, drawn.
    plan = Term("price > 100").bind(schema).explain()
    assert plan.startswith(">") and "column price" in plan
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, Scalar, Term } = require('yggdryl')

    const schema = new Field('trades', 'struct<ccy:utf8,price:decimal(9,2),size:bigint>', false)
    const price = Term.column('price')

    const composed = price.gt('100').and(Term.column('ccy').eq("'EUR'"))
    assert.ok(composed.equals("price > 100 and ccy = 'EUR'"))
    assert.equal(Term.column('size').add(1).toString(), 'size + 1')
    assert.equal(Term.column('trade').child('legs').slice(1, 3).toString(), 'trade.legs[1:3]')

    // A parameter is supplied at bind, and never again.
    const late = new Term('size >= :floor')
    assert.deepEqual(late.parameters, ['floor'])
    const bound = late.bind(schema, { floor: 10 })
    assert.equal(bound.term.toString(), 'size >= 10')
    assert.equal(bound.matches(Scalar.from([null, null, 11])), true)

    // Simplification keeps the answer and drops nodes.
    assert.equal(new Term('a = 1 or a = 2').simplify().toString(), 'a in (1, 2)')

    // The plan a term runs, drawn.
    const plan = new Term('price > 100').bind(schema).explain()
    assert.ok(plan.startsWith('>') && plan.includes('column price'))
    ```

## Literals

A literal holds a `Scalar` in one datatype. `literal` infers the datatype from the value; `typed_literal` declares it and checks the value against it.

| Spelling | Reads as |
| --- | --- |
| `1` | `int64` |
| `1.5` | `float64` |
| `'text'` | `utf8` |
| `true`, `null` | `boolean`, the untyped null |
| `int32 '5'`, `date32 '2024-01-01'`, `utf8 null` | that datatype, its text read strictly |

At bind, a literal meets the column it is compared with and is converted once into that column's type: `price > 100` on a `decimal(9,2)` column becomes `price > decimal32(9,2) '100.00'`, and `i = '1'` on an `int64` column becomes `i = 1`. A constant subtree is folded by evaluating it, so `n > 2 * 1000` binds as `n > 2000`.

## Predicate segments

`list[filter]` keeps the elements of a list of structs for which `filter`, a boolean over the element's own fields, answers exactly true; the answer is a list of the same item type, so `[0]`, `[-1]`, `.name` and further predicates compose after it. The predicate is typed and bound against the element struct - a name inside it is the element's field, never the row's - and a parameter inside it is supplied at bind like any other. A null element is dropped, an element the predicate answers false or unknown for is dropped, no match is the empty list, and a null list stays null. The vectorized tier runs the predicate once over the flattened elements, filters them, and rebuilds the offsets; the statistics tier treats the segment as a column decode that proves nothing.

=== "Rust"

    ```rust
    use yggdryl::expression::Term;
    use yggdryl::{Field, Scalar};

    let schema: Field = "trades:struct<legs:list<struct<ccy:utf8,size:bigint>>>".parse()?;
    let leg = |ccy: &str, size: i64| Scalar::from_sequence([Scalar::from(ccy), Scalar::from(size)]);
    let row = Scalar::from_sequence([Scalar::from_sequence([leg("EUR", 1), leg("USD", 2), leg("EUR", 3)])]);

    // The predicate reads the element's fields and the answer keeps the list's item type.
    let eur: Term = "legs[ccy = 'EUR']".parse()?;
    assert_eq!(eur.to_string(), "legs[ccy = 'EUR']");
    assert_eq!(eur.columns(), vec!["legs".to_owned()]);
    let bound = eur.bind(&schema)?;
    assert_eq!(bound.field().dtype(), schema.fields()[0].dtype());
    assert_eq!(bound.eval(&row)?, Scalar::from_sequence([leg("EUR", 1), leg("EUR", 3)]));

    // A position and a name compose after it; a parameter inside is supplied at bind.
    let last: Term = "legs[ccy = :ccy and size > 1][-1].size".parse()?;
    let bound = last.bind_with(&schema, &[("ccy", Scalar::from("EUR"))])?;
    assert_eq!(bound.term().to_string(), "legs[ccy = 'EUR' and size > 1][-1].size");
    assert_eq!(bound.eval(&row)?, Scalar::from(3_i64));

    // No match is the empty list; a null list stays null.
    assert_eq!("legs[ccy = 'JPY']".parse::<Term>()?.bind(&schema)?.eval(&row)?, Scalar::from_sequence([]));
    let missing = Scalar::from_sequence([Scalar::Null]);
    assert_eq!("legs[true]".parse::<Term>()?.bind(&schema)?.eval(&missing)?, Scalar::Null);
    ```

=== "Python"

    ```python
    from yggdryl import Field, Term

    schema = Field("trades", "struct<legs:list<struct<ccy:utf8,size:bigint>>>", False)
    row = {"legs": [{"ccy": "EUR", "size": 1}, {"ccy": "USD", "size": 2}, {"ccy": "EUR", "size": 3}]}

    # The predicate reads the element's fields and the answer keeps the list's item type.
    eur = Term("legs[ccy = 'EUR']")
    assert str(eur) == "legs[ccy = 'EUR']"
    assert eur.columns() == ["legs"]
    bound = eur.bind(schema)
    assert bound.field.dtype == schema.fields[0].dtype
    assert len(bound.eval(row)) == 2

    # A position and a name compose after it; a parameter inside is supplied at bind.
    last = Term("legs[ccy = :ccy and size > 1][-1].size")
    bound = last.bind(schema, {"ccy": "EUR"})
    assert str(bound.term) == "legs[ccy = 'EUR' and size > 1][-1].size"
    assert bound.eval(row) == 3

    # No match is the empty list; a null list stays null.
    assert Term("legs[ccy = 'JPY']").bind(schema).eval(row) == []
    assert Term("legs[true]").bind(schema).eval({"legs": None}) is None
    ```

=== "JavaScript"

    ```javascript
    const assert = require('node:assert/strict')
    const { Field, Scalar, Term } = require('yggdryl')

    const schema = new Field('trades', 'struct<legs:list<struct<ccy:utf8,size:bigint>>>', false)
    const leg = (ccy, size) => [ccy, BigInt(size)]
    const row = Scalar.from([[leg('EUR', 1), leg('USD', 2), leg('EUR', 3)]])

    // The predicate reads the element's fields and the answer keeps the list's item type.
    const eur = new Term("legs[ccy = 'EUR']")
    assert.equal(eur.toString(), "legs[ccy = 'EUR']")
    assert.deepEqual(eur.columns, ['legs'])
    let bound = eur.bind(schema)
    assert.ok(bound.field.dtype.equals(schema.fields[0].dtype))
    assert.ok(bound.eval(row).equals(Scalar.from([leg('EUR', 1), leg('EUR', 3)])))

    // A position and a name compose after it; a parameter inside is supplied at bind.
    const last = new Term("legs[ccy = :ccy and size > 1][-1].size")
    bound = last.bind(schema, { ccy: 'EUR' })
    assert.equal(bound.term.toString(), "legs[ccy = 'EUR' and size > 1][-1].size")
    assert.ok(bound.eval(row).equals(Scalar.from(3n)))

    // No match is the empty list; a null list stays null.
    assert.ok(new Term("legs[ccy = 'JPY']").bind(schema).eval(row).equals(Scalar.from([])))
    assert.ok(new Term('legs[true]').bind(schema).eval(Scalar.from([null])).equals(Scalar.from(null)))
    ```

## Binding

| Step | Does |
| --- | --- |
| resolve | every column name to an index, ASCII case-insensitively; an unknown name lists the ones there are |
| type | the output `Field` of every node against the schema; `is_predicate` is whether the root answers a boolean |
| coerce | each literal into the operand it meets; operands with no common type compare as text |
| fold | constant subtrees evaluated once |
| order | the operands of `and` cheapest-first: free holder attributes, then stats, then rows, so a listing stops at the first `false` without a decode |

`Bound` then answers a row (`eval`, `matches`), a batch (`evaluate`, `filter_mask`, `filter`), a reader (`filter_reader`), a holder (`matches_holder`), and a container's statistics (`statistics_prune`, `statistics_certainty`); [Evaluate](evaluate.md) and [Holder attributes](holder.md) show each.

## Edges

- `bind` without a parameter the term names -> refused naming the parameter.
- A parameter supplied twice, or one the term never names -> refused.
- A column named in two cases -> ambiguity error listing both.
- `simplify` -> a fixed point: simplifying twice changes nothing.
- `explain` on a `Bound` -> the cheapest-first order, which is the order the tree runs in.
- A term with holder attributes bound against an empty struct -> binds; `reads_rows` is false.
- A predicate segment on a computed value -> refused at parse; on anything but a list of structs, or with a predicate that answers no boolean -> refused at bind naming the datatype.

## Commands

=== "Rust"

    ```bash
    cargo test --features "parquet iceberg" -p yggdryl --lib -- expression::tests::binds_and_evaluates_rows expression::tests::a_literal_is_converted_once_into_the_column_it_meets expression::tests::a_constant_subtree_is_folded_by_evaluating_it expression::tests::binding_simplifies_before_it_lowers expression::tests::parameters_are_supplied_at_bind_and_never_again expression::tests::an_unknown_column_names_the_ones_there_are expression::tests::cheapest_first_is_stable_when_costs_tie expression::tests::a_simplification_has_fewer_nodes_and_one_shape expression::tests::a_simplification_answers_what_the_original_answered
    cargo test --features "parquet iceberg" -p yggdryl --test expression -- predicate_segment
    cargo bench -p yggdryl --bench expression -- expression_bind expression_predicate_path
    ```

=== "Python"

    ```bash
    python/.venv/bin/python -m pytest python/tests/expression -k "binding_resolves or parameters or composes or simplification or nested_value"
    ```

=== "JavaScript"

    ```bash
    node --test --test-name-pattern="binding resolves|arithmetic builders|either spelling" node/tests/expression
    ```
