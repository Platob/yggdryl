# Expressions

`yggdryl::expressions` is the one filtering and selection vocabulary: a value that says what to keep
and what to compute, over the columns of a row.

=== "Rust"

    ```rust
    use yggdryl::Expr;

    // Parsed from SQL-like text, built with constructors, or built with the
    // language's own operators - and all three are the same value.
    let parsed: Expr = "venue = 'XNAS' AND price > 10".parse()?;
    let built = Expr::column("venue")
        .eq(Expr::literal("XNAS"))
        .and(Expr::column("price").gt(Expr::literal(10)));

    use yggdryl::expressions::{col, lit};
    let with_operators = col("venue").eq(lit("XNAS")) & col("price").gt(lit(10));

    assert_eq!(parsed, built);
    assert_eq!(parsed, with_operators);

    // The canonical text round-trips, and says which columns a read must decode.
    assert_eq!(parsed.to_string(), "venue = 'XNAS' AND price > 10");
    assert_eq!(parsed.columns(), vec!["venue".to_owned(), "price".to_owned()]);
    ```

!!! note "Rust first"

    The expression value is Rust-only for now. The Python and JavaScript surfaces are the next
    phase of this work; until they land, every example on this page is Rust alone rather than a
    fabricated binding tab.

An expression carries no schema, no handle, and no table format. Binding it against a struct
[`Field`](field.md) once produces a plan, and that one plan answers every question the rest of the
library asks: row at a time over a [`Value`](text.md), vectorized over an Arrow `RecordBatch`, and
three-valued over column statistics so a file, a manifest, or a partition directory is skipped
without being read. The three agree because they read the same plan - a filter that *prunes* and a
filter that *selects* must never be two implementations of one comparison.

## Take a filter from untrusted text

A predicate is data, not executed SQL. There is no engine to inject into, no catalog to reach, and
no way to name anything the schema does not declare - so text from a query string, a config file, or
a request body is safe to parse, and a bad one is a typed error with a byte offset the caller can
point at.

=== "Rust"

    ```rust
    use yggdryl::{Error, Expr};

    // A byte offset, and what was expected there.
    let error = "venue = ".parse::<Expr>().unwrap_err();
    let Error::Parse { position, reason, .. } = &error else {
        panic!("expected a parse error, got {error}");
    };
    assert_eq!(*position, 8);
    assert!(reason.contains("expected a value"));

    // An unterminated delimiter reports the *opener*, which is the position a
    // caller can actually fix.
    let error = "venue = 'XNAS".parse::<Expr>().unwrap_err();
    let Error::Parse { position, .. } = &error else { panic!("{error}") };
    assert_eq!(*position, 8);

    // And a function outside the closed vocabulary names the vocabulary it is
    // not in, rather than being looked up somewhere.
    let error = "system('rm -rf /') = 1".parse::<Expr>().unwrap_err();
    assert!(error.to_string().contains("coalesce"));
    ```

## Name a column that needs quoting

A column called `total amount`, `select`, or `prix (€)` has to be addressable, and only a delimiter
pair can make it so. Three are accepted on the way in - `"ansi"`, `` `hive` ``, and `[t-sql]` - each
doubling its own closer to embed it. Everything between the pair is part of the name: whitespace,
punctuation, operators, keywords, digits, and Unicode alike. One canonical spelling comes back out.

=== "Rust"

    ```rust
    use yggdryl::Expr;

    // Three spellings in, one spelling out.
    for text in ["\"total amount\" = 1", "`total amount` = 1", "[total amount] = 1"] {
        assert_eq!(text.parse::<Expr>()?.to_string(), "\"total amount\" = 1");
    }

    // A doubled closer embeds the delimiter itself.
    let embedded: Expr = "\"say \"\"hi\"\" now\" = 1".parse()?;
    assert_eq!(embedded.columns(), vec!["say \"hi\" now".to_owned()]);

    // Whitespace inside an encapsulator is data, and survives the round trip.
    let padded: Expr = "\"  a  \" IS NULL".parse()?;
    assert_eq!(padded.columns(), vec!["  a  ".to_owned()]);
    assert_eq!(padded.to_string(), "\"  a  \" IS NULL");
    ```

A **double-quoted token is always an identifier, never a string**. That is the ANSI rule and it
removes the one real ambiguity in the grammar:

=== "Rust"

    ```rust
    use yggdryl::Expr;

    // A column compared to a string.
    let column: Expr = "\"venue\" = 'XNAS'".parse()?;
    assert!(!column.columns().is_empty());

    // Two strings, which are not equal, so the whole thing folds to FALSE.
    let strings: Expr = "'venue' = 'XNAS'".parse()?;
    assert!(strings.columns().is_empty());
    assert!(strings.simplify().is_always_false());
    ```

## Reach inside a value

A column path descends by child, key, index, and range. Two conventions, stated once and loudly:

* **Indices are 0-based**, matching `Value::get` and Arrow's own `[]`. A negative index counts back
  from the end.
* **Ranges are half-open**, matching Rust and Python: `a[1:3]` is elements 1 and 2.

A range is *not* `BETWEEN`. `tags[1:3]` selects items; `n BETWEEN 1 AND 3` is a predicate. Both
exist, neither parses as the other.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Expr, Value};

    let schema = DataType::from_fields([
        DataType::list(DataType::Int64.nullable_field("item")).nullable_field("tags"),
        DataType::Utf8.nullable_field("path"),
    ])?
    .required_field("row");
    let row = Value::record(
        schema.data_type().clone(),
        [
            Value::from_sequence([Value::I64(10), Value::I64(20), Value::I64(30)]),
            Value::from("abcdef"),
        ],
    )?;
    let read = |text: &str| -> yggdryl::Result<Value> {
        text.parse::<Expr>()?.bind(&schema)?.evaluate(&row)
    };

    assert_eq!(read("tags[0]")?, Value::I64(10));      // 0-based
    assert_eq!(read("tags[-1]")?, Value::I64(30));     // from the end
    assert_eq!(read("tags[99]")?, Value::Null);        // out of range is null
    assert_eq!(read("tags[1:3]")?.len(), 2);           // half-open
    assert_eq!(read("tags[3:1]")?.len(), 0);           // inverted is empty
    assert_eq!(read("path[1:3]")?, Value::from("bc")); // text slices characters

    // A range is not BETWEEN: this one is a predicate over a scalar.
    let predicate: Expr = "tags[0] BETWEEN 1 AND 3".parse()?;
    assert_eq!(predicate.to_string(), "tags[0] BETWEEN 1 AND 3");
    ```

Text slices Unicode scalar values so an index never splits a character; binary slices bytes. A
struct child usually has its own leaf statistics and so can prune, while a list element, a map
entry, and every range answer *maybe* - no statistic bounds one, and pruning on a column's bounds
would lose rows.

## Bind once, evaluate many

Binding is where a name becomes a slot chain and a text becomes a typed value. It happens **once per
read** - never per batch, never per row - and it is also where the literal is folded into the
column's own type. That folding is a correctness rule rather than an optimization: `Value`'s
ordering is a *total* order for sorting, so a decimal sorts after every integer regardless of
magnitude, and only folding makes `price > '10.5'` compare two decimals.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Expr, Value};

    let schema = DataType::from_fields([
        DataType::Decimal128 { precision: 10, scale: 2 }.nullable_field("price"),
    ])?
    .required_field("row");

    // The text literal became the column's own decimal, once, at bind time.
    let bound = "price > '10.5'".parse::<Expr>()?.bind(&schema)?;
    assert_eq!(bound.to_expr().to_string(), "price > 10.50");

    // One plan, used over a row and over a batch - the same answer both ways.
    let predicate = bound.into_predicate()?;
    let row = Value::record(schema.data_type().clone(), [Value::Decimal(2_000, 2)])?;
    assert!(predicate.matches(&row)?);
    ```

A name the schema does not declare is an error listing what it does:

=== "Rust"

    ```rust
    use yggdryl::{DataType, Expr};

    let schema = DataType::from_fields([
        DataType::Int64.nullable_field("price"),
        DataType::Utf8.nullable_field("venue"),
    ])?
    .required_field("row");

    let error = "prise > 1".parse::<Expr>()?.bind(&schema).unwrap_err();
    let message = error.to_string();
    assert!(message.contains("prise"));
    assert!(message.contains("price, venue"));
    ```

## Null semantics

Evaluation is SQL three-valued logic, identically in all three evaluators.

| left    | right   | `=`     | `<>`    | `IS NULL` |
| ------- | ------- | ------- | ------- | --------- |
| value   | value   | true/false | true/false | false  |
| value   | null    | unknown | unknown | false     |
| null    | value   | unknown | unknown | true      |
| null    | null    | unknown | unknown | true      |

A filter keeps a row only on `true`, so **unknown drops the row** - which is the one place a reader
is usually surprised:

=== "Rust"

    ```rust
    use yggdryl::{DataType, Expr, Value};

    let schema = DataType::from_fields([DataType::Utf8.nullable_field("venue")])?
        .required_field("row");
    let missing = Value::record(schema.data_type().clone(), [Value::Null])?;

    let keep = |text: &str| -> yggdryl::Result<bool> {
        text.parse::<Expr>()?.bind(&schema)?.into_predicate()?.matches(&missing)
    };

    // `venue <> 'XNAS'` does NOT select the rows whose venue is null.
    assert!(!keep("venue <> 'XNAS'")?);
    // `venue IS NULL` is how that is asked.
    assert!(keep("venue IS NULL")?);
    // And so `DELETE WHERE price > 10` would leave a null price alone.
    assert!(!keep("venue > 'A'")?);
    ```

## Select and compute columns

A selection is an ordered projection of named expressions. A selection of bare columns produces
exactly the root `select_by_names` produces; one that computes adds the column its expression names,
evaluated *after* the encoding's own projection of the columns it reads.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Field, Selection};

    let schema = DataType::from_fields([
        DataType::Int64.nullable_field("price"),
        DataType::Utf8.nullable_field("venue"),
    ])?
    .required_field("row");

    let selection: Selection = "venue, price * 2 AS doubled, price".parse()?;

    // The root the selection produces, with nothing opened and no data read.
    use yggdryl::expressions::Apply;
    let root = selection.apply_field(&schema)?;
    let names: Vec<&str> = root.fields().iter().map(Field::name).collect();
    assert_eq!(names, vec!["venue", "doubled", "price"]);

    // An unnamed computed column takes its own canonical spelling.
    let unnamed: Selection = "price * 2".parse()?;
    assert_eq!(unnamed.apply_field(&schema)?.fields()[0].name(), "price * 2");

    // The columns a read must decode are the columns the selection *reads*.
    assert_eq!(selection.columns(), vec!["venue".to_owned(), "price".to_owned()]);
    ```

## Apply to what you already hold

One verb reaches every carrier that holds values - and a schema alone, when the caller only wants
the shape of the answer.

=== "Rust"

    ```rust
    use std::sync::Arc;

    use arrow_array::{Int64Array, RecordBatch, StringArray};
    use yggdryl::expressions::{Apply, ArrowApply};
    use yggdryl::{DataType, Expr, Value, arrow};

    let schema = DataType::from_fields([
        DataType::Utf8.nullable_field("venue"),
        DataType::Int64.nullable_field("id"),
    ])?
    .required_field("row");
    let batch = RecordBatch::try_new(
        arrow::schema_from_field(&schema)?,
        vec![
            Arc::new(StringArray::from(vec![Some("XNAS"), Some("XNYS"), None])),
            Arc::new(Int64Array::from(vec![Some(1_i64), Some(2), Some(3)])),
        ],
    )?;

    let filter: Expr = "venue = 'XNAS'".parse()?;

    // A batch, filtered.
    let kept = filter.apply_arrow_batch(batch.clone())?;
    assert_eq!(kept.num_rows(), 1);

    // A stream, filtered lazily - the schema answers before the first batch.
    let reader = arrow::batch_reader(batch.schema(), [batch.clone()]);
    let streamed = filter.apply_arrow_batch_reader(reader)?;
    assert_eq!(streamed.schema(), batch.schema());

    // A schema alone: nothing opened, no data allocated. Filtering does not
    // change a schema, so the answer is the root it was bound to.
    assert_eq!(filter.apply_field(&schema)?.field_len(), 2);

    // And one row at a time.
    let row = Value::record(schema.data_type().clone(), [Value::from("XNAS"), Value::I64(1)])?;
    assert_eq!(filter.apply_value(&schema, &row)?, Value::Bool(true));
    ```

## Watch the optimizer work

Binding runs one optimizer over one plan graph. `explain` names the rules that fired, so an
optimization is auditable and a regression is diagnosable by a reader rather than a debugger.

=== "Rust"

    ```rust
    use yggdryl::{DataType, Expr};

    let schema = DataType::from_fields([DataType::Int32.nullable_field("id")])?
        .required_field("row");

    // A long OR of equalities with a cast around the column: the worst shape a
    // pushdown can meet, because a cast on a column destroys pruning outright.
    let written: Expr =
        "CAST(id AS int64) = 1 OR CAST(id AS int64) = 2 OR CAST(id AS int64) = 3".parse()?;
    let bound = written.bind(&schema)?;

    // One IN list, and the cast moved to the literals - so the column is now
    // comparable against statistics again.
    assert_eq!(bound.to_expr().to_string(), "id IN (1, 2, 3)");

    let explained = bound.explain();
    assert!(explained.contains("cast moved from column to literal"));
    assert!(explained.contains("OR of equalities to an IN list"));
    ```

Every rule is semantics-preserving under three-valued logic, **or it declines**. Two folds that look
obvious and are not:

=== "Rust"

    ```rust
    use yggdryl::{DataType, Expr};

    // `a = a` is unknown when `a` is null, so it is never TRUE.
    assert_eq!("a = a".parse::<Expr>()?.simplify().to_string(), "a = a");
    assert_eq!("a <> a".parse::<Expr>()?.simplify().to_string(), "a <> a");

    // A contradiction is unknown when the column is null, so it folds only
    // where the schema proves it cannot be.
    let nullable = DataType::from_fields([DataType::Int64.nullable_field("a")])?
        .required_field("row");
    let held = "a > 5 AND a < 3".parse::<Expr>()?.bind(&nullable)?;
    assert!(!held.is_always_false());
    assert!(held.explain().contains("declined contradictory range"));

    let required = DataType::from_fields([DataType::Int64.required_field("a")])?
        .required_field("row");
    let folded = "a > 5 AND a < 3".parse::<Expr>()?.bind(&required)?;
    assert!(folded.is_always_false());
    ```

## Prune without reading

A statistics source says what it knows about a column, and the plan answers what that proves. The
one rule that outranks the others: **`Maybe` is always safe; `AlwaysFalse` must be provable**,
because a wrong one loses rows silently.

=== "Rust"

    ```rust
    use yggdryl::expressions::{BoundColumn, Certainty, ColumnStats, StatsSource};
    use yggdryl::{DataType, Expr, Value};

    struct File;

    impl StatsSource for File {
        fn stats(&self, column: &BoundColumn) -> Option<ColumnStats> {
            match column.name() {
                // Every row under `venue=XNAS/` holds that one value.
                "venue" => Some(ColumnStats::constant(Value::from("XNAS"))),
                "id" => Some(ColumnStats::range(Value::I64(100), Value::I64(200))),
                _ => None,
            }
        }
    }

    let schema = DataType::from_fields([
        DataType::Utf8.nullable_field("venue"),
        DataType::Int64.nullable_field("id"),
    ])?
    .required_field("row");
    let decide = |text: &str| -> yggdryl::Result<Certainty> {
        Ok(text.parse::<Expr>()?.bind(&schema)?.into_predicate()?.evaluate_stats(&File))
    };

    // Provably nothing: the file is never opened.
    assert_eq!(decide("venue = 'XNYS'")?, Certainty::AlwaysFalse);
    assert_eq!(decide("id > 500")?, Certainty::AlwaysFalse);
    // Provably everything: the conjunct never runs against a single row.
    assert_eq!(decide("venue = 'XNAS'")?, Certainty::AlwaysTrue);
    // Unsettled: the rows have to answer.
    assert_eq!(decide("id > 150")?, Certainty::Maybe);
    // Nothing known: never prune.
    assert_eq!(decide("absent > 1").is_err(), true);
    ```

The **residual** is what a source did not settle, and it is the third answer that matters:

=== "Rust"

    ```rust
    use yggdryl::expressions::{BoundColumn, ColumnStats, StatsSource};
    use yggdryl::{DataType, Expr, Value};

    struct Partition;

    impl StatsSource for Partition {
        fn stats(&self, column: &BoundColumn) -> Option<ColumnStats> {
            (column.name() == "venue").then(|| ColumnStats::constant(Value::from("XNAS")))
        }
    }

    let schema = DataType::from_fields([
        DataType::Utf8.nullable_field("venue"),
        DataType::Int64.nullable_field("id"),
    ])?
    .required_field("row");
    let predicate = "venue = 'XNAS' AND id > 100"
        .parse::<Expr>()?
        .bind(&schema)?
        .into_predicate()?;

    // The partition settled one conjunct, so only the other reaches the rows.
    let residual = predicate.residual(&Partition).expect("the file can match");
    assert_eq!(residual.len(), 1);
    assert_eq!(residual[0].to_string(), "id > 100");

    // An empty list would mean "every row matches"; `None` means "no row does".
    struct Elsewhere;
    impl StatsSource for Elsewhere {
        fn stats(&self, column: &BoundColumn) -> Option<ColumnStats> {
            (column.name() == "venue").then(|| ColumnStats::constant(Value::from("XNYS")))
        }
    }
    assert!(predicate.residual(&Elsewhere).is_none());
    ```

## What the grammar accepts

Precedence, loosest first:

| level | operators |
| ----- | --------- |
| 1 | `OR` |
| 2 | `AND` |
| 3 | `NOT` |
| 4 | `=` `<>` `!=` `<` `<=` `>` `>=` `IS [NOT] NULL` `[NOT] IN` `[NOT] BETWEEN` `[NOT] LIKE` `ILIKE` |
| 5 | `+` `-` |
| 6 | `*` `/` `%` |
| 7 | unary `-` |
| 8 | `::type`, then `.child` `['key']` `[0]` `[1:3]` |

Literal forms:

| form | example | becomes |
| ---- | ------- | ------- |
| integer | `42` | `Value::I64` |
| exact decimal | `10.50` | `Value::Decimal(1050, 2)` - never an `f64` |
| float | `1.5e0` | `Value::F64`; also `NAN`, `INFINITY` |
| string | `'it''s'` | `Value::String`; `''` escapes the quote |
| bytes | `X'DEADBEEF'` | `Value::Bytes` |
| boolean, absence | `TRUE` `FALSE` `NULL` | `Value::Bool`, `Value::Null` |
| temporal | `DATE '2024-01-01'`, `TIME '12:30:00'`, `TIMESTAMP '2024-01-01T00:00:00Z'`, `INTERVAL 'PT1H'` | the matching `Value` variant |

A fractional literal is an **exact decimal keeping the scale as written**, because `0.1` has no
binary expansion and a price that arrives as `0.1` must leave as `0.1`. A float is spelled with an
exponent, which is also how one is written back out.

Casts go through the one type grammar, so every type [`DataType::from_str`](datatype.md) accepts is
accepted here, nested ones included: `CAST(x AS decimal(10, 2))`, `TRY_CAST(x AS int64)`, `x::int32`.

Functions are a closed set: `coalesce`, `length`, `lower`, `upper`, `trim`, `substring`, `abs`,
`truncate`, and `year`/`month`/`day`/`hour`/`minute`/`second` (also `EXTRACT(YEAR FROM x)`). A name
outside it is a parse error listing the vocabulary it is not in. The set is closed deliberately: an
open registry is a plugin system, and a plugin system cannot promise that the three evaluators
agree.

A calendar function returns the calendar field - `year(DATE '2024-03-01')` is `2024` - which is
SQL's meaning and deliberately not Iceberg's `years` transform, which counts from 1970.

## What it deliberately does not do

| absent | why |
| ------ | --- |
| subqueries | an expression evaluates over one row; a subquery needs a second read |
| joins | a bound source is selected from or merged on a key, never joined on an arbitrary predicate |
| aggregates, `GROUP BY`, windows | all of them need more than one row in hand, which is a materialization this library does not do |
| an open function registry | three evaluators cannot be promised to agree about a function none of them knows |
| `bucket` pruning | the hash is Iceberg's Murmur3, which this library does not implement - the limit is named rather than emulated |

## Where it is used

The expression is the filter and the selection everywhere a record is read or written:

* [`io`](io.md) - a folder prunes `column=value` directories and filters rows; a Parquet read skips
  row groups on their footer statistics.
* [`iceberg`](iceberg.md) - manifests, data files, and partition tuples are pruned by the same plan
  that then filters the rows.
* [`generic`](generic.md) - `RecordOptions::with_filter` and `with_selection`, with
  `filter_partitions` and `select_by_names` surviving as sugar for the same value.

## Migration

The `(column, value)` pair vocabulary still works and answers identically. It is sugar for one
equality per pair, and the expression form is what adds ranges, null tests, set membership, nested
access, and computed columns.

| before | after |
| ------ | ----- |
| `.with_filter_partitions([("venue", "XNAS")])` | `.with_filter("venue = 'XNAS'")?` |
| `.with_select_by_names(["id", "price"])` | `.with_selection("id, price")?` |
| `table.plan(&[("venue", "XNAS")])` | `table.plan_matching(&filter)` |
| `table.scan_where(&[("venue", "XNAS")], None)` | `table.scan_matching(&filter, None)` |
| `handle.children_where(&[("venue", "XNAS")], false)` | `handle.children_matching(&filter, false)` |

=== "Rust"

    ```rust
    use yggdryl::io::partition::pairs_to_expr;

    // The pair form builds exactly the expression, quoting included.
    assert_eq!(
        pairs_to_expr(&[("venue", "XNAS"), ("total amount", "3")]).to_string(),
        "venue = 'XNAS' AND \"total amount\" = '3'"
    );
    // And the text `null` means the absence a directory name spells.
    assert_eq!(pairs_to_expr(&[("venue", "null")]).to_string(), "venue IS NULL");
    ```

<!-- notebooks: generated by scripts/build_docs_notebooks.py -->

## Notebooks

Every example on this page, as a notebook generated from these blocks and
shipped unexecuted:
[Rust](notebooks/expressions-rust.ipynb){ download }.

<!-- /notebooks -->
